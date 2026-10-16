use crate::cancellation_scope_status::CancellationScopeStatus;
#[cfg(feature = "async")]
use std::task::Waker;

/// Tree bookkeeping for one owner and its active completion obligations.
#[derive(Debug)]
pub(crate) struct CancellationScopeRecord {
    pub(crate) parent: Option<usize>,
    pub(crate) status: CancellationScopeStatus,
    /// Protection directly owned by this scope, excluding its descendants.
    pub(crate) protected: bool,
    /// Active protections at this node and anywhere beneath it.
    pub(crate) retained: usize,
    #[cfg(feature = "async")]
    pub(crate) waker: Option<Waker>,
}

impl CancellationScopeRecord {
    pub(crate) fn new(parent: Option<usize>) -> Self {
        Self {
            parent,
            status: CancellationScopeStatus::Pending,
            protected: false,
            retained: 0,
            #[cfg(feature = "async")]
            waker: None,
        }
    }
}
