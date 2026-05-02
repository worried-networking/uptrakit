use rmcp::{ErrorData, Json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uptrakit_shared_db::TenantDb;
use uptrakit_web_api_types::pagination::PaginatedResponse;
use uptrakit_web_api_types::update_history::{UpdateHistoryQuery, UpdateHistoryResponse};
use uuid::Uuid;

use crate::tools::{McpHandler, mcp_error};
use uptrakit_web_api::McpRequestContext;
use uptrakit_web_api::auth::permissions::Permission;
use uptrakit_web_api::queries;

// ---------------------------------------------------------------------------
// Input types
// ---------------------------------------------------------------------------

/// Input parameters for the `list_update_history` MCP tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListUpdateHistoryInput {
    /// Filter by host UUID (optional).
    pub host_id: Option<String>,
    /// Filter by software item UUID (optional).
    pub software_item_id: Option<String>,
    /// Filter by status: queued, pending, in_progress, completed, failed (optional).
    pub status: Option<String>,
    /// Page number (1-indexed, default 1).
    pub page: Option<u64>,
    /// Items per page (default 20, max 1000).
    pub per_page: Option<u64>,
}

/// Input parameters for the `get_update_history_detail` MCP tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetUpdateHistoryDetailInput {
    /// UUID of the update history record.
    pub id: String,
}

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// A single update history item returned by `list_update_history`.
///
/// The `output` field is intentionally omitted — use `get_update_history_detail`
/// to retrieve rendered terminal output for a specific record.
#[derive(Debug, Serialize, JsonSchema)]
pub struct UpdateHistorySummary {
    pub id: String,
    pub host_id: String,
    pub host_name: String,
    pub software_item_id: String,
    pub software_item_name: String,
    pub from_version: Option<String>,
    pub to_version: String,
    pub status: String,
    pub actor_type: String,
    pub actor_id: String,
    pub actor_name: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub created_at: String,
    pub update_category: String,
    pub interactive: bool,
    pub output_truncated: bool,
    pub pre_update_protection_status: Option<String>,
    pub pre_update_protection_summary: Option<String>,
    pub recovery_hint: Option<String>,
}

impl From<UpdateHistoryResponse> for UpdateHistorySummary {
    fn from(r: UpdateHistoryResponse) -> Self {
        Self {
            id: r.id.to_string(),
            host_id: r.host_id.to_string(),
            host_name: r.host_name,
            software_item_id: r.software_item_id.to_string(),
            software_item_name: r.software_item_name,
            from_version: r.from_version,
            to_version: r.to_version,
            status: r.status.to_string(),
            actor_type: r.actor_type,
            actor_id: r.actor_id,
            actor_name: r.actor_name,
            started_at: r.started_at.to_string(),
            completed_at: r.completed_at.map(|t| t.to_string()),
            created_at: r.created_at.to_string(),
            update_category: r.update_category,
            interactive: r.interactive,
            output_truncated: r.output_truncated,
            pre_update_protection_status: r.pre_update_protection_status,
            pre_update_protection_summary: r.pre_update_protection_summary,
            recovery_hint: r.recovery_hint,
        }
    }
}

/// Paginated list of update history summaries.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ListUpdateHistoryResult {
    pub items: Vec<UpdateHistorySummary>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
    pub total_pages: u64,
}

impl From<PaginatedResponse<UpdateHistoryResponse>> for ListUpdateHistoryResult {
    fn from(p: PaginatedResponse<UpdateHistoryResponse>) -> Self {
        Self {
            items: p
                .items
                .into_iter()
                .map(UpdateHistorySummary::from)
                .collect(),
            total: p.total,
            page: p.page,
            per_page: p.per_page,
            total_pages: p.total_pages,
        }
    }
}

/// Detailed update history record with rendered terminal output.
#[derive(Debug, Serialize, JsonSchema)]
pub struct UpdateHistoryDetailResult {
    pub id: String,
    pub host_id: String,
    pub host_name: String,
    pub software_item_id: String,
    pub software_item_name: String,
    pub from_version: Option<String>,
    pub to_version: String,
    pub status: String,
    /// Terminal output with ANSI escape sequences stripped (plain text).
    pub output: String,
    pub actor_type: String,
    pub actor_id: String,
    pub actor_name: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub created_at: String,
    pub update_category: String,
    pub interactive: bool,
    pub output_truncated: bool,
    pub pre_update_protection_status: Option<String>,
    pub pre_update_protection_summary: Option<String>,
    pub recovery_hint: Option<String>,
}

impl UpdateHistoryDetailResult {
    fn from_response_with_output(r: UpdateHistoryResponse, rendered: String) -> Self {
        Self {
            id: r.id.to_string(),
            host_id: r.host_id.to_string(),
            host_name: r.host_name,
            software_item_id: r.software_item_id.to_string(),
            software_item_name: r.software_item_name,
            from_version: r.from_version,
            to_version: r.to_version,
            status: r.status.to_string(),
            output: rendered,
            actor_type: r.actor_type,
            actor_id: r.actor_id,
            actor_name: r.actor_name,
            started_at: r.started_at.to_string(),
            completed_at: r.completed_at.map(|t| t.to_string()),
            created_at: r.created_at.to_string(),
            update_category: r.update_category,
            interactive: r.interactive,
            output_truncated: r.output_truncated,
            pre_update_protection_status: r.pre_update_protection_status,
            pre_update_protection_summary: r.pre_update_protection_summary,
            recovery_hint: r.recovery_hint,
        }
    }
}

