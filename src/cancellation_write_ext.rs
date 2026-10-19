use crate::CancellationScope;
use std::io;
use std::io::Write;

pub(crate) const WRITE_CHUNK_SIZE: usize = 64 * 1024;

/// Synchronous output checkpoints that honor a supplied execution scope.
///
/// Cancellation is returned as [`io::ErrorKind::Other`] containing a
/// [`crate::CancellationError`], rather than `Interrupted`, which normal
/// `write_all` implementations retry. Already-written bytes remain written.
/// A checkpoint cannot interrupt an underlying write or flush that is blocked.
pub trait CancellationWriteExt: Write {
    /// Writes the complete buffer, checking the scope before each bounded write.
    ///
    /// Individual write requests are limited to 64 KiB. Interrupted writes are
    /// retried after another cancellation checkpoint. An empty buffer still
    /// checks cancellation, and scope deferral is respected.
    ///
    /// # Errors
    /// Returns cancellation, an original writer error, or [`io::ErrorKind::WriteZero`]
    /// if the writer cannot make progress while bytes remain.
    fn write_all_with_cancellation(
        &mut self,
        mut buf: &[u8],
        cancellation: &CancellationScope,
    ) -> io::Result<()> {
        loop {
            cancellation.bail_if_cancelled().map_err(io::Error::other)?;
            if buf.is_empty() {
                return Ok(());
            }
            let chunk = &buf[..buf.len().min(WRITE_CHUNK_SIZE)];
            match self.write(chunk) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "failed to write the complete buffer",
                    ));
                }
                Ok(written) => buf = &buf[written..],
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
    }

    /// Flushes after checking the supplied execution scope.
    ///
    /// # Errors
    /// Returns cancellation before flushing or the writer's original error.
    fn flush_with_cancellation(&mut self, cancellation: &CancellationScope) -> io::Result<()> {
        cancellation.bail_if_cancelled().map_err(io::Error::other)?;
        self.flush()
    }
}

impl<W: Write + ?Sized> CancellationWriteExt for W {}
