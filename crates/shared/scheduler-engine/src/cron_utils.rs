use std::str::FromStr;

use time::OffsetDateTime;

/// Normalize a cron expression for the `cron` crate.
///
/// The `cron` crate requires 6 or 7 fields (with a seconds field). Standard
/// 5-field cron expressions are normalized by prepending `0 ` (fire at second 0).
fn normalize_cron(expr: &str) -> String {
    let fields = expr.split_whitespace().count();
    if fields == 5 {
        format!("0 {expr}")
    } else {
        expr.to_string()
    }
}

/// Compute the next run time after `after` for a cron expression.
///
/// Bridges the `cron` crate (chrono-based) and the `time` crate via unix timestamps.
/// Accepts both standard 5-field and extended 6/7-field cron expressions.
pub fn next_run_after(cron_expr: &str, after: OffsetDateTime) -> Option<OffsetDateTime> {
    let normalized = normalize_cron(cron_expr);
    let schedule = cron::Schedule::from_str(&normalized).ok()?;
    let after_chrono = chrono::DateTime::from_timestamp(after.unix_timestamp(), 0)?;
    let next_chrono = schedule.after(&after_chrono).next()?;
    OffsetDateTime::from_unix_timestamp(next_chrono.timestamp()).ok()
}

/// Validate a cron expression.
///
/// Accepts both standard 5-field and extended 6/7-field cron expressions.
pub fn validate_cron(cron_expr: &str) -> Result<(), String> {
    let normalized = normalize_cron(cron_expr);
    cron::Schedule::from_str(&normalized)
        .map(|_| ())
        .map_err(|e| format!("invalid cron expression: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_run_after_every_5_minutes() {
        let expr = "*/5 * * * *";
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let next = next_run_after(expr, now);
        assert!(next.is_some());
        let next = next.unwrap();
        assert!(next > now);
        // Should be within 5 minutes
        assert!(next.unix_timestamp() - now.unix_timestamp() <= 300);
    }

    #[test]
    fn next_run_after_hourly() {
        let expr = "0 * * * *";
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let next = next_run_after(expr, now);
        assert!(next.is_some());
        let next = next.unwrap();
        assert!(next > now);
        assert!(next.unix_timestamp() - now.unix_timestamp() <= 3600);
    }

    #[test]
    fn next_run_after_daily_at_3am() {
        let expr = "0 3 * * *";
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let next = next_run_after(expr, now);
        assert!(next.is_some());
        let next = next.unwrap();
        assert!(next > now);
        // Should be within 24 hours
        assert!(next.unix_timestamp() - now.unix_timestamp() <= 86400);
    }

    #[test]
    fn next_run_after_six_field_expression() {
        // 6-field expression with seconds
        let expr = "0 */5 * * * *";
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let next = next_run_after(expr, now);
        assert!(next.is_some());
    }

    #[test]
    fn validate_cron_valid() {
        assert!(validate_cron("*/5 * * * *").is_ok());
        assert!(validate_cron("0 * * * *").is_ok());
        assert!(validate_cron("0 3 * * *").is_ok());
        assert!(validate_cron("0 */6 * * *").is_ok());
        assert!(validate_cron("0 */12 * * *").is_ok());
    }

    #[test]
    fn validate_cron_invalid() {
        assert!(validate_cron("not a cron").is_err());
        assert!(validate_cron("").is_err());
        assert!(validate_cron("* * *").is_err());
    }

    #[test]
    fn next_run_after_invalid_expression_returns_none() {
        let now = OffsetDateTime::now_utc();
        assert!(next_run_after("invalid", now).is_none());
    }

    #[test]
    fn normalize_prepends_seconds_for_five_fields() {
        assert_eq!(normalize_cron("*/5 * * * *"), "0 */5 * * * *");
        assert_eq!(normalize_cron("0 3 * * *"), "0 0 3 * * *");
    }

    #[test]
    fn normalize_keeps_six_fields_unchanged() {
        assert_eq!(normalize_cron("0 */5 * * * *"), "0 */5 * * * *");
    }
}
