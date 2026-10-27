#![cfg(feature = "figue")]

use teamy_cancellation::CancellationToken;
use teamy_cancellation::StopAfterArgs;

#[test]
fn stop_after_arguments_preserve_cli_names() {
    let args: StopAfterArgs =
        figue::from_slice(&["--stop-after", "fixture_span", "--stop-after-duration", "1ms"])
            .unwrap();

    assert_eq!(args.stop_after_span.as_deref(), Some("fixture_span"));
    assert_eq!(args.stop_after_duration.as_deref(), Some("1ms"));
    assert!(args.stop_after_span_layer(CancellationToken::new()).is_some());
}

#[test]
fn invalid_duration_is_rejected_without_cancelling() {
    let args = StopAfterArgs {
        stop_after_duration: Some("invalid".to_owned()),
        ..StopAfterArgs::default()
    };
    let token = CancellationToken::new();

    args.start_stop_after_duration_thread(token.clone())
        .expect_err("invalid duration");

    assert!(!token.is_cancelled());
}

#[cfg(feature = "async")]
#[test]
fn duration_cancels_async_waiters_with_the_configured_reason() {
    use std::sync::Arc;
    use std::sync::mpsc;
    use std::task::Context;
    use std::task::Wake;
    use std::task::Waker;
    use std::time::Duration;

    struct WakeSignal(mpsc::Sender<()>);

    impl Wake for WakeSignal {
        fn wake(self: Arc<Self>) {
            let _ = self.0.send(());
        }
    }

    let args = StopAfterArgs {
        stop_after_duration: Some("1ms".to_owned()),
        ..StopAfterArgs::default()
    };
    let token = CancellationToken::new();
    let mut wait = std::pin::pin!(token.cancelled());
    let (sender, receiver) = mpsc::channel();
    let waker = Waker::from(Arc::new(WakeSignal(sender)));
    let mut context = Context::from_waker(&waker);
    assert!(wait.as_mut().poll(&mut context).is_pending());

    let timer = args
        .start_stop_after_duration_thread(token.clone())
        .unwrap()
        .unwrap();

    receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("duration must wake the registered async waiter");
    timer.join().unwrap();
    assert!(wait.as_mut().poll(&mut context).is_ready());
    assert_eq!(
        token.cancellation_reason().as_deref(),
        Some("Operation cancelled after --stop-after-duration 1ms")
    );
}
