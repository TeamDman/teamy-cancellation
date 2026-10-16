#[cfg(feature = "arbitrary")]
use arbitrary::Arbitrary;
use std::sync::Arc;
use std::sync::Barrier;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use teamy_cancellation::CancellationToken;

#[test]
fn listener_runs_once_and_keeps_the_first_reason() {
    let token = CancellationToken::new();
    let reasons = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&reasons);
    let _listener = token.register_listener(move |reason| {
        observed.lock().unwrap().push(reason.to_owned());
    });

    token.request_cancel("first stop");
    token.request_cancel("second stop");

    assert_eq!(*reasons.lock().unwrap(), vec!["first stop"]);
}

#[test]
fn late_listener_runs_before_registration_returns() {
    let token = CancellationToken::new();
    token.request_cancel("already stopped");
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&calls);

    let _listener = token.register_listener(move |reason| {
        assert_eq!(reason, "already stopped");
        observed.fetch_add(1, Ordering::Relaxed);
    });

    assert_eq!(calls.load(Ordering::Relaxed), 1);
    token.request_cancel("repeated stop");
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[test]
fn dropping_listener_unregisters_its_callback() {
    let token = CancellationToken::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&calls);
    let listener = token.register_listener(move |_| {
        observed.fetch_add(1, Ordering::Relaxed);
    });

    drop(listener);
    token.request_cancel("stop");

    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[test]
fn listener_observes_published_state_and_can_register_reentrantly() {
    let token = CancellationToken::new();
    let observed_token = token.clone();
    let nested_calls = Arc::new(AtomicUsize::new(0));
    let nested_observed = Arc::clone(&nested_calls);
    let _listener = token.register_listener(move |reason| {
        assert!(observed_token.is_cancelled());
        assert_eq!(
            observed_token.cancellation_reason().as_deref(),
            Some(reason)
        );
        let nested_observed = Arc::clone(&nested_observed);
        let _nested = observed_token.register_listener(move |_| {
            nested_observed.fetch_add(1, Ordering::Relaxed);
        });
    });

    token.request_cancel("stop");

    assert_eq!(nested_calls.load(Ordering::Relaxed), 1);
}

#[test]
fn registering_during_cancellation_never_loses_or_repeats_notification() {
    for _ in 0..100 {
        let token = CancellationToken::new();
        let requester = token.clone();
        let start = Arc::new(Barrier::new(2));
        let requester_start = Arc::clone(&start);
        let request = std::thread::spawn(move || {
            requester_start.wait();
            requester.request_cancel("racing stop");
        });
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&calls);
        start.wait();

        let _listener = token.register_listener(move |reason| {
            assert_eq!(reason, "racing stop");
            observed.fetch_add(1, Ordering::Relaxed);
        });
        request.join().unwrap();
        token.request_cancel("repeat");

        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }
}

#[cfg(feature = "inheritance")]
#[test]
fn ancestor_and_local_requests_notify_a_descendant_listener_once() {
    let parent = CancellationToken::new();
    let child = parent.child_token();
    let grandchild = child.child_token();
    let reasons = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&reasons);
    let _listener = grandchild.register_listener(move |reason| {
        observed.lock().unwrap().push(reason.to_owned());
    });

    parent.request_cancel("ancestor stop");
    child.request_cancel("local stop");
    grandchild.request_cancel("leaf stop");

    assert_eq!(*reasons.lock().unwrap(), vec!["ancestor stop"]);
    assert_eq!(
        grandchild.cancellation_reason().as_deref(),
        Some("leaf stop")
    );
}

#[cfg(feature = "arbitrary")]
#[test]
fn arbitrary_tokens_have_isolated_uncancelled_state() {
    let mut input = arbitrary::Unstructured::new(&[]);
    let first = CancellationToken::arbitrary(&mut input).unwrap();
    let second = CancellationToken::arbitrary(&mut input).unwrap();

    first.request_cancel("synthetic stop");

    assert!(!second.is_cancelled());
    assert_eq!(second.cancellation_reason(), None);
}
