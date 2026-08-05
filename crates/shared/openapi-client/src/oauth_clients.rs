//! Operator OAuth client management (`/api/oauth/clients`).

use crate::Result;
use crate::UptrakitClient;
use crate::types_impl::oauth::{
    DcrRegistrationRequest, DcrRegistrationResponse, OAuthClientResponse,
};
use crate::types_impl::pagination::{PaginatedResponse, PaginationParams};

impl UptrakitClient {
    /// List registered OAuth clients (paginated, newest first).
    pub async fn list_clients(
        &self,
        params: &PaginationParams,
    ) -> Result<PaginatedResponse<OAuthClientResponse>> {
        self.get_with_query(crate::paths::oauth::CLIENTS, params)
            .await
    }

    /// Manually register an OAuth client (operator, RFC 7591 shape).
    ///
    /// The response carries a one-time `registration_access_token` — never
    /// log or `Debug`-format it.
    pub async fn manual_register_client(
        &self,
        req: &DcrRegistrationRequest,
    ) -> Result<DcrRegistrationResponse> {
        self.post_json(crate::paths::oauth::CLIENTS, req).await
    }

    /// Revoke an OAuth client (cascades to its consents and refresh tokens).
    pub async fn revoke_client(&self, client_id: &str) -> Result<()> {
        self.delete(&crate::paths::oauth::client_by_id(client_id))
            .await
    }

    /// Promote an OAuth client to trusted.
    pub async fn trust_client(&self, client_id: &str) -> Result<()> {
        self.post_empty_no_content(&crate::paths::oauth::client_trust(client_id))
            .await
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn client_by_id_encodes_url_shaped_ids() {
        let path = crate::paths::oauth::client_by_id("https://example.com/client.json");
        let rest = path
            .strip_prefix("/api/oauth/clients/")
            .expect("path must start with the clients prefix");
        assert!(
            !rest.contains('/'),
            "encoded id must be a single path segment: {path}"
        );
    }
}
