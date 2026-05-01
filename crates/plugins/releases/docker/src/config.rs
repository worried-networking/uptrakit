use std::collections::HashMap;

use rootcause::prelude::*;
use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::{
    PluginConfig, PluginConfigValidationError, SecretString,
};

use crate::error::{DockerError, Result};

/// Sentinel value used to indicate a masked secret in API responses.
const SECRET_MASK: &str = "***";

/// Selects the container runtime used for `dial-stdio` tunnelling over SSH.
///
/// Only relevant when the plugin is used via an SSH executor that supports
/// stdio tunnels (e.g. `agent-ssh`). For local connections bollard connects
/// to a socket directly without invoking a runtime CLI.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContainerRuntime {
    /// Probe the remote host for Docker, then Podman; use whichever responds first.
    #[default]
    Auto,
    /// Always invoke `docker system dial-stdio`.
    Docker,
    /// Always invoke `podman system dial-stdio`.
    Podman,
}

fn is_auto_runtime(r: &ContainerRuntime) -> bool {
    *r == ContainerRuntime::Auto
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

/// TLS certificate configuration for encrypted TCP connections to a remote
/// Docker daemon (`tcp://` with `--tlsverify`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DockerTlsConfig {
    /// Path to the CA certificate used to verify the daemon's certificate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_cert_path: Option<String>,
    /// Path to the client certificate for mutual TLS authentication.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_cert_path: Option<String>,
    /// Path to the client private key for mutual TLS authentication.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_key_path: Option<String>,
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
#[non_exhaustive]
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

    /// Include only containers that have ALL of these labels with matching values.
    ///
    /// An empty map (the default) means no filter — all containers are included.
    /// Keys must be non-empty and ≤ 253 characters; values must be ≤ 4 096 characters.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub include_labels: HashMap<String, String>,

    /// Exclude containers that have ANY of these labels with matching values.
    ///
    /// An empty map (the default) means no filter. Applied after `include_labels`.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub exclude_labels: HashMap<String, String>,

    /// TLS configuration for encrypted TCP connections to a remote Docker daemon.
    ///
    /// Only used when `docker_host` starts with `tcp://` or `http://`.
    /// When set, the plugin connects using `bollard::Docker::connect_with_ssl()`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls: Option<DockerTlsConfig>,

    /// Container runtime to use for SSH stdio tunnelling.
    ///
    /// In `Auto` mode (the default) the agent probes the remote host for Docker
    /// first, then Podman, and uses whichever is found. Set explicitly to
    /// `"docker"` or `"podman"` to skip auto-detection.
    #[serde(default, skip_serializing_if = "is_auto_runtime")]
    pub container_runtime: ContainerRuntime,

    /// When `true`, read registry credentials from the Docker credential store
    /// (`~/.docker/config.json` on the host where this plugin executes).
    ///
    /// **Credential resolution order:**
    /// 1. Explicit `auth` in this config (always wins when set).
    /// 2. `~/.docker/config.json` on the execution host:
    ///    - `credHelpers.<registry>` → invoke `docker-credential-{helper} get`
    ///    - `auths.<registry>` → base64-decoded Basic auth
    /// 3. Unauthenticated (if nothing found).
    ///
    /// Defaults to `false` to avoid unexpected reads from the credential store.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub use_system_credentials: bool,

    /// OCI platform to use for multi-arch image version checking.
    ///
    /// When set, version detection and release fetching compare the
    /// platform-specific manifest digest instead of the image index digest.
    /// This prevents false-positive update notifications when only a different
    /// platform's image is updated.
    ///
    /// Format: `os/arch` or `os/arch/variant` (e.g. `linux/amd64`, `linux/arm/v7`).
    /// When absent, the image-index digest is used (backwards-compatible behaviour).
    /// Auto-detected during discovery from the locally installed image's architecture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,

    /// Indicate that a successful update requires a host restart to take effect.
    ///
    /// When `true`, the controller transitions the software item to
    /// `AwaitingRestart` after a successful update instead of marking it
    /// `UpToDate` immediately. The item moves to `UpToDate` once the agent
    /// reports a digest that matches the pulled image (i.e. after the host has
    /// restarted and the new container is running).
    ///
    /// Defaults to `false`. Set to `true` for images where the running
    /// container is only replaced after an explicit restart or reboot.
    #[serde(default)]
    pub resumable: bool,
}

