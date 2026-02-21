use crate::Result;
use crate::UptrakitClient;
use uptrakit_web_api_types::api_tokens::{
    ApiTokenListResponse, CreateApiTokenRequest, CreateApiTokenResponse,
};
use uuid::Uuid;

impl UptrakitClient {
    /// Create a new API token.
    pub async fn create_api_token(
        &self,
        req: &CreateApiTokenRequest,
    ) -> Result<CreateApiTokenResponse> {
        self.post_json(crate::paths::api_tokens::BASE, req).await
    }

    /// List all API tokens for the current user.
    pub async fn list_api_tokens(&self) -> Result<ApiTokenListResponse> {
        self.get(crate::paths::api_tokens::BASE).await
    }

    /// Revoke an API token by ID.
    pub async fn revoke_api_token(&self, id: &Uuid) -> Result<()> {
        self.delete(&crate::paths::api_tokens::by_id(id)).await
    }
}

#[cfg(test)]
mod tests {
    use uptrakit_web_api_types::api_tokens::CreateApiTokenRequest;

    #[test]
    fn create_api_token_request_serialization() {
        let req = CreateApiTokenRequest {
            name: "my-token".to_string(),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["name"], "my-token");
    }
}
