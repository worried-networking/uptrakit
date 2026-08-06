//! Role management endpoints (`/api/v1/roles`) — M1.6a.
//!
//! Reads and mutations gate on `access:manage` via [`CanManageAccess`].
//! Built-ins are immutable (409 `built_in_role_immutable` on update/delete)
//! but assignable — user-role linkage stays on `users.rs` (Task 6). Deleting
//! a role that carries a role-subject grant reaching the system plane
//! additionally requires `system.access:manage`, checked inline against the
//! engine (same body-dependent fine-check pattern as `access_grants.rs`).
//!
//! `RoleQueryError` is mapped centrally in `api_error/mappings.rs`
//! (mirroring `AccessGrantError`) — handlers propagate it with `?`, never
//! matching `.current_context()` inline.

use crate::AppState;
use crate::api_error::ApiError;
use crate::error_response::{error_response, error_response_with_code};
use crate::extract::Validated;
use crate::middleware::action::{AccessAuthority, CanManageAccess, require_system_access};
use crate::middleware::require_auth::{AuthenticatedApiTokenId, authenticated_user_audit_actor};
use crate::queries::roles::{self as role_queries, RoleNameCollision, RoleView};
use crate::tenant_db::TenantDb;
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{SqliteTransactionMode, TransactionOptions, TransactionTrait};
use std::sync::Arc;
use uptrakit_audit_log::{AbsentView, AuditActionType, AuditEntry, AuditOutcome, Stateful};
use uptrakit_shared_db::access_grants::{
    GrantSubject, GuardedMutation, LockoutVerdict, begin_guarded, check_lockout,
    delete_grants_for_role, list_grants, patterns_reach_system_plane,
};
use uptrakit_shared_db::entity::role;
use uuid::Uuid;

pub use uptrakit_web_api_types::roles::{CreateRoleRequest, RoleResponse, UpdateRoleRequest};

// --- Endpoints ---

