use crate::CancellationError;
use crate::CancellationScope;
use crate::CancellationWait;
use crate::cancellation_scope_kind::CancellationScopeKind;
use crate::cancellation_scope_owner::CancellationScopeOwner;
use std::fmt;
use std::panic::Location;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

/// Owns one lazily admitted future and its linked cancellation execution scope.
///
/// Protected descendants keep owning wrappers polling across suspension. The
/// caller must still retain this future: explicit drop, ordinary errors, task
/// aborts, and fail-fast combinators are not postponed. Parallel required work
/// needs a coordinator that deliberately drains it on those paths.
///
/// The inner future stays owned until this wrapper is dropped, even after a
/// result is returned. This preserves outcome/destructor ordering and borrowed
/// sources must remain valid until the wrapper is dropped.
#[must_use = "Futures do nothing unless polled or awaited"]
pub struct CancellableScopeFuture<C, F: IntoFuture> {
    owner: CancellationScopeOwner,
    kind: CancellationScopeKind,
    explanation: Option<String>,
    caller: &'static Location<'static>,
    factory: Option<Box<C>>,
    future: Option<Pin<Box<F::IntoFuture>>>,
    completed: bool,
    auto_cancel: bool,
    cancelled: Option<Pin<Box<CancellationWait>>>,
}

impl<C, F: IntoFuture> CancellableScopeFuture<C, F> {
    pub(crate) fn new(
        scope: CancellationScope,
        kind: CancellationScopeKind,
        explanation: Option<String>,
        factory: C,
        caller: &'static Location<'static>,
    ) -> Self {
        Self {
            cancelled: Some(Box::pin(scope.cancelled())),
            owner: CancellationScopeOwner::new(scope),
            kind,
            explanation,
            caller,
            factory: Some(Box::new(factory)),
            future: None,
            completed: false,
            auto_cancel: true,
        }
    }

    pub(crate) fn cooperative(mut self) -> Self {
        self.auto_cancel = false;
        self
    }

    fn stopping(&self) -> bool {
        let scope = self.owner.scope();
        if self.auto_cancel {
            scope.state.stop_if_cancelled(&scope.token)
        } else {
            scope.state.is_closed()
        }
    }

    fn finish(&mut self) {
        self.completed = true;
        self.cancelled.take();
        self.owner.finish();
        // Keep the user future/factory owned until Drop. Their destructors may
        // panic or depend on a result the caller has not yet retained.
    }
}

impl<C, F, T, E> Future for CancellableScopeFuture<C, F>
where
    C: FnOnce(CancellationScope) -> F,
    F: IntoFuture<Output = Result<T, E>>,
    E: From<CancellationError>,
{
    type Output = Result<T, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        assert!(
            !this.completed,
            "CancellableScopeFuture polled after completion"
        );

        if let Some(factory) = this.factory.take() {
            if !this
                .owner
                .start(this.kind, this.explanation.take(), this.caller)
            {
                let error = this.owner.scope().cancellation_error();
                // Do not destroy captured caller data in the rejected factory
                // until the wrapper is dropped, for the same ordering as futures.
                this.factory = Some(factory);
                this.finish();
                return Poll::Ready(Err(error.into()));
            }
            this.future = Some(Box::pin(factory(this.owner.scope().clone()).into_future()));
        }

        let scope = this.owner.scope();
        if this.stopping() {
            let error = scope.cancellation_error();
            this.finish();
            return Poll::Ready(Err(error.into()));
        }

        let result = this
            .future
            .as_mut()
            .expect("An admitted scope has a future")
            .as_mut()
            .poll(cx);
        if let Poll::Ready(result) = result {
            this.finish();
            return Poll::Ready(result);
        }

        if this
            .cancelled
            .as_mut()
            .is_some_and(|cancelled| cancelled.as_mut().poll(cx).is_ready())
        {
            this.cancelled.take();
        }
        let scope = this.owner.scope();
        // Registration precedes the final check: cancellation, owner closure,
        // or the last protected descendant finishing cannot lose its wakeup.
        scope.state.register_waker(cx.waker());
        if this.stopping() {
            let error = scope.cancellation_error();
            this.finish();
            Poll::Ready(Err(error.into()))
        } else {
            Poll::Pending
        }
    }
}

impl<C, F: IntoFuture> Drop for CancellableScopeFuture<C, F> {
    fn drop(&mut self) {
        // Invalidate saved handles before user-owned fields are destroyed.
        self.owner.finish();
    }
}

impl<C, F: IntoFuture> fmt::Debug for CancellableScopeFuture<C, F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancellableScopeFuture")
            .field("scope", self.owner.scope())
            .field("completed", &self.completed)
            .finish_non_exhaustive()
    }
}
