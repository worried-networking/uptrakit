//! Shutdown signal abstraction for the service event loop.
//!
//! [`ShutdownSignal`] is a poll-based trait that the event loop uses to
//! detect shutdown requests. Two implementors are provided:
//!
//! - [`SignalShutdown`] wraps [`SignalWatcher`](crate::signal::SignalWatcher)
//!   for standalone services that shut down on OS signals.
//! - [`TokenShutdown`] wraps a
//!   [`CancellationToken`](tokio_util::sync::CancellationToken) for embedded
//!   services that shut down when the host cancels a token.

use std::pin::Pin;
use std::task::{Context, Poll};

use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};

use crate::shared_types::ShutdownCause;
use crate::signal::SignalWatcher;

/// A source of shutdown signals for the service event loop.
///
/// Implementors provide a poll-based interface that the event loop calls on
/// each iteration to check whether shutdown has been requested.
///
/// # Post-completion behavior
///
/// Callers should treat the first `Poll::Ready` as the definitive shutdown
/// cause and should not rely on specific behavior from subsequent polls.
/// `TokenShutdown` is idempotent (returns the same cause on every poll after
/// cancellation). `SignalShutdown` may return additional signals on
/// subsequent polls.
pub trait ShutdownSignal: Send {
    /// Poll for a shutdown signal.
    ///
    /// Returns `Poll::Ready(cause)` when shutdown is requested, or
    /// `Poll::Pending` when no shutdown signal is available.
    fn poll_shutdown(&mut self, cx: &mut Context<'_>) -> Poll<ShutdownCause>;
}

/// Shutdown signal source backed by OS signals via [`SignalWatcher`].
///
/// Wraps `SignalWatcher::poll_signal` and maps each [`crate::signal::Signal`] to
/// [`ShutdownCause::Signal`].
pub struct SignalShutdown {
    watcher: SignalWatcher,
}

impl SignalShutdown {
    /// Create a new `SignalShutdown` from an existing [`SignalWatcher`].
    pub fn new(watcher: SignalWatcher) -> Self {
        Self { watcher }
    }

    /// Create a new `SignalShutdown`, registering OS signal handlers.
    pub fn from_default() -> std::io::Result<Self> {
        Ok(Self::new(SignalWatcher::new()?))
    }
}

impl ShutdownSignal for SignalShutdown {
    fn poll_shutdown(&mut self, cx: &mut Context<'_>) -> Poll<ShutdownCause> {
        self.watcher.poll_signal(cx).map(ShutdownCause::Signal)
    }
}

/// Shutdown signal source backed by a [`CancellationToken`].
///
/// Used by embedded services where the host process signals shutdown by
/// cancelling a token rather than sending an OS signal.
///
/// `poll_shutdown` is idempotent: once the token is cancelled, every
/// subsequent poll returns `Poll::Ready` with the same [`ShutdownCause`].
pub struct TokenShutdown {
    fut: Pin<Box<WaitForCancellationFutureOwned>>,
    cause: ShutdownCause,
}

impl TokenShutdown {
    /// Create a new `TokenShutdown`.
    ///
    /// `token` is consumed by [`CancellationToken::cancelled_owned`] to
    /// produce an owned future. If the caller needs to retain a handle to the
    /// token, clone it before passing it here.
    pub fn new(token: CancellationToken, cause: ShutdownCause) -> Self {
        Self {
            fut: Box::pin(token.cancelled_owned()),
            cause,
        }
    }
}

impl ShutdownSignal for TokenShutdown {
    fn poll_shutdown(&mut self, cx: &mut Context<'_>) -> Poll<ShutdownCause> {
        self.fut.as_mut().poll(cx).map(|()| self.cause)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::signal::Signal;
    use std::future::poll_fn;
    use std::task::Poll;

    use nix::sys::signal::{self as nix_signal, Signal as NixSignal};
    use nix::unistd::getpid;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn token_shutdown_pending_before_cancellation() {
        let token = CancellationToken::new();
        let mut shutdown =
            TokenShutdown::new(token.clone(), ShutdownCause::Signal(Signal::Terminate));

        let result = poll_fn(|cx| Poll::Ready(shutdown.poll_shutdown(cx))).await;
        assert!(result.is_pending());
    }

    #[tokio::test]
    async fn token_shutdown_ready_after_cancellation() {
        let token = CancellationToken::new();
        let mut shutdown =
            TokenShutdown::new(token.clone(), ShutdownCause::Signal(Signal::Terminate));
        token.cancel();

        let cause = poll_fn(|cx| shutdown.poll_shutdown(cx)).await;
        assert_eq!(cause, ShutdownCause::Signal(Signal::Terminate));
    }

    #[tokio::test]
    async fn token_shutdown_idempotent_after_cancellation() {
        let token = CancellationToken::new();
        let mut shutdown = TokenShutdown::new(token.clone(), ShutdownCause::ServerRestarting);
        token.cancel();

        let first = poll_fn(|cx| shutdown.poll_shutdown(cx)).await;
        let second = poll_fn(|cx| shutdown.poll_shutdown(cx)).await;

        assert_eq!(first, ShutdownCause::ServerRestarting);
        assert_eq!(second, ShutdownCause::ServerRestarting);
    }

    #[tokio::test]
    async fn token_shutdown_pre_cancelled() {
        let token = CancellationToken::new();
        token.cancel();
        let mut shutdown = TokenShutdown::new(token, ShutdownCause::Signal(Signal::Interrupt));

        let cause = poll_fn(|cx| shutdown.poll_shutdown(cx)).await;
        assert_eq!(cause, ShutdownCause::Signal(Signal::Interrupt));
    }

    #[tokio::test]
    async fn token_shutdown_different_causes() {
        let token = CancellationToken::new();
        let mut shutdown = TokenShutdown::new(token.clone(), ShutdownCause::ServerRestarting);
        token.cancel();

        let cause = poll_fn(|cx| shutdown.poll_shutdown(cx)).await;
        assert_eq!(cause, ShutdownCause::ServerRestarting);

        let token2 = CancellationToken::new();
        let mut shutdown2 =
            TokenShutdown::new(token2.clone(), ShutdownCause::Signal(Signal::Hangup));
        token2.cancel();

        let cause2 = poll_fn(|cx| shutdown2.poll_shutdown(cx)).await;
        assert_eq!(cause2, ShutdownCause::Signal(Signal::Hangup));
    }

    #[tokio::test]
    async fn shutdown_signal_object_safety() {
        let token = CancellationToken::new();
        let mut shutdown =
            TokenShutdown::new(token.clone(), ShutdownCause::Signal(Signal::Terminate));
        let dyn_shutdown: &mut dyn ShutdownSignal = &mut shutdown;

        let result = poll_fn(|cx| Poll::Ready(dyn_shutdown.poll_shutdown(cx))).await;
        assert!(result.is_pending());

        token.cancel();

        let cause = poll_fn(|cx| dyn_shutdown.poll_shutdown(cx)).await;
        assert_eq!(cause, ShutdownCause::Signal(Signal::Terminate));
    }

    #[tokio::test]
    async fn signal_shutdown_maps_signal_to_cause() {
        let mut shutdown = SignalShutdown::from_default().expect("failed to create signal watcher");
        nix_signal::kill(getpid(), NixSignal::SIGTERM).expect("failed to send SIGTERM");
        tokio::task::yield_now().await;

        let cause = poll_fn(|cx| shutdown.poll_shutdown(cx)).await;
        assert_eq!(cause, ShutdownCause::Signal(Signal::Terminate));
    }
}
