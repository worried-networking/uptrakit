use std::net::SocketAddr;
use std::sync::Arc;

use ipnet::IpNet;
use sea_orm::DatabaseConnection;
use tokio::sync::{RwLock, RwLockWriteGuard};
use uuid::Uuid;

use crate::SettingKey;
use crate::auth;
use crate::auth::authentication::AuthenticationSettings;
use crate::auth::registration::RegistrationSettings;
use crate::settings_store::{RawSettings, RawSettingsExt};

const DEFAULT_AGENT_CERT_LIFETIME_DAYS: u16 = 7;
const DEFAULT_RENEWAL_WINDOW_HOURS: u16 = 6;

/// Default listen addresses used when neither CLI nor DB provides a value.
pub const DEFAULT_HTTPS_ADDR: &str = "[::]:8443";
pub const DEFAULT_REAL_IP_HEADER: &str = "X-Forwarded-For";
pub const DEFAULT_MQTT_PORT: u16 = 1883;
pub const DEFAULT_MQTT_CLIENT_ID: &str = "uptrakit-controller";
pub const DEFAULT_MQTT_TOPIC_PREFIX: &str = "uptrakit";

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
    pub backend_url: Option<String>,
}

impl Default for NetworkSettings {
    fn default() -> Self {
        Self {
            trusted_proxies: Vec::new(),
            real_ip_header: DEFAULT_REAL_IP_HEADER.to_string(),
            extra_sans: Vec::new(),
            https_addr: DEFAULT_HTTPS_ADDR.parse().unwrap(),
            forwarded_client_cert_info_header: None,
            forwarded_client_cert_pem_header: None,
            backend_url: None,
        }
    }
}

/// MQTT broker settings persisted in the DB. All changes require a restart.
#[derive(Clone, Debug, Default)]
pub struct MqttSettings {
    pub host: Option<String>,
    pub port: u16,
    pub client_id: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub topic_prefix: String,
}

#[derive(Clone)]
pub struct Settings {
    inner: Arc<Inner>,
}

