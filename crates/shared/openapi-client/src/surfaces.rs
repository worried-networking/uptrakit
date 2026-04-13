use crate::Result;
use crate::UptrakitClient;
use uptrakit_web_api_types::surfaces::{
    InvokeSurfaceInteractionRequest, SurfaceProviderInfo, SurfaceResponse,
};

impl UptrakitClient {
    /// List registered surfaces. Optional `slot` and `page` filters map to query params.
    pub async fn list_surfaces(
        &self,
        slot: Option<&str>,
        page: Option<&str>,
    ) -> Result<Vec<SurfaceResponse>> {
        let url = format!("{}{}", self.base_url, crate::paths::surfaces::BASE);
        let mut req = self.http.get(&url).bearer_auth(self.token_or_err()?);
        let mut query_params: Vec<(&str, &str)> = Vec::new();
        if let Some(slot) = slot {
            query_params.push(("slot", slot));
        }
        if let Some(page) = page {
            query_params.push(("page", page));
        }
        if !query_params.is_empty() {
            req = req.query(&query_params);
        }
        let resp = self.send_with_retry(req).await?;
        self.handle_response(resp).await
    }

    /// List targeted providers for a surface.
    pub async fn list_surface_providers(
        &self,
        surface_id: &str,
    ) -> Result<Vec<SurfaceProviderInfo>> {
        self.get(&crate::paths::surfaces::providers(surface_id))
            .await
    }

    /// Invoke a surface interaction.
    pub async fn invoke_surface_interaction(
        &self,
        surface_id: &str,
        interaction_id: &str,
        request: &InvokeSurfaceInteractionRequest,
    ) -> Result<serde_json::Value> {
        self.post_json(
            &crate::paths::surfaces::interaction(surface_id, interaction_id),
            request,
        )
        .await
    }
}
