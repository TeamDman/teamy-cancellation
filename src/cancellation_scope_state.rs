use crate::CancellationToken;
use crate::cancellation_scope_kind::CancellationScopeKind;
use crate::cancellation_scope_tree::CancellationScopeTree;
use std::sync::Arc;
use std::sync::Mutex;
#[cfg(feature = "async")]
use std::task::Waker;

/// Shared identity of an execution scope, whose lifetime is owned by a runner.
/// Parent handles keep their records available while descendants still exist;
/// owner closure is explicit and does not depend on the last handle being dropped.
#[derive(Debug)]
pub(crate) struct CancellationScopeState {
    tree: Arc<Mutex<CancellationScopeTree>>,
    id: usize,
    _parent: Option<Arc<Self>>,
}

impl CancellationScopeState {
    pub(crate) fn root() -> Arc<Self> {
        let mut tree = CancellationScopeTree::default();
        let id = tree.insert(None);
        Arc::new(Self {
            tree: Arc::new(Mutex::new(tree)),
            id,
            _parent: None,
        })
    }

    pub(crate) fn child(self: &Arc<Self>) -> Arc<Self> {
        let id = self
            .tree
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(Some(self.id));
        Arc::new(Self {
            tree: Arc::clone(&self.tree),
            id,
            _parent: Some(Arc::clone(self)),
        })
    }

    pub(crate) fn admit(&self, token: &CancellationToken, kind: CancellationScopeKind) -> bool {
        let mut tree = self
            .tree
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Checking sticky intent while holding the same lock used by
        // stop_if_cancelled linearizes admission against owner cancellation.
        let admitted = tree.admit(self.id, token.is_cancelled(), kind);
        #[cfg(feature = "async")]
        let wakers = if admitted { Vec::new() } else { tree.wakers() };
        drop(tree);
        #[cfg(feature = "async")]
        for waker in wakers {
            waker.wake();
        }
        admitted
    }

    #[cfg(any(feature = "async", all(test, feature = "deferral")))]
    pub(crate) fn stop_if_cancelled(&self, token: &CancellationToken) -> bool {
        let stopping;
        #[cfg(feature = "async")]
        let wakers;
        {
            let mut tree = self
                .tree
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            stopping = tree.is_stopping(self.id, token.is_cancelled());
            if stopping {
                tree.close(self.id);
            }
            #[cfg(feature = "async")]
            {
                wakers = if stopping { tree.wakers() } else { Vec::new() };
            }
        }
        #[cfg(feature = "async")]
        for waker in wakers {
            waker.wake();
        }
        stopping
    }

    #[cfg(feature = "async")]
    pub(crate) fn is_closed(&self) -> bool {
        self.tree
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_stopping(self.id, false)
    }

    pub(crate) fn is_stopping(&self, token: &CancellationToken) -> bool {
        self.tree
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_stopping(self.id, token.is_cancelled())
    }

    pub(crate) fn close(&self) {
        #[cfg(feature = "async")]
        let wakers;
        {
            let mut tree = self
                .tree
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            tree.close(self.id);
            #[cfg(feature = "async")]
            {
                wakers = tree.wakers();
            }
        }
        #[cfg(feature = "async")]
        for waker in wakers {
            waker.wake();
        }
    }

    #[cfg(feature = "async")]
    pub(crate) fn register_waker(&self, waker: &Waker) {
        // RawWaker clone/drop hooks may execute user code. Neither operation
        // runs under the tree mutex, including replacement or stale admission.
        let incoming = waker.clone();
        let previous = self
            .tree
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .register_waker(self.id, incoming);
        drop(previous);
    }
}

impl Drop for CancellationScopeState {
    fn drop(&mut self) {
        #[cfg(feature = "async")]
        let wakers;
        {
            let mut tree = self
                .tree
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // Descendants hold an Arc to this node, so none can survive the
            // final handle and require this record after it is removed.
            tree.remove(self.id);
            #[cfg(feature = "async")]
            {
                wakers = tree.wakers();
            }
        }
        #[cfg(feature = "async")]
        for waker in wakers {
            waker.wake();
        }
    }
}

#[cfg(all(test, feature = "deferral"))]
mod tests {
    use super::CancellationScopeState;
    use crate::CancellationToken;
    use crate::cancellation_scope_kind::CancellationScopeKind;
    use std::sync::Arc;
    use std::sync::Barrier;
    #[cfg(feature = "async")]
    use std::sync::Weak;
    #[cfg(feature = "async")]
    use std::sync::atomic::AtomicUsize;
    #[cfg(feature = "async")]
    use std::sync::atomic::Ordering;
    #[cfg(feature = "async")]
    use std::task::Wake;
    #[cfg(feature = "async")]
    use std::task::Waker;

