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
        self.get_with_query(crate::paths::update_history::BASE, query)
            .await
    }

    /// Fetch all update history entries matching the given filters across all pages.
    ///
    /// Automatically iterates through every page at [`MAX_PER_PAGE`] items per
    /// request. The `page` and `per_page` fields of `query` are ignored; use
    /// [`list_update_history`] for manual pagination control.
    ///
    /// [`MAX_PER_PAGE`]: uptrakit_web_api_types::pagination::MAX_PER_PAGE
    /// [`list_update_history`]: Self::list_update_history
    pub async fn list_all_update_history(
        &self,
        query: &UpdateHistoryQuery,
    ) -> Result<Vec<UpdateHistoryResponse>> {
        self.fetch_all_pages(crate::paths::update_history::BASE, query)
            .await
    }

    /// Get a single update history entry by ID.
    pub async fn get_update_history(&self, id: &Uuid) -> Result<UpdateHistoryResponse> {
        self.get(&crate::paths::update_history::by_id(id)).await
    }
}

#[cfg(test)]
mod tests {
    use uptrakit_web_api_types::update_history::{UpdateHistoryQuery, UpdateStatus};
    use uuid::Uuid;

    #[test]
    fn update_history_query_serialization_with_filters() {
        let host_id = Uuid::parse_str("11111111-1111-1111-1111-111111111111").expect("valid uuid");
        let query = UpdateHistoryQuery::new(
            Some(host_id),
            None,
            Some(UpdateStatus::Completed),
            Some(2),
            Some(10),
        );
        let qs = serde_urlencoded::to_string(&query).expect("serialize");
        assert!(qs.contains("host_id=11111111-1111-1111-1111-111111111111"));
        assert!(qs.contains("status=completed"));
        assert!(qs.contains("page=2"));
        assert!(qs.contains("per_page=10"));
    }

    #[test]
    fn update_history_query_serialization_skips_none() {
        let query = UpdateHistoryQuery::new(None, None, None, None, None);
        let qs = serde_urlencoded::to_string(&query).expect("serialize");
        assert!(qs.is_empty());
    }
}
