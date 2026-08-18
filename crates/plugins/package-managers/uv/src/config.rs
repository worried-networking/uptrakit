use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::{
    PluginConfig, PluginConfigValidationError, TypeSettings,
};

/// Configuration for the uv Tools plugin.
///
/// Tracks Python CLI tools installed via `uv tool install` on agent hosts.
/// No secrets — the `package_identifier` is the PyPI project name.
///
/// The default config (`{}`) fetches releases from the PyPI Simple API
/// (`https://pypi.org/simple`). Set `index_url` for self-hosted mirrors.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UvConfig {
    /// Include pre-release versions (PEP 440 pre/dev segments, e.g. `1.0rc1`,
    /// `2.0.0.dev1`) when fetching upstream releases. Defaults to `false`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub include_prereleases: bool,

    /// Custom PyPI Simple index base URL.
    ///
    /// When `None` (the default), the plugin uses `https://pypi.org/simple`.
    /// Set to the Simple-API root of a self-hosted mirror (devpi, Artifactory,
    /// Nexus, GitLab). `http` is allowed and private/LAN addresses are
    /// admitted (SSRF protection is relaxed to permissive for custom indexes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_url: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !b
}

impl PluginConfig for UvConfig {
    fn validate_identifier(value: &str) -> Result<(), PluginConfigValidationError> {
        crate::validate_identifier(value)
    }

    fn validate(&self) -> Result<(), PluginConfigValidationError> {
        if let Some(raw) = &self.index_url {
            if raw.is_empty() {
                return Err(PluginConfigValidationError::invalid_field(
                    "index_url",
                    "must not be empty when set; omit the field to use https://pypi.org/simple",
                ));
            }
            let parsed = url::Url::parse(raw).map_err(|e| {
                PluginConfigValidationError::invalid_field("index_url", format!("invalid URL: {e}"))
            })?;
            if parsed.scheme() != "http" && parsed.scheme() != "https" {
                return Err(PluginConfigValidationError::invalid_field(
                    "index_url",
                    "must use the http or https scheme",
                ));
            }
            if !parsed.username().is_empty() || parsed.password().is_some() {
                return Err(PluginConfigValidationError::invalid_field(
                    "index_url",
                    "must not embed credentials in the URL",
                ));
            }
        }
        Ok(())
    }
}

impl TypeSettings for UvConfig {
    fn type_settings_form_schema()
    -> Vec<uptrakit_plugin_infrastructure_core::form_schema::FormFieldDescriptor> {
        use uptrakit_plugin_infrastructure_core::form_schema::{
            FormFieldDescriptor, FormFieldType,
        };
        vec![
            FormFieldDescriptor::new("include_prereleases", "Include pre-releases")
                .with_type(FormFieldType::Toggle)
                .with_help_text(
                    "Include PEP 440 pre-release and dev versions (e.g. 1.0rc1, 2.0.0.dev1) \
                     in available updates. Defaults to false.",
                ),
            FormFieldDescriptor::new("index_url", "Custom index URL")
                .with_type(FormFieldType::Text)
                .with_help_text(
                    "PyPI Simple index base URL. Defaults to https://pypi.org/simple. \
                     Set for self-hosted mirrors (devpi, Artifactory, Nexus, GitLab).",
                ),
        ]
    }

    fn type_settings_sample() -> serde_json::Value {
        serde_json::json!({})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_accepts_default_config() {
        UvConfig::default().validate().unwrap();
    }

    #[test]
    fn validate_accepts_https_and_http_index_urls() {
        for u in [
            "https://pypi.org/simple",
            "http://mirror.lan:3141/root/pypi/+simple/",
        ] {
            let config = UvConfig {
                include_prereleases: false,
                index_url: Some(u.to_string()),
            };
            config.validate().unwrap();
        }
    }

    #[test]
    fn validate_rejects_bad_index_urls() {
        for u in [
            "not a url",
            "ftp://mirror/simple",
            "https://user:pw@mirror/simple",
            "https://user@mirror/simple",
        ] {
            let config = UvConfig {
                include_prereleases: false,
                index_url: Some(u.to_string()),
            };
            config.validate().unwrap_err();
        }
    }

    #[test]
    fn validate_rejects_empty_index_url() {
        let config = UvConfig {
            include_prereleases: false,
            index_url: Some(String::new()),
        };
        let Err(err) = config.validate() else {
            panic!("expected empty index_url to be rejected");
        };
        match err {
            PluginConfigValidationError::InvalidField { field, message } => {
                assert_eq!(field, "index_url");
                assert!(
                    message.contains("must not be empty when set"),
                    "expected the dedicated empty-string message, got: {message}"
                );
            }
            other => panic!("expected InvalidField, got {other:?}"),
        }
    }

    #[test]
    fn deserialize_empty_object_gives_defaults() {
        let config: UvConfig = serde_json::from_str("{}").expect("deserialize");
        assert!(!config.include_prereleases);
        assert!(config.index_url.is_none());
    }

    #[test]
    fn serialization_roundtrips() {
        let config = UvConfig {
            include_prereleases: true,
            index_url: Some("https://mirror.example/simple".to_string()),
        };
        let json = serde_json::to_value(&config).expect("serialize");
        assert_eq!(json["include_prereleases"], true);
        assert_eq!(json["index_url"], "https://mirror.example/simple");
        let back: UvConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(back, config);
    }

    #[test]
    fn serialization_default_elides_both_fields() {
        let json = serde_json::to_value(UvConfig::default()).expect("serialize");
        assert_eq!(json, serde_json::json!({}));
    }
}
