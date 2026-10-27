use crate::CancellationError;
use eyre::Report;
use std::error::Error;
#[cfg(feature = "io")]
use std::io;

/// Recognize cooperative cancellation in reports and I/O errors.
///
/// Native typed report downcasts take precedence, including cancellation stored
/// as typed context. Otherwise these helpers follow exposed error sources. With
/// `io`, they also inspect a directly downcastable I/O context and enter boxed
/// I/O payloads instead of skipping them. Hidden error graphs are not inspected.
pub trait CancellationErrorExt {
    /// Borrow the cancellation recognized by native downcasting or exposed sources.
    #[must_use]
    fn as_cancellation(&self) -> Option<&CancellationError>;

    /// Mutably borrow a natively downcastable cancellation.
    ///
    /// Reports inspect typed contexts; I/O errors inspect their direct payload.
    /// This does not traverse sources, which expose only shared references.
    #[must_use]
    fn as_cancellation_mut(&mut self) -> Option<&mut CancellationError>;

    /// Extract a natively downcastable cancellation without cloning its reason.
    ///
    /// Reports use native downcasting, discarding surrounding report context on
    /// success. I/O errors extract only their direct payload, discarding the I/O
    /// kind on success. Source chains are not searched.
    ///
    /// # Errors
    /// Returns the original error unchanged if native downcasting fails, even
    /// when [`Self::as_cancellation`] recognizes cancellation in a source.
    fn into_cancellation(self) -> Result<CancellationError, Self>
    where
        Self: Sized;

    /// Whether this error exposes a typed cooperative cancellation.
    #[must_use]
    fn is_cancellation(&self) -> bool {
        self.as_cancellation().is_some()
    }
}

impl CancellationErrorExt for Report {
    fn as_cancellation(&self) -> Option<&CancellationError> {
        if let Some(cancellation) = self.downcast_ref::<CancellationError>() {
            return Some(cancellation);
        }
        #[cfg(feature = "io")]
        if let Some(error) = self.downcast_ref::<io::Error>()
            && let Some(cancellation) = cancellation_in_chain(error)
        {
            return Some(cancellation);
        }
        cancellation_in_chain(self.as_ref())
    }

    fn as_cancellation_mut(&mut self) -> Option<&mut CancellationError> {
        self.downcast_mut::<CancellationError>()
    }

    fn into_cancellation(self) -> Result<CancellationError, Self> {
        self.downcast::<CancellationError>()
    }
}

#[cfg(feature = "io")]
impl CancellationErrorExt for io::Error {
    fn as_cancellation(&self) -> Option<&CancellationError> {
        cancellation_in_chain(self)
    }

    fn as_cancellation_mut(&mut self) -> Option<&mut CancellationError> {
        self.get_mut()
            .and_then(|payload| payload.downcast_mut::<CancellationError>())
    }

    fn into_cancellation(self) -> Result<CancellationError, Self> {
        self.downcast::<CancellationError>()
    }
}

fn cancellation_in_chain<'error>(
    mut error: &'error (dyn Error + 'static),
) -> Option<&'error CancellationError> {
    loop {
        if let Some(cancellation) = error.downcast_ref::<CancellationError>() {
            return Some(cancellation);
        }
        #[cfg(feature = "io")]
        if let Some(io_error) = error.downcast_ref::<io::Error>()
            && let Some(payload) = io_error.get_ref()
        {
            error = payload;
            continue;
        }
        error = error.source()?;
    }
}
