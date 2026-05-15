use std::sync::Arc;

use crate::config::{
    AuditConfig, DbConfig, EmbeddedServicesConfig, NatsConfig, NetworkConfig, PluginsConfig,
    TlsConfig, ZeroconfConfig,
};

/// In-process delta carrying the new value for one config section.
///
/// Wire-incompatible by design: `RuntimeConfigDelta` is **never serialised**.
/// Each variant wraps the new section value in an [`Arc`] so that receivers
/// can clone cheaply without copying the entire section.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum RuntimeConfigDelta {
    /// New database connection and pool settings.
    Db(Arc<DbConfig>),
    /// New network listener settings (HTTPS + PKI).
    Network(Arc<NetworkConfig>),
    /// New NATS messaging server settings.
    Nats(Arc<NatsConfig>),
    /// New TLS certificate and key settings.
    Tls(Arc<TlsConfig>),
    /// New audit log settings.
    Audit(Arc<AuditConfig>),
    /// New zero-configuration auto-discovery settings.
    Zeroconf(Arc<ZeroconfConfig>),
    /// New embedded-services toggle settings.
    EmbeddedServices(Arc<EmbeddedServicesConfig>),
    /// Plugin settings reload signal (DB-driven; version counter incremented by
    /// `ConfigReconciler` on each plugin settings change).
    Plugins(Arc<PluginsConfig>),
    /// Signal `PluginsReloadable` to re-read plugin configuration from the DB.
    ///
    /// Unlike `Plugins(Arc<PluginsConfig>)`, this variant carries no config
    /// payload — the distinction is structural, eliminating the sentinel-value
    /// anti-pattern where callers passed `PluginsConfig::default()` as a trigger.
    PluginsDbRefresh,
}

impl RuntimeConfigDelta {
    /// Return a stable `&'static str` discriminant for this variant.
    ///
    /// Used by `dedup_deltas` to deduplicate lists by variant tag without
    /// requiring `PartialEq` on the payload types.
    #[must_use]
    pub fn variant_tag(&self) -> &'static str {
        match self {
            Self::Db(_) => "Db",
            Self::Network(_) => "Network",
            Self::Nats(_) => "Nats",
            Self::Tls(_) => "Tls",
            Self::Audit(_) => "Audit",
            Self::Zeroconf(_) => "Zeroconf",
            Self::EmbeddedServices(_) => "EmbeddedServices",
            Self::Plugins(_) => "Plugins",
            Self::PluginsDbRefresh => "PluginsDbRefresh",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn variant_tags_are_unique() {
        let tags: HashSet<&str> = vec![
            RuntimeConfigDelta::Db(Arc::new(Default::default())).variant_tag(),
            RuntimeConfigDelta::Network(Arc::new(Default::default())).variant_tag(),
            RuntimeConfigDelta::Nats(Arc::new(Default::default())).variant_tag(),
            RuntimeConfigDelta::Tls(Arc::new(Default::default())).variant_tag(),
            RuntimeConfigDelta::Audit(Arc::new(Default::default())).variant_tag(),
            RuntimeConfigDelta::Zeroconf(Arc::new(Default::default())).variant_tag(),
            RuntimeConfigDelta::EmbeddedServices(Arc::new(Default::default())).variant_tag(),
            RuntimeConfigDelta::Plugins(Arc::new(Default::default())).variant_tag(),
            RuntimeConfigDelta::PluginsDbRefresh.variant_tag(),
        ]
        .into_iter()
        .collect();
        assert_eq!(tags.len(), 9, "every variant must have a unique tag");
    }
}
