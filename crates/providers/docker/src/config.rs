use rootcause::prelude::*;
use serde::{Deserialize, Serialize};
use uptrakit_provider_core::{SecretMasking, SecretString};

use crate::error::{DockerError, Result};

/// Sentinel value used to indicate a masked secret in API responses.
const SECRET_MASK: &str = "***";

/// Tracking mode for the Docker provider.
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

/// Configuration for restarting a service via `docker compose`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComposeRestartConfig {
    /// Path to the Compose file (e.g. `"docker-compose.prod.yml"`).
    /// When absent, Compose uses its default file lookup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compose_file: Option<String>,
    /// The specific Compose service to restart.
    /// When absent, all services are restarted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    /// Working directory in which to run `docker compose`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
}

/// Configuration for the Docker provider.
///
/// The `package_identifier` on each software item (a Docker image reference
/// such as `nginx` or `ghcr.io/owner/app:latest`) drives all registry
/// operations — no `image` field is needed in the config.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DockerConfig {
    /// Docker daemon endpoint override (e.g. `"unix:///var/run/docker.sock"`,
    /// `"tcp://host:2375"`, or `"ssh://user@host"` when the `ssh` feature is
    /// enabled).  Omit to use the platform default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docker_host: Option<String>,

    /// Path to the SSH private key used when `docker_host` is an `ssh://` URI.
    /// Normally injected by `agent-ssh`; set manually only when using a
    /// non-default key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_key_path: Option<String>,

    /// Authentication credentials for the Docker registry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<DockerAuth>,

    /// Tracking mode: `semver_tags` or `digest_tracking`.
    #[serde(default)]
    pub tracking_mode: TrackingMode,

    /// Regex patterns to filter tags (semver mode, OR logic, empty = all).
    #[serde(default)]
    pub tag_patterns: Vec<String>,

    /// Prefix to strip from tags before semver parsing (default `"v"`).
    #[serde(default = "default_tag_strip_prefix")]
    pub tag_strip_prefix: String,

    /// Whether to include pre-release semver versions.
    #[serde(default)]
    pub include_prereleases: bool,

    /// Tag to track in digest mode (default `"latest"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracked_tag: Option<String>,

    /// Maximum tags per API request (pagination, default `1000`).
    #[serde(default = "default_page_size")]
    pub page_size: u32,

    /// Restart via `docker compose` after pulling a new image.
    ///
    /// Runs `[cd {working_dir} &&] docker compose [-f {file}] up -d [{service}]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compose_restart: Option<ComposeRestartConfig>,

    /// Shell command to run after pulling the new image.
    ///
    /// Supports `{image}`, `{tag}`, and `{digest}` placeholders.
    /// If absent (and `compose_restart` is also absent), only `docker pull`
    /// is performed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_pull_command: Option<String>,
}

impl Default for DockerConfig {
    fn default() -> Self {
        Self {
            docker_host: None,
            ssh_key_path: None,
            auth: None,
            tracking_mode: TrackingMode::default(),
            tag_patterns: Vec::new(),
            tag_strip_prefix: default_tag_strip_prefix(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: default_page_size(),
            compose_restart: None,
            post_pull_command: None,
        }
    }
}

fn default_tag_strip_prefix() -> String {
    "v".to_string()
}

fn default_page_size() -> u32 {
    1000
}

impl DockerConfig {
    /// Validate the configuration.
    ///
    /// An empty (all-defaults) config is valid — discovery can proceed
    /// without any fields set.
    pub fn validate(&self) -> Result<()> {
        if self.page_size == 0 {
            bail!(DockerError::Configuration(
                "page_size must be greater than 0".to_string()
            ));
        }

        for pattern in &self.tag_patterns {
            regex::Regex::new(pattern).map_err(|e| {
                report!(DockerError::InvalidPattern(format!(
                    "invalid regex pattern '{pattern}': {e}"
                )))
            })?;
        }

        if let Some(ref cmd) = self.post_pull_command
            && cmd.is_empty()
        {
            bail!(DockerError::Configuration(
                "post_pull_command must not be an empty string when set".to_string()
            ));
        }

        if let Some(ref cr) = self.compose_restart
            && let Some(ref file) = cr.compose_file
            && file.split('/').any(|seg| seg == "..")
        {
            bail!(DockerError::Configuration(
                "compose_file must not contain '..' path segments".to_string()
            ));
        }

        if let Some(ref docker_host) = self.docker_host
            && docker_host.starts_with("ssh://")
            && let Some(ref key_path) = self.ssh_key_path
            && key_path.is_empty()
        {
            bail!(DockerError::Configuration(
                "ssh_key_path must not be empty when set".to_string()
            ));
        }

        Ok(())
    }

