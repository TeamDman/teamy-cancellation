use crate::CancellationListener;
#[cfg(feature = "async")]
use crate::CancellationWait;
use crate::cancellation_inner::CancellationInner;
use crate::cancellation_listener_state::CancellationListenerState;
use crate::cancellation_registration::CancellationRegistration;
#[cfg(feature = "async")]
use crate::cancellation_waker::CancellationWaker;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::Ordering;

/// Shared cancellation intent, reasons, and notifications.
///
/// Execution ownership and protected completion belong to `CancellationScope`,
/// whose checkpoints respect required completion. Raw token observation always
/// reports the cancellation request without postponement.
#[derive(Clone, Default)]
#[cfg_attr(feature = "facet", derive(facet::Facet))]
#[cfg_attr(feature = "facet", facet(opaque))]
pub struct CancellationToken {
    inner: Arc<CancellationInner>,
}

impl CancellationToken {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn new_with_on_cancel_request(
        on_cancel_request: impl Fn(&str, bool) + Send + Sync + 'static,
    ) -> Self {
        Self {
            inner: Arc::new(CancellationInner {
                on_cancel_request: Some(Arc::new(on_cancel_request)),
                ..CancellationInner::default()
            }),
        }
    }

    #[cfg(feature = "inheritance")]
    #[must_use]
    pub fn child_token(&self) -> Self {
        Self {
            inner: Arc::new(CancellationInner {
                parent: Some(self.clone()),
                ..CancellationInner::default()
            }),
        }
    }

    /// Request cancellation, preserving the first local reason.
    ///
    /// The requested state and all async wakeups are published before callbacks.
    /// The constructor hook runs for every request; dynamic listeners run once.
    pub fn request_cancel(&self, reason: impl Into<String>) {
        let reason = reason.into();
        let accepted = {
            let mut stored = self
                .inner
                .reason
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let accepted = stored.is_none();
            if accepted {
                *stored = Some(reason.clone());
            }
            self.inner.cancelled.store(true, Ordering::Release);
            accepted
        };
        let listeners;
        #[cfg(feature = "async")]
        let waiters;
        {
            let registrations = self
                .inner
                .registrations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            listeners = registrations.listeners();
            #[cfg(feature = "async")]
            {
                waiters = registrations.waiters();
            }
        }
        #[cfg(feature = "async")]
        for waiter in waiters {
            waiter.wake();
        }

        #[cfg(feature = "tracing")]
        if accepted {
            tracing::info!(reason, "Cancellation requested");
        } else {
            tracing::debug!(
                reason,
                "Ignoring cancellation request because cancellation has already been requested"
            );
        }

        let stored_reason = self
            .cancellation_reason()
            .unwrap_or_else(|| String::from("Operation cancelled"));
        for listener in listeners {
            listener.notify(&stored_reason);
        }
        if let Some(hook) = &self.inner.on_cancel_request {
            hook(&reason, accepted);
        }
    }

    /// Observe raw cancellation intent, including an inherited parent's request.
    /// Execution-scope protection does not change this flag.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
            || self.parent().as_ref().is_some_and(Self::is_cancelled)
    }

    #[must_use]
    pub fn cancellation_reason(&self) -> Option<String> {
        let reason = self
            .inner
            .reason
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        reason.or_else(|| self.parent().as_ref().and_then(Self::cancellation_reason))
    }

    /// Listen once for cancellation on this token or an inherited parent.
    ///
    /// The callback receives a snapshot of the first notification's reason. An
    /// already requested token invokes the callback before this method returns.
    /// Dropping the returned handle unregisters the listener; repeated requests
    /// do not repeat its callback. Callbacks run outside internal locks.
    #[must_use]
    pub fn register_listener(
        &self,
        callback: impl Fn(&str) + Send + Sync + 'static,
    ) -> CancellationListener {
        let state = Arc::new(CancellationListenerState::new(callback));
        let mut listener = CancellationListener {
            state,
            registrations: Vec::new(),
        };
        if !self.is_cancelled() {
            for inner in self.cancellation_chain() {
                let id = inner
                    .registrations
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .add_listener(&listener.state);
                listener.registrations.push(CancellationRegistration {
                    inner: Arc::downgrade(&inner),
                    id,
                });
            }
        }
        // Register then recheck so a racing request cannot miss this listener.
        if self.is_cancelled() {
            listener.state.notify(
                &self
                    .cancellation_reason()
                    .unwrap_or_else(|| String::from("Operation cancelled")),
            );
        }
        listener
    }

    /// Wait for sticky requested cancellation without needing a runtime.
    #[cfg(feature = "async")]
    #[must_use]
    pub fn cancelled(&self) -> CancellationWait {
        CancellationWait::new(self.clone())
    }

    fn parent(&self) -> Option<Self> {
        #[cfg(feature = "inheritance")]
        {
            self.inner.parent.clone()
        }
        #[cfg(not(feature = "inheritance"))]
        {
            None
        }
    }

    fn cancellation_chain(&self) -> Vec<Arc<CancellationInner>> {
        let mut chain = Vec::new();
        let mut current = Some(self.clone());
        while let Some(token) = current {
            chain.push(Arc::clone(&token.inner));
            current = token.parent();
        }
        chain
    }

    #[cfg(feature = "async")]
    pub(crate) fn register_waiter(
        &self,
        waiter: &Arc<CancellationWaker>,
    ) -> Vec<CancellationRegistration> {
        self.cancellation_chain()
            .into_iter()
            .map(|inner| {
                let id = inner
                    .registrations
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .add_waiter(waiter);
                CancellationRegistration {
                    inner: Arc::downgrade(&inner),
                    id,
                }
            })
            .collect()
    }
}

impl fmt::Debug for CancellationToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CancellationToken")
            .field("is_cancelled", &self.is_cancelled())
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "arbitrary")]
impl<'a> arbitrary::Arbitrary<'a> for CancellationToken {
    fn arbitrary(_input: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        // A generated request gets isolated local state, never a process signal
        // handler or a token shared with any live invocation.
        Ok(Self::new())
    }
}
