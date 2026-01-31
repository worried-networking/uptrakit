use std::net::SocketAddr;
use std::sync::Arc;

use ipnet::IpNet;
use sea_orm::DatabaseConnection;
use tokio::sync::{RwLock, RwLockWriteGuard};

use crate::auth;
use crate::auth::authentication::AuthenticationSettings;
use crate::auth::registration::RegistrationSettings;
use crate::settings_store::load_setting;

const SETTING_KEY_AGENT_CERT_LIFETIME: &str = "agent_certificate.lifetime_days";
const DEFAULT_AGENT_CERT_LIFETIME_DAYS: u16 = 7;

const SETTING_KEY_RENEWAL_WINDOW_HOURS: &str = "agent_certificate.renewal_window_hours";
const DEFAULT_RENEWAL_WINDOW_HOURS: u16 = 6;

// Network setting DB keys
pub const SETTING_KEY_TRUSTED_PROXIES: &str = "network.trusted_proxies";
pub const SETTING_KEY_REAL_IP_HEADER: &str = "network.real_ip_header";
pub const SETTING_KEY_EXTRA_SANS: &str = "network.extra_sans";
pub const SETTING_KEY_HTTP_ADDR: &str = "network.http_addr";
pub const SETTING_KEY_HTTPS_ADDR: &str = "network.https_addr";

// MQTT setting DB keys
pub const SETTING_KEY_MQTT_HOST: &str = "mqtt.host";
pub const SETTING_KEY_MQTT_PORT: &str = "mqtt.port";
pub const SETTING_KEY_MQTT_CLIENT_ID: &str = "mqtt.client_id";
pub const SETTING_KEY_MQTT_USERNAME: &str = "mqtt.username";
pub const SETTING_KEY_MQTT_PASSWORD: &str = "mqtt.password";
pub const SETTING_KEY_MQTT_TOPIC_PREFIX: &str = "mqtt.topic_prefix";

/// Default listen addresses used when neither CLI nor DB provides a value.
pub const DEFAULT_HTTP_ADDR: &str = "[::]:8080";
pub const DEFAULT_HTTPS_ADDR: &str = "[::]:8443";
pub const DEFAULT_REAL_IP_HEADER: &str = "X-Forwarded-For";
pub const DEFAULT_MQTT_PORT: u16 = 1883;
pub const DEFAULT_MQTT_CLIENT_ID: &str = "uptrakit-controller";
pub const DEFAULT_MQTT_TOPIC_PREFIX: &str = "uptrakit";

/// Network-related settings persisted in the DB and changeable at runtime
/// (except for listen addresses which require a restart).
#[derive(Clone, Debug)]
pub struct NetworkSettings {
    pub trusted_proxies: Vec<IpNet>,
    pub real_ip_header: String,
    pub extra_sans: Vec<String>,
    pub http_addr: SocketAddr,
    pub https_addr: SocketAddr,
}

