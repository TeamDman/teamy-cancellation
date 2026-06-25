use crate::CancellationToken;
use crate::cancellation_state::CancellationState;
use crate::cancellation_state::CtrlCAction;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
#[cfg_attr(feature = "facet", derive(facet::Facet))]
pub struct CtrlCHandler {
    /// Whether the handler should print `^C` to stderr when Ctrl+C is received.
    pub should_eprintln_on_ctrl_c: bool,
}

impl CtrlCHandler {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            should_eprintln_on_ctrl_c: true,
        }
    }

    /// Create the process cancellation token and install the process-wide Ctrl+C
    /// handler.
    ///
    /// # Errors
    ///
    /// Returns an error if the platform handler cannot be registered.
    pub fn install(self) -> eyre::Result<CancellationToken> {
        let cancellation_token = CancellationToken::new();
        let handler_token = cancellation_token.clone();
        let handler_state = Arc::new(Mutex::new(CancellationState::new()));
        let ctrlc_state = Arc::clone(&handler_state);
        ctrlc::set_handler(move || handle_ctrl_c(self, &handler_token, &ctrlc_state))
            .map_err(|error| eyre::eyre!(error))?;
        Ok(cancellation_token)
    }
}

impl Default for CtrlCHandler {
    fn default() -> Self {
        Self::new()
    }
}

fn handle_ctrl_c(
    handler: CtrlCHandler,
    cancellation_token: &CancellationToken,
    state: &Mutex<CancellationState>,
) {
    if handler.should_eprintln_on_ctrl_c {
        eprintln!("\x1b[31m^C\x1b[0m");
    }
    let action = {
        let mut state = state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.record_ctrl_c(Instant::now())
    };
    cancellation_token.request_cancel("Operation cancelled by Ctrl+C");
    match action {
        CtrlCAction::RequestGracefulShutdown => {
            #[cfg(feature = "tracing")]
            tracing::warn!("Ctrl+C received; graceful shutdown requested");
        }
        CtrlCAction::ForceExit => {
            #[cfg(feature = "tracing")]
            tracing::warn!("Second Ctrl+C received; forcing exit");
            std::process::exit(130);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CtrlCHandler;

    #[test]
    fn default_handler_prints_ctrl_c() {
        assert!(CtrlCHandler::default().should_eprintln_on_ctrl_c);
    }
}
