use std::sync::Mutex;
use std::task::Waker;

/// The current executor waker for one independently owned cancellation wait.
#[derive(Default)]
pub(crate) struct CancellationWaker {
    waker: Mutex<Option<Waker>>,
}

impl CancellationWaker {
    pub(crate) fn register(&self, waker: &Waker) {
        let mut stored = self
            .waker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !stored
            .as_ref()
            .is_some_and(|stored| stored.will_wake(waker))
        {
            *stored = Some(waker.clone());
        }
    }

    pub(crate) fn wake(&self) {
        let waker = self
            .waker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}
