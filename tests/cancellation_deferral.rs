#![cfg(feature = "deferral")]

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use teamy_cancellation::CancellationToken;

#[test]
fn nested_deferrals_do_not_resume_cancellation_until_every_guard_is_dropped() {
    let token = CancellationToken::new().for_operation();
    let outer = token.defer_cancellation("recording mutation outcome");
    let inner = token.defer_cancellation("invalidating related reads");
    token.request_cancel("stop");
    assert!(token.bail_if_cancelled().is_ok());
    drop(outer);
    assert!(token.bail_if_cancelled().is_ok());
    drop(inner);
    assert!(token.bail_if_cancelled().is_err());
}

#[test]
fn raw_intent_still_notifies_listeners_while_required_completion_is_deferred() {
    let token = CancellationToken::new();
    let notified = Arc::new(AtomicBool::new(false));
    let listener_observed = Arc::clone(&notified);
    let _listener = token.register_listener(move |reason| {
        assert_eq!(reason, "stop after mutation");
        listener_observed.store(true, Ordering::SeqCst);
    });
    let deferral = token.defer_cancellation("recording the mutation outcome");

    token.request_cancel("stop after mutation");

    assert!(notified.load(Ordering::SeqCst));
    assert!(token.is_cancelled());
    assert_eq!(
        token.cancellation_reason().as_deref(),
        Some("stop after mutation"),
    );
    assert!(token.bail_if_cancelled().is_ok());

    drop(deferral);
    assert!(token.bail_if_cancelled().is_err());
    assert!(token.is_cancelled());
}

#[test]
fn clones_share_required_completion_until_the_guard_is_dropped() {
    let token = CancellationToken::new().for_operation();
    let completing = token.clone();
    let checkpoint = completing.clone();
    let deferral = completing.defer_cancellation("finishing a replacement file");

    token.request_cancel("stop the operation");

    assert!(token.bail_if_cancelled().is_ok());
    assert!(checkpoint.bail_if_cancelled().is_ok());
    drop(deferral);
    assert!(token.bail_if_cancelled().is_err());
    assert!(checkpoint.bail_if_cancelled().is_err());
}

#[test]
fn an_independent_scope_does_not_inherit_another_operations_deferral() {
    let token = CancellationToken::new();
    let deferral = token.defer_cancellation("finish the active mutation");
    let independent = token.for_operation();

    token.request_cancel("stop all operations");

    assert!(token.bail_if_cancelled().is_ok());
    assert!(independent.bail_if_cancelled().is_err());

    let independent_cleanup = independent.defer_cancellation("finish its required cleanup");
    drop(deferral);
    assert!(token.bail_if_cancelled().is_err());
    assert!(independent.bail_if_cancelled().is_ok());

    drop(independent_cleanup);
    assert!(independent.bail_if_cancelled().is_err());
}

#[test]
fn already_requested_cancellation_allows_late_required_cleanup_and_resumes_afterward() {
    let token = CancellationToken::new().for_operation();
    token.request_cancel("stop before cleanup");
    assert!(token.bail_if_cancelled().is_err());

    {
        let _cleanup = token.defer_cancellation("finish cleanup from the completed mutation");
        assert!(token.bail_if_cancelled().is_ok());
        assert!(token.is_cancelled());
        assert_eq!(
            token.cancellation_reason().as_deref(),
            Some("stop before cleanup"),
        );
    }

    let error = token.bail_if_cancelled().unwrap_err();
    assert_eq!(error.to_string(), "stop before cleanup");
}

#[cfg(feature = "inheritance")]
#[test]
fn a_parent_deferral_does_not_hide_cancellation_from_a_child_operation() {
    let parent = CancellationToken::new();
    let child = parent.child_token();
    let parent_completion = parent.defer_cancellation("finish the parent operation");

    parent.request_cancel("stop the operation tree");

    assert!(parent.bail_if_cancelled().is_ok());
    assert!(child.bail_if_cancelled().is_err());
    let child_completion = child.defer_cancellation("finish the child's own cleanup");

    drop(parent_completion);
    assert!(parent.bail_if_cancelled().is_err());
    assert!(child.bail_if_cancelled().is_ok());
    assert_eq!(
        child.cancellation_reason().as_deref(),
        Some("stop the operation tree"),
    );

    drop(child_completion);
    assert!(child.bail_if_cancelled().is_err());
}

#[cfg(feature = "tracing-subscriber")]
#[test]
fn sections_acknowledge_once_and_unregister_when_they_finish() {
    use std::sync::Mutex;
    use tracing_subscriber::layer::SubscriberExt;

    #[derive(Default)]
    struct AcknowledgmentVisitor {
        explanation: Option<String>,
        caller_file: Option<String>,
        caller_line: Option<u64>,
    }

    impl tracing::field::Visit for AcknowledgmentVisitor {
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            match field.name() {
                "explanation" => self.explanation = Some(value.to_owned()),
                "caller_file" => self.caller_file = Some(value.to_owned()),
                _ => {}
            }
        }

        fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
            if field.name() == "caller_line" {
                self.caller_line = Some(value);
            }
        }

        fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
    }

    struct AcknowledgmentLayer(Arc<Mutex<Vec<(String, String, u64)>>>);

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for AcknowledgmentLayer {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut visitor = AcknowledgmentVisitor::default();
            event.record(&mut visitor);
            if let Some(explanation) = visitor.explanation {
                self.0.lock().unwrap().push((
                    explanation,
                    visitor
                        .caller_file
                        .expect("Caller file must accompany acknowledgment"),
                    visitor
                        .caller_line
                        .expect("Caller line must accompany acknowledgment"),
                ));
            }
        }
    }

    let observed = Arc::new(Mutex::new(Vec::new()));
    let subscriber =
        tracing_subscriber::registry().with(AcknowledgmentLayer(Arc::clone(&observed)));
    let expected_line = tracing::subscriber::with_default(subscriber, || {
        let token = CancellationToken::new().for_operation();
        let expected_line = line!() + 1;
        let first = token.defer_cancellation("first required phase");
        token.request_cancel("stop");
        token.request_cancel("repeated");
        drop(first);

        // Cleanup entered after cancellation must acknowledge immediately.
        let late = token.defer_cancellation("late required cleanup");
        drop(late);

        // A completed section must no longer report a postponed cancellation.
        let token = CancellationToken::new().for_operation();
        let completed = token.defer_cancellation("already completed");
        drop(completed);
        token.request_cancel("stop");
        expected_line
    });
    let observed = observed.lock().unwrap();
    assert_eq!(
        observed
            .iter()
            .map(|event| event.0.as_str())
            .collect::<Vec<_>>(),
        ["first required phase", "late required cleanup"]
    );
    assert_eq!(observed[0].1, file!());
    assert_eq!(observed[0].2, u64::from(expected_line));
}
