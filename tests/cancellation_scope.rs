use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use teamy_cancellation::CancellationError;
use teamy_cancellation::CancellationToken;

#[test]
fn an_existing_request_prevents_root_callback_admission() {
    let token = CancellationToken::new();
    token.request_cancel("stop before the operation");
    let entered = AtomicBool::new(false);

    let result: Result<(), CancellationError> = token.with_scope(|_| {
        entered.store(true, Ordering::SeqCst);
        Ok(())
    });

    assert!(!entered.load(Ordering::SeqCst));
    assert_eq!(result.unwrap_err().reason, "stop before the operation");
}

#[test]
fn a_scope_checkpoint_preserves_the_shared_request_reason() {
    let token = CancellationToken::new();

    let result: Result<(), CancellationError> = token.with_scope(|scope| {
        scope.request_cancel("the user stopped this operation");
        scope.request_cancel("a later repeated request");
        assert!(scope.is_cancelled());
        assert_eq!(
            scope.cancellation_reason().as_deref(),
            Some("the user stopped this operation")
        );
        scope.bail_if_cancelled()
    });

    assert_eq!(
        result.unwrap_err().reason,
        "the user stopped this operation"
    );
    assert!(token.is_cancelled());
}

#[test]
fn handle_clones_do_not_extend_child_or_root_execution_lifetimes() -> Result<(), CancellationError>
{
    let token = CancellationToken::new();

    let saved_owner = token.with_scope(|owner| {
        let saved_child = owner.with_scope(|child| Ok::<_, CancellationError>(child.clone()))?;
        let closed_child: Result<(), CancellationError> = saved_child.with_scope(|_| {
            panic!("a completed child must not admit work through a saved handle");
        });
        assert!(closed_child.is_err());

        // The parent remains open; finishing its child does not close it.
        owner.with_scope(|next| next.bail_if_cancelled())?;
        Ok::<_, CancellationError>(owner.clone())
    })?;

    let closed_owner: Result<(), CancellationError> = saved_owner.with_scope(|_| {
        panic!("a completed root must not admit work through a saved handle");
    });
    assert!(closed_owner.is_err());
    assert!(!token.is_cancelled());
    Ok(())
}

#[test]
fn a_completed_callback_preserves_its_own_error_when_cancellation_is_requested() {
    let token = CancellationToken::new();

    let result: Result<(), eyre::Report> = token.with_scope(|scope| {
        scope.request_cancel("arrived alongside the callback failure");
        Err(eyre::eyre!("original callback failure"))
    });

    let error = result.unwrap_err();
    assert_eq!(error.to_string(), "original callback failure");
    assert!(error.downcast_ref::<CancellationError>().is_none());
}

#[test]
fn unbound_request_scopes_cannot_admit_work_or_share_live_intent() {
    let unbound = teamy_cancellation::CancellationScope::unbound();
    let independent = teamy_cancellation::CancellationScope::default();
    assert!(unbound.bail_if_cancelled().is_err());
    let result: Result<(), CancellationError> = unbound.with_scope(|_| {
        panic!("an unbound request scope must not execute work");
    });
    assert!(result.is_err());
    unbound.request_cancel("inert input state");
    assert!(!independent.is_cancelled());
    #[cfg(feature = "deferral")]
    {
        let result: Result<(), CancellationError> =
            unbound.finish_required_sync("not admitted", |_| {
                panic!("required cleanup must not open an unbound scope");
            });
        assert!(result.is_err());
    }
}
