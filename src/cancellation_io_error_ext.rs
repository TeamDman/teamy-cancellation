use crate::CancellationError;
use std::io;

/// Access a cancellation stored directly inside an [`io::Error`].
///
/// These methods inspect only the boxed payload, without searching its error
/// source chain. A wrapper whose source is a [`CancellationError`] is not itself
/// a cancellation payload.
///
/// When owning the I/O error, prefer [`Self::into_cancellation`] to moving a
/// borrowed payload through a temporary or default placeholder. Extraction moves
/// the original cancellation without cloning its reason. Successful extraction
/// removes the I/O wrapper, including its error kind.
pub trait CancellationIoErrorExt {
    /// Borrow the direct cancellation payload, when present.
    #[must_use]
    fn as_cancellation(&self) -> Option<&CancellationError>;

    /// Mutably borrow the direct cancellation payload, when present.
    #[must_use]
    fn as_cancellation_mut(&mut self) -> Option<&mut CancellationError>;

    /// Move the direct cancellation payload out of its I/O wrapper.
    ///
    /// # Errors
    /// Returns the original I/O error unchanged when its direct payload is not
    /// a [`CancellationError`], retaining its kind, OS code, and boxed payload.
    fn into_cancellation(self) -> Result<CancellationError, Self>
    where
        Self: Sized;
}

impl CancellationIoErrorExt for io::Error {
    fn as_cancellation(&self) -> Option<&CancellationError> {
        self.get_ref()
            .and_then(|payload| payload.downcast_ref::<CancellationError>())
    }

    fn as_cancellation_mut(&mut self) -> Option<&mut CancellationError> {
        self.get_mut()
            .and_then(|payload| payload.downcast_mut::<CancellationError>())
    }

    fn into_cancellation(self) -> Result<CancellationError, Self> {
        self.downcast::<CancellationError>()
    }
}