// ---------------------------------------------------------------------------
// Implementations
// ---------------------------------------------------------------------------

impl McpHandler {
    /// Core logic for `list_update_history`.
    pub(crate) async fn list_update_history_impl(
        &self,
        ctx: McpRequestContext,
        input: ListUpdateHistoryInput,
    ) -> Result<Json<ListUpdateHistoryResult>, ErrorData> {
        if !ctx.has_permission(&Permission::ViewSoftware) {
            return Err(ErrorData::invalid_request(
                "permission denied: ViewSoftware required",
                None,
            ));
        }

        #[expect(
            clippy::map_err_ignore,
            reason = "uuid::Error and the parse error from `UpdateStatus::FromStr` carry no contextual information beyond what the formatted message conveys"
        )]
        let host_id = input
            .host_id
            .as_deref()
            .map(|s| {
                s.parse::<Uuid>().map_err(|_| {
                    ErrorData::invalid_params(format!("invalid host_id UUID: {s}"), None)
                })
            })
            .transpose()?;

        #[expect(
            clippy::map_err_ignore,
            reason = "uuid::Error carries no contextual information beyond what the formatted message conveys"
        )]
        let software_item_id = input
            .software_item_id
            .as_deref()
            .map(|s| {
                s.parse::<Uuid>().map_err(|_| {
                    ErrorData::invalid_params(format!("invalid software_item_id UUID: {s}"), None)
                })
            })
            .transpose()?;

        #[expect(
            clippy::map_err_ignore,
            reason = "the parse error from `UpdateStatus::FromStr` carries no contextual information beyond what the formatted message conveys"
        )]
        let status = input
            .status
            .as_deref()
            .map(|s| {
                s.parse::<uptrakit_web_api_types::update_history::UpdateStatus>()
                    .map_err(|_| {
                        ErrorData::invalid_params(format!("invalid status value: {s}"), None)
                    })
            })
            .transpose()?;

        let query = UpdateHistoryQuery::new(
            host_id,
            software_item_id,
            status,
            input.page,
            input.per_page,
        );

        let tenant_db = TenantDb::new(self.state.db().clone(), ctx.tenant_id);

        let paginated = queries::update_history::list_update_history(&tenant_db, &query)
            .await
            .map_err(|e| mcp_error(format!("database error: {e}")))?;

        Ok(Json(ListUpdateHistoryResult::from(paginated)))
    }

    /// Core logic for `get_update_history_detail`.
    pub(crate) async fn get_update_history_detail_impl(
        &self,
        ctx: McpRequestContext,
        input: GetUpdateHistoryDetailInput,
    ) -> Result<Json<UpdateHistoryDetailResult>, ErrorData> {
        if !ctx.has_permission(&Permission::ViewSoftware) {
            return Err(ErrorData::invalid_request(
                "permission denied: ViewSoftware required",
                None,
            ));
        }

        #[expect(
            clippy::map_err_ignore,
            reason = "uuid::Error carries no contextual information beyond what the formatted message conveys"
        )]
        let id = input.id.parse::<Uuid>().map_err(|_| {
            ErrorData::invalid_params(format!("invalid id UUID: {}", input.id), None)
        })?;

        let tenant_db = TenantDb::new(self.state.db().clone(), ctx.tenant_id);

        let record = queries::update_history::get_update_history(&tenant_db, id)
            .await
            .map_err(|e| mcp_error(format!("database error: {e}")))?
            .ok_or_else(|| ErrorData::invalid_params("update history record not found", None))?;

        let rendered = crate::terminal::render_terminal_output(record.output.as_bytes());

        Ok(Json(UpdateHistoryDetailResult::from_response_with_output(
            record, rendered,
        )))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_tool_types_exist() {
        // Verify that the input and output types are constructable and serialisable.
        let input = ListUpdateHistoryInput {
            host_id: None,
            software_item_id: None,
            status: None,
            page: Some(1),
            per_page: Some(20),
        };
        assert!(input.page == Some(1));

        let detail_input = GetUpdateHistoryDetailInput {
            id: Uuid::nil().to_string(),
        };
        assert_eq!(detail_input.id, Uuid::nil().to_string());

        let summary = UpdateHistorySummary {
            id: Uuid::nil().to_string(),
            host_id: Uuid::nil().to_string(),
            host_name: "host".to_owned(),
            software_item_id: Uuid::nil().to_string(),
            software_item_name: "pkg".to_owned(),
            from_version: None,
            to_version: "1.0.0".to_owned(),
            status: "completed".to_owned(),
            actor_type: "user".to_owned(),
            actor_id: Uuid::nil().to_string(),
            actor_name: Some("Alice".to_owned()),
            started_at: "2024-01-01T00:00:00Z".to_owned(),
            completed_at: None,
            created_at: "2024-01-01T00:00:00Z".to_owned(),
            update_category: "bugfix".to_owned(),
            interactive: false,
            output_truncated: false,
            pre_update_protection_status: None,
            pre_update_protection_summary: None,
            recovery_hint: None,
        };
        let json = serde_json::to_string(&summary).expect("serialisation must succeed");
        assert!(json.contains("completed"));
        // The raw terminal output field must not appear in list summaries;
        // only output_truncated (a bool flag) is allowed.
        assert!(
            !json.contains("\"output\""),
            "raw output field must not appear in list summary"
        );
    }
}
