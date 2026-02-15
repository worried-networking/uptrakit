use rootcause::prelude::*;
use serde::{Deserialize, Serialize};
use uptrakit_provider_core::{SecretMasking, SecretString};

use crate::error::{DockerRegistryError, Result};

/// Sentinel value used to indicate a masked secret in API responses.
const SECRET_MASK: &str = "***";

/// Tracking mode for the Docker Registry provider.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackingMode {
    /// Track semver-parseable tags, filter and sort by version.
    #[default]
    SemverTags,
    /// Track digest changes of a specific tag.
    DigestTracking,
}

/// Authentication configuration for Docker registries.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DockerAuth {
    /// HTTP Basic authentication.
    Basic {
        username: String,
        password: SecretString,
    },
    /// Bearer token authentication.
    Bearer { token: SecretString },
}

/// Configuration for the Docker Registry provider.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DockerRegistryConfig {
    /// Full image reference (e.g. `nginx`, `ghcr.io/owner/repo`).
    pub image: String,

    /// Override registry hostname. Inferred from `image` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,

    /// Authentication credentials.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<DockerAuth>,

    /// Tracking mode: semver_tags or digest_tracking.
    #[serde(default)]
    pub tracking_mode: TrackingMode,

    /// Regex patterns to filter tags (semver mode, OR logic, empty = all).
    #[serde(default)]
    pub tag_patterns: Vec<String>,

    /// Prefix to strip from tags before semver parsing.
    #[serde(default = "default_tag_strip_prefix")]
    pub tag_strip_prefix: String,

    /// Whether to include pre-release semver versions.
    #[serde(default)]
    pub include_prereleases: bool,

    /// Tag to track in digest mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracked_tag: Option<String>,

    /// Maximum tags per API request (pagination).
    #[serde(default = "default_page_size")]
    pub page_size: u32,
    /// Shell command to run after pulling the new image.
    ///
    /// Supports `{image}`, `{tag}`, and `{version}` placeholders.
    /// If absent, only `docker pull` is performed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restart_command: Option<String>,
}

fn default_tag_strip_prefix() -> String {
    "v".to_string()
}

fn default_page_size() -> u32 {
    1000
}

impl DockerRegistryConfig {
    /// Validate the configuration.
    pub fn validate(&self) -> Result<()> {
        if self.image.is_empty() {
            bail!(DockerRegistryError::Configuration(
                "image must not be empty".to_string()
            ));
        }

        if self.page_size == 0 {
            bail!(DockerRegistryError::Configuration(
                "page_size must be greater than 0".to_string()
            ));
        }

        for pattern in &self.tag_patterns {
            regex::Regex::new(pattern).map_err(|e| {
                report!(DockerRegistryError::InvalidPattern(format!(
                    "invalid regex pattern '{pattern}': {e}"
                )))
            })?;
        }

        Ok(())
    }

    /// Resolve the registry hostname from the image reference.
    ///
    /// - Explicit `registry` field takes precedence
    /// - `ghcr.io/owner/repo` -> `ghcr.io`
    /// - `nginx` (no slash or Docker Hub official) -> `registry-1.docker.io`
    /// - `myregistry.com/repo` -> `myregistry.com`
    pub fn resolved_registry(&self) -> &str {
        if let Some(ref registry) = self.registry {
            return registry.as_str();
        }
        infer_registry(&self.image)
    }

    /// Extract the repository path from the image reference.
    ///
    /// - `nginx` -> `library/nginx` (Docker Hub official image)
    /// - `ghcr.io/owner/repo` -> `owner/repo`
    /// - `myregistry.com/path/repo` -> `path/repo`
    pub fn resolved_repository(&self) -> String {
        resolve_repository(&self.image)
    }

    /// The tag to track in digest mode.
    pub fn resolved_tracked_tag(&self) -> &str {
        self.tracked_tag.as_deref().unwrap_or("latest")
    }

    /// Best-effort web URL for the image.
    pub fn image_web_url(&self, tag: &str) -> String {
        let registry = self.resolved_registry();
        let repo = self.resolved_repository();

        if registry == "registry-1.docker.io" {
            // Docker Hub official or user images
            let hub_path = repo.strip_prefix("library/").unwrap_or(&repo);
            format!("https://hub.docker.com/_/{hub_path}/tags?name={tag}")
        } else if registry == "ghcr.io" {
            format!("https://ghcr.io/{repo}:{tag}")
        } else {
            format!("https://{registry}/{repo}:{tag}")
        }
    }
}

