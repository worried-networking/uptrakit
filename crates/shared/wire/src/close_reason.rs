//! WebSocket close reason enum shared between the controller (sender)
//! and services (receiver).
//!
//! Using a typed enum instead of string constants ensures that:
//! - Adding a new reason triggers exhaustive-match compile errors at every call site
//! - The receiver stores `Option<CloseReason>` instead of `Option<String>`
//! - IDE navigation, rename-refactor, and usage search all work precisely
//!
//! The wire format is unchanged: [`Display`] produces identical strings to the
//! former constants, and [`FromStr`] parses them back. Unknown strings from
//! future controller versions become [`CloseReason::Unknown`].

use std::fmt;
use std::str::FromStr;

/// Reason included in a WebSocket close frame by the controller.
///
/// Known variants map 1:1 to the wire strings sent in close frames.
/// [`Unknown`](Self::Unknown) provides forward compatibility for strings
/// not yet recognized by the receiver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloseReason {
    /// The service's TLS certificate was rotated by the controller.
    CertificateRotated,
    /// The service's TLS certificate was revoked.
    CertificateRevoked,
    /// No valid certificate was presented during the TLS handshake.
    NoValidCertificate,
    /// An internal server error occurred.
    InternalError,
    /// The presented certificate is not recognized by the controller.
    CertificateNotRecognized,
    /// The service has been deactivated by an administrator.
    ServiceDeactivated,
    /// The service has not been approved yet.
    ServiceNotApproved,
    /// The service was not found in the controller's database.
    ServiceNotFound,
    /// The enrollment handshake timed out.
    EnrollmentTimeout,
    /// The service exceeded the connection rate limit.
    RateLimitExceeded,
    /// A newer connection from the same service superseded this one.
    Superseded,
    /// The agent's protocol version is too old to be supported.
    VersionTooOld,
    /// A close reason string not recognized by this build.
    ///
    /// Provides forward compatibility: a newer controller may send reasons
    /// that an older service does not yet know about.
    Unknown(String),
}

impl CloseReason {
    /// Returns the wire-format string for this close reason.
    pub fn as_str(&self) -> &str {
        match self {
            Self::CertificateRotated => "certificate rotated",
            Self::CertificateRevoked => "certificate revoked",
            Self::NoValidCertificate => "no valid certificate",
            Self::InternalError => "internal error",
            Self::CertificateNotRecognized => "certificate not recognized",
            Self::ServiceDeactivated => "service deactivated",
            Self::ServiceNotApproved => "service not approved",
            Self::ServiceNotFound => "service not found",
            Self::EnrollmentTimeout => "enrollment timeout",
            Self::RateLimitExceeded => "rate limit exceeded",
            Self::Superseded => "superseded by new connection",
            Self::VersionTooOld => "agent version too old",
            Self::Unknown(s) => s,
        }
    }
}

impl fmt::Display for CloseReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when parsing a close reason string fails.
///
/// In practice this error is never returned because [`CloseReason::Unknown`]
/// catches all unrecognized strings, but the type exists to satisfy the
/// [`FromStr`] trait contract and project conventions.
#[derive(Debug, thiserror::Error)]
#[error("invalid close reason")]
pub struct ParseCloseReasonError;

impl FromStr for CloseReason {
    type Err = ParseCloseReasonError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "certificate rotated" => Self::CertificateRotated,
            "certificate revoked" => Self::CertificateRevoked,
            "no valid certificate" => Self::NoValidCertificate,
            "internal error" => Self::InternalError,
            "certificate not recognized" => Self::CertificateNotRecognized,
            "service deactivated" => Self::ServiceDeactivated,
            "service not approved" => Self::ServiceNotApproved,
            "service not found" => Self::ServiceNotFound,
            "enrollment timeout" => Self::EnrollmentTimeout,
            "rate limit exceeded" => Self::RateLimitExceeded,
            "superseded by new connection" => Self::Superseded,
            "agent version too old" => Self::VersionTooOld,
            other => Self::Unknown(other.to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All known variants with their expected wire strings.
    const KNOWN_VARIANTS: &[(CloseReason, &str)] = &[
        (CloseReason::CertificateRotated, "certificate rotated"),
        (CloseReason::CertificateRevoked, "certificate revoked"),
        (CloseReason::NoValidCertificate, "no valid certificate"),
        (CloseReason::InternalError, "internal error"),
        (
            CloseReason::CertificateNotRecognized,
            "certificate not recognized",
        ),
        (CloseReason::ServiceDeactivated, "service deactivated"),
        (CloseReason::ServiceNotApproved, "service not approved"),
        (CloseReason::ServiceNotFound, "service not found"),
        (CloseReason::EnrollmentTimeout, "enrollment timeout"),
        (CloseReason::RateLimitExceeded, "rate limit exceeded"),
        (CloseReason::Superseded, "superseded by new connection"),
        (CloseReason::VersionTooOld, "agent version too old"),
    ];

    #[test]
    fn display_produces_wire_strings() {
        for (variant, expected) in KNOWN_VARIANTS {
            assert_eq!(variant.to_string(), *expected);
        }
    }

    #[test]
    fn as_str_matches_display() {
        for (variant, expected) in KNOWN_VARIANTS {
            assert_eq!(variant.as_str(), *expected);
        }
    }

    #[test]
    fn from_str_roundtrip_known_variants() {
        for (variant, wire_str) in KNOWN_VARIANTS {
            let parsed: CloseReason = wire_str.parse().expect("parse should succeed");
            assert_eq!(&parsed, variant);
            assert_eq!(parsed.to_string(), *wire_str);
        }
    }

    #[test]
    fn from_str_unknown_passthrough() {
        let parsed: CloseReason = "some future reason".parse().expect("parse should succeed");
        assert_eq!(
            parsed,
            CloseReason::Unknown("some future reason".to_string())
        );
        assert_eq!(parsed.to_string(), "some future reason");
        assert_eq!(parsed.as_str(), "some future reason");
    }

    #[test]
    fn from_str_empty_string() {
        let parsed: CloseReason = "".parse().expect("parse should succeed");
        assert_eq!(parsed, CloseReason::Unknown(String::new()));
    }

    #[test]
    fn equality_known_variants() {
        assert_eq!(
            CloseReason::CertificateRotated,
            CloseReason::CertificateRotated
        );
        assert_ne!(
            CloseReason::CertificateRotated,
            CloseReason::CertificateRevoked
        );
    }

    #[test]
    fn equality_unknown_variants() {
        assert_eq!(
            CloseReason::Unknown("x".to_string()),
            CloseReason::Unknown("x".to_string())
        );
        assert_ne!(
            CloseReason::Unknown("x".to_string()),
            CloseReason::Unknown("y".to_string())
        );
    }

    #[test]
    fn clone_works() {
        let original = CloseReason::CertificateRotated;
        let cloned = original.clone();
        assert_eq!(original, cloned);

        let original = CloseReason::Unknown("test".to_string());
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }
}
