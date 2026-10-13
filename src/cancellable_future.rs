use crate::CancellationError;
use crate::CancellationToken;
use crate::CancellationWait;
use std::fmt;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

/// Cooperatively stops an explicitly cancellation-safe fallible future.
///
/// A cancellation request wakes the wrapper even when the operation is waiting
/// on unrelated I/O. With the `deferral` feature, a `CancellationDeferral`
/// in the same token scope can postpone stopping the operation.
/// Like ordinary futures, completed wrapper ownership is released when it is
/// dropped; callers must retain borrowed sources until then.
///
/// The wrapper cannot interrupt a running poll or inspect internal await points.
/// If the operation completes during its poll, its original result wins over a
/// concurrent cancellation request. Register required completion work before
/// entering the operation's irreversible phase.
#[must_use = "Futures do nothing unless polled or awaited"]
pub struct CancellableFuture<F: Future> {
    token: CancellationToken,
    future: Pin<Box<F>>,
    completed: bool,
    cancelled: Option<Pin<Box<CancellationWait>>>,
    #[cfg(feature = "deferral")]
    deferral_waiter: Option<usize>,
}

impl CancellationToken {
    /// Wraps a fallible operation that may be dropped when cancellation is
    /// requested.
    ///
    /// The operation may borrow caller-owned data and retains its original
    /// error type through `E: From<CancellationError>`. Cancellation already
    /// requested before the initial poll prevents the operation from starting.
    ///
    /// With the `deferral` feature, this wrapper uses this token's operation
    /// scope. Required completion work must enter `defer_cancellation()` using
    /// a clone of the same token. Use `for_operation()` when starting independent
    /// concurrent work; it creates a separate deferral scope.
    ///
    /// # Cancellation safety
    ///
    /// Without a live deferral, cancellation stops polling the operation.
    /// The inner future remains owned until this wrapper is dropped; keep its
    /// borrowed sources alive until then. The caller must opt in only when
    /// abandoning the operation is appropriate.
    /// External task aborts and dropping this wrapper are not postponed.
    pub fn cancellable<F, T, E>(&self, future: F) -> CancellableFuture<F::IntoFuture>
    where
        F: IntoFuture<Output = Result<T, E>>,
        E: From<CancellationError>,
    {
        CancellableFuture {
            token: self.clone(),
            future: Box::pin(future.into_future()),
            completed: false,
            cancelled: Some(Box::pin(self.cancelled())),
            #[cfg(feature = "deferral")]
            deferral_waiter: None,
        }
    }
}

impl<F, T, E> Future for CancellableFuture<F>
where
    F: Future<Output = Result<T, E>>,
    E: From<CancellationError>,
{
    type Output = Result<T, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        assert!(!this.completed, "CancellableFuture polled after completion");

        if this.token.should_cancel() {
            this.finish();
            return Poll::Ready(Err(CancellationError::from_token(&this.token).into()));
        }

        let result = this.future.as_mut().poll(cx);
        if let Poll::Ready(result) = result {
            this.finish();
            return Poll::Ready(result);
        }

        // Register wakeups before the final state check. A cancellation or,
        // with deferral enabled, final guard drop racing with registration is
        // caught by that check.
        if this
            .cancelled
            .as_mut()
            .is_some_and(|cancelled| cancelled.as_mut().poll(cx).is_ready())
        {
            this.cancelled.take();
        }
        #[cfg(feature = "deferral")]
        {
            this.deferral_waiter = Some(
                this.token
                    .deferral_state
                    .register_waker(this.deferral_waiter, cx.waker()),
            );
        }
        if this.token.should_cancel() {
            this.finish();
            Poll::Ready(Err(CancellationError::from_token(&this.token).into()))
        } else {
            Poll::Pending
        }
    }
}

impl<F: Future> CancellableFuture<F> {
    fn finish(&mut self) {
        #[cfg(feature = "deferral")]
        if let Some(id) = self.deferral_waiter.take() {
            self.token.deferral_state.unregister_waker(id);
        }
        self.cancelled.take();
        // Do not destroy user-owned futures inside poll: their destructor can
        // panic while a returned result still owns dependent or borrowed values.
        // The executor must first retain the outcome, then dispose this wrapper.
        self.completed = true;
    }
}

#[cfg(feature = "deferral")]
impl<F: Future> Drop for CancellableFuture<F> {
    fn drop(&mut self) {
        if let Some(id) = self.deferral_waiter.take() {
            self.token.deferral_state.unregister_waker(id);
        }
    }
}

impl<F: Future> fmt::Debug for CancellableFuture<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancellableFuture")
            .field("token", &self.token)
            .field("completed", &self.completed)
            .finish_non_exhaustive()
    }
}
