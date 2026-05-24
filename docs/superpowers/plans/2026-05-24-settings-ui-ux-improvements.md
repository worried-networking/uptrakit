# Settings UI/UX Improvements Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Merge Registration + Authentication into one `AccessSettings` component backed by a unified `PUT /api/v1/settings/access` endpoint; add a
dedicated "MCP Access" tab lifting the OAuth Clients page; standardise all General-tab forms with `createFormDraft`; add `RadioCardGroup` component;
fix label width.

**Architecture:** Backend collapses two separate settings endpoints into one transactional handler. Frontend introduces a reusable `createFormDraft`
factory (Svelte 5 runes, `.svelte.ts`) used by all editable forms. Old routes (`/settings/registration`, `/settings/authentication`) and the
`authentication/oauth-clients/` page route are deleted with no backwards-compatibility shim.

**Tech Stack:** Rust (Axum, SeaORM, Tokio), Svelte 5 + TypeScript, Tailwind v4, Vitest + @testing-library/svelte.

---

## File Map

### Created

| File                                                         | Purpose                                                            |
| ------------------------------------------------------------ | ------------------------------------------------------------------ |
| `crates/shared/web-api-types/src/settings_access.rs`         | New `AccessSettingsResponse` + `UpdateAccessSettingsRequest` types |
| `crates/ui/web-api/src/routes/settings_access.rs`            | `GET /PUT /api/v1/settings/access` handlers                        |
| `crates/ui/web-api/src/integration_tests/settings_access.rs` | Integration tests for the new endpoint                             |
| `crates/ui/cli/src/commands/settings/access.rs`              | `settings access show/update` CLI subcommand                       |
| `frontend/src/lib/api/settings.ts`                           | ETag-aware `getAccessSettings` + `updateAccessSettings`            |
| `frontend/src/lib/forms/draft.svelte.ts`                     | `createFormDraft<T>` factory                                       |
| `frontend/src/lib/forms/index.ts`                            | Barrel re-export                                                   |
| `frontend/src/lib/components/forms/RadioCardGroup.svelte`    | Horizontal card-tile radio selector                                |
| `frontend/src/lib/components/forms/RadioCardGroup.test.ts`   | Unit tests for RadioCardGroup                                      |
| `frontend/src/routes/settings/AccessSettings.svelte`         | Merged Registration & Authentication form                          |
| `frontend/src/routes/settings/AccessSettings.test.ts`        | Unit tests for AccessSettings                                      |
| `frontend/src/routes/settings/McpAccessTab.svelte`           | Lifted OAuth Clients tab content                                   |
| `frontend/src/routes/settings/RegisterClientDialog.svelte`   | Moved from `authentication/oauth-clients/`                         |

### Modified

| File                                                           | Change                                                                                                      |
| -------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| `crates/shared/web-api-types/src/lib.rs`                       | Add `pub mod settings_access;`                                                                              |
| `crates/shared/web-api-types/src/settings_combined.rs`         | Remove `registration` + `authentication` fields from `CombinedSettingsResponse`                             |
| `crates/ui/web-api/src/routes/mod.rs`                          | Add `pub mod settings_access;`, remove `settings` + `settings_auth`                                         |
| `crates/ui/web-api/src/router.rs`                              | Wire new route, remove old routes from OpenAPI paths list                                                   |
| `crates/ui/web-api/src/routes/settings_combined.rs`            | Remove registration + authentication source queries                                                         |
| `crates/ui/web-api/src/integration_tests/mod.rs`               | Add `mod settings_access;`                                                                                  |
| `crates/ui/web-api/src/integration_tests/settings.rs`          | Remove deleted registration/auth tests; add access redirect tests                                           |
| `crates/shared/openapi-client/src/lib.rs`                      | Add `get_with_etag` + `put_json_with_etag` private helpers                                                  |
| `crates/shared/openapi-client/src/paths.rs`                    | Add `ACCESS`, remove `REGISTRATION` + `AUTHENTICATION`                                                      |
| `crates/shared/openapi-client/src/settings.rs`                 | Add `get_access_settings` + `update_access_settings` (ETag-aware), remove old                               |
| `crates/ui/cli/src/commands/settings/mod.rs`                   | Add `Access` variant, update `HumanOutput` for `CombinedSettingsResponse`                                   |
| `frontend/src/lib/types.ts`                                    | Add `AccessSettingsData` + `UpdateAccessSettingsRequest`; remove old types; trim `CombinedSettingsResponse` |
| `frontend/src/lib/api.ts`                                      | Remove old registration/authentication functions + imports; update `getCombinedSettings` return type        |
| `frontend/src/lib/components/forms/FormFieldRow.svelte`        | Add `dirty` prop + `w-fit` on label                                                                         |
| `frontend/src/lib/components/forms/index.ts`                   | Export `RadioCardGroup`                                                                                     |
| `frontend/src/routes/settings/AgentCertificateSettings.svelte` | Add `createFormDraft` draft pattern                                                                         |
| `frontend/src/routes/settings/+page.svelte`                    | Wire `AccessSettings` + `McpAccessTab`, remove old components + OAuth block                                 |
| `frontend/src/routes/settings/surface-tabs.test.ts`            | Remove `registration` + `authentication` from mock                                                          |

### Deleted

| File                                                          | Reason                                                    |
| ------------------------------------------------------------- | --------------------------------------------------------- |
| `crates/ui/web-api/src/routes/settings.rs`                    | Old registration handlers                                 |
| `crates/ui/web-api/src/routes/settings_auth.rs`               | Old authentication handlers                               |
| `crates/shared/web-api-types/src/settings.rs`                 | Types moved to `settings_access.rs`                       |
| `crates/shared/web-api-types/src/settings_auth.rs`            | Types moved to `settings_access.rs`                       |
| `crates/ui/cli/src/commands/settings/registration.rs`         | Replaced by `access.rs`                                   |
| `crates/ui/cli/src/commands/settings/authentication.rs`       | Replaced by `access.rs`                                   |
| `frontend/src/routes/settings/RegistrationSettings.svelte`    | Replaced by `AccessSettings.svelte`                       |
| `frontend/src/routes/settings/RegistrationSettings.test.ts`   | Replaced by `AccessSettings.test.ts`                      |
| `frontend/src/routes/settings/AuthenticationSettings.svelte`  | Replaced by `AccessSettings.svelte`                       |
| `frontend/src/routes/settings/AuthenticationSettings.test.ts` | Replaced by `AccessSettings.test.ts`                      |
| `frontend/src/routes/settings/authentication/`                | Entire directory — content moved to `McpAccessTab.svelte` |

---

### Task 1: Backend types

**Files:**

- Create: `crates/shared/web-api-types/src/settings_access.rs`
- Modify: `crates/shared/web-api-types/src/lib.rs`
- Modify: `crates/shared/web-api-types/src/settings_combined.rs`

- [ ] **Step 1: Create `settings_access.rs`**

```rust
// crates/shared/web-api-types/src/settings_access.rs

use serde::{Deserialize, Serialize};

use crate::registration::RegistrationMode;
use crate::validation::{Validate, ValidationError};
use uptrakit_shared_types::SecretString;

// No #[non_exhaustive] — UpdateAccessSettingsRequest is constructed in external crates
// (openapi-client, CLI, tests) via struct literals; #[non_exhaustive] would break that.
// AccessSettingsResponse carries #[non_exhaustive] because it is never constructed externally.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateAccessSettingsRequest {
    pub mode: RegistrationMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<SecretString>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub require_token_for_oidc: Option<bool>,
    pub password_auth_enabled: Option<bool>,
    pub two_factor_required: Option<bool>,
}

impl Validate for UpdateAccessSettingsRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        match self.mode {
            RegistrationMode::Invite => {
                if self.token.is_none() {
                    return Err(ValidationError {
                        field: "token",
                        message: "required when mode is invite".to_string(),
                    });
                }
            }
            _ => {
                if self.token.is_some() {
                    return Err(ValidationError {
                        field: "token",
                        message: "only valid when mode is invite".to_string(),
                    });
                }
                if self.require_token_for_oidc.is_some() {
                    return Err(ValidationError {
                        field: "require_token_for_oidc",
                        message: "only valid when mode is invite".to_string(),
                    });
                }
            }
        }
        Ok(())
    }
}

#[non_exhaustive]
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AccessSettingsResponse {
    pub mode: RegistrationMode,
    pub require_token_for_oidc: bool,
    pub password_auth_enabled: bool,
    pub two_factor_required: bool,
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — is_ok/is_err provides readable failure messages"
    )]
    use super::*;

    #[test]
    fn validate_invite_requires_token() {
        let req = UpdateAccessSettingsRequest {
            mode: RegistrationMode::Invite,
            token: None,
            require_token_for_oidc: None,
            password_auth_enabled: None,
            two_factor_required: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_invite_with_token_ok() {
        let req = UpdateAccessSettingsRequest {
            mode: RegistrationMode::Invite,
            token: Some(SecretString::new("abc".to_string())),
            require_token_for_oidc: None,
            password_auth_enabled: None,
            two_factor_required: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_open_with_token_rejected() {
        let req = UpdateAccessSettingsRequest {
            mode: RegistrationMode::Open,
            token: Some(SecretString::new("abc".to_string())),
            require_token_for_oidc: None,
            password_auth_enabled: None,
            two_factor_required: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_closed_no_token_ok() {
        let req = UpdateAccessSettingsRequest {
            mode: RegistrationMode::Closed,
            token: None,
            require_token_for_oidc: None,
            password_auth_enabled: Some(true),
            two_factor_required: None,
        };
        assert!(req.validate().is_ok());
    }

}
```

- [ ] **Step 2: Register `settings_access` in `lib.rs`**

In `crates/shared/web-api-types/src/lib.rs`, add after the `settings_auth` line (line 46):

```rust
pub mod settings_access;
```

- [ ] **Step 3: Write failing unit test first (already in settings_access.rs above)**

Run:

```bash
cargo test -p uptrakit-web-api-types --all-features 2>&1 | tail -20
```

Expected: PASS for the new tests (they live in the new file, nothing imports the deleted types yet).

- [ ] **Step 4: Update `settings_combined.rs` — strip registration + authentication fields**

Replace the entire file content:

```rust
// crates/shared/web-api-types/src/settings_combined.rs

use serde::{Deserialize, Serialize};

use crate::enrollment_tokens::EnrollmentTokensSummary;
use crate::settings_agent_certs::AgentCertificateSettingsResponse;
use crate::settings_nats::NatsSettingsResponse;
use crate::settings_network::NetworkSettingsResponse;

#[non_exhaustive]
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CombinedSettingsResponse {
    pub agent_certificates: AgentCertificateSettingsResponse,
    pub enrollment_tokens: EnrollmentTokensSummary,
    pub multi_tenancy_enabled: bool,
}

/// Combined response for all global (infrastructure-scoped) settings.
#[non_exhaustive]
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct GlobalSettingsCombinedResponse {
    pub network: NetworkSettingsResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nats: Option<NatsSettingsResponse>,
}

#[cfg(test)]
mod tests {
    use super::*;

}
```

- [ ] **Step 5: Run tests**

```bash
cargo test -p uptrakit-web-api-types --all-features 2>&1 | tail -20
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git commit --only \
        crates/shared/web-api-types/src/settings_access.rs \
        crates/shared/web-api-types/src/lib.rs \
        crates/shared/web-api-types/src/settings_combined.rs \
        -m "feat(types): add AccessSettings types; trim CombinedSettingsResponse"
```

