use rmcp::{ErrorData, Json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use uptrakit_controller_core::auth::Permission;
use uptrakit_controller_core::update::{ActorInfo, DispatchOutcome, UpdateDispatchParams};
use uptrakit_web_api_queries::queries::update_types::ActorType;
use uptrakit_web_api_types::oauth::McpScope;

use crate::context::{McpRequestContext, McpTriggerError};
use crate::oauth::tool_auth::{ToolAuth, require_scopes};
use crate::state::McpState;
use crate::tools::{McpHandler, mcp_error};

pub(crate) const TRIGGER_UPDATE_AUTH: ToolAuth = ToolAuth {
    required_scopes: &[McpScope::Write],
    required_permissions: &[Permission::TriggerUpdates],
};

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
        require_scopes(&ctx, TRIGGER_UPDATE_AUTH.required_scopes)
            .map_err(|e| ErrorData::invalid_request(format!("insufficient_scope: {e}"), None))?;

        if !ctx.has_permission(&Permission::TriggerUpdates) {
            return Err(ErrorData::invalid_request(
                "permission denied: TriggerUpdates required",
                None,
            ));
        }

        #[expect(
            clippy::map_err_ignore,
            reason = "uuid::Error carries no contextual information beyond what the formatted message conveys"
        )]
        let host_id = input.host_id.parse::<Uuid>().map_err(|_| {
            ErrorData::invalid_params(format!("invalid host_id UUID: {}", input.host_id), None)
        })?;

        #[expect(
            clippy::map_err_ignore,
            reason = "uuid::Error carries no contextual information beyond what the formatted message conveys"
        )]
        let software_item_id = input.software_item_id.parse::<Uuid>().map_err(|_| {
            ErrorData::invalid_params(
                format!("invalid software_item_id UUID: {}", input.software_item_id),
                None,
            )
        })?;

        let (update_history_id, outcome) = mcp_trigger_update(
            &self.state,
            &ctx,
            host_id,
            software_item_id,
            input.to_version,
        )
        .await
        .map_err(|err| mcp_error(format!("trigger_update failed: {err}")))?;

        let status = dispatch_outcome_to_str(&outcome);

        Ok(Json(TriggerUpdateResult {
            update_history_id: update_history_id.to_string(),
            status: status.to_owned(),
        }))
    }
}

/// Convert a [`DispatchOutcome`] to the string status exposed by the MCP tool.
///
/// `DispatchOutcome` is `#[non_exhaustive]`; the wildcard arm logs a warning
/// and returns `"failed"` so new variants are handled safely.
fn dispatch_outcome_to_str(outcome: &DispatchOutcome) -> &'static str {
    match outcome {
        DispatchOutcome::Sent => "pending",
        DispatchOutcome::Queued => "queued",
        DispatchOutcome::Failed => "failed",
        _ => {
            tracing::warn!("unhandled DispatchOutcome variant; reporting status as \"failed\"");
            "failed"
        }
    }
}

// ---------------------------------------------------------------------------
// McpState-based trigger
// ---------------------------------------------------------------------------

/// Trigger a software update from an MCP tool call.
///
/// Uses `McpState.update_dispatcher` — no `AppState` required.
/// Returns `(update_history_id, DispatchOutcome)` on success.
///
/// # Errors
///
/// Returns [`McpTriggerError`] if the host is not found, the software item is
/// not found, an update is already active, the host is not configured,
/// the agent is unavailable, or an internal error occurs.
pub async fn mcp_trigger_update(
    state: &McpState,
    ctx: &McpRequestContext,
    host_id: Uuid,
    software_item_id: Uuid,
    to_version: String,
) -> Result<(Uuid, DispatchOutcome), McpTriggerError> {
    let params = UpdateDispatchParams::new(
        ctx.tenant_id,
        host_id,
        software_item_id,
        to_version,
        ActorInfo::new(ActorType::ApiToken, ctx.token_id.to_string()),
        None,
        false,
    );

    state
        .update_dispatcher
        .dispatch(params)
        .await
        .map(|r| (r.update_history_id, r.outcome))
        .map_err(|e| McpTriggerError::from(e.current_context()))
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

    #[test]
    fn mcp_trigger_error_variants_exist() {
        // Compile-time check that the variants named in the spec are present.
        let _ = McpTriggerError::PermissionDenied;
        let _ = McpTriggerError::HostNotFound;
        let _ = McpTriggerError::SoftwareItemNotFound;
        let _ = McpTriggerError::NotConfigured;
        let _ = McpTriggerError::AgentUnavailable;
        let _ = McpTriggerError::AlreadyInProgress;
        let _ = McpTriggerError::Internal;
    }

    #[test]
    fn dispatch_outcome_sent_maps_to_pending() {
        assert_eq!(dispatch_outcome_to_str(&DispatchOutcome::Sent), "pending");
    }

    #[test]
    fn dispatch_outcome_queued_maps_to_queued() {
        assert_eq!(dispatch_outcome_to_str(&DispatchOutcome::Queued), "queued");
    }

    #[test]
    fn dispatch_outcome_failed_maps_to_failed() {
        assert_eq!(dispatch_outcome_to_str(&DispatchOutcome::Failed), "failed");
    }
}
