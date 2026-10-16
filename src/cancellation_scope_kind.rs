/// Admission policy and protection owned by an execution scope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CancellationScopeKind {
    Cancellable,
    #[cfg(feature = "deferral")]
    Deferred,
    #[cfg(feature = "deferral")]
    Required,
}

impl CancellationScopeKind {
    pub(crate) fn accepts_requested_cancellation(self) -> bool {
        match self {
            Self::Cancellable => false,
            #[cfg(feature = "deferral")]
            Self::Deferred => false,
            #[cfg(feature = "deferral")]
            Self::Required => true,
        }
    }

    pub(crate) fn is_protected(self) -> bool {
        match self {
            Self::Cancellable => false,
            #[cfg(feature = "deferral")]
            Self::Deferred | Self::Required => true,
        }
    }
}
