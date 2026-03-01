use std::fmt;
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use ipnet::IpNet;
use sea_orm::DatabaseConnection;
use uuid::Uuid;

use crate::SettingKey;
use crate::auth;
use crate::auth::authentication::AuthenticationSettings;
use crate::auth::registration::RegistrationSettings;
use crate::settings_store::{RawSettings, RawSettingsExt};

const DEFAULT_AGENT_CERT_LIFETIME_DAYS: u16 = 7;
const DEFAULT_RENEWAL_WINDOW_HOURS: u16 = 6;
const DEFAULT_MQTT_MAX_CLIENTS_PER_TENANT: u16 = 10;

/// Default listen addresses used when neither CLI nor DB provides a value.
pub const DEFAULT_HTTPS_ADDR: &str = "[::]:8443";
pub const DEFAULT_REAL_IP_HEADER: &str = "X-Forwarded-For";

fn warn_unrecognised_keys(raw: &RawSettings) {
    for key in raw.keys() {
        if SettingKey::from_db_key(key).is_none() {
            tracing::warn!(
                key,
                "unrecognised setting key in database — may be stale or misspelled"
            );
        }
    }
}

/// Network-related settings persisted in the DB and changeable at runtime
/// (except for listen addresses which require a restart).
#[derive(Clone, Debug)]
pub struct NetworkSettings {
    pub trusted_proxies: Vec<IpNet>,
    pub real_ip_header: String,
    pub extra_sans: Vec<String>,
    pub https_addr: SocketAddr,
    pub forwarded_client_cert_info_header: Option<String>,
    pub forwarded_client_cert_pem_header: Option<String>,
    pub pki_addr: Option<String>,
}

impl Default for NetworkSettings {
    fn default() -> Self {
        Self {
            trusted_proxies: Vec::new(),
            real_ip_header: DEFAULT_REAL_IP_HEADER.to_string(),
            extra_sans: Vec::new(),
            https_addr: SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 8443, 0, 0)),
            forwarded_client_cert_info_header: None,
            forwarded_client_cert_pem_header: None,
            pki_addr: None,
        }
    }
}

/// SMTP settings snapshot for sending email notifications.
///
/// Per-tenant: each tenant configures their own SMTP server. The password is
/// stored encrypted at rest in the `settings` table and decrypted into memory
/// here at load time. It is never returned over the API; callers receive
/// `has_password: bool` instead.
#[derive(Clone, Default)]
pub struct SmtpSettingsSnapshot {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    /// Decrypted SMTP password — never logged or sent over the API.
    pub password: Option<String>,
    pub from_address: Option<String>,
    pub from_name: Option<String>,
    /// TLS mode: `"starttls"` (default), `"tls"`, or `"none"`.
    pub tls_mode: String,
}

impl SmtpSettingsSnapshot {
    /// Returns `true` when the minimum required fields (host + from_address) are present.
    pub fn is_configured(&self) -> bool {
        self.host.as_ref().is_some_and(|h| !h.is_empty())
            && self.from_address.as_ref().is_some_and(|f| !f.is_empty())
    }
}

impl fmt::Debug for SmtpSettingsSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SmtpSettingsSnapshot")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("from_address", &self.from_address)
            .field("from_name", &self.from_name)
            .field("tls_mode", &self.tls_mode)
            .finish()
    }
}

/// Immutable snapshot of all settings. Published atomically via a watch channel
/// so readers never see a mix of old and new values.
#[derive(Clone, Debug)]
pub struct SettingsSnapshot {
    pub registration: RegistrationSettings,
    pub authentication: AuthenticationSettings,
    pub agent_cert_lifetime_days: u16,
    pub renewal_window_hours: u16,
    pub network: NetworkSettings,
    pub mqtt_max_clients_per_tenant: u16,
    pub smtp: SmtpSettingsSnapshot,
}

#[derive(Clone)]
pub struct Settings {
    inner: Arc<Inner>,
}

struct Inner {
    snapshot_tx: tokio::sync::watch::Sender<SettingsSnapshot>,
    snapshot_rx: tokio::sync::watch::Receiver<SettingsSnapshot>,
    /// Per-tenant settings version counter (for cross-instance invalidation).
    version: AtomicI64,
    /// Global settings version counter (for cross-instance invalidation).
    global_version: AtomicI64,
    /// Serialises writes so that concurrent set_* calls don't clobber each other.
    write_mutex: tokio::sync::Mutex<()>,
}

