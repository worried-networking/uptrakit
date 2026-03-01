use serde::{Deserialize, Serialize};
use uptrakit_shared_types::SecretString;
use uuid::Uuid;

use crate::mqtt_transport::MqttTransport;
use crate::validation::{Validate, ValidationError};

// Re-export from shared-types for backward compatibility.
pub use uptrakit_shared_types::{MqttClientConnectionStatus, ParseMqttClientConnectionStatusError};

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MqttLimitResponse {
    pub max_clients_per_tenant: u16,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateMqttLimitRequest {
    pub max_clients_per_tenant: u16,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MqttClientResponse {
    pub id: Uuid,
    pub enabled: bool,
    pub transport: MqttTransport,
    pub host: String,
    pub port: u16,
    pub url: String,
    pub client_id: String,
    pub username: Option<String>,
    pub has_password: bool,
    pub has_ca_cert: bool,
    pub topic_prefix: String,
    pub ha_discovery: bool,
    pub ha_discovery_prefix: String,
    pub connection_status: MqttClientConnectionStatus,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateMqttClientRequest {
    /// MQTT URL (e.g. `mqtt://broker:1883`, `mqtts://broker:8883`).
    /// If provided, `transport`, `host`, and `port` are extracted from it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport: Option<MqttTransport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<SecretString>,
    /// Custom CA certificate in PEM format for TLS connections to private brokers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ca_pem: Option<SecretString>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ha_discovery: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ha_discovery_prefix: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateMqttClientRequest {
    /// MQTT URL — if provided, overrides `transport`, `host`, `port`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport: Option<MqttTransport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// Set to `null` to clear, omit to keep existing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<serde_json::Value>,
    /// Set to `null` to clear, omit to keep existing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<serde_json::Value>,
    /// Set to `null` to clear, omit to keep existing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ca_pem: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ha_discovery: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ha_discovery_prefix: Option<String>,
}

impl Validate for CreateMqttClientRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if let Some(ref url) = self.url
            && url.len() > 2048
        {
            return Err(ValidationError {
                field: "url",
                message: "must not exceed 2048 characters".to_string(),
            });
        }
        if let Some(ref host) = self.host {
            if host.is_empty() {
                return Err(ValidationError {
                    field: "host",
                    message: "must not be empty".to_string(),
                });
            }
            if host.len() > 253 {
                return Err(ValidationError {
                    field: "host",
                    message: "must not exceed 253 characters".to_string(),
                });
            }
        }
        if let Some(ref client_id) = self.client_id {
            if client_id.is_empty() {
                return Err(ValidationError {
                    field: "client_id",
                    message: "must not be empty".to_string(),
                });
            }
            if client_id.len() > 256 {
                return Err(ValidationError {
                    field: "client_id",
                    message: "must not exceed 256 characters".to_string(),
                });
            }
        }
        if let Some(ref username) = self.username
            && username.len() > 256
        {
            return Err(ValidationError {
                field: "username",
                message: "must not exceed 256 characters".to_string(),
            });
        }
        if let Some(ref password) = self.password
            && password.expose_secret().len() > 256
        {
            return Err(ValidationError {
                field: "password",
                message: "must not exceed 256 characters".to_string(),
            });
        }
        if let Some(ref topic_prefix) = self.topic_prefix {
            if topic_prefix.is_empty() {
                return Err(ValidationError {
                    field: "topic_prefix",
                    message: "must not be empty".to_string(),
                });
            }
            if topic_prefix.len() > 128 {
                return Err(ValidationError {
                    field: "topic_prefix",
                    message: "must not exceed 128 characters".to_string(),
                });
            }
        }
        if let Some(ref prefix) = self.ha_discovery_prefix {
            if prefix.is_empty() {
                return Err(ValidationError {
                    field: "ha_discovery_prefix",
                    message: "must not be empty".to_string(),
                });
            }
            if prefix.len() > 128 {
                return Err(ValidationError {
                    field: "ha_discovery_prefix",
                    message: "must not exceed 128 characters".to_string(),
                });
            }
        }
        Ok(())
    }
}

impl Validate for UpdateMqttClientRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if let Some(ref url) = self.url
            && url.len() > 2048
        {
            return Err(ValidationError {
                field: "url",
                message: "must not exceed 2048 characters".to_string(),
            });
        }
        if let Some(ref host) = self.host {
            if host.is_empty() {
                return Err(ValidationError {
                    field: "host",
                    message: "must not be empty".to_string(),
                });
            }
            if host.len() > 253 {
                return Err(ValidationError {
                    field: "host",
                    message: "must not exceed 253 characters".to_string(),
                });
            }
        }
        if let Some(ref client_id) = self.client_id {
            if client_id.is_empty() {
                return Err(ValidationError {
                    field: "client_id",
                    message: "must not be empty".to_string(),
                });
            }
            if client_id.len() > 256 {
                return Err(ValidationError {
                    field: "client_id",
                    message: "must not exceed 256 characters".to_string(),
                });
            }
        }
        // For nullable JSON fields (username, password, ca_pem): null = clear, string = set.
        // Validate length when a string value is provided.
        if let Some(ref val) = self.username {
            if let Some(s) = val.as_str() {
                if s.len() > 256 {
                    return Err(ValidationError {
                        field: "username",
                        message: "must not exceed 256 characters".to_string(),
                    });
                }
            } else if !val.is_null() {
                return Err(ValidationError {
                    field: "username",
                    message: "must be a string or null".to_string(),
                });
            }
        }
        if let Some(ref val) = self.password {
            if let Some(s) = val.as_str() {
                if s.len() > 256 {
                    return Err(ValidationError {
                        field: "password",
                        message: "must not exceed 256 characters".to_string(),
                    });
                }
            } else if !val.is_null() {
                return Err(ValidationError {
                    field: "password",
                    message: "must be a string or null".to_string(),
                });
            }
        }
        if let Some(ref topic_prefix) = self.topic_prefix {
            if topic_prefix.is_empty() {
                return Err(ValidationError {
                    field: "topic_prefix",
                    message: "must not be empty".to_string(),
                });
            }
            if topic_prefix.len() > 128 {
                return Err(ValidationError {
                    field: "topic_prefix",
                    message: "must not exceed 128 characters".to_string(),
                });
            }
        }
        if let Some(ref prefix) = self.ha_discovery_prefix {
            if prefix.is_empty() {
                return Err(ValidationError {
                    field: "ha_discovery_prefix",
                    message: "must not be empty".to_string(),
                });
            }
            if prefix.len() > 128 {
                return Err(ValidationError {
                    field: "ha_discovery_prefix",
                    message: "must not exceed 128 characters".to_string(),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_status_from_str_valid() {
        assert_eq!(
            "online".parse::<MqttClientConnectionStatus>().ok(),
            Some(MqttClientConnectionStatus::Online)
        );
        assert_eq!(
            "offline".parse::<MqttClientConnectionStatus>().ok(),
            Some(MqttClientConnectionStatus::Offline)
        );
        assert_eq!(
            "connecting".parse::<MqttClientConnectionStatus>().ok(),
            Some(MqttClientConnectionStatus::Connecting)
        );
    }

    #[test]
    fn connection_status_from_str_invalid() {
        assert!("unknown".parse::<MqttClientConnectionStatus>().is_err());
        assert!("".parse::<MqttClientConnectionStatus>().is_err());
        assert!("ONLINE".parse::<MqttClientConnectionStatus>().is_err());
    }

    #[test]
    fn connection_status_as_str_round_trips_through_from_str() {
        for status in [
            MqttClientConnectionStatus::Online,
            MqttClientConnectionStatus::Offline,
            MqttClientConnectionStatus::Connecting,
        ] {
            let s = status.as_str();
            let parsed: MqttClientConnectionStatus = s
                .parse()
                .expect("from_str should succeed for as_str output");
            assert_eq!(parsed, status);
        }
    }

    #[test]
    fn connection_status_display_matches_as_str() {
        for status in [
            MqttClientConnectionStatus::Online,
            MqttClientConnectionStatus::Offline,
            MqttClientConnectionStatus::Connecting,
        ] {
            assert_eq!(format!("{status}"), status.as_str());
        }
    }

    #[test]
    fn connection_status_default_is_offline() {
        assert_eq!(
            MqttClientConnectionStatus::default(),
            MqttClientConnectionStatus::Offline
        );
    }

    #[test]
    fn connection_status_serde_round_trip() {
        for status in [
            MqttClientConnectionStatus::Online,
            MqttClientConnectionStatus::Offline,
            MqttClientConnectionStatus::Connecting,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let deserialized: MqttClientConnectionStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, status);
        }
    }

    #[test]
    fn create_request_validates_empty_host() {
        let req = CreateMqttClientRequest {
            url: None,
            transport: None,
            host: Some(String::new()),
            port: None,
            enabled: None,
            client_id: None,
            username: None,
            password: None,
            ca_pem: None,
            topic_prefix: None,
            ha_discovery: None,
            ha_discovery_prefix: None,
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "host");
    }

    #[test]
    fn create_request_validates_empty_client_id() {
        let req = CreateMqttClientRequest {
            url: None,
            transport: None,
            host: None,
            port: None,
            enabled: None,
            client_id: Some(String::new()),
            username: None,
            password: None,
            ca_pem: None,
            topic_prefix: None,
            ha_discovery: None,
            ha_discovery_prefix: None,
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "client_id");
    }

    #[test]
    fn create_request_validates_oversized_url() {
        let req = CreateMqttClientRequest {
            url: Some("a".repeat(2049)),
            transport: None,
            host: None,
            port: None,
            enabled: None,
            client_id: None,
            username: None,
            password: None,
            ca_pem: None,
            topic_prefix: None,
            ha_discovery: None,
            ha_discovery_prefix: None,
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "url");
    }

    #[test]
    fn create_request_valid_when_all_none() {
        let req = CreateMqttClientRequest {
            url: None,
            transport: None,
            host: None,
            port: None,
            enabled: None,
            client_id: None,
            username: None,
            password: None,
            ca_pem: None,
            topic_prefix: None,
            ha_discovery: None,
            ha_discovery_prefix: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn update_request_rejects_non_string_username() {
        let req = UpdateMqttClientRequest {
            url: None,
            transport: None,
            host: None,
            port: None,
            enabled: None,
            client_id: None,
            username: Some(serde_json::json!(42)),
            password: None,
            ca_pem: None,
            topic_prefix: None,
            ha_discovery: None,
            ha_discovery_prefix: None,
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "username");
    }

    #[test]
    fn update_request_accepts_null_password() {
        let req = UpdateMqttClientRequest {
            url: None,
            transport: None,
            host: None,
            port: None,
            enabled: None,
            client_id: None,
            username: None,
            password: Some(serde_json::Value::Null),
            ca_pem: None,
            topic_prefix: None,
            ha_discovery: None,
            ha_discovery_prefix: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn update_request_validates_empty_topic_prefix() {
        let req = UpdateMqttClientRequest {
            url: None,
            transport: None,
            host: None,
            port: None,
            enabled: None,
            client_id: None,
            username: None,
            password: None,
            ca_pem: None,
            topic_prefix: Some(String::new()),
            ha_discovery: None,
            ha_discovery_prefix: None,
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "topic_prefix");
    }
}
