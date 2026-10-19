#![cfg(feature = "io")]

use std::io;
use std::io::Write;
use teamy_cancellation::CancellableWriter;
use teamy_cancellation::CancellationError;
use teamy_cancellation::CancellationScope;
use teamy_cancellation::CancellationToken;
use teamy_cancellation::CancellationWriteExt;

#[derive(Default)]
struct RecordingWriter {
    bytes: Vec<u8>,
    write_calls: usize,
    flush_calls: usize,
    largest_request: usize,
    short_write_limit: Option<usize>,
    interrupted_writes: usize,
    cancel_on_call: Option<(CancellationToken, usize)>,
}

impl Write for RecordingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.write_calls += 1;
        self.largest_request = self.largest_request.max(buf.len());
        if let Some((token, call)) = &self.cancel_on_call
            && self.write_calls == *call
        {
            token.request_cancel("stop the output");
        }
        if self.interrupted_writes > 0 {
            self.interrupted_writes -= 1;
            return Err(io::ErrorKind::Interrupted.into());
        }
        let written = buf.len().min(self.short_write_limit.unwrap_or(buf.len()));
        self.bytes.extend_from_slice(&buf[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flush_calls += 1;
        Ok(())
    }
}

fn assert_cancellation(error: &io::Error, expected_reason: &str) {
    assert_eq!(error.kind(), io::ErrorKind::Other);
    let cancellation = error
        .get_ref()
        .and_then(|source| source.downcast_ref::<CancellationError>())
        .expect("the I/O error must retain typed cancellation");
    assert_eq!(cancellation.reason, expected_reason);
}

#[test]
fn short_writes_keep_the_prefix_and_stop_after_a_request() -> Result<(), CancellationError> {
    let token = CancellationToken::new();
    let mut writer = RecordingWriter {
        short_write_limit: Some(3),
        cancel_on_call: Some((token.clone(), 2)),
        ..RecordingWriter::default()
    };
    token.with_scope(|scope| {
        let dynamic_writer: &mut dyn Write = &mut writer;
        let error = dynamic_writer
            .write_all_with_cancellation(b"a long output record", &scope)
            .unwrap_err();
        assert_cancellation(&error, "stop the output");
        assert_eq!(writer.bytes, b"a long");
        assert_eq!(writer.write_calls, 2);
        Ok(())
    })
}

#[test]
fn the_adapter_bounds_large_requests_before_the_next_checkpoint() -> Result<(), CancellationError> {
    let token = CancellationToken::new();
    let mut writer = RecordingWriter {
        cancel_on_call: Some((token.clone(), 1)),
        ..RecordingWriter::default()
    };
    let input = vec![b'x'; 200_000];
    token.with_scope(|scope| {
        let mut cancellable = CancellableWriter::new(&mut writer, &scope);
        let error = cancellable.write_all(&input).unwrap_err();
        assert_cancellation(&error, "stop the output");
        assert_eq!(writer.bytes.len(), 64 * 1024);
        assert_eq!(writer.largest_request, 64 * 1024);
        assert_eq!(writer.write_calls, 1);
        Ok(())
    })
}

#[test]
fn repeated_interruptions_recheck_cancellation_instead_of_retrying_forever()
-> Result<(), CancellationError> {
    let token = CancellationToken::new();
    let mut writer = RecordingWriter {
        interrupted_writes: usize::MAX,
        cancel_on_call: Some((token.clone(), 3)),
        ..RecordingWriter::default()
    };
    token.with_scope(|scope| {
        let error = writer
            .write_all_with_cancellation(b"never written", &scope)
            .unwrap_err();
        assert_cancellation(&error, "stop the output");
        assert_eq!(writer.write_calls, 3);
        assert!(writer.bytes.is_empty());
        Ok(())
    })
}

#[test]
fn temporary_interruptions_and_partial_writes_preserve_all_bytes() -> Result<(), CancellationError>
{
    let token = CancellationToken::new();
    let mut writer = RecordingWriter {
        short_write_limit: Some(2),
        interrupted_writes: 2,
        ..RecordingWriter::default()
    };
    token.with_scope(|scope| {
        writer
            .write_all_with_cancellation(b"a complete record", &scope)
            .unwrap();
        assert_eq!(writer.bytes, b"a complete record");
        assert_eq!(writer.interrupted_writes, 0);
        Ok(())
    })
}

