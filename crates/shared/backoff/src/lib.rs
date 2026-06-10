//! Exponential backoff with jitter for reconnect loops.
//!
//! The guard pattern forces explicit resolution of each attempt cycle so
//! partial-success bugs — where a healthy cycle erroneously escalates the
//! backoff — become loud at compile time (via `#[must_use]`) and at runtime
//! (via `Drop` warning) rather than silently inflating delays.
//!
//! # Quick start
//!
//! ```rust
//! use std::time::Duration;
//! use uptrakit_backoff::Backoff;
//!
//! let mut backoff = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));
//!
//! // Healthy cycle — reset so next attempt starts from base:
//! let guard = backoff.attempt();
//! guard.reset();
//!
//! // Unhealthy cycle — escalate so next attempt waits longer:
//! let guard = backoff.attempt();
//! let delay = guard.sample_delay();
//! guard.escalate();
//! // sleep(delay) here in a real loop
//! let _ = delay;
//! ```

#![warn(missing_docs)]

use std::time::Duration;

use rand::Rng;

/// Exponential backoff with jitter for reconnect loops.
///
/// Tracks a `current` delay that starts at `base` and doubles (capped at
/// `max`) on each unhealthy attempt cycle. A random jitter in the range
/// `[0, current/4]` is added to every sampled delay to prevent thundering
/// herd.
///
/// Use [`attempt`](Backoff::attempt) to begin a tracked cycle.  The returned
/// [`AttemptGuard`] **must** be resolved via [`reset`](AttemptGuard::reset) or
/// [`escalate`](AttemptGuard::escalate) before it is dropped.
///
/// # Example
///
/// ```rust
/// use std::time::Duration;
/// use uptrakit_backoff::Backoff;
///
/// let mut backoff = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));
/// let guard = backoff.attempt();
/// let delay = guard.sample_delay();
/// // work succeeded
/// guard.reset();
/// // delay is in [2s, 2.5s]
/// assert!(delay >= Duration::from_secs(2));
/// assert!(delay <= Duration::from_millis(2500));
/// ```
#[non_exhaustive]
pub struct Backoff {
    /// The delay used for the next attempt cycle.
    pub(crate) current: Duration,
    /// The minimum (base) delay.
    pub(crate) base: Duration,
    /// The maximum delay.
    pub(crate) max: Duration,
}

impl Backoff {
    /// Create a new `Backoff` starting at `base`, doubling up to `max`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::time::Duration;
    /// use uptrakit_backoff::Backoff;
    ///
    /// let backoff = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));
    /// ```
    pub fn new(base: Duration, max: Duration) -> Self {
        Self {
            current: base,
            base,
            max,
        }
    }

    /// Begin a tracked attempt cycle.
    ///
    /// Returns an [`AttemptGuard`] that **must** be resolved via
    /// [`reset`](AttemptGuard::reset) or [`escalate`](AttemptGuard::escalate)
    /// before it is dropped. Dropping an unresolved guard emits a `warn!` log
    /// record; state is **not mutated** on unresolved drop.
    ///
    /// Only one guard can be live at a time — the borrow checker enforces this.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::time::Duration;
    /// use uptrakit_backoff::Backoff;
    ///
    /// let mut backoff = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));
    /// let guard = backoff.attempt();
    /// guard.reset();
    /// ```
    pub fn attempt(&mut self) -> AttemptGuard<'_> {
        AttemptGuard {
            backoff: self,
            resolved: false,
        }
    }

    /// Sample a `base + jitter` delay **independent of `current`**.
    ///
    /// Returns a value in `[base, base + base/4]` regardless of how escalated
    /// the backoff state is.  Use this when a caller has already resolved a
    /// guard via [`reset`](AttemptGuard::reset) (so `current == base`) and
    /// needs the post-reset delay without spinning a fake `attempt()` cycle.
    ///
    /// Jitter is re-sampled on every call — two consecutive calls return
    /// different values; the `sample_` prefix is deliberate.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::time::Duration;
    /// use uptrakit_backoff::Backoff;
    ///
    /// let mut backoff = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));
    /// let guard = backoff.attempt();
    /// guard.reset(); // current is now base (2s)
    ///
    /// // Sample base+jitter without advancing state:
    /// let delay = backoff.sample_base_jitter();
    /// assert!(delay >= Duration::from_secs(2));
    /// assert!(delay <= Duration::from_millis(2500));
    /// ```
    pub fn sample_base_jitter(&self) -> Duration {
        sample_jitter(self.base)
    }
}

