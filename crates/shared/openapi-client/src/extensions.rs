use crate::Result;
use crate::UptrakitClient;
use uptrakit_shared_types::SecretString;
use uptrakit_web_api_types::extensions::{
    ExtensionProviderInfo, ExtensionResponse, InvokeExtensionActionRequest,
};
use uuid::Uuid;

impl UptrakitClient {
    /// List all registered extensions across connected services.
    pub async fn list_extensions(&self) -> Result<Vec<ExtensionResponse>> {
        self.get(crate::paths::extensions::BASE).await
    }

    /// List service instances that provide a specific extension.
    pub async fn list_extension_providers(
        &self,
        extension_id: &str,
    ) -> Result<Vec<ExtensionProviderInfo>> {
        self.get(&crate::paths::extensions::providers(extension_id))
            .await
    }

    /// Invoke an action on an extension, optionally targeting a specific service.
    ///
    /// `params` is the arbitrary JSON payload forwarded to the extension handler.
    /// `sensitive_params` is ECIES-encrypted ciphertext (base64) passed through
    /// opaquely by the controller.
    /// When `service_id` is `None`, the server selects a provider automatically.
    pub async fn invoke_extension_action(
        &self,
        extension_id: &str,
        action_id: &str,
        params: serde_json::Value,
        sensitive_params: Option<SecretString>,
        service_id: Option<&Uuid>,
    ) -> Result<serde_json::Value> {
        let path = crate::paths::extensions::action(extension_id, action_id);
        let url = format!("{}{}", self.base_url, path);

        let body = InvokeExtensionActionRequest {
            params,
            sensitive_params,
        };

        let mut req = self
            .http
            .post(&url)
            .bearer_auth(self.token_or_err()?)
            .json(&body);

        if let Some(sid) = service_id {
            req = req.query(&[("service_id", sid.to_string())]);
        }

        let resp = self.send_with_retry(req).await?;
        self.handle_response(resp).await
    }
}
