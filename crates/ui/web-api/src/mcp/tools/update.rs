use std::sync::Arc;

use rmcp::{ErrorData, Json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uptrakit_web_api_types::software_items::TriggerUpdateStatus;
use uuid::Uuid;

use crate::auth::permissions::Permission;
use crate::mcp::auth::McpRequestContext;
use crate::mcp::tools::{McpHandler, mcp_error};
use crate::middleware::require_auth::{AuthenticatedApiTokenId, AuthenticatedUser};
use crate::queries::update_triggers::TriggerUpdateParams;
use crate::queries::update_types::ActorType;

// ---------------------------------------------------------------------------
// Input / output types
// ---------------------------------------------------------------------------

/// Input parameters for the `trigger_update` MCP tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TriggerUpdateInput {
    /// UUID of the host to update.
    pub host_id: String,
    /// UUID of the software item to update.
    pub software_item_id: String,
    /// Target version string (e.g. `"1.2.3"`).
    pub to_version: String,
}

/// Result returned by the `trigger_update` MCP tool.
#[derive(Debug, Serialize, JsonSchema)]
pub struct TriggerUpdateResult {
    /// UUID of the created update history record.
    pub update_history_id: String,
    /// Dispatch status: `"pending"`, `"queued"`, or `"failed"`.
    pub status: String,
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl McpHandler {
    /// Core logic for `trigger_update`.
    pub(crate) async fn trigger_update_impl(
        &self,
        ctx: McpRequestContext,
        input: TriggerUpdateInput,
    ) -> Result<Json<TriggerUpdateResult>, ErrorData> {
        if !ctx.has_permission(&Permission::TriggerUpdates) {
            return Err(ErrorData::invalid_request(
                "permission denied: TriggerUpdates required",
                None,
            ));
        }

        let host_id = input.host_id.parse::<Uuid>().map_err(|_| {
            ErrorData::invalid_params(format!("invalid host_id UUID: {}", input.host_id), None)
        })?;

        let software_item_id = input.software_item_id.parse::<Uuid>().map_err(|_| {
            ErrorData::invalid_params(
                format!("invalid software_item_id UUID: {}", input.software_item_id),
                None,
            )
        })?;

        let actor_id_str = ctx.token_id.to_string();

        let state = Arc::clone(&self.state);
        let tenant_db = crate::tenant_db::TenantDb(uptrakit_shared_db::TenantDb::new(
            state.db().clone(),
            ctx.tenant_id,
        ));
        let mut_ctx = state.mutation_context();

        // Construct helpers for audit emission.
        let audit_user = AuthenticatedUser {
            user_id: ctx.user_id,
            auth_method: crate::auth::AuthMethod::ApiToken,
            permissions: ctx.permissions.clone(),
            jti: None,
        };
        let audit_token = AuthenticatedApiTokenId(ctx.token_id);

        let result = crate::actions::software_items::trigger_update(
            &tenant_db,
            &mut_ctx,
            TriggerUpdateParams {
                tenant_id: ctx.tenant_id,
                item_id: software_item_id,
                host_id,
                to_version: input.to_version.clone(),
                actor_type: ActorType::ApiToken.as_str(),
                actor_id: &actor_id_str,
                release_info: None,
                interactive: false,
            },
        )
        .await;

        let result = match result {
            Ok(r) => r,
            Err(err) => {
                let (outcome, reason_code) = err.current_context().trigger_audit_classification();
                crate::routes::software_items::emit_software_update_audit(
                    &state,
                    ctx.tenant_id,
                    &audit_user,
                    Some(audit_token),
                    software_item_id,
                    outcome,
                    serde_json::json!({
                        "host_id": host_id,
                        "to_version": input.to_version,
                        "interactive": false,
                        "reason_code": reason_code,
                    }),
                );
                return Err(mcp_error(format!("trigger_update failed: {err}")));
            }
        };

        if let Some(work) = result.pending_protection_work {
            crate::update_orchestrator::spawn_protection_and_dispatch(Arc::clone(&state), *work);
        }

        let status = match result.initial_status {
            uptrakit_shared_db::entity::update_history::UpdateStatus::Pending => {
                TriggerUpdateStatus::Pending
            }
            uptrakit_shared_db::entity::update_history::UpdateStatus::Failed => {
                TriggerUpdateStatus::Failed
            }
            _ => TriggerUpdateStatus::Queued,
        };

        let audit_outcome = if matches!(status, TriggerUpdateStatus::Failed) {
            uptrakit_audit_log::AuditOutcome::Failed
        } else {
            uptrakit_audit_log::AuditOutcome::Success
        };

        crate::routes::software_items::emit_software_update_audit(
            &state,
            ctx.tenant_id,
            &audit_user,
            Some(audit_token),
            software_item_id,
            audit_outcome,
            serde_json::json!({
                "host_id": host_id,
                "to_version": input.to_version,
                "interactive": false,
                "update_history_id": result.update_history_id,
                "dispatch_status": status.to_string(),
            }),
        );

        Ok(Json(TriggerUpdateResult {
            update_history_id: result.update_history_id.to_string(),
            status: status.to_string(),
        }))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_update_input_type_exists() {
        let input = TriggerUpdateInput {
            host_id: Uuid::nil().to_string(),
            software_item_id: Uuid::nil().to_string(),
            to_version: "1.0.0".to_owned(),
        };
        assert_eq!(input.to_version, "1.0.0");

        let result = TriggerUpdateResult {
            update_history_id: Uuid::nil().to_string(),
            status: "queued".to_owned(),
        };
        let json = serde_json::to_string(&result).expect("serialisation must succeed");
        assert!(json.contains("queued"));
    }
}
