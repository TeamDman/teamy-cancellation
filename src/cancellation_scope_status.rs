/// Lifecycle of a future-owned cancellation scope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CancellationScopeStatus {
    Pending,
    Active,
    Closed,
}