/// Compose an OCI platform string from Bollard image inspect fields.
///
/// Returns `None` when `os` or `architecture` are unavailable.
/// Examples: `("linux", "amd64", None)` → `"linux/amd64"`,
///           `("linux", "arm", Some("v7"))` → `"linux/arm/v7"`.
pub(crate) fn form_platform_string(
    os: Option<&str>,
    architecture: Option<&str>,
    variant: Option<&str>,
) -> Option<String> {
    match (os, architecture) {
        (Some(os), Some(arch)) => {
            if let Some(v) = variant {
                Some(format!("{os}/{arch}/{v}"))
            } else {
                Some(format!("{os}/{arch}"))
            }
        }
        _ => None,
    }
}

impl DockerConfig {
    /// Validate the configuration (internal, returns crate error type).
    ///
    /// An empty (all-defaults) config is valid — discovery can proceed
    /// without any fields set.
    pub(crate) fn validate_inner(&self) -> Result<()> {
        if let Some(ref cmd) = self.post_pull_command
            && let Err(e) = uptrakit_shared_types::command_validation::validate_command_length(
                cmd,
                "post_pull_command",
            )
        {
            bail!(DockerError::Configuration(e));
        }

        if let Some(ref cr) = self.compose_restart {
            if let Some(ref file) = cr.compose_file
                && file.split('/').any(|seg| seg == "..")
            {
                bail!(DockerError::Configuration(
                    "compose_file must not contain '..' path segments".to_string()
                ));
            }
            if let Some(ref dir) = cr.working_dir
                && dir.split('/').any(|seg| seg == "..")
            {
                bail!(DockerError::Configuration(
                    "working_dir must not contain '..' path segments".to_string()
                ));
            }
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

        if let Some(ref tls) = self.tls {
            for (field, path) in [
                ("tls.ca_cert_path", tls.ca_cert_path.as_deref()),
                ("tls.client_cert_path", tls.client_cert_path.as_deref()),
                ("tls.client_key_path", tls.client_key_path.as_deref()),
            ] {
                if let Some(p) = path {
                    if p.is_empty() {
                        bail!(DockerError::Configuration(format!(
                            "{field} must not be empty when set"
                        )));
                    }
                    if p.split('/').any(|seg| seg == "..") {
                        bail!(DockerError::Configuration(format!(
                            "{field} must not contain '..' path segments"
                        )));
                    }
                }
            }
        }

        for (map_name, map) in [
            ("include_labels", &self.include_labels),
            ("exclude_labels", &self.exclude_labels),
        ] {
            for (key, value) in map {
                if key.is_empty() {
                    bail!(DockerError::Configuration(format!(
                        "{map_name}: label key must not be empty"
                    )));
                }
                if key.len() > 253 {
                    bail!(DockerError::Configuration(format!(
                        "{map_name}: label key '{key}' exceeds maximum length of 253 characters"
                    )));
                }
                if value.len() > 4096 {
                    bail!(DockerError::Configuration(format!(
                        "{map_name}: value for label '{key}' exceeds maximum length of 4096 characters"
                    )));
                }
            }
        }

        if let Some(ref platform) = self.platform {
            // OCI platform format: os/arch or os/arch/variant
            let valid = {
                let parts: Vec<&str> = platform.splitn(3, '/').collect();
                parts.len() >= 2
                    && parts.iter().all(|p| !p.is_empty())
                    && parts.iter().all(|p| {
                        p.chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
                    })
            };
            if !valid {
                bail!(DockerError::Configuration(format!(
                    "platform '{platform}' is not a valid OCI platform string \
                     (expected os/arch or os/arch/variant, e.g. linux/amd64, linux/arm/v7)"
                )));
            }
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
}

impl PluginConfig for DockerConfig {
    fn validate(&self) -> std::result::Result<(), PluginConfigValidationError> {
        self.validate_inner()
            .map_err(|e| PluginConfigValidationError::Contract(e.to_string()))
    }

    fn validate_identifier(value: &str) -> std::result::Result<(), PluginConfigValidationError> {
        crate::validate_identifier(value).map_err(PluginConfigValidationError::InvalidIdentifier)
    }

    fn form_schema() -> Vec<uptrakit_plugin_infrastructure_core::form_schema::FormFieldDescriptor> {
        use uptrakit_plugin_infrastructure_core::form_schema::{
            FormFieldDescriptor, FormFieldType, FormSelectOptionDescriptor,
        };
        vec![
            FormFieldDescriptor::new("docker_host", "Docker Host")
                .with_placeholder("unix:///var/run/docker.sock")
                .with_help_text("Docker daemon endpoint override (tcp://, unix://, or ssh://)"),
            FormFieldDescriptor::new("container_runtime", "Container Runtime")
                .with_type(FormFieldType::Select)
                .with_options(vec![
                    FormSelectOptionDescriptor::new("auto", "Auto-detect"),
                    FormSelectOptionDescriptor::new("docker", "Docker"),
                    FormSelectOptionDescriptor::new("podman", "Podman"),
                ])
                .with_help_text("Container runtime for SSH dial-stdio tunnelling (auto = probe Docker then Podman)"),
            FormFieldDescriptor::new("ssh_key_path", "SSH Key Path")
                .with_help_text("Path to SSH private key (only for ssh:// docker hosts)"),
            FormFieldDescriptor::new("auth._type", "Registry Auth")
                .with_type(FormFieldType::Select)
                .with_options(vec![
                    FormSelectOptionDescriptor::new("", "None"),
                    FormSelectOptionDescriptor::new("basic", "Basic (username/password)"),
                    FormSelectOptionDescriptor::new("bearer", "Bearer Token"),
                ]),
            FormFieldDescriptor::new("auth.username", "Registry Username")
                .with_visible_when("auth._type", vec!["basic".to_string()]),
            FormFieldDescriptor::new("auth.password", "Registry Password")
                .with_type(FormFieldType::Password)
                .sensitive()
                .with_visible_when("auth._type", vec!["basic".to_string()]),
            FormFieldDescriptor::new("auth.token", "Registry Token")
                .with_type(FormFieldType::Password)
                .sensitive()
                .with_visible_when("auth._type", vec!["bearer".to_string()]),
            FormFieldDescriptor::new("tracked_tag", "Tracked Tag")
                .with_help_text("Tag to track (overrides the tag in the image reference)"),
            FormFieldDescriptor::new("platform", "Platform")
                .with_placeholder("linux/amd64")
                .with_help_text(
                    "OCI platform for multi-arch images (e.g. linux/amd64, linux/arm/v7). \
                     Auto-detected during discovery; override only when needed.",
                ),
            FormFieldDescriptor::new("post_pull_command", "Post-pull Command").with_help_text(
                "Shell command to run after pulling (supports {image}, {tag}, {digest})",
            ),
            FormFieldDescriptor::new("compose_restart._enabled", "Compose Restart")
                .with_type(FormFieldType::Toggle)
                .with_help_text("Restart via docker compose after pulling a new image"),
            FormFieldDescriptor::new("compose_restart.compose_file", "Compose File")
                .with_placeholder("docker-compose.yml")
                .with_help_text("Path to the Compose file")
                .with_visible_when("compose_restart._enabled", vec!["true".to_string()]),
            FormFieldDescriptor::new("compose_restart.service", "Compose Service")
                .with_help_text("Specific service to restart (blank = all services)")
                .with_visible_when("compose_restart._enabled", vec!["true".to_string()]),
            FormFieldDescriptor::new("compose_restart.working_dir", "Compose Working Dir")
                .with_help_text("Working directory for docker compose")
                .with_visible_when("compose_restart._enabled", vec!["true".to_string()]),
            FormFieldDescriptor::new("resumable", "Resumable (Restart Required)")
                .with_type(FormFieldType::Toggle)
                .with_help_text("Mark the update as requiring a host restart to take effect. The item stays in AwaitingRestart until the new container is running after reboot."),
        ]
    }

    fn with_secrets_masked(mut self) -> Self {
        self.auth = self.auth.map(|a| match a {
            DockerAuth::Basic { username, .. } => DockerAuth::Basic {
                username,
                password: SecretString::new(SECRET_MASK),
            },
            DockerAuth::Bearer { .. } => DockerAuth::Bearer {
                token: SecretString::new(SECRET_MASK),
            },
        });
        self
    }

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
            ) if incoming_pw.expose_secret() == SECRET_MASK => {
                *incoming_pw = existing_pw.clone();
            }
            (
                DockerAuth::Bearer {
                    token: incoming_token,
                },
                DockerAuth::Bearer {
                    token: existing_token,
                },
            ) if incoming_token.expose_secret() == SECRET_MASK => {
                *incoming_token = existing_token.clone();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── validate ─────────────────────────────────────────────────────────────

    #[test]
    fn empty_config_validates_ok() {
        let config = DockerConfig::default();
        assert!(config.validate_inner().is_ok());
    }

    #[test]
    fn validation_fails_empty_post_pull_command() {
        let config = DockerConfig {
            post_pull_command: Some(String::new()),
            ..Default::default()
        };
        let err = config.validate_inner().unwrap_err();
        assert!(err.to_string().contains("post_pull_command"));
    }

    #[test]
    fn validation_fails_post_pull_command_over_limit() {
        let cmd = "x".repeat(uptrakit_shared_types::command_validation::MAX_COMMAND_LENGTH + 1);
        let config = DockerConfig {
            post_pull_command: Some(cmd),
            ..Default::default()
        };
        let err = config.validate_inner().unwrap_err();
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
        assert!(config.validate_inner().is_ok());
    }

    #[test]
    fn validation_passes_non_empty_post_pull_command() {
        let config = DockerConfig {
            post_pull_command: Some("docker compose up -d".to_string()),
            ..Default::default()
        };
        assert!(config.validate_inner().is_ok());
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
        let err = config.validate_inner().unwrap_err();
        assert!(err.to_string().contains("compose_file"));
    }

    #[test]
    fn validation_rejects_working_dir_with_path_traversal() {
        let config = DockerConfig {
            compose_restart: Some(ComposeRestartConfig {
                compose_file: None,
                service: None,
                working_dir: Some("../../../etc".to_string()),
            }),
            ..Default::default()
        };
        let err = config.validate_inner().unwrap_err();
        assert!(err.to_string().contains("working_dir"));
    }

    #[test]
    fn validation_accepts_valid_working_dir() {
        let config = DockerConfig {
            compose_restart: Some(ComposeRestartConfig {
                compose_file: None,
                service: None,
                working_dir: Some("/opt/app/docker".to_string()),
            }),
            ..Default::default()
        };
        assert!(config.validate_inner().is_ok());
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
        assert!(config.validate_inner().is_ok());
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
                password: SecretString::new("secret123"),
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
                token: SecretString::new("ghcr_token"),
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
                password: SecretString::new("real_password"),
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
                token: SecretString::new("real_token"),
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
                password: SecretString::new("old_password"),
            }),
            ..Default::default()
        };
        let mut incoming = DockerConfig {
            auth: Some(DockerAuth::Basic {
                username: "user".to_string(),
                password: SecretString::new("new_password"),
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
            password: SecretString::new("pass"),
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
            token: SecretString::new("my-token"),
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
                password: SecretString::new("pass"),
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

    // ── Label filter validation ──────────────────────────────────────────────

    #[test]
    fn validation_rejects_empty_label_key() {
        let mut config = DockerConfig::default();
        config
            .include_labels
            .insert(String::new(), "val".to_string());
        assert!(
            config
                .validate()
                .unwrap_err()
                .to_string()
                .contains("must not be empty")
        );
    }

    #[test]
    fn validation_rejects_label_key_over_253() {
        let mut config = DockerConfig::default();
        config
            .include_labels
            .insert("x".repeat(254), "val".to_string());
        assert!(
            config
                .validate_inner()
                .unwrap_err()
                .to_string()
                .contains("253")
        );
    }

    #[test]
    fn validation_rejects_label_value_over_4096() {
        let mut config = DockerConfig::default();
        config
            .include_labels
            .insert("key".to_string(), "v".repeat(4097));
        assert!(
            config
                .validate_inner()
                .unwrap_err()
                .to_string()
                .contains("4096")
        );
    }

    #[test]
    fn validation_passes_valid_labels() {
        let mut config = DockerConfig::default();
        config
            .include_labels
            .insert("com.example.managed".to_string(), "true".to_string());
        assert!(config.validate_inner().is_ok());
    }

    // ── DockerTlsConfig validation ───────────────────────────────────────────

    #[test]
    fn validation_rejects_tls_ca_cert_with_dotdot() {
        let config = DockerConfig {
            docker_host: Some("tcp://host:2376".to_string()),
            tls: Some(DockerTlsConfig {
                ca_cert_path: Some("../etc/ca.pem".to_string()),
                client_cert_path: None,
                client_key_path: None,
            }),
            ..Default::default()
        };
        let err = config.validate_inner().unwrap_err();
        assert!(err.to_string().contains("ca_cert_path"));
    }

    #[test]
    fn validation_rejects_empty_tls_path() {
        let config = DockerConfig {
            tls: Some(DockerTlsConfig {
                ca_cert_path: Some(String::new()),
                client_cert_path: None,
                client_key_path: None,
            }),
            ..Default::default()
        };
        let err = config.validate_inner().unwrap_err();
        assert!(err.to_string().contains("ca_cert_path"));
    }

    #[test]
    fn validation_passes_valid_tls_config() {
        let config = DockerConfig {
            docker_host: Some("tcp://host:2376".to_string()),
            tls: Some(DockerTlsConfig {
                ca_cert_path: Some("/etc/docker/ca.pem".to_string()),
                client_cert_path: Some("/etc/docker/cert.pem".to_string()),
                client_key_path: Some("/etc/docker/key.pem".to_string()),
            }),
            ..Default::default()
        };
        assert!(config.validate_inner().is_ok());
    }

    // ── ContainerRuntime ──────────────────────────────────────────────────────

    #[test]
    fn container_runtime_default_is_auto() {
        assert_eq!(
            DockerConfig::default().container_runtime,
            ContainerRuntime::Auto
        );
    }

    #[test]
    fn container_runtime_serialization_roundtrip() {
        let config = DockerConfig {
            container_runtime: ContainerRuntime::Podman,
            ..Default::default()
        };
        let json = serde_json::to_string(&config).expect("serialize");
        assert!(json.contains(r#""container_runtime":"podman""#));
        let de: DockerConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(de.container_runtime, ContainerRuntime::Podman);
    }

    #[test]
    fn container_runtime_omitted_when_auto() {
        let config = DockerConfig::default();
        let json = serde_json::to_string(&config).expect("serialize");
        assert!(
            !json.contains("container_runtime"),
            "auto should be omitted: {json}"
        );
    }

    #[test]
    fn use_system_credentials_defaults_to_false() {
        assert!(!DockerConfig::default().use_system_credentials);
    }

    // ── platform field ───────────────────────────────────────────────────────

    #[test]
    fn validation_accepts_valid_platform_amd64() {
        let config = DockerConfig {
            platform: Some("linux/amd64".to_string()),
            ..Default::default()
        };
        assert!(config.validate_inner().is_ok());
    }

    #[test]
    fn validation_accepts_valid_platform_armv7() {
        let config = DockerConfig {
            platform: Some("linux/arm/v7".to_string()),
            ..Default::default()
        };
        assert!(config.validate_inner().is_ok());
    }

    #[test]
    fn validation_rejects_platform_with_spaces() {
        let config = DockerConfig {
            platform: Some("linux amd64".to_string()),
            ..Default::default()
        };
        assert!(
            config
                .validate()
                .unwrap_err()
                .to_string()
                .contains("platform")
        );
    }

    #[test]
    fn validation_rejects_platform_single_part() {
        let config = DockerConfig {
            platform: Some("linux".to_string()),
            ..Default::default()
        };
        assert!(
            config
                .validate()
                .unwrap_err()
                .to_string()
                .contains("platform")
        );
    }

    #[test]
    fn platform_omitted_when_none() {
        let config = DockerConfig::default();
        let json = serde_json::to_string(&config).expect("serialize");
        assert!(
            !json.contains("platform"),
            "platform should be omitted: {json}"
        );
    }

    #[test]
    fn platform_serialization_roundtrip() {
        let config = DockerConfig {
            platform: Some("linux/arm/v7".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let de: DockerConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(de.platform.as_deref(), Some("linux/arm/v7"));
    }

    #[test]
    fn form_platform_string_os_arch() {
        assert_eq!(
            super::form_platform_string(Some("linux"), Some("amd64"), None).as_deref(),
            Some("linux/amd64")
        );
    }

    #[test]
    fn form_platform_string_with_variant() {
        assert_eq!(
            super::form_platform_string(Some("linux"), Some("arm"), Some("v7")).as_deref(),
            Some("linux/arm/v7")
        );
    }

    #[test]
    fn form_platform_string_missing_arch() {
        assert!(super::form_platform_string(Some("linux"), None, None).is_none());
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
        assert!(config.docker_host.is_none());
        assert!(config.auth.is_none());
        assert!(config.tracked_tag.is_none());
    }
}
