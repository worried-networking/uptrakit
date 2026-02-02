use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MqttSettingsResponse {
    pub host: Option<String>,
    pub port: u16,
    pub client_id: String,
    pub username: Option<String>,
    pub has_password: bool,
    pub topic_prefix: String,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateMqttSettingsRequest {
    pub host: Option<serde_json::Value>,
    pub port: Option<u16>,
    pub client_id: Option<String>,
    pub username: Option<serde_json::Value>,
    pub password: Option<String>,
    pub topic_prefix: Option<String>,
}
