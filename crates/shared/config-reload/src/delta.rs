use std::sync::Arc;

use crate::config::{
    AuditConfig, DbConfig, EmbeddedServicesConfig, NatsConfig, NetworkConfig, TlsConfig,
    ZeroconfConfig,
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
}
