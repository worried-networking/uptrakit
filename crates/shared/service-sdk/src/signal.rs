//! Cross-platform signal handling abstraction.
//!
//! [`SignalWatcher`] encapsulates `SIGINT`, `SIGTERM`, and `SIGHUP` into a
//! single `recv()` call, removing `#[cfg(unix)]` blocks from service code.
//! On non-Unix platforms, only `Ctrl+C` (`SIGINT`) is supported.

use std::pin::Pin;
use std::task::{Context, Poll};

/// A signal received by the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// `SIGINT` (Ctrl+C).
    Interrupt,
    /// `SIGTERM`.
    Terminate,
    /// `SIGHUP` — used for graceful restart.
    Hangup,
}

impl std::fmt::Display for Signal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Signal::Interrupt => write!(f, "SIGINT"),
            Signal::Terminate => write!(f, "SIGTERM"),
            Signal::Hangup => write!(f, "SIGHUP"),
        }
    }
}

/// Watches for OS signals and delivers them as [`Signal`] values.
///
/// Create one instance per service and call [`recv()`](Self::recv) in a
/// `tokio::select!` branch.
pub struct SignalWatcher {
    #[cfg(unix)]
    sigint: tokio::signal::unix::Signal,
    #[cfg(unix)]
    sigterm: tokio::signal::unix::Signal,
    #[cfg(unix)]
    sighup: tokio::signal::unix::Signal,
    #[cfg(not(unix))]
    ctrl_c_fut: Pin<Box<dyn std::future::Future<Output = std::io::Result<()>> + Send>>,
}

impl SignalWatcher {
    /// Create a new signal watcher.
    ///
    /// On Unix this registers `SIGINT`, `SIGTERM`, and `SIGHUP` handlers.
    pub fn new() -> std::io::Result<Self> {
        #[cfg(unix)]
        {
            use tokio::signal::unix::SignalKind;
            let sigint = tokio::signal::unix::signal(SignalKind::interrupt())?;
            let sigterm = tokio::signal::unix::signal(SignalKind::terminate())?;
            let sighup = tokio::signal::unix::signal(SignalKind::hangup())?;
            Ok(Self {
                sigint,
                sigterm,
                sighup,
            })
        }
        #[cfg(not(unix))]
        {
            Ok(Self {
                ctrl_c_fut: Box::pin(tokio::signal::ctrl_c()),
            })
        }
    }

    /// Wait for the next signal.
    ///
    /// Returns the signal that was received. On non-Unix platforms, only
    /// [`Signal::Interrupt`] can be returned (SIGTERM/SIGHUP are not
    /// supported).
    pub async fn recv(&mut self) -> Signal {
        // Borrow each field separately to avoid overlapping mutable borrows
        // of `self` inside `tokio::select!`.
        #[cfg(unix)]
        {
            let sigint = &mut self.sigint;
            let sigterm = &mut self.sigterm;
            let sighup = &mut self.sighup;
            tokio::select! {
                biased;
                _ = sigint.recv() => Signal::Interrupt,
                _ = sigterm.recv() => Signal::Terminate,
                _ = sighup.recv() => Signal::Hangup,
            }
        }
        #[cfg(not(unix))]
        {
            // Only SIGINT is available on non-Unix.
            tokio::signal::ctrl_c().await.ok();
            Signal::Interrupt
        }
    }

