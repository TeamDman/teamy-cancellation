use crate::cancellation_deferral_storage::CancellationDeferralStorage;
use std::sync::Mutex;
#[cfg(feature = "async")]
use std::task::Waker;

/// Coordinates cancellation deferrals for one operation.
///
/// Token clones share this state. Independent operations receive separate state
/// so finishing one operation's required work does not postpone another's stop.
#[derive(Debug, Default)]
pub(crate) struct CancellationDeferralState {
    storage: Mutex<CancellationDeferralStorage>,
}

impl CancellationDeferralState {
    pub(crate) fn is_deferred(&self) -> bool {
        !self
            .storage
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .active
            .is_empty()
    }

    pub(crate) fn enter(&self) -> usize {
        let mut storage = self
            .storage
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let id = storage.allocate_id();
        storage.active.insert(id);
        id
    }

    pub(crate) fn exit(&self, id: usize) {
        let mut storage = self
            .storage
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        storage.active.remove(&id);
        #[cfg(feature = "async")]
        let waiters = if storage.active.is_empty() {
            storage.waiters.values().cloned().collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        // Wake outside the lock: a custom executor may immediately poll again.
        drop(storage);
        #[cfg(feature = "async")]
        for waiter in waiters {
            waiter.wake();
        }
    }

    #[cfg(feature = "async")]
    pub(crate) fn register_waker(&self, existing: Option<usize>, waker: &Waker) -> usize {
        let mut storage = self
            .storage
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let id = existing.unwrap_or_else(|| storage.allocate_id());
        if !storage
            .waiters
            .get(&id)
            .is_some_and(|registered| registered.will_wake(waker))
        {
            storage.waiters.insert(id, waker.clone());
        }
        id
    }

    #[cfg(feature = "async")]
    pub(crate) fn unregister_waker(&self, id: usize) {
        self.storage
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .waiters
            .remove(&id);
    }
}
