use axum::http::StatusCode;
use uptrakit_audit_log::{AuditActionType, AuditOutcome, RegisteredAuditAction};
use uptrakit_web_api_auth::auth::{device_flow::DeviceFlowError, error::AuthError};

pub(crate) trait AuthErrorAuditExt {
    fn logout_verify_classification(&self) -> (AuditOutcome, &'static str);
    fn refresh_rotation_classification(&self) -> (StatusCode, AuditOutcome, &'static str);
}

impl AuthErrorAuditExt for AuthError {
    fn logout_verify_classification(&self) -> (AuditOutcome, &'static str) {
        match self {
            AuthError::InvalidRefreshToken
            | AuthError::RefreshTokenExpired
            | AuthError::RefreshTokenRevoked => {
                (AuditOutcome::Denied, "invalid_or_expired_refresh_token")
            }
            _ => (AuditOutcome::Failed, "refresh_token_verify_failed"),
        }
    }

    fn refresh_rotation_classification(&self) -> (StatusCode, AuditOutcome, &'static str) {
        match self {
            AuthError::InvalidRefreshToken
            | AuthError::RefreshTokenExpired
            | AuthError::RefreshTokenRevoked
            | AuthError::InvalidSession => (
                StatusCode::UNAUTHORIZED,
                AuditOutcome::Denied,
                "invalid_or_expired_refresh_token",
            ),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                AuditOutcome::Failed,
                "refresh_rotation_failed",
            ),
        }
    }
}

pub(crate) trait DeviceFlowErrorAuditExt {
    fn approval_classification(&self) -> (RegisteredAuditAction, AuditOutcome, &'static str);
    fn denial_classification(&self) -> (AuditOutcome, &'static str);
}

impl DeviceFlowErrorAuditExt for DeviceFlowError {
    fn approval_classification(&self) -> (RegisteredAuditAction, AuditOutcome, &'static str) {
        match self {
            DeviceFlowError::NotFound => (
                AuditActionType::AUTH_DEVICE_DENY,
                AuditOutcome::Denied,
                "device_flow_not_found",
            ),
            DeviceFlowError::AlreadyAuthorized => (
                AuditActionType::AUTH_DEVICE_DENY,
                AuditOutcome::Denied,
                "device_flow_already_authorized",
            ),
            DeviceFlowError::TokenGeneration(_) => (
                AuditActionType::AUTH_DEVICE_APPROVE,
                AuditOutcome::Failed,
                "device_flow_token_generation_error",
            ),
            DeviceFlowError::Database(_) => (
                AuditActionType::AUTH_DEVICE_APPROVE,
                AuditOutcome::Failed,
                "device_flow_database_error",
            ),
        }
    }

    fn denial_classification(&self) -> (AuditOutcome, &'static str) {
        match self {
            DeviceFlowError::NotFound => (AuditOutcome::Denied, "device_flow_not_found"),
            DeviceFlowError::AlreadyAuthorized => {
                (AuditOutcome::Denied, "device_flow_already_authorized")
            }
            DeviceFlowError::TokenGeneration(_) | DeviceFlowError::Database(_) => {
                (AuditOutcome::Failed, "device_flow_deny_failed")
            }
        }
    }
}

/// High-level classification for audit events used to drive filtered views.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AuditClass {
    /// Routine authentication lifecycle events.
    AuthEvent,
    /// OAuth client registration, trust, revocation, and CIMD lifecycle.
    ClientLifecycle,
    /// Events that indicate a potential attack or critical config change.
    SecurityCritical,
}

/// Returns the audit class for a given OAuth action, or `None` for non-OAuth actions.
pub(crate) fn oauth_audit_class(action: RegisteredAuditAction) -> Option<AuditClass> {
    use uptrakit_audit_log::AuditActionType;
    if action == AuditActionType::OAUTH_REFRESH_REPLAY_DETECTED
        || action == AuditActionType::OAUTH_CONFIG_AUDIENCE_HOSTS_CHANGED
        || action == AuditActionType::OAUTH_CIMD_PARSE_FAILED
        || action == AuditActionType::OAUTH_CLIENT_REGISTRATION_RATE_LIMITED
        || action == AuditActionType::OAUTH_RATE_LIMITED
    {
        Some(AuditClass::SecurityCritical)
    } else if action == AuditActionType::OAUTH_CLIENT_REGISTERED
        || action == AuditActionType::OAUTH_CLIENT_FIRST_USE
        || action == AuditActionType::OAUTH_CLIENT_METADATA_REFRESHED
        || action == AuditActionType::OAUTH_CLIENT_METADATA_CHANGED_MATERIALLY
        || action == AuditActionType::OAUTH_CLIENT_TRUSTED
        || action == AuditActionType::OAUTH_CLIENT_REVOKED
    {
        Some(AuditClass::ClientLifecycle)
    } else if action == AuditActionType::OAUTH_AUTHORIZE_REQUEST
        || action == AuditActionType::OAUTH_TOKEN_ISSUED
        || action == AuditActionType::OAUTH_TOKEN_REJECTED
        || action == AuditActionType::OAUTH_REFRESH_ROTATED
        || action == AuditActionType::OAUTH_CONSENT_GRANT
        || action == AuditActionType::OAUTH_CONSENT_DENY
        || action == AuditActionType::OAUTH_CONSENT_REVOKE
        || action == AuditActionType::MCP_OAUTH_AUTHENTICATE
    {
        Some(AuditClass::AuthEvent)
    } else {
        None
    }
}

/// Returns `true` if the action should appear in security-relevant filtered audit streams.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "consumed by oauth route handlers not yet wired up"
    )
)]
pub(crate) fn is_security_relevant(action: RegisteredAuditAction) -> bool {
    matches!(
        oauth_audit_class(action),
        Some(AuditClass::SecurityCritical)
    )
}