---

### Task 2: Access settings handler

**Files:**

- Create: `crates/ui/web-api/src/routes/settings_access.rs`

- [ ] **Step 1: Write the failing integration test placeholder first**

Skip to Task 6 (integration tests) after this task if running TDD strictly. The handler will compile-fail without the route wired — so write the
handler file first, then wire it in Task 3, then add tests in Task 6.

- [ ] **Step 2: Create `settings_access.rs` handler**

```rust
// crates/ui/web-api/src/routes/settings_access.rs

use std::sync::Arc;

use axum::{
    Extension, Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{
    ColumnTrait, QueryFilter, SqliteTransactionMode, TransactionOptions, TransactionTrait,
};
use uptrakit_audit_log::{AbsentView, AuditEntry, AuditOutcome, Event, Stateful};
use uptrakit_web_api_queries::queries::tenant_settings::TenantSettingView;
use uptrakit_web_api_types::settings_access::{AccessSettingsResponse, UpdateAccessSettingsRequest};
use uptrakit_web_api_types::validation::Validate;

use crate::AppState;
use crate::auth::AuthMethod;
use crate::auth::registration::RegistrationMode;
use crate::error_response::error_response;
use crate::extractors::{IfMatch, SettingsVersion};
use crate::middleware::permission::{CanManageAuthSettings, CanViewSettings};
use crate::middleware::require_auth::{AuthenticatedApiTokenId, authenticated_user_audit_actor};
#[cfg(feature = "oidc")]
use {
    crate::tenant_db::TenantDb,
    uptrakit_shared_db::entity::oidc_provider,
};

fn current_response(state: &AppState) -> AccessSettingsResponse {
    let reg = state.settings.registration();
    let auth = state.settings.authentication();
    AccessSettingsResponse {
        mode: reg.mode,
        require_token_for_oidc: reg.require_token_for_oidc,
        password_auth_enabled: auth.password_auth_enabled,
        two_factor_required: auth.two_factor_required,
    }
}

/// Get access settings (registration + authentication)
#[utoipa::path(
    get,
    path = "/api/v1/settings/access",
    responses(
        (status = 200, description = "Current access settings", body = AccessSettingsResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    extensions(("x-required-permission" = json!("view_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_access_settings(
    State(state): State<Arc<AppState>>,
    CanViewSettings(_user): CanViewSettings,
) -> Response {
    let version = state
        .settings_version_cache
        .get(uptrakit_config_reload::config::Scope::Tenant(
            state.default_tenant_id,
        ))
        .unwrap_or(0);
    let etag = format!("W/\"settings-v{version}\"");
    (
        StatusCode::OK,
        [(axum::http::header::ETAG, etag)],
        Json(current_response(&state)),
    )
        .into_response()
}

/// Update access settings (registration + authentication in one transaction)
#[utoipa::path(
    put,
    path = "/api/v1/settings/access",
    request_body = UpdateAccessSettingsRequest,
    responses(
        (status = 200, description = "Access settings updated", body = AccessSettingsResponse),
        (status = 409, description = "Safety check failed (e.g., disabling password auth while using it)"),
        (status = 422, description = "Validation error (e.g., invite mode without token)")
    ),
    tag = "Settings",
    extensions(("x-required-permission" = json!("manage_auth_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_access_settings(
    State(state): State<Arc<AppState>>,
    CanManageAuthSettings(user): CanManageAuthSettings,
    _if_match: IfMatch<SettingsVersion>,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    #[cfg(feature = "oidc")] tenant_db: TenantDb,
    Json(req): Json<UpdateAccessSettingsRequest>,
) -> Response {
    // ── 1. Type-level validation ─────────────────────────────────────────────
    if let Err(e) = req.validate() {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("{}: {}", e.field, e.message),
        );
    }

    let api_token_id = api_token_id.map(|v| v.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = state.default_tenant_id;

    // ── 2. Safety checks (reads; run BEFORE opening BEGIN IMMEDIATE) ─────────
    if let Some(false) = req.password_auth_enabled {
        let previous_enabled = state.settings.authentication().password_auth_enabled;

        if user.auth_method == AuthMethod::Password {
            emit_auth_event(
                &state,
                actor_type,
                actor_id,
                AuditOutcome::Denied,
                "cannot_disable_password_auth_while_using_password",
                previous_enabled,
                false,
            );
            return error_response(
                StatusCode::CONFLICT,
                "Cannot disable password authentication while logged in with a password",
            );
        }

        #[cfg(feature = "oidc")]
        {
            let active_providers = match tenant_db
                .find::<oidc_provider::Entity>()
                .filter(oidc_provider::Column::IsActive.eq(true))
                .filter(oidc_provider::Column::DeactivatedAt.is_null())
                .all(tenant_db.db())
                .await
            {
                Ok(providers) => providers,
                Err(e) => {
                    tracing::error!("Failed to query OIDC providers: {e}");
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            };

            if active_providers.is_empty() {
                emit_auth_event(
                    &state,
                    actor_type,
                    actor_id,
                    AuditOutcome::Denied,
                    "cannot_disable_password_auth_without_active_oidc_providers",
                    previous_enabled,
                    false,
                );
                return error_response(
                    StatusCode::CONFLICT,
                    "Cannot disable password authentication with no active OIDC providers",
                );
            }
        }

        if !cfg!(feature = "oidc") {
            emit_auth_event(
                &state,
                actor_type,
                actor_id,
                AuditOutcome::Denied,
                "cannot_disable_password_auth_without_oidc_support",
                previous_enabled,
                false,
            );
            return error_response(
                StatusCode::CONFLICT,
                "Cannot disable password authentication: OIDC support is not enabled",
            );
        }
    }

    // ── 3. Capture before-state for audit ────────────────────────────────────
    let before_reg = state.settings.registration();
    let before_auth = state.settings.authentication();

    let before_reg_view = TenantSettingView {
        key: "registration".to_string(),
        value: serde_json::json!({
            "mode": before_reg.mode.as_str(),
            "require_token_for_oidc": before_reg.require_token_for_oidc,
        }),
    };
    let before_auth_view = TenantSettingView {
        key: "authentication".to_string(),
        value: serde_json::json!({
            "password_auth_enabled": before_auth.password_auth_enabled,
            "two_factor_required": before_auth.two_factor_required,
        }),
    };

    let had_existing_reg = before_reg.token_hash.is_some()
        || before_reg.mode != RegistrationMode::Closed
        || before_reg.require_token_for_oidc;

    // ── 4. BEGIN IMMEDIATE transaction — write both settings atomically ───────
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
            tracing::error!("Failed to begin tx for access settings update: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };
    let hook = state.audit_emitter.commit_hook();

    // Write registration settings
    let mut reg = state.settings.registration();
    if let Err(e) = reg
        .update(
            &tx,
            tenant_id,
            req.mode,
            req.token.as_ref().map(|t| t.expose_secret().to_string()),
            req.require_token_for_oidc,
        )
        .await
    {
        tracing::error!(error = ?e, "Failed to update registration settings");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    // Write authentication settings
    let mut auth = state.settings.authentication();
    if let Some(enabled) = req.password_auth_enabled {
        auth.password_auth_enabled = enabled;
    }
    if let Some(required) = req.two_factor_required {
        auth.two_factor_required = required;
    }
    if let Err(e) = auth.save(&tx, tenant_id).await {
        tracing::error!("Failed to save authentication settings: {e:?}");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    // ── 5. Build after-state views ────────────────────────────────────────────
    let after_reg_view = TenantSettingView {
        key: "registration".to_string(),
        value: serde_json::json!({
            "mode": reg.mode.as_str(),
            "require_token_for_oidc": reg.require_token_for_oidc,
        }),
    };
    let after_auth_view = TenantSettingView {
        key: "authentication".to_string(),
        value: serde_json::json!({
            "password_auth_enabled": auth.password_auth_enabled,
            "two_factor_required": auth.two_factor_required,
        }),
    };

    // ── 6. Emit stateful audit events ─────────────────────────────────────────
    let reg_entry_result = if had_existing_reg {
        AuditEntry::<Stateful>::tenant_setting_update(&before_reg_view, &after_reg_view)
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::Success)
            .details(serde_json::json!({ "setting_area": "registration" }))
            .build()
    } else {
        AuditEntry::<Stateful>::tenant_setting_update(&AbsentView(&after_reg_view), &after_reg_view)
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::Success)
            .details(serde_json::json!({ "setting_area": "registration" }))
            .build()
    };
    let reg_entry = match reg_entry_result {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("Failed to build registration audit entry: {e}");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(e) = state.audit_emitter.emit_stateful(&tx, &hook, reg_entry).await {
        tracing::error!("Failed to emit registration audit: {e}");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    // Only emit auth audit event when the request actually touched auth fields.
    // Emitting with identical before/after produces spurious "changed" audit entries.
    let auth_fields_touched =
        req.password_auth_enabled.is_some() || req.two_factor_required.is_some();
    if auth_fields_touched {
        let auth_entry = match AuditEntry::<Stateful>::tenant_setting_update(
            &before_auth_view,
            &after_auth_view,
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::Success)
        .details(serde_json::json!({ "setting_area": "authentication" }))
        .build()
        {
            Ok(e) => e,
            Err(e) => {
                tracing::error!("Failed to build authentication audit entry: {e}");
                drop(tx);
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };
        if let Err(e) = state.audit_emitter.emit_stateful(&tx, &hook, auth_entry).await {
            tracing::error!("Failed to emit authentication audit: {e}");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    }

    // ── 7. Commit ─────────────────────────────────────────────────────────────
    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit access settings update: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    // ── 8. Bump settings version cache ────────────────────────────────────────
    let scope = uptrakit_config_reload::config::Scope::Tenant(tenant_id);
    let next = state
        .settings_version_cache
        .get(scope)
        .unwrap_or(0)
        .saturating_add(1);
    state.settings_version_cache.update(scope, next);

    state.settings.set_registration(reg).await;
    state.settings.set_authentication(auth).await;

    // Return updated ETag so clients can chain subsequent PUTs without a GET.
    let new_etag = format!("W/\"settings-v{next}\"");
    (
        StatusCode::OK,
        [(axum::http::header::ETAG, new_etag)],
        Json(current_response(&state)),
    )
        .into_response()
}

// ── Audit helpers ─────────────────────────────────────────────────────────────

fn emit_auth_event(
    state: &AppState,
    actor_type: uptrakit_audit_log::AuditActorType,
    actor_id: Option<uuid::Uuid>,
    outcome: AuditOutcome,
    reason_code: &'static str,
    previous_enabled: bool,
    new_enabled: bool,
) {
    if let Ok(entry) = AuditEntry::<Event>::builder_event(
        uptrakit_audit_log::AuditActionType::TENANT_SETTING_UPDATE,
    )
    .tenant_scope(state.default_tenant_id)
    .actor(actor_type, actor_id)
    .target(
        "tenant_setting",
        "authentication".to_string(),
        Some("authentication".to_string()),
    )
    .outcome(outcome)
    .details(serde_json::json!({
        "reason_code": reason_code,
        "previous_enabled": previous_enabled,
        "new_enabled": new_enabled,
    }))
    .build()
    {
        state.audit_emitter.emit_event(entry);
    }
}
```

- [ ] **Step 3: Check compilation (before wiring)**

```bash
cargo check -p uptrakit-web-api --all-features 2>&1 | grep "error\[" | head -20
```

