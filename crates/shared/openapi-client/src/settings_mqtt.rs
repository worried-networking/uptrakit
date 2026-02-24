use crate::Result;
use crate::UptrakitClient;
use uptrakit_web_api_types::settings_mqtt::{
    CreateMqttClientRequest, MqttClientResponse, MqttLimitResponse, UpdateMqttClientRequest,
    UpdateMqttLimitRequest,
};
use uuid::Uuid;

impl UptrakitClient {
    /// List all MQTT client configurations.
    pub async fn list_mqtt_settings(&self) -> Result<Vec<MqttClientResponse>> {
        self.get(crate::paths::settings_mqtt::BASE).await
    }

    /// Create a new MQTT client configuration.
    pub async fn create_mqtt_settings(
        &self,
        req: &CreateMqttClientRequest,
    ) -> Result<MqttClientResponse> {
        self.post_json(crate::paths::settings_mqtt::BASE, req).await
    }

    /// Get the MQTT client limit.
    pub async fn get_mqtt_limit(&self) -> Result<MqttLimitResponse> {
        self.get(crate::paths::settings_mqtt::LIMIT).await
    }

    /// Update the MQTT client limit.
    pub async fn update_mqtt_limit(
        &self,
        req: &UpdateMqttLimitRequest,
    ) -> Result<MqttLimitResponse> {
        self.put_json(crate::paths::settings_mqtt::LIMIT, req).await
    }

    /// Get a single MQTT client configuration by ID.
    pub async fn get_mqtt_settings(&self, id: &Uuid) -> Result<MqttClientResponse> {
        self.get(&crate::paths::settings_mqtt::by_id(id)).await
    }

    /// Update an existing MQTT client configuration.
    pub async fn update_mqtt_settings(
        &self,
        id: &Uuid,
        req: &UpdateMqttClientRequest,
    ) -> Result<MqttClientResponse> {
        self.put_json(&crate::paths::settings_mqtt::by_id(id), req)
            .await
    }

    /// Delete an MQTT client configuration.
    pub async fn delete_mqtt_settings(&self, id: &Uuid) -> Result<()> {
        self.delete(&crate::paths::settings_mqtt::by_id(id)).await
    }
}

#[cfg(test)]
mod tests {
    use uptrakit_web_api_types::settings_mqtt::{
        CreateMqttClientRequest, UpdateMqttClientRequest, UpdateMqttLimitRequest,
    };

    #[test]
    fn create_mqtt_client_request_serialization() {
        let req = CreateMqttClientRequest {
            url: Some("mqtt://broker:1883".to_string()),
            transport: None,
            host: None,
            port: None,
            enabled: Some(true),
            client_id: Some("uptrakit-1".to_string()),
            username: None,
            password: None,
            ca_pem: None,
            topic_prefix: Some("uptrakit".to_string()),
            ha_discovery: Some(true),
            ha_discovery_prefix: Some("homeassistant".to_string()),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["url"], "mqtt://broker:1883");
        assert_eq!(json["enabled"], true);
        assert_eq!(json["client_id"], "uptrakit-1");
        assert_eq!(json["topic_prefix"], "uptrakit");
        assert_eq!(json["ha_discovery"], true);
        assert_eq!(json["ha_discovery_prefix"], "homeassistant");
    }

    #[test]
    fn update_mqtt_client_request_serialization() {
        let req = UpdateMqttClientRequest {
            url: None,
            transport: None,
            host: Some("new-broker".to_string()),
            port: Some(8883),
            enabled: Some(false),
            client_id: None,
            username: None,
            password: None,
            ca_pem: None,
            topic_prefix: None,
            ha_discovery: None,
            ha_discovery_prefix: None,
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["host"], "new-broker");
        assert_eq!(json["port"], 8883);
        assert_eq!(json["enabled"], false);
    }

    #[test]
    fn update_mqtt_limit_request_serialization() {
        let req = UpdateMqttLimitRequest {
            max_clients_per_tenant: 5,
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["max_clients_per_tenant"], 5);
    }
}