#[test]
fn a_writer_that_makes_no_progress_returns_write_zero() -> Result<(), CancellationError> {
    let token = CancellationToken::new();
    let mut writer = RecordingWriter {
        short_write_limit: Some(0),
        ..RecordingWriter::default()
    };
    token.with_scope(|scope| {
        let error = writer
            .write_all_with_cancellation(b"cannot progress", &scope)
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::WriteZero);
        assert_eq!(writer.write_calls, 1);
        Ok(())
    })
}

#[derive(Debug)]
struct OriginalFailure;

impl std::fmt::Display for OriginalFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the original writer failure")
    }
}

impl std::error::Error for OriginalFailure {}

struct FailingWriter;

impl Write for FailingWriter {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, OriginalFailure))
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            OriginalFailure,
        ))
    }
}

#[test]
fn original_write_and_flush_errors_keep_their_kind_and_source() -> Result<(), CancellationError> {
    CancellationToken::new().with_scope(|scope| {
        let mut failing = FailingWriter;
        let mut writer = CancellableWriter::new(&mut failing, &scope);
        for (error, kind) in [
            (
                writer.write_all(b"record").unwrap_err(),
                io::ErrorKind::BrokenPipe,
            ),
            (writer.flush().unwrap_err(), io::ErrorKind::PermissionDenied),
        ] {
            assert_eq!(error.kind(), kind);
            assert!(error.get_ref().unwrap().is::<OriginalFailure>());
        }
        Ok(())
    })
}

#[test]
fn closed_scopes_reject_writes_empty_buffers_and_flushes_without_io() {
    let mut writer = RecordingWriter::default();
    let scope = CancellationScope::unbound();
    let mut cancellable = CancellableWriter::new(&mut writer, &scope);
    assert_cancellation(
        &cancellable.write(b"record").unwrap_err(),
        "Cancellation scope is closed",
    );
    assert_cancellation(
        &cancellable.write_all(b"").unwrap_err(),
        "Cancellation scope is closed",
    );
    assert_cancellation(
        &cancellable.flush().unwrap_err(),
        "Cancellation scope is closed",
    );
    assert_eq!(writer.write_calls, 0);
    assert_eq!(writer.flush_calls, 0);
}

#[test]
fn cancellation_between_records_prevents_the_next_write_and_flush() -> Result<(), CancellationError>
{
    let token = CancellationToken::new();
    let mut writer = RecordingWriter::default();
    token.with_scope(|scope| {
        let mut cancellable = CancellableWriter::new(&mut writer, &scope);
        writeln!(cancellable, "first record").unwrap();
        token.request_cancel("no more records");
        assert_cancellation(
            &cancellable.write_all(b"second record").unwrap_err(),
            "no more records",
        );
        assert_cancellation(&cancellable.flush().unwrap_err(), "no more records");
        assert_eq!(writer.bytes, b"first record\n");
        assert_eq!(writer.flush_calls, 0);
        Ok(())
    })
}

#[cfg(feature = "deferral")]
#[test]
fn a_protected_scope_finishes_required_output_and_flush_after_cancellation()
-> Result<(), CancellationError> {
    let token = CancellationToken::new();
    let mut writer = RecordingWriter {
        short_write_limit: Some(1000),
        cancel_on_call: Some((token.clone(), 1)),
        ..RecordingWriter::default()
    };
    let input = vec![b'x'; 200_000];
    token.with_scope(|scope| {
        scope.with_deferral("finish this required output", |protected| {
            let mut cancellable = CancellableWriter::new(&mut writer, &protected);
            cancellable.write_all(&input).unwrap();
            cancellable.flush().unwrap();
            assert!(protected.is_cancelled());
            Ok::<_, CancellationError>(())
        })?;
        assert_eq!(writer.bytes, input);
        assert_eq!(writer.flush_calls, 1);
        assert!(scope.bail_if_cancelled().is_err());
        Ok(())
    })
}