/// Compute `delay + jitter` where jitter is in `[0, delay/4]`.
fn sample_jitter(delay: Duration) -> Duration {
    let quarter_ms = delay.as_millis() as u64 / 4;
    let jitter = if quarter_ms > 0 {
        let jitter_ms = rand::rng().random_range(0..=quarter_ms);
        Duration::from_millis(jitter_ms)
    } else {
        Duration::ZERO
    };
    delay + jitter
}

/// A tracked attempt cycle guard for [`Backoff`].
///
/// Returned by [`Backoff::attempt`]. **Must** be resolved via
/// [`reset`](Self::reset) or [`escalate`](Self::escalate) before it is
/// dropped. Dropping an unresolved guard emits a `warn!` log record; the
/// backoff state is **not mutated** so the delay is preserved for the next
/// attempt — this avoids silently inflating delays after a `?`-driven early
/// exit.
///
/// # Verb semantics
///
/// - [`reset`](Self::reset): the cycle was **healthy**. Call when the work
///   returned `Ok`, or when the work returned `Err` but the cycle reached a
///   meaningful application-level milestone (e.g. the WebSocket upgrade
///   completed before a server-initiated close).  Sets `current` back to
///   `base`.
///
/// - [`escalate`](Self::escalate): the cycle was **unhealthy**. Call when the
///   attempt failed without reaching a meaningful milestone (e.g. TCP refused,
///   DNS error, pre-upgrade transient).  Doubles `current` up to `max`.
///
/// # One live guard at a time
///
/// Because `attempt` borrows `&mut Backoff`, the borrow checker statically
/// prevents two `AttemptGuard`s from coexisting:
///
/// ```compile_fail
/// use std::time::Duration;
/// use uptrakit_backoff::Backoff;
///
/// let mut backoff = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));
/// let g1 = backoff.attempt();
/// let g2 = backoff.attempt(); // error[E0499]: cannot borrow `backoff` as mutable more than once
/// g1.reset();
/// g2.reset();
/// ```
#[must_use = "AttemptGuard must be resolved via .reset() or .escalate()"]
pub struct AttemptGuard<'a> {
    backoff: &'a mut Backoff,
    resolved: bool,
}

impl<'a> AttemptGuard<'a> {
    /// Sample a delay for this attempt cycle (`current + jitter`).
    ///
    /// Does **not** advance backoff state. Jitter is re-sampled on every call —
    /// two consecutive calls return different values; the `sample_` prefix is
    /// deliberate. Store the result **before** resolving the guard:
    ///
    /// ```rust
    /// use std::time::Duration;
    /// use uptrakit_backoff::Backoff;
    ///
    /// let mut backoff = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));
    /// let guard = backoff.attempt();
    /// let delay = guard.sample_delay(); // store before consuming the guard
    /// guard.escalate();
    /// // now sleep `delay`
    /// ```
    pub fn sample_delay(&self) -> Duration {
        sample_jitter(self.backoff.current)
    }

    /// Resolve the cycle as healthy: set `current` to `base`.
    ///
    /// Call when the work returned `Ok`, or when the work returned `Err` but
    /// the cycle reached a meaningful application-level milestone (e.g. the
    /// WebSocket upgrade completed before a server-initiated close).
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::time::Duration;
    /// use uptrakit_backoff::Backoff;
    ///
    /// let mut backoff = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));
    /// // Simulate a healthy cycle:
    /// backoff.attempt().reset();
    /// // current is now base (2s) regardless of prior escalation.
    /// ```
    pub fn reset(mut self) {
        self.resolved = true;
        tracing::trace!(
            base_ms = self.backoff.base.as_millis() as u64,
            "backoff reset"
        );
        self.backoff.current = self.backoff.base;
    }

