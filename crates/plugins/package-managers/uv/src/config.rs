use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::{
    PluginConfig, PluginConfigValidationError, TypeSettings,
};

/// Configuration for the uv Tools plugin.
///
/// Tracks Python CLI tools installed via `uv tool install` on agent hosts.
/// No secrets — the `package_identifier` is the PyPI project name.
///
/// `include_prereleases` and `index_url` control controller-side release
/// fetching from the PyPI Simple API. The default config (`{}`) fetches
/// releases from `https://pypi.org/simple`; `index_url` overrides that for
/// self-hosted mirrors.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UvConfig {
    /// Include pre-release versions (PEP 440 pre/dev segments, e.g. `1.0rc1`,
    /// `2.0.0.dev1`) in available updates. Defaults to `false`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub include_prereleases: bool,

    /// Custom PyPI Simple index base URL.
    ///
    /// `None` (the default) uses `https://pypi.org/simple`; set this to the
    /// Simple-API root of a self-hosted mirror (devpi, Artifactory, Nexus,
    /// GitLab) to override it. `http` is allowed and private/LAN addresses
    /// are admitted — SSRF protection is relaxed to permissive for all
    /// requests this plugin makes, including redirect targets.
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
            if parsed.query().is_some() || parsed.fragment().is_some() {
                return Err(PluginConfigValidationError::invalid_field(
                    "index_url",
                    "must not contain a query string or fragment; the package path is \
                     appended to this base URL",
                ));
            }
        }
        Ok(())
    }
}

impl UvConfig {
    /// Effective PyPI Simple index base URL, trailing slashes trimmed.
    ///
    /// Operators commonly configure mirrors pip-style with a trailing slash;
    /// trimming here keeps URL joins (`{base}/{name}/`) free of `//` and makes
    /// the update-preflight index comparison (Plan 3) canonical.
    pub(crate) fn effective_index_url(&self) -> &str {
        self.index_url
            .as_deref()
            .unwrap_or("https://pypi.org/simple")
            .trim_end_matches('/')
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
                    "Include PEP 440 pre-release and dev versions (e.g. 1.0rc1, \
                     2.0.0.dev1) in available updates. Defaults to false.",
                ),
            FormFieldDescriptor::new("index_url", "Custom index URL")
                .with_type(FormFieldType::Text)
                .with_help_text(
                    "Defaults to https://pypi.org/simple. Set for self-hosted mirrors \
                     (devpi, Artifactory, Nexus, GitLab); relaxes SSRF protection to \
                     permissive for all requests this plugin makes, including redirect \
                     targets.",
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
    fn validate_rejects_index_url_with_query_string() {
        let config = UvConfig {
            include_prereleases: false,
            index_url: Some("https://nexus.lan/simple?foo=1".to_string()),
        };
        let Err(err) = config.validate() else {
            panic!("expected query-string index_url to be rejected");
        };
        match err {
            PluginConfigValidationError::InvalidField { field, message } => {
                assert_eq!(field, "index_url");
                assert!(
                    message.contains("query string"),
                    "expected the query/fragment message, got: {message}"
                );
            }
            other => panic!("expected InvalidField, got {other:?}"),
        }
    }

    #[test]
    fn validate_rejects_index_url_with_fragment() {
        let config = UvConfig {
            include_prereleases: false,
            index_url: Some("https://nexus.lan/simple#frag".to_string()),
        };
        let Err(err) = config.validate() else {
            panic!("expected fragment index_url to be rejected");
        };
        match err {
            PluginConfigValidationError::InvalidField { field, message } => {
                assert_eq!(field, "index_url");
                assert!(
                    message.contains("fragment"),
                    "expected the query/fragment message, got: {message}"
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

    // Note: no generic non-default-value roundtrip test. `Serialize`/
    // `Deserialize` here are plain derives with no custom logic beyond the
    // `skip_serializing_if` elision — testing that a derived roundtrip
    // roundtrips is testing serde's derive macros, not project-owned
    // behavior. The project-owned half (both fields elided at their default
    // values) is covered below.
    #[test]
    fn serialization_default_elides_both_fields() {
        let json = serde_json::to_value(UvConfig::default()).expect("serialize");
        assert_eq!(json, serde_json::json!({}));
    }

    #[test]
    fn effective_index_url_default_and_trim() {
        assert_eq!(
            UvConfig::default().effective_index_url(),
            "https://pypi.org/simple"
        );
        let config = UvConfig {
            include_prereleases: false,
            index_url: Some("https://mirror.lan/simple/".to_string()),
        };
        assert_eq!(config.effective_index_url(), "https://mirror.lan/simple");
    }
}
