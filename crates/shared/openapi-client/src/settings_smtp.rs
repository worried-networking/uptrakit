use crate::Result;
use crate::UptrakitClient;
use uptrakit_web_api_types::settings_smtp::{SmtpSettingsResponse, UpdateSmtpSettingsRequest};

impl UptrakitClient {
    /// Get the global SMTP settings for the tenant.
    pub async fn get_smtp_settings(&self) -> Result<SmtpSettingsResponse> {
        self.get(crate::paths::settings_smtp::BASE).await
    }

    /// Update the global SMTP settings for the tenant.
    pub async fn update_smtp_settings(
        &self,
        req: &UpdateSmtpSettingsRequest,
    ) -> Result<SmtpSettingsResponse> {
        self.put_json(crate::paths::settings_smtp::BASE, req).await
    }
}

#[cfg(test)]
mod tests {
    use uptrakit_web_api_types::settings_smtp::UpdateSmtpSettingsRequest;

    #[test]
    fn update_smtp_request_full_serialization() {
        let req = UpdateSmtpSettingsRequest {
            host: Some("smtp.example.com".to_string()),
            port: Some(587),
            username: Some(serde_json::Value::String("user@example.com".to_string())),
            password: Some(serde_json::Value::String("secret".to_string())),
            from_address: Some("noreply@example.com".to_string()),
            from_name: Some(serde_json::Value::String("Uptrakit".to_string())),
            tls_mode: Some("starttls".to_string()),
            helo_host: None,
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["host"], "smtp.example.com");
        assert_eq!(json["port"], 587);
        assert_eq!(json["username"], "user@example.com");
        assert_eq!(json["from_address"], "noreply@example.com");
        assert_eq!(json["tls_mode"], "starttls");
    }

    #[test]
    fn update_smtp_request_null_clears_optional_fields() {
        let req = UpdateSmtpSettingsRequest {
            host: None,
            port: None,
            username: Some(serde_json::Value::Null),
            password: Some(serde_json::Value::Null),
            from_address: None,
            from_name: Some(serde_json::Value::Null),
            tls_mode: None,
            helo_host: None,
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert!(json["username"].is_null());
        assert!(json["password"].is_null());
        assert!(json["from_name"].is_null());
    }

    #[test]
    fn update_smtp_request_partial_update() {
        let req = UpdateSmtpSettingsRequest {
            host: Some("relay.example.com".to_string()),
            port: Some(465),
            username: None,
            password: None,
            from_address: None,
            from_name: None,
            tls_mode: Some("tls".to_string()),
            helo_host: None,
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["host"], "relay.example.com");
        assert_eq!(json["port"], 465);
        assert_eq!(json["tls_mode"], "tls");
        // Absent fields should not appear
        assert!(json.get("username").is_none() || json["username"].is_null());
    }
}