impl SecretMasking for DockerRegistryConfig {
    /// Return a copy with secret fields masked for API responses.
    ///
    /// `None` auth stays `None`. When auth is present, password/token fields
    /// are replaced with the mask sentinel.
    fn with_secrets_masked(mut self) -> Self {
        self.auth = self.auth.map(|a| match a {
            DockerAuth::Basic { username, .. } => DockerAuth::Basic {
                username,
                password: SecretString::new(SECRET_MASK.to_string()),
            },
            DockerAuth::Bearer { .. } => DockerAuth::Bearer {
                token: SecretString::new(SECRET_MASK.to_string()),
            },
        });
        self
    }

    /// Restore masked secrets from an existing config (for PUT updates).
    ///
    /// If the incoming auth credentials contain the mask sentinel, take
    /// the value from `existing`.
    fn restore_secrets_from(&mut self, existing: &Self) {
        let Some(existing_auth) = &existing.auth else {
            return;
        };
        let Some(incoming_auth) = &mut self.auth else {
            return;
        };
        match (incoming_auth, existing_auth) {
            (
                DockerAuth::Basic {
                    password: incoming_pw,
                    ..
                },
                DockerAuth::Basic {
                    password: existing_pw,
                    ..
                },
            ) => {
                if incoming_pw.expose_secret() == SECRET_MASK {
                    *incoming_pw = existing_pw.clone();
                }
            }
            (
                DockerAuth::Bearer {
                    token: incoming_token,
                },
                DockerAuth::Bearer {
                    token: existing_token,
                },
            ) => {
                if incoming_token.expose_secret() == SECRET_MASK {
                    *incoming_token = existing_token.clone();
                }
            }
            _ => {}
        }
    }
}

/// Infer the registry hostname from an image reference.
fn infer_registry(image: &str) -> &str {
    // If image contains a slash, the first component might be a registry
    if let Some(first_slash) = image.find('/') {
        let first_component = &image[..first_slash];
        // A registry hostname typically contains a dot or a colon (port), or is "localhost"
        if first_component.contains('.')
            || first_component.contains(':')
            || first_component == "localhost"
        {
            return first_component;
        }
    }
    // Default to Docker Hub
    "registry-1.docker.io"
}

