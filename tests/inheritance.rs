#![cfg(feature = "inheritance")]

use teamy_cancellation::CancellationToken;

#[test]
fn child_token_observes_parent_cancellation() {
    let parent = CancellationToken::new();
    let child = parent.child_token();

    parent.request_cancel("daemon stop");

    assert!(child.is_cancelled());
    assert_eq!(child.cancellation_reason().as_deref(), Some("daemon stop"));
}

#[test]
fn child_token_cancellation_does_not_cancel_parent() {
    let parent = CancellationToken::new();
    let child = parent.child_token();

    child.request_cancel("client disconnected");

    assert!(!parent.is_cancelled());
    assert!(child.is_cancelled());
    assert_eq!(
        child.cancellation_reason().as_deref(),
        Some("client disconnected")
    );
}

#[test]
fn child_token_keeps_own_reason_after_parent_cancels() {
    let parent = CancellationToken::new();
    let child = parent.child_token();

    child.request_cancel("client disconnected");
    parent.request_cancel("daemon stop");

    assert_eq!(
        child.cancellation_reason().as_deref(),
        Some("client disconnected")
    );
}
