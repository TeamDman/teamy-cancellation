#[cfg(feature = "async")]
use crate::CancellableScopeFuture;
use crate::CancellationError;
use crate::CancellationListener;
use crate::CancellationToken;
#[cfg(feature = "async")]
use crate::CancellationWait;
use crate::cancellation_scope_kind::CancellationScopeKind;
use crate::cancellation_scope_owner::CancellationScopeOwner;
use crate::cancellation_scope_state::CancellationScopeState;
use std::panic::Location;
use std::sync::Arc;

/// The cancellation policy and ownership of one executing operation.
///
/// A scope is supplied by a bounded callback or future. Nested scope helpers
/// create linked children, so required completion protects enclosing owners
/// without shielding independent siblings. Clones refer to the same scope;
/// they cannot prolong protection or reopen an owner that has finished.
///
/// Pass this handle to operation implementations. A raw [`CancellationToken`]
/// observes cancellation intent without participating in execution ownership.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "facet", derive(facet::Facet))]
#[cfg_attr(feature = "facet", facet(opaque))]
pub struct CancellationScope {
    // Private because execution helpers, rather than handle clones, own the
    // scope's admission, required-completion lease, and closed lifetime.
    pub(crate) token: CancellationToken,
    pub(crate) state: Arc<CancellationScopeState>,
}

impl Default for CancellationScope {
    fn default() -> Self {
        Self::unbound()
    }
}

#[cfg(feature = "arbitrary")]
impl<'a> arbitrary::Arbitrary<'a> for CancellationScope {
    fn arbitrary(_input: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self::unbound())
    }
}

impl CancellationScope {
    /// An inert, isolated placeholder for reflected or generated request input.
    ///
    /// This scope is already closed: checkpoints and child admission reject it,
    /// including required cleanup. Invocation infrastructure must replace it with
    /// the scope supplied by a live execution callback before dispatch. Default
    /// and Arbitrary construction have this same inert behavior, never an owner
    /// whose lifetime outlives a callback or a process signal handler.
    #[must_use]
    pub fn unbound() -> Self {
        let scope = Self::root(CancellationToken::new());
        scope.state.close();
        scope
    }

    pub(crate) fn root(token: CancellationToken) -> Self {
        Self {
            token,
            state: CancellationScopeState::root(),
        }
    }

    pub(crate) fn child(&self) -> Self {
        Self {
            token: self.token.clone(),
            state: self.state.child(),
        }
    }

    /// Requests cancellation of the shared operation signal.
    pub fn request_cancel(&self, reason: impl Into<String>) {
        self.token.request_cancel(reason);
    }

    /// Observes raw cancellation intent, including during required completion.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    /// Returns the shared cancellation request's reason.
    #[must_use]
    pub fn cancellation_reason(&self) -> Option<String> {
        self.token.cancellation_reason()
    }

    /// Registers a once-only listener for raw intent, regardless of protection.
    #[must_use = "Keep the listener alive to receive its notification"]
    pub fn register_listener(
        &self,
        callback: impl Fn(&str) + Send + Sync + 'static,
    ) -> CancellationListener {
        self.token.register_listener(callback)
    }

    /// Waits for raw intent, regardless of this scope's cancellation policy.
    #[cfg(feature = "async")]
    #[must_use = "Cancellation waits do nothing unless polled or awaited"]
    pub fn cancelled(&self) -> CancellationWait {
        self.token.cancelled()
    }

    /// Checks whether this scope may cooperatively stop.
    ///
    /// Protected descendants keep their owning scope alive. Protection does not
    /// flow through an ancestor into an unrelated sibling. A closed scope always
    /// rejects further work, including through handles retained after completion.
    ///
    /// # Errors
    /// Returns cancellation when this scope may stop or its owner has closed.
    #[track_caller]
    pub fn bail_if_cancelled(&self) -> Result<(), CancellationError> {
        if self.state.is_stopping(&self.token) {
            Err(self.cancellation_error())
        } else {
            Ok(())
        }
    }

    pub(crate) fn cancellation_error(&self) -> CancellationError {
        if self.token.is_cancelled() {
            CancellationError::from_token(&self.token)
        } else {
            CancellationError::new("Cancellation scope is closed")
        }
    }

    /// Executes a linked child scope synchronously.
    ///
    /// The scope closes when the callback returns or unwinds. Returning an
    /// unpolled future from the callback does not extend that scope's lifetime.
    ///
    /// # Errors
    /// Returns cancellation before admission, or the callback's original error.
    #[track_caller]
    pub fn with_scope<C, T, E>(&self, callback: C) -> Result<T, E>
    where
        C: FnOnce(Self) -> Result<T, E>,
        E: From<CancellationError>,
    {
        self.child()
            .run_callback(CancellationScopeKind::Cancellable, None, callback)
    }