    /// The tag to track in digest mode.
    pub fn resolved_tracked_tag(&self) -> &str {
        self.tracked_tag.as_deref().unwrap_or("latest")
    }
}

impl SecretMasking for DockerConfig {
    /// Return a copy with secret fields masked for API responses.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_validates_ok() {
        let config = DockerConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validation_fails_zero_page_size() {
        let config = DockerConfig {
            page_size: 0,
            ..Default::default()
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("page_size"));
    }

    #[test]
    fn validation_fails_invalid_regex() {
        let config = DockerConfig {
            tag_patterns: vec!["[invalid".to_string()],
            ..Default::default()
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("invalid regex"));
    }

    #[test]
    fn validation_passes_valid_regex() {
        let config = DockerConfig {
            tag_patterns: vec![r"^\d+\.\d+\.\d+$".to_string()],
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validation_fails_empty_post_pull_command() {
        let config = DockerConfig {
            post_pull_command: Some(String::new()),
            ..Default::default()
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("post_pull_command"));
    }

    #[test]
    fn validation_passes_non_empty_post_pull_command() {
        let config = DockerConfig {
            post_pull_command: Some("docker compose up -d".to_string()),
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validation_fails_compose_file_with_dotdot() {
        let config = DockerConfig {
            compose_restart: Some(ComposeRestartConfig {
                compose_file: Some("../secret/docker-compose.yml".to_string()),
                service: None,
                working_dir: None,
            }),
            ..Default::default()
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("compose_file"));
    }

    #[test]
    fn validation_passes_valid_compose_config() {
        let config = DockerConfig {
            compose_restart: Some(ComposeRestartConfig {
                compose_file: Some("docker-compose.prod.yml".to_string()),
                service: Some("app".to_string()),
                working_dir: Some("/opt/app".to_string()),
            }),
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn resolved_tracked_tag_default() {
        let config = DockerConfig::default();
        assert_eq!(config.resolved_tracked_tag(), "latest");
    }

    #[test]
    fn resolved_tracked_tag_custom() {
        let config = DockerConfig {
            tracked_tag: Some("stable".to_string()),
            ..Default::default()
        };
        assert_eq!(config.resolved_tracked_tag(), "stable");
    }

    #[test]
    fn with_secrets_masked_basic_auth() {
        let config = DockerConfig {
            auth: Some(DockerAuth::Basic {
                username: "user".to_string(),
                password: SecretString::new("secret123".to_string()),
            }),
            ..Default::default()
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
        let config = DockerConfig {
            auth: Some(DockerAuth::Bearer {
                token: SecretString::new("ghcr_token".to_string()),
            }),
            ..Default::default()
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
        let config = DockerConfig::default();
        let masked = config.with_secrets_masked();
        assert!(masked.auth.is_none());
    }

    #[test]
    fn restore_secrets_from_basic_password() {
        let existing = DockerConfig {
            auth: Some(DockerAuth::Basic {
                username: "user".to_string(),
                password: SecretString::new("real_password".to_string()),
            }),
            ..Default::default()
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
        let existing = DockerConfig {
            auth: Some(DockerAuth::Bearer {
                token: SecretString::new("real_token".to_string()),
            }),
            ..Default::default()
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
        let existing = DockerConfig {
            auth: Some(DockerAuth::Basic {
                username: "user".to_string(),
                password: SecretString::new("old_password".to_string()),
            }),
            ..Default::default()
        };
        let mut incoming = DockerConfig {
            auth: Some(DockerAuth::Basic {
                username: "user".to_string(),
                password: SecretString::new("new_password".to_string()),
            }),
            ..Default::default()
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
    fn auth_omitted_when_none() {
        let config = DockerConfig::default();
        let json = serde_json::to_string(&config).expect("serialize");
        assert!(!json.contains("auth"));
    }

    #[test]
    fn serialization_roundtrip() {
        let config = DockerConfig {
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
            compose_restart: Some(ComposeRestartConfig {
                compose_file: Some("docker-compose.yml".to_string()),
                service: Some("app".to_string()),
                working_dir: Some("/opt/app".to_string()),
            }),
            post_pull_command: Some("systemctl restart myapp".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let deserialized: DockerConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.tracking_mode, config.tracking_mode);
        assert_eq!(deserialized.tag_patterns, config.tag_patterns);
        assert_eq!(deserialized.include_prereleases, config.include_prereleases);
        assert_eq!(deserialized.tracked_tag, config.tracked_tag);
        assert_eq!(deserialized.page_size, config.page_size);
        assert_eq!(deserialized.post_pull_command, config.post_pull_command);
    }
}
