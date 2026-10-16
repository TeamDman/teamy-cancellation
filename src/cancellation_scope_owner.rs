#[cfg(feature = "deferral")]
use crate::CancellationListener;
use crate::CancellationScope;
use crate::cancellation_scope_kind::CancellationScopeKind;
use std::panic::Location;

/// Closes one callback/future-owned scope independently of surviving handles.
#[derive(Debug)]
pub(crate) struct CancellationScopeOwner {
    scope: CancellationScope,
    #[cfg(feature = "deferral")]
    acknowledgment: Option<CancellationListener>,
}

impl CancellationScopeOwner {
    pub(crate) fn new(scope: CancellationScope) -> Self {
        Self {
            scope,
            #[cfg(feature = "deferral")]
            acknowledgment: None,
        }
    }

    pub(crate) fn scope(&self) -> &CancellationScope {
        &self.scope
    }

    pub(crate) fn start(
        &mut self,
        kind: CancellationScopeKind,
        explanation: Option<String>,
        caller: &'static Location<'static>,
    ) -> bool {
        if !self.scope.state.admit(&self.scope.token, kind) {
            return false;
        }
        #[cfg(feature = "deferral")]
        if let Some(explanation) = explanation {
            self.acknowledgment = Some(self.scope.register_listener(move |reason| {
                #[cfg(feature = "tracing")]
                tracing::info!(
                    reason,
                    explanation,
                    caller_file = caller.file(),
                    caller_line = caller.line(),
                    caller_column = caller.column(),
                    "Cancellation acknowledged; postponing cancellation until this scope finishes"
                );
                #[cfg(not(feature = "tracing"))]
                let _ = (reason, &explanation, caller);
            }));
        }
        #[cfg(not(feature = "deferral"))]
        let _ = (explanation, caller);
        true
    }

    pub(crate) fn finish(&mut self) {
        #[cfg(feature = "deferral")]
        self.acknowledgment.take();
        self.scope.state.close();
    }
}

impl Drop for CancellationScopeOwner {
    fn drop(&mut self) {
        self.finish();
    }
}