impl Settings {
    /// Construct from pre-loaded values (for tests).
    pub fn new(registration: RegistrationSettings, agent_cert_lifetime_days: u16) -> Self {
        Self::with_renewal_window(
            registration,
            agent_cert_lifetime_days,
            DEFAULT_RENEWAL_WINDOW_HOURS,
        )
    }

    /// Construct with all values (for tests or when loading from DB).
    pub fn with_renewal_window(
        registration: RegistrationSettings,
        agent_cert_lifetime_days: u16,
        renewal_window_hours: u16,
    ) -> Self {
        let snapshot = SettingsSnapshot {
            registration,
            authentication: AuthenticationSettings::default(),
            agent_cert_lifetime_days,
            renewal_window_hours,
            network: NetworkSettings::default(),
            mqtt_max_clients_per_tenant: DEFAULT_MQTT_MAX_CLIENTS_PER_TENANT,
            smtp: SmtpSettingsSnapshot::default(),
        };
        let (tx, rx) = tokio::sync::watch::channel(snapshot);
        Self {
            inner: Arc::new(Inner {
                snapshot_tx: tx,
                snapshot_rx: rx,
                version: AtomicI64::new(0),
                global_version: AtomicI64::new(0),
                write_mutex: tokio::sync::Mutex::new(()),
            }),
        }
    }

    /// Load all settings from DB (both global and per-tenant tables).
    /// Generates initial registration token if no users exist.
    ///
    /// Returns `(Settings, global_raw, tenant_raw, Option<plaintext_token>)` —
    /// the caller can pass the global raw map to reconciliation without
    /// re-reading from DB.
    pub async fn load(
        db: &DatabaseConnection,
        tenant_id: Uuid,
    ) -> auth::Result<(Self, RawSettings, RawSettings, Option<String>)> {
        let global_raw = crate::settings_store::load_all_global_settings(db).await?;
        let tenant_raw = crate::settings_store::load_all_settings(db, tenant_id).await?;

        // Merge: global settings first, then per-tenant overrides
        let mut combined = global_raw.clone();
        combined.extend(tenant_raw.clone());
        warn_unrecognised_keys(&combined);

        let (registration, token) =
            RegistrationSettings::initialize(db, tenant_id, &combined).await?;
        let authentication = AuthenticationSettings::from_raw(&combined);

        let agent_cert_lifetime_days = combined
            .get_setting(SettingKey::AgentCertLifetimeDays)
            .and_then(|v| v.as_u64()?.try_into().ok())
            .unwrap_or(DEFAULT_AGENT_CERT_LIFETIME_DAYS);

        let renewal_window_hours = combined
            .get_setting(SettingKey::AgentCertRenewalWindowHours)
            .and_then(|v| v.as_u64()?.try_into().ok())
            .unwrap_or(DEFAULT_RENEWAL_WINDOW_HOURS);

        let network = Self::load_network_settings(&combined);

        let mqtt_max_clients_per_tenant = combined
            .get_setting(SettingKey::MqttMaxClientsPerTenant)
            .and_then(|v| v.as_u64()?.try_into().ok())
            .unwrap_or(DEFAULT_MQTT_MAX_CLIENTS_PER_TENANT);

        let smtp = Self::load_smtp_settings(&combined);

        // Read initial version counters
        let (version, global_version) =
            crate::settings_store::get_settings_versions(db, tenant_id).await?;

        let snapshot = SettingsSnapshot {
            registration,
            authentication,
            agent_cert_lifetime_days,
            renewal_window_hours,
            network,
            mqtt_max_clients_per_tenant,
            smtp,
        };
        let (tx, rx) = tokio::sync::watch::channel(snapshot);
        let settings = Self {
            inner: Arc::new(Inner {
                snapshot_tx: tx,
                snapshot_rx: rx,
                version: AtomicI64::new(version),
                global_version: AtomicI64::new(global_version),
                write_mutex: tokio::sync::Mutex::new(()),
            }),
        };

        Ok((settings, global_raw, tenant_raw, token))
    }

    fn load_network_settings(raw: &RawSettings) -> NetworkSettings {
        let trusted_proxies = raw
            .get_setting(SettingKey::TrustedProxies)
            .and_then(|v| {
                v.as_array().map(|arr| {
                    arr.iter()
                        .filter_map(|s| s.as_str()?.parse::<IpNet>().ok())
                        .collect()
                })
            })
            .unwrap_or_default();

        let real_ip_header = raw
            .get_setting(SettingKey::RealIpHeader)
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_REAL_IP_HEADER)
            .to_string();

