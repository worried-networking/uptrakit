use std::fmt;

use time::OffsetDateTime;
use uuid::Uuid;

/// Type of authenticated actor that performed the request.
///
/// Internal-only typed enum following the `ActorType`/`BatchType` pattern:
/// `Copy`, `as_str()` + `Display`, not `#[non_exhaustive]`, no `Other(String)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditActorType {
    User,
    ApiToken,
    Oidc,
}

impl AuditActorType {
    /// String representation for database storage.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::ApiToken => "api_token",
            Self::Oidc => "oidc",
        }
    }
}

impl fmt::Display for AuditActorType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single audit log entry capturing an authenticated HTTP request.
#[derive(Clone, Debug)]
pub struct AuditEntry {
    /// UUIDv7 identifier.
    pub id: Uuid,
    /// Tenant scope. `None` routes to `system_audit_logs`, `Some` to `audit_logs`.
    pub tenant_id: Option<Uuid>,
    /// The user or API token that performed the action.
    pub actor_id: Uuid,
    /// Type of actor.
    pub actor_type: AuditActorType,
    /// How the actor authenticated (e.g. "password", "oidc", "api_token").
    pub auth_method: String,
    /// HTTP method (e.g. "GET", "POST").
    pub http_method: String,
    /// Raw request path (e.g. "/api/v1/hosts/abc").
    pub http_path: String,
    /// Axum `MatchedPath` route pattern (e.g. "/api/v1/hosts/{id}").
    pub route_pattern: Option<String>,
    /// HTTP response status code.
    pub http_status: u16,
    /// Client IP address (from `resolve_ip` middleware).
    pub client_ip: Option<String>,
    /// User-Agent header value.
    pub user_agent: Option<String>,
    /// Request duration in milliseconds.
    pub duration_ms: u64,
    /// When the request occurred.
    pub occurred_at: OffsetDateTime,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actor_type_as_str_round_trip() {
        assert_eq!(AuditActorType::User.as_str(), "user");
        assert_eq!(AuditActorType::ApiToken.as_str(), "api_token");
        assert_eq!(AuditActorType::Oidc.as_str(), "oidc");
    }

    #[test]
    fn actor_type_display() {
        assert_eq!(AuditActorType::User.to_string(), "user");
        assert_eq!(AuditActorType::ApiToken.to_string(), "api_token");
        assert_eq!(AuditActorType::Oidc.to_string(), "oidc");
    }
}