Expected: errors about missing imports or `settings_access` module not found — this is normal. Proceed to Task 3 to wire it.

---

### Task 3: Router + combined handler cleanup

**Files:**

- Modify: `crates/ui/web-api/src/routes/mod.rs`
- Modify: `crates/ui/web-api/src/router.rs`
- Modify: `crates/ui/web-api/src/routes/settings_combined.rs`
- Delete: `crates/ui/web-api/src/routes/settings.rs`
- Delete: `crates/ui/web-api/src/routes/settings_auth.rs`

- [ ] **Step 1: Add + remove module declarations in `routes/mod.rs`**

In `crates/ui/web-api/src/routes/mod.rs`:

Remove these two lines:

```rust
pub mod settings;
pub mod settings_auth;
```

Add in their place (keep alphabetical order):

```rust
pub mod settings_access;
```

- [ ] **Step 2: Update `router.rs` OpenAPI paths list**

In the `paths(...)` block inside `#[derive(OpenApi)]` in `crates/ui/web-api/src/router.rs`:

Replace:

```rust
        crate::routes::settings::get_registration_settings,
        crate::routes::settings::update_registration_settings,
        crate::routes::settings_combined::get_combined_settings,
        crate::routes::settings_auth::get_authentication_settings,
        crate::routes::settings_auth::update_authentication_settings,
```

With:

```rust
        crate::routes::settings_access::get_access_settings,
        crate::routes::settings_access::update_access_settings,
        crate::routes::settings_combined::get_combined_settings,
```

- [ ] **Step 3: Update `router.rs` route registration in `build_router`**

In `build_router`, replace:

```rust
        .routes(routes!(
            crate::routes::settings::get_registration_settings,
            crate::routes::settings::update_registration_settings
        ))
        .routes(routes!(
            crate::routes::settings_combined::get_combined_settings
        ))
        .routes(routes!(
            crate::routes::settings_auth::get_authentication_settings,
            crate::routes::settings_auth::update_authentication_settings
        ))
```

With:

```rust
        .routes(routes!(
            crate::routes::settings_access::get_access_settings,
            crate::routes::settings_access::update_access_settings
        ))
        .routes(routes!(
            crate::routes::settings_combined::get_combined_settings
        ))
```

- [ ] **Step 4: Update `settings_combined.rs` — remove registration + authentication**

Replace the entire file:

```rust
// crates/ui/web-api/src/routes/settings_combined.rs

use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::AppState;
use crate::error_response::error_response;
use crate::middleware::permission::CanViewSettings;
use crate::queries::enrollment_tokens as et_queries;
use uptrakit_web_api_types::enrollment_tokens::EnrollmentTokensSummary;
use uptrakit_web_api_types::settings_agent_certs::AgentCertificateSettingsResponse;
use uptrakit_web_api_types::settings_combined::CombinedSettingsResponse;

/// Get core settings for the settings page (excludes access settings, which self-load).
#[utoipa::path(
    get,
    path = "/api/v1/settings",
    responses(
        (status = 200, description = "Combined core settings", body = CombinedSettingsResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    extensions(("x-required-permission" = json!("view_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_combined_settings(
    State(state): State<Arc<AppState>>,
    CanViewSettings(_user): CanViewSettings,
) -> Response {
    let agent_certificates = AgentCertificateSettingsResponse {
        lifetime_hours: state.settings.agent_cert_lifetime_hours(),
        renewal_window_hours_override: state.settings.renewal_window_hours_override(),
        effective_renewal_window_hours: state.settings.renewal_window_hours(),
    };

    let active_count =
        match et_queries::count_active_tokens(state.db(), state.default_tenant_id).await {
            Ok(count) => count,
            Err(e) => {
                tracing::error!("Failed to count active enrollment tokens: {}", e);
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };
    let multi_tenancy_enabled =
        match crate::settings_store::is_multi_tenancy_enabled(state.db()).await {
            Ok(enabled) => enabled,
            Err(e) => {
                tracing::error!("Failed to load multi-tenancy mode: {}", e);
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    let response = CombinedSettingsResponse {
        agent_certificates,
        enrollment_tokens: EnrollmentTokensSummary { active_count },
        multi_tenancy_enabled,
    };

    (StatusCode::OK, Json(response)).into_response()
}
```

- [ ] **Step 5: Delete old handler files**

```bash
rm crates/ui/web-api/src/routes/settings.rs
rm crates/ui/web-api/src/routes/settings_auth.rs
```

- [ ] **Step 6: Compile check**

```bash
cargo check -p uptrakit-web-api --all-features 2>&1 | grep "error\[" | head -30
```

Expected: no errors from the deleted modules. Fix any remaining compile errors (e.g., `CombinedSettingsResponse` field references in other files).

- [ ] **Step 7: Run backend tests**

```bash
cargo test -p uptrakit-web-api --no-default-features --features db-sqlite 2>&1 | tail -30
```

Expected: PASS. The old settings + settings_auth tests no longer exist; no regressions.

- [ ] **Step 8: Commit**

```bash
git commit --only \
        crates/ui/web-api/src/routes/settings_access.rs \
        crates/ui/web-api/src/routes/mod.rs \
        crates/ui/web-api/src/routes/settings_combined.rs \
        crates/ui/web-api/src/router.rs \
        -m "feat(api): add GET/PUT /api/v1/settings/access; remove registration+auth endpoints"
```

---

### Task 4: OpenAPI client + CLI

**Files:**

- Modify: `crates/shared/openapi-client/src/paths.rs`
- Modify: `crates/shared/openapi-client/src/settings.rs`
- Create: `crates/ui/cli/src/commands/settings/access.rs`
- Modify: `crates/ui/cli/src/commands/settings/mod.rs`
- Delete: `crates/ui/cli/src/commands/settings/registration.rs`
- Delete: `crates/ui/cli/src/commands/settings/authentication.rs`

- [ ] **Step 1: Add `get_with_etag` + `put_json_with_etag` helpers to `crates/shared/openapi-client/src/lib.rs`**

The `IfMatch` extractor requires `If-Match` on every PUT — without it the server returns 428. Add two private helper methods alongside the existing
`put_json`:

```rust
    async fn get_with_etag<T: DeserializeOwned>(&self, path: &str) -> Result<(T, String)> {
        let url = format!("{}{}", self.base_url, path);
        let req = self.http.get(&url).bearer_auth(self.token_or_err()?);
        let resp = self.send_with_retry(req).await?;
        let etag = resp
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body: T = self.handle_response(resp).await?;
        Ok((body, etag))
    }

    async fn put_json_with_etag<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &impl Serialize,
        etag: &str,
    ) -> Result<(T, String)> {
        let url = format!("{}{}", self.base_url, path);
        let req = self
            .http
            .put(&url)
            .bearer_auth(self.token_or_err()?)
            .header(reqwest::header::IF_MATCH, etag)
            .json(body);
        let resp = self.send_with_retry(req).await?;
        let new_etag = resp
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body: T = self.handle_response(resp).await?;
        Ok((body, new_etag))
    }
```

Place these immediately after the `put_json` method (around line 476 in `lib.rs`).

- [ ] **Step 2: Update `paths.rs`**

In `crates/shared/openapi-client/src/paths.rs`, inside `pub(crate) mod settings { ... }`:

Remove:

```rust
    /// `GET /api/v1/settings/registration` · `PUT …`
    pub(crate) const REGISTRATION: &str = "/api/v1/settings/registration";
    /// `GET /api/v1/settings/authentication` · `PUT …`
    pub(crate) const AUTHENTICATION: &str = "/api/v1/settings/authentication";
```

Add:

```rust
    /// `GET /api/v1/settings/access` · `PUT …`
    pub(crate) const ACCESS: &str = "/api/v1/settings/access";
```

- [ ] **Step 2: Update `settings.rs` in openapi-client**

In `crates/shared/openapi-client/src/settings.rs`:

Remove the imports for deleted types:

```rust
use crate::types_impl::settings::{
    RegistrationSettingsResponse, UpdateRegistrationSettingsRequest,
};
use crate::types_impl::settings_auth::{
    AuthenticationSettingsResponse, UpdateAuthenticationSettingsRequest,
};
```

Add:

```rust
use crate::types_impl::settings_access::{AccessSettingsResponse, UpdateAccessSettingsRequest};
```

Remove the four old methods:

- `get_registration_settings`
- `update_registration_settings`
- `get_authentication_settings`
- `update_authentication_settings`

Add:

```rust
    /// Get access settings (registration + authentication combined).
    /// Returns the response body and the raw `ETag` header value.
    pub async fn get_access_settings(&self) -> Result<(AccessSettingsResponse, String)> {
        self.get_with_etag(crate::paths::settings::ACCESS).await
    }

    /// Update access settings.  `etag` must be the value returned by a prior
    /// `get_access_settings` call — the server requires `If-Match` for optimistic locking.
    pub async fn update_access_settings(
        &self,
        req: &UpdateAccessSettingsRequest,
        etag: &str,
    ) -> Result<(AccessSettingsResponse, String)> {
        self.put_json_with_etag(crate::paths::settings::ACCESS, req, etag)
            .await
    }
```

Also update the `CombinedSettingsResponse` import — the type no longer has `registration` or `authentication` fields, so any reference to those in the
client crate needs updating. Check `settings.rs` does not reference those fields directly.

- [ ] **Step 3: Create `access.rs` CLI command**

```rust
// crates/ui/cli/src/commands/settings/access.rs

use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::HumanOutput;
use clap::Subcommand;
use rootcause::prelude::*;
use uptrakit_openapi_client::types::registration::RegistrationMode;
use uptrakit_openapi_client::types::settings_access::{
    AccessSettingsResponse, UpdateAccessSettingsRequest,
};
use uptrakit_openapi_client::types::SecretString;

#[derive(Debug, Subcommand)]
pub enum AccessCommands {
    /// Show registration and authentication settings
    Show,
    /// Update access settings
    Update {
        /// Registration mode (open, invite, closed) — required because the backend
        /// PUT replaces the full registration+auth state; use `show` first to read
        /// current mode before passing it here.
        #[arg(long, value_parser = crate::commands::parse_registration_mode, required = true)]
        mode: RegistrationMode,
        /// Registration token (required when mode is invite)
        #[arg(long)]
        token: Option<String>,
        /// Require registration token for OIDC users (only valid with invite mode)
        #[arg(long)]
        require_token_for_oidc: Option<bool>,
        /// Enable or disable password authentication
        #[arg(long)]
        password_auth_enabled: Option<bool>,
        /// Require two-factor authentication for all users
        #[arg(long)]
        two_factor_required: Option<bool>,
    },
}

// ── Human output ─────────────────────────────────────────────────────────────

impl HumanOutput for AccessSettingsResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::from("Registration:\n");
        out.push_str(&format!(
            "  Mode:                    {}\n",
            self.mode.as_str()
        ));
        out.push_str(&format!(
            "  Require Token for OIDC:  {}\n",
            self.require_token_for_oidc
        ));
        out.push_str("\nAuthentication:\n");
        out.push_str(&format!(
            "  Password Auth Enabled:   {}\n",
            self.password_auth_enabled
        ));
        out.push_str(&format!(
            "  Two-Factor Required:     {}\n",
            self.two_factor_required
        ));
        out
    }
}

// ── Commands ─────────────────────────────────────────────────────────────────

pub async fn access_show(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<AccessSettingsResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let (resp, _etag) = client.get_access_settings().await.context_to()?;
    Ok(resp)
}

pub async fn access_update(
    server: Option<&str>,
    auth_token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
    mode: RegistrationMode,
    reg_token: Option<String>,
    require_token_for_oidc: Option<bool>,
    password_auth_enabled: Option<bool>,
    two_factor_required: Option<bool>,
) -> Result<AccessSettingsResponse> {
    let client = authenticated_client(server, auth_token, insecure, request_timeout)?;
    // GET first to obtain current ETag — server requires If-Match for optimistic locking.
    let (_current, etag) = client.get_access_settings().await.context_to()?;
    let req = UpdateAccessSettingsRequest {
        mode,
        token: reg_token.map(SecretString::new),
        require_token_for_oidc,
        password_auth_enabled,
        two_factor_required,
    };
    let (resp, _new_etag) = client
        .update_access_settings(&req, &etag)
        .await
        .context_to()?;
    Ok(resp)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_settings_human_output() {
        // Use serde deserialization — AccessSettingsResponse is #[non_exhaustive] and
        // cannot be constructed via struct literal in external crates.
        let resp: AccessSettingsResponse = serde_json::from_value(serde_json::json!({
            "mode": "invite",
            "require_token_for_oidc": true,
            "password_auth_enabled": false,
            "two_factor_required": true
        }))
        .expect("fixture should deserialize");
        let s = resp.to_human_string();
        assert!(s.contains("invite"), "mode missing");
        assert!(s.contains("true"), "require_token_for_oidc missing");
        assert!(s.contains("false"), "password_auth_enabled missing");
    }
}
```

