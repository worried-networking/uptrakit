use crate::Result;
use crate::UptrakitClient;
use uptrakit_web_api_types::audit_logs::{
    AuditLogListParams, AuditLogResponse, SystemAuditLogResponse,
};
use uptrakit_web_api_types::pagination::PaginatedResponse;

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
    use uptrakit_web_api_types::audit_logs::AuditLogListParams;
    use uuid::Uuid;

    #[test]
    fn audit_log_list_params_serialization_with_filters() {
        let actor_id = Uuid::parse_str("11111111-1111-1111-1111-111111111111").expect("valid uuid");
        let params = AuditLogListParams {
            actor_type: Some("user".to_string()),
            method: Some("GET".to_string()),
            status: Some(200),
            from: Some("2025-01-01T00:00:00Z".to_string()),
            to: Some("2025-12-31T23:59:59Z".to_string()),
            actor_id: Some(actor_id),
            page: Some(2),
            per_page: Some(10),
        };
        let qs = serde_urlencoded::to_string(&params).expect("serialize");
        assert!(qs.contains("actor_type=user"));
        assert!(qs.contains("method=GET"));
        assert!(qs.contains("status=200"));
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