struct Inner {
    registration: RwLock<RegistrationSettings>,
    authentication: RwLock<AuthenticationSettings>,
    agent_cert_lifetime_days: RwLock<u16>,
    renewal_window_hours: RwLock<u16>,
    network: RwLock<NetworkSettings>,
    mqtt: RwLock<MqttSettings>,
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
        Self {
            inner: Arc::new(Inner {
                registration: RwLock::new(registration),
                authentication: RwLock::new(AuthenticationSettings::default()),
                agent_cert_lifetime_days: RwLock::new(agent_cert_lifetime_days),
                renewal_window_hours: RwLock::new(renewal_window_hours),
                network: RwLock::new(NetworkSettings::default()),
                mqtt: RwLock::new(MqttSettings {
                    port: DEFAULT_MQTT_PORT,
                    client_id: DEFAULT_MQTT_CLIENT_ID.to_string(),
                    topic_prefix: DEFAULT_MQTT_TOPIC_PREFIX.to_string(),
                    ..Default::default()
                }),
            }),
        }
    }

    /// Load all settings from DB in a single bulk query. Generates initial
    /// registration token if no users exist.
    ///
    /// Returns `(Settings, RawSettings, Option<plaintext_token>)` — the caller
    /// can pass the raw map to reconciliation without re-reading from DB.
    pub async fn load(
        db: &DatabaseConnection,
        tenant_id: Uuid,
    ) -> auth::Result<(Self, RawSettings, Option<String>)> {
        let raw = crate::settings_store::load_all_settings(db, tenant_id).await?;
        warn_unrecognised_keys(&raw);

        let (registration, token) = RegistrationSettings::initialize(db, tenant_id, &raw).await?;
        let authentication = AuthenticationSettings::from_raw(&raw);

        let agent_cert_lifetime_days = raw
            .get_setting(SettingKey::AgentCertLifetimeDays)
            .and_then(|v| v.as_u64()?.try_into().ok())
            .unwrap_or(DEFAULT_AGENT_CERT_LIFETIME_DAYS);

        let renewal_window_hours = raw
            .get_setting(SettingKey::AgentCertRenewalWindowHours)
            .and_then(|v| v.as_u64()?.try_into().ok())
            .unwrap_or(DEFAULT_RENEWAL_WINDOW_HOURS);

        let network = Self::load_network_settings(&raw);
        let mqtt = Self::load_mqtt_settings(&raw);

        let settings = Self {
            inner: Arc::new(Inner {
                registration: RwLock::new(registration),
                authentication: RwLock::new(authentication),
                agent_cert_lifetime_days: RwLock::new(agent_cert_lifetime_days),
                renewal_window_hours: RwLock::new(renewal_window_hours),
                network: RwLock::new(network),
                mqtt: RwLock::new(mqtt),
            }),
        };

        Ok((settings, raw, token))
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
                DEFAULT_HTTPS_ADDR
                    .parse()
                    .expect("valid default HTTPS addr")
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

        let backend_url = raw
            .get_setting(SettingKey::BackendUrl)
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
            backend_url,
        }
    }

    fn load_mqtt_settings(raw: &RawSettings) -> MqttSettings {
        let host = raw
            .get_setting(SettingKey::MqttHost)
            .and_then(|v| v.as_str().map(String::from));

        let port = raw
            .get_setting(SettingKey::MqttPort)
            .and_then(|v| v.as_u64()?.try_into().ok())
            .unwrap_or(DEFAULT_MQTT_PORT);

        let client_id = raw
            .get_setting(SettingKey::MqttClientId)
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_MQTT_CLIENT_ID)
            .to_string();

        let username = raw
            .get_setting(SettingKey::MqttUsername)
            .and_then(|v| v.as_str().map(String::from));

        let password = raw
            .get_setting(SettingKey::MqttPassword)
            .and_then(|v| v.as_str().map(String::from));

        let topic_prefix = raw
            .get_setting(SettingKey::MqttTopicPrefix)
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_MQTT_TOPIC_PREFIX)
            .to_string();

        MqttSettings {
            host,
            port,
            client_id,
            username,
            password,
            topic_prefix,
        }
    }

    // --- Registration ---

    /// Read registration settings (acquires read lock, returns clone).
    pub async fn registration(&self) -> RegistrationSettings {
        self.inner.registration.read().await.clone()
    }

    /// Acquire write access to registration settings.
    pub async fn registration_write(&self) -> RwLockWriteGuard<'_, RegistrationSettings> {
        self.inner.registration.write().await
    }

    // --- Authentication ---

    /// Read authentication settings (acquires read lock, returns clone).
    pub async fn authentication(&self) -> AuthenticationSettings {
        self.inner.authentication.read().await.clone()
    }

    /// Acquire write access to authentication settings.
    pub async fn authentication_write(&self) -> RwLockWriteGuard<'_, AuthenticationSettings> {
        self.inner.authentication.write().await
    }

    // --- Agent certificates ---

    /// Read the agent certificate lifetime in days.
    pub async fn agent_cert_lifetime_days(&self) -> u16 {
        *self.inner.agent_cert_lifetime_days.read().await
    }

    /// Update the agent certificate lifetime in days.
    pub async fn set_agent_cert_lifetime_days(&self, days: u16) {
        *self.inner.agent_cert_lifetime_days.write().await = days;
    }

    /// Read the certificate renewal window in hours.
    pub async fn renewal_window_hours(&self) -> u16 {
        *self.inner.renewal_window_hours.read().await
    }

    /// Update the certificate renewal window in hours.
    pub async fn set_renewal_window_hours(&self, hours: u16) {
        *self.inner.renewal_window_hours.write().await = hours;
    }

    // --- Network settings ---

    /// Read the full network settings snapshot.
    pub async fn network(&self) -> NetworkSettings {
        self.inner.network.read().await.clone()
    }

    /// Read trusted proxies.
    pub async fn trusted_proxies(&self) -> Vec<IpNet> {
        self.inner.network.read().await.trusted_proxies.clone()
    }

    /// Read real IP header name.
    pub async fn real_ip_header(&self) -> String {
        self.inner.network.read().await.real_ip_header.clone()
    }

    /// Read extra SANs.
    pub async fn extra_sans(&self) -> Vec<String> {
        self.inner.network.read().await.extra_sans.clone()
    }

    /// Read the HTTPS listen address.
    pub async fn https_addr(&self) -> SocketAddr {
        self.inner.network.read().await.https_addr
    }

    /// Replace all network settings.
    pub async fn set_network(&self, net: NetworkSettings) {
        *self.inner.network.write().await = net;
    }

    /// Update only trusted proxies.
    pub async fn set_trusted_proxies(&self, proxies: Vec<IpNet>) {
        self.inner.network.write().await.trusted_proxies = proxies;
    }

    /// Update only real IP header.
    pub async fn set_real_ip_header(&self, header: String) {
        self.inner.network.write().await.real_ip_header = header;
    }

    /// Update only extra SANs.
    pub async fn set_extra_sans(&self, sans: Vec<String>) {
        self.inner.network.write().await.extra_sans = sans;
    }

    /// Update only HTTPS listen address.
    pub async fn set_https_addr(&self, addr: SocketAddr) {
        self.inner.network.write().await.https_addr = addr;
    }

    /// Read forwarded client cert info header name.
    pub async fn forwarded_client_cert_info_header(&self) -> Option<String> {
        self.inner
            .network
            .read()
            .await
            .forwarded_client_cert_info_header
            .clone()
    }

    /// Update forwarded client cert info header name.
    pub async fn set_forwarded_client_cert_info_header(&self, header: Option<String>) {
        self.inner
            .network
            .write()
            .await
            .forwarded_client_cert_info_header = header;
    }

    /// Read forwarded client cert PEM header name.
    pub async fn forwarded_client_cert_pem_header(&self) -> Option<String> {
        self.inner
            .network
            .read()
            .await
            .forwarded_client_cert_pem_header
            .clone()
    }

    /// Update forwarded client cert PEM header name.
    pub async fn set_forwarded_client_cert_pem_header(&self, header: Option<String>) {
        self.inner
            .network
            .write()
            .await
            .forwarded_client_cert_pem_header = header;
    }

    /// Read the backend URL.
    pub async fn backend_url(&self) -> Option<String> {
        self.inner.network.read().await.backend_url.clone()
    }

    /// Update the backend URL.
    pub async fn set_backend_url(&self, url: Option<String>) {
        self.inner.network.write().await.backend_url = url;
    }

    // --- MQTT settings ---

    /// Read the full MQTT settings snapshot.
    pub async fn mqtt(&self) -> MqttSettings {
        self.inner.mqtt.read().await.clone()
    }

    /// Replace all MQTT settings.
    pub async fn set_mqtt(&self, mqtt: MqttSettings) {
        *self.inner.mqtt.write().await = mqtt;
    }
}
