//! Boot-seeded `tokio::sync::watch` fan-out channels for runtime config sections.
//!
//! Each config section gets its own typed sender/receiver pair so that
//! subsystems can subscribe only to the sections they care about, avoiding
//! spurious wakeups.

use std::sync::Arc;

use tokio::sync::watch;
use uptrakit_shared_types::SecretString;

use crate::config::{
    AuditConfig, DbConfig, EmbeddedServicesConfig, LogConfig, NatsConfig, NetworkConfig,
    RuntimeConfig, TlsConfig, ZeroconfConfig,
};

/// Senders for all runtime config sections.
///
/// Held by the [`crate::coordinator::ReloadCoordinator`] — it calls
/// [`watch::Sender::send`] after a successful reload apply to push the new
/// values to all subscribers.
pub struct RuntimeConfigChannels {
    /// Database config sender.
    pub db: watch::Sender<Arc<DbConfig>>,
    /// Network config sender.
    pub network: watch::Sender<Arc<NetworkConfig>>,
    /// NATS config sender.
    pub nats: watch::Sender<Arc<NatsConfig>>,
    /// TLS config sender.
    pub tls: watch::Sender<Arc<TlsConfig>>,
    /// Audit config sender.
    pub audit: watch::Sender<Arc<AuditConfig>>,
    /// Boot-time only. Log path changes require reexec; no live delta variant.
    pub log: watch::Sender<Arc<LogConfig>>,
    /// Boot-time only. Master key changes require reexec; no live delta variant.
    pub master_key: watch::Sender<Arc<SecretString>>,
    /// Embedded services config sender.
    pub embedded_services: watch::Sender<Arc<EmbeddedServicesConfig>>,
    /// Zero-configuration auto-discovery config sender.
    pub zeroconf: watch::Sender<Arc<ZeroconfConfig>>,
}

/// Receivers for all runtime config sections.
///
/// Distributed to subsystems at startup. Each subsystem calls
/// [`watch::Receiver::borrow`] to read the current value or awaits
/// [`watch::Receiver::changed`] to react to live updates.
pub struct RuntimeConfigReceivers {
    /// Database config receiver.
    pub db: watch::Receiver<Arc<DbConfig>>,
    /// Network config receiver.
    pub network: watch::Receiver<Arc<NetworkConfig>>,
    /// NATS config receiver.
    pub nats: watch::Receiver<Arc<NatsConfig>>,
    /// TLS config receiver.
    pub tls: watch::Receiver<Arc<TlsConfig>>,
    /// Audit config receiver.
    pub audit: watch::Receiver<Arc<AuditConfig>>,
    /// Boot-time only. Log path changes require reexec; no live delta variant.
    pub log: watch::Receiver<Arc<LogConfig>>,
    /// Boot-time only. Master key changes require reexec; no live delta variant.
    pub master_key: watch::Receiver<Arc<SecretString>>,
    /// Embedded services config receiver.
    pub embedded_services: watch::Receiver<Arc<EmbeddedServicesConfig>>,
    /// Zero-configuration auto-discovery config receiver.
    pub zeroconf: watch::Receiver<Arc<ZeroconfConfig>>,
}

impl RuntimeConfigChannels {
    /// Create a matched sender/receiver pair seeded with the values from
    /// `runtime`.
    ///
    /// All receivers start as already-seen (no pending change notification).
    #[must_use]
    pub fn from_runtime(runtime: &RuntimeConfig) -> (Self, RuntimeConfigReceivers) {
        let (db_tx, db_rx) = watch::channel(Arc::new(runtime.db.clone()));
        let (net_tx, net_rx) = watch::channel(Arc::new(runtime.network.clone()));
        let (nats_tx, nats_rx) = watch::channel(Arc::new(runtime.nats.clone()));
        let (tls_tx, tls_rx) = watch::channel(Arc::new(runtime.tls.clone()));
        let (audit_tx, audit_rx) = watch::channel(Arc::new(runtime.audit.clone()));
        let (log_tx, log_rx) = watch::channel(Arc::new(runtime.log.clone()));
        let (mk_tx, mk_rx) = watch::channel(Arc::new(runtime.master_key.clone()));
        let (emb_tx, emb_rx) = watch::channel(Arc::new(runtime.embedded_services.clone()));
        let (zc_tx, zc_rx) = watch::channel(Arc::new(runtime.zeroconf.clone()));

        let senders = Self {
            db: db_tx,
            network: net_tx,
            nats: nats_tx,
            tls: tls_tx,
            audit: audit_tx,
            log: log_tx,
            master_key: mk_tx,
            embedded_services: emb_tx,
            zeroconf: zc_tx,
        };
        let receivers = RuntimeConfigReceivers {
            db: db_rx,
            network: net_rx,
            nats: nats_rx,
            tls: tls_rx,
            audit: audit_rx,
            log: log_rx,
            master_key: mk_rx,
            embedded_services: emb_rx,
            zeroconf: zc_rx,
        };
        (senders, receivers)
    }
}
