use crate::CancellationScope;
use crate::CancellationWriteExt;
use crate::cancellation_write_ext::WRITE_CHUNK_SIZE;
use std::io;
use std::io::Write;

/// A borrowed synchronous writer with scope-aware output checkpoints.
///
/// Each underlying write receives at most 64 KiB, allowing large-buffer
/// serializers to observe cancellation as they progress. Interrupted writes
/// retried by `write_all` check cancellation again. Deferral on the supplied
/// scope is respected; the adapter does not create an execution owner.
///
/// Already-written output is retained. Synchronous cancellation cannot preempt
/// an underlying write or flush that is blocked. Dropping this adapter does not
/// flush the writer.
#[derive(Debug)]
pub struct CancellableWriter<'a, W: ?Sized> {
    writer: &'a mut W,
    cancellation: &'a CancellationScope,
}

impl<'a, W: Write + ?Sized> CancellableWriter<'a, W> {
    /// Borrows a writer and the existing scope of its executing operation.
    #[must_use]
    pub fn new(writer: &'a mut W, cancellation: &'a CancellationScope) -> Self {
        Self {
            writer,
            cancellation,
        }
    }
}

impl<W: Write + ?Sized> Write for CancellableWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.cancellation
            .bail_if_cancelled()
            .map_err(io::Error::other)?;
        self.writer.write(&buf[..buf.len().min(WRITE_CHUNK_SIZE)])
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.writer
            .write_all_with_cancellation(buf, self.cancellation)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush_with_cancellation(self.cancellation)
    }
}
