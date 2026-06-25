use std::time::Duration;
use std::time::Instant;

#[derive(Debug)]
#[cfg_attr(feature = "facet", derive(facet::Facet))]
#[cfg_attr(feature = "facet", facet(opaque))]
pub struct CancellationState {
    cancelled: bool,
    last_ctrl_c: Option<Instant>,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "facet", derive(facet::Facet))]
pub enum CtrlCAction {
    RequestGracefulShutdown,
    ForceExit,
}

impl CancellationState {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            cancelled: false,
            last_ctrl_c: None,
        }
    }

    pub fn record_ctrl_c(&mut self, now: Instant) -> CtrlCAction {
        let force_exit = self
            .last_ctrl_c
            .is_some_and(|last| now.duration_since(last) <= Duration::from_secs(1));
        self.last_ctrl_c = Some(now);
        self.cancelled = true;
        if force_exit {
            CtrlCAction::ForceExit
        } else {
            CtrlCAction::RequestGracefulShutdown
        }
    }

    #[cfg(test)]
    #[must_use]
    pub const fn is_cancelled(&self) -> bool {
        self.cancelled
    }
}

impl Default for CancellationState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::CancellationState;
    use super::CtrlCAction;
    use std::time::Duration;
    use std::time::Instant;

    #[test]
    fn first_ctrl_c_requests_graceful_shutdown() {
        let mut state = CancellationState::new();

        let action = state.record_ctrl_c(Instant::now());

        assert_eq!(action, CtrlCAction::RequestGracefulShutdown);
        assert!(state.is_cancelled());
    }

    #[test]
    fn second_fast_ctrl_c_forces_exit() {
        let mut state = CancellationState::new();
        let now = Instant::now();
        state.record_ctrl_c(now);

        let action = state.record_ctrl_c(now + Duration::from_millis(250));

        assert_eq!(action, CtrlCAction::ForceExit);
    }

    #[test]
    fn second_slow_ctrl_c_stays_graceful() {
        let mut state = CancellationState::new();
        let now = Instant::now();
        state.record_ctrl_c(now);

        let action = state.record_ctrl_c(now + Duration::from_secs(2));

        assert_eq!(action, CtrlCAction::RequestGracefulShutdown);
    }
}
