#![cfg(feature = "deferral")]

use std::panic::AssertUnwindSafe;
use std::panic::catch_unwind;
use std::sync::Arc;
use std::sync::Mutex;
use teamy_cancellation::CancellationError;
use teamy_cancellation::CancellationScope;
use teamy_cancellation::CancellationToken;
#[cfg(feature = "tracing-subscriber")]
use tracing_subscriber::layer::SubscriberExt;

fn assert_closed_scope_rejects_new_work(scope: &CancellationScope) {
    assert!(scope.bail_if_cancelled().is_err());
    let child: Result<(), CancellationError> = scope.with_scope(|_| {
        panic!("a closed scope must not execute a new child");
    });
    assert!(child.is_err());

    let protected: Result<(), CancellationError> = scope.with_deferral("new mutation", |_| {
        panic!("a closed scope must not execute a protected child");
    });
    assert!(protected.is_err());

    let cleanup: Result<(), CancellationError> =
        scope.finish_required_sync("late required cleanup", |_| {
            panic!("required cleanup must not reopen a closed owner");
        });
    assert!(cleanup.is_err());
}

#[test]
fn protected_lineage_continues_while_an_ordinary_sibling_stops() -> Result<(), CancellationError> {
    let token = CancellationToken::new();

    token.with_scope(|owner| {
        // Both children remain open during the protected callback. Protection
        // belongs to the mutation branch, not the ordinary sibling or token.
        let mutation_parent = owner.clone();
        owner.with_scope(|ordinary| {
            mutation_parent.with_deferral("finish the submitted mutation", |protected| {
                token.request_cancel("stop independent work");
                assert!(ordinary.bail_if_cancelled().is_err());
                protected.bail_if_cancelled()?;

                let sibling: Result<(), CancellationError> = ordinary.with_scope(|_| {
                    panic!("an ordinary sibling must reject work after cancellation");
                });
                assert!(sibling.is_err());

                protected.with_scope(|nested| {
                    assert!(nested.is_cancelled());
                    nested.bail_if_cancelled()?;
                    nested.with_deferral("record the mutation outcome", |completion| {
                        completion.with_scope(|descendant| {
                            assert!(descendant.is_cancelled());
                            Ok::<_, CancellationError>(())
                        })
                    })
                })?;
                // Completing the inner protected child must not end the outer
                // scope's still-active required-completion lease.
                protected.bail_if_cancelled()
            })
        })
    })
}

#[test]
fn protected_scopes_keep_raw_intent_and_listener_notifications_visible()
-> Result<(), CancellationError> {
    let token = CancellationToken::new();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let raw_observed = Arc::clone(&observed);
    let _raw_listener = token.register_listener(move |reason| {
        raw_observed
            .lock()
            .unwrap()
            .push(("raw", reason.to_owned()));
    });

    token.with_scope(|owner| {
        owner.with_deferral("persist the completed mutation", |protected| {
            let scope_observed = Arc::clone(&observed);
            let _scope_listener = protected.register_listener(move |reason| {
                scope_observed
                    .lock()
                    .unwrap()
                    .push(("scope", reason.to_owned()));
            });

            protected.request_cancel("stop after mutation");
            protected.request_cancel("repeated request");

            assert!(token.is_cancelled());
            assert!(owner.is_cancelled());
            assert!(protected.is_cancelled());
            assert_eq!(
                protected.cancellation_reason().as_deref(),
                Some("stop after mutation")
            );
            protected.with_scope(|_| Ok::<_, CancellationError>(()))
        })
    })?;

    let mut observed = observed.lock().unwrap().clone();
    observed.sort();
    assert_eq!(
        observed,
        [
            ("raw", "stop after mutation".to_owned()),
            ("scope", "stop after mutation".to_owned()),
        ]
    );
    assert!(token.is_cancelled());
    Ok(())
}

#[test]
fn an_already_cancelled_token_does_not_admit_a_new_root_scope() {
    let token = CancellationToken::new();
    token.request_cancel("stop before admission");

    let result: Result<(), CancellationError> = token.with_scope(|_| {
        panic!("an already cancelled token must not start a new root scope");
    });

    let error = result.expect_err("root admission must return typed cancellation");
    assert_eq!(error.reason, "stop before admission");
}

