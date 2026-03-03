use rootcause::prelude::*;
use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::{SecretMasking, SecretString};

use crate::error::{DockerError, Result};

/// Sentinel value used to indicate a masked secret in API responses.
const SECRET_MASK: &str = "***";

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

/// Configuration for the Docker plugin.
///
/// The `package_identifier` on each software item (a Docker image reference
/// such as `nginx` or `ghcr.io/owner/app:latest`) drives all registry
/// operations — no `image` field is needed in the config.
///
/// The Docker plugin always tracks the **SHA digest** of a specific tag.
/// The tag is resolved in this order:
/// 1. `tracked_tag` field in this config (explicit override).
/// 2. The tag embedded in `package_identifier` (e.g. `:latest` in
///    `nginx:latest`).
/// 3. `"latest"` as the fallback default when neither is set.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
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

    /// Tag to track, overriding the tag embedded in `package_identifier`.
    ///
    /// When absent, the tag is taken from `package_identifier` (defaulting to
    /// `"latest"` if the identifier has no tag component).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracked_tag: Option<String>,

    /// Restart via `docker compose` after pulling a new image.
    ///
    /// Runs `[cd {working_dir} &&] docker compose [-f {file}] up -d [{service}]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compose_restart: Option<ComposeRestartConfig>,

    /// Shell command to run after pulling the new image.
    ///
    /// Supports `{image}`, `{tag}`, and `{digest}` placeholders.
    /// If absent (and `compose_restart` is also absent), the plugin
    /// automatically recreates all containers that use this image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_pull_command: Option<String>,
}

impl DockerConfig {
    /// Validate a Docker image reference (package identifier) string.
    ///
    /// Delegates to [`crate::image_ref::validate_identifier`]. A valid reference
    /// must be non-empty and must not contain whitespace or control characters.
    ///
    /// Called by the plugin registry's `validate_package_identifier` dispatch.
    pub fn validate_identifier(value: &str) -> std::result::Result<(), String> {
        crate::validate_identifier(value)
    }