- [ ] **Step 4: Update `settings/mod.rs`**

In `crates/ui/cli/src/commands/settings/mod.rs`:

**Remove:**

```rust
pub mod authentication;
pub mod registration;
// ...
pub use authentication::AuthenticationCommands;
pub use registration::RegistrationCommands;
// ...
use self::authentication::{authentication_show, authentication_update};
use self::registration::{RegistrationUpdateParams, registration_show, registration_update};
```

**Add:**

```rust
pub mod access;
pub use access::AccessCommands;
use self::access::{access_show, access_update};
```

Replace the `Registration` and `Authentication` variants in `SettingsCommands`:

```rust
// Remove:
    Registration {
        #[command(subcommand)]
        command: RegistrationCommands,
    },
    Authentication {
        #[command(subcommand)]
        command: AuthenticationCommands,
    },

// Add:
    /// Registration and authentication settings
    Access {
        #[command(subcommand)]
        command: AccessCommands,
    },
```

Update `HumanOutput for CombinedSettingsResponse` — remove references to `self.registration` and `self.authentication` (these fields no longer exist):

```rust
impl HumanOutput for CombinedSettingsResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str("Agent Certificates:\n");
        out.push_str(&format!(
            "  Lifetime (hours):        {}\n",
            self.agent_certificates.lifetime_hours
        ));
        let window_desc = match self.agent_certificates.renewal_window_hours_override {
            None => format!(
                "automatic ({} hours)",
                self.agent_certificates.effective_renewal_window_hours
            ),
            Some(h) => format!("{h} hours (custom override)"),
        };
        out.push_str(&format!("  Renewal Window:          {window_desc}\n"));
        out.push_str("\nEnrollment Tokens:\n");
        out.push_str(&format!(
            "  Active:                  {}\n",
            self.enrollment_tokens.active_count
        ));
        out
    }
}
```

Update the `dispatch` function — replace the `Registration` and `Authentication` match arms with an `Access` arm:

```rust
        SettingsCommands::Access { command } => match command {
            AccessCommands::Show => {
                let resp = access_show(
                    ctx.server.as_deref(),
                    ctx.token.as_deref(),
                    ctx.insecure,
                    ctx.request_timeout,
                )
                .await?;
                crate::output::print_output(ctx.format, &resp)?;
            }
            AccessCommands::Update {
                mode,
                token,
                require_token_for_oidc,
                password_auth_enabled,
                two_factor_required,
            } => {
                // mode is required by Clap (required = true); no unwrap_or needed
                let resp = access_update(
                    ctx.server.as_deref(),
                    ctx.token.as_deref(),
                    ctx.insecure,
                    ctx.request_timeout,
                    mode,
                    token,
                    require_token_for_oidc,
                    password_auth_enabled,
                    two_factor_required,
                )
                .await?;
                crate::output::print_output(ctx.format, &resp)?;
            }
        },
```

Also update the `combined_settings_human_output` test in `mod.rs` — remove references to `registration` and `authentication` fields when constructing
`CombinedSettingsResponse`:

```rust
    #[test]
    fn combined_settings_human_output() {
        let resp = CombinedSettingsResponse {
            agent_certificates: uptrakit_openapi_client::types::settings_agent_certs::AgentCertificateSettingsResponse {
                lifetime_hours: 8760,
                renewal_window_hours_override: None,
                effective_renewal_window_hours: 336,
            },
            enrollment_tokens: uptrakit_openapi_client::types::enrollment_tokens::EnrollmentTokensSummary { active_count: 3 },
            multi_tenancy_enabled: false,
        };
        let s = resp.to_human_string();
        assert!(s.contains("8760"), "lifetime_hours missing");
        assert!(s.contains("3"), "enrollment_tokens missing");
    }
```

- [ ] **Step 5: Delete old CLI files**

```bash
rm crates/ui/cli/src/commands/settings/registration.rs
rm crates/ui/cli/src/commands/settings/authentication.rs
```

- [ ] **Step 6: Compile and test**

```bash
cargo check --all-features 2>&1 | grep "error\[" | head -20
cargo test -p uptrakit-cli --all-features 2>&1 | tail -20
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git commit --only \
        crates/shared/openapi-client/src/lib.rs \
        crates/shared/openapi-client/src/paths.rs \
        crates/shared/openapi-client/src/settings.rs \
        crates/ui/cli/src/commands/settings/access.rs \
        crates/ui/cli/src/commands/settings/mod.rs \
        -m "feat(cli): add settings access subcommand with ETag-aware GET-then-PUT; remove registration+authentication"
```

---

### Task 5: Backend integration tests

**Files:**

- Create: `crates/ui/web-api/src/integration_tests/settings_access.rs`
- Modify: `crates/ui/web-api/src/integration_tests/mod.rs`
- Modify: `crates/ui/web-api/src/integration_tests/settings.rs`

- [ ] **Step 1: Write failing tests first**

Create `crates/ui/web-api/src/integration_tests/settings_access.rs`:

```rust
#![expect(
    clippy::let_underscore_must_use,
    reason = "fire-and-forget channel sends in tests drop results intentionally"
)]

use crate::test_harness::TestApp;
use crate::test_harness::fixtures::register_and_get_token;

fn ensure_crypto_provider() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let _ = jsonwebtoken::crypto::aws_lc::DEFAULT_PROVIDER.install_default();
}

/// Read the current ETag for `/api/v1/settings/access` so tests don't hard-code
/// the version number (fragile if `TestApp::new()` ever bumps it during init).
async fn current_access_etag(
    client: &crate::test_harness::TestClient,
    token: &str,
) -> String {
    let res = client
        .get("/api/v1/settings/access")
        .bearer(token)
        .send()
        .await;
    res.headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("W/\"settings-v0\"")
        .to_string()
}

#[tokio::test]
async fn get_access_settings_returns_200() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/settings/access")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert!(body["mode"].as_str().is_some(), "mode field missing");
    assert!(
        body["password_auth_enabled"].as_bool().is_some(),
        "password_auth_enabled field missing"
    );
    assert!(
        body["two_factor_required"].as_bool().is_some(),
        "two_factor_required field missing"
    );
}

#[tokio::test]
async fn get_access_settings_returns_etag() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let res = client
        .get("/api/v1/settings/access")
        .bearer(&token)
        .send()
        .await;

    assert_eq!(res.status(), http::StatusCode::OK);
    assert!(
        res.headers().get("etag").is_some(),
        "ETag header missing"
    );
}

#[tokio::test]
async fn update_access_settings_returns_etag() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;
    let etag = current_access_etag(&client, &token).await;

    let res = client
        .put_json(
            "/api/v1/settings/access",
            &serde_json::json!({ "mode": "open" }),
        )
        .bearer(&token)
        .header("if-match", &etag)
        .send()
        .await;

    assert_eq!(res.status(), http::StatusCode::OK);
    assert!(
        res.headers().get("etag").is_some(),
        "PUT response must include ETag so client can chain subsequent saves"
    );
}

#[tokio::test]
async fn update_access_settings_open_mode_returns_200() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;
    let etag = current_access_etag(&client, &token).await;

    let (status, body): (_, serde_json::Value) = client
        .put_json(
            "/api/v1/settings/access",
            &serde_json::json!({ "mode": "open" }),
        )
        .bearer(&token)
        .header("if-match", &etag)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["mode"], "open");
}

#[tokio::test]
async fn update_access_settings_invite_without_token_returns_422() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;
    let etag = current_access_etag(&client, &token).await;

    let (status, _): (_, serde_json::Value) = client
        .put_json(
            "/api/v1/settings/access",
            &serde_json::json!({ "mode": "invite" }),
        )
        .bearer(&token)
        .header("if-match", &etag)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn update_access_settings_invite_with_token_returns_200() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;
    let etag = current_access_etag(&client, &token).await;

    let (status, body): (_, serde_json::Value) = client
        .put_json(
            "/api/v1/settings/access",
            &serde_json::json!({ "mode": "invite", "token": "secret123" }),
        )
        .bearer(&token)
        .header("if-match", &etag)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["mode"], "invite");
}

#[tokio::test]
async fn update_access_settings_persists_on_get() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let etag = current_access_etag(&client, &token).await;
    client
        .put_json(
            "/api/v1/settings/access",
            &serde_json::json!({ "mode": "open", "password_auth_enabled": true }),
        )
        .bearer(&token)
        .header("if-match", &etag)
        .send_json::<(_, serde_json::Value)>()
        .await;

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/settings/access")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["mode"], "open");
}

#[tokio::test]
async fn get_access_settings_requires_auth() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();

    let (status, _): (_, serde_json::Value) = client
        .get("/api/v1/settings/access")
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn get_combined_settings_no_longer_has_registration_field() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/settings")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert!(body.get("registration").is_none(), "registration field must be absent");
    assert!(body.get("authentication").is_none(), "authentication field must be absent");
    assert!(body.get("agent_certificates").is_some(), "agent_certificates missing");
    assert!(body.get("enrollment_tokens").is_some(), "enrollment_tokens missing");
    assert!(body.get("multi_tenancy_enabled").is_some(), "multi_tenancy_enabled missing");
}
```

- [ ] **Step 2: Register the module in `integration_tests/mod.rs`**

Add:

```rust
mod settings_access;
```

- [ ] **Step 3: Prune `integration_tests/settings.rs`**

Remove `get_registration_settings_returns_200` and `update_registration_settings_returns_200` tests. Keep `get_combined_settings_returns_200` but
update it to assert on the new shape (no `registration` or `authentication` fields). The file can be deleted entirely if it becomes empty after
pruning; however keep `get_combined_settings_returns_200` renamed as `get_combined_settings_returns_ok_shape` to check the new response structure.

- [ ] **Step 4: Run integration tests**

```bash
cargo test -p uptrakit-web-api --no-default-features --features db-sqlite settings_access 2>&1 | tail -30
```

Expected: all new tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/web-api/src/integration_tests/settings_access.rs \
        crates/ui/web-api/src/integration_tests/mod.rs \
        crates/ui/web-api/src/integration_tests/settings.rs
