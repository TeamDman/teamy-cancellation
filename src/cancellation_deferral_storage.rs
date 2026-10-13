#[cfg(feature = "async")]
use std::collections::BTreeMap;
use std::collections::BTreeSet;
#[cfg(feature = "async")]
use std::task::Waker;

/// The active scopes and suspended callers belonging to one operation.
#[derive(Debug, Default)]
pub(crate) struct CancellationDeferralStorage {
    pub(crate) next_id: usize,
    pub(crate) active: BTreeSet<usize>,
    #[cfg(feature = "async")]
    pub(crate) waiters: BTreeMap<usize, Waker>,
}

impl CancellationDeferralStorage {
    pub(crate) fn allocate_id(&mut self) -> usize {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("Cancellation registration ID exhausted");
        id
    }
}
