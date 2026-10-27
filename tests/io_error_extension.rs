#![cfg(feature = "io")]

use std::error::Error;
use std::fmt;
use std::io;
use std::ptr::from_ref;
use teamy_cancellation::CancellationError;
use teamy_cancellation::CancellationErrorExt;

#[test]
fn borrowing_mutating_and_consuming_preserve_the_original_reason_allocation() {
    let reason = String::from("cancel this output");
    let original_pointer = reason.as_ptr();
    let mut error = io::Error::other(CancellationError::new(reason));

    let borrowed = error.as_cancellation().unwrap();
    assert_eq!(borrowed.reason, "cancel this output");
    assert_eq!(borrowed.reason.as_ptr(), original_pointer);

    error
        .as_cancellation_mut()
        .unwrap()
        .reason
        .make_ascii_uppercase();

    let cancellation = error.into_cancellation().unwrap();
    assert_eq!(cancellation.reason, "CANCEL THIS OUTPUT");
    assert_eq!(cancellation.reason.as_ptr(), original_pointer);
}

#[derive(Debug)]
struct DestinationFailure {
    message: String,
}

impl fmt::Display for DestinationFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for DestinationFailure {}

#[test]
fn unrelated_errors_retain_their_kind_os_code_and_boxed_payload_identity() {
    let payload: Box<dyn Error + Send + Sync> = Box::new(DestinationFailure {
        message: "synthetic destination failure".to_owned(),
    });
    let original_pointer = from_ref(payload.downcast_ref::<DestinationFailure>().unwrap());
    let mut error = io::Error::new(io::ErrorKind::PermissionDenied, payload);
    assert!(error.as_cancellation().is_none());
    assert!(error.as_cancellation_mut().is_none());

    let original = error.into_cancellation().unwrap_err();
    assert_eq!(original.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(original.raw_os_error(), None);
    assert_eq!(
        from_ref(
            original
                .get_ref()
                .unwrap()
                .downcast_ref::<DestinationFailure>()
                .unwrap()
        ),
        original_pointer
    );

    for mut error in [
        io::Error::from_raw_os_error(12_345),
        io::Error::from(io::ErrorKind::UnexpectedEof),
    ] {
        let kind = error.kind();
        let os_code = error.raw_os_error();
        assert!(error.as_cancellation().is_none());
        assert!(error.as_cancellation_mut().is_none());

        let original = error.into_cancellation().unwrap_err();
        assert_eq!(original.kind(), kind);
        assert_eq!(original.raw_os_error(), os_code);
        assert!(original.get_ref().is_none());
    }
}

#[derive(Debug)]
struct WrappedCancellation(CancellationError);

impl fmt::Display for WrappedCancellation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("synthetic wrapper around cancellation")
    }
}

impl Error for WrappedCancellation {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.0)
    }
}

#[test]
fn cancellation_in_a_source_chain_is_not_a_direct_cancellation_payload() {
    let mut error = io::Error::other(WrappedCancellation(CancellationError::new(
        "nested cancellation remains wrapped",
    )));
    assert!(
        error
            .get_ref()
            .unwrap()
            .source()
            .unwrap()
            .is::<CancellationError>()
    );
    assert_eq!(
        error.as_cancellation().unwrap().reason,
        "nested cancellation remains wrapped"
    );
    assert!(error.is_cancellation());
    assert!(error.as_cancellation_mut().is_none());

    let original = error.into_cancellation().unwrap_err();
    let wrapper = original
        .get_ref()
        .unwrap()
        .downcast_ref::<WrappedCancellation>()
        .unwrap();
    assert_eq!(original.kind(), io::ErrorKind::Other);
    assert_eq!(wrapper.0.reason, "nested cancellation remains wrapped");
}
