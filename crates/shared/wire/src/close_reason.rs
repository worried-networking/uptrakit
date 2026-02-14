//! WebSocket close reason constants shared between the controller (sender)
//! and services (receiver).
//!
//! Using constants instead of string literals ensures that a typo on either
//! side is caught at compile time and that close-reason matching is exhaustive.

/// The service's TLS certificate was rotated by the controller.
pub const CERTIFICATE_ROTATED: &str = "certificate rotated";

/// The service's TLS certificate was revoked.
pub const CERTIFICATE_REVOKED: &str = "certificate revoked";

/// No valid certificate was presented during the TLS handshake.
pub const NO_VALID_CERTIFICATE: &str = "no valid certificate";

/// An internal server error occurred.
pub const INTERNAL_ERROR: &str = "internal error";

/// The presented certificate is not recognized by the controller.
pub const CERTIFICATE_NOT_RECOGNIZED: &str = "certificate not recognized";

/// The service has been deactivated by an administrator.
pub const SERVICE_DEACTIVATED: &str = "service deactivated";

/// The service has not been approved yet.
pub const SERVICE_NOT_APPROVED: &str = "service not approved";

/// The service was not found in the controller's database.
pub const SERVICE_NOT_FOUND: &str = "service not found";

/// The enrollment handshake timed out.
pub const ENROLLMENT_TIMEOUT: &str = "enrollment timeout";

/// The service exceeded the connection rate limit.
pub const RATE_LIMIT_EXCEEDED: &str = "rate limit exceeded";

/// A newer connection from the same service superseded this one.
pub const SUPERSEDED: &str = "superseded by new connection";

/// The agent's protocol version is too old to be supported.
pub const VERSION_TOO_OLD: &str = "agent version too old";
