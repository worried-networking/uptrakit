//! Grant management endpoints (`/api/v1/access/grants`) — M1.6a.
//!
//! Coarse gate: `access:manage` via [`CanManageAccess`]. Operations whose
//! request or target reaches the `system.` plane additionally require
//! `system.access:manage`, checked inline against the engine because the
//! requirement depends on request content — the static scope lists
//! deliberately carry only the coarse action (design doc, CI-gate decision).
//!
//! All persistence goes through the engine-owned
//! `uptrakit_shared_db::access_grants` module; its write-path validation
//! (pattern matrix, plane purity, tenant-encoding rule 2, B9 selector
//! phase gate, bounds) is authoritative — handlers add nothing on top.

use crate::AppState;
use crate::api_error::ApiError;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::action::{AccessAuthority, CanManageAccess, require_system_access};
use crate::middleware::require_auth::{AuthenticatedApiTokenId, authenticated_user_audit_actor};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{SqliteTransactionMode, TransactionOptions, TransactionTrait};
use std::sync::Arc;
use uptrakit_audit_log::{AbsentView, AuditEntry, AuditOutcome, Stateful};
use uptrakit_shared_db::access_grants::{
    GrantSubject, NewGrant, ResolvedGrant, insert_grant, list_grants, load_grant,
    patterns_reach_system_plane,
};
use uptrakit_shared_types::access::ActionPattern;
use uuid::Uuid;

pub use uptrakit_web_api_types::access_grants::{
    AccessGrantResponse, CreateAccessGrantRequest, GrantSubjectTypeParam, ListAccessGrantsQuery,
    UpdateAccessGrantRequest,
};

// --- Endpoints ---

/// Create a grant.
#[utoipa::path(
    post,
    path = "/api/v1/access/grants",
    request_body = CreateAccessGrantRequest,
    responses(
        (status = 201, description = "Grant created", body = AccessGrantResponse),
        (status = 400, description = "Validation, pattern-parse, or encoding error"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized. System-plane patterns additionally require system.access:manage (evaluated against the request body at runtime)."),
        (status = 409, description = "Per-subject grant limit reached")
    ),
    tag = "Access",
    security(("oauth2" = ["access:manage"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn create_access_grant(
    State(state): State<Arc<AppState>>,
    CanManageAccess(caller): CanManageAccess,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Extension(authority): Extension<AccessAuthority>,
    Validated(body): Validated<CreateAccessGrantRequest>,
) -> Result<Response, ApiError> {
    let api_token_id = api_token_id.map(|v| v.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&caller, api_token_id);

    let patterns: Vec<ActionPattern> = match body.patterns.iter().map(|p| p.parse()).collect() {
        Ok(p) => p,
        Err(e) => {
            return Ok(error_response(
                StatusCode::BAD_REQUEST,
                format!("Invalid pattern: {e}"),
            ));
        }
    };
    let system_plane = patterns_reach_system_plane(&patterns)?;
    if system_plane {
        // APPROVED: body-dependent fine check (corpus 07, restated invariant)
        if let Some(denied) = require_system_access(&state.access_engine, &authority) {
            return Ok(denied);
        }
    }
    let subject = match body.subject_type {
        GrantSubjectTypeParam::User => GrantSubject::User(body.subject_id),
        GrantSubjectTypeParam::Role => GrantSubject::Role(body.subject_id),
        // GrantSubjectTypeParam is #[non_exhaustive] in another crate.
        _ => {
            return Ok(error_response(
                StatusCode::BAD_REQUEST,
                "Unknown subject type",
            ));
        }
    };
    // Encoding rule 2: role-subject and system-plane rows are global.
    let tenant_id = match (subject, system_plane) {
        (GrantSubject::User(_), false) => Some(state.default_tenant_id),
        _ => None,
    };

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
            tracing::error!("Failed to begin transaction for grant create: {e}");
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    let grant_id = match insert_grant(
        &tx,
        NewGrant {
            subject,
            tenant_id,
            patterns: &patterns,
            selector: body.selector,
            description: body.description,
            created_by: Some(caller.user_id),
        },
    )
    .await
    {
        Ok(id) => id,
        Err(e) => {
            drop(tx);
            return Err(e.into());
        }
    };
    let resolved = match load_grant(&tx, grant_id).await {
        Ok(g) => g,
        Err(e) => {
            drop(tx);
            return Err(e.into());
        }
    };
    let view = AccessGrantView::from(&resolved);
    let hook = state.audit_emitter.commit_hook();
    let mut audit_builder = AuditEntry::<Stateful>::access_grant_create(&AbsentView(&view), &view)
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::Success);
    audit_builder = match tenant_id {
        Some(t) => audit_builder.tenant_scope(t),
        None => audit_builder.system_scope(),
    };
    let audit_entry = match audit_builder.build() {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!("Failed to build audit entry for grant create: {e}");
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
        tracing::error!("Failed to emit audit entry for grant create: {e}");
        drop(tx);
        return Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error",
        ));
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit grant create: {e}");
        return Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error",
        ));
    }
    hook.flush_after_commit().await;

    let (user_ids, role_ids) = subject_invalidation_ids(subject);
    state
        .access_engine
        .invalidate_subjects(&user_ids, &role_ids);
    state
        .notification
        .notification_service
        .publish_controller_event(uptrakit_wire::ControllerMessage::AccessInvalidated(
            uptrakit_wire::AccessInvalidatedPayload::new(user_ids, role_ids),
        ))
        .await;
    Ok((StatusCode::CREATED, Json(resolved_to_response(&resolved))).into_response())
}

