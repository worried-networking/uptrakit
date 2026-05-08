use rand::Rng;
use time::OffsetDateTime;

/// Compute the next run time from `now`, adding `interval_seconds` plus random
/// jitter in `[0, jitter_seconds]`.
///
/// The jitter prevents thundering-herd effects when multiple controller
/// instances poll simultaneously.
pub fn compute_next_run_at(
    now: OffsetDateTime,
    interval_seconds: i32,
    jitter_seconds: i32,
) -> OffsetDateTime {
    let jitter = if jitter_seconds > 0 {
        rand::rng().random_range(0..=jitter_seconds)
    } else {
        0
    };
    now + time::Duration::seconds(i64::from(interval_seconds) + i64::from(jitter))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_next_run_at_basic() {
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let next = compute_next_run_at(now, 300, 0);
        assert_eq!(next, now + time::Duration::seconds(300));
    }

    #[test]
    fn compute_next_run_at_with_jitter() {
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let next = compute_next_run_at(now, 300, 30);
        let delta = (next - now).whole_seconds();
        assert!(delta >= 300, "delta {delta} should be >= 300");
        assert!(delta <= 330, "delta {delta} should be <= 330");
    }

    #[test]
    fn compute_next_run_at_zero_jitter() {
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        // Zero jitter should always produce exactly interval_seconds.
        for _ in 0..100 {
            let next = compute_next_run_at(now, 600, 0);
            assert_eq!((next - now).whole_seconds(), 600);
        }
    }

    #[test]
    fn compute_next_run_at_jitter_range() {
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let mut saw_min = false;
        let mut saw_max = false;
        // Run enough iterations to statistically cover the range.
        for _ in 0..10_000 {
            let next = compute_next_run_at(now, 100, 10);
            let delta = (next - now).whole_seconds();
            assert!(delta >= 100);
            assert!(delta <= 110);
            if delta == 100 {
                saw_min = true;
            }
            if delta == 110 {
                saw_max = true;
            }
        }
        assert!(saw_min, "should have hit minimum jitter at least once");
        assert!(saw_max, "should have hit maximum jitter at least once");
    }
}
