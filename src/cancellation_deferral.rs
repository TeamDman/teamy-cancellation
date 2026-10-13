use crate::CancellationListener;
use crate::CancellationToken;
use crate::cancellation_deferral_state::CancellationDeferralState;
use std::fmt;
use std::panic::Location;
use std::sync::Arc;

/// Postpones cooperative cancellation while an operation finishes required work.
///
/// This is an operation-local cancellation policy, not a lock or an exclusive
/// section. The token remains cancelled, and unrelated operation tokens remain
/// responsive. Each explanation is acknowledged once when cancellation arrives.
///
/// Dropping the guard resumes cancellation checkpoints. With the `async`
/// feature, it also wakes cancellable futures. Dropping an operation's owner,
/// task aborts, and process termination can still interrupt this work; let the
/// operation finish before dropping its owner.
#[must_use = "Keep the guard alive until the required work has finished"]
pub struct CancellationDeferral {
    state: Arc<CancellationDeferralState>,
    registration: usize,
    listener: Option<CancellationListener>,
    explanation: Arc<str>,
    caller: &'static Location<'static>,
}

impl CancellationToken {
    /// Postpones cancellation in this token's operation scope until the guard
    /// is dropped.
    ///
    /// Call this before starting a mutation whose outcome and associated cleanup
    /// must be observed together. Nested guards each retain their explanation;
    /// cancellation resumes after the last guard is dropped. A guard may also
    /// be entered after cancellation to finish already-required cleanup.
    ///
    /// Use [`Self::for_operation`] for independent operations. Clones within an
    /// operation share its deferrals. With the `tracing` feature, this section
    /// emits an acknowledgment containing its explanation when cancellation is
    /// requested, including when cancellation was already requested on entry.
    #[track_caller]
    pub fn defer_cancellation(&self, explanation: impl Into<String>) -> CancellationDeferral {
        let caller = Location::caller();
        let explanation = Arc::<str>::from(explanation.into());
        let registration = self.deferral_state.enter();
        let listener_explanation = Arc::clone(&explanation);
        let mut deferral = CancellationDeferral {
            state: Arc::clone(&self.deferral_state),
            registration,
            listener: None,
            explanation,
            caller,
        };
        let listener = self.register_listener(move |reason| {
            #[cfg(feature = "tracing")]
            tracing::info!(
                reason,
                explanation = listener_explanation.as_ref(),
                caller_file = caller.file(),
                caller_line = caller.line(),
                caller_column = caller.column(),
                "Cancellation acknowledged; postponing cancellation until this section finishes"
            );
            #[cfg(not(feature = "tracing"))]
            let _ = (reason, &listener_explanation);
        });
        deferral.listener = Some(listener);
        deferral
    }
}

impl Drop for CancellationDeferral {
    fn drop(&mut self) {
        // Unregister first so a completed section no longer reports a delay.
        self.listener.take();
        self.state.exit(self.registration);
    }
}

impl fmt::Debug for CancellationDeferral {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancellationDeferral")
            .field("explanation", &self.explanation)
            .field("caller", &self.caller)
            .finish_non_exhaustive()
    }
}
