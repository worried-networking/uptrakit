use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use uuid::Uuid;

use uptrakit_audit_log::{AuditActionType, AuditActorType, AuditEmitter, AuditEntry, AuditOutcome};
use uptrakit_shared_db::entity::prelude::*;
use uptrakit_shared_db::entity::{permission, role_permission, user_role};
use uptrakit_web_api_auth::auth::api_token::ApiTokenService;
// Alias to avoid name collision with the DB `Permission` entity from entity::prelude::*.
use uptrakit_web_api_auth::auth::permissions::Permission as AuthPermission;
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
    let entry = AuditEntry::builder(AuditActionType::AUTH_API_TOKEN_AUTHENTICATE)
        .tenant_scope(default_tenant_id)
        .actor(AuditActorType::ApiToken, None)
        .outcome(outcome)
        .details(serde_json::json!({ "reason_code": reason_code }))
        .request_id_opt(request_id)
        .build();

    match entry {
        Ok(e) => audit_emitter.emit_best_effort(e),
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
    default_tenant_id: Uuid,
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

    let permissions = get_user_permissions(db, default_tenant_id, user_id)
        .await
        .map_err(|e| {
            tracing::error!(err = %e, user_id = %user_id, "failed to load user permissions");
            AuthFailure::InternalError
        })?;

    Ok((
        AuthenticatedUser::new(user_id, AuthMethod::ApiToken, permissions, None),
        token_id,
    ))
}

async fn get_user_permissions(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    user_id: Uuid,
) -> uptrakit_web_api_auth::auth::Result<Vec<AuthPermission>> {
    use rootcause::prelude::*;

    let user_roles = UserRole::find()
        .filter(user_role::Column::TenantId.eq(tenant_id))
        .filter(user_role::Column::UserId.eq(user_id))
        .all(db)
        .await
        .context_to()?;

    let role_ids: Vec<Uuid> = user_roles.iter().map(|ur| ur.role_id).collect();
    if role_ids.is_empty() {
        return Ok(Vec::new());
    }

    let role_perms = RolePermission::find()
        .filter(role_permission::Column::RoleId.is_in(role_ids))
        .all(db)
        .await
        .context_to()?;

    let perm_ids: Vec<Uuid> = role_perms.iter().map(|rp| rp.permission_id).collect();
    if perm_ids.is_empty() {
        return Ok(Vec::new());
    }

    let perm_models = Permission::find()
        .filter(permission::Column::Id.is_in(perm_ids))
        .all(db)
        .await
        .context_to()?;

    let mut seen = std::collections::HashSet::new();
    let permissions = perm_models
        .into_iter()
        .filter_map(|p| p.name.parse::<AuthPermission>().ok())
        .filter(|p| seen.insert(p.clone()))
        .collect();

    Ok(permissions)
}

fn classify_api_token_verify_error(error: &rootcause::Report<AuthError>) -> AuthFailure {
    match error.current_context() {
        AuthError::ApiTokenNotFound | AuthError::ApiTokenRevoked => AuthFailure::InvalidApiToken,
        _ => AuthFailure::InternalError,
    }
}
