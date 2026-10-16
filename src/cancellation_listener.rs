use crate::cancellation_listener_state::CancellationListenerState;
use crate::cancellation_registration::CancellationRegistration;
use std::fmt;
use std::sync::Arc;

/// A cancellation notification registration owned by its listening operation.
///
/// Dropping this handle unregisters it. A callback already dispatched on another
/// thread may still finish. Callbacks run outside the token's locks and may query
/// the token or register other listeners.
#[must_use = "dropping the listener unregisters its callback"]
pub struct CancellationListener {
    pub(crate) state: Arc<CancellationListenerState>,
    pub(crate) registrations: Vec<CancellationRegistration>,
}

impl Drop for CancellationListener {
    fn drop(&mut self) {
        self.state.deactivate();
        self.registrations.clear();
    }
}

impl fmt::Debug for CancellationListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancellationListener")
            .field("notified", &self.state.is_notified())
            .finish_non_exhaustive()
    }
}
