use crate::Result;
use crate::UptrakitClient;
use uptrakit_web_api_types::pagination::PaginatedResponse;
use uptrakit_web_api_types::update_history::{UpdateHistoryQuery, UpdateHistoryResponse};
use uuid::Uuid;

impl UptrakitClient {
    /// List update history with optional filters and pagination.
    pub async fn list_update_history(
        &self,
        query: &UpdateHistoryQuery,
    ) -> Result<PaginatedResponse<UpdateHistoryResponse>> {
        self.get_with_query("/api/v1/update-history", query).await
    }

    /// Get a single update history entry by ID.
    pub async fn get_update_history(&self, id: &Uuid) -> Result<UpdateHistoryResponse> {
        let path = format!("/api/v1/update-history/{id}");
        self.get(&path).await
    }
}

#[cfg(test)]
mod tests {
    use uptrakit_web_api_types::update_history::UpdateHistoryQuery;
    use uuid::Uuid;

    #[test]
    fn update_history_query_serialization_with_filters() {
        let host_id = Uuid::parse_str("11111111-1111-1111-1111-111111111111").expect("valid uuid");
        let query = UpdateHistoryQuery {
            host_id: Some(host_id),
            software_item_id: None,
            status: Some("completed".to_string()),
            page: Some(2),
            per_page: Some(10),
        };
        let qs = serde_urlencoded::to_string(&query).expect("serialize");
        assert!(qs.contains("host_id=11111111-1111-1111-1111-111111111111"));
        assert!(qs.contains("status=completed"));
        assert!(qs.contains("page=2"));
        assert!(qs.contains("per_page=10"));
    }

    #[test]
    fn update_history_query_serialization_skips_none() {
        let query = UpdateHistoryQuery {
            host_id: None,
            software_item_id: None,
            status: None,
            page: None,
            per_page: None,
        };
        let qs = serde_urlencoded::to_string(&query).expect("serialize");
        assert!(qs.is_empty());
    }
}
