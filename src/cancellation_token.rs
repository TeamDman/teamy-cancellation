use std::fmt;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

#[derive(Clone, Default)]
#[cfg_attr(feature = "facet", derive(facet::Facet))]
#[cfg_attr(feature = "facet", facet(opaque))]
pub struct CancellationToken {
    inner: Arc<CancellationInner>,
}

type CancelHook = dyn Fn(&str, bool) + Send + Sync + 'static;

#[derive(Default)]
struct CancellationInner {
    cancelled: AtomicBool,
    reason: Mutex<Option<String>>,
    on_cancel_request: Option<Arc<CancelHook>>,
    #[cfg(feature = "inheritance")]
    parent: Option<CancellationToken>,
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
                cancelled: AtomicBool::new(false),
                reason: Mutex::new(None),
                on_cancel_request: Some(Arc::new(on_cancel_request)),
                #[cfg(feature = "inheritance")]
                parent: None,
            }),
        }
    }

    #[cfg(feature = "inheritance")]
    #[must_use]
    pub fn child_token(&self) -> Self {
        Self {
            inner: Arc::new(CancellationInner {
                cancelled: AtomicBool::new(false),
                reason: Mutex::new(None),
                on_cancel_request: None,
                parent: Some(self.clone()),
            }),
        }
    }

    pub fn request_cancel(&self, reason: impl Into<String>) {
        let reason = reason.into();
        #[cfg(feature = "tracing")]
        let accepted = self.store_reason_if_missing(&reason);
        #[cfg(not(feature = "tracing"))]
        let accepted = self.store_reason_if_missing(&reason);

        #[cfg(feature = "tracing")]
        if accepted {
            tracing::info!(reason, "Cancellation requested");
        } else {
            tracing::debug!(
                reason,
                "Ignoring cancellation request because cancellation has already been requested"
            );
        }

        self.run_on_cancel_request_hook(&reason, accepted);

        self.inner.cancelled.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
            || self
                .parent()
                .as_ref()
                .is_some_and(CancellationToken::is_cancelled)
    }

    #[must_use]
    pub fn cancellation_reason(&self) -> Option<String> {
        let reason = self
            .inner
            .reason
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        reason.or_else(|| {
            self.parent()
                .as_ref()
                .and_then(CancellationToken::cancellation_reason)
        })
    }

    /// # Errors
    ///
    /// Returns an error after cancellation has been requested.
    #[track_caller]
    pub fn bail_if_cancelled(&self) -> eyre::Result<()> {
        if self.is_cancelled() {
            let reason = self
                .cancellation_reason()
                .unwrap_or_else(|| String::from("Operation cancelled"));
            eyre::bail!("{reason}");
        }
        Ok(())
    }
}

impl fmt::Debug for CancellationToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancellationToken")
            .field("is_cancelled", &self.is_cancelled())
            .finish_non_exhaustive()
    }
}

impl CancellationToken {
    #[cfg(feature = "inheritance")]
    fn parent(&self) -> Option<CancellationToken> {
        self.inner.parent.clone()
    }

    #[cfg(not(feature = "inheritance"))]
    fn parent(&self) -> Option<CancellationToken> {
        None
    }

    fn store_reason_if_missing(&self, reason: &str) -> bool {
        let mut stored_reason = self
            .inner
            .reason
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if stored_reason.is_none() {
            *stored_reason = Some(reason.to_string());
            true
        } else {
            false
        }
    }

    fn run_on_cancel_request_hook(&self, reason: &str, accepted: bool) {
        if let Some(on_cancel_request) = &self.inner.on_cancel_request {
            #[cfg(feature = "tracing")]
            tracing::debug!("Running on_cancel_request hook");
            on_cancel_request(reason, accepted);
        }
    }
}