    #[test]
    fn protected_descendant_retains_owners_without_protecting_a_sibling() {
        let token = CancellationToken::new();
        let root = CancellationScopeState::root();
        assert!(root.admit(&token, CancellationScopeKind::Cancellable));
        let intermediate = root.child();
        assert!(intermediate.admit(&token, CancellationScopeKind::Cancellable));
        let protected = intermediate.child();
        assert!(protected.admit(&token, CancellationScopeKind::Deferred));
        let nested = protected.child();
        assert!(nested.admit(&token, CancellationScopeKind::Cancellable));
        let sibling = root.child();
        assert!(sibling.admit(&token, CancellationScopeKind::Cancellable));

        token.request_cancel("stop ordinary work");

        assert!(!root.stop_if_cancelled(&token));
        assert!(!intermediate.stop_if_cancelled(&token));
        assert!(!protected.stop_if_cancelled(&token));
        assert!(!nested.stop_if_cancelled(&token));
        assert!(sibling.stop_if_cancelled(&token));

        protected.close();

        assert!(root.stop_if_cancelled(&token));
        assert!(nested.stop_if_cancelled(&token));
    }

    #[test]
    fn forced_owner_completion_releases_protected_descendant_retention() {
        let token = CancellationToken::new();
        let root = CancellationScopeState::root();
        assert!(root.admit(&token, CancellationScopeKind::Cancellable));
        let owner = root.child();
        assert!(owner.admit(&token, CancellationScopeKind::Cancellable));
        let protected = owner.child();
        assert!(protected.admit(&token, CancellationScopeKind::Deferred));

        // An inner Ready/error or explicit future drop may abandon this subtree.
        owner.close();
        token.request_cancel("stop remaining work");

        assert!(root.stop_if_cancelled(&token));
        assert!(protected.is_stopping(&token));
        assert!(
            !protected
                .child()
                .admit(&token, CancellationScopeKind::Required)
        );
    }

    #[test]
    fn required_admission_and_owner_cancellation_choose_one_winner() {
        for _ in 0..100 {
            let token = CancellationToken::new();
            let root = CancellationScopeState::root();
            assert!(root.admit(&token, CancellationScopeKind::Cancellable));
            let required = root.child();
            token.request_cancel("race required completion against owner cancellation");
            let start = Arc::new(Barrier::new(2));
            let requester_start = Arc::clone(&start);
            let requester_token = token.clone();
            let requester_required = Arc::clone(&required);
            let request = std::thread::spawn(move || {
                requester_start.wait();
                requester_required.admit(&requester_token, CancellationScopeKind::Required)
            });

            start.wait();
            let owner_stopped = root.stop_if_cancelled(&token);
            let child_admitted = request.join().unwrap();

            assert_ne!(owner_stopped, child_admitted);
        }
    }

    #[cfg(feature = "async")]
    #[test]
    fn owner_closure_releases_its_waker_and_notifies_outside_the_tree_lock() {
        struct ReentrantWake {
            scope: Weak<CancellationScopeState>,
            token: CancellationToken,
            calls: AtomicUsize,
        }

        impl Wake for ReentrantWake {
            fn wake(self: Arc<Self>) {
                self.wake_by_ref();
            }

            fn wake_by_ref(self: &Arc<Self>) {
                // Reentering the scope would deadlock if wake held its mutex.
                if let Some(scope) = self.scope.upgrade() {
                    assert!(scope.is_stopping(&self.token));
                }
                self.calls.fetch_add(1, Ordering::Relaxed);
            }
        }

        let token = CancellationToken::new();
        let root = CancellationScopeState::root();
        assert!(root.admit(&token, CancellationScopeKind::Cancellable));
        let counter = Arc::new(ReentrantWake {
            scope: Arc::downgrade(&root),
            token,
            calls: AtomicUsize::new(0),
        });
        let waker = Waker::from(Arc::clone(&counter));
        root.register_waker(&waker);

        root.close();

        assert_eq!(counter.calls.load(Ordering::Relaxed), 1);
        root.close();
        assert_eq!(counter.calls.load(Ordering::Relaxed), 1);
        // Simulate close winning the race just before poll registration.
        root.register_waker(&waker);
        drop(waker);
        assert_eq!(Arc::strong_count(&counter), 1);
    }

    #[cfg(feature = "async")]
    #[test]
    fn finishing_an_unrelated_scope_does_not_wake_another_owner() {
        #[derive(Default)]
        struct WakeCounter(AtomicUsize);

        impl Wake for WakeCounter {
            fn wake(self: Arc<Self>) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }

        let token = CancellationToken::new();
        let root = CancellationScopeState::root();
        assert!(root.admit(&token, CancellationScopeKind::Cancellable));
        let counter = Arc::<WakeCounter>::default();
        root.register_waker(&Waker::from(Arc::clone(&counter)));
        let sibling = root.child();
        assert!(sibling.admit(&token, CancellationScopeKind::Cancellable));

        sibling.close();
        sibling.close();
        drop(sibling);

        assert_eq!(counter.0.load(Ordering::Relaxed), 0);
    }
}
