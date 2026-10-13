#![cfg(feature = "async")]

use std::pin::Pin;
use std::sync::Arc;
use std::sync::Barrier;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::task::Context;
use std::task::Poll;
use std::task::Wake;
use std::task::Waker;
use teamy_cancellation::CancellationError;
use teamy_cancellation::CancellationToken;

#[derive(Default)]
struct WakeCounter {
    calls: AtomicUsize,
}

impl Wake for WakeCounter {
    fn wake(self: Arc<Self>) {
        self.calls.fetch_add(1, Ordering::Relaxed);
    }
}

impl WakeCounter {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }
}

fn poll_with<F: Future + Unpin>(future: &mut F, counter: &Arc<WakeCounter>) -> Poll<F::Output> {
    let waker = Waker::from(Arc::clone(counter));
    Pin::new(future).poll(&mut Context::from_waker(&waker))
}

#[test]
fn waiting_after_cancellation_is_immediately_ready() {
    let token = CancellationToken::new();
    token.request_cancel("earlier stop");

    assert!(poll_with(&mut token.cancelled(), &Arc::default()).is_ready());
}

#[test]
fn a_request_wakes_every_independent_waiter() {
    let token = CancellationToken::new();
    let mut waiters = (0..32)
        .map(|_| (token.cancelled(), Arc::<WakeCounter>::default()))
        .collect::<Vec<_>>();
    for (wait, counter) in &mut waiters {
        assert!(poll_with(wait, counter).is_pending());
    }

    token.request_cancel("stop everyone");

    for (wait, counter) in &mut waiters {
        assert_eq!(counter.calls(), 1);
        assert!(poll_with(wait, counter).is_ready());
    }
}

#[test]
fn polling_with_a_new_waker_replaces_the_previous_waker() {
    let token = CancellationToken::new();
    let mut wait = token.cancelled();
    let previous = Arc::<WakeCounter>::default();
    let current = Arc::<WakeCounter>::default();
    assert!(poll_with(&mut wait, &previous).is_pending());
    assert!(poll_with(&mut wait, &current).is_pending());

    token.request_cancel("stop");

    assert_eq!(previous.calls(), 0);
    assert_eq!(current.calls(), 1);
    assert!(poll_with(&mut wait, &current).is_ready());
}

#[test]
fn dropping_a_waiter_unregisters_its_waker() {
    let token = CancellationToken::new();
    let mut wait = token.cancelled();
    let counter = Arc::<WakeCounter>::default();
    assert!(poll_with(&mut wait, &counter).is_pending());

    drop(wait);
    token.request_cancel("stop");

    assert_eq!(counter.calls(), 0);
}

#[test]
fn a_request_racing_with_poll_registration_is_not_lost() {
    for _ in 0..100 {
        let token = CancellationToken::new();
        let requester = token.clone();
        let start = Arc::new(Barrier::new(2));
        let requester_start = Arc::clone(&start);
        let request = std::thread::spawn(move || {
            requester_start.wait();
            requester.request_cancel("racing stop");
        });
        let mut wait = token.cancelled();
        let counter = Arc::<WakeCounter>::default();
        start.wait();
        let pending = poll_with(&mut wait, &counter).is_pending();
        request.join().unwrap();

        if pending {
            assert!(counter.calls() > 0, "a pending waiter was never awakened");
        }
        assert!(poll_with(&mut wait, &counter).is_ready());
    }
}

#[test]
fn hook_panic_cannot_hide_cancellation_or_prevent_wakeups() {
    let token = CancellationToken::new_with_on_cancel_request(|_, _| panic!("hook failed"));
    let mut wait = token.cancelled();
    let counter = Arc::<WakeCounter>::default();
    assert!(poll_with(&mut wait, &counter).is_pending());

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        token.request_cancel("stop");
    }));

    assert!(result.is_err());
    assert!(token.is_cancelled());
    assert_eq!(token.cancellation_reason().as_deref(), Some("stop"));
    assert_eq!(counter.calls(), 1);
    assert!(poll_with(&mut wait, &counter).is_ready());
}

#[test]
fn synchronous_checkpoint_preserves_the_typed_cancellation_error() {
    let token = CancellationToken::new();
    token.request_cancel("typed stop");

    let error = token.bail_if_cancelled().unwrap_err();

    assert_eq!(
        error.downcast_ref::<CancellationError>().unwrap().reason,
        "typed stop"
    );
}

#[cfg(feature = "inheritance")]
#[test]
fn an_ancestor_request_wakes_all_descendant_waiters() {
    let parent = CancellationToken::new();
    let child = parent.child_token();
    let grandchild = child.child_token();
    let counter = Arc::<WakeCounter>::default();
    let mut wait = grandchild.cancelled();
    assert!(poll_with(&mut wait, &counter).is_pending());

    parent.request_cancel("ancestor stop");

    assert_eq!(counter.calls(), 1);
    assert!(poll_with(&mut wait, &counter).is_ready());
    assert_eq!(
        grandchild.cancellation_reason().as_deref(),
        Some("ancestor stop")
    );
}

#[cfg(feature = "inheritance")]
#[test]
fn a_child_request_does_not_wake_parent_or_sibling_waiters() {
    let parent = CancellationToken::new();
    let child = parent.child_token();
    let sibling = parent.child_token();
    let mut parent_wait = parent.cancelled();
    let mut child_wait = child.cancelled();
    let mut sibling_wait = sibling.cancelled();
    let parent_counter = Arc::<WakeCounter>::default();
    let child_counter = Arc::<WakeCounter>::default();
    let sibling_counter = Arc::<WakeCounter>::default();
    assert!(poll_with(&mut parent_wait, &parent_counter).is_pending());
    assert!(poll_with(&mut child_wait, &child_counter).is_pending());
    assert!(poll_with(&mut sibling_wait, &sibling_counter).is_pending());

    child.request_cancel("child stop");

    assert_eq!(child_counter.calls(), 1);
    assert!(poll_with(&mut child_wait, &child_counter).is_ready());
    assert_eq!(parent_counter.calls(), 0);
    assert!(poll_with(&mut parent_wait, &parent_counter).is_pending());
    assert_eq!(sibling_counter.calls(), 0);
    assert!(poll_with(&mut sibling_wait, &sibling_counter).is_pending());
}
