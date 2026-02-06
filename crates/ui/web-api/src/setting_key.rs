use std::fmt;

/// Typed key for every DB-persisted setting.
///
/// Replaces scattered `const SETTING_KEY_*: &str` constants across modules.
/// Use [`as_str`](SettingKey::as_str) for DB access and
/// [`from_db_key`](SettingKey::from_db_key) to parse a raw DB key string.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(strum::EnumIter))]
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
    EnrollmentTokenHash,
    MqttEnrollmentTokenHash,
    ForwardedClientCertInfoHeader,
    ForwardedClientCertPemHeader,
    PkiAddr,
    MultiTenancyEnabled,
    RegistrationRequireTokenForOidc,
    MqttMaxClientsPerTenant,
}

impl SettingKey {
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
            Self::EnrollmentTokenHash => "agent_enrollment.token_hash",
            Self::MqttEnrollmentTokenHash => "mqtt_enrollment.token_hash",
            Self::ForwardedClientCertInfoHeader => "network.forwarded_client_cert_info_header",
            Self::ForwardedClientCertPemHeader => "network.forwarded_client_cert_pem_header",
            Self::PkiAddr => "network.pki_addr",
            Self::MultiTenancyEnabled => "multi_tenancy.enabled",
            Self::RegistrationRequireTokenForOidc => "registration.require_token_for_oidc",
            Self::MqttMaxClientsPerTenant => "mqtt.max_clients_per_tenant",
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
            "agent_enrollment.token_hash" => Some(Self::EnrollmentTokenHash),
            "mqtt_enrollment.token_hash" => Some(Self::MqttEnrollmentTokenHash),
            "network.forwarded_client_cert_info_header" => {
                Some(Self::ForwardedClientCertInfoHeader)
            }
            "network.forwarded_client_cert_pem_header" => Some(Self::ForwardedClientCertPemHeader),
            "network.pki_addr" => Some(Self::PkiAddr),
            "multi_tenancy.enabled" => Some(Self::MultiTenancyEnabled),
            "registration.require_token_for_oidc" => Some(Self::RegistrationRequireTokenForOidc),
            "mqtt.max_clients_per_tenant" => Some(Self::MqttMaxClientsPerTenant),
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
                | Self::PkiAddr
                | Self::MultiTenancyEnabled
                | Self::MqttMaxClientsPerTenant
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
    use strum::IntoEnumIterator;

    use super::*;

    #[test]
    fn round_trip_all_variants() {
        for key in SettingKey::iter() {
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
        for key in SettingKey::iter() {
            assert_eq!(key.to_string(), key.as_str());
        }
    }

    #[test]
    fn global_keys_identified() {
        assert!(SettingKey::TrustedProxies.is_global());
        assert!(SettingKey::RealIpHeader.is_global());
        assert!(SettingKey::ExtraSans.is_global());
        assert!(SettingKey::HttpsAddr.is_global());
        assert!(SettingKey::ForwardedClientCertInfoHeader.is_global());
        assert!(SettingKey::ForwardedClientCertPemHeader.is_global());
        assert!(SettingKey::PkiAddr.is_global());
        assert!(SettingKey::MultiTenancyEnabled.is_global());
        assert!(SettingKey::MqttMaxClientsPerTenant.is_global());
        // Per-tenant keys
        assert!(!SettingKey::RegistrationMode.is_global());
        assert!(!SettingKey::EnrollmentTokenHash.is_global());
        assert!(!SettingKey::MqttEnrollmentTokenHash.is_global());
    }
}
