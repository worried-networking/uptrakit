//! Cross-platform signal handling abstraction.
//!
//! [`SignalWatcher`] encapsulates `SIGINT`, `SIGTERM`, and `SIGHUP` into a
//! single `recv()` call, removing `#[cfg(unix)]` blocks from service code.
//! On non-Unix platforms, `SIGTERM` and `SIGHUP` are replaced with
//! permanently pending futures.

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
    sigterm: tokio::signal::unix::Signal,
    #[cfg(unix)]
    sighup: tokio::signal::unix::Signal,
}

impl SignalWatcher {
    /// Create a new signal watcher.
    ///
    /// On Unix this registers `SIGTERM` and `SIGHUP` handlers.
    /// `SIGINT` (Ctrl+C) is handled via `tokio::signal::ctrl_c()`.
    pub fn new() -> std::io::Result<Self> {
        #[cfg(unix)]
        {
            use tokio::signal::unix::SignalKind;
            let sigterm = tokio::signal::unix::signal(SignalKind::terminate())?;
            let sighup = tokio::signal::unix::signal(SignalKind::hangup())?;
            Ok(Self { sigterm, sighup })
        }
        #[cfg(not(unix))]
        {
            Ok(Self {})
        }
    }

    /// Wait for the next signal.
    ///
    /// Returns the signal that was received. On non-Unix platforms, only
    /// [`Signal::Interrupt`] can be returned (SIGTERM/SIGHUP are not
    /// supported).
    pub async fn recv(&mut self) -> Signal {
        // Destructure to get separate borrows for each field, avoiding
        // the double `&mut self` borrow that `tokio::select!` would reject.
        #[cfg(unix)]
        {
            let sigterm = &mut self.sigterm;
            let sighup = &mut self.sighup;
            tokio::select! {
                biased;
                _ = tokio::signal::ctrl_c() => Signal::Interrupt,
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
