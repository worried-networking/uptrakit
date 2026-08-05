use crate::Result;
use crate::UptrakitClient;
use crate::types_impl::access_grants::{
    AccessGrantResponse, CreateAccessGrantRequest, ListAccessGrantsQuery,
};

impl UptrakitClient {
    /// List grants (active tenant + global rows), optionally one subject's.
    pub async fn list_access_grants(
        &self,
        query: &ListAccessGrantsQuery,
    ) -> Result<Vec<AccessGrantResponse>> {
        self.get_with_query(crate::paths::access_grants::BASE, query)
            .await
    }

    /// Get a single grant.
    pub async fn get_access_grant(&self, id: &crate::Uuid) -> Result<AccessGrantResponse> {
        self.get(&crate::paths::access_grants::by_id(id)).await
    }

    /// Create a grant.
    pub async fn create_access_grant(
        &self,
        req: &CreateAccessGrantRequest,
    ) -> Result<AccessGrantResponse> {
        self.post_json(crate::paths::access_grants::BASE, req).await
    }
}