git commit --only \
        crates/ui/web-api/src/integration_tests/settings_access.rs \
        crates/ui/web-api/src/integration_tests/mod.rs \
        crates/ui/web-api/src/integration_tests/settings.rs \
        -m "test(api): add integration tests for GET/PUT /api/v1/settings/access"
```

---

### Task 6: Frontend types + API layer

**Files:**

- Modify: `frontend/src/lib/types.ts`
- Modify: `frontend/src/lib/api.ts`
- Create: `frontend/src/lib/api/settings.ts`

- [ ] **Step 1: Write failing test to verify types compile (vitest type check)**

```bash
cd frontend && npm run check 2>&1 | grep "Error" | head -20
```

Expected: errors about `RegistrationSettings`, `AuthenticationSettings` etc. in consumers after the types are removed. This is the expected red state.

- [ ] **Step 2: Update `frontend/src/lib/types.ts`**

Remove these interfaces:

- `RegistrationSettings`
- `UpdateRegistrationSettings`
- `AuthenticationSettings`
- `UpdateAuthenticationSettings`

Update `CombinedSettingsResponse` — remove `registration` and `authentication` fields:

```typescript
export interface CombinedSettingsResponse {
  agent_certificates: AgentCertificateSettings;
  enrollment_tokens: EnrollmentTokensSummary;
  multi_tenancy_enabled: boolean;
}
```

Add new types:

```typescript
export interface AccessSettingsData {
  mode: "open" | "invite" | "closed";
  require_token_for_oidc: boolean;
  password_auth_enabled: boolean;
  two_factor_required: boolean;
}

export interface UpdateAccessSettingsRequest {
  mode: "open" | "invite" | "closed";
  token?: string;
  require_token_for_oidc?: boolean;
  password_auth_enabled?: boolean;
  two_factor_required?: boolean;
}

export interface AccessSettingsWithEtag {
  data: AccessSettingsData;
  etag: string | null;
}
```

- [ ] **Step 3: Update `frontend/src/lib/api.ts`**

Remove the four old functions:

- `getRegistrationSettings`
- `updateRegistrationSettings`
- `getAuthenticationSettings`
- `updateAuthenticationSettings`

Remove their type imports:

```typescript
// Remove from imports:
RegistrationSettings,
UpdateRegistrationSettings,
AuthenticationSettings,
UpdateAuthenticationSettings,
```

Update `getCombinedSettings` return type to match the trimmed interface (TypeScript will error if the old fields are still referenced in the return
type annotation).

- [ ] **Step 4: Create `frontend/src/lib/api/settings.ts`**

```typescript
// frontend/src/lib/api/settings.ts

import { authenticatedFetch, extractErrorMessage } from "$lib/api";
import type { AccessSettingsData, AccessSettingsWithEtag, UpdateAccessSettingsRequest } from "$lib/types";

export async function getAccessSettings(): Promise<AccessSettingsWithEtag> {
  let res: Response;
  try {
    res = await authenticatedFetch("/api/v1/settings/access");
  } catch (err) {
    if (err instanceof DOMException && (err.name === "AbortError" || err.name === "TimeoutError")) {
      throw new Error("Request timed out. Please try again.");
    } else if (err instanceof TypeError) {
      throw new Error("Network error: Unable to connect to the server. Check your network connection.");
    }
    throw err;
  }
  if (!res.ok) {
    const message = await extractErrorMessage(res);
    throw new Error(message);
  }
  const data: AccessSettingsData = await res.json();
  return { data, etag: res.headers.get("etag") };
}

export async function updateAccessSettings(body: UpdateAccessSettingsRequest, etag: string | null): Promise<AccessSettingsWithEtag> {
  const headers: Record<string, string> = {};
  if (etag !== null) headers["if-match"] = etag;
  let res: Response;
  try {
    res = await authenticatedFetch("/api/v1/settings/access", {
      method: "PUT",
      body: JSON.stringify(body),
      headers,
    });
  } catch (err) {
    if (err instanceof DOMException && (err.name === "AbortError" || err.name === "TimeoutError")) {
      throw new Error("Request timed out. Please try again.");
    } else if (err instanceof TypeError) {
      throw new Error("Network error: Unable to connect to the server. Check your network connection.");
    }
    throw err;
  }
  if (!res.ok) {
    const message = await extractErrorMessage(res);
    throw new Error(message);
  }
  const data: AccessSettingsData = await res.json();
  return { data, etag: res.headers.get("etag") };
}
```

- [ ] **Step 5: Run type check**

```bash
cd frontend && npm run check 2>&1 | grep "Error" | head -30
```

Expected: errors only in components that still import old types — those will be fixed in subsequent tasks.

- [ ] **Step 6: Commit**

```bash
cd frontend && git add src/lib/types.ts src/lib/api.ts src/lib/api/settings.ts
git commit --only \
        frontend/src/lib/types.ts \
        frontend/src/lib/api.ts \
        frontend/src/lib/api/settings.ts \
        -m "feat(frontend): add AccessSettings types and ETag-aware API functions"
```

---

### Task 7: `createFormDraft` utility

**Files:**

- Create: `frontend/src/lib/forms/draft.svelte.ts`
- Create: `frontend/src/lib/forms/index.ts`

- [ ] **Step 1: Write failing Vitest test first**

Create `frontend/src/lib/forms/draft.svelte.test.ts`:

```typescript
import { describe, expect, it } from "vitest";
import { createFormDraft } from "./draft.svelte";

describe("createFormDraft", () => {
  it("isDirty is false on creation", () => {
    const form = createFormDraft({ name: "alice", enabled: true });
    expect(form.isDirty).toBe(false);
  });

  it("isDirty becomes true after update", () => {
    const form = createFormDraft({ name: "alice", enabled: true });
    form.update("name", "bob");
    expect(form.isDirty).toBe(true);
  });

  it("isFieldDirty tracks individual fields", () => {
    const form = createFormDraft({ name: "alice", enabled: true });
    form.update("name", "bob");
    expect(form.isFieldDirty("name")).toBe(true);
    expect(form.isFieldDirty("enabled")).toBe(false);
  });

  it("discard restores draft to serverValues", () => {
    const form = createFormDraft({ name: "alice", enabled: true });
    form.update("name", "bob");
    form.discard();
    expect(form.draft.name).toBe("alice");
    expect(form.isDirty).toBe(false);
  });

  it("load sets both serverValues and draft", () => {
    const form = createFormDraft({ name: "alice", enabled: true });
    form.load({ name: "carol", enabled: false });
    expect(form.draft.name).toBe("carol");
    expect(form.serverValues.name).toBe("carol");
    expect(form.isDirty).toBe(false);
  });

  it("commit sets serverValues to new state", () => {
    const form = createFormDraft({ name: "alice", enabled: true });
    form.update("name", "bob");
    form.commit({ name: "bob", enabled: true });
    expect(form.serverValues.name).toBe("bob");
    expect(form.isDirty).toBe(false);
  });
});
```

- [ ] **Step 2: Run failing test**

```bash
cd frontend && npm run test -- --run src/lib/forms/draft.svelte.test.ts 2>&1 | tail -20
```

Expected: FAIL — module not found.

- [ ] **Step 3: Create `draft.svelte.ts`**

```typescript
// frontend/src/lib/forms/draft.svelte.ts

export interface FormDraft<T extends Record<string, unknown>> {
  readonly draft: T;
  readonly serverValues: T;
  readonly isDirty: boolean;
  isFieldDirty(key: keyof T): boolean;
  update<K extends keyof T>(key: K, value: T[K]): void;
  load(values: T): void;
  commit(updated: T): void;
  discard(): void;
}

export function createFormDraft<T extends Record<string, unknown>>(initial: T): FormDraft<T> {
  let serverValues = $state<T>({ ...initial });
  let draft = $state<T>({ ...initial });

  const isDirty = $derived((Object.keys(serverValues) as (keyof T)[]).some((k) => draft[k] !== serverValues[k]));

  return {
    get draft() {
      return draft;
    },
    get serverValues() {
      return serverValues;
    },
    get isDirty() {
      return isDirty;
    },
    isFieldDirty(key) {
      return draft[key] !== serverValues[key];
    },
    update(key, value) {
      draft[key] = value;
    },
    load(values) {
      serverValues = { ...values };
      draft = { ...values };
    },
    commit(updated) {
      serverValues = { ...updated };
      draft = { ...updated };
    },
    discard() {
      draft = { ...serverValues };
    },
  };
}
```

- [ ] **Step 4: Create `forms/index.ts`**

```typescript
// frontend/src/lib/forms/index.ts
export { createFormDraft } from "./draft.svelte";
export type { FormDraft } from "./draft.svelte";
```

- [ ] **Step 5: Run tests**

```bash
cd frontend && npm run test -- --run src/lib/forms/draft.svelte.test.ts 2>&1 | tail -20
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/lib/forms/draft.svelte.ts \
        frontend/src/lib/forms/index.ts \
        frontend/src/lib/forms/draft.svelte.test.ts
git commit --only \
        frontend/src/lib/forms/draft.svelte.ts \
        frontend/src/lib/forms/index.ts \
        frontend/src/lib/forms/draft.svelte.test.ts \
        -m "feat(frontend): add createFormDraft Svelte 5 reactive utility"
```

---

### Task 8: `FormFieldRow` fix + `RadioCardGroup` component

**Files:**

- Modify: `frontend/src/lib/components/forms/FormFieldRow.svelte`
- Create: `frontend/src/lib/components/forms/RadioCardGroup.svelte`
- Create: `frontend/src/lib/components/forms/RadioCardGroup.test.ts`
- Modify: `frontend/src/lib/components/forms/index.ts`

- [ ] **Step 1: Write failing RadioCardGroup test**

Create `frontend/src/lib/components/forms/RadioCardGroup.test.ts`:

```typescript
import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import RadioCardGroup from "./RadioCardGroup.svelte";

const options = [
  { value: "open", label: "Open", description: "Anyone can create an account." },
  { value: "invite", label: "Invite Only", description: "Token required." },
  { value: "closed", label: "Closed", description: "No new accounts." },
];

