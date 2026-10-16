#![cfg(feature = "async")]

use std::future::pending;
use std::future::poll_fn;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::task::Context;
use std::task::Poll;
use std::task::Wake;
use std::task::Waker;
use teamy_cancellation::CancellationError;
use teamy_cancellation::CancellationScope;
use teamy_cancellation::CancellationToken;

#[derive(Default)]
struct WakeCounter(AtomicUsize);

impl Wake for WakeCounter {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn poll_once<F: Future + ?Sized>(
    future: Pin<&mut F>,
    counter: &Arc<WakeCounter>,
) -> Poll<F::Output> {
    let waker = Waker::from(Arc::clone(counter));
    future.poll(&mut Context::from_waker(&waker))
}

#[test]
fn a_preexisting_request_prevents_the_scope_factory_from_starting() {
    let token = CancellationToken::new();
    token.request_cancel("stop before mutation");
    let calls = AtomicUsize::new(0);
    let mut future = Box::pin(token.cancellable_scope(|_| {
        calls.fetch_add(1, Ordering::SeqCst);
        async { Ok::<_, CancellationError>(()) }
    }));
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    let Poll::Ready(Err(error)) = poll_once(future.as_mut(), &Arc::default()) else {
        panic!("A cancelled operation must stop before its factory executes");
    };

    assert_eq!(error.reason, "stop before mutation");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn cancellation_wakes_a_scope_waiting_on_unrelated_work() {
    let token = CancellationToken::new();
    let mut future =
        Box::pin(token.cancellable_scope(|_| pending::<Result<(), CancellationError>>()));
    let counter = Arc::default();
    assert!(poll_once(future.as_mut(), &counter).is_pending());

    token.request_cancel("user stopped a slow read");

    assert!(counter.0.load(Ordering::SeqCst) > 0);
    let Poll::Ready(Err(error)) = poll_once(future.as_mut(), &counter) else {
        panic!("The waiting operation must acknowledge cancellation");
    };
    assert_eq!(error.reason, "user stopped a slow read");
}

#[test]
fn a_scope_can_borrow_and_update_caller_owned_values() {
    let token = CancellationToken::new();
    let mut owned = String::from("borrowed");
    let mut future = Box::pin(token.cancellable_scope(|_| async {
        owned.push_str(" value");
        Ok::<_, CancellationError>(owned.len())
    }));

    assert_eq!(
        poll_once(future.as_mut(), &Arc::default()),
        Poll::Ready(Ok(14))
    );
    drop(future);
    assert_eq!(owned, "borrowed value");
}

#[test]
fn a_ready_scope_preserves_its_result_when_cancellation_arrives_during_poll() {
    let token = CancellationToken::new();
    let completing = token.clone();
    let mut success = Box::pin(token.cancellable_scope(move |_| {
        poll_fn(move |_| {
            completing.request_cancel("arrived with result");
            Poll::Ready(Ok::<_, CancellationError>(42))
        })
    }));
    assert_eq!(
        poll_once(success.as_mut(), &Arc::default()),
        Poll::Ready(Ok(42))
    );

    let token = CancellationToken::new();
    let completing = token.clone();
    let mut failure = Box::pin(token.cancellable_scope(move |_| {
        poll_fn(move |_| {
            completing.request_cancel("arrived with failure");
            Poll::Ready(Err::<(), _>(eyre::eyre!("original operation failure")))
        })
    }));
    let Poll::Ready(Err(error)) = poll_once(failure.as_mut(), &Arc::default()) else {
        panic!("The operation's own failure must remain observable");
    };
    assert_eq!(error.to_string(), "original operation failure");
    assert!(error.downcast_ref::<CancellationError>().is_none());
}

#[test]
fn completed_owners_close_saved_scope_handles_before_the_future_is_dropped() {
    let token = CancellationToken::new();
    let saved = Arc::new(Mutex::new(None::<CancellationScope>));
    let captured = Arc::clone(&saved);
    let mut parent = Box::pin(token.cancellable_scope(move |scope| {
        *captured.lock().unwrap() = Some(scope.clone());
        async { Ok::<_, CancellationError>(()) }
    }));
    assert!(matches!(
        poll_once(parent.as_mut(), &Arc::default()),
        Poll::Ready(Ok(()))
    ));
    let closed = saved.lock().unwrap().take().unwrap();
    let entered = AtomicBool::new(false);
    let mut child = Box::pin(closed.cancellable_scope(|_| {
        entered.store(true, Ordering::SeqCst);
        async { Ok::<_, CancellationError>(()) }
    }));

    assert!(matches!(
        poll_once(child.as_mut(), &Arc::default()),
        Poll::Ready(Err(_))
    ));
    assert!(!entered.load(Ordering::SeqCst));

    #[cfg(feature = "deferral")]
    {
        let mut required = Box::pin(closed.finish_required("late cleanup", |_| {
            entered.store(true, Ordering::SeqCst);
            async { Ok::<_, CancellationError>(()) }
        }));
        assert!(matches!(
            poll_once(required.as_mut(), &Arc::default()),
            Poll::Ready(Err(_))
        ));
        assert!(!entered.load(Ordering::SeqCst));
    }
}

#[cfg(feature = "deferral")]
async fn wait_for(flag: &AtomicBool) {
    poll_fn(|_| {
        if flag.load(Ordering::SeqCst) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
    .await;
}

#[cfg(feature = "deferral")]
#[derive(Default)]
struct CompletionControls {
    prelude: AtomicBool,
    first: AtomicBool,
    second: AtomicBool,
    entered: AtomicBool,
    completed: AtomicUsize,
    sibling_stopped: AtomicBool,
    protected_handle: Mutex<Option<CancellationScope>>,
}

#[cfg(feature = "deferral")]
async fn complete_two_required_steps(
    scope: CancellationScope,
    controls: &CompletionControls,
) -> Result<(), CancellationError> {
    scope
        .deferred(
            "recording mutation and invalidating related caches",
            |protected| async move {
                *controls.protected_handle.lock().unwrap() = Some(protected.clone());
                controls.entered.store(true, Ordering::SeqCst);
                wait_for(&controls.first).await;
                protected.bail_if_cancelled()?;
                controls.completed.fetch_add(1, Ordering::SeqCst);

                // This child is admitted after cancellation, through the protected
                // handle. An ordinary sibling must not inherit its protection.
                protected
                    .cancellable_scope(|last| async move {
                        last.bail_if_cancelled()?;
                        wait_for(&controls.second).await;
                        last.bail_if_cancelled()?;
                        controls.completed.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, CancellationError>(())
                    })
                    .await
            },
        )
        .await?;

    // Required work is complete. The next suspension must respect cancellation,
    // even though a cloned protected handle is still retained by this fixture.
    pending::<Result<(), CancellationError>>().await
}

#[cfg(feature = "deferral")]
#[test]
fn deeply_nested_required_work_retains_ancestors_without_protecting_a_sibling() {
    let token = CancellationToken::new();
    let controls = CompletionControls::default();
    let observed = &controls;
    let mut future = Box::pin(token.cancellable_scope(move |owner| async move {
        let mut required = Box::pin(owner.cancellable_scope(move |middle| async move {
            // The deferral is created after an ordinary suspension, not at
            // construction time or before the enclosing wrappers start.
            wait_for(&observed.prelude).await;
            middle
                .cancellable_scope(|deep| complete_two_required_steps(deep, observed))
                .await
        }));
        let mut sibling =
            Box::pin(owner.cancellable_scope(|_| pending::<Result<(), CancellationError>>()));
        let mut sibling_done = false;

        // Keep both owned branches alive while required work finishes. Unlike
        // try_join!, this fixture does not drop one branch when the other stops.
        poll_fn(move |cx| {
            let progress = required.as_mut().poll(cx);
            if !sibling_done && let Poll::Ready(result) = sibling.as_mut().poll(cx) {
                assert!(result.is_err());
                observed.sibling_stopped.store(true, Ordering::SeqCst);
                sibling_done = true;
            }
            progress
        })
        .await
    }));
    let counter = Arc::default();

    assert!(poll_once(future.as_mut(), &counter).is_pending());
    assert!(!controls.entered.load(Ordering::SeqCst));
    controls.prelude.store(true, Ordering::SeqCst);
    assert!(poll_once(future.as_mut(), &counter).is_pending());
    assert!(controls.entered.load(Ordering::SeqCst));

    token.request_cancel("stop after required work started");
    assert!(poll_once(future.as_mut(), &counter).is_pending());
    assert!(controls.sibling_stopped.load(Ordering::SeqCst));
    assert_eq!(controls.completed.load(Ordering::SeqCst), 0);

    controls.first.store(true, Ordering::SeqCst);
    assert!(poll_once(future.as_mut(), &counter).is_pending());
    assert_eq!(controls.completed.load(Ordering::SeqCst), 1);

    controls.second.store(true, Ordering::SeqCst);
    let Poll::Ready(Err(error)) = poll_once(future.as_mut(), &counter) else {
        panic!("Cancellation must resume after both required steps finish");
    };
    assert_eq!(error.reason, "stop after required work started");
    assert_eq!(controls.completed.load(Ordering::SeqCst), 2);
    assert!(controls.protected_handle.lock().unwrap().is_some());
}

#[cfg(feature = "deferral")]
#[test]
fn unpolled_deferred_work_does_not_postpone_its_owner() {
    let token = CancellationToken::new();
    let entered = &AtomicBool::new(false);
    let mut future = Box::pin(token.cancellable_scope(|scope| async move {
        let _unpolled = scope.deferred("unused work", |_| {
            entered.store(true, Ordering::SeqCst);
            async { Ok::<_, CancellationError>(()) }
        });
        pending::<Result<(), CancellationError>>().await
    }));
    let counter = Arc::default();
    assert!(poll_once(future.as_mut(), &counter).is_pending());

    token.request_cancel("stop before required work starts");

    assert!(matches!(
        poll_once(future.as_mut(), &counter),
        Poll::Ready(Err(_))
    ));
    assert!(!entered.load(Ordering::SeqCst));
}

#[cfg(feature = "deferral")]
#[test]
fn ordinary_deferral_admission_rejects_cancellation_before_its_factory_runs() {
    let token = CancellationToken::new();
    let entered = &AtomicBool::new(false);
    let mut future = Box::pin(token.cancellable_scope(|scope| async move {
        scope.request_cancel("stop before the irreversible phase");
        scope
            .deferred("a mutation that has not started", |_| {
                entered.store(true, Ordering::SeqCst);
                async { Ok::<_, CancellationError>(()) }
            })
            .await
    }));

    let Poll::Ready(Err(error)) = poll_once(future.as_mut(), &Arc::default()) else {
        panic!("Ordinary deferral must not admit new work after cancellation");
    };
    assert_eq!(error.reason, "stop before the irreversible phase");
    assert!(!entered.load(Ordering::SeqCst));
}

#[cfg(feature = "deferral")]
#[test]
fn already_required_cleanup_can_start_after_cancellation_in_an_open_owner() {
    let token = CancellationToken::new();
    let may_finish = &AtomicBool::new(false);
    let finished = &AtomicBool::new(false);
    let mut future = Box::pin(token.cancellable_scope(|scope| async move {
        scope.request_cancel("stop after the remote outcome");
        scope
            .finish_required(
                "recording the already completed mutation",
                |protected| async move {
                    protected.bail_if_cancelled()?;
                    wait_for(may_finish).await;
                    protected.bail_if_cancelled()?;
                    finished.store(true, Ordering::SeqCst);
                    Ok::<_, CancellationError>(42)
                },
            )
            .await
    }));
    let counter = Arc::default();

    assert!(poll_once(future.as_mut(), &counter).is_pending());
    assert!(token.is_cancelled());
    assert!(!finished.load(Ordering::SeqCst));
    may_finish.store(true, Ordering::SeqCst);

    assert_eq!(poll_once(future.as_mut(), &counter), Poll::Ready(Ok(42)));
    assert!(finished.load(Ordering::SeqCst));
}

#[cfg(feature = "deferral")]
#[test]
fn required_completion_wakes_a_waiting_owner_before_the_child_future_is_dropped() {
    type OwnedFuture = Pin<Box<dyn Future<Output = Result<(), CancellationError>> + Send>>;

    let token = CancellationToken::new();
    let child_slot = Arc::new(Mutex::new(None::<OwnedFuture>));
    let captured_slot = Arc::clone(&child_slot);
    let may_finish = Arc::new(AtomicBool::new(false));
    let captured_gate = Arc::clone(&may_finish);
    let mut owner = Box::pin(token.cancellable_scope(move |scope| {
        // Polling this explicitly linked child separately models an owning
        // coordinator whose required child can wake independently.
        *captured_slot.lock().unwrap() = Some(Box::pin(scope.deferred(
            "finishing independently polled required work",
            move |_| async move {
                wait_for(&captured_gate).await;
                Ok::<_, CancellationError>(())
            },
        )));
        pending::<Result<(), CancellationError>>()
    }));
    let counter = Arc::default();
    assert!(poll_once(owner.as_mut(), &counter).is_pending());
    let mut child = child_slot.lock().unwrap().take().unwrap();
    assert!(poll_once(child.as_mut(), &Arc::default()).is_pending());

    token.request_cancel("stop while completion is pending");
    assert!(poll_once(owner.as_mut(), &counter).is_pending());
    let before_completion = counter.0.load(Ordering::SeqCst);
    may_finish.store(true, Ordering::SeqCst);
    assert!(matches!(
        poll_once(child.as_mut(), &Arc::default()),
        Poll::Ready(Ok(()))
    ));

    assert!(counter.0.load(Ordering::SeqCst) > before_completion);
    assert!(matches!(
        poll_once(owner.as_mut(), &counter),
        Poll::Ready(Err(_))
    ));
}

#[cfg(feature = "deferral")]
#[test]
fn cancellation_admission_racing_with_owner_abandonment_never_starts_unretained_work() {
    for _ in 0..24 {
        let token = CancellationToken::new();
        let saved = Arc::new(Mutex::new(None::<CancellationScope>));
        let captured = Arc::clone(&saved);
        let mut owner = Box::pin(token.cancellable_scope(move |scope| {
            *captured.lock().unwrap() = Some(scope);
            pending::<Result<(), CancellationError>>()
        }));
        assert!(poll_once(owner.as_mut(), &Arc::default()).is_pending());
        let scope = saved.lock().unwrap().take().unwrap();
        let start = Arc::new(std::sync::Barrier::new(2));
        let child_start = Arc::clone(&start);
        let entered = Arc::new(AtomicBool::new(false));
        let child_entered = Arc::clone(&entered);
        let child_thread = std::thread::spawn(move || {
            let mut child = Box::pin(scope.finish_required("required cleanup", move |_| {
                child_entered.store(true, Ordering::SeqCst);
                pending::<Result<(), CancellationError>>()
            }));
            child_start.wait();
            let result = poll_once(child.as_mut(), &Arc::default());
            (child, result)
        });

        token.request_cancel("race with required-work admission");
        start.wait();
        let owner_result = poll_once(owner.as_mut(), &Arc::default());
        let (child, child_result) = child_thread.join().unwrap();

        if entered.load(Ordering::SeqCst) {
            assert!(owner_result.is_pending());
            assert!(child_result.is_pending());
            drop(child);
            assert!(matches!(
                poll_once(owner.as_mut(), &Arc::default()),
                Poll::Ready(Err(_))
            ));
        } else {
            assert!(matches!(owner_result, Poll::Ready(Err(_))));
            assert!(matches!(child_result, Poll::Ready(Err(_))));
        }
    }
}

#[cfg(feature = "deferral")]
#[test]
fn dropping_an_owner_can_abandon_required_work_and_unregister_its_wakeups() {
    struct DropNotice(Arc<AtomicBool>);

    impl Drop for DropNotice {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    let token = CancellationToken::new();
    let dropped = Arc::new(AtomicBool::new(false));
    let notice = DropNotice(Arc::clone(&dropped));
    let mut owner = Box::pin(token.cancellable_scope(move |scope| async move {
        scope
            .deferred("owned required work", move |_| async move {
                let _notice = notice;
                pending::<Result<(), CancellationError>>().await
            })
            .await
    }));
    let counter = Arc::default();
    assert!(poll_once(owner.as_mut(), &counter).is_pending());
    assert!(!dropped.load(Ordering::SeqCst));

    drop(owner);

    assert!(dropped.load(Ordering::SeqCst));
    let before = counter.0.load(Ordering::SeqCst);
    token.request_cancel("stop after its owner was dropped");
    assert_eq!(counter.0.load(Ordering::SeqCst), before);
}

#[test]
fn a_ready_result_precedes_the_wrapped_future_destructor() {
    struct PanickingDropFuture;

    impl Future for PanickingDropFuture {
        type Output = Result<u8, CancellationError>;

        fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
            Poll::Ready(Ok(42))
        }
    }

    impl Drop for PanickingDropFuture {
        fn drop(&mut self) {
            panic!("synthetic future destructor failure");
        }
    }

    let token = CancellationToken::new();
    let mut future = Box::pin(token.cancellable_scope(|_| PanickingDropFuture));
    assert_eq!(
        poll_once(future.as_mut(), &Arc::default()),
        Poll::Ready(Ok(42))
    );
    // The host can retain the outcome before independently guarding Drop.
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(future))).is_err());
}
