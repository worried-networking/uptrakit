//! HTTP handlers for `GET /api/v1/global-settings/oauth` and
//! `PUT /api/v1/global-settings/oauth`.
//!
//! These settings drive the MCP OAuth 2.1 authorization-server feature.
//! Changes are persisted to `global_settings` and take effect after the
//! controller is restarted.

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

pub use uptrakit_web_api_types::settings_oauth::{
    OAuthSettingsResponse, UpdateOAuthSettingsRequest,
};

use crate::AppState;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::extractors::{IfMatch, SettingsVersion};
use crate::middleware::permission::CanManageGlobalSettings;
use crate::middleware::require_auth::{AuthenticatedApiTokenId, authenticated_user_audit_actor};
use crate::oauth::resolve_mcp_enabled;
use crate::settings_store::{load_global_setting_raw, upsert_global_setting_raw};

/// Load all four OAuth settings from DB. Missing keys fall back to defaults.
async fn load_oauth_settings_from_db(state: &AppState) -> OAuthSettingsFromDb {
    let canonical_host = load_global_setting_raw(state.db(), "oauth.canonical_host")
        .await
        .unwrap_or(None)
        .and_then(|v| v.as_str().map(ToOwned::to_owned));

    let mcp_raw: Option<bool> = load_global_setting_raw(state.db(), "oauth.mcp_enabled")
        .await
        .unwrap_or(None)
        .and_then(|v| v.as_bool());
    let mcp = resolve_mcp_enabled(mcp_raw, canonical_host.as_deref());

    let dcr = load_global_setting_raw(state.db(), "oauth.dcr_enabled")
        .await
        .unwrap_or(None)
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let cimd = load_global_setting_raw(state.db(), "oauth.cimd_enabled")
        .await
        .unwrap_or(None)
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    OAuthSettingsFromDb {
        mcp,
        dcr,
        cimd,
        canonical_host,
    }
}

struct OAuthSettingsFromDb {
    mcp: bool,
    dcr: bool,
    cimd: bool,
    canonical_host: Option<String>,
}

impl OAuthSettingsFromDb {
    fn restart_required(&self, state: &AppState) -> bool {
        self.mcp != state.oauth.enabled
            || self.dcr != state.oauth.dcr_enabled
            || self.cimd != state.oauth.cimd_enabled
    }

    fn into_response(self, state: &AppState) -> OAuthSettingsResponse {
        let restart_required = self.restart_required(state);
        OAuthSettingsResponse {
            mcp_enabled: self.mcp,
            dcr_enabled: self.dcr,
            cimd_enabled: self.cimd,
            canonical_host: self.canonical_host,
            restart_required,
        }
    }
}

fn emit_oauth_failed_event(
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
        "oauth".to_string(),
        Some("oauth".to_string()),
    )
    .outcome(AuditOutcome::Failed)
    .details(details)
    .build()
    {
        state.audit_emitter.emit_event(entry);
    }
}

