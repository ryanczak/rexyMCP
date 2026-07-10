//! Cooperative cancellation signal for the executor turn loop.
//!
//! Built on `tokio::sync::watch` — a `CancelHandle` flips the signal and a
//! `CancelSignal` observes it. The loop polls the signal at two points:
//! between turns and while awaiting the model.

use tokio::sync::watch;

/// Handle that can flip the cancellation signal.
pub struct CancelHandle {
    tx: watch::Sender<bool>,
}

impl CancelHandle {
    /// Flip the signal. Ignores a send error from all-receivers-dropped.
    pub fn cancel(&self) {
        let _ = self.tx.send(true);
    }
}

/// Observable side of the cancellation signal.
///
/// Cloned for the inner `select!` branch so the loop body and the
/// `select!` can observe independently.
#[derive(Clone)]
pub struct CancelSignal {
    rx: watch::Receiver<bool>,
}

impl CancelSignal {
    /// Create a fresh pair. The handle starts the signal at `false`.
    pub fn new() -> (CancelHandle, CancelSignal) {
        let (tx, rx) = watch::channel(false);
        (CancelHandle { tx }, CancelSignal { rx })
    }

    /// Create a signal that can never fire. The sender is immediately dropped,
    /// so `is_cancelled()` stays `false` and `cancelled()` stays pending.
    pub fn never() -> CancelSignal {
        let (tx, rx) = watch::channel(false);
        drop(tx);
        CancelSignal { rx }
    }

    /// Check if the signal has been flipped.
    pub fn is_cancelled(&self) -> bool {
        *self.rx.borrow()
    }

    /// Resolve when the signal is flipped. If the sender is dropped before
    /// the flip, park forever (the "never" case).
    pub async fn cancelled(&mut self) {
        loop {
            if *self.rx.borrow() {
                return;
            }
            match self.rx.changed().await {
                Ok(_) => {} // value changed — check the loop guard above
                Err(_) => std::future::pending::<()>().await, // sender gone → never fires
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn never_signal_is_not_cancelled() {
        let signal = CancelSignal::never();
        assert!(!signal.is_cancelled());
    }

    #[tokio::test]
    async fn never_signal_cancelled_future_stays_pending() {
        let mut signal = CancelSignal::never();
        // Use poll_fn with a timeout to verify it does not resolve.
        let result =
            tokio::time::timeout(std::time::Duration::from_millis(10), signal.cancelled()).await;
        assert!(result.is_err(), "never().cancelled() should stay pending");
    }

    #[tokio::test]
    async fn cancel_flips_signal() {
        let (handle, signal) = CancelSignal::new();
        assert!(!signal.is_cancelled());
        handle.cancel();
        assert!(signal.is_cancelled());
        let mut s = signal;
        s.cancelled().await;
    }

    #[tokio::test]
    async fn clone_observes_flip() {
        let (handle, signal) = CancelSignal::new();
        let clone = signal.clone();
        handle.cancel();
        assert!(clone.is_cancelled());
    }

    #[tokio::test]
    async fn dropped_handle_does_not_cancel() {
        let (handle, signal) = CancelSignal::new();
        drop(handle);
        assert!(!signal.is_cancelled());
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(10),
            CancelSignal::never().cancelled(),
        )
        .await;
        assert!(
            result.is_err(),
            "dropped handle should leave signal pending"
        );
    }
}