    /// Poll for the next OS signal without blocking.
    ///
    /// Returns `Poll::Ready(signal)` when a signal is received, or
    /// `Poll::Pending` when no signal is available. On Unix, signals
    /// are polled in fixed-priority order: `SIGINT` > `SIGTERM` >
    /// `SIGHUP`. When multiple signals are pending, the highest-priority
    /// signal is returned first; remaining signals are delivered on
    /// subsequent calls.
    ///
    /// # Platform support
    ///
    /// On Unix, all three signals (`SIGINT`, `SIGTERM`, `SIGHUP`) are
    /// polled via `tokio::signal::unix::Signal::poll_recv`. On non-Unix,
    /// only `Signal::Interrupt` can be returned via an unvalidated
    /// best-effort fallback that polls a stored `ctrl_c()` future.
    pub fn poll_signal(&mut self, cx: &mut Context<'_>) -> Poll<Signal> {
        #[cfg(unix)]
        {
            // SIGINT — highest priority.
            match Pin::new(&mut self.sigint).poll_recv(cx) {
                Poll::Ready(Some(())) => return Poll::Ready(Signal::Interrupt),
                Poll::Ready(None) => {
                    tracing::error!(
                        "SIGINT signal driver shut down, triggering synthetic shutdown"
                    );
                    return Poll::Ready(Signal::Interrupt);
                }
                Poll::Pending => {}
            }

            // SIGTERM.
            match Pin::new(&mut self.sigterm).poll_recv(cx) {
                Poll::Ready(Some(())) => return Poll::Ready(Signal::Terminate),
                Poll::Ready(None) => {
                    tracing::error!(
                        "SIGTERM signal driver shut down, triggering synthetic shutdown"
                    );
                    return Poll::Ready(Signal::Interrupt);
                }
                Poll::Pending => {}
            }

            // SIGHUP — lowest priority.
            match Pin::new(&mut self.sighup).poll_recv(cx) {
                Poll::Ready(Some(())) => return Poll::Ready(Signal::Hangup),
                Poll::Ready(None) => {
                    tracing::error!(
                        "SIGHUP signal driver shut down, triggering synthetic shutdown"
                    );
                    return Poll::Ready(Signal::Interrupt);
                }
                Poll::Pending => {}
            }

            Poll::Pending
        }
        #[cfg(not(unix))]
        {
            match std::future::Future::poll(self.ctrl_c_fut.as_mut(), cx) {
                Poll::Ready(Ok(())) => {
                    self.ctrl_c_fut = Box::pin(tokio::signal::ctrl_c());
                    Poll::Ready(Signal::Interrupt)
                }
                Poll::Ready(Err(error)) => {
                    tracing::error!(
                        error = %error,
                        "ctrl_c handler failed, triggering synthetic shutdown"
                    );
                    self.ctrl_c_fut = Box::pin(tokio::signal::ctrl_c());
                    Poll::Ready(Signal::Interrupt)
                }
                Poll::Pending => Poll::Pending,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use nix::sys::signal::{self as nix_signal, Signal as NixSignal};
    #[cfg(unix)]
    use nix::unistd::getpid;
    #[cfg(unix)]
    use std::future::poll_fn;
    #[cfg(unix)]
    use std::time::Duration;
    #[cfg(unix)]
    static UNIX_SIGNAL_TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    #[test]
    fn signal_display() {
        assert_eq!(Signal::Interrupt.to_string(), "SIGINT");
        assert_eq!(Signal::Terminate.to_string(), "SIGTERM");
        assert_eq!(Signal::Hangup.to_string(), "SIGHUP");
    }

    #[test]
    fn signal_equality() {
        assert_eq!(Signal::Interrupt, Signal::Interrupt);
        assert_ne!(Signal::Interrupt, Signal::Terminate);
    }

    #[tokio::test]
    async fn signal_watcher_new() {
        let watcher = SignalWatcher::new();
        assert!(watcher.is_ok());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn poll_signal_pending_when_no_signal() {
        let _guard = UNIX_SIGNAL_TEST_MUTEX.lock().await;
        let mut watcher = SignalWatcher::new().expect("failed to create signal watcher");
        let result = poll_fn(|cx| Poll::Ready(watcher.poll_signal(cx))).await;
        assert!(result.is_pending());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn poll_signal_returns_sigterm() {
        let _guard = UNIX_SIGNAL_TEST_MUTEX.lock().await;
        let mut watcher = SignalWatcher::new().expect("failed to create signal watcher");
        nix_signal::kill(getpid(), NixSignal::SIGTERM).expect("failed to send SIGTERM");
        tokio::task::yield_now().await;

        let signal = poll_fn(|cx| watcher.poll_signal(cx)).await;
        assert_eq!(signal, Signal::Terminate);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn poll_signal_returns_sighup() {
        let _guard = UNIX_SIGNAL_TEST_MUTEX.lock().await;
        let mut watcher = SignalWatcher::new().expect("failed to create signal watcher");
        nix_signal::kill(getpid(), NixSignal::SIGHUP).expect("failed to send SIGHUP");
        tokio::task::yield_now().await;

        let signal = poll_fn(|cx| watcher.poll_signal(cx)).await;
        assert_eq!(signal, Signal::Hangup);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn poll_signal_priority_sigint_over_sigterm() {
        let _guard = UNIX_SIGNAL_TEST_MUTEX.lock().await;
        let mut watcher = SignalWatcher::new().expect("failed to create signal watcher");
        nix_signal::kill(getpid(), NixSignal::SIGTERM).expect("failed to send SIGTERM");
        nix_signal::kill(getpid(), NixSignal::SIGINT).expect("failed to send SIGINT");
        // Allow both process-level signals to reach Tokio signal streams so this
        // test validates priority rather than delivery timing.
        tokio::time::sleep(Duration::from_millis(10)).await;

        let first = poll_fn(|cx| watcher.poll_signal(cx)).await;
        let second = poll_fn(|cx| watcher.poll_signal(cx)).await;

        assert_eq!(first, Signal::Interrupt);
        assert_eq!(second, Signal::Terminate);
    }
}