impl Default for NetworkSettings {
    fn default() -> Self {
        Self {
            trusted_proxies: Vec::new(),
            real_ip_header: DEFAULT_REAL_IP_HEADER.to_string(),
            extra_sans: Vec::new(),
            http_addr: DEFAULT_HTTP_ADDR.parse().unwrap(),
            https_addr: DEFAULT_HTTPS_ADDR.parse().unwrap(),
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

    /// Load all settings from DB. Generates initial registration token
    /// if no users exist. Returns `(Settings, Option<plaintext_token>)`.
    pub async fn load(db: &DatabaseConnection) -> auth::Result<(Self, Option<String>)> {
        let (registration, token) = RegistrationSettings::initialize(db).await?;
        let authentication = AuthenticationSettings::load(db).await?;

        let agent_cert_lifetime_days = match load_setting(db, SETTING_KEY_AGENT_CERT_LIFETIME).await
        {
            Ok(Some(v)) => match v.as_u64().and_then(|n| u16::try_from(n).ok()) {
                Some(days) => days,
                None => DEFAULT_AGENT_CERT_LIFETIME_DAYS,
            },
            _ => DEFAULT_AGENT_CERT_LIFETIME_DAYS,
        };

        let renewal_window_hours = match load_setting(db, SETTING_KEY_RENEWAL_WINDOW_HOURS).await {
            Ok(Some(v)) => match v.as_u64().and_then(|n| u16::try_from(n).ok()) {
                Some(hours) => hours,
                None => DEFAULT_RENEWAL_WINDOW_HOURS,
            },
            _ => DEFAULT_RENEWAL_WINDOW_HOURS,
        };

        // Load network settings from DB
        let network = Self::load_network_settings(db).await;

        // Load MQTT settings from DB
        let mqtt = Self::load_mqtt_settings(db).await;

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

        Ok((settings, token))
    }

    async fn load_network_settings(db: &DatabaseConnection) -> NetworkSettings {
        let trusted_proxies = match load_setting(db, SETTING_KEY_TRUSTED_PROXIES).await {
            Ok(Some(v)) => v
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|s| s.as_str()?.parse::<IpNet>().ok())
                        .collect()
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        };

        let real_ip_header = match load_setting(db, SETTING_KEY_REAL_IP_HEADER).await {
            Ok(Some(v)) => v.as_str().unwrap_or(DEFAULT_REAL_IP_HEADER).to_string(),
            _ => DEFAULT_REAL_IP_HEADER.to_string(),
        };

        let extra_sans = match load_setting(db, SETTING_KEY_EXTRA_SANS).await {
            Ok(Some(v)) => v
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|s| s.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        };

        let http_addr = match load_setting(db, SETTING_KEY_HTTP_ADDR).await {
            Ok(Some(v)) => v
                .as_str()
                .and_then(|s| s.parse::<SocketAddr>().ok())
                .unwrap_or_else(|| DEFAULT_HTTP_ADDR.parse().unwrap()),
            _ => DEFAULT_HTTP_ADDR.parse().unwrap(),
        };

        let https_addr = match load_setting(db, SETTING_KEY_HTTPS_ADDR).await {
            Ok(Some(v)) => v
                .as_str()
                .and_then(|s| s.parse::<SocketAddr>().ok())
                .unwrap_or_else(|| DEFAULT_HTTPS_ADDR.parse().unwrap()),
            _ => DEFAULT_HTTPS_ADDR.parse().unwrap(),
        };

        NetworkSettings {
            trusted_proxies,
            real_ip_header,
            extra_sans,
            http_addr,
            https_addr,
        }
    }

    async fn load_mqtt_settings(db: &DatabaseConnection) -> MqttSettings {
        let host = match load_setting(db, SETTING_KEY_MQTT_HOST).await {
            Ok(Some(v)) => v.as_str().map(String::from),
            _ => None,
        };

        let port = match load_setting(db, SETTING_KEY_MQTT_PORT).await {
            Ok(Some(v)) => v
                .as_u64()
                .and_then(|n| u16::try_from(n).ok())
                .unwrap_or(DEFAULT_MQTT_PORT),
            _ => DEFAULT_MQTT_PORT,
        };

        let client_id = match load_setting(db, SETTING_KEY_MQTT_CLIENT_ID).await {
            Ok(Some(v)) => v.as_str().unwrap_or(DEFAULT_MQTT_CLIENT_ID).to_string(),
            _ => DEFAULT_MQTT_CLIENT_ID.to_string(),
        };

        let username = match load_setting(db, SETTING_KEY_MQTT_USERNAME).await {
            Ok(Some(v)) => v.as_str().map(String::from),
            _ => None,
        };

        let password = match load_setting(db, SETTING_KEY_MQTT_PASSWORD).await {
            Ok(Some(v)) => v.as_str().map(String::from),
            _ => None,
        };

        let topic_prefix = match load_setting(db, SETTING_KEY_MQTT_TOPIC_PREFIX).await {
            Ok(Some(v)) => v.as_str().unwrap_or(DEFAULT_MQTT_TOPIC_PREFIX).to_string(),
            _ => DEFAULT_MQTT_TOPIC_PREFIX.to_string(),
        };

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

    /// Read the HTTP listen address.
    pub async fn http_addr(&self) -> SocketAddr {
        self.inner.network.read().await.http_addr
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

    /// Update only HTTP listen address.
    pub async fn set_http_addr(&self, addr: SocketAddr) {
        self.inner.network.write().await.http_addr = addr;
    }

    /// Update only HTTPS listen address.
    pub async fn set_https_addr(&self, addr: SocketAddr) {
        self.inner.network.write().await.https_addr = addr;
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
