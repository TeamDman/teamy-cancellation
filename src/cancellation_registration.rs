use crate::cancellation_inner::CancellationInner;
use std::fmt;
use std::sync::Weak;

/// Removes one node's listener or waiter entry when its owner finishes.
pub(crate) struct CancellationRegistration {
    pub(crate) inner: Weak<CancellationInner>,
    pub(crate) id: u64,
}

impl Drop for CancellationRegistration {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.upgrade() {
            inner
                .registrations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(self.id);
        }
    }
}

impl fmt::Debug for CancellationRegistration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancellationRegistration")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}
