use sea_orm::{DatabaseConnection, EntityTrait};
use uuid::Uuid;

use uptrakit_audit_log::{AuditActionType, AuditActorType, AuditEmitter, AuditEntry, AuditOutcome};
use uptrakit_shared_db::entity::prelude::*;
use uptrakit_web_api_auth::auth::api_token::ApiTokenService;
use uptrakit_web_api_auth::auth::{AuthError, AuthMethod};

use crate::auth::{AuthFailure, AuthenticatedUser};

/// Emit an audit entry for an API token authentication attempt.
pub fn emit_api_token_auth_audit(
    audit_emitter: &AuditEmitter,
    default_tenant_id: Uuid,
    request_id: Option<String>,
    outcome: AuditOutcome,
    reason_code: &'static str,
) {
    let entry = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
        AuditActionType::AUTH_API_TOKEN_AUTHENTICATE,
    )
    .tenant_scope(default_tenant_id)
    .actor(AuditActorType::ApiToken, None)
    .outcome(outcome)
    .details(serde_json::json!({ "reason_code": reason_code }))
    .request_id_opt(request_id)
    .build();

    match entry {
        Ok(e) => audit_emitter.emit_event(e),
        Err(e) => tracing::warn!(err = %e, "failed to build audit entry for api token auth"),
    }
}

/// Authenticate using a `upk_`-prefixed API token (requires DB lookup).
///
/// # Errors
///
/// Returns [`AuthFailure::InvalidApiToken`] if the token is not found or revoked,
/// [`AuthFailure::UserNotFound`] if the associated user no longer exists,
/// [`AuthFailure::UserDeactivated`] if the user is inactive, and
/// [`AuthFailure::InternalError`] on database failures.
pub async fn authenticate_api_token(
    db: &DatabaseConnection,
    token: &str,
) -> Result<(AuthenticatedUser, Uuid), AuthFailure> {
    let service = ApiTokenService::new(db.clone());

    let (user_id, token_id) = service
        .verify_token(token)
        .await
        .map_err(|error| classify_api_token_verify_error(&error))?;

    let user = User::find_by_id(user_id)
        .one(db)
        .await
        .map_err(|e| {
            tracing::error!(err = %e, %user_id, "db error fetching user for api token auth");
            AuthFailure::InternalError
        })?
        .ok_or(AuthFailure::UserNotFound)?;

    if !user.is_active {
        return Err(AuthFailure::UserDeactivated);
    }

    Ok((
        AuthenticatedUser::new(user_id, AuthMethod::ApiToken, None),
        token_id,
    ))
}

fn classify_api_token_verify_error(error: &rootcause::Report<AuthError>) -> AuthFailure {
    match error.current_context() {
        AuthError::ApiTokenNotFound | AuthError::ApiTokenRevoked => AuthFailure::InvalidApiToken,
        _ => {
            tracing::warn!(err = %error, "unexpected error variant classifying api token verify failure");
            AuthFailure::InternalError
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rootcause::report;
    use uptrakit_web_api_auth::auth::AuthError;

    #[test]
    fn classify_not_found_returns_invalid_api_token() {
        let err = report!(AuthError::ApiTokenNotFound);
        match classify_api_token_verify_error(&err) {
            AuthFailure::InvalidApiToken => (),
            other => panic!("expected InvalidApiToken, got {other:?}"),
        }
    }

    #[test]
    fn classify_revoked_returns_invalid_api_token() {
        let err = report!(AuthError::ApiTokenRevoked);
        match classify_api_token_verify_error(&err) {
            AuthFailure::InvalidApiToken => (),
            other => panic!("expected InvalidApiToken, got {other:?}"),
        }
    }

    #[test]
    fn classify_internal_returns_internal_error() {
        let err = report!(AuthError::Internal("db fail".into()));
        match classify_api_token_verify_error(&err) {
            AuthFailure::InternalError => (),
            other => panic!("expected InternalError, got {other:?}"),
        }
    }
}