/// Get OAuth settings
///
/// Returns the current MCP OAuth authorization-server configuration.
/// Values are read from `global_settings` and reflect what will take effect
/// on next restart. The `restart_required` field indicates whether the
/// in-memory (boot-time) state differs from the persisted values.
#[utoipa::path(
    get,
    path = "/api/v1/global-settings/oauth",
    responses(
        (status = 200, description = "OAuth settings", body = OAuthSettingsResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Global Settings",
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_oauth_settings(
    State(state): State<Arc<AppState>>,
    CanManageGlobalSettings(_user): CanManageGlobalSettings,
) -> Response {
    let db_state = load_oauth_settings_from_db(&state).await;
    (StatusCode::OK, Json(db_state.into_response(&state))).into_response()
}

/// Update OAuth settings
///
/// Update the MCP OAuth authorization-server configuration. All fields are
/// optional — omitted fields keep their current value. Changes are written to
/// `global_settings` and take effect after the controller is restarted.
#[utoipa::path(
    put,
    path = "/api/v1/global-settings/oauth",
    request_body = UpdateOAuthSettingsRequest,
    responses(
        (status = 200, description = "OAuth settings updated", body = OAuthSettingsResponse),
        (status = 400, description = "Invalid values"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Global Settings",
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_oauth_settings(
    State(state): State<Arc<AppState>>,
    _if_match: IfMatch<SettingsVersion>,
    CanManageGlobalSettings(user): CanManageGlobalSettings,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Validated(req): Validated<UpdateOAuthSettingsRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|v| v.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);

    // Nothing to update — return current state immediately.
    if req.mcp_enabled.is_none()
        && req.dcr_enabled.is_none()
        && req.cimd_enabled.is_none()
        && req.canonical_host.is_none()
    {
        let db_state = load_oauth_settings_from_db(&state).await;
        return (StatusCode::OK, Json(db_state.into_response(&state))).into_response();
    }

    // Read before-values for keys being updated.
    let before_mcp = if req.mcp_enabled.is_some() {
        load_global_setting_raw(state.db(), "oauth.mcp_enabled")
            .await
            .unwrap_or(None)
    } else {
        None
    };
    let before_dcr = if req.dcr_enabled.is_some() {
        load_global_setting_raw(state.db(), "oauth.dcr_enabled")
            .await
            .unwrap_or(None)
    } else {
        None
    };
    let before_cimd = if req.cimd_enabled.is_some() {
        load_global_setting_raw(state.db(), "oauth.cimd_enabled")
            .await
            .unwrap_or(None)
    } else {
        None
    };
    let before_canonical = if req.canonical_host.is_some() {
        load_global_setting_raw(state.db(), "oauth.canonical_host")
            .await
            .unwrap_or(None)
    } else {
        None
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
            tracing::error!("Failed to begin tx for oauth settings update: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };
    let hook = state.audit_emitter.commit_hook();

    macro_rules! write_bool_setting {
        ($opt:expr, $key:expr, $before:expr, $label:expr) => {
            if let Some(val) = $opt {
                let new_value = serde_json::json!(val);
                if let Err(e) = upsert_global_setting_raw(&tx, $key, new_value.clone()).await {
                    tracing::error!("Failed to save {}: {e:?}", $key);
                    drop(tx);
                    emit_oauth_failed_event(
                        &state,
                        actor_type,
                        actor_id,
                        serde_json::json!({
                            "setting_area": "oauth",
                            "reason_code": concat!("oauth_", $label, "_upsert_failed"),
                            "setting_key": $key,
                        }),
                    );
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
                let key_str = $key.to_string();
                let after_view = GlobalSettingView { key: key_str.clone(), value: new_value };
                let audit_entry = match $before {
                    Some(bv) => {
                        let before_view = GlobalSettingView { key: key_str, value: bv };
                        AuditEntry::<Stateful>::global_setting_update(&before_view, &after_view)
                    }
                    None => AuditEntry::<Stateful>::global_setting_update(
                        &AbsentView(&after_view),
                        &after_view,
                    ),
                }
                .system_scope()
                .actor(actor_type, actor_id)
                .outcome(AuditOutcome::Success)
                .details(serde_json::json!({
                    "setting_area": "oauth",
                    "changed_keys": [$key],
                }))
                .build();

                match audit_entry {
                    Ok(entry) => {
                        if let Err(e) =
                            state.audit_emitter.emit_stateful(&tx, &hook, entry).await
                        {
                            tracing::error!("Failed to emit stateful audit for {}: {e}", $key);
                            drop(tx);
                            return error_response(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                "Internal server error",
                            );
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to build audit entry for {}: {e}", $key);
                        drop(tx);
                        return error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "Internal server error",
                        );
                    }
                }
            }
        };
    }

    write_bool_setting!(
        req.mcp_enabled,
        "oauth.mcp_enabled",
        before_mcp,
        "mcp_enabled"
    );
    write_bool_setting!(
        req.dcr_enabled,
        "oauth.dcr_enabled",
        before_dcr,
        "dcr_enabled"
    );
    write_bool_setting!(
        req.cimd_enabled,
        "oauth.cimd_enabled",
        before_cimd,
        "cimd_enabled"
    );

    // canonical_host — string or null (clear)
    if let Some(ref raw) = req.canonical_host {
        let trimmed = raw.trim().to_string();
        let (db_value, _new_host) = if trimmed.is_empty() {
            (serde_json::Value::Null, None::<String>)
        } else {
            (serde_json::json!(trimmed.clone()), Some(trimmed))
        };
        if let Err(e) =
            upsert_global_setting_raw(&tx, "oauth.canonical_host", db_value.clone()).await
        {
            tracing::error!("Failed to save oauth.canonical_host: {e:?}");
            drop(tx);
            emit_oauth_failed_event(
                &state,
                actor_type,
                actor_id,
                serde_json::json!({
                    "setting_area": "oauth",
                    "reason_code": "oauth_canonical_host_upsert_failed",
                    "setting_key": "oauth.canonical_host",
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        let key_str = "oauth.canonical_host".to_string();
        let after_view = GlobalSettingView {
            key: key_str.clone(),
            value: db_value,
        };
        let audit_entry = match before_canonical {
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
            "setting_area": "oauth",
            "changed_keys": ["oauth.canonical_host"],
        }))
        .build();

        match audit_entry {
            Ok(entry) => {
                if let Err(e) = state.audit_emitter.emit_stateful(&tx, &hook, entry).await {
                    tracing::error!("Failed to emit stateful audit for oauth.canonical_host: {e}");
                    drop(tx);
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            }
            Err(e) => {
                tracing::error!("Failed to build audit entry for oauth.canonical_host: {e}");
                drop(tx);
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        }
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit oauth settings update: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    // Read fresh from DB for the response.
    let db_state = load_oauth_settings_from_db(&state).await;
    (StatusCode::OK, Json(db_state.into_response(&state))).into_response()
}
