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
            | AuthError::RefreshTokenRevoked => (
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
    fn poll_status_classification(&self) -> (AuditOutcome, &'static str);
    fn poll_consume_classification(&self) -> (AuditOutcome, &'static str);
    fn approval_classification(&self) -> (RegisteredAuditAction, AuditOutcome, &'static str);
}

impl DeviceFlowErrorAuditExt for DeviceFlowError {
    fn poll_status_classification(&self) -> (AuditOutcome, &'static str) {
        match self {
            DeviceFlowError::NotFound | DeviceFlowError::AlreadyAuthorized => {
                (AuditOutcome::Denied, "device_flow_not_found")
            }
            DeviceFlowError::TokenGeneration(_) | DeviceFlowError::Database(_) => {
                (AuditOutcome::Failed, "device_flow_status_lookup_failed")
            }
        }
    }

    fn poll_consume_classification(&self) -> (AuditOutcome, &'static str) {
        match self {
            DeviceFlowError::NotFound | DeviceFlowError::AlreadyAuthorized => {
                (AuditOutcome::Denied, "device_flow_not_found")
            }
            DeviceFlowError::TokenGeneration(_) | DeviceFlowError::Database(_) => {
                (AuditOutcome::Failed, "device_flow_consume_failed")
            }
        }
    }

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
}
