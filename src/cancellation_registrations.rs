use crate::cancellation_listener_state::CancellationListenerState;
#[cfg(feature = "async")]
use crate::cancellation_waker::CancellationWaker;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Weak;

/// Owns registration identities; weak entries do not keep listeners alive.
#[derive(Default)]
pub(crate) struct CancellationRegistrations {
    next_id: u64,
    listeners: BTreeMap<u64, Weak<CancellationListenerState>>,
    #[cfg(feature = "async")]
    waiters: BTreeMap<u64, Weak<CancellationWaker>>,
}

impl CancellationRegistrations {
    fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("cancellation registration IDs exhausted");
        id
    }

    pub(crate) fn add_listener(&mut self, listener: &Arc<CancellationListenerState>) -> u64 {
        let id = self.next_id();
        self.listeners.insert(id, Arc::downgrade(listener));
        id
    }

    #[cfg(feature = "async")]
    pub(crate) fn add_waiter(&mut self, waiter: &Arc<CancellationWaker>) -> u64 {
        let id = self.next_id();
        self.waiters.insert(id, Arc::downgrade(waiter));
        id
    }

    pub(crate) fn remove(&mut self, id: u64) {
        self.listeners.remove(&id);
        #[cfg(feature = "async")]
        self.waiters.remove(&id);
    }

    pub(crate) fn listeners(&self) -> Vec<Arc<CancellationListenerState>> {
        self.listeners.values().filter_map(Weak::upgrade).collect()
    }

    #[cfg(feature = "async")]
    pub(crate) fn waiters(&self) -> Vec<Arc<CancellationWaker>> {
        self.waiters.values().filter_map(Weak::upgrade).collect()
    }
}