/// Resolve the repository path from an image reference.
fn resolve_repository(image: &str) -> String {
    if let Some(first_slash) = image.find('/') {
        let first_component = &image[..first_slash];
        // Check if first component is a registry hostname
        if first_component.contains('.')
            || first_component.contains(':')
            || first_component == "localhost"
        {
            // Strip the registry prefix
            return image[first_slash + 1..].to_string();
        }
        // Not a registry hostname, so the whole thing is the repo path (e.g. "user/repo")
        return image.to_string();
    }
    // Single name like "nginx" -> Docker Hub official: "library/nginx"
    format!("library/{image}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let json = r#"{"image":"nginx"}"#;
        let config: DockerRegistryConfig = serde_json::from_str(json).expect("deserialize");
        assert_eq!(config.image, "nginx");
        assert!(config.registry.is_none());
        assert!(config.auth.is_none());
        assert_eq!(config.tracking_mode, TrackingMode::SemverTags);
        assert!(config.tag_patterns.is_empty());
        assert_eq!(config.tag_strip_prefix, "v");
        assert!(!config.include_prereleases);
        assert!(config.tracked_tag.is_none());
        assert_eq!(config.page_size, 1000);
    }

    #[test]
    fn validation_passes_minimal() {
        let config = DockerRegistryConfig {
            image: "nginx".to_string(),
            registry: None,
            auth: None,
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 1000,
            restart_command: None,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validation_fails_empty_image() {
        let config = DockerRegistryConfig {
            image: String::new(),
            registry: None,
            auth: None,
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 1000,
            restart_command: None,
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("image"));
    }

    #[test]
    fn validation_fails_zero_page_size() {
        let config = DockerRegistryConfig {
            image: "nginx".to_string(),
            registry: None,
            auth: None,
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 0,
            restart_command: None,
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("page_size"));
    }

    #[test]
    fn validation_fails_invalid_regex() {
        let config = DockerRegistryConfig {
            image: "nginx".to_string(),
            registry: None,
            auth: None,
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec!["[invalid".to_string()],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 1000,
            restart_command: None,
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("invalid regex"));
    }

    #[test]
    fn validation_passes_valid_regex() {
        let config = DockerRegistryConfig {
            image: "nginx".to_string(),
            registry: None,
            auth: None,
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![r"^\d+\.\d+\.\d+$".to_string()],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 1000,
            restart_command: None,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn resolved_registry_docker_hub_official() {
        let config = DockerRegistryConfig {
            image: "nginx".to_string(),
            registry: None,
            auth: None,
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 1000,
            restart_command: None,
        };
        assert_eq!(config.resolved_registry(), "registry-1.docker.io");
    }

    #[test]
    fn resolved_registry_docker_hub_user() {
        let config = DockerRegistryConfig {
            image: "myuser/myrepo".to_string(),
            registry: None,
            auth: None,
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 1000,
            restart_command: None,
        };
        assert_eq!(config.resolved_registry(), "registry-1.docker.io");
    }

    #[test]
    fn resolved_registry_ghcr() {
        let config = DockerRegistryConfig {
            image: "ghcr.io/owner/repo".to_string(),
            registry: None,
            auth: None,
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 1000,
            restart_command: None,
        };
        assert_eq!(config.resolved_registry(), "ghcr.io");
    }

    #[test]
    fn resolved_registry_private() {
        let config = DockerRegistryConfig {
            image: "registry.example.com/myapp".to_string(),
            registry: None,
            auth: None,
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 1000,
            restart_command: None,
        };
        assert_eq!(config.resolved_registry(), "registry.example.com");
    }

    #[test]
    fn resolved_registry_override() {
        let config = DockerRegistryConfig {
            image: "nginx".to_string(),
            registry: Some("my-mirror.example.com".to_string()),
            auth: None,
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 1000,
            restart_command: None,
        };
        assert_eq!(config.resolved_registry(), "my-mirror.example.com");
    }

    #[test]
    fn resolved_registry_localhost() {
        let config = DockerRegistryConfig {
            image: "localhost/myapp".to_string(),
            registry: None,
            auth: None,
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 1000,
            restart_command: None,
        };
        assert_eq!(config.resolved_registry(), "localhost");
    }

    #[test]
    fn resolved_registry_with_port() {
        let config = DockerRegistryConfig {
            image: "myhost:5000/myapp".to_string(),
            registry: None,
            auth: None,
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 1000,
            restart_command: None,
        };
        assert_eq!(config.resolved_registry(), "myhost:5000");
    }

    #[test]
    fn resolved_repository_docker_hub_official() {
        let config = DockerRegistryConfig {
            image: "nginx".to_string(),
            registry: None,
            auth: None,
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 1000,
            restart_command: None,
        };
        assert_eq!(config.resolved_repository(), "library/nginx");
    }

    #[test]
    fn resolved_repository_docker_hub_user() {
        let config = DockerRegistryConfig {
            image: "myuser/myrepo".to_string(),
            registry: None,
            auth: None,
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 1000,
            restart_command: None,
        };
        assert_eq!(config.resolved_repository(), "myuser/myrepo");
    }

    #[test]
    fn resolved_repository_ghcr() {
        let config = DockerRegistryConfig {
            image: "ghcr.io/owner/repo".to_string(),
            registry: None,
            auth: None,
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 1000,
            restart_command: None,
        };
        assert_eq!(config.resolved_repository(), "owner/repo");
    }

    #[test]
    fn resolved_repository_private_nested() {
        let config = DockerRegistryConfig {
            image: "registry.example.com/org/team/app".to_string(),
            registry: None,
            auth: None,
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 1000,
            restart_command: None,
        };
        assert_eq!(config.resolved_repository(), "org/team/app");
    }

    #[test]
    fn resolved_tracked_tag_default() {
        let config = DockerRegistryConfig {
            image: "nginx".to_string(),
            registry: None,
            auth: None,
            tracking_mode: TrackingMode::DigestTracking,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 1000,
            restart_command: None,
        };
        assert_eq!(config.resolved_tracked_tag(), "latest");
    }

    #[test]
    fn resolved_tracked_tag_custom() {
        let config = DockerRegistryConfig {
            image: "nginx".to_string(),
            registry: None,
            auth: None,
            tracking_mode: TrackingMode::DigestTracking,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: Some("stable".to_string()),
            page_size: 1000,
            restart_command: None,
        };
        assert_eq!(config.resolved_tracked_tag(), "stable");
    }

    #[test]
    fn serialization_roundtrip() {
        let config = DockerRegistryConfig {
            image: "ghcr.io/owner/repo".to_string(),
            registry: Some("ghcr.io".to_string()),
            auth: Some(DockerAuth::Basic {
                username: "user".to_string(),
                password: SecretString::new("pass".to_string()),
            }),
            tracking_mode: TrackingMode::DigestTracking,
            tag_patterns: vec![r"^\d+".to_string()],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: true,
            tracked_tag: Some("main".to_string()),
            page_size: 500,
            restart_command: None,
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let deserialized: DockerRegistryConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.image, config.image);
        assert_eq!(deserialized.registry, config.registry);
        assert_eq!(deserialized.tracking_mode, config.tracking_mode);
        assert_eq!(deserialized.tag_patterns, config.tag_patterns);
        assert_eq!(deserialized.include_prereleases, config.include_prereleases);
        assert_eq!(deserialized.tracked_tag, config.tracked_tag);
        assert_eq!(deserialized.page_size, config.page_size);
    }

    #[test]
    fn auth_omitted_when_none() {
        let config = DockerRegistryConfig {
            image: "nginx".to_string(),
            registry: None,
            auth: None,
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 1000,
            restart_command: None,
        };
        let json = serde_json::to_string(&config).expect("serialize");
        assert!(!json.contains("auth"));
    }

    #[test]
    fn auth_basic_serialization() {
        let auth = DockerAuth::Basic {
            username: "user".to_string(),
            password: SecretString::new("pass".to_string()),
        };
        let json = serde_json::to_string(&auth).expect("serialize");
        assert!(json.contains(r#""type":"basic""#));
        assert!(json.contains(r#""username":"user""#));
        let deserialized: DockerAuth = serde_json::from_str(&json).expect("deserialize");
        match deserialized {
            DockerAuth::Basic { username, password } => {
                assert_eq!(username, "user");
                assert_eq!(password.expose_secret(), "pass");
            }
            _ => panic!("expected Basic auth"),
        }
    }

    #[test]
    fn auth_bearer_serialization() {
        let auth = DockerAuth::Bearer {
            token: SecretString::new("my-token".to_string()),
        };
        let json = serde_json::to_string(&auth).expect("serialize");
        assert!(json.contains(r#""type":"bearer""#));
        let deserialized: DockerAuth = serde_json::from_str(&json).expect("deserialize");
        match deserialized {
            DockerAuth::Bearer { token } => {
                assert_eq!(token.expose_secret(), "my-token");
            }
            _ => panic!("expected Bearer auth"),
        }
    }

    #[test]
    fn with_secrets_masked_basic_auth() {
        let config = DockerRegistryConfig {
            image: "nginx".to_string(),
            registry: None,
            auth: Some(DockerAuth::Basic {
                username: "user".to_string(),
                password: SecretString::new("secret123".to_string()),
            }),
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 1000,
            restart_command: None,
        };
        let masked = config.with_secrets_masked();
        match masked.auth.unwrap() {
            DockerAuth::Basic { username, password } => {
                assert_eq!(username, "user");
                assert_eq!(password.expose_secret(), SECRET_MASK);
            }
            _ => panic!("expected Basic auth"),
        }
    }

    #[test]
    fn with_secrets_masked_bearer_auth() {
        let config = DockerRegistryConfig {
            image: "nginx".to_string(),
            registry: None,
            auth: Some(DockerAuth::Bearer {
                token: SecretString::new("ghcr_token".to_string()),
            }),
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 1000,
            restart_command: None,
        };
        let masked = config.with_secrets_masked();
        match masked.auth.unwrap() {
            DockerAuth::Bearer { token } => {
                assert_eq!(token.expose_secret(), SECRET_MASK);
            }
            _ => panic!("expected Bearer auth"),
        }
    }

    #[test]
    fn with_secrets_masked_no_auth_stays_none() {
        let config = DockerRegistryConfig {
            image: "nginx".to_string(),
            registry: None,
            auth: None,
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 1000,
            restart_command: None,
        };
        let masked = config.with_secrets_masked();
        assert!(masked.auth.is_none());
    }

    #[test]
    fn restore_secrets_from_basic_password() {
        let existing = DockerRegistryConfig {
            image: "nginx".to_string(),
            registry: None,
            auth: Some(DockerAuth::Basic {
                username: "user".to_string(),
                password: SecretString::new("real_password".to_string()),
            }),
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 1000,
            restart_command: None,
        };
        let mut incoming = existing.clone().with_secrets_masked();
        incoming.restore_secrets_from(&existing);
        match incoming.auth.unwrap() {
            DockerAuth::Basic { password, .. } => {
                assert_eq!(password.expose_secret(), "real_password");
            }
            _ => panic!("expected Basic auth"),
        }
    }

    #[test]
    fn restore_secrets_from_bearer_token() {
        let existing = DockerRegistryConfig {
            image: "nginx".to_string(),
            registry: None,
            auth: Some(DockerAuth::Bearer {
                token: SecretString::new("real_token".to_string()),
            }),
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 1000,
            restart_command: None,
        };
        let mut incoming = existing.clone().with_secrets_masked();
        incoming.restore_secrets_from(&existing);
        match incoming.auth.unwrap() {
            DockerAuth::Bearer { token } => {
                assert_eq!(token.expose_secret(), "real_token");
            }
            _ => panic!("expected Bearer auth"),
        }
    }

    #[test]
    fn restore_secrets_from_keeps_new_password() {
        let existing = DockerRegistryConfig {
            image: "nginx".to_string(),
            registry: None,
            auth: Some(DockerAuth::Basic {
                username: "user".to_string(),
                password: SecretString::new("old_password".to_string()),
            }),
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 1000,
            restart_command: None,
        };
        let mut incoming = DockerRegistryConfig {
            image: "nginx".to_string(),
            registry: None,
            auth: Some(DockerAuth::Basic {
                username: "user".to_string(),
                password: SecretString::new("new_password".to_string()),
            }),
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 1000,
            restart_command: None,
        };
        incoming.restore_secrets_from(&existing);
        match incoming.auth.unwrap() {
            DockerAuth::Basic { password, .. } => {
                assert_eq!(password.expose_secret(), "new_password");
            }
            _ => panic!("expected Basic auth"),
        }
    }

    #[test]
    fn tracking_mode_serialization() {
        let semver = TrackingMode::SemverTags;
        let json = serde_json::to_string(&semver).expect("serialize");
        assert_eq!(json, r#""semver_tags""#);

        let digest = TrackingMode::DigestTracking;
        let json = serde_json::to_string(&digest).expect("serialize");
        assert_eq!(json, r#""digest_tracking""#);

        let roundtrip: TrackingMode =
            serde_json::from_str(r#""semver_tags""#).expect("deserialize");
        assert_eq!(roundtrip, TrackingMode::SemverTags);

        let roundtrip: TrackingMode =
            serde_json::from_str(r#""digest_tracking""#).expect("deserialize");
        assert_eq!(roundtrip, TrackingMode::DigestTracking);
    }

    #[test]
    fn image_web_url_docker_hub_official() {
        let config = DockerRegistryConfig {
            image: "nginx".to_string(),
            registry: None,
            auth: None,
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 1000,
            restart_command: None,
        };
        let url = config.image_web_url("1.25.0");
        assert_eq!(url, "https://hub.docker.com/_/nginx/tags?name=1.25.0");
    }

    #[test]
    fn image_web_url_docker_hub_user() {
        let config = DockerRegistryConfig {
            image: "myuser/myrepo".to_string(),
            registry: None,
            auth: None,
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 1000,
            restart_command: None,
        };
        let url = config.image_web_url("latest");
        assert_eq!(
            url,
            "https://hub.docker.com/_/myuser/myrepo/tags?name=latest"
        );
    }

    #[test]
    fn image_web_url_ghcr() {
        let config = DockerRegistryConfig {
            image: "ghcr.io/owner/repo".to_string(),
            registry: None,
            auth: None,
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 1000,
            restart_command: None,
        };
        let url = config.image_web_url("v1.0.0");
        assert_eq!(url, "https://ghcr.io/owner/repo:v1.0.0");
    }

    #[test]
    fn image_web_url_generic() {
        let config = DockerRegistryConfig {
            image: "registry.example.com/myapp".to_string(),
            registry: None,
            auth: None,
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 1000,
            restart_command: None,
        };
        let url = config.image_web_url("latest");
        assert_eq!(url, "https://registry.example.com/myapp:latest");
    }
}