#[test]
fn late_cleanup_is_explicit_and_does_not_readmit_ordinary_work() -> Result<(), CancellationError> {
    let token = CancellationToken::new();

    token.with_scope(|owner| {
        owner.request_cancel("stop after an already completed mutation");

        let ordinary: Result<(), CancellationError> = owner.with_scope(|_| {
            panic!("ordinary child admission must observe cancellation");
        });
        assert!(ordinary.is_err());
        let new_mutation: Result<(), CancellationError> =
            owner.with_deferral("start another mutation", |_| {
                panic!("normal deferral admission must observe cancellation");
            });
        assert!(new_mutation.is_err());

        let completed = owner.finish_required_sync("persist the existing outcome", |cleanup| {
            assert!(cleanup.is_cancelled());
            cleanup.with_scope(|nested| {
                assert_eq!(
                    nested.cancellation_reason().as_deref(),
                    Some("stop after an already completed mutation")
                );
                Ok::<_, CancellationError>("outcome recorded")
            })
        })?;
        assert_eq!(completed, "outcome recorded");

        let next: Result<(), CancellationError> = owner.with_scope(|_| {
            panic!("finishing required cleanup must not readmit ordinary work");
        });
        assert!(next.is_err());
        Ok(())
    })
}

#[test]
fn a_saved_ordinary_handle_cannot_extend_its_completed_owner() -> Result<(), CancellationError> {
    let token = CancellationToken::new();
    let saved = token.with_scope(|owner| Ok::<_, CancellationError>(owner.clone()))?;

    assert!(!token.is_cancelled());
    assert_closed_scope_rejects_new_work(&saved);
    Ok(())
}

#[test]
fn a_saved_protected_handle_closes_before_the_parent_finishes() -> Result<(), CancellationError> {
    let token = CancellationToken::new();

    token.with_scope(|owner| {
        let saved = owner.with_deferral("finish one mutation", |protected| {
            Ok::<_, CancellationError>(protected.clone())
        })?;

        // A retained clone is not a guard. The protected callback has finished,
        // even though its ordinary parent can still admit unrelated work.
        assert_closed_scope_rejects_new_work(&saved);
        owner.with_scope(|_| Ok::<_, CancellationError>(()))?;
        token.request_cancel("stop after the protected callback finishes");
        assert!(owner.bail_if_cancelled().is_err());
        Ok(())
    })
}

#[test]
fn panicking_callbacks_close_both_escaped_owner_and_protected_handles() {
    let token = CancellationToken::new();
    let mut escaped_owner = None;
    let mut escaped_protected = None;

    let panic = catch_unwind(AssertUnwindSafe(|| {
        let _: Result<(), CancellationError> = token.with_scope(|owner| {
            escaped_owner = Some(owner.clone());
            owner.with_deferral("finish the mutation", |protected| {
                escaped_protected = Some(protected.clone());
                panic!("synthetic callback failure");
            })
        });
    }));

    assert!(panic.is_err());
    assert!(!token.is_cancelled());
    assert_closed_scope_rejects_new_work(&escaped_owner.unwrap());
    assert_closed_scope_rejects_new_work(&escaped_protected.unwrap());
}

#[cfg(feature = "tracing-subscriber")]
#[test]
fn sections_acknowledge_once_and_unregister_when_they_finish() -> Result<(), CancellationError> {
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
                        .expect("caller file must accompany acknowledgment"),
                    visitor
                        .caller_line
                        .expect("caller line must accompany acknowledgment"),
                ));
            }
        }
    }

    let observed = Arc::new(Mutex::new(Vec::new()));
    let subscriber =
        tracing_subscriber::registry().with(AcknowledgmentLayer(Arc::clone(&observed)));
    let expected_line = tracing::subscriber::with_default(subscriber, || {
        let token = CancellationToken::new();
        let expected_line = token.with_scope(|owner| {
            let expected_line = line!() + 1;
            owner.with_deferral("first required phase", |protected| {
                protected.request_cancel("stop");
                protected.request_cancel("repeated");
                Ok::<_, CancellationError>(())
            })?;

            owner.finish_required_sync("late required cleanup", |_| {
                Ok::<_, CancellationError>(())
            })?;
            Ok::<_, CancellationError>(expected_line)
        })?;

        let completed = CancellationToken::new();
        completed.with_scope(|owner| {
            owner.with_deferral("already completed", |_| Ok::<_, CancellationError>(()))
        })?;
        completed.request_cancel("stop");
        Ok::<_, CancellationError>(expected_line)
    })?;

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
    Ok(())
}
