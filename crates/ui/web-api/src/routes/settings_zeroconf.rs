//! HTTP handlers for `GET /api/v1/global-settings/zeroconf` and
//! `PUT /api/v1/global-settings/zeroconf`.
//!
//! Zeroconf settings control automatic service discovery and enrollment via
//! mDNS/DNS-SD. Changes take effect after the controller is restarted.

use std::sync::Arc;

use axum::{
    Extension, Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{SqliteTransactionMode, TransactionOptions, TransactionTrait};
use uptrakit_audit_log::{AbsentView, AuditEntry, AuditOutcome, Event, Stateful};
use uptrakit_web_api_queries::queries::global_settings::GlobalSettingView;

pub use uptrakit_web_api_types::settings_zeroconf::{
    UpdateZeroconfSettingsRequest, ZeroconfSettingsResponse,
};

use crate::AppState;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::extractors::{IfMatch, SettingsVersion};
use crate::middleware::permission::CanManageGlobalSettings;
use crate::middleware::require_auth::{AuthenticatedApiTokenId, authenticated_user_audit_actor};
use crate::settings::ZeroconfSnapshot;
use crate::settings_store::{load_global_setting_raw, upsert_global_setting_raw};

/// Emit a failed zeroconf settings audit event (no DB write).
fn emit_zeroconf_failed_event(
    state: &AppState,
    actor_type: uptrakit_audit_log::AuditActorType,
    actor_id: Option<uuid::Uuid>,
    details: serde_json::Value,
) {
    if let Ok(entry) = AuditEntry::<Event>::builder_event(
        uptrakit_audit_log::AuditActionType::GLOBAL_SETTING_UPDATE,
    )
    .system_scope()
    .actor(actor_type, actor_id)
    .target(
        "global_setting",
        "zeroconf".to_string(),
        Some("zeroconf".to_string()),
    )
    .outcome(AuditOutcome::Failed)
    .details(details)
    .build()
    {
        state.audit_emitter.emit_event(entry);
    }
}

/// Get zeroconf settings
///
/// Returns the current zeroconf discovery configuration including the
/// read-only CA fingerprint used for trust-on-first-use verification.
#[utoipa::path(
    get,
    path = "/api/v1/global-settings/zeroconf",
    responses(
        (status = 200, description = "Zeroconf settings", body = ZeroconfSettingsResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Global Settings",
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_zeroconf_settings(
    State(state): State<Arc<AppState>>,
    CanManageGlobalSettings(_user): CanManageGlobalSettings,
) -> Response {
    let snap = state.settings.zeroconf();
    let ca_fingerprint = state.cert.ca_snapshot.borrow().active_fingerprint.clone();
    let resp = ZeroconfSettingsResponse {
        enabled: snap.enabled,
        url: snap.url,
        pki_addr: snap.pki_addr,
        ca_fingerprint: Some(ca_fingerprint),
    };
    (StatusCode::OK, Json(resp)).into_response()
}

/// Update zeroconf settings
///
/// Update the zeroconf discovery configuration. All fields are optional —
/// omitted fields keep their current value.
///
/// - `url`: empty string clears the value; non-empty must start with `https://`.
/// - `pki_addr`: empty string clears the value; non-empty must start with
///   `http://` or `https://`.
#[utoipa::path(
    put,
    path = "/api/v1/global-settings/zeroconf",
    request_body = UpdateZeroconfSettingsRequest,
    responses(
        (status = 200, description = "Zeroconf settings updated", body = ZeroconfSettingsResponse),
        (status = 400, description = "Invalid values"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Global Settings",
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_zeroconf_settings(
    State(state): State<Arc<AppState>>,
    _if_match: IfMatch<SettingsVersion>,
    CanManageGlobalSettings(user): CanManageGlobalSettings,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Validated(req): Validated<UpdateZeroconfSettingsRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let mut snap = state.settings.zeroconf();

    // Nothing requested — return immediately without opening a transaction.
    if req.enabled.is_none() && req.url.is_none() && req.pki_addr.is_none() {
        let ca_fingerprint = state.cert.ca_snapshot.borrow().active_fingerprint.clone();
        return (
            StatusCode::OK,
            Json(ZeroconfSettingsResponse {
                enabled: snap.enabled,
                url: snap.url,
                pki_addr: snap.pki_addr,
                ca_fingerprint: Some(ca_fingerprint),
            }),
        )
            .into_response();
    }

    // Read current DB values for each key that will be updated, before opening
    // the transaction, so we can build accurate before-snapshots.
    let before_enabled = if req.enabled.is_some() {
        load_global_setting_raw(state.db(), "zeroconf.enabled")
            .await
            .unwrap_or(None)
    } else {
        None
    };
    let before_url = if req.url.is_some() {
        load_global_setting_raw(state.db(), "zeroconf.url")
            .await
            .unwrap_or(None)
    } else {
        None
    };
    let before_pki_addr = if req.pki_addr.is_some() {
        load_global_setting_raw(state.db(), "zeroconf.pki_addr")
            .await
            .unwrap_or(None)
    } else {
        None
    };

    // Open a single BEGIN IMMEDIATE transaction for all zeroconf writes.
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
            tracing::error!("Failed to begin tx for zeroconf settings update: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };
    let hook = state.audit_emitter.commit_hook();

    // --- enabled ---
    if let Some(val) = req.enabled {
        let new_value = serde_json::json!(val);
        if let Err(e) = upsert_global_setting_raw(&tx, "zeroconf.enabled", new_value.clone()).await
        {
            tracing::error!("Failed to save zeroconf.enabled: {e:?}");
            drop(tx);
            emit_zeroconf_failed_event(
                &state,
                actor_type,
                actor_id,
                serde_json::json!({
                    "setting_area": "zeroconf",
                    "reason_code": "zeroconf_enabled_upsert_failed",
                    "setting_key": "zeroconf.enabled",
                    "requested_enabled": val,
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }

        let key_str = "zeroconf.enabled".to_string();
        let after_view = GlobalSettingView {
            key: key_str.clone(),
            value: new_value,
        };
        let audit_entry = match before_enabled {
            Some(bv) => {
                let before_view = GlobalSettingView {
                    key: key_str,
                    value: bv,
                };
                AuditEntry::<Stateful>::global_setting_update(&before_view, &after_view)
            }
            None => {
                AuditEntry::<Stateful>::global_setting_update(&AbsentView(&after_view), &after_view)
            }
        }
        .system_scope()
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::Success)
        .details(serde_json::json!({
            "setting_area": "zeroconf",
            "changed_keys": ["zeroconf.enabled"],
        }))
        .build();

        match audit_entry {
            Ok(entry) => {
                if let Err(e) = state.audit_emitter.emit_stateful(&tx, &hook, entry).await {
                    tracing::error!("Failed to emit stateful audit for zeroconf.enabled: {e}");
                    drop(tx);
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            }
            Err(e) => {
                tracing::error!("Failed to build audit entry for zeroconf.enabled: {e}");
                drop(tx);
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        }

        snap.enabled = val;
    }

    // --- url ---
    if let Some(ref raw) = req.url {
        let trimmed = raw.trim_end_matches('/');
        let (db_value, new_url) = if trimmed.is_empty() {
            (serde_json::Value::Null, None)
        } else {
            (serde_json::json!(trimmed), Some(trimmed.to_string()))
        };
        if let Err(e) = upsert_global_setting_raw(&tx, "zeroconf.url", db_value.clone()).await {
            tracing::error!("Failed to save zeroconf.url: {e:?}");
            drop(tx);
            emit_zeroconf_failed_event(
                &state,
                actor_type,
                actor_id,
                serde_json::json!({
                    "setting_area": "zeroconf",
                    "reason_code": "zeroconf_url_upsert_failed",
                    "setting_key": "zeroconf.url",
                    "requested_url_cleared": trimmed.is_empty(),
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }

        let key_str = "zeroconf.url".to_string();
        let after_view = GlobalSettingView {
            key: key_str.clone(),
            value: db_value,
        };
        let audit_entry = match before_url {
            Some(bv) => {
                let before_view = GlobalSettingView {
                    key: key_str,
                    value: bv,
                };
                AuditEntry::<Stateful>::global_setting_update(&before_view, &after_view)
            }
            None => {
                AuditEntry::<Stateful>::global_setting_update(&AbsentView(&after_view), &after_view)
            }
        }
        .system_scope()
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::Success)
        .details(serde_json::json!({
            "setting_area": "zeroconf",
            "changed_keys": ["zeroconf.url"],
        }))
        .build();

        match audit_entry {
            Ok(entry) => {
                if let Err(e) = state.audit_emitter.emit_stateful(&tx, &hook, entry).await {
                    tracing::error!("Failed to emit stateful audit for zeroconf.url: {e}");
                    drop(tx);
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            }
            Err(e) => {
                tracing::error!("Failed to build audit entry for zeroconf.url: {e}");
                drop(tx);
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        }

        snap.url = new_url;
    }

    // --- pki_addr ---
    if let Some(ref raw) = req.pki_addr {
        let trimmed = raw.trim_end_matches('/');
        let (db_value, new_addr) = if trimmed.is_empty() {
            (serde_json::Value::Null, None)
        } else {
            (serde_json::json!(trimmed), Some(trimmed.to_string()))
        };
        if let Err(e) = upsert_global_setting_raw(&tx, "zeroconf.pki_addr", db_value.clone()).await
        {
            tracing::error!("Failed to save zeroconf.pki_addr: {e:?}");
            drop(tx);
            emit_zeroconf_failed_event(
                &state,
                actor_type,
                actor_id,
                serde_json::json!({
                    "setting_area": "zeroconf",
                    "reason_code": "zeroconf_pki_addr_upsert_failed",
                    "setting_key": "zeroconf.pki_addr",
                    "requested_pki_addr_cleared": trimmed.is_empty(),
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }

        let key_str = "zeroconf.pki_addr".to_string();
        let after_view = GlobalSettingView {
            key: key_str.clone(),
            value: db_value,
        };
        let audit_entry = match before_pki_addr {
            Some(bv) => {
                let before_view = GlobalSettingView {
                    key: key_str,
                    value: bv,
                };
                AuditEntry::<Stateful>::global_setting_update(&before_view, &after_view)
            }
            None => {
                AuditEntry::<Stateful>::global_setting_update(&AbsentView(&after_view), &after_view)
            }
        }
        .system_scope()
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::Success)
        .details(serde_json::json!({
            "setting_area": "zeroconf",
            "changed_keys": ["zeroconf.pki_addr"],
        }))
        .build();

        match audit_entry {
            Ok(entry) => {
                if let Err(e) = state.audit_emitter.emit_stateful(&tx, &hook, entry).await {
                    tracing::error!("Failed to emit stateful audit for zeroconf.pki_addr: {e}");
                    drop(tx);
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            }
            Err(e) => {
                tracing::error!("Failed to build audit entry for zeroconf.pki_addr: {e}");
                drop(tx);
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        }

        snap.pki_addr = new_addr;
    }

    // Commit the transaction (all key writes + all audit rows).
    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit zeroconf settings update: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    // Update in-memory cache.
    let updated = ZeroconfSnapshot::new(snap.enabled, snap.url.clone(), snap.pki_addr.clone());
    state.settings.set_zeroconf(updated).await;

    let ca_fingerprint = state.cert.ca_snapshot.borrow().active_fingerprint.clone();
    let resp = ZeroconfSettingsResponse {
        enabled: snap.enabled,
        url: snap.url,
        pki_addr: snap.pki_addr,
        ca_fingerprint: Some(ca_fingerprint),
    };

    (StatusCode::OK, Json(resp)).into_response()
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    #![expect(
        clippy::expect_used,
        reason = "test code: panics on failure are acceptable"
    )]
    #![expect(clippy::panic, reason = "test code: panics on failure are acceptable")]

    use super::*;
    use crate::auth::AuthMethod;
    use crate::auth::permissions::Permission;
    use crate::middleware::permission::CanManageGlobalSettings;
    use crate::middleware::require_auth::AuthenticatedUser;
    use sea_orm::{
        ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    };
    use uptrakit_shared_db::entity::system_audit_log;

    async fn latest_system_audit_row(
        db: &sea_orm::DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
    ) -> system_audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = system_audit_log::Entity::find()
                .filter(system_audit_log::Column::ActionType.eq(action_type))
                .order_by_desc(system_audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query system audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected system audit row for action {action_type}");
    }

    async fn wait_for_system_audit_rows(db: &sea_orm::DatabaseConnection, expected: u64) {
        for _ in 0..50 {
            let count = system_audit_log::Entity::find()
                .count(db)
                .await
                .expect("count system audit rows");
            if count == expected {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected {expected} system audit rows");
    }

    #[tokio::test]
    async fn update_zeroconf_settings_persistence_failure_writes_failed_system_audit_event() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

        db.execute_unprepared("DROP TABLE global_settings")
            .await
            .expect("drop global_settings table");

        let user_id = uuid::Uuid::now_v7();
        let response = update_zeroconf_settings(
            State(Arc::clone(&state)),
            crate::extractors::IfMatch::for_test(),
            CanManageGlobalSettings::new(AuthenticatedUser::new(
                user_id,
                AuthMethod::Password,
                vec![Permission::ManageGlobalSettings],
                None,
            )),
            None,
            Validated(UpdateZeroconfSettingsRequest {
                enabled: Some(true),
                url: None,
                pki_addr: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        wait_for_system_audit_rows(&db, 1).await;
        let row = latest_system_audit_row(
            &db,
            uptrakit_audit_log::AuditActionType::GLOBAL_SETTING_UPDATE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::User.as_str()
        );
        assert_eq!(row.actor_id, Some(user_id));
        assert_eq!(row.target_type.as_deref(), Some("global_setting"));
        assert_eq!(row.target_id.as_deref(), Some("zeroconf"));
        let details = row.details_json.expect("details");
        assert_eq!(details["setting_area"], serde_json::json!("zeroconf"));
        assert_eq!(
            details["reason_code"],
            serde_json::json!("zeroconf_enabled_upsert_failed")
        );
        assert_eq!(details["requested_enabled"], serde_json::json!(true));
    }
}