        let extra_sans = raw
            .get_setting(SettingKey::ExtraSans)
            .and_then(|v| {
                v.as_array().map(|arr| {
                    arr.iter()
                        .filter_map(|s| s.as_str().map(String::from))
                        .collect()
                })
            })
            .unwrap_or_default();

        let https_addr = raw
            .get_setting(SettingKey::HttpsAddr)
            .and_then(|v| v.as_str()?.parse::<SocketAddr>().ok())
            .unwrap_or_else(|| {
                SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 8443, 0, 0))
            });

        let forwarded_client_cert_info_header = raw
            .get_setting(SettingKey::ForwardedClientCertInfoHeader)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);

        let forwarded_client_cert_pem_header = raw
            .get_setting(SettingKey::ForwardedClientCertPemHeader)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);

        let pki_addr = raw
            .get_setting(SettingKey::PkiAddr)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);

        NetworkSettings {
            trusted_proxies,
            real_ip_header,
            extra_sans,
            https_addr,
            forwarded_client_cert_info_header,
            forwarded_client_cert_pem_header,
            pki_addr,
        }
    }

    /// Reload all settings from the database and publish atomically.
    ///
    /// Used by the periodic settings check when a version mismatch is detected.
    /// Loads from both `global_settings` and per-tenant `settings` tables.
    pub async fn reload_from_db(
        &self,
        db: &DatabaseConnection,
        tenant_id: Uuid,
    ) -> auth::Result<()> {
        let global_raw = crate::settings_store::load_all_global_settings(db).await?;
        let tenant_raw = crate::settings_store::load_all_settings(db, tenant_id).await?;
        let mut combined = global_raw;
        combined.extend(tenant_raw);

        let registration = RegistrationSettings::from_raw(&combined);
        let authentication = AuthenticationSettings::from_raw(&combined);

        let agent_cert_lifetime_days = combined
            .get_setting(SettingKey::AgentCertLifetimeDays)
            .and_then(|v| v.as_u64()?.try_into().ok())
            .unwrap_or(DEFAULT_AGENT_CERT_LIFETIME_DAYS);

        let renewal_window_hours = combined
            .get_setting(SettingKey::AgentCertRenewalWindowHours)
            .and_then(|v| v.as_u64()?.try_into().ok())
            .unwrap_or(DEFAULT_RENEWAL_WINDOW_HOURS);

        let network = Self::load_network_settings(&combined);

        let mqtt_max_clients_per_tenant = combined
            .get_setting(SettingKey::MqttMaxClientsPerTenant)
            .and_then(|v| v.as_u64()?.try_into().ok())
            .unwrap_or(DEFAULT_MQTT_MAX_CLIENTS_PER_TENANT);

        let smtp = Self::load_smtp_settings(&combined);

        // Publish complete snapshot atomically
        let _guard = self.inner.write_mutex.lock().await;
        let _ = self.inner.snapshot_tx.send(SettingsSnapshot {
            registration,
            authentication,
            agent_cert_lifetime_days,
            renewal_window_hours,
            network,
            mqtt_max_clients_per_tenant,
            smtp,
        });

        // Update cached version counters
        let (version, global_version) =
            crate::settings_store::get_settings_versions(db, tenant_id).await?;
        self.inner.version.store(version, Ordering::Release);
        self.inner
            .global_version
            .store(global_version, Ordering::Release);

        Ok(())
    }

    /// Check whether the DB version counters differ from the cached ones.
    ///
    /// If they differ, perform a full reload. Returns `true` if a reload happened.
    pub async fn check_version_and_reload(
        &self,
        db: &DatabaseConnection,
        tenant_id: Uuid,
    ) -> auth::Result<bool> {
        let (db_version, db_global_version) =
            crate::settings_store::get_settings_versions(db, tenant_id).await?;

        let cached_version = self.inner.version.load(Ordering::Acquire);
        let cached_global_version = self.inner.global_version.load(Ordering::Acquire);

        if db_version == cached_version && db_global_version == cached_global_version {
            return Ok(false);
        }

        self.reload_from_db(db, tenant_id).await?;
        Ok(true)
    }

    // --- Snapshot access (synchronous) ---

    /// Read the full settings snapshot.
    pub fn snapshot(&self) -> SettingsSnapshot {
        self.inner.snapshot_rx.borrow().clone()
    }

    // --- Registration ---

    /// Read registration settings (synchronous, no lock contention).
    pub fn registration(&self) -> RegistrationSettings {
        self.inner.snapshot_rx.borrow().registration.clone()
    }

    /// Replace registration settings (acquires write mutex for atomic publish).
    pub async fn set_registration(&self, reg: RegistrationSettings) {
        let _guard = self.inner.write_mutex.lock().await;
        self.inner
            .snapshot_tx
            .send_modify(|snap| snap.registration = reg);
    }

    // --- Authentication ---

    /// Read authentication settings (synchronous, no lock contention).
    pub fn authentication(&self) -> AuthenticationSettings {
        self.inner.snapshot_rx.borrow().authentication.clone()
    }

    /// Replace authentication settings (acquires write mutex for atomic publish).
    pub async fn set_authentication(&self, auth: AuthenticationSettings) {
        let _guard = self.inner.write_mutex.lock().await;
        self.inner
            .snapshot_tx
            .send_modify(|snap| snap.authentication = auth);
    }

    // --- Agent certificates ---

    /// Read the agent certificate lifetime in days (synchronous).
    pub fn agent_cert_lifetime_days(&self) -> u16 {
        self.inner.snapshot_rx.borrow().agent_cert_lifetime_days
    }

    /// Update the agent certificate lifetime in days.
    pub async fn set_agent_cert_lifetime_days(&self, days: u16) {
        let _guard = self.inner.write_mutex.lock().await;
        self.inner
            .snapshot_tx
            .send_modify(|snap| snap.agent_cert_lifetime_days = days);
    }

    /// Read the certificate renewal window in hours (synchronous).
    pub fn renewal_window_hours(&self) -> u16 {
        self.inner.snapshot_rx.borrow().renewal_window_hours
    }

    /// Update the certificate renewal window in hours.
    pub async fn set_renewal_window_hours(&self, hours: u16) {
        let _guard = self.inner.write_mutex.lock().await;
        self.inner
            .snapshot_tx
            .send_modify(|snap| snap.renewal_window_hours = hours);
    }

    // --- Network settings ---

    /// Read the full network settings snapshot (synchronous).
    pub fn network(&self) -> NetworkSettings {
        self.inner.snapshot_rx.borrow().network.clone()
    }

    /// Read trusted proxies (synchronous).
    pub fn trusted_proxies(&self) -> Vec<IpNet> {
        self.inner
            .snapshot_rx
            .borrow()
            .network
            .trusted_proxies
            .clone()
    }

    /// Read real IP header name (synchronous).
    pub fn real_ip_header(&self) -> String {
        self.inner
            .snapshot_rx
            .borrow()
            .network
            .real_ip_header
            .clone()
    }

    /// Read extra SANs (synchronous).
    pub fn extra_sans(&self) -> Vec<String> {
        self.inner.snapshot_rx.borrow().network.extra_sans.clone()
    }

    /// Read the HTTPS listen address (synchronous).
    pub fn https_addr(&self) -> SocketAddr {
        self.inner.snapshot_rx.borrow().network.https_addr
    }

    /// Replace all network settings.
    pub async fn set_network(&self, net: NetworkSettings) {
        let _guard = self.inner.write_mutex.lock().await;
        self.inner
            .snapshot_tx
            .send_modify(|snap| snap.network = net);
    }

    /// Update only trusted proxies.
    pub async fn set_trusted_proxies(&self, proxies: Vec<IpNet>) {
        let _guard = self.inner.write_mutex.lock().await;
        self.inner
            .snapshot_tx
            .send_modify(|snap| snap.network.trusted_proxies = proxies);
    }

    /// Update only real IP header.
    pub async fn set_real_ip_header(&self, header: String) {
        let _guard = self.inner.write_mutex.lock().await;
        self.inner
            .snapshot_tx
            .send_modify(|snap| snap.network.real_ip_header = header);
    }

    /// Update only extra SANs.
    pub async fn set_extra_sans(&self, sans: Vec<String>) {
        let _guard = self.inner.write_mutex.lock().await;
        self.inner
            .snapshot_tx
            .send_modify(|snap| snap.network.extra_sans = sans);
    }

    /// Update only HTTPS listen address.
    pub async fn set_https_addr(&self, addr: SocketAddr) {
        let _guard = self.inner.write_mutex.lock().await;
        self.inner
            .snapshot_tx
            .send_modify(|snap| snap.network.https_addr = addr);
    }

    /// Read forwarded client cert info header name (synchronous).
    pub fn forwarded_client_cert_info_header(&self) -> Option<String> {
        self.inner
            .snapshot_rx
            .borrow()
            .network
            .forwarded_client_cert_info_header
            .clone()
    }

    /// Update forwarded client cert info header name.
    pub async fn set_forwarded_client_cert_info_header(&self, header: Option<String>) {
        let _guard = self.inner.write_mutex.lock().await;
        self.inner
            .snapshot_tx
            .send_modify(|snap| snap.network.forwarded_client_cert_info_header = header);
    }

    /// Read forwarded client cert PEM header name (synchronous).
    pub fn forwarded_client_cert_pem_header(&self) -> Option<String> {
        self.inner
            .snapshot_rx
            .borrow()
            .network
            .forwarded_client_cert_pem_header
            .clone()
    }

    /// Update forwarded client cert PEM header name.
    pub async fn set_forwarded_client_cert_pem_header(&self, header: Option<String>) {
        let _guard = self.inner.write_mutex.lock().await;
        self.inner
            .snapshot_tx
            .send_modify(|snap| snap.network.forwarded_client_cert_pem_header = header);
    }

    /// Read the backend URL (synchronous).
    pub fn pki_addr(&self) -> Option<String> {
        self.inner.snapshot_rx.borrow().network.pki_addr.clone()
    }

    /// Update the backend URL.
    pub async fn set_pki_addr(&self, url: Option<String>) {
        let _guard = self.inner.write_mutex.lock().await;
        self.inner
            .snapshot_tx
            .send_modify(|snap| snap.network.pki_addr = url);
    }

    // --- MQTT settings ---

    /// Read the maximum number of MQTT clients per tenant (synchronous).
    pub fn mqtt_max_clients_per_tenant(&self) -> u16 {
        self.inner.snapshot_rx.borrow().mqtt_max_clients_per_tenant
    }

    /// Update the maximum number of MQTT clients per tenant.
    pub async fn set_mqtt_max_clients_per_tenant(&self, max: u16) {
        let _guard = self.inner.write_mutex.lock().await;
        self.inner
            .snapshot_tx
            .send_modify(|snap| snap.mqtt_max_clients_per_tenant = max);
    }

    // --- SMTP settings ---

    /// Read the SMTP settings snapshot (synchronous).
    pub fn smtp(&self) -> SmtpSettingsSnapshot {
        self.inner.snapshot_rx.borrow().smtp.clone()
    }

    /// Replace SMTP settings (acquires write mutex for atomic publish).
    pub async fn set_smtp(&self, smtp: SmtpSettingsSnapshot) {
        let _guard = self.inner.write_mutex.lock().await;
        self.inner
            .snapshot_tx
            .send_modify(|snap| snap.smtp = smtp);
    }

    /// Load SMTP settings from a [`RawSettings`] map.
    ///
    /// The `smtp.password` key is expected to be stored encrypted; it is
    /// transparently decrypted here. Unrecognised or unparseable values are
    /// silently ignored so that a bad DB row never prevents the server from
    /// starting.
    pub fn load_smtp_settings(raw: &RawSettings) -> SmtpSettingsSnapshot {
        let host = raw
            .get_setting(SettingKey::SmtpHost)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);

        let port = raw
            .get_setting(SettingKey::SmtpPort)
            .and_then(|v| v.as_u64()?.try_into().ok());

        let username = raw
            .get_setting(SettingKey::SmtpUsername)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);

        let password = raw
            .get_setting(SettingKey::SmtpPassword)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .and_then(|stored| {
                if uptrakit_crypto::is_encrypted(stored) {
                    uptrakit_crypto::decrypt_str(stored)
                        .map_err(|e| {
                            tracing::warn!("failed to decrypt SMTP password: {e}");
                        })
                        .ok()
                } else {
                    // Legacy plaintext — use as-is (migration happens on next write)
                    Some(stored.to_string())
                }
            });

        let from_address = raw
            .get_setting(SettingKey::SmtpFromAddress)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);

        let from_name = raw
            .get_setting(SettingKey::SmtpFromName)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);

        let tls_mode = raw
            .get_setting(SettingKey::SmtpTlsMode)
            .and_then(|v| v.as_str())
            .filter(|s| matches!(*s, "starttls" | "tls" | "none"))
            .unwrap_or("starttls")
            .to_string();

        SmtpSettingsSnapshot {
            host,
            port,
            username,
            password,
            from_address,
            from_name,
            tls_mode,
        }
    }
}
