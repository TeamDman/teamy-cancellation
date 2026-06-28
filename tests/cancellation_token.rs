use std::sync::Arc;
use std::sync::Mutex;
use teamy_cancellation::CancellationToken;

#[test]
fn cancellation_token_clones_share_cancellation() {
    let token = CancellationToken::new();
    let clone = token.clone();

    clone.request_cancel("profile stop");

    assert!(token.is_cancelled());
    assert_eq!(token.cancellation_reason().as_deref(), Some("profile stop"));
}

#[test]
fn cancellation_token_keeps_first_reason() {
    let token = CancellationToken::new();

    token.request_cancel("first");
    token.request_cancel("second");

    assert_eq!(token.cancellation_reason().as_deref(), Some("first"));
    assert!(
        token
            .bail_if_cancelled()
            .unwrap_err()
            .to_string()
            .contains("first")
    );
}

#[test]
fn cancellation_hook_observes_first_and_repeated_requests() {
    let observed = Arc::new(Mutex::new(Vec::<(String, bool)>::new()));
    let hook_observed = Arc::clone(&observed);
    let token = CancellationToken::new_with_on_cancel_request(move |reason, was_first| {
        hook_observed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((reason.to_string(), was_first));
    });

    token.request_cancel("first");
    token.request_cancel("second");

    let observed = observed
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert_eq!(
        observed,
        vec![
            (String::from("first"), true),
            (String::from("second"), false),
        ]
    );
}