#[cfg(test)]
mod tests {
    use super::{AuditClass, is_security_relevant, oauth_audit_class};
    use uptrakit_audit_log::AuditActionType;

    #[test]
    fn replay_detected_is_security_critical() {
        assert_eq!(
            oauth_audit_class(AuditActionType::OAUTH_REFRESH_REPLAY_DETECTED),
            Some(AuditClass::SecurityCritical),
        );
    }

    #[test]
    fn config_change_is_security_critical() {
        assert_eq!(
            oauth_audit_class(AuditActionType::OAUTH_CONFIG_AUDIENCE_HOSTS_CHANGED),
            Some(AuditClass::SecurityCritical),
        );
    }

    #[test]
    fn token_issued_is_auth_event() {
        assert_eq!(
            oauth_audit_class(AuditActionType::OAUTH_TOKEN_ISSUED),
            Some(AuditClass::AuthEvent),
        );
    }

    #[test]
    fn client_registered_is_client_lifecycle() {
        assert_eq!(
            oauth_audit_class(AuditActionType::OAUTH_CLIENT_REGISTERED),
            Some(AuditClass::ClientLifecycle),
        );
    }

    #[test]
    fn security_relevant_includes_security_critical_events() {
        assert!(is_security_relevant(
            AuditActionType::OAUTH_REFRESH_REPLAY_DETECTED
        ));
        assert!(is_security_relevant(
            AuditActionType::OAUTH_CONFIG_AUDIENCE_HOSTS_CHANGED
        ));
        assert!(is_security_relevant(
            AuditActionType::OAUTH_CIMD_PARSE_FAILED
        ));
    }

    #[test]
    fn security_relevant_excludes_routine_events() {
        assert!(!is_security_relevant(AuditActionType::OAUTH_TOKEN_ISSUED));
        assert!(!is_security_relevant(
            AuditActionType::OAUTH_CLIENT_REGISTERED
        ));
        assert!(!is_security_relevant(AuditActionType::OAUTH_CONSENT_GRANT));
    }

    #[test]
    fn non_oauth_action_returns_none() {
        assert_eq!(oauth_audit_class(AuditActionType::AUTH_LOGIN), None);
    }
}
