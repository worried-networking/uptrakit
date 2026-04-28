use crate::Result;
use crate::UptrakitClient;
use crate::generated::types::audit_logs::{
    AuditLogListParams, AuditLogResponse, SystemAuditLogResponse,
};
use crate::generated::types::pagination::PaginatedResponse;

impl UptrakitClient {
    /// List tenant-scoped audit log entries with optional filters and pagination.
    pub async fn list_audit_logs(
        &self,
        params: &AuditLogListParams,
    ) -> Result<PaginatedResponse<AuditLogResponse>> {
        self.get_with_query(crate::paths::audit_logs::BASE, params)
            .await
    }

    /// List system-level audit log entries with optional filters and pagination.
    pub async fn list_system_audit_logs(
        &self,
        params: &AuditLogListParams,
    ) -> Result<PaginatedResponse<SystemAuditLogResponse>> {
        self.get_with_query(crate::paths::audit_logs::SYSTEM, params)
            .await
    }
}

#[cfg(test)]
mod tests {
    use crate::generated::types::audit_logs::AuditLogListParams;
    use uuid::Uuid;

    #[test]
    fn audit_log_list_params_serialization_with_semantic_filters() {
        let actor_id = Uuid::parse_str("11111111-1111-1111-1111-111111111111").expect("valid uuid");
        let params = AuditLogListParams {
            actor_type: Some("user".to_string()),
            action_type: Some("plugin_config.create".to_string()),
            outcome: Some("success".to_string()),
            target_type: Some("plugin_config".to_string()),
            target_id: Some("019semantic".to_string()),
            from: Some("2025-01-01T00:00:00Z".to_string()),
            to: Some("2025-12-31T23:59:59Z".to_string()),
            actor_id: Some(actor_id),
            page: Some(2),
            per_page: Some(10),
        };
        let qs = serde_urlencoded::to_string(&params).expect("serialize");
        assert!(qs.contains("actor_type=user"));
        assert!(qs.contains("action_type=plugin_config.create"));
        assert!(qs.contains("outcome=success"));
        assert!(qs.contains("target_type=plugin_config"));
        assert!(qs.contains("target_id=019semantic"));
        assert!(qs.contains("page=2"));
        assert!(qs.contains("per_page=10"));
        assert!(qs.contains("actor_id=11111111-1111-1111-1111-111111111111"));
    }

    #[test]
    fn audit_log_list_params_serialization_skips_none() {
        let params = AuditLogListParams::default();
        let qs = serde_urlencoded::to_string(&params).expect("serialize");
        assert!(qs.is_empty());
    }
}
