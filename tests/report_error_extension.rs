use eyre::Report;
use std::error::Error;
use std::fmt;
#[cfg(feature = "io")]
use std::io;
use std::ptr;
use teamy_cancellation::CancellationError;
#[cfg(feature = "io")]
use teamy_cancellation::CancellationIoErrorExt;
use teamy_cancellation::CancellationReportExt;

#[derive(Debug)]
struct SourcedFailure {
    message: &'static str,
    source: Box<dyn Error + Send + Sync>,
}

impl fmt::Display for SourcedFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message)
    }
}

impl Error for SourcedFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.source.as_ref())
    }
}

#[derive(Debug)]
struct MessageOnlyFailure(&'static str);

impl fmt::Display for MessageOnlyFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl Error for MessageOnlyFailure {}

#[test]
fn native_typed_context_takes_precedence_and_normalization_is_idempotent() {
    let report = Report::new(SourcedFailure {
        message: "custom wrapper",
        source: Box::new(CancellationError::new("inner source reason")),
    })
    .wrap_err(CancellationError::new("selected typed context"))
    .wrap_err("outer report context");
    let original_reason = report.as_cancellation().unwrap().reason.as_ptr();
    let original_chain: Vec<_> = report.chain().map(ToString::to_string).collect();
    assert_eq!(
        report.as_cancellation().unwrap().reason,
        "selected typed context"
    );
    assert!(report.is_cancellation());

    let normalized = report.normalize_cancellation().normalize_cancellation();

    assert_eq!(normalized.to_string(), "outer report context");
    assert_eq!(
        normalized.as_cancellation().unwrap().reason.as_ptr(),
        original_reason
    );
    assert_eq!(
        normalized
            .chain()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        original_chain
    );
}

#[test]
fn normalizing_a_custom_source_preserves_the_original_report_and_handler() {
    let report = Report::new(SourcedFailure {
        message: "adapter failed",
        source: Box::new(CancellationError::new("source cancellation")),
    })
    .wrap_err("rendering a report");
    let original_source = ptr::from_ref(report.downcast_ref::<SourcedFailure>().unwrap());
    let original_handler = ptr::from_ref(report.handler());
    let original_chain: Vec<_> = report.chain().map(ToString::to_string).collect();
    assert!(report.downcast_ref::<CancellationError>().is_none());
    assert_eq!(
        report.as_cancellation().unwrap().reason,
        "source cancellation"
    );

    let normalized = report.normalize_cancellation();

    assert_eq!(normalized.to_string(), "source cancellation");
    assert!(normalized.downcast_ref::<CancellationError>().is_some());
    assert!(ptr::eq(
        original_source,
        normalized.downcast_ref::<SourcedFailure>().unwrap()
    ));
    assert!(ptr::eq(original_handler, normalized.handler()));
    assert_eq!(
        normalized
            .chain()
            .skip(1)
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        original_chain
    );
}

#[test]
fn cancellation_words_do_not_change_an_unrelated_report() {
    let report = Report::new(MessageOnlyFailure("operation cancelled"))
        .wrap_err("cancellation requested while another operation failed");
    let original_error = ptr::from_ref(report.downcast_ref::<MessageOnlyFailure>().unwrap());
    let original_handler = ptr::from_ref(report.handler());
    let original_chain: Vec<_> = report.chain().map(ToString::to_string).collect();
    assert!(!report.is_cancellation());
    assert!(report.as_cancellation().is_none());

    let normalized = report.normalize_cancellation();

    assert!(normalized.downcast_ref::<CancellationError>().is_none());
    assert!(ptr::eq(
        original_error,
        normalized.downcast_ref::<MessageOnlyFailure>().unwrap()
    ));
    assert!(ptr::eq(original_handler, normalized.handler()));
    assert_eq!(
        normalized
            .chain()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        original_chain
    );
}

#[cfg(feature = "io")]
#[test]
fn direct_io_cancellation_normalizes_without_losing_the_original_io_payload() {
    let report = Report::new(io::Error::other(CancellationError::new("I/O cancellation")));
    let original_io = report.downcast_ref::<io::Error>().unwrap();
    let original_payload = ptr::from_ref(original_io.as_cancellation().unwrap());
    assert!(report.downcast_ref::<CancellationError>().is_none());
    assert_eq!(report.as_cancellation().unwrap().reason, "I/O cancellation");

    let normalized = report.normalize_cancellation();

    assert_eq!(
        normalized
            .downcast_ref::<CancellationError>()
            .unwrap()
            .reason,
        "I/O cancellation"
    );
    let original_io = normalized.downcast_ref::<io::Error>().unwrap();
    assert_eq!(original_io.kind(), io::ErrorKind::Other);
    assert!(ptr::eq(
        original_payload,
        original_io.as_cancellation().unwrap()
    ));
}

#[cfg(feature = "io")]
#[test]
fn custom_sources_and_nested_io_payloads_are_traversed_without_discarding_wrappers() {
    let report = Report::new(SourcedFailure {
        message: "outer source wrapper",
        source: Box::new(io::Error::new(
            io::ErrorKind::PermissionDenied,
            io::Error::other(CancellationError::new("nested I/O cancellation")),
        )),
    });
    assert!(report.downcast_ref::<CancellationError>().is_none());
    assert_eq!(
        report.as_cancellation().unwrap().reason,
        "nested I/O cancellation"
    );

    let normalized = report.normalize_cancellation();

    assert!(normalized.downcast_ref::<CancellationError>().is_some());
    let wrapper = normalized.downcast_ref::<SourcedFailure>().unwrap();
    let outer_io = wrapper.source.downcast_ref::<io::Error>().unwrap();
    assert_eq!(outer_io.kind(), io::ErrorKind::PermissionDenied);
    let inner_io = outer_io
        .get_ref()
        .unwrap()
        .downcast_ref::<io::Error>()
        .unwrap();
    assert_eq!(
        inner_io.as_cancellation().unwrap().reason,
        "nested I/O cancellation"
    );
}

#[cfg(feature = "io")]
#[test]
fn an_io_valued_typed_context_is_checked_even_when_absent_from_the_source_chain() {
    let report = Report::new(MessageOnlyFailure("original destination context"))
        .wrap_err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            CancellationError::new("I/O typed context cancellation"),
        ))
        .wrap_err("outer display context");
    assert!(!report.chain().any(<dyn Error>::is::<io::Error>));
    assert!(report.downcast_ref::<CancellationError>().is_none());
    assert_eq!(
        report.as_cancellation().unwrap().reason,
        "I/O typed context cancellation"
    );

    let normalized = report.normalize_cancellation();

    assert!(normalized.downcast_ref::<CancellationError>().is_some());
    assert!(normalized.downcast_ref::<MessageOnlyFailure>().is_some());
    let original_io = normalized.downcast_ref::<io::Error>().unwrap();
    assert_eq!(original_io.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(
        original_io.as_cancellation().unwrap().reason,
        "I/O typed context cancellation"
    );
}
