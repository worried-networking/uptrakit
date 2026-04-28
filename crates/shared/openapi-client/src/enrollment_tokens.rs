use crate::Result;
use crate::UptrakitClient;
use crate::types_impl::enrollment_tokens::{
    CreateEnrollmentTokenRequest, EnrollmentTokenCreatedResponse, EnrollmentTokenResponse,
    ListEnrollmentTokensQuery,
};
use crate::types_impl::pagination::PaginatedResponse;
use uuid::Uuid;

impl UptrakitClient {
    /// Create a new enrollment token.
    pub async fn create_enrollment_token(
        &self,
        req: &CreateEnrollmentTokenRequest,
    ) -> Result<EnrollmentTokenCreatedResponse> {
        self.post_json(crate::paths::enrollment_tokens::BASE, req)
            .await
    }

    /// List enrollment tokens with pagination.
    pub async fn list_enrollment_tokens(
        &self,
        query: &ListEnrollmentTokensQuery,
    ) -> Result<PaginatedResponse<EnrollmentTokenResponse>> {
        self.get_with_query(crate::paths::enrollment_tokens::BASE, query)
            .await
    }

    /// Fetch all enrollment tokens across all pages.
    pub async fn list_all_enrollment_tokens(
        &self,
        query: &ListEnrollmentTokensQuery,
    ) -> Result<Vec<EnrollmentTokenResponse>> {
        self.fetch_all_pages(crate::paths::enrollment_tokens::BASE, query)
            .await
    }

    /// Get a single enrollment token by ID.
    pub async fn get_enrollment_token(&self, id: &Uuid) -> Result<EnrollmentTokenResponse> {
        self.get(&crate::paths::enrollment_tokens::by_id(id)).await
    }

    /// Revoke an enrollment token (soft-delete).
    pub async fn revoke_enrollment_token(&self, id: &Uuid) -> Result<()> {
        self.delete(&crate::paths::enrollment_tokens::by_id(id))
            .await
    }
}

#[cfg(test)]
mod tests {
    use crate::types_impl::enrollment_tokens::{
        CreateEnrollmentTokenRequest, ListEnrollmentTokensQuery,
    };

    #[test]
    fn create_request_serialization() {
        let req = CreateEnrollmentTokenRequest {
            name: "CI Token".to_string(),
            allowed_capabilities: Some(vec!["software_discovery".to_string()]),
            max_uses: Some(10),
            expires_in_seconds: Some(3600),
        };
        let json = serde_json::to_string(&req).expect("serialize");
        assert!(json.contains("CI Token"));
        assert!(json.contains("software_discovery"));
        let parsed: CreateEnrollmentTokenRequest =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.name, "CI Token");
        assert_eq!(parsed.max_uses, Some(10));
    }

    #[test]
    fn list_query_serialization_with_params() {
        let query = ListEnrollmentTokensQuery {
            page: Some(2),
            per_page: Some(25),
        };
        let qs = serde_urlencoded::to_string(&query).expect("serialize");
        assert!(qs.contains("page=2"));
        assert!(qs.contains("per_page=25"));
    }

    #[test]
    fn list_query_serialization_empty() {
        let query = ListEnrollmentTokensQuery {
            page: None,
            per_page: None,
        };
        let qs = serde_urlencoded::to_string(&query).expect("serialize");
        assert!(qs.is_empty());
    }
}
