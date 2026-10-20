use crate::CancellationError;
use eyre::Report;
use std::error::Error;
#[cfg(feature = "io")]
use std::io;

/// Recognize cooperative cancellation while retaining an [`eyre::Report`].
///
/// Native typed report downcasts take precedence, including cancellation stored
/// as typed context. Otherwise these helpers follow exposed error sources. With
/// `io`, they also inspect a directly downcastable I/O context and enter boxed
/// I/O payloads instead of skipping them. Hidden error graphs are not inspected.
pub trait CancellationReportExt {
    /// Borrow the cancellation recognized by native downcasting or exposed sources.
    #[must_use]
    fn as_cancellation(&self) -> Option<&CancellationError>;

    /// Whether this report exposes a typed cooperative cancellation.
    #[must_use]
    fn is_cancellation(&self) -> bool;

    /// Make recognized cancellation available to native report downcasting.
    ///
    /// A report already downcastable to [`CancellationError`] is returned
    /// unchanged. Otherwise a recognized cancellation is cloned into an outer
    /// typed context, preserving the original report, handler, and source chain.
    /// Its cancellation reason becomes the outer display message. Unrelated
    /// reports are returned unchanged.
    #[must_use]
    fn normalize_cancellation(self) -> Self
    where
        Self: Sized;
}

impl CancellationReportExt for Report {
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

    fn is_cancellation(&self) -> bool {
        self.as_cancellation().is_some()
    }

    fn normalize_cancellation(self) -> Self {
        if self.downcast_ref::<CancellationError>().is_some() {
            return self;
        }
        match self.as_cancellation().cloned() {
            Some(cancellation) => self.wrap_err(cancellation),
            None => self,
        }
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
