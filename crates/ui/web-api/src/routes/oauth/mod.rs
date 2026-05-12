//! OAuth 2.0 endpoints (RFC 8628 device grant + RFC 8414 metadata).
//!
//! See `docs/superpowers/specs/2026-05-12-rfc8628-device-auth-design.md`.

pub mod device_authorization;
mod helpers;
pub mod metadata;
pub mod token;