describe("RadioCardGroup", () => {
  it("renders all options", () => {
    render(RadioCardGroup, { name: "mode", value: "open", options });
    expect(screen.getByText("Open")).toBeTruthy();
    expect(screen.getByText("Invite Only")).toBeTruthy();
    expect(screen.getByText("Closed")).toBeTruthy();
  });

  it("selected card has aria-checked=true", () => {
    render(RadioCardGroup, { name: "mode", value: "invite", options });
    const inviteCard = screen.getByRole("radio", { name: /invite only/i });
    expect(inviteCard).toHaveAttribute("aria-checked", "true");
  });

  it("unselected cards have aria-checked=false", () => {
    render(RadioCardGroup, { name: "mode", value: "open", options });
    const inviteCard = screen.getByRole("radio", { name: /invite only/i });
    expect(inviteCard).toHaveAttribute("aria-checked", "false");
  });

  it("container has role=radiogroup", () => {
    render(RadioCardGroup, { name: "mode", value: "open", options });
    expect(screen.getByRole("radiogroup")).toBeTruthy();
  });

  it("calls onchange when a card is clicked", async () => {
    const onchange = vi.fn();
    render(RadioCardGroup, { name: "mode", value: "open", options, onchange });
    await fireEvent.click(screen.getByRole("radio", { name: /closed/i }));
    expect(onchange).toHaveBeenCalledWith("closed");
  });

  it("disabled cards do not fire onchange", async () => {
    const onchange = vi.fn();
    render(RadioCardGroup, { name: "mode", value: "open", options, onchange, disabled: true });
    await fireEvent.click(screen.getByRole("radio", { name: /closed/i }));
    expect(onchange).not.toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: Run failing test**

```bash
cd frontend && npm run test -- --run src/lib/components/forms/RadioCardGroup.test.ts 2>&1 | tail -20
```

Expected: FAIL — module not found.

- [ ] **Step 3: Create `RadioCardGroup.svelte`**

```svelte
<!-- frontend/src/lib/components/forms/RadioCardGroup.svelte -->
<script lang="ts" generics="T extends string">
  export interface RadioCardOption {
    value: T;
    label: string;
    description?: string;
  }

  let {
    name,
    value,
    options,
    onchange,
    disabled = false
  }: {
    name: string;
    value: T;
    options: RadioCardOption[];
    onchange?: (value: T) => void;
    disabled?: boolean;
  } = $props();

  function select(v: T) {
    if (!disabled) onchange?.(v);
  }

  function handleKeydown(e: KeyboardEvent, idx: number) {
    if (disabled) return;
    let next = idx;
    if (e.key === 'ArrowRight' || e.key === 'ArrowDown') {
      e.preventDefault();
      next = (idx + 1) % options.length;
    } else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') {
      e.preventDefault();
      next = (idx - 1 + options.length) % options.length;
    }
    if (next !== idx) {
      onchange?.(options[next].value);
    }
  }
</script>

<div
  role="radiogroup"
  aria-label={name}
  style="display: grid; grid-template-columns: repeat({options.length}, 1fr); gap: 0.5rem;"
>
  {#each options as option, i (option.value)}
    {@const selected = option.value === value}
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      aria-label={option.label}
      {disabled}
      onclick={() => select(option.value)}
      onkeydown={(e) => handleKeydown(e, i)}
      class="
        rounded px-3 py-3 text-left transition-[background,border-color,color]
        duration-[var(--duration-fast,150ms)] cursor-pointer
        {selected
          ? 'border-2 border-[rgba(var(--accent-rgb,6,182,212),0.6)] bg-[rgba(var(--accent-rgb,6,182,212),0.07)] text-[var(--accent-bright)]'
          : 'border border-[var(--border-subtle)] bg-transparent text-[var(--text-secondary)]'}
        disabled:cursor-not-allowed disabled:opacity-50
      "
    >
      <div class="text-sm font-semibold">{option.label}</div>
      {#if option.description}
        <div class="mt-1 text-xs leading-relaxed opacity-70">{option.description}</div>
      {/if}
    </button>
  {/each}
</div>
```

- [ ] **Step 4: Export from `forms/index.ts`**

In `frontend/src/lib/components/forms/index.ts`, add:

```typescript
export { default as RadioCardGroup } from "./RadioCardGroup.svelte";
```

- [ ] **Step 5: Fix `FormFieldRow.svelte` — add `dirty` prop + `w-fit` label**

Replace the current `$props()` destructure:

```svelte
let {
  label,
  hint,
  error,
  inputId,
  required = false,
  dirty = false,
  children
}: {
  label: string;
  hint?: string;
  error?: string;
  inputId?: string;
  required?: boolean;
  dirty?: boolean;
  children: Snippet;
} = $props();
```

Replace the outer div opening tag:

```svelte
<div
  class="grid gap-3 md:items-start {labelColClass} {dirty ? 'border-l-2 border-[var(--accent)] pl-2' : ''}"
  data-ui="form-field-row"
>
```

Add `w-fit` to the `<label>` element:

```svelte
<label class="w-fit text-sm font-medium text-[var(--text-primary)]" for={inputId}>{label}</label>
```

- [ ] **Step 6: Run tests**

```bash
cd frontend && npm run test -- --run src/lib/components/forms/RadioCardGroup.test.ts 2>&1 | tail -20
```

Expected: PASS.

- [ ] **Step 7: Full frontend lint + type-check**

```bash
cd frontend && npm run lint && npm run check 2>&1 | grep "Error\|error" | head -20
```

Expected: no errors from the new files.

- [ ] **Step 8: Commit**

```bash
git commit --only \
        frontend/src/lib/components/forms/FormFieldRow.svelte \
        frontend/src/lib/components/forms/RadioCardGroup.svelte \
        frontend/src/lib/components/forms/RadioCardGroup.test.ts \
        frontend/src/lib/components/forms/index.ts \
        -m "feat(ui): add RadioCardGroup; fix FormFieldRow label width + dirty highlight"
```

---

### Task 9: `AccessSettings.svelte`

**Files:**

- Create: `frontend/src/routes/settings/AccessSettings.svelte`
- Create: `frontend/src/routes/settings/AccessSettings.test.ts`

- [ ] **Step 1: Write failing test first**

Create `frontend/src/routes/settings/AccessSettings.test.ts`:

```typescript
import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";

vi.mock("$lib/api/settings", () => ({
  getAccessSettings: vi.fn(async () => ({
    data: { mode: "open", require_token_for_oidc: false, password_auth_enabled: true, two_factor_required: false },
    etag: 'W/"settings-v0"',
  })),
  updateAccessSettings: vi.fn(async () => ({
    data: { mode: "open", require_token_for_oidc: false, password_auth_enabled: true, two_factor_required: false },
    etag: 'W/"settings-v1"',
  })),
}));
vi.mock("$lib/stores/network.svelte", () => ({ getIsOnline: vi.fn(() => true) }));

import * as settingsApi from "$lib/api/settings";
import AccessSettings from "./AccessSettings.svelte";

const defaultProps = { onSuccess: vi.fn(), onError: vi.fn() };

afterEach(() => vi.clearAllMocks());

describe("AccessSettings", () => {
  it("Save button is disabled when form is not dirty", async () => {
    render(AccessSettings, defaultProps);
    await waitFor(() => expect(screen.getByRole("button", { name: "Save" })).toBeTruthy());
    const btn = screen.getByRole("button", { name: "Save" });
    expect(btn).toBeDisabled();
  });

  it("Discard button hidden when not dirty", async () => {
    render(AccessSettings, defaultProps);
    await waitFor(() => screen.getByRole("button", { name: "Save" }));
    expect(screen.queryByRole("button", { name: "Discard" })).toBeNull();
  });

  it("calls updateAccessSettings on save", async () => {
    render(AccessSettings, defaultProps);
    await waitFor(() => screen.getByRole("button", { name: "Save" }));
    // RadioCardGroup: click "Closed" card to make form dirty
    const closedCard = screen.getByRole("radio", { name: /closed/i });
    await fireEvent.click(closedCard);
    const btn = screen.getByRole("button", { name: "Save" });
    expect(btn).not.toBeDisabled();
    await fireEvent.click(btn);
    await waitFor(() => expect(settingsApi.updateAccessSettings).toHaveBeenCalled());
  });

  it("shows registration token field only in invite mode", async () => {
    render(AccessSettings, defaultProps);
    await waitFor(() => screen.getByRole("button", { name: "Save" }));
    expect(screen.queryByLabelText(/registration token/i)).toBeNull();
    const inviteCard = screen.getByRole("radio", { name: /invite only/i });
    await fireEvent.click(inviteCard);
    await waitFor(() => expect(screen.getByLabelText(/registration token/i)).toBeTruthy());
  });
});
```

- [ ] **Step 2: Run failing test**

```bash
cd frontend && npm run test -- --run src/routes/settings/AccessSettings.test.ts 2>&1 | tail -20
```

Expected: FAIL — module not found.

- [ ] **Step 3: Create `AccessSettings.svelte`**

```svelte
<!-- frontend/src/routes/settings/AccessSettings.svelte -->
<script lang="ts">
  import { onMount } from 'svelte';
  import { SectionCard } from '$lib/components/ui';
  import { FormFieldRow, Checkbox, Input, RadioCardGroup } from '$lib/components/forms';
  import Button from '$lib/components/Button.svelte';
  import { createFormDraft } from '$lib/forms/draft.svelte';
  import { getAccessSettings, updateAccessSettings } from '$lib/api/settings';

  let {
    onSuccess,
    onError
  }: {
    onSuccess: (msg: string) => void;
    onError: (msg: string) => void;
  } = $props();

  interface AccessDraft {
    mode: 'open' | 'invite' | 'closed';
    token: string;
    requireTokenForOidc: boolean;
    passwordAuthEnabled: boolean;
    twoFactorRequired: boolean;
  }

  const form = createFormDraft<AccessDraft>({
    mode: 'open',
    token: '',
    requireTokenForOidc: false,
    passwordAuthEnabled: true,
    twoFactorRequired: false
  });

  // Exclude token from dirty check — server never returns it, always starts empty
  const isDirty = $derived(
    form.isFieldDirty('mode') ||
      form.isFieldDirty('requireTokenForOidc') ||
      form.isFieldDirty('passwordAuthEnabled') ||
      form.isFieldDirty('twoFactorRequired')
  );

  let etag: string | null = null;
  let loading = $state(true);
  let saving = $state(false);

  const modeOptions = [
    { value: 'open' as const, label: 'Open', description: 'Anyone can create an account.' },
    { value: 'invite' as const, label: 'Invite Only', description: 'Token required to register.' },
    { value: 'closed' as const, label: 'Closed', description: 'No new accounts allowed.' }
  ];

  onMount(async () => {
    try {
      const { data, etag: e } = await getAccessSettings();
      etag = e;
      form.load({
        mode: data.mode,
        token: '',
        requireTokenForOidc: data.require_token_for_oidc,
        passwordAuthEnabled: data.password_auth_enabled,
        twoFactorRequired: data.two_factor_required
      });
    } catch (e) {
      onError(e instanceof Error ? e.message : 'Failed to load access settings');
    } finally {
      loading = false;
    }
  });

  async function save() {
    saving = true;
    try {
      const body: import('$lib/types').UpdateAccessSettingsRequest = {
        mode: form.draft.mode,
        password_auth_enabled: form.draft.passwordAuthEnabled,
        two_factor_required: form.draft.twoFactorRequired
      };
      if (form.draft.mode === 'invite') {
        if (form.draft.token) body.token = form.draft.token;
        body.require_token_for_oidc = form.draft.requireTokenForOidc;
      }
      const { data, etag: newEtag } = await updateAccessSettings(body, etag);
      etag = newEtag;
      form.commit({
        mode: data.mode,
        token: '',
        requireTokenForOidc: data.require_token_for_oidc,
        passwordAuthEnabled: data.password_auth_enabled,
        twoFactorRequired: data.two_factor_required
      });
      onSuccess('Access settings saved.');
    } catch (e) {
      onError(e instanceof Error ? e.message : 'Failed to save access settings');
    } finally {
      saving = false;
    }
  }
</script>

<SectionCard
  title="Registration & Authentication"
  description="Control who can register and how users authenticate."
>
  {#if loading}
    <p class="text-[var(--text-secondary)]">Loading...</p>
  {:else}
    <div class="space-y-4">
      <FormFieldRow label="Registration Mode" dirty={form.isFieldDirty('mode')}>
        <RadioCardGroup
          name="registration-mode"
          value={form.draft.mode}
          options={modeOptions}
          onchange={(v) => form.update('mode', v as 'open' | 'invite' | 'closed')}
          disabled={saving}
        />
      </FormFieldRow>

      {#if form.draft.mode === 'invite'}
        <FormFieldRow
          label="Registration Token"
          inputId="reg-token"
          hint="Set a new token. Leave blank to keep the current token."
          dirty={false}
        >
          <Input
            id="reg-token"
            type="password"
            placeholder="Enter a new registration token"
            bind:value={form.draft.token}
            disabled={saving}
          />
        </FormFieldRow>

        <FormFieldRow label="OIDC First Login" dirty={form.isFieldDirty('requireTokenForOidc')}>
          <label class="flex items-center gap-2">
            <Checkbox
              id="oidc-first-login"
              bind:checked={form.draft.requireTokenForOidc}
              disabled={saving}
            />
            <span class="text-sm">Require registration token for OIDC users</span>
          </label>
        </FormFieldRow>
      {/if}

      <FormFieldRow label="Password Authentication" dirty={form.isFieldDirty('passwordAuthEnabled')}>
        <label class="flex items-center gap-2">
          <Checkbox
            id="password-auth"
            bind:checked={form.draft.passwordAuthEnabled}
            disabled={saving}
          />
          <span class="text-sm">Enable password-based login</span>
        </label>
      </FormFieldRow>

      <FormFieldRow label="Require Two-Factor Auth" dirty={form.isFieldDirty('twoFactorRequired')}>
        <label class="flex items-center gap-2">
          <Checkbox
            id="two-factor-required"
            bind:checked={form.draft.twoFactorRequired}
            disabled={saving}
          />
          <span class="text-sm">Require 2FA for all users</span>
        </label>
      </FormFieldRow>

      <div class="flex gap-2">
        <Button disabled={!isDirty || saving} loading={saving} onclick={save}>Save</Button>
        {#if isDirty}
          <Button variant="ghost" disabled={saving} onclick={() => form.discard()}>Discard</Button>
        {/if}
      </div>
    </div>
  {/if}
</SectionCard>
```

- [ ] **Step 4: Run test**

```bash
cd frontend && npm run test -- --run src/routes/settings/AccessSettings.test.ts 2>&1 | tail -30
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/routes/settings/AccessSettings.svelte \
        frontend/src/routes/settings/AccessSettings.test.ts
git commit --only \
        frontend/src/routes/settings/AccessSettings.svelte \
        frontend/src/routes/settings/AccessSettings.test.ts \
        -m "feat(ui): add AccessSettings component with draft pattern and RadioCardGroup"
```

---

### Task 10: `AgentCertificateSettings` draft + `McpAccessTab`

**Files:**

- Modify: `frontend/src/routes/settings/AgentCertificateSettings.svelte`
- Create: `frontend/src/routes/settings/RegisterClientDialog.svelte`
- Create: `frontend/src/routes/settings/McpAccessTab.svelte`

- [ ] **Step 1: Update `AgentCertificateSettings.svelte` — add `createFormDraft`**

The component currently holds three `$state` variables manually (`certLifetimeDays`, `useAutoRenewal`, `certRenewalWindowHours`) with a `$effect` that
syncs from the `settings` prop. Replace with `createFormDraft`.

```svelte
<!-- frontend/src/routes/settings/AgentCertificateSettings.svelte -->
<script lang="ts">
  import { updateAgentCertificateSettings } from '$lib/api';
  import type { AgentCertificateSettings } from '$lib/types';
  import { SectionCard } from '$lib/components/ui';
  import { FormFieldRow, Checkbox, Input } from '$lib/components/forms';
  import Button from '$lib/components/Button.svelte';
  import { createFormDraft } from '$lib/forms/draft.svelte';

  let {
    settings,
    onSuccess,
    onError
  }: {
    settings: AgentCertificateSettings | undefined;
    onSuccess: (msg: string) => void;
    onError: (msg: string) => void;
  } = $props();

  interface CertDraft {
    lifetimeDays: number;
    useAutoRenewal: boolean;
    renewalWindowHours: number;
  }

  const form = createFormDraft<CertDraft>({
    lifetimeDays: 365,
    useAutoRenewal: true,
    renewalWindowHours: 24
  });

  let saving = $state(false);

  $effect(() => {
    if (settings) {
      form.load({
        lifetimeDays: settings.lifetime_days,
        useAutoRenewal: settings.renewal_window_hours_override === null,
        renewalWindowHours:
          settings.renewal_window_hours_override ?? settings.effective_renewal_window_hours
      });
    }
  });

  async function saveCertificates() {
    saving = true;
    try {
      const renewalHours = form.draft.useAutoRenewal ? 0 : form.draft.renewalWindowHours;
      const res = await updateAgentCertificateSettings({
        lifetime_days: form.draft.lifetimeDays,
        renewal_window_hours: renewalHours
      });
      form.commit({
        lifetimeDays: res.lifetime_days,
        useAutoRenewal: res.renewal_window_hours_override === null,
        renewalWindowHours:
          res.renewal_window_hours_override ?? res.effective_renewal_window_hours
      });
      onSuccess('Agent certificate settings saved.');
    } catch (e) {
      onError(e instanceof Error ? e.message : 'Failed to save agent certificate settings');
    } finally {
      saving = false;
    }
  }
</script>
```

Keep the template section (`<SectionCard>` block) unchanged except update the bindings from old state variables to `form.draft.*` and add dirty
indicators and Discard button:

- `certLifetimeDays` → `form.draft.lifetimeDays`
- `useAutoRenewal` → `form.draft.useAutoRenewal`
- `certRenewalWindowHours` → `form.draft.renewalWindowHours`

Update the Save button:

```svelte
<Button disabled={!form.isDirty || saving} loading={saving} onclick={saveCertificates}>Save</Button>
{#if form.isDirty}
  <Button variant="ghost" disabled={saving} onclick={() => form.discard()}>Discard</Button>
{/if}
```

Add `dirty={form.isFieldDirty('lifetimeDays')}` to the Certificate Lifetime `FormFieldRow`.

- [ ] **Step 2: Move `RegisterClientDialog` to settings directory**

Copy the full content of `frontend/src/routes/settings/authentication/oauth-clients/RegisterClientDialog.svelte` to
`frontend/src/routes/settings/RegisterClientDialog.svelte`.

Update any relative imports within the file to reference the new location (typically none — the dialog only imports from `$lib`).

- [ ] **Step 3: Create `McpAccessTab.svelte`**

Extract and adapt the content from `frontend/src/routes/settings/authentication/oauth-clients/+page.svelte`:

Key changes:

- Remove the `PageShell` wrapper
- Replace `RegisterClientDialog` import path to `./RegisterClientDialog.svelte`
- Replace the manual draft state (`draft`, `oauthSettings`, manual `isDirty`, `handleDiscard`) with `createFormDraft`

```svelte
<!-- frontend/src/routes/settings/McpAccessTab.svelte -->
<script lang="ts">
  import { onMount } from 'svelte';
  import { getOAuthSettings, updateOAuthSettings } from '$lib/api/oauth';
  import { createFormDraft } from '$lib/forms/draft.svelte';
  // ... (copy remaining imports from the original +page.svelte, adjusting RegisterClientDialog path)
  import RegisterClientDialog from './RegisterClientDialog.svelte';

  // OAuth settings draft
  interface OAuthSettingsDraft {
    mcp_enabled: boolean;
    dcr_enabled: boolean;
    cimd_enabled: boolean;
    canonical_host: string | null;
  }

  const oauthDraft = createFormDraft<OAuthSettingsDraft>({
    mcp_enabled: false,
    dcr_enabled: false,
    cimd_enabled: false,
    canonical_host: null
  });

  let oauthSettingsEtag = $state<string | null>(null);

  // ... (copy all other state and functions from the original +page.svelte)
  // Replace:
  //   oauthSettings = data  → oauthDraft.load(data)
  //   isDirty (manual)      → oauthDraft.isDirty
  //   handleDiscard()       → oauthDraft.discard()
  //   draft.*               → oauthDraft.draft.*
  //   handleSettingsChange  → oauthDraft.commit(response)

  async function handleSettingsChange(updatedDraft: OAuthSettingsDraft) {
    const payload = {
      ...updatedDraft,
      // Map empty string → null before sending to API
      canonical_host: updatedDraft.canonical_host === '' ? null : updatedDraft.canonical_host
    };
    const { data, etag } = await updateOAuthSettings(payload, oauthSettingsEtag);
    oauthSettingsEtag = etag;
    oauthDraft.commit(data);
  }

  onMount(async () => {
    // loadClients() and loadOAuthSettings() — copied from original +page.svelte
    const { data, etag } = await getOAuthSettings();
    oauthSettingsEtag = etag;
    oauthDraft.load(data);
  });
</script>

<!-- Template: copy from original +page.svelte, remove PageShell wrapper, use oauthDraft.draft.* -->
```

The full template is large (original `+page.svelte` is 416 lines). Copy it entirely, then:

1. Delete the `<PageShell>` opening and closing tags
2. Replace all `draft.*` references with `oauthDraft.draft.*`
3. Replace `{isDirty}` with `{oauthDraft.isDirty}`
4. Replace `handleDiscard()` call with `oauthDraft.discard()`
5. Add `dirty` prop to any `FormFieldRow` that wraps an OAuth settings field

- [ ] **Step 4: Update `AgentCertificateSettings.test.ts` — dirty the form before Save assertions**

The existing test clicks the Save button without first making the form dirty. After the `createFormDraft` migration, Save is disabled when `!isDirty`,
so those clicks will be no-ops. Update each test that clicks Save to first change a field value:

```typescript
// Before the Save click, change the lifetime input to make form dirty:
const lifetimeInput = screen.getByRole("spinbutton", { name: /certificate lifetime/i });
await fireEvent.input(lifetimeInput, { target: { value: "180" } });
// Now Save is enabled
const btn = screen.getByRole("button", { name: "Save" });
await fireEvent.click(btn);
```

Also assert that the Discard button appears when dirty and disappears after Save.

- [ ] **Step 4b: Run `AgentCertificateSettings` tests**

```bash
cd frontend && npm run test -- --run src/routes/settings/AgentCertificateSettings.test.ts 2>&1 | tail -20
```

Expected: PASS after updating the test.

- [ ] **Step 5: Type check**

```bash
cd frontend && npm run check 2>&1 | grep "Error" | head -20
```

Expected: no errors from the new/modified files.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/routes/settings/AgentCertificateSettings.svelte \
        frontend/src/routes/settings/RegisterClientDialog.svelte \
        frontend/src/routes/settings/McpAccessTab.svelte
git commit --only \
        frontend/src/routes/settings/AgentCertificateSettings.svelte \
        frontend/src/routes/settings/RegisterClientDialog.svelte \
        frontend/src/routes/settings/McpAccessTab.svelte \
        -m "feat(ui): add McpAccessTab; migrate AgentCertificateSettings to createFormDraft"
```

---

### Task 11: Settings page wiring + cleanup

**Files:**

- Modify: `frontend/src/routes/settings/+page.svelte`
- Modify: `frontend/src/routes/settings/surface-tabs.test.ts`
- Delete: `frontend/src/routes/settings/RegistrationSettings.svelte`
- Delete: `frontend/src/routes/settings/RegistrationSettings.test.ts`
- Delete: `frontend/src/routes/settings/AuthenticationSettings.svelte`
- Delete: `frontend/src/routes/settings/AuthenticationSettings.test.ts`
- Delete: `frontend/src/routes/settings/authentication/` (entire directory)

- [ ] **Step 1: Update `surface-tabs.test.ts` mock**

In the `getCombinedSettings` mock, change:

```typescript
  getCombinedSettings: vi.fn(async () => ({
    registration: {},
    authentication: {},
    agent_certificates: { ... },
    enrollment_tokens: {},
    multi_tenancy_enabled: false
  })),
```

To:

```typescript
  getCombinedSettings: vi.fn(async () => ({
    agent_certificates: {
      lifetime_days: 365,
      renewal_window_hours_override: null,
      effective_renewal_window_hours: 73
    },
    enrollment_tokens: {},
    multi_tenancy_enabled: false
  })),
```

- [ ] **Step 2: Run surface-tabs test to confirm mock is correct**

```bash
cd frontend && npm run test -- --run src/routes/settings/surface-tabs.test.ts 2>&1 | tail -20
```

Expected: PASS (or may fail due to other changes — fix any remaining mock issues).

- [ ] **Step 3: Update `+page.svelte`**

**a) Add `'mcp-access'` to `BUILTIN_TAB_IDS`:**

Find the `BUILTIN_TAB_IDS` Set definition and add `'mcp-access'`.

**b) Update `tabItems` array:**

Insert after the `general` tab item, guarded by `canManageOAuthClients`:

```svelte
...(canManageOAuthClients ? [{ id: 'mcp-access', label: 'MCP Access' }] : []),
```

**c) Replace imports:**

Remove:

```typescript
import RegistrationSettings from "./RegistrationSettings.svelte";
import AuthenticationSettings from "./AuthenticationSettings.svelte";
```

Add:

```typescript
import AccessSettings from "./AccessSettings.svelte";
import McpAccessTab from "./McpAccessTab.svelte";
```

**d) Remove OAuth Clients `SectionCard` block:**

Delete the block that renders the "OAuth Clients" card in the General tab (the `SectionCard` with a link to the `oauth-clients` route, currently
around lines 324–331). This is the block that this spec replaces with the `McpAccessTab` route.

**e) Replace `RegistrationSettings` + `AuthenticationSettings` with `AccessSettings`:**

In the General tab render, find:

```svelte
<RegistrationSettings settings={registrationSettings} ... />
<AuthenticationSettings settings={authSettings} ... />
```

Replace with:

```svelte
<AccessSettings {onSuccess} {onError} />
```

Note: `AccessSettings` is self-loading; no `settings` prop is passed.

Also remove the state variables `registrationSettings`, `authSettings` and their associated error/retry handling from the `<script>` block.

Remove destructuring of `combined.registration` and `combined.authentication` wherever they appear.

**f) Add `McpAccessTab` branch:**

After the `{:else if activeTab === 'general'}` block, add:

```svelte
{:else if activeTab === 'mcp-access'}
  <McpAccessTab />
```

- [ ] **Step 4: Delete old files**

```bash
rm frontend/src/routes/settings/RegistrationSettings.svelte
rm frontend/src/routes/settings/RegistrationSettings.test.ts
rm frontend/src/routes/settings/AuthenticationSettings.svelte
rm frontend/src/routes/settings/AuthenticationSettings.test.ts
rm -rf frontend/src/routes/settings/authentication/
```

- [ ] **Step 5: Verify `settings-panels.test.ts` needs no changes**

```bash
grep -n "registration\|authentication\|oauth-clients" \
  frontend/src/routes/settings/settings-panels.test.ts 2>/dev/null | head -20
```

If any references remain, remove them. The spec §7 calls for removing OAuth Clients button assertions — confirm they are absent before proceeding.

- [ ] **Step 6: Run all frontend tests**

```bash
cd frontend && npm run test 2>&1 | tail -40
```

Expected: PASS. Fix any remaining import errors caused by the deleted files.

- [ ] **Step 7: Full lint + type check + build + e2e**

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run build 2>&1 | tail -30
```

Then run e2e tests on macOS + Chromium (required per quality gates — `FormFieldRow` layout changes may produce Playwright snapshot diffs that need
updating):

```bash
cd frontend && npm run test:e2e 2>&1 | tail -40
```

Expected: PASS. Update any snapshot diffs caused by `border-l-2` dirty highlight or `w-fit` label.

- [ ] **Step 8: Run Rust quality gates**

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features 2>&1 | tail -30
cargo deny check
bash ci/verify_typed_audit_actions.sh
bash ci/verify_handler_state_contract.sh
bash ci/verify_no_security_audit.sh
python3 ci/check_plugin_semantic_boundary.py
python3 ci/verify_db_access_policy.py
sentrux check .
markdownlint --config .markdownlint.json '**/*.md'
```

Expected: PASS on all gates.

- [ ] **Step 9: Commit**

```bash
git commit --only \
        frontend/src/routes/settings/+page.svelte \
        frontend/src/routes/settings/surface-tabs.test.ts \
        frontend/src/routes/settings/settings-panels.test.ts \
        -m "feat(ui): wire AccessSettings + McpAccessTab into settings page; remove old components"
```

---

### Task 12: Documentation

**Files:**

- Modify: `docs/development/ui/README.md`
- Modify: `docs/development/ui/primitives.md`

- [ ] **Step 1: Update `docs/development/ui/README.md`**

In the form primitives section (or wherever `FormFieldRow`, `Checkbox`, `Input` etc. are listed), add:

```markdown
- **`RadioCardGroup`** — Horizontal card-tile selector for mutually exclusive enum values. See `primitives.md` for usage.
```

- [ ] **Step 2: Update `docs/development/ui/primitives.md`**

Add a `RadioCardGroup` entry:

````markdown
### RadioCardGroup

**Location:** `frontend/src/lib/components/forms/RadioCardGroup.svelte` **Import:** `import { RadioCardGroup } from '$lib/components/forms';`

Horizontal card-tile selector for mutually exclusive string options. No radio indicators — selection is conveyed by accent border + background tint
only.

**Props:**

| Prop       | Type                      | Description                        |
| ---------- | ------------------------- | ---------------------------------- |
| `name`     | `string`                  | ARIA label for the group           |
| `value`    | `string`                  | Currently selected value           |
| `options`  | `RadioCardOption[]`       | `{ value, label, description? }[]` |
| `onchange` | `(value: string) => void` | Called when selection changes      |
| `disabled` | `boolean`                 | Disables all cards                 |

**Accessibility:** `role="radiogroup"` on container; each card has `role="radio"` + `aria-checked`.

**Example:**

```svelte
<RadioCardGroup
  name="registration-mode"
  value={form.draft.mode}
  options={[
    { value: 'open', label: 'Open', description: 'Anyone can register.' },
    { value: 'invite', label: 'Invite Only', description: 'Token required.' },
    { value: 'closed', label: 'Closed', description: 'No new accounts.' }
  ]}
  onchange={(v) => form.update('mode', v)}
/>
```
````

---

### createFormDraft

**Location:** `frontend/src/lib/forms/draft.svelte.ts` **Import:** `import { createFormDraft } from '$lib/forms/draft.svelte';`

Svelte 5 reactive factory for the settings draft pattern: tracks server-committed state vs. in-progress edits, computes dirty state, and provides
load/commit/discard lifecycle methods.

**Interface:**

```typescript
interface FormDraft<T> {
  readonly draft: T; // current in-progress edits
  readonly serverValues: T; // last committed server state
  readonly isDirty: boolean; // any field differs from serverValues
  isFieldDirty(key: keyof T): boolean;
  update<K extends keyof T>(key: K, value: T[K]): void;
  load(values: T): void; // on data fetch — sets both draft and serverValues
  commit(updated: T): void; // on successful save — sets both to the server response
  discard(): void; // reset draft to serverValues
}
```

**When to use:** Any editable settings form that needs a Save/Discard pair with disabled-when-clean Save button and per-field dirty indicators.

**Critical:** Do **not** destructure the return value — `const { draft } = form` takes a snapshot. Always access through `form.draft`, `form.isDirty`,
etc.

- [ ] **Step 3: Lint docs**

```bash
npx markdownlint --config .markdownlint.json 'docs/development/ui/**/*.md' 2>&1 | head -20
```

Fix any lint errors:

```bash
npx prettier --prose-wrap always --print-width 150 --write docs/development/ui/README.md docs/development/ui/primitives.md
```

- [ ] **Step 4: Commit**

```bash
git add docs/development/ui/README.md docs/development/ui/primitives.md
git commit --only \
        docs/development/ui/README.md \
        docs/development/ui/primitives.md \
        -m "docs(ui): add RadioCardGroup and createFormDraft entries to primitives"
```

---

## Self-Review

### Spec coverage check

| Spec requirement                                                      | Task        |
| --------------------------------------------------------------------- | ----------- |
| MCP Access tab after General                                          | Task 11     |
| `canManageOAuthClients` guard on tab                                  | Task 11     |
| `mcp-access` in `BUILTIN_TAB_IDS`                                     | Task 11     |
| Delete `authentication/` directory                                    | Task 11     |
| Label width fix (`w-fit`)                                             | Task 8      |
| `dirty` prop on `FormFieldRow`                                        | Task 8      |
| `createFormDraft` factory                                             | Task 7      |
| `RadioCardGroup` component                                            | Task 8      |
| `RadioCardGroup` tests                                                | Task 8      |
| `AccessSettings.svelte`                                               | Task 9      |
| `AccessSettings` self-loads via `getAccessSettings()`                 | Task 9      |
| Custom `isDirty` excluding `token`                                    | Task 9      |
| `commit({ ...response, token: '' })` shape                            | Task 9      |
| `AgentCertificateSettings` draft pattern                              | Task 10     |
| `McpAccessTab.svelte` lifting OAuth Clients                           | Task 10     |
| `canonical_host` empty-string→null mapping                            | Task 10     |
| `createFormDraft` in `McpAccessTab`                                   | Task 10     |
| `PUT /api/v1/settings/access` unified handler                         | Task 2      |
| `GET /api/v1/settings/access` with ETag                               | Task 2      |
| Transaction order: checks → IMMEDIATE → write → bump → commit → audit | Task 2      |
| `settings_version_cache` bumped unconditionally                       | Task 2      |
| `CombinedSettingsResponse` drops `registration` + `authentication`    | Tasks 1 + 3 |
| Delete old `/registration` and `/authentication` endpoints            | Task 3      |
| `Validate` impl with invite-mode cross-field check                    | Task 1      |
| `#[non_exhaustive]` on new public structs                             | Task 1      |
| `SecretString` for token field                                        | Task 1      |
| OpenAPI client updated                                                | Task 4      |
| CLI `settings access` subcommand                                      | Task 4      |
| Delete old CLI `registration.rs` + `authentication.rs`                | Task 4      |
| Frontend `getAccessSettings` + `updateAccessSettings` (ETag)          | Task 6      |
| Frontend types updated                                                | Task 6      |
| Integration tests for new endpoint                                    | Task 5      |
| `surface-tabs.test.ts` mock updated                                   | Task 11     |
| Documentation (`primitives.md`, `ui/README.md`)                       | Task 12     |

### Dependency version audit

No new external dependencies introduced. All changes are within the existing workspace.

### Post-draft idiom audit

- Svelte 5: all reactive state uses `$state`/`$derived` — no `writable()` stores
- No `let` outside `$state` for reactive values
- `createFormDraft` returns getter-based object — avoids Svelte 5 reactivity pitfalls
- Rust: `emit_stateful` inside tx before commit — consistent with existing audit pattern
- Rust: pre-condition reads before `BEGIN IMMEDIATE` — correct SQLite locking order
