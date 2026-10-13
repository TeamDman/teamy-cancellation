#![cfg(feature = "async")]

use std::future::pending;
use std::future::poll_fn;
use std::pin::Pin;
use std::sync::Arc;
#[cfg(feature = "deferral")]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::task::Context;
use std::task::Poll;
use std::task::Wake;
use std::task::Waker;
use teamy_cancellation::CancellationError;
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

fn poll_once<F: Future + Unpin>(future: &mut F, counter: &Arc<WakeCounter>) -> Poll<F::Output> {
    let waker = Waker::from(Arc::clone(counter));
    Pin::new(future).poll(&mut Context::from_waker(&waker))
}

#[test]
fn a_preexisting_request_prevents_the_operation_from_starting() {
    let token = CancellationToken::new();
    token.request_cancel("stop before mutation");
    let polls = AtomicUsize::new(0);
    let mut future = token.cancellable(poll_fn(|_| {
        polls.fetch_add(1, Ordering::SeqCst);
        Poll::Ready(Ok::<_, CancellationError>(()))
    }));
    let Poll::Ready(Err(error)) = poll_once(&mut future, &Arc::default()) else {
        panic!("A cancelled operation must stop before executing");
    };
    assert_eq!(error.reason, "stop before mutation");
    assert_eq!(polls.load(Ordering::SeqCst), 0);
}

#[test]
fn cancellation_wakes_an_operation_waiting_on_unrelated_work() {
    let token = CancellationToken::new();
    let mut future = token.cancellable(pending::<Result<(), CancellationError>>());
    let counter = Arc::default();
    assert!(poll_once(&mut future, &counter).is_pending());

    token.request_cancel("user stopped a slow read");
    assert!(counter.0.load(Ordering::SeqCst) > 0);
    let Poll::Ready(Err(error)) = poll_once(&mut future, &counter) else {
        panic!("The waiting operation must acknowledge cancellation");
    };
    assert_eq!(error.reason, "user stopped a slow read");
}

#[test]
fn the_operation_can_borrow_caller_owned_values() {
    let token = CancellationToken::new();
    let owned = String::from("borrowed");
    let mut future = token.cancellable(async { Ok::<_, CancellationError>(owned.len()) });
    assert_eq!(poll_once(&mut future, &Arc::default()), Poll::Ready(Ok(8)));
    assert_eq!(owned, "borrowed");
}

#[test]
fn a_ready_operation_retains_its_result_when_cancellation_arrives_during_poll() {
    let token = CancellationToken::new();
    let mut success = token.cancellable(poll_fn(|_| {
        token.request_cancel("arrived with result");
        Poll::Ready(Ok::<_, CancellationError>(42))
    }));
    assert_eq!(
        poll_once(&mut success, &Arc::default()),
        Poll::Ready(Ok(42))
    );

    let token = CancellationToken::new();
    let mut failure = token.cancellable(poll_fn(|_| {
        token.request_cancel("arrived with failure");
        Poll::Ready(Err::<(), _>(eyre::eyre!("original operation failure")))
    }));
    let Poll::Ready(Err(error)) = poll_once(&mut failure, &Arc::default()) else {
        panic!("The operation's own error must remain observable");
    };
    assert_eq!(error.to_string(), "original operation failure");
    assert!(error.downcast_ref::<CancellationError>().is_none());
}

#[cfg(feature = "deferral")]
#[test]
fn a_deferred_operation_finishes_required_work_after_a_cancellation_request() {
    let token = CancellationToken::new().for_operation();
    let body_token = token.clone();
    let may_finish = AtomicBool::new(false);
    let finished = AtomicBool::new(false);
    let mut future = token.cancellable(async {
        let _deferral = body_token.defer_cancellation("recording mutation and invalidating caches");
        poll_fn(|_| {
            if may_finish.load(Ordering::SeqCst) {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await;
        body_token.bail_if_cancelled()?;
        finished.store(true, Ordering::SeqCst);
        Ok::<_, eyre::Report>("mutation outcome")
    });
    let counter = Arc::default();
    assert!(poll_once(&mut future, &counter).is_pending());

    token.request_cancel("stop after mutation started");
    assert!(token.is_cancelled());
    assert!(poll_once(&mut future, &counter).is_pending());
    assert!(!finished.load(Ordering::SeqCst));

    may_finish.store(true, Ordering::SeqCst);
    let Poll::Ready(Ok(outcome)) = poll_once(&mut future, &counter) else {
        panic!("Required completion work must be allowed to finish");
    };
    assert_eq!(outcome, "mutation outcome");
    assert!(finished.load(Ordering::SeqCst));
    assert!(token.bail_if_cancelled().is_err());
}

#[cfg(feature = "deferral")]
#[test]
fn dropping_the_last_deferral_wakes_a_wrapper_after_raw_cancellation_is_ready() {
    let token = CancellationToken::new().for_operation();
    let deferral = token.defer_cancellation("finishing an owned cache write");
    let mut future = token.cancellable(pending::<Result<(), CancellationError>>());
    let counter = Arc::default();
    assert!(poll_once(&mut future, &counter).is_pending());

    token.request_cancel("stop");
    assert!(poll_once(&mut future, &counter).is_pending());
    let before_guard_drop = counter.0.load(Ordering::SeqCst);
    drop(deferral);
    assert!(counter.0.load(Ordering::SeqCst) > before_guard_drop);
    assert!(matches!(
        poll_once(&mut future, &counter),
        Poll::Ready(Err(_))
    ));
}

#[cfg(feature = "deferral")]
#[test]
fn an_independent_operation_stops_while_another_operation_finishes_cleanup() {
    let token = CancellationToken::new();
    let completing = token.for_operation();
    let independent = token.for_operation();
    let deferral = completing.defer_cancellation("persisting operation artifacts");
    token.request_cancel("stop all operations");
    assert!(completing.bail_if_cancelled().is_ok());
    assert!(independent.bail_if_cancelled().is_err());
    assert!(token.bail_if_cancelled().is_err());

    let mut independent_future =
        independent.cancellable(pending::<Result<(), CancellationError>>());
    assert!(matches!(
        poll_once(&mut independent_future, &Arc::default()),
        Poll::Ready(Err(_))
    ));
    drop(deferral);
    assert!(completing.bail_if_cancelled().is_err());
}

#[cfg(feature = "deferral")]
#[test]
fn dropping_a_wrapper_unregisters_its_deferral_waker() {
    let token = CancellationToken::new().for_operation();
    let deferral = token.defer_cancellation("cleanup");
    let mut future = token.cancellable(pending::<Result<(), CancellationError>>());
    let counter = Arc::default();
    assert!(poll_once(&mut future, &counter).is_pending());
    drop(future);

    let before = counter.0.load(Ordering::SeqCst);
    token.request_cancel("stop");
    drop(deferral);
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
    let mut future = token.cancellable(PanickingDropFuture);
    assert_eq!(poll_once(&mut future, &Arc::default()), Poll::Ready(Ok(42)));
    // The host can retain that outcome before independently guarding Drop.
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(future))).is_err());
}
