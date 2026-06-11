//! Exponential backoff with jitter for reconnect loops.
//!
//! API surface: 4 methods on [`Backoff`]: [`new`](Backoff::new),
//! [`reset`](Backoff::reset), [`escalate`](Backoff::escalate),
//! [`sample_base_jitter`](Backoff::sample_base_jitter).
//!
//! The plain-method API avoids guard ceremony. Every call site explicitly
//! chooses the verb (`reset` on healthy cycles, `escalate` on unhealthy ones)
//! with an inline `// reset chosen: <reason>` / `// escalate chosen: <reason>`
//! comment as the audit log. Verb choice is a call-site responsibility;
//! `#[must_use]` on `escalate`'s return ensures the caller uses the delay.

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
/// # Example
///
/// ```rust
/// use std::time::Duration;
/// use uptrakit_backoff::Backoff;
///
/// let mut backoff = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));
/// // Healthy cycle:
/// backoff.reset();
/// // Unhealthy cycle:
/// let delay = backoff.escalate();
/// assert!(delay >= Duration::from_secs(2));
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

    /// Set `current` to `base`. Call when the backoff cycle was healthy —
    /// work returned `Ok`, or work returned `Err` after reaching a meaningful
    /// application-level milestone (e.g. WebSocket upgrade completed before a
    /// server-initiated close).
    ///
    /// Returns `()` — most success paths break out of the loop without
    /// sleeping, so a `Duration` return would be discarded.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::time::Duration;
    /// use uptrakit_backoff::Backoff;
    ///
    /// let mut backoff = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));
    /// // Simulate prior escalation, then reset on healthy cycle:
    /// let _advance = backoff.escalate(); // advance to 4s
    /// backoff.reset();
    /// // current is now base (2s).
    /// let delay = backoff.sample_base_jitter();
    /// assert!(delay >= Duration::from_secs(2));
    /// assert!(delay <= Duration::from_millis(2500));
    /// ```
    pub fn reset(&mut self) {
        tracing::trace!(base_ms = self.base.as_millis() as u64, "backoff reset");
        self.current = self.base;
    }

    /// Sample a delay from the **pre-escalation** `current + jitter`, then
    /// advance `current` to `min(current * 2, max)`. Returns the sampled
    /// delay — the caller should `sleep(delay).await` before the next attempt.
    ///
    /// Call when the backoff cycle was unhealthy — fast-fail, no meaningful
    /// milestone reached (TCP refused, DNS error, pre-upgrade transient).
    ///
    /// The order is: **sample pre-escalation `current + jitter`**, then
    /// **advance `current`**, then **return the sample**. This means the
    /// returned delay reflects the current window, not the next one.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::time::Duration;
    /// let mut b = uptrakit_backoff::Backoff::new(Duration::from_secs(1), Duration::from_secs(60));
    /// let delay = b.escalate();  // bound; compiles fine
    /// assert!(delay >= Duration::from_secs(1));
    /// ```
    ///
    /// Dropping the return is a compile error with `#[deny(unused_must_use)]`:
    ///
    /// ```compile_fail
    /// #![deny(unused_must_use)]
    /// use std::time::Duration;
    /// let mut b = uptrakit_backoff::Backoff::new(Duration::from_secs(1), Duration::from_secs(60));
    /// b.escalate(); // ERROR: unused `Duration` that must be used
    /// ```
    #[must_use = "the returned Duration is the delay to sleep before the next attempt"]
    pub fn escalate(&mut self) -> Duration {
        let delay = sample_jitter(self.current);
        self.current = (self.current * 2).min(self.max);
        tracing::trace!(
            next_ms = self.current.as_millis() as u64,
            "backoff escalated"
        );
        delay
    }

    /// Sample a `base + jitter` delay **independent of `current`**.
    ///
    /// Returns a value in `[base, base + base/4]` regardless of how escalated
    /// the backoff state is. Use this when a caller has already called
    /// [`reset`](Self::reset) (so `current == base`) and needs the post-reset
    /// delay without further state mutation.
    ///
    /// Use this from any arm that just called `reset()` and needs the
    /// post-reset delay without spinning a second mutation. Today's consumers
    /// are the enrollment / reconnect partial-progress arms and the reconnect
    /// `LoopOutcome::Disconnected` arm (where the upstream `Ok` arm already
    /// called `reset()`).
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
    /// backoff.reset(); // current is now base (2s)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_sets_current_to_base() {
        let mut b = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));
        // Advance past base.
        let _walk = b.escalate(); // current = 4s
        let _walk = b.escalate(); // current = 8s
        assert_eq!(b.current, Duration::from_secs(8));
        b.reset();
        // After reset, sample_base_jitter should return base+jitter.
        let delay = b.sample_base_jitter();
        assert!(delay >= Duration::from_secs(2), "delay {delay:?} < base 2s");
        assert!(
            delay <= Duration::from_millis(2500),
            "delay {delay:?} > base+jitter 2.5s"
        );
        assert_eq!(
            b.current,
            Duration::from_secs(2),
            "current must equal base after reset"
        );
    }

    #[test]
    fn escalate_returns_current_plus_jitter_and_doubles_state() {
        let mut b = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));

        // First escalate: current=2s → returned delay in [2s, 2.5s], then current=4s.
        let d1 = b.escalate();
        assert!(d1 >= Duration::from_secs(2), "d1 {d1:?} < 2s");
        assert!(d1 <= Duration::from_millis(2500), "d1 {d1:?} > 2.5s");
        assert_eq!(
            b.current,
            Duration::from_secs(4),
            "current should be 4s after first escalate"
        );

        // Second escalate: current=4s → returned delay in [4s, 5s], then current=8s.
        let d2 = b.escalate();
        assert!(d2 >= Duration::from_secs(4), "d2 {d2:?} < 4s");
        assert!(d2 <= Duration::from_millis(5000), "d2 {d2:?} > 5s");
        assert_eq!(
            b.current,
            Duration::from_secs(8),
            "current should be 8s after second escalate"
        );
    }

    #[test]
    fn escalate_caps_at_max() {
        let mut b = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));
        // Escalate: 2→4→8→16→32→60(cap).
        // After 5 escalations: current = 64s clamped to 60s.
        for _ in 0..5 {
            let _walk = b.escalate();
        }
        assert_eq!(
            b.current,
            Duration::from_secs(60),
            "current must be capped at 60s"
        );

        // Subsequent escalates: returned delay must be in [60s, 75s]; current stays at cap.
        for _ in 0..3 {
            let delay = b.escalate();
            assert!(
                delay >= Duration::from_secs(60),
                "at-cap delay {delay:?} should be >= 60s"
            );
            assert!(
                delay <= Duration::from_millis(75_000),
                "at-cap delay {delay:?} exceeds cap+jitter 75s"
            );
            assert_eq!(
                b.current,
                Duration::from_secs(60),
                "current must remain at cap"
            );
        }
    }

    #[test]
    fn sample_base_jitter_samples_base_plus_jitter_without_state_change() {
        let mut b = Backoff::new(Duration::from_secs(8), Duration::from_secs(60));
        // Advance current away from base.
        let _walk = b.escalate(); // current = 16s
        let current_before = b.current;

        // N=20 calls; verify range and that jitter is re-sampled.
        let mut samples = Vec::new();
        for _ in 0..20 {
            let d = b.sample_base_jitter();
            assert!(d >= Duration::from_secs(8), "sample {d:?} < base 8s");
            assert!(
                d <= Duration::from_millis(10_000),
                "sample {d:?} > base+jitter 10s"
            );
            samples.push(d);
        }

        // Not all equal (jitter re-samples).
        let all_equal = samples.iter().all(|d| d == &samples[0]);
        assert!(
            !all_equal,
            "consecutive calls must return different values due to jitter; got: {:?}",
            samples
        );

        // State unchanged.
        assert_eq!(
            b.current, current_before,
            "sample_base_jitter must not mutate state"
        );
    }

    #[test]
    fn bug_regression_reset_at_cap_returns_base() {
        // Reproduces the user-reported bug: after escalating to cap,
        // reset() must return current to base, not inherit the cap.
        let mut b = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));

        // Escalate to cap.
        for _ in 0..10 {
            let _walk = b.escalate();
        }
        assert_eq!(b.current, Duration::from_secs(60), "expected cap");

        b.reset();
        assert_eq!(
            b.current,
            Duration::from_secs(2),
            "expected base after reset"
        );

        // sample_base_jitter after reset must return base range.
        let delay = b.sample_base_jitter();
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
