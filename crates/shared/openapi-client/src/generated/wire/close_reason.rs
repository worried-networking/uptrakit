// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
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
#[non_exhaustive]
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
    /// The service sent an unexpected or malformed protocol message.
    ///
    /// Used when the service violates the expected message sequence, for
    /// example by sending a message other than `Register` as the first
    /// frame after authentication completes.
    ProtocolError,
    /// A newer connection from the same service superseded this one.
    Superseded,
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
            Self::ProtocolError => "protocol error",
            Self::Superseded => "superseded by new connection",
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
            "protocol error" => Self::ProtocolError,
            "superseded by new connection" => Self::Superseded,
            other => Self::Unknown(other.to_string()),
        })
    }
}
