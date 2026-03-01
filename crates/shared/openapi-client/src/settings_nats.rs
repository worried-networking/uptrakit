use crate::Result;
use crate::UptrakitClient;
use uptrakit_web_api_types::settings_nats::{NatsSettingsResponse, UpdateNatsSettingsRequest};

impl UptrakitClient {
    /// Get the global NATS server URL settings.
    pub async fn get_nats_settings(&self) -> Result<NatsSettingsResponse> {
        self.get(crate::paths::settings_nats::BASE).await
    }

    /// Update the global NATS server URL settings.
    ///
    /// Changes take effect after the controller is restarted.
    pub async fn update_nats_settings(
        &self,
        req: &UpdateNatsSettingsRequest,
    ) -> Result<NatsSettingsResponse> {
        self.put_json(crate::paths::settings_nats::BASE, req).await
    }
}

#[cfg(test)]
mod tests {
    use uptrakit_web_api_types::settings_nats::UpdateNatsSettingsRequest;

    #[test]
    fn update_nats_request_with_url_serializes() {
        let req = UpdateNatsSettingsRequest {
            url: Some(serde_json::json!("nats://host:4222")),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["url"], "nats://host:4222");
    }

    #[test]
    fn update_nats_request_clear_url_serializes_null() {
        let req = UpdateNatsSettingsRequest {
            url: Some(serde_json::Value::Null),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert!(json["url"].is_null());
    }

    #[test]
    fn update_nats_request_no_url_omits_field() {
        let req = UpdateNatsSettingsRequest { url: None };
        let json = serde_json::to_value(&req).expect("serialize");
        assert!(json.get("url").is_none() || json["url"].is_null());
    }
}
