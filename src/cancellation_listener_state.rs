use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

/// Delivers one notification across registrations in a token's ancestor chain.
pub(crate) struct CancellationListenerState {
    callback: Box<dyn Fn(&str) + Send + Sync + 'static>,
    notified: AtomicBool,
    active: AtomicBool,
}

impl CancellationListenerState {
    pub(crate) fn new(callback: impl Fn(&str) + Send + Sync + 'static) -> Self {
        Self {
            callback: Box::new(callback),
            notified: AtomicBool::new(false),
            active: AtomicBool::new(true),
        }
    }

    pub(crate) fn notify(&self, reason: &str) {
        if self.active.load(Ordering::Acquire) && !self.notified.swap(true, Ordering::AcqRel) {
            (self.callback)(reason);
        }
    }

    pub(crate) fn deactivate(&self) {
        self.active.store(false, Ordering::Release);
    }

    pub(crate) fn is_notified(&self) -> bool {
        self.notified.load(Ordering::Acquire)
    }
}
