use rmcp::{ErrorData, Json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::mcp::auth::McpRequestContext;
use crate::mcp::tools::{McpHandler, mcp_error};
use crate::mcp_trigger_update;

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
        use crate::auth::permissions::Permission;
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

        let (update_history_id, status) = mcp_trigger_update(
            std::sync::Arc::clone(&self.state),
            &ctx,
            host_id,
            software_item_id,
            input.to_version.clone(),
        )
        .await
        .map_err(|err| mcp_error(format!("trigger_update failed: {err}")))?;

        Ok(Json(TriggerUpdateResult {
            update_history_id: update_history_id.to_string(),
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
    use crate::McpTriggerError;

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
}
