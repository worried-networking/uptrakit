//! Runtime connection context injected by agents into plugin creation.
//!
//! [`ConnectionContext`] allows agents — in particular `agent-ssh` — to override
//! plugin configuration fields that depend on transport-level knowledge (e.g.
//! which Docker daemon endpoint to reach).  It is constructed once per host
//! connection and threaded through all handler functions so that plugins can
//! be pointed at the right daemon without modifying the user-visible config JSON
//! stored in the database.

use std::sync::Arc;

use uptrakit_plugin_infrastructure_registry::PluginType;

/// Runtime connection details injected by the agent into plugin creation.
///
/// # Merging into plugin config
///
/// Before the plugin registry deserializes a plugin configuration, call
/// [`apply_to_config`] to merge applicable fields from this context into the
/// config JSON.  Fields that are already present in the user-visible config
/// are left unchanged (user config takes precedence).
///
/// # Default
///
/// [`ConnectionContext::default()`] (all fields `None` / empty) is used by the
/// local `agent` and in tests — it has no effect on plugin configuration.
#[derive(Clone, Default)]
pub struct ConnectionContext {
    /// Docker daemon endpoint (overrides `DockerConfig.docker_host` when not
    /// already set by the user in their config).
    ///
    /// Populated by `agent-ssh`:
    /// `"ssh://user@host:port"` — points bollard at the remote daemon via
    /// the system `ssh` binary (bollard `ssh` feature / `openssh` crate).
    pub docker_host_override: Option<String>,

    /// Path to the SSH private key file used when `docker_host_override` is
    /// an `ssh://` URI.
    ///
    /// Injected by `agent-ssh` so bollard can authenticate to the remote
    /// Docker daemon using the host's stored key.  When `None`, bollard falls
    /// back to the default SSH key locations (`~/.ssh/id_ed25519`, SSH
    /// agent, etc.).
    pub ssh_key_path: Option<std::path::PathBuf>,

    /// Opaque RAII handles kept alive for the lifetime of any operation that
    /// uses this context.
    ///
    /// `agent-ssh` uses this to carry cleanup handles (e.g. a temporary SSH
    /// private key file) into spawned update tasks, ensuring they are not
    /// dropped prematurely.  Because `ConnectionContext` is `Clone`, cloning
    /// it increments the `Arc` reference counts inside this field; handles
    /// are cleaned up only when the last clone is dropped.
    ///
    /// The local agent and tests always leave this empty.
    pub keep_alive: Vec<Arc<dyn std::any::Any + Send + Sync>>,
}

impl std::fmt::Debug for ConnectionContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionContext")
            .field("docker_host_override", &self.docker_host_override)
            .field("ssh_key_path", &self.ssh_key_path)
            .field(
                "keep_alive",
                &format_args!("[{} handle(s)]", self.keep_alive.len()),
            )
            .finish()
    }
}

impl ConnectionContext {
    /// Merge applicable fields from this context into a plugin config JSON.
    ///
    /// Only the fields relevant to `plugin_type` are applied.  Fields that
    /// already exist in `config` are preserved (user config takes precedence).
    pub fn apply_to_config(&self, plugin_type: &PluginType, config: &mut serde_json::Value) {
        if !matches!(plugin_type, PluginType::ReleasesDocker) {
            return;
        }
        let Some(obj) = config.as_object_mut() else {
            return;
        };

        if !obj.contains_key("docker_host")
            && let Some(ref host) = self.docker_host_override
        {
            obj.insert("docker_host".to_string(), serde_json::json!(host));
        }

        if !obj.contains_key("ssh_key_path")
            && let Some(ref path) = self.ssh_key_path
        {
            obj.insert(
                "ssh_key_path".to_string(),
                serde_json::json!(path.to_string_lossy().as_ref()),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_context_is_all_none() {
        let ctx = ConnectionContext::default();
        assert!(ctx.docker_host_override.is_none());
        assert!(ctx.ssh_key_path.is_none());
        assert!(ctx.keep_alive.is_empty());
    }

    #[test]
    fn apply_injects_docker_host_for_docker_plugin() {
        let ctx = ConnectionContext {
            docker_host_override: Some("ssh://user@host:2222".to_string()),
            ssh_key_path: Some(std::path::PathBuf::from("/home/user/.ssh/id_ed25519")),
            ..Default::default()
        };
        let mut config = serde_json::json!({});

        ctx.apply_to_config(&PluginType::ReleasesDocker, &mut config);

        assert_eq!(config["docker_host"], "ssh://user@host:2222");
        assert_eq!(config["ssh_key_path"], "/home/user/.ssh/id_ed25519");
    }

    #[test]
    fn apply_does_not_overwrite_existing_docker_host() {
        let ctx = ConnectionContext {
            docker_host_override: Some("ssh://user@host:2222".to_string()),
            ..Default::default()
        };
        let mut config = serde_json::json!({ "docker_host": "unix:///custom.sock" });

        ctx.apply_to_config(&PluginType::ReleasesDocker, &mut config);

        // Existing value must be preserved
        assert_eq!(config["docker_host"], "unix:///custom.sock");
    }

    #[test]
    fn apply_does_nothing_for_non_docker_plugins() {
        let ctx = ConnectionContext {
            docker_host_override: Some("ssh://user@host:2222".to_string()),
            ..Default::default()
        };
        let mut config = serde_json::json!({});

        ctx.apply_to_config(&PluginType::ReleasesGithub, &mut config);

        assert!(config.get("docker_host").is_none());
    }

    #[test]
    fn keep_alive_clone_shares_arc() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static DROPPED: AtomicUsize = AtomicUsize::new(0);

        struct DropCounter;
        impl Drop for DropCounter {
            fn drop(&mut self) {
                DROPPED.fetch_add(1, Ordering::Relaxed);
            }
        }

        let counter = Arc::new(DropCounter);
        let ctx = ConnectionContext {
            keep_alive: vec![counter as Arc<dyn std::any::Any + Send + Sync>],
            ..Default::default()
        };

        // Clone shares the Arc — not yet dropped.
        let cloned = ctx.clone();
        assert_eq!(DROPPED.load(Ordering::Relaxed), 0);

        // Drop the original — still not dropped (cloned holds it).
        drop(ctx);
        assert_eq!(DROPPED.load(Ordering::Relaxed), 0);

        // Drop the clone — now the counter is dropped.
        drop(cloned);
        assert_eq!(DROPPED.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn apply_default_context_leaves_config_unchanged() {
        let ctx = ConnectionContext::default();
        let original = serde_json::json!({ "tracking_mode": "semver_tags" });
        let mut config = original.clone();

        ctx.apply_to_config(&PluginType::ReleasesDocker, &mut config);

        assert_eq!(config, original);
    }
}