/// List roles for the active tenant plus the global built-ins.
#[utoipa::path(
    get,
    path = "/api/v1/roles",
    responses(
        (status = 200, description = "Global built-ins plus the tenant's custom roles", body = [RoleResponse]),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Users",
    security(("oauth2" = ["access:manage"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_roles(
    tenant_db: TenantDb,
    CanManageAccess(_user): CanManageAccess,
) -> Result<Response, ApiError> {
    let roles = role_queries::list_roles(tenant_db.db(), tenant_db.tenant_id()).await?;
    let out: Vec<RoleResponse> = roles.iter().map(role_to_response).collect();
    Ok((StatusCode::OK, Json(out)).into_response())
}

/// Get a single role.
#[utoipa::path(
    get,
    path = "/api/v1/roles/{id}",
    params(
        ("id" = Uuid, Path, description = "Role id")
    ),
    responses(
        (status = 200, description = "Role details", body = RoleResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Role not found")
    ),
    tag = "Users",
    security(("oauth2" = ["access:manage"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_role(
    tenant_db: TenantDb,
    CanManageAccess(_user): CanManageAccess,
    Path(role_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let role = role_queries::get_role(tenant_db.db(), tenant_db.tenant_id(), role_id).await?;
    Ok((StatusCode::OK, Json(role_to_response(&role))).into_response())
}

/// Create a tenant-scoped custom role.
#[utoipa::path(
    post,
    path = "/api/v1/roles",
    request_body = CreateRoleRequest,
    responses(
        (status = 201, description = "Role created", body = RoleResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 409, description = "role_name_shadows_global: collides with a global built-in name. role_name_taken: collides with another custom role in this tenant.")
    ),
    tag = "Users",
    security(("oauth2" = ["access:manage"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn create_role(
    State(state): State<Arc<AppState>>,
    CanManageAccess(caller): CanManageAccess,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Validated(body): Validated<CreateRoleRequest>,
) -> Result<Response, ApiError> {
    let api_token_id = api_token_id.map(|v| v.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&caller, api_token_id);

    if let Some(response) = collision_response(
        role_queries::find_role_name_collision(
            state.db(),
            state.default_tenant_id,
            &body.name,
            None,
        )
        .await?,
    ) {
        return Ok(response);
    }

    let tx = match state
        .db()
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
    {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("Failed to begin transaction for role create: {e}");
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    let created = match role_queries::create_role(
        &tx,
        state.default_tenant_id,
        &body.name,
        body.description.clone(),
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            drop(tx);
            return Err(e.into());
        }
    };

    let view = RoleView::from(&created);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::role_create(&AbsentView(&view), &view)
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::Success)
        .tenant_scope(state.default_tenant_id)
        .build()
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!("Failed to build audit entry for role create: {e}");
            drop(tx);
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!("Failed to emit audit entry for role create: {e}");
        drop(tx);
        return Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error",
        ));
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit role create: {e}");
        return Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error",
        ));
    }
    hook.flush_after_commit().await;

    // A fresh role has no grants or assignments yet — nothing to
    // invalidate (adding-only mutation).
    Ok((StatusCode::CREATED, Json(role_to_response(&created))).into_response())
}

/// Rename/re-describe an own-tenant custom role. `tenant_id` and
/// `is_built_in` are immutable.
#[utoipa::path(
    put,
    path = "/api/v1/roles/{id}",
    params(
        ("id" = Uuid, Path, description = "Role id")
    ),
    request_body = UpdateRoleRequest,
    responses(
        (status = 200, description = "Role updated", body = RoleResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Role not found"),
        (status = 409, description = "built_in_role_immutable: built-ins cannot be renamed. role_name_shadows_global / role_name_taken: name collision.")
    ),
    tag = "Users",
    security(("oauth2" = ["access:manage"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_role(
    State(state): State<Arc<AppState>>,
    CanManageAccess(caller): CanManageAccess,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(role_id): Path<Uuid>,
    Validated(body): Validated<UpdateRoleRequest>,
) -> Result<Response, ApiError> {
    let api_token_id = api_token_id.map(|v| v.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&caller, api_token_id);

    let existing = role_queries::get_role(state.db(), state.default_tenant_id, role_id).await?;
    if let Some(response) = built_in_immutable_response(&existing) {
        return Ok(response);
    }
    if existing.tenant_id != Some(state.default_tenant_id) {
        return Ok(error_response(StatusCode::NOT_FOUND, "Role not found"));
    }
    if let Some(response) = collision_response(
        role_queries::find_role_name_collision(
            state.db(),
            state.default_tenant_id,
            &body.name,
            Some(role_id),
        )
        .await?,
    ) {
        return Ok(response);
    }

    let tx = match state
        .db()
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
    {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("Failed to begin transaction for role update: {e}");
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    let (before, after) = match role_queries::update_role(
        &tx,
        state.default_tenant_id,
        role_id,
        &body.name,
        body.description.clone(),
    )
    .await
    {
        Ok(pair) => pair,
        Err(e) => {
            drop(tx);
            return Err(e.into());
        }
    };

    let before_view = RoleView::from(&before);
    let after_view = RoleView::from(&after);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::role_update(&before_view, &after_view)
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::Success)
        .tenant_scope(state.default_tenant_id)
        .build()
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!("Failed to build audit entry for role update: {e}");
            drop(tx);
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!("Failed to emit audit entry for role update: {e}");
        drop(tx);
        return Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error",
        ));
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit role update: {e}");
        return Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error",
        ));
    }
    hook.flush_after_commit().await;

    // A rename never changes authority — nothing to invalidate.
    Ok((StatusCode::OK, Json(role_to_response(&after))).into_response())
}

/// Delete an own-tenant custom role, cascading its grants and assignments.
///
/// Cross-instance revocation latency is bounded by the 60 s cache TTL
/// backstop, same as grant deletion.
#[utoipa::path(
    delete,
    path = "/api/v1/roles/{id}",
    params(
        ("id" = Uuid, Path, description = "Role id")
    ),
    responses(
        (status = 204, description = "Role deleted"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized. Deleting a role carrying a system-plane grant additionally requires system.access:manage."),
        (status = 404, description = "Role not found"),
        (status = 409, description = "built_in_role_immutable: built-ins cannot be deleted. lockout_access_manage / lockout_system_access: this change would remove the last remaining covering holder.")
    ),
    tag = "Users",
    security(("oauth2" = ["access:manage"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn delete_role(
    State(state): State<Arc<AppState>>,
    CanManageAccess(caller): CanManageAccess,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Extension(authority): Extension<AccessAuthority>,
    Path(role_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let api_token_id = api_token_id.map(|v| v.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&caller, api_token_id);

    let existing = role_queries::get_role(state.db(), state.default_tenant_id, role_id).await?;
    if let Some(response) = built_in_immutable_response(&existing) {
        return Ok(response);
    }
    if existing.tenant_id != Some(state.default_tenant_id) {
        return Ok(error_response(StatusCode::NOT_FOUND, "Role not found"));
    }

    let tx = match begin_guarded(state.db()).await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("Failed to begin transaction for role delete: {e}");
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    let load = match list_grants(
        &tx,
        state.default_tenant_id,
        Some(GrantSubject::Role(role_id)),
    )
    .await
    {
        Ok(l) => l,
        Err(e) => {
            drop(tx);
            return Err(e.into());
        }
    };
    if load.corrupt_skipped > 0 {
        // Fail closed: a corrupt row for this role could hide system-plane
        // authority the fine check below must see.
        tracing::error!(
            role_id = %role_id,
            corrupt_skipped = load.corrupt_skipped,
            "role delete: corrupt grant rows skipped, refusing to proceed"
        );
        drop(tx);
        return Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error",
        ));
    }

    let mut system_plane = false;
    for grant in &load.grants {
        match patterns_reach_system_plane(&grant.patterns) {
            Ok(v) => system_plane |= v,
            Err(e) => {
                drop(tx);
                return Err(e.into());
            }
        }
    }
    if system_plane {
        // APPROVED: body-dependent fine check (corpus 07, restated invariant)
        if let Some(denied) =
            require_system_access(&state.access_engine, &state.audit_emitter, &authority)
        {
            drop(tx);
            return Ok(denied);
        }
    }

    let verdict = match check_lockout(
        &tx,
        state.default_tenant_id,
        &GuardedMutation::DeleteRole { role_id },
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            drop(tx);
            return Err(e.into());
        }
    };
    if !matches!(verdict, LockoutVerdict::Permitted) {
        drop(tx);
        return Ok(crate::routes::access_grants::lockout_denial_response(
            &state,
            AuditActionType::ROLE_DELETE.into(),
            (actor_type, actor_id),
            "role",
            role_id.to_string(),
            verdict,
        ));
    }

    if let Err(e) = role_queries::delete_role_rows(&tx, state.default_tenant_id, role_id).await {
        drop(tx);
        return Err(e.into());
    }
    if let Err(e) = delete_grants_for_role(&tx, role_id).await {
        drop(tx);
        return Err(e.into());
    }

    let view = RoleView::from(&existing);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::role_delete(&view, &AbsentView(&view))
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::Success)
        .tenant_scope(state.default_tenant_id)
        .build()
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!("Failed to build audit entry for role delete: {e}");
            drop(tx);
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!("Failed to emit audit entry for role delete: {e}");
        drop(tx);
        return Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error",
        ));
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit role delete: {e}");
        return Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error",
        ));
    }
    hook.flush_after_commit().await;

    state.access_engine.invalidate_subjects(&[], &[role_id]);
    state
        .notification
        .notification_service
        .publish_controller_event(uptrakit_wire::ControllerMessage::AccessInvalidated(
            uptrakit_wire::AccessInvalidatedPayload::new(vec![], vec![role_id]),
        ))
        .await;
    Ok(StatusCode::NO_CONTENT.into_response())
}

// --- Helpers ---

fn role_to_response(m: &role::Model) -> RoleResponse {
    RoleResponse {
        id: m.id,
        name: m.name.clone(),
        description: m.description.clone(),
        is_built_in: m.is_built_in,
        tenant_id: m.tenant_id,
        created_at: m.created_at,
    }
}

/// 409 for a built-in target — `None` when the role is a mutable custom
/// role, so callers can `?`-free early-return with `if let Some(...)`.
fn built_in_immutable_response(existing: &role::Model) -> Option<Response> {
    existing.is_built_in.then(|| {
        error_response_with_code(
            StatusCode::CONFLICT,
            "Built-in roles are immutable",
            "built_in_role_immutable",
        )
    })
}

/// 409 for a name-collision probe hit — `None` on a clean probe.
fn collision_response(collision: Option<RoleNameCollision>) -> Option<Response> {
    match collision {
        None => None,
        Some(RoleNameCollision::Global) => Some(error_response_with_code(
            StatusCode::CONFLICT,
            "A global built-in role already uses this name",
            "role_name_shadows_global",
        )),
        Some(RoleNameCollision::Tenant) => Some(error_response_with_code(
            StatusCode::CONFLICT,
            "Another role in this tenant already uses this name",
            "role_name_taken",
        )),
    }
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;
    use crate::test_harness::TestApp;
    use crate::test_harness::fixtures::{
        link_role, open_registration, revoke_role_grants_covering, role_id_by_name,
        stage_user_with_grant, stage_zero_role_user,
    };
    use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
    use uptrakit_shared_db::access_grants::{NewGrant, insert_grant};
    use uptrakit_shared_db::entity::user_role;

    #[tokio::test]
    async fn role_crud_roundtrip_under_access_manage() {
        let app = TestApp::new().await;
        let client = app.client();
        open_registration(&app).await;
        let (_admin_id, token) = stage_user_with_grant(
            &app,
            "roles-admin@test.local",
            &["access:manage"],
            Some(app.tenant_id),
        )
        .await;

        let (_, before_list): (http::StatusCode, Vec<RoleResponse>) =
            client.get("/api/v1/roles").bearer(&token).send_json().await;
        let before_count = before_list.len();

        let (status, created): (http::StatusCode, RoleResponse) = client
            .post_json(
                "/api/v1/roles",
                &serde_json::json!({ "name": "custom-role", "description": "probe" }),
            )
            .bearer(&token)
            .send_json()
            .await;
        assert_eq!(status, http::StatusCode::CREATED);
        assert!(!created.is_built_in);
        assert_eq!(created.tenant_id, Some(app.tenant_id));
        assert_eq!(created.name, "custom-role");

        let (status, listed): (_, Vec<RoleResponse>) =
            client.get("/api/v1/roles").bearer(&token).send_json().await;
        assert_eq!(status, http::StatusCode::OK);
        assert_eq!(listed.len(), before_count + 1);

        let (status, fetched): (_, RoleResponse) = client
            .get(&format!("/api/v1/roles/{}", created.id))
            .bearer(&token)
            .send_json()
            .await;
        assert_eq!(status, http::StatusCode::OK);
        assert_eq!(fetched.id, created.id);

        let (status, renamed): (http::StatusCode, RoleResponse) = client
            .put_json(
                &format!("/api/v1/roles/{}", created.id),
                &serde_json::json!({ "name": "renamed-role", "description": "renamed" }),
            )
            .bearer(&token)
            .send_json()
            .await;
        assert_eq!(status, http::StatusCode::OK);
        assert_eq!(renamed.name, "renamed-role");

        let status = client
            .delete(&format!("/api/v1/roles/{}", created.id))
            .bearer(&token)
            .send_status()
            .await;
        assert_eq!(status, http::StatusCode::NO_CONTENT);

        let status = client
            .get(&format!("/api/v1/roles/{}", created.id))
            .bearer(&token)
            .send_status()
            .await;
        assert_eq!(status, http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn built_in_role_update_and_delete_are_409_built_in_role_immutable() {
        let app = TestApp::new().await;
        let client = app.client();
        open_registration(&app).await;
        let (_admin_id, token) = stage_user_with_grant(
            &app,
            "built-in-admin@test.local",
            &["access:manage"],
            Some(app.tenant_id),
        )
        .await;
        let viewer_id = role_id_by_name(&app, "viewer").await;

        let (status, body): (
            http::StatusCode,
            uptrakit_web_api_types::error::ErrorResponse,
        ) = client
            .put_json(
                &format!("/api/v1/roles/{viewer_id}"),
                &serde_json::json!({ "name": "viewer", "description": "nope" }),
            )
            .bearer(&token)
            .send_json()
            .await;
        assert_eq!(status, http::StatusCode::CONFLICT);
        assert_eq!(body.code, Some("built_in_role_immutable".to_string()));

        let (status, body): (
            http::StatusCode,
            uptrakit_web_api_types::error::ErrorResponse,
        ) = client
            .delete(&format!("/api/v1/roles/{viewer_id}"))
            .bearer(&token)
            .send_json()
            .await;
        assert_eq!(status, http::StatusCode::CONFLICT);
        assert_eq!(body.code, Some("built_in_role_immutable".to_string()));
    }

    #[tokio::test]
    async fn creating_role_named_viewer_is_409_role_name_shadows_global() {
        let app = TestApp::new().await;
        let client = app.client();
        open_registration(&app).await;
        let (_admin_id, token) = stage_user_with_grant(
            &app,
            "shadow-admin@test.local",
            &["access:manage"],
            Some(app.tenant_id),
        )
        .await;

        let (status, body): (
            http::StatusCode,
            uptrakit_web_api_types::error::ErrorResponse,
        ) = client
            .post_json(
                "/api/v1/roles",
                &serde_json::json!({ "name": "viewer", "description": null }),
            )
            .bearer(&token)
            .send_json()
            .await;
        assert_eq!(status, http::StatusCode::CONFLICT);
        assert_eq!(body.code, Some("role_name_shadows_global".to_string()));

        let (status, created): (http::StatusCode, RoleResponse) = client
            .post_json(
                "/api/v1/roles",
                &serde_json::json!({ "name": "not-yet-operator", "description": null }),
            )
            .bearer(&token)
            .send_json()
            .await;
        assert_eq!(status, http::StatusCode::CREATED);

        let (status, body): (
            http::StatusCode,
            uptrakit_web_api_types::error::ErrorResponse,
        ) = client
            .put_json(
                &format!("/api/v1/roles/{}", created.id),
                &serde_json::json!({ "name": "operator", "description": null }),
            )
            .bearer(&token)
            .send_json()
            .await;
        assert_eq!(status, http::StatusCode::CONFLICT);
        assert_eq!(body.code, Some("role_name_shadows_global".to_string()));
    }

    #[tokio::test]
    async fn role_delete_cascades_grants_and_assignments_and_drops_authority() {
        let app = TestApp::new().await;
        let client = app.client();
        open_registration(&app).await;
        let (_admin_id, token) = stage_user_with_grant(
            &app,
            "cascade-admin@test.local",
            &["access:manage"],
            Some(app.tenant_id),
        )
        .await;

        let (status, created): (http::StatusCode, RoleResponse) = client
            .post_json(
                "/api/v1/roles",
                &serde_json::json!({ "name": "hosts-reader", "description": null }),
            )
            .bearer(&token)
            .send_json()
            .await;
        assert_eq!(status, http::StatusCode::CREATED);

        insert_grant(
            &app.db,
            NewGrant {
                subject: GrantSubject::Role(created.id),
                tenant_id: None,
                patterns: &["hosts:read".parse().expect("test pattern")],
                selector: uptrakit_shared_types::access::Selector::All,
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("stage role-subject grant");

        let (holder_id, holder_token) = stage_zero_role_user(&app).await;
        link_role(&app, holder_id, created.id).await;

        let status = client
            .get("/api/v1/hosts")
            .bearer(&holder_token)
            .send_status()
            .await;
        assert_eq!(status, http::StatusCode::OK, "role grant must be active");

        let status = client
            .delete(&format!("/api/v1/roles/{}", created.id))
            .bearer(&token)
            .send_status()
            .await;
        assert_eq!(status, http::StatusCode::NO_CONTENT);

        let remaining = list_grants(&app.db, app.tenant_id, Some(GrantSubject::Role(created.id)))
            .await
            .expect("list grants")
            .grants;
        assert!(remaining.is_empty(), "role's grants must be cascaded");

        let assignment_count = user_role::Entity::find()
            .filter(user_role::Column::RoleId.eq(created.id))
            .count(&app.db)
            .await
            .expect("count assignments");
        assert_eq!(assignment_count, 0, "role's assignments must be cascaded");

        // No re-login: invalidation happens without the holder re-authenticating.
        let status = client
            .get("/api/v1/hosts")
            .bearer(&holder_token)
            .send_status()
            .await;
        assert_eq!(
            status,
            http::StatusCode::FORBIDDEN,
            "authority must drop immediately on role delete"
        );
    }

    #[tokio::test]
    async fn deleting_role_with_system_grants_requires_system_access_manage() {
        let app = TestApp::new().await;
        let client = app.client();
        open_registration(&app).await;
        let (_admin_id, tenant_only) = stage_user_with_grant(
            &app,
            "tenant-only-role-admin@test.local",
            &["access:manage"],
            Some(app.tenant_id),
        )
        .await;

        let (status, created): (http::StatusCode, RoleResponse) = client
            .post_json(
                "/api/v1/roles",
                &serde_json::json!({ "name": "system-role", "description": null }),
            )
            .bearer(&tenant_only)
            .send_json()
            .await;
        assert_eq!(status, http::StatusCode::CREATED);

        insert_grant(
            &app.db,
            NewGrant {
                subject: GrantSubject::Role(created.id),
                tenant_id: None,
                patterns: &["system.*:*".parse().expect("test pattern")],
                selector: uptrakit_shared_types::access::Selector::All,
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("stage system-plane role grant");

        let status = client
            .delete(&format!("/api/v1/roles/{}", created.id))
            .bearer(&tenant_only)
            .send_status()
            .await;
        assert_eq!(
            status,
            http::StatusCode::FORBIDDEN,
            "tenant-plane access:manage alone must not authorize deleting a system-plane role"
        );

        let (admin2_id, sys_admin) = stage_user_with_grant(
            &app,
            "sys-role-admin@test.local",
            &["access:manage"],
            Some(app.tenant_id),
        )
        .await;
        insert_grant(
            &app.db,
            NewGrant {
                subject: GrantSubject::User(admin2_id),
                tenant_id: None,
                patterns: &["system.access:manage".parse().expect("test pattern")],
                selector: uptrakit_shared_types::access::Selector::All,
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("stage system grant");
        app.state
            .access_engine
            .invalidate_subjects(&[admin2_id], &[]);

        let status = client
            .delete(&format!("/api/v1/roles/{}", created.id))
            .bearer(&sys_admin)
            .send_status()
            .await;
        // The role was never linked to any user, so deleting it cannot trip
        // a lockout for anybody — the fine check passing must yield 204.
        assert_eq!(status, http::StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn users_manage_only_principal_gets_403_on_role_routes_incl_reads() {
        let app = TestApp::new().await;
        let client = app.client();
        open_registration(&app).await;
        let (_id, token) = stage_user_with_grant(
            &app,
            "users-mgr-roles@test.local",
            &["users:manage"],
            Some(app.tenant_id),
        )
        .await;
        let viewer_id = role_id_by_name(&app, "viewer").await;

        let status = client
            .get("/api/v1/roles")
            .bearer(&token)
            .send_status()
            .await;
        assert_eq!(status, http::StatusCode::FORBIDDEN);

        let status = client
            .get(&format!("/api/v1/roles/{viewer_id}"))
            .bearer(&token)
            .send_status()
            .await;
        assert_eq!(status, http::StatusCode::FORBIDDEN);

        let status = client
            .post_json(
                "/api/v1/roles",
                &serde_json::json!({ "name": "nope", "description": null }),
            )
            .bearer(&token)
            .send_status()
            .await;
        assert_eq!(status, http::StatusCode::FORBIDDEN);

        let status = client
            .put_json(
                &format!("/api/v1/roles/{viewer_id}"),
                &serde_json::json!({ "name": "nope", "description": null }),
            )
            .bearer(&token)
            .send_status()
            .await;
        assert_eq!(status, http::StatusCode::FORBIDDEN);

        let status = client
            .delete(&format!("/api/v1/roles/{viewer_id}"))
            .bearer(&token)
            .send_status()
            .await;
        assert_eq!(status, http::StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn deleting_last_covering_role_is_409_lockout() {
        let app = TestApp::new().await;
        let client = app.client();
        open_registration(&app).await;
        let settings_manager_role_id = role_id_by_name(&app, "settings_manager").await;

        // Owner still holds `access:manage` via `settings_manager` while
        // creating the role and its covering grant.
        let (status, created): (http::StatusCode, RoleResponse) = client
            .post_json(
                "/api/v1/roles",
                &serde_json::json!({ "name": "sole-holder-role", "description": null }),
            )
            .bearer(&client_owner_token(&app).await)
            .send_json()
            .await;
        assert_eq!(status, http::StatusCode::CREATED);

        insert_grant(
            &app.db,
            NewGrant {
                subject: GrantSubject::Role(created.id),
                tenant_id: None,
                patterns: &["access:manage".parse().expect("test pattern")],
                selector: uptrakit_shared_types::access::Selector::All,
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("stage covering role grant");

        let (holder_id, holder_token) = stage_zero_role_user(&app).await;
        link_role(&app, holder_id, created.id).await;

        // Now strip the owner's own covering grant so the new role's
        // assignment is the SOLE remaining covering holder.
        revoke_role_grants_covering(
            &app,
            settings_manager_role_id,
            &[uptrakit_shared_types::access::actions::ACCESS_MANAGE],
        )
        .await;

        let (status, body): (
            http::StatusCode,
            uptrakit_web_api_types::error::ErrorResponse,
        ) = client
            .delete(&format!("/api/v1/roles/{}", created.id))
            .bearer(&holder_token)
            .send_json()
            .await;
        assert_eq!(status, http::StatusCode::CONFLICT);
        assert_eq!(body.code, Some("lockout_access_manage".to_string()));
    }

    /// Log back in as the already-registered owner to get a fresh bearer
    /// token, without re-registering (registration is opened exactly once
    /// per [`open_registration`] call).
    async fn client_owner_token(app: &TestApp) -> String {
        let client = app.client();
        let (status, auth) = crate::test_harness::fixtures::login_user(
            &client,
            "owner@test.local",
            "TestPassword123!",
        )
        .await;
        assert_eq!(status, http::StatusCode::OK, "owner login failed");
        auth.access_token.expose_secret().to_string()
    }
}