    /// Executes a bounded protected child synchronously.
    ///
    /// Admission respects an existing cancellation request unless this is part
    /// of an already-protected ancestor. Protection ends on return or unwind.
    /// Use [`Self::finish_required_sync`] for cleanup already owed after a request.
    ///
    /// # Errors
    /// Returns cancellation before admission, or the callback's original error.
    #[cfg(feature = "deferral")]
    #[track_caller]
    pub fn with_deferral<C, T, E>(
        &self,
        explanation: impl Into<String>,
        callback: C,
    ) -> Result<T, E>
    where
        C: FnOnce(Self) -> Result<T, E>,
        E: From<CancellationError>,
    {
        self.child().run_callback(
            CancellationScopeKind::Deferred,
            Some(explanation.into()),
            callback,
        )
    }

    /// Finishes already-required synchronous work despite requested cancellation.
    ///
    /// This bypasses requested cancellation at admission, but cannot reopen a
    /// closed owner. Prefer protecting a mutation and its cleanup together before
    /// starting the mutation. Ordinary errors and explicit owner drops still apply.
    ///
    /// # Errors
    /// Returns cancellation for a closed owner, or the callback's original error.
    #[cfg(feature = "deferral")]
    #[track_caller]
    pub fn finish_required_sync<C, T, E>(
        &self,
        explanation: impl Into<String>,
        callback: C,
    ) -> Result<T, E>
    where
        C: FnOnce(Self) -> Result<T, E>,
        E: From<CancellationError>,
    {
        self.child().run_callback(
            CancellationScopeKind::Required,
            Some(explanation.into()),
            callback,
        )
    }

    #[track_caller]
    pub(crate) fn run_callback<C, T, E>(
        self,
        kind: CancellationScopeKind,
        explanation: Option<String>,
        callback: C,
    ) -> Result<T, E>
    where
        C: FnOnce(Self) -> Result<T, E>,
        E: From<CancellationError>,
    {
        let mut owner = CancellationScopeOwner::new(self);
        if !owner.start(kind, explanation, Location::caller()) {
            return Err(owner.scope().cancellation_error().into());
        }
        callback(owner.scope().clone())
    }

    /// Owns and polls a linked, independently cancellable child operation.
    ///
    /// Use the factory's supplied child scope for its operation implementations;
    /// capturing an ancestor instead skips this wrapper's ownership relationship.
    /// The factory is called only after admission on the first poll. Deeply
    /// nested required completion keeps this wrapper and its owning ancestors
    /// polling. An unrelated sibling's protection does not postpone this child.
    ///
    /// Only library-generated cancellation is postponed. An ordinary error,
    /// explicit drop, fail-fast combinator, or task abort can still destroy work.
    #[cfg(feature = "async")]
    #[track_caller]
    pub fn cancellable_scope<C, F, T, E>(&self, factory: C) -> CancellableScopeFuture<C, F>
    where
        C: FnOnce(Self) -> F,
        F: IntoFuture<Output = Result<T, E>>,
        E: From<CancellationError>,
    {
        CancellableScopeFuture::new(
            self.child(),
            CancellationScopeKind::Cancellable,
            None,
            factory,
            Location::caller(),
        )
    }

    /// Creates a linked child with an independently requestable cancellation signal.
    ///
    /// The returned token controls this child and its descendants; requesting it
    /// does not cancel the parent or siblings. Parent requests still propagate.
    /// Its future has the same ownership links and lazy admission as
    /// `cancellable_scope`, so required completion retains enclosing owners.
    #[cfg(all(feature = "async", feature = "inheritance"))]
    #[track_caller]
    pub fn cancellable_child_scope<C, F, T, E>(
        &self,
        factory: C,
    ) -> (CancellationToken, CancellableScopeFuture<C, F>)
    where
        C: FnOnce(Self) -> F,
        F: IntoFuture<Output = Result<T, E>>,
        E: From<CancellationError>,
    {
        let token = self.token.child_token();
        let scope = Self {
            token: token.clone(),
            state: self.state.child(),
        };
        let future = CancellableScopeFuture::new(
            scope,
            CancellationScopeKind::Cancellable,
            None,
            factory,
            Location::caller(),
        );
        (token, future)
    }

    /// Owns a linked async lifetime without automatically abandoning its body.
    ///
    /// Admission still observes cancellation. Once admitted, the body owns its
    /// checkpoints and cleanup; child cancellable scopes can stop pending I/O.
    /// This is appropriate for application lifetimes and shutdown coordinators.
    #[cfg(feature = "async")]
    #[track_caller]
    pub fn with_async_scope<C, F, T, E>(&self, factory: C) -> CancellableScopeFuture<C, F>
    where
        C: FnOnce(Self) -> F,
        F: IntoFuture<Output = Result<T, E>>,
        E: From<CancellationError>,
    {
        self.cancellable_scope(factory).cooperative()
    }