    /// Resolve the cycle as unhealthy: double `current`, capped at `max`.
    ///
    /// Call when the attempt failed without reaching a meaningful milestone
    /// (e.g. TCP refused, DNS error, pre-upgrade transient failure).
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::time::Duration;
    /// use uptrakit_backoff::Backoff;
    ///
    /// let mut backoff = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));
    /// let guard = backoff.attempt();
    /// let delay = guard.sample_delay();
    /// guard.escalate();
    /// // current is now 4s (doubled from 2s)
    /// ```
    pub fn escalate(mut self) {
        self.resolved = true;
        self.backoff.current = (self.backoff.current * 2).min(self.backoff.max);
        tracing::trace!(
            next_ms = self.backoff.current.as_millis() as u64,
            "backoff escalated"
        );
    }
}

impl Drop for AttemptGuard<'_> {
    fn drop(&mut self) {
        if !self.resolved && !std::thread::panicking() {
            tracing::warn!(
                "backoff guard dropped unresolved (state unchanged); \
                 resolve before any ? or early-return between attempt() and resolution"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::layer::SubscriberExt;

    use super::*;

    /// Newtype wrapping a channel sender, implementing `MakeWriter` for
    /// `tracing_subscriber::fmt::Layer`. Avoids orphan-rule violation.
    struct ChannelMakeWriter(Arc<Mutex<std::sync::mpsc::Sender<String>>>);

    struct ChannelWriter(Arc<Mutex<std::sync::mpsc::Sender<String>>>);

    impl std::io::Write for ChannelWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let s = String::from_utf8_lossy(buf).into_owned();
            #[expect(
                clippy::io_other_error,
                reason = "PoisonError doesn't implement Send+Sync for Error::other()"
            )]
            {
                self.0
                    .lock()
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?
                    .send(s)
                    .map_err(|_e| {
                        std::io::Error::new(std::io::ErrorKind::BrokenPipe, "receiver dropped")
                    })?;
            }
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for ChannelMakeWriter {
        type Writer = ChannelWriter;
        fn make_writer(&'a self) -> Self::Writer {
            ChannelWriter(Arc::clone(&self.0))
        }
    }

    fn make_channel_subscriber() -> (std::sync::mpsc::Receiver<String>, impl tracing::Subscriber) {
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let writer = ChannelMakeWriter(Arc::new(Mutex::new(tx)));
        let fmt_layer = tracing_subscriber::fmt::Layer::new()
            .with_writer(writer)
            .with_ansi(false);
        let subscriber = tracing_subscriber::Registry::default().with(fmt_layer);
        (rx, subscriber)
    }

    #[test]
    fn attempt_reset_sets_current_to_base() {
        let mut b = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));
        // Escalate to move current away from base.
        b.attempt().escalate(); // current = 4s
        b.attempt().escalate(); // current = 8s
        // Now reset via a guard — current returns to base.
        b.attempt().reset();
        // After reset, sample_delay should return base+jitter.
        let guard = b.attempt();
        let delay = guard.sample_delay();
        guard.reset();
        assert!(delay >= Duration::from_secs(2), "delay {delay:?} < base 2s");
        assert!(
            delay <= Duration::from_millis(2500),
            "delay {delay:?} > base+jitter 2.5s"
        );
    }

    #[test]
    fn attempt_escalate_doubles_with_cap() {
        let mut b = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));
        for _ in 0..6 {
            b.attempt().escalate();
        }
        // After 6 doublings: 2→4→8→16→32→60(cap)→60(cap). current = 60s.
        let guard = b.attempt();
        let delay = guard.sample_delay();
        guard.escalate(); // resolve
        assert!(
            delay >= Duration::from_secs(60),
            "delay {delay:?} should be >= 60s (cap)"
        );
        assert!(
            delay <= Duration::from_millis(75_000),
            "delay {delay:?} exceeds cap+jitter"
        );
    }

    #[test]
    fn dropping_unresolved_guard_warns_and_does_not_mutate_state() {
        let (rx, subscriber) = make_channel_subscriber();
        {
            let _guard = tracing::subscriber::set_default(subscriber);

            let mut b = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));
            // Escalate once so current != base, to prove state is not mutated.
            b.attempt().escalate(); // current = 4s
            let current_before = b.current;

            {
                // Drop without resolving.
                let _unresolved = b.attempt();
            }

            // State must be unchanged.
            assert_eq!(
                b.current, current_before,
                "state must not mutate on unresolved drop"
            );
        }
        // _guard drops here automatically, flushing subscriber

        let output: String = rx.try_iter().collect();
        assert!(
            output.contains("backoff guard dropped unresolved"),
            "expected warn log, got: {output:?}"
        );
    }

    #[test]
    fn dropping_unresolved_guard_during_panic_does_not_warn() {
        let (rx, subscriber) = make_channel_subscriber();
        {
            let _default_guard = tracing::subscriber::set_default(subscriber);

            let result = std::panic::catch_unwind(|| {
                let mut b = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));
                let _unresolved = b.attempt();
                panic!("inner panic");
            });

            assert!(result.is_err(), "catch_unwind must return Err");
        }
        // _default_guard drops here automatically, flushing subscriber

        let output: String = rx.try_iter().collect();
        assert!(
            !output.contains("backoff guard dropped unresolved"),
            "warn must NOT fire during panic unwind, got: {output:?}"
        );
    }

    #[test]
    fn sample_delay_does_not_advance_state() {
        let mut b = Backoff::new(Duration::from_secs(4), Duration::from_secs(60));
        let guard = b.attempt();
        // Call sample_delay multiple times — state must not change.
        let d1 = guard.sample_delay();
        let d2 = guard.sample_delay();
        let d3 = guard.sample_delay();
        // Each must be in [4s, 5s]
        for d in [d1, d2, d3] {
            assert!(d >= Duration::from_secs(4), "delay {d:?} < base");
            assert!(
                d <= Duration::from_millis(5000),
                "delay {d:?} > base+jitter"
            );
        }
        guard.reset(); // resolve
        // After reset, current = base = 4s still. Sample again.
        let g2 = b.attempt();
        let after = g2.sample_delay();
        g2.reset();
        assert!(
            after >= Duration::from_secs(4),
            "state advanced unexpectedly: {after:?}"
        );
    }

    #[test]
    fn sample_base_jitter_samples_base_plus_jitter_without_state_change() {
        let mut b = Backoff::new(Duration::from_secs(8), Duration::from_secs(60));
        // Escalate so current != base
        b.attempt().escalate(); // current = 16s
        let current_before = b.current;

        // Sample N=20 times and verify jitter is re-sampled (not all equal).
        let mut samples = Vec::new();
        for _ in 0..20 {
            let d = b.sample_base_jitter();
            assert!(d >= Duration::from_secs(8), "sample {d:?} < base");
            assert!(
                d <= Duration::from_millis(10_000),
                "sample {d:?} > base+jitter"
            );
            samples.push(d);
        }

        // Assert not all samples are equal (jitter is re-sampled on each call).
        let all_equal = samples.iter().all(|d| d == &samples[0]);
        assert!(
            !all_equal,
            "consecutive calls must return different values due to jitter; all samples: {:?}",
            samples
        );

        // State unchanged
        assert_eq!(
            b.current, current_before,
            "sample_base_jitter must not mutate state"
        );
    }

    #[test]
    fn bug_regression_reset_at_cap_returns_base() {
        // Reproduces the user-reported bug: after escalating to cap,
        // a reset() must return current to base, not inherit the cap.
        let mut b = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));

        // Escalate until capped.
        for _ in 0..10 {
            b.attempt().escalate();
        }
        assert_eq!(b.current, Duration::from_secs(60), "expected cap");

        // Reset via guard.
        b.attempt().reset();
        assert_eq!(
            b.current,
            Duration::from_secs(2),
            "expected base after reset"
        );

        // sample_delay after reset must be in base range.
        let guard = b.attempt();
        let delay = guard.sample_delay();
        guard.reset();
        assert!(
            delay >= Duration::from_secs(2),
            "delay {delay:?} < base after reset"
        );
        assert!(
            delay <= Duration::from_millis(2500),
            "delay {delay:?} > base+jitter after reset"
        );
    }
}
