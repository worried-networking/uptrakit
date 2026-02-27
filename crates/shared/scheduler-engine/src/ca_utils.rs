//! Shared CA certificate rotation utilities.
//!
//! Extracted from `crates/core/controller/src/pki.rs` so both the embedded
//! scheduler and external scheduler can use the same logic.

use time::OffsetDateTime;

/// CA rotation window: rotate when the CA certificate expires within this many days.
pub const CA_ROTATION_WINDOW_DAYS: i64 = 183;

/// Extract the `not_after` timestamp from a PEM-encoded certificate.
pub fn cert_not_after(pem: &str) -> Option<OffsetDateTime> {
    let (_, pem_block) = x509_parser::pem::parse_x509_pem(pem.as_bytes()).ok()?;
    let cert = pem_block.parse_x509().ok()?;
    OffsetDateTime::from_unix_timestamp(cert.validity().not_after.timestamp()).ok()
}

/// Returns `true` if the CA certificate expires within 183 days (6 months).
///
/// Returns `true` for invalid PEM (fail-safe: trigger rotation if we can't parse).
pub fn should_rotate_ca(cert_pem: &str) -> bool {
    let Some(not_after) = cert_not_after(cert_pem) else {
        return true;
    };
    let threshold = OffsetDateTime::now_utc() + time::Duration::days(CA_ROTATION_WINDOW_DAYS);
    not_after <= threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_pem_triggers_rotation() {
        assert!(should_rotate_ca("not a real cert"));
    }

    #[test]
    fn empty_pem_triggers_rotation() {
        assert!(should_rotate_ca(""));
    }

    #[test]
    fn cert_not_after_returns_none_for_invalid_pem() {
        assert!(cert_not_after("garbage").is_none());
    }
}