    /// Owns a cooperative child lifetime with an independently requestable signal.
    ///
    /// This combines `with_async_scope` with `cancellable_child_scope`'s signal
    /// separation. The body must run its shutdown work after a cancellation.
    #[cfg(all(feature = "async", feature = "inheritance"))]
    #[track_caller]
    pub fn with_async_child_scope<C, F, T, E>(
        &self,
        factory: C,
    ) -> (CancellationToken, CancellableScopeFuture<C, F>)
    where
        C: FnOnce(Self) -> F,
        F: IntoFuture<Output = Result<T, E>>,
        E: From<CancellationError>,
    {
        let (token, future) = self.cancellable_child_scope(factory);
        (token, future.cooperative())
    }

    /// Owns a bounded protected future and its required-completion relationship.
    ///
    /// Protection starts at first-poll admission and persists across suspension.
    /// It shields this subtree and retains all enclosing scope wrappers, without
    /// shielding ordinary sibling scopes. Existing cancellation prevents new
    /// optional protected work; an already-protected ancestor may continue it.
    ///
    /// The body's original result is preserved. Pending cancellation remains
    /// observable at the next scope checkpoint or child-operation admission.
    /// The returned future must itself remain owned and polled to make progress.
    #[cfg(all(feature = "async", feature = "deferral"))]
    #[track_caller]
    pub fn deferred<C, F, T, E>(
        &self,
        explanation: impl Into<String>,
        factory: C,
    ) -> CancellableScopeFuture<C, F>
    where
        C: FnOnce(Self) -> F,
        F: IntoFuture<Output = Result<T, E>>,
        E: From<CancellationError>,
    {
        CancellableScopeFuture::new(
            self.child(),
            CancellationScopeKind::Deferred,
            Some(explanation.into()),
            factory,
            Location::caller(),
        )
    }

    /// Owns already-required async cleanup, allowing admission after cancellation.
    ///
    /// This has [`Self::deferred`]'s protection and lifetime, but bypasses raw
    /// cancellation at admission. It still rejects a closed owning scope.
    #[cfg(all(feature = "async", feature = "deferral"))]
    #[track_caller]
    pub fn finish_required<C, F, T, E>(
        &self,
        explanation: impl Into<String>,
        factory: C,
    ) -> CancellableScopeFuture<C, F>
    where
        C: FnOnce(Self) -> F,
        F: IntoFuture<Output = Result<T, E>>,
        E: From<CancellationError>,
    {
        CancellableScopeFuture::new(
            self.child(),
            CancellationScopeKind::Required,
            Some(explanation.into()),
            factory,
            Location::caller(),
        )
    }
}

impl CancellationToken {
    /// Owns a root async scope while leaving early return and cleanup to its body.
    ///
    /// Use cancellable children for operations safe to abandon. The root owner
    /// remains live across suspension so the application can restore resources
    /// and drain required work after a request. Pre-admission cancellation still
    /// prevents the factory from running.
    #[cfg(feature = "async")]
    #[track_caller]
    pub fn with_async_scope<C, F, T, E>(&self, factory: C) -> CancellableScopeFuture<C, F>
    where
        C: FnOnce(CancellationScope) -> F,
        F: IntoFuture<Output = Result<T, E>>,
        E: From<CancellationError>,
    {
        self.cancellable_scope(factory).cooperative()
    }

    /// Executes a root execution scope synchronously.
    ///
    /// Independent calls share cancellation intent and have independent execution
    /// ownership. Nested operations should use the supplied scope's helpers.
    ///
    /// # Errors
    /// Returns cancellation before admission, or the callback's original error.
    #[track_caller]
    pub fn with_scope<C, T, E>(&self, callback: C) -> Result<T, E>
    where
        C: FnOnce(CancellationScope) -> Result<T, E>,
        E: From<CancellationError>,
    {
        CancellationScope::root(self.clone()).run_callback(
            CancellationScopeKind::Cancellable,
            None,
            callback,
        )
    }

    /// Owns a root cancellable future and supplies its execution scope.
    ///
    /// Cancellation already requested before first-poll admission prevents the
    /// factory from running. Use the supplied scope to create linked descendants;
    /// starting another root from a raw token establishes independent ownership.
    ///
    /// Checks occur at poll boundaries, rather than every syntactic await. A
    /// completed body's original result wins over concurrent cancellation.
    #[cfg(feature = "async")]
    #[track_caller]
    pub fn cancellable_scope<C, F, T, E>(&self, factory: C) -> CancellableScopeFuture<C, F>
    where
        C: FnOnce(CancellationScope) -> F,
        F: IntoFuture<Output = Result<T, E>>,
        E: From<CancellationError>,
    {
        CancellableScopeFuture::new(
            CancellationScope::root(self.clone()),
            CancellationScopeKind::Cancellable,
            None,
            factory,
            Location::caller(),
        )
    }
}
