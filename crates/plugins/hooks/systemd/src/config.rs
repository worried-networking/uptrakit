use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::PluginConfig;

/// Maximum length for a systemd service name.
const MAX_SERVICE_NAME_LEN: usize = 256;

/// Configuration for the systemd hook plugin.
///
/// Stops the specified systemd service before an update and starts it
/// again afterwards, regardless of whether the update succeeded.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemdHookConfig {
    /// The systemd service unit name (e.g. `"nginx"`, `"my-app.service"`).
    ///
    /// Validated to contain only safe characters: `[a-zA-Z0-9._@:-]`.
    #[serde(default)]
    pub service_name: String,
}

impl PluginConfig for SystemdHookConfig {
    fn validate(&self) -> Result<(), String> {
        validate_service_name(&self.service_name)
    }

    fn form_schema() -> Vec<uptrakit_plugin_infrastructure_core::form_schema::FormFieldDescriptor> {
        use uptrakit_plugin_infrastructure_core::form_schema::FormFieldDescriptor;
        vec![
            FormFieldDescriptor::new("service_name", "Service Name")
                .required()
                .with_help_text("Systemd service unit name (e.g. nginx, my-app.service)"),
        ]
    }
}

/// Validate a systemd service name.
///
/// Allows only `[a-zA-Z0-9._@:-]` and enforces a maximum length.
pub fn validate_service_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("service_name is required".to_string());
    }
    if name.len() > MAX_SERVICE_NAME_LEN {
        return Err(format!(
            "service_name exceeds maximum length of {MAX_SERVICE_NAME_LEN}"
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "._@:-".contains(c))
    {
        return Err(
            "service_name contains invalid characters; allowed: [a-zA-Z0-9._@:-]".to_string(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_service_name() {
        assert!(validate_service_name("nginx").is_ok());
        assert!(validate_service_name("my-app.service").is_ok());
        assert!(validate_service_name("user@1000.service").is_ok());
        assert!(validate_service_name("dbus-org.freedesktop.NetworkManager.service").is_ok());
    }

    #[test]
    fn invalid_service_name_empty() {
        assert!(validate_service_name("").is_err());
    }

    #[test]
    fn invalid_service_name_shell_chars() {
        assert!(validate_service_name("nginx; rm -rf /").is_err());
        assert!(validate_service_name("app$(whoami)").is_err());
    }

    #[test]
    fn invalid_service_name_too_long() {
        let name = "a".repeat(MAX_SERVICE_NAME_LEN + 1);
        assert!(validate_service_name(&name).is_err());
    }

    #[test]
    fn config_serde_roundtrip() {
        let config = SystemdHookConfig {
            service_name: "nginx".to_string(),
        };
        let json = serde_json::to_value(&config).unwrap();
        let deserialized: SystemdHookConfig = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized, config);
    }

    #[test]
    fn config_validation() {
        let valid = SystemdHookConfig {
            service_name: "nginx".to_string(),
        };
        assert!(valid.validate().is_ok());

        let empty = SystemdHookConfig {
            service_name: String::new(),
        };
        assert!(empty.validate().is_err());
    }
}
