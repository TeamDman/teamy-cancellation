use crate::CancellationToken;
use crate::cancellation_registration::CancellationRegistration;
use crate::cancellation_waker::CancellationWaker;
use std::fmt;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;

/// A sticky signal observing cancellation on this token or any ancestor.
///
/// Each wait owns its own waker registration. Dropping a pending wait removes
/// its registrations. This observes raw cancellation intent even when an
/// operation-local deferral postpones cooperative stopping.
#[must_use = "Futures do nothing unless polled or awaited"]
pub struct CancellationWait {
    token: CancellationToken,
    waker: Arc<CancellationWaker>,
    registrations: Option<Vec<CancellationRegistration>>,
}

impl CancellationWait {
    pub(crate) fn new(token: CancellationToken) -> Self {
        Self {
            token,
            waker: Arc::default(),
            registrations: None,
        }
    }
}

impl Future for CancellationWait {
    type Output = ();

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if this.token.is_cancelled() {
            this.registrations.take();
            return Poll::Ready(());
        }

        this.waker.register(context.waker());
        if this.registrations.is_none() {
            this.registrations = Some(this.token.register_waiter(&this.waker));
        }
        // A request racing registration is sticky, so the second check closes
        // the gap even if its wake snapshot did not yet include this waiter.
        if this.token.is_cancelled() {
            this.registrations.take();
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

impl fmt::Debug for CancellationWait {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CancellationWait")
            .field("token", &self.token)
            .finish_non_exhaustive()
    }
}
