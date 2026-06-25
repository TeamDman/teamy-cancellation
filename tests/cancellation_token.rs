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
