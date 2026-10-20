#![cfg(feature = "async")]

#[cfg(feature = "arbitrary")]
use arbitrary::{Arbitrary, Unstructured};
use std::future::pending;
#[cfg(all(feature = "deferral", feature = "inheritance"))]
use std::future::poll_fn;
use std::pin::Pin;
#[cfg(all(feature = "deferral", feature = "inheritance"))]
use std::sync::Arc;
#[cfg(all(feature = "deferral", feature = "inheritance"))]
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Waker};
#[cfg(feature = "arbitrary")]
use teamy_cancellation::CancellationScope;
use teamy_cancellation::{CancellationError, CancellationToken};

fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

#[test]
fn a_cooperative_owner_handles_child_cancellation_before_its_cleanup() {
    let token = CancellationToken::new();
    let cleaned_up = AtomicBool::new(false);
    let cleanup = &cleaned_up;
    let mut owner = Box::pin(token.with_async_scope(|scope| async move {
        let error = scope
            .cancellable_scope(|_| pending::<Result<(), CancellationError>>())
            .await
            .expect_err("the child must acknowledge cancellation");
        assert_eq!(error.reason, "stop the operation");
        cleanup.store(true, Ordering::SeqCst);
        Ok::<_, CancellationError>(42)
    }));
    assert!(poll_once(owner.as_mut()).is_pending());
    token.request_cancel("stop the operation");
    assert_eq!(poll_once(owner.as_mut()), Poll::Ready(Ok(42)));
    assert!(cleaned_up.load(Ordering::SeqCst));
}

#[cfg(all(feature = "deferral", feature = "inheritance"))]
#[test]
fn independent_child_cancellation_and_deep_protection_have_distinct_ownership() {
    let parent = CancellationToken::new();
    let selected_control = Arc::new(Mutex::new(None));
    let selected_stopped = Arc::new(AtomicBool::new(false));
    let finish_protected = Arc::new(AtomicBool::new(false));
    let controls = Arc::clone(&selected_control);
    let stopped = Arc::clone(&selected_stopped);
    let finish = Arc::clone(&finish_protected);
    let mut owner = Box::pin(parent.cancellable_scope(move |scope| async move {
        let (control, selected) =
            scope.cancellable_child_scope(|_| pending::<Result<(), CancellationError>>());
        *controls.lock().unwrap() = Some(control);
        let mut selected = Box::pin(selected);
        let (_, protected) = scope.cancellable_child_scope(move |child| {
            child.cancellable_scope(move |nested| {
                nested.deferred("finish the admitted outcome", move |_| {
                    poll_fn(move |_| {
                        if finish.load(Ordering::SeqCst) {
                            Poll::Ready(Ok::<_, CancellationError>(17))
                        } else {
                            Poll::Pending
                        }
                    })
                })
            })
        });
        let mut protected = Box::pin(protected);
        poll_fn(move |context| {
            if !stopped.load(Ordering::SeqCst)
                && let Poll::Ready(result) = selected.as_mut().poll(context)
            {
                assert_eq!(result.unwrap_err().reason, "cancel only the selected child");
                stopped.store(true, Ordering::SeqCst);
            }
            protected.as_mut().poll(context)
        })
        .await
    }));
    assert!(poll_once(owner.as_mut()).is_pending());
    selected_control
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .request_cancel("cancel only the selected child");
    assert!(
        !parent.is_cancelled(),
        "child intent must not cancel its owner"
    );
    assert!(poll_once(owner.as_mut()).is_pending());
    assert!(selected_stopped.load(Ordering::SeqCst));
    parent.request_cancel("close all operations");
    assert!(
        poll_once(owner.as_mut()).is_pending(),
        "deep required work retains its owners"
    );
    finish_protected.store(true, Ordering::SeqCst);
    assert_eq!(poll_once(owner.as_mut()), Poll::Ready(Ok(17)));
}

#[cfg(feature = "arbitrary")]
#[test]
fn generated_request_metadata_needs_an_owned_scope_before_execution() {
    let placeholder = CancellationScope::arbitrary(&mut Unstructured::new(&[])).unwrap();
    let entered = AtomicBool::new(false);
    let mut rejected = Box::pin(placeholder.cancellable_scope(|_| {
        entered.store(true, Ordering::SeqCst);
        async { Ok::<_, CancellationError>(()) }
    }));
    assert!(matches!(poll_once(rejected.as_mut()), Poll::Ready(Err(_))));
    assert!(!entered.load(Ordering::SeqCst));
    let token = CancellationToken::new();
    let marker = &entered;
    let mut rebound = Box::pin(token.with_async_scope(|bound| {
        bound.cancellable_scope(move |_| {
            marker.store(true, Ordering::SeqCst);
            async { Ok::<_, CancellationError>(()) }
        })
    }));
    assert_eq!(poll_once(rebound.as_mut()), Poll::Ready(Ok(())));
    assert!(entered.load(Ordering::SeqCst));
}