/// List grants (active tenant plus global rows), optionally one subject's.
#[utoipa::path(
    get,
    path = "/api/v1/access/grants",
    params(ListAccessGrantsQuery),
    responses(
        (status = 200, description = "Grants for the active tenant plus global rows", body = [AccessGrantResponse]),
        (status = 400, description = "subject_type and subject_id must be supplied together"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Access",
    security(("oauth2" = ["access:manage"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_access_grants(
    State(state): State<Arc<AppState>>,
    CanManageAccess(_user): CanManageAccess,
    Query(params): Query<ListAccessGrantsQuery>,
) -> Result<Response, ApiError> {
    let subject = match (params.subject_type, params.subject_id) {
        (None, None) => None,
        (Some(GrantSubjectTypeParam::User), Some(id)) => Some(GrantSubject::User(id)),
        (Some(GrantSubjectTypeParam::Role), Some(id)) => Some(GrantSubject::Role(id)),
        _ => {
            return Ok(error_response(
                StatusCode::BAD_REQUEST,
                "subject_type and subject_id must be supplied together",
            ));
        }
    };
    let load = list_grants(state.db(), state.default_tenant_id, subject).await?;
    let out: Vec<AccessGrantResponse> = load.grants.iter().map(resolved_to_response).collect();
    Ok((StatusCode::OK, Json(out)).into_response())
}

/// Get a single grant.
#[utoipa::path(
    get,
    path = "/api/v1/access/grants/{id}",
    params(
        ("id" = Uuid, Path, description = "Grant id")
    ),
    responses(
        (status = 200, description = "Grant details", body = AccessGrantResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Grant not found")
    ),
    tag = "Access",
    security(("oauth2" = ["access:manage"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_access_grant(
    State(state): State<Arc<AppState>>,
    CanManageAccess(_user): CanManageAccess,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let resolved = load_grant(state.db(), id).await?;
    if resolved.tenant_id.is_some() && resolved.tenant_id != Some(state.default_tenant_id) {
        return Ok(error_response(StatusCode::NOT_FOUND, "Grant not found"));
    }
    Ok((StatusCode::OK, Json(resolved_to_response(&resolved))).into_response())
}

// --- Helpers ---

fn subject_invalidation_ids(subject: GrantSubject) -> (Vec<Uuid>, Vec<Uuid>) {
    match subject {
        GrantSubject::User(id) => (vec![id], vec![]),
        GrantSubject::Role(id) => (vec![], vec![id]),
    }
}

fn resolved_to_response(g: &ResolvedGrant) -> AccessGrantResponse {
    let (subject_type, subject_id) = match g.subject {
        GrantSubject::User(id) => (GrantSubjectTypeParam::User, id),
        GrantSubject::Role(id) => (GrantSubjectTypeParam::Role, id),
    };
    AccessGrantResponse {
        id: g.id,
        tenant_id: g.tenant_id,
        subject_type,
        subject_id,
        patterns: g.patterns.iter().map(ToString::to_string).collect(),
        selector: g.selector.clone(),
        description: g.description.clone(),
    }
}

/// Audit snapshot for a grant. Patterns/selector/description are
/// non-secret data; `tenant_id` is part of the row's encoding (NULL =
/// global) and is included deliberately.
#[derive(uptrakit_audit_log::AuditView)]
#[audit(target_type = "access_grant")]
pub(crate) struct AccessGrantView {
    pub id: Uuid,
    pub tenant_id: Option<Uuid>,
    pub subject_type: String,
    pub subject_id: Uuid,
    pub patterns: Vec<String>,
    pub selector: serde_json::Value,
    pub description: Option<String>,
}

impl From<&ResolvedGrant> for AccessGrantView {
    fn from(g: &ResolvedGrant) -> Self {
        let (subject_type, subject_id) = match g.subject {
            GrantSubject::User(id) => ("user".to_string(), id),
            GrantSubject::Role(id) => ("role".to_string(), id),
        };
        Self {
            id: g.id,
            tenant_id: g.tenant_id,
            subject_type,
            subject_id,
            patterns: g.patterns.iter().map(ToString::to_string).collect(),
            selector: serde_json::to_value(&g.selector).unwrap_or_default(),
            description: g.description.clone(),
        }
    }
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    #![expect(
        clippy::expect_used,
        reason = "test code: panics on failure are acceptable"
    )]
    #![expect(clippy::panic, reason = "test code: panics on failure are acceptable")]

    use super::*;
    use crate::test_harness::TestApp;
    use crate::test_harness::fixtures::{
        open_registration, stage_user_with_grant, stage_zero_role_user,
    };
    use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
    use uptrakit_shared_db::entity::audit_log;

    async fn latest_grant_create_audit_row_for_target(
        db: &sea_orm::DatabaseConnection,
        target_grant_id: Uuid,
    ) -> audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = audit_log::Entity::find()
                .filter(
                    audit_log::Column::ActionType
                        .eq(uptrakit_audit_log::AuditActionType::ACCESS_GRANT_CREATE),
                )
                .filter(audit_log::Column::TargetType.eq("access_grant"))
                .filter(audit_log::Column::TargetId.eq(target_grant_id.to_string()))
                .order_by_desc(audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected access_grant.create audit row");
    }

    #[tokio::test]
    async fn create_and_list_and_get_roundtrip_under_access_manage() {
        let app = TestApp::new().await;
        let client = app.client();
        open_registration(&app).await;
        let (_admin_id, token) = stage_user_with_grant(
            &app,
            "grants-admin@test.local",
            &["access:manage"],
            Some(app.tenant_id),
        )
        .await;
        let (subject_id, _) = stage_zero_role_user(&app).await;

        let (status, created): (http::StatusCode, AccessGrantResponse) = client
            .post_json(
                "/api/v1/access/grants",
                &serde_json::json!({
                    "subject_type": "user",
                    "subject_id": subject_id,
                    "patterns": ["hosts:read"],
                    "description": "probe"
                }),
            )
            .bearer(&token)
            .send_json()
            .await;
        assert_eq!(status, http::StatusCode::CREATED);
        assert_eq!(
            created.tenant_id,
            Some(app.tenant_id),
            "tenant-plane user grant gets the active tenant"
        );
        assert_eq!(created.patterns, vec!["hosts:read".to_string()]);

        let (status, listed): (_, Vec<AccessGrantResponse>) = client
            .get(&format!(
                "/api/v1/access/grants?subject_type=user&subject_id={subject_id}"
            ))
            .bearer(&token)
            .send_json()
            .await;
        assert_eq!(status, http::StatusCode::OK);
        assert_eq!(listed.len(), 1);

        let (status, fetched): (_, AccessGrantResponse) = client
            .get(&format!("/api/v1/access/grants/{}", created.id))
            .bearer(&token)
            .send_json()
            .await;
        assert_eq!(status, http::StatusCode::OK);
        assert_eq!(fetched.id, created.id);
    }

    #[tokio::test]
    async fn users_manage_only_principal_gets_403_on_grant_routes() {
        // E2 one direction for this family; Task 6 owns the lifecycle side.
        let app = TestApp::new().await;
        let client = app.client();
        open_registration(&app).await;
        let (_id, token) = stage_user_with_grant(
            &app,
            "users-mgr@test.local",
            &["users:manage"],
            Some(app.tenant_id),
        )
        .await;
        let status = client
            .get("/api/v1/access/grants")
            .bearer(&token)
            .send_status()
            .await;
        assert_eq!(status, http::StatusCode::FORBIDDEN);
        let status = client
            .post_json(
                "/api/v1/access/grants",
                &serde_json::json!({
                    "subject_type": "user", "subject_id": uuid::Uuid::now_v7(), "patterns": ["hosts:read"]
                }),
            )
            .bearer(&token)
            .send_status()
            .await;
        assert_eq!(status, http::StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn minting_system_plane_grant_requires_system_access_manage() {
        // New-row matrix: access:manage alone → 403 on system-plane body;
        // + system.access:manage (via covering system grant) → 200/201.
        let app = TestApp::new().await;
        let client = app.client();
        open_registration(&app).await;
        let (_id, tenant_only) = stage_user_with_grant(
            &app,
            "tenant-admin@test.local",
            &["access:manage"],
            Some(app.tenant_id),
        )
        .await;
        let (victim_id, _) = stage_zero_role_user(&app).await;
        let body = serde_json::json!({
            "subject_type": "user", "subject_id": victim_id, "patterns": ["system.*:*"]
        });
        let status = client
            .post_json("/api/v1/access/grants", &body)
            .bearer(&tenant_only)
            .send_status()
            .await;
        assert_eq!(
            status,
            http::StatusCode::FORBIDDEN,
            "tenant admin must not mint system-plane authority"
        );

        // Second principal: tenant-plane access:manage grant PLUS a global
        // system.access:manage grant (two inserts — planes cannot mix in one row).
        let (admin2_id, sys_admin) = stage_user_with_grant(
            &app,
            "sys-admin@test.local",
            &["access:manage"],
            Some(app.tenant_id),
        )
        .await;
        insert_grant(
            &app.db,
            NewGrant {
                subject: GrantSubject::User(admin2_id),
                tenant_id: None,
                patterns: &["system.access:manage".parse().expect("p")],
                selector: uptrakit_shared_types::access::Selector::All,
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("system grant");
        app.state
            .access_engine
            .invalidate_subjects(&[admin2_id], &[]);
        let status = client
            .post_json("/api/v1/access/grants", &body)
            .bearer(&sys_admin)
            .send_status()
            .await;
        assert_eq!(status, http::StatusCode::CREATED);
    }

    #[tokio::test]
    async fn grant_create_writes_stateful_audit_row_with_snapshots() {
        let app = TestApp::new().await;
        let client = app.client();
        open_registration(&app).await;
        let (_admin_id, token) = stage_user_with_grant(
            &app,
            "audit-admin@test.local",
            &["access:manage"],
            Some(app.tenant_id),
        )
        .await;
        let (subject_id, _) = stage_zero_role_user(&app).await;

        let (status, created): (http::StatusCode, AccessGrantResponse) = client
            .post_json(
                "/api/v1/access/grants",
                &serde_json::json!({
                    "subject_type": "user",
                    "subject_id": subject_id,
                    "patterns": ["hosts:read"],
                    "description": "audit probe"
                }),
            )
            .bearer(&token)
            .send_json()
            .await;
        assert_eq!(status, http::StatusCode::CREATED);

        let row = latest_grant_create_audit_row_for_target(&app.db, created.id).await;
        assert_eq!(
            uptrakit_audit_log::AuditActionType::ACCESS_GRANT_CREATE,
            row.action_type
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        // before snapshot is absent ({}), after snapshot has grant fields
        let before = row.before_snapshot.expect("before_snapshot");
        assert_eq!(before, serde_json::json!({}));
        let after = row.after_snapshot.expect("after_snapshot");
        assert_eq!(after["patterns"], serde_json::json!(["hosts:read"]));
        assert_eq!(after["subject_type"], serde_json::json!("user"));
    }

    /// Insert a second, non-default tenant row. `access_grants.tenant_id`
    /// carries an FK to `tenants.id`, so a genuinely separate tenant is
    /// required (a bare `Uuid::now_v7()` would fail the FK) — mirrors the
    /// `insert_tenant` idiom in `oauth/services/refresh_token.rs`.
    async fn insert_other_tenant(db: &sea_orm::DatabaseConnection) -> Uuid {
        let now = time::OffsetDateTime::now_utc();
        let id = Uuid::now_v7();
        uptrakit_shared_db::entity::tenant::ActiveModel {
            id: sea_orm::Set(id),
            name: sea_orm::Set("other-tenant".into()),
            slug: sea_orm::Set(id.to_string()),
            is_default: sea_orm::Set(false),
            created_at: sea_orm::Set(now),
            updated_at: sea_orm::Set(now),
            deactivated_at: sea_orm::Set(None),
        }
        .insert(db)
        .await
        .expect("insert other tenant");
        id
    }

    #[tokio::test]
    async fn get_access_grant_from_other_tenant_returns_404() {
        let app = TestApp::new().await;
        let client = app.client();
        open_registration(&app).await;
        let (_admin_id, token) = stage_user_with_grant(
            &app,
            "cross-tenant-admin@test.local",
            &["access:manage"],
            Some(app.tenant_id),
        )
        .await;

        let other_tenant_id = insert_other_tenant(&app.db).await;
        let pattern: ActionPattern = "hosts:read".parse().expect("test pattern");
        let other_tenant_grant_id = insert_grant(
            &app.db,
            NewGrant {
                subject: GrantSubject::User(Uuid::now_v7()),
                tenant_id: Some(other_tenant_id),
                patterns: &[pattern],
                selector: uptrakit_shared_types::access::Selector::All,
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("stage other-tenant grant");

        let status = client
            .get(&format!("/api/v1/access/grants/{other_tenant_grant_id}"))
            .bearer(&token)
            .send_status()
            .await;
        assert_eq!(
            status,
            http::StatusCode::NOT_FOUND,
            "a grant belonging to another tenant must not be visible"
        );
    }
}
