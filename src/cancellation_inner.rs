#[cfg(feature = "inheritance")]
use crate::CancellationToken;
use crate::cancellation_registrations::CancellationRegistrations;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

pub(crate) type CancelHook = dyn Fn(&str, bool) + Send + Sync + 'static;

/// Shared requested-cancellation state, independent of operation deferrals.
#[derive(Default)]
pub(crate) struct CancellationInner {
    pub(crate) cancelled: AtomicBool,
    pub(crate) reason: Mutex<Option<String>>,
    pub(crate) on_cancel_request: Option<Arc<CancelHook>>,
    pub(crate) registrations: Mutex<CancellationRegistrations>,
    #[cfg(feature = "inheritance")]
    pub(crate) parent: Option<CancellationToken>,
}
