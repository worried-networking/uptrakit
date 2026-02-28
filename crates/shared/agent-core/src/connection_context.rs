//! Runtime connection context injected by agents into plugin creation.
//!
//! [`ConnectionContext`] carries RAII handles that must stay alive for the
//! duration of an operation (e.g. spawned update tasks). Docker daemon
//! connectivity is now handled by the [`StdioTunnel`] abstraction on the
//! executor — the context no longer overrides plugin configuration fields.

use std::sync::Arc;

/// Runtime connection details injected by the agent into plugin creation.
///
/// # Default
///
/// [`ConnectionContext::default()`] (empty `keep_alive` vec) is used by the
/// local `agent` and in tests — it has no effect on plugin configuration.
#[derive(Clone, Default)]
pub struct ConnectionContext {
    /// Opaque RAII handles kept alive for the lifetime of any operation that
    /// uses this context.
    ///
    /// Because `ConnectionContext` is `Clone`, cloning it increments the `Arc`
    /// reference counts inside this field; handles are cleaned up only when the
    /// last clone is dropped.
    ///
    /// The local agent and tests always leave this empty.
    pub keep_alive: Vec<Arc<dyn std::any::Any + Send + Sync>>,
}

impl std::fmt::Debug for ConnectionContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionContext")
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
    /// Currently a no-op: Docker daemon connectivity is handled via the
    /// executor's [`StdioTunnel`] support rather than config overrides.
    /// This method is retained for forward compatibility with future
    /// transport-level config injection needs.
    pub fn apply_to_config(
        &self,
        _plugin_type: &uptrakit_plugin_infrastructure_registry::PluginType,
        _config: &mut serde_json::Value,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_registry::PluginType;

    #[test]
    fn default_context_is_empty() {
        let ctx = ConnectionContext::default();
        assert!(ctx.keep_alive.is_empty());
    }

    #[test]
    fn apply_is_noop() {
        let ctx = ConnectionContext::default();
        let original = serde_json::json!({ "tracking_mode": "semver_tags" });
        let mut config = original.clone();

        ctx.apply_to_config(&PluginType::ReleasesDocker, &mut config);

        assert_eq!(config, original);
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
}