    /// Validate the configuration.
    ///
    /// An empty (all-defaults) config is valid — discovery can proceed
    /// without any fields set.
    pub fn validate(&self) -> Result<()> {
        if let Some(ref cmd) = self.post_pull_command
            && let Err(e) =
                uptrakit_shared_types::command_validation::validate_command_length(
                    cmd,
                    "post_pull_command",
                )
        {
            bail!(DockerError::Configuration(e));
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

    /// Returns the configured tag override, if any.
    ///
    /// When `Some`, this value takes priority over the tag in
    /// `package_identifier`. When `None`, callers should fall back to the tag
    /// parsed from `package_identifier` (which itself defaults to `"latest"`).
    pub(crate) fn resolved_tracked_tag<'a>(&'a self, image_tag: &'a str) -> &'a str {
        self.tracked_tag.as_deref().unwrap_or(image_tag)
    }

    /// Returns `true` when the config is at all defaults — i.e. it was
    /// produced by deserializing an empty JSON object `{}`.
    ///
    /// The server sends an empty config with `plugin_config_id: None` when no
    /// pre-existing Docker plugin config exists for the tenant. `discover_software()`
    /// uses this to decide whether to emit [`uptrakit_plugin_infrastructure_core::DiscoveryTarget`]
    /// values so the controller can auto-create the default plugin config and
    /// role assignments.  When a real config is present the server sends
    /// `plugin_config_id: Some(_)` and the items are processed via the
    /// config-ID path (no targets needed).
    pub(crate) fn is_discover_all_mode(&self) -> bool {
        self.docker_host.is_none()
            && self.ssh_key_path.is_none()
            && self.auth.is_none()
            && self.tracked_tag.is_none()
            && self.compose_restart.is_none()
            && self.post_pull_command.is_none()
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

    // ── is_discover_all_mode ──────────────────────────────────────────────────

    #[test]
    fn is_discover_all_mode_true_for_default_config() {
        assert!(DockerConfig::default().is_discover_all_mode());
    }

    #[test]
    fn is_discover_all_mode_false_when_docker_host_set() {
        let config = DockerConfig {
            docker_host: Some("tcp://host:2375".to_string()),
            ..Default::default()
        };
        assert!(!config.is_discover_all_mode());
    }

    #[test]
    fn is_discover_all_mode_false_when_auth_set() {
        let config = DockerConfig {
            auth: Some(DockerAuth::Basic {
                username: "user".to_string(),
                password: SecretString::new("pass".to_string()),
            }),
            ..Default::default()
        };
        assert!(!config.is_discover_all_mode());
    }

    #[test]
    fn is_discover_all_mode_false_when_tracked_tag_set() {
        let config = DockerConfig {
            tracked_tag: Some("stable".to_string()),
            ..Default::default()
        };
        assert!(!config.is_discover_all_mode());
    }

    #[test]
    fn is_discover_all_mode_false_when_compose_restart_set() {
        let config = DockerConfig {
            compose_restart: Some(ComposeRestartConfig {
                compose_file: None,
                service: None,
                working_dir: None,
            }),
            ..Default::default()
        };
        assert!(!config.is_discover_all_mode());
    }

    #[test]
    fn is_discover_all_mode_false_when_post_pull_command_set() {
        let config = DockerConfig {
            post_pull_command: Some("systemctl restart myapp".to_string()),
            ..Default::default()
        };
        assert!(!config.is_discover_all_mode());
    }

    // ── validate ─────────────────────────────────────────────────────────────

    #[test]
    fn empty_config_validates_ok() {
        let config = DockerConfig::default();
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
    fn validation_fails_post_pull_command_over_limit() {
        let cmd = "x".repeat(uptrakit_shared_types::command_validation::MAX_COMMAND_LENGTH + 1);
        let config = DockerConfig {
            post_pull_command: Some(cmd),
            ..Default::default()
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("post_pull_command"));
        assert!(err.to_string().contains("exceeds maximum length"));
    }

    #[test]
    fn validation_passes_post_pull_command_at_limit() {
        let cmd = "x".repeat(uptrakit_shared_types::command_validation::MAX_COMMAND_LENGTH);
        let config = DockerConfig {
            post_pull_command: Some(cmd),
            ..Default::default()
        };
        assert!(config.validate().is_ok());
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

    // ── resolved_tracked_tag ─────────────────────────────────────────────────

    #[test]
    fn resolved_tracked_tag_falls_back_to_image_tag() {
        let config = DockerConfig::default();
        assert_eq!(config.resolved_tracked_tag("latest"), "latest");
        assert_eq!(config.resolved_tracked_tag("stable"), "stable");
    }

    #[test]
    fn resolved_tracked_tag_config_override_wins() {
        let config = DockerConfig {
            tracked_tag: Some("main".to_string()),
            ..Default::default()
        };
        assert_eq!(config.resolved_tracked_tag("latest"), "main");
        assert_eq!(config.resolved_tracked_tag("stable"), "main");
    }

    // ── secret masking ───────────────────────────────────────────────────────

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
            tracked_tag: Some("main".to_string()),
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
        assert_eq!(deserialized.tracked_tag, config.tracked_tag);
        assert_eq!(deserialized.post_pull_command, config.post_pull_command);
    }

    #[test]
    fn old_semver_fields_silently_ignored_on_deserialize() {
        // Existing configs stored in DB may contain the now-removed
        // tracking_mode / tag_patterns / tag_strip_prefix / include_prereleases /
        // page_size fields.  They must be ignored gracefully.
        let json = serde_json::json!({
            "tracking_mode": "semver_tags",
            "tag_patterns": ["^v\\d+\\.\\d+\\.\\d+$"],
            "tag_strip_prefix": "v",
            "include_prereleases": false,
            "page_size": 500
        });
        let config: DockerConfig = serde_json::from_value(json).expect("deserialize");
        // Falls back to all defaults because semver fields are ignored
        assert!(config.is_discover_all_mode());
    }
}
