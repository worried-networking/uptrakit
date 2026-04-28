use std::time::Duration;

use rand::Rng;

/// Exponential backoff with jitter for reconnection delays.
///
/// Doubles the delay on each call to [`next_delay`], capped at `max`.
/// Adds random jitter in the range `[0, delay/4]` to prevent thundering herd.
/// Call [`reset`] after a successful connection to return to the base delay.
pub struct Backoff {
    current: Duration,
    base: Duration,
    max: Duration,
}

impl Backoff {
    /// Create a new backoff starting at `base`, doubling up to `max`.
    pub fn new(base: Duration, max: Duration) -> Self {
        Self {
            current: base,
            base,
            max,
        }
    }

    /// Return the next delay and advance the internal state.
    ///
    /// The returned delay is `current + jitter`, where jitter is in `[0, current/4]`.
    pub fn next_delay(&mut self) -> Duration {
        let delay = self.current;
        self.current = (self.current * 2).min(self.max);

        // Add jitter: random value in [0, delay/4] to spread out reconnections.
        let quarter_ms = delay.as_millis() as u64 / 4;
        let jitter = if quarter_ms > 0 {
            let jitter_ms = rand::rng().random_range(0..=quarter_ms);
            Duration::from_millis(jitter_ms)
        } else {
            Duration::ZERO
        };

        let total = delay + jitter;
        tracing::trace!(
            delay_ms = total.as_millis() as u64,
            base_ms = delay.as_millis() as u64,
            jitter_ms = jitter.as_millis() as u64,
            next_ms = self.current.as_millis() as u64,
            "backoff delay computed"
        );
        total
    }

    /// Reset to the base delay after a successful connection.
    pub fn reset(&mut self) {
        tracing::trace!(base_ms = self.base.as_millis() as u64, "backoff reset");
        self.current = self.base;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doubling_behaviour() {
        let mut b = Backoff::new(Duration::from_secs(1), Duration::from_secs(60));
        // First delay is ~1s (plus jitter ≤ 250ms)
        let d1 = b.next_delay();
        assert!(d1 >= Duration::from_secs(1));
        assert!(d1 <= Duration::from_millis(1250));

        // Second delay is ~2s
        let d2 = b.next_delay();
        assert!(d2 >= Duration::from_secs(2));
        assert!(d2 <= Duration::from_millis(2500));

        // Third delay is ~4s
        let d3 = b.next_delay();
        assert!(d3 >= Duration::from_secs(4));
        assert!(d3 <= Duration::from_millis(5000));
    }

    #[test]
    fn max_cap() {
        let mut b = Backoff::new(Duration::from_secs(32), Duration::from_secs(60));
        // 32s
        let _ = b.next_delay();
        // Would be 64s but capped to 60s
        let d = b.next_delay();
        assert!(d >= Duration::from_secs(60));
        assert!(d <= Duration::from_millis(60_000 + 15_000)); // 60s + 25% jitter
        // Next stays at 60s
        let d = b.next_delay();
        assert!(d >= Duration::from_secs(60));
    }

    #[test]
    fn reset_returns_to_base() {
        let mut b = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));
        let _ = b.next_delay(); // 2s -> internal becomes 4s
        let _ = b.next_delay(); // 4s -> internal becomes 8s
        b.reset();
        let d = b.next_delay();
        assert!(d >= Duration::from_secs(2));
        assert!(d <= Duration::from_millis(2500));
    }

    #[test]
    fn zero_base_does_not_panic() {
        let mut b = Backoff::new(Duration::ZERO, Duration::from_secs(60));
        let d = b.next_delay();
        assert_eq!(d, Duration::ZERO);
    }
}
