use std::fmt;

/// Typed key for every DB-persisted setting.
///
/// Replaces scattered `const SETTING_KEY_*: &str` constants across modules.
/// Use [`as_str`](SettingKey::as_str) for DB access and
/// [`from_db_key`](SettingKey::from_db_key) to parse a raw DB key string.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SettingKey {
    RegistrationMode,
    RegistrationTokenHash,
    PasswordAuthEnabled,
    AgentCertLifetimeDays,
    AgentCertRenewalWindowHours,
    TrustedProxies,
    RealIpHeader,
    ExtraSans,
    HttpsAddr,
    MqttHost,
    MqttPort,
    MqttClientId,
    MqttUsername,
    MqttPassword,
    MqttTopicPrefix,
    EnrollmentTokenHash,
    ForwardedClientCertInfoHeader,
    ForwardedClientCertPemHeader,
    BackendUrl,
    MultiTenancyEnabled,
}

impl SettingKey {
    /// Every known setting key, in definition order.
    pub const ALL: &[SettingKey] = &[
        Self::RegistrationMode,
        Self::RegistrationTokenHash,
        Self::PasswordAuthEnabled,
        Self::AgentCertLifetimeDays,
        Self::AgentCertRenewalWindowHours,
        Self::TrustedProxies,
        Self::RealIpHeader,
        Self::ExtraSans,
        Self::HttpsAddr,
        Self::MqttHost,
        Self::MqttPort,
        Self::MqttClientId,
        Self::MqttUsername,
        Self::MqttPassword,
        Self::MqttTopicPrefix,
        Self::EnrollmentTokenHash,
        Self::ForwardedClientCertInfoHeader,
        Self::ForwardedClientCertPemHeader,
        Self::BackendUrl,
        Self::MultiTenancyEnabled,
    ];

    /// The DB string representation of this key.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RegistrationMode => "registration.mode",
            Self::RegistrationTokenHash => "registration.token_hash",
            Self::PasswordAuthEnabled => "auth.password_enabled",
            Self::AgentCertLifetimeDays => "agent_certificate.lifetime_days",
            Self::AgentCertRenewalWindowHours => "agent_certificate.renewal_window_hours",
            Self::TrustedProxies => "network.trusted_proxies",
            Self::RealIpHeader => "network.real_ip_header",
            Self::ExtraSans => "network.extra_sans",
            Self::HttpsAddr => "network.https_addr",
            Self::MqttHost => "mqtt.host",
            Self::MqttPort => "mqtt.port",
            Self::MqttClientId => "mqtt.client_id",
            Self::MqttUsername => "mqtt.username",
            Self::MqttPassword => "mqtt.password",
            Self::MqttTopicPrefix => "mqtt.topic_prefix",
            Self::EnrollmentTokenHash => "agent_enrollment.token_hash",
            Self::ForwardedClientCertInfoHeader => "network.forwarded_client_cert_info_header",
            Self::ForwardedClientCertPemHeader => "network.forwarded_client_cert_pem_header",
            Self::BackendUrl => "network.backend_url",
            Self::MultiTenancyEnabled => "multi_tenancy.enabled",
        }
    }

    /// Parse a raw DB key string into a `SettingKey`, if recognised.
    pub fn from_db_key(key: &str) -> Option<Self> {
        match key {
            "registration.mode" => Some(Self::RegistrationMode),
            "registration.token_hash" => Some(Self::RegistrationTokenHash),
            "auth.password_enabled" => Some(Self::PasswordAuthEnabled),
            "agent_certificate.lifetime_days" => Some(Self::AgentCertLifetimeDays),
            "agent_certificate.renewal_window_hours" => Some(Self::AgentCertRenewalWindowHours),
            "network.trusted_proxies" => Some(Self::TrustedProxies),
            "network.real_ip_header" => Some(Self::RealIpHeader),
            "network.extra_sans" => Some(Self::ExtraSans),
            "network.https_addr" => Some(Self::HttpsAddr),
            "mqtt.host" => Some(Self::MqttHost),
            "mqtt.port" => Some(Self::MqttPort),
            "mqtt.client_id" => Some(Self::MqttClientId),
            "mqtt.username" => Some(Self::MqttUsername),
            "mqtt.password" => Some(Self::MqttPassword),
            "mqtt.topic_prefix" => Some(Self::MqttTopicPrefix),
            "agent_enrollment.token_hash" => Some(Self::EnrollmentTokenHash),
            "network.forwarded_client_cert_info_header" => {
                Some(Self::ForwardedClientCertInfoHeader)
            }
            "network.forwarded_client_cert_pem_header" => Some(Self::ForwardedClientCertPemHeader),
            "network.backend_url" => Some(Self::BackendUrl),
            "multi_tenancy.enabled" => Some(Self::MultiTenancyEnabled),
            _ => None,
        }
    }

    /// Returns `true` for settings that are global (not per-tenant).
    pub const fn is_global(self) -> bool {
        matches!(
            self,
            Self::TrustedProxies
                | Self::RealIpHeader
                | Self::ExtraSans
                | Self::HttpsAddr
                | Self::ForwardedClientCertInfoHeader
                | Self::ForwardedClientCertPemHeader
                | Self::BackendUrl
                | Self::MultiTenancyEnabled
        )
    }
}

impl fmt::Display for SettingKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_all_variants() {
        for &key in SettingKey::ALL {
            let s = key.as_str();
            let parsed = SettingKey::from_db_key(s);
            assert_eq!(parsed, Some(key), "round-trip failed for {s}");
        }
    }

    #[test]
    fn unknown_key_returns_none() {
        assert_eq!(SettingKey::from_db_key("nonexistent.key"), None);
        assert_eq!(SettingKey::from_db_key(""), None);
    }

    #[test]
    fn display_matches_as_str() {
        for &key in SettingKey::ALL {
            assert_eq!(key.to_string(), key.as_str());
        }
    }

    #[test]
    fn all_has_correct_count() {
        // 20 variants defined in the enum
        assert_eq!(SettingKey::ALL.len(), 20);
    }

    #[test]
    fn global_keys_identified() {
        assert!(SettingKey::TrustedProxies.is_global());
        assert!(SettingKey::RealIpHeader.is_global());
        assert!(SettingKey::ExtraSans.is_global());
        assert!(SettingKey::HttpsAddr.is_global());
        assert!(SettingKey::ForwardedClientCertInfoHeader.is_global());
        assert!(SettingKey::ForwardedClientCertPemHeader.is_global());
        assert!(SettingKey::BackendUrl.is_global());
        assert!(SettingKey::MultiTenancyEnabled.is_global());
        // Per-tenant keys
        assert!(!SettingKey::RegistrationMode.is_global());
        assert!(!SettingKey::MqttHost.is_global());
        assert!(!SettingKey::EnrollmentTokenHash.is_global());
    }
}
