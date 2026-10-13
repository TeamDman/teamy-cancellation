use crate::CancellationToken;
use std::fmt;

/// A cooperative stop that retains the original cancellation reason.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CancellationError {
    pub reason: String,
}

impl CancellationError {
    /// Creates a cancellation outcome with the caller's reason.
    #[must_use]
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }

    pub(crate) fn from_token(token: &CancellationToken) -> Self {
        Self::new(
            token
                .cancellation_reason()
                .unwrap_or_else(|| String::from("Operation cancelled")),
        )
    }
}

impl fmt::Display for CancellationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.reason)
    }
}

impl std::error::Error for CancellationError {}
