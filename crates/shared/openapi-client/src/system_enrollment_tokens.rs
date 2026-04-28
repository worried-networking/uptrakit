use crate::Result;
use crate::UptrakitClient;
use crate::generated::types::pagination::PaginatedResponse;
use crate::generated::types::system_enrollment_tokens::{
    CreateSystemEnrollmentTokenRequest, ListSystemEnrollmentTokensQuery,
    SystemEnrollmentTokenCreatedResponse, SystemEnrollmentTokenResponse,
};
use uuid::Uuid;

impl UptrakitClient {
    /// Create a new system enrollment token.
    ///
    /// The plaintext token is only returned in this response — store it
    /// securely immediately after creation.
    pub async fn create_system_enrollment_token(
        &self,
        req: &CreateSystemEnrollmentTokenRequest,
    ) -> Result<SystemEnrollmentTokenCreatedResponse> {
        self.post_json(crate::paths::system_enrollment_tokens::BASE, req)
            .await
    }

    /// List system enrollment tokens with pagination.
    pub async fn list_system_enrollment_tokens(
        &self,
        query: &ListSystemEnrollmentTokensQuery,
    ) -> Result<PaginatedResponse<SystemEnrollmentTokenResponse>> {
        self.get_with_query(crate::paths::system_enrollment_tokens::BASE, query)
            .await
    }

    /// Fetch all system enrollment tokens across all pages.
    pub async fn list_all_system_enrollment_tokens(
        &self,
        query: &ListSystemEnrollmentTokensQuery,
    ) -> Result<Vec<SystemEnrollmentTokenResponse>> {
        self.fetch_all_pages(crate::paths::system_enrollment_tokens::BASE, query)
            .await
    }

    /// Get a single system enrollment token by ID.
    pub async fn get_system_enrollment_token(
        &self,
        id: &Uuid,
    ) -> Result<SystemEnrollmentTokenResponse> {
        self.get(&crate::paths::system_enrollment_tokens::by_id(id))
            .await
    }

    /// Revoke a system enrollment token (soft-delete).
    pub async fn revoke_system_enrollment_token(&self, id: &Uuid) -> Result<()> {
        self.delete(&crate::paths::system_enrollment_tokens::by_id(id))
            .await
    }
}

#[cfg(test)]
mod tests {
    use crate::generated::types::system_enrollment_tokens::{
        CreateSystemEnrollmentTokenRequest, ListSystemEnrollmentTokensQuery,
    };

    #[test]
    fn create_request_serialization() {
        let req = CreateSystemEnrollmentTokenRequest {
            name: "MQTT Bridge Token".to_string(),
            max_uses: Some(5),
            expires_in_seconds: Some(86400),
        };
        let json = serde_json::to_string(&req).expect("serialize");
        assert!(json.contains("MQTT Bridge Token"));
        let parsed: CreateSystemEnrollmentTokenRequest =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.name, "MQTT Bridge Token");
        assert_eq!(parsed.max_uses, Some(5));
        assert_eq!(parsed.expires_in_seconds, Some(86400));
    }

    #[test]
    fn list_query_serialization_with_params() {
        let query = ListSystemEnrollmentTokensQuery {
            page: Some(2),
            per_page: Some(25),
        };
        let qs = serde_urlencoded::to_string(&query).expect("serialize");
        assert!(qs.contains("page=2"));
        assert!(qs.contains("per_page=25"));
    }

    #[test]
    fn list_query_serialization_empty() {
        let query = ListSystemEnrollmentTokensQuery {
            page: None,
            per_page: None,
        };
        let qs = serde_urlencoded::to_string(&query).expect("serialize");
        assert!(qs.is_empty());
    }
}
