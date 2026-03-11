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
    AgentCertLifetimeHours,
    AgentCertRenewalWindowHours,
    TrustedProxies,
    RealIpHeader,
    Sans,
    HttpsAddr,
    ForwardedClientCertInfoHeader,
    ForwardedClientCertPemHeader,
    PkiAddr,
    PkiActiveCaFingerprint,
    PkiCaVersion,
    MultiTenancyEnabled,
    RegistrationRequireTokenForOidc,
    MqttMaxClientsPerTenant,
    JwtSigningKey,
    MasterKeyVerification,
    SmtpHost,
    SmtpPort,
    SmtpUsername,
    SmtpPassword,
    SmtpFromAddress,
    SmtpFromName,
    SmtpTlsMode,
    NatsUrl,
    /// Per-tenant audit log filter mode (`all`, `mutations`, `none`).
    ///
    /// Overrides the global `--audit-log-filter` CLI flag for this tenant.
    ///
    /// DB key: `audit_log.filter`
    AuditLogFilter,
    /// Per-tenant audit log retention period in days.
    ///
    /// Default: 90 days. Set to 0 to disable retention cleanup for this tenant.
    ///
    /// DB key: `audit_log.retention_days`
    AuditLogRetentionDays,
    /// Whether mDNS/DNS-SD zero-configuration advertising is enabled.
    ///
    /// DB key: `zeroconf.enabled`
    ZeroconfEnabled,
    /// Override URL advertised via mDNS (for reverse proxy deployments).
    ///
    /// DB key: `zeroconf.url`
    ZeroconfUrl,
    /// Override PKI address advertised via mDNS (for reverse proxy deployments).
    ///
    /// DB key: `zeroconf.pki_addr`
    ZeroconfPkiAddr,
}

impl SettingKey {
    /// The DB string representation of this key.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RegistrationMode => "registration.mode",
            Self::RegistrationTokenHash => "registration.token_hash",
            Self::PasswordAuthEnabled => "auth.password_enabled",
            Self::AgentCertLifetimeHours => "agent_certificate.lifetime_hours",
            Self::AgentCertRenewalWindowHours => "agent_certificate.renewal_window_hours",
            Self::TrustedProxies => "network.trusted_proxies",
            Self::RealIpHeader => "network.real_ip_header",
            Self::Sans => "network.sans",
            Self::HttpsAddr => "network.https_addr",
            Self::ForwardedClientCertInfoHeader => "network.forwarded_client_cert_info_header",
            Self::ForwardedClientCertPemHeader => "network.forwarded_client_cert_pem_header",
            Self::PkiAddr => "network.pki_addr",
            Self::PkiActiveCaFingerprint => "pki.active_ca_fingerprint",
            Self::PkiCaVersion => "pki.ca_version",
            Self::MultiTenancyEnabled => "multi_tenancy.enabled",
            Self::RegistrationRequireTokenForOidc => "registration.require_token_for_oidc",
            Self::MqttMaxClientsPerTenant => "mqtt.max_clients_per_tenant",
            Self::JwtSigningKey => "auth.jwt_signing_key",
            Self::MasterKeyVerification => "crypto.master_key_verification",
            Self::SmtpHost => "smtp.host",
            Self::SmtpPort => "smtp.port",
            Self::SmtpUsername => "smtp.username",
            Self::SmtpPassword => "smtp.password",
            Self::SmtpFromAddress => "smtp.from_address",
            Self::SmtpFromName => "smtp.from_name",
            Self::SmtpTlsMode => "smtp.tls_mode",
            Self::NatsUrl => "nats.url",
            Self::AuditLogFilter => "audit_log.filter",
            Self::AuditLogRetentionDays => "audit_log.retention_days",
            Self::ZeroconfEnabled => "zeroconf.enabled",
            Self::ZeroconfUrl => "zeroconf.url",
            Self::ZeroconfPkiAddr => "zeroconf.pki_addr",
        }
    }

    /// Parse a raw DB key string into a `SettingKey`, if recognised.
    pub fn from_db_key(key: &str) -> Option<Self> {
        match key {
            "registration.mode" => Some(Self::RegistrationMode),
            "registration.token_hash" => Some(Self::RegistrationTokenHash),
            "auth.password_enabled" => Some(Self::PasswordAuthEnabled),
            "agent_certificate.lifetime_hours" => Some(Self::AgentCertLifetimeHours),
            "agent_certificate.renewal_window_hours" => Some(Self::AgentCertRenewalWindowHours),
            "network.trusted_proxies" => Some(Self::TrustedProxies),
            "network.real_ip_header" => Some(Self::RealIpHeader),
            "network.sans" => Some(Self::Sans),
            "network.https_addr" => Some(Self::HttpsAddr),
            "network.forwarded_client_cert_info_header" => {
                Some(Self::ForwardedClientCertInfoHeader)
            }
            "network.forwarded_client_cert_pem_header" => Some(Self::ForwardedClientCertPemHeader),
            "network.pki_addr" => Some(Self::PkiAddr),
            "pki.active_ca_fingerprint" => Some(Self::PkiActiveCaFingerprint),
            "pki.ca_version" => Some(Self::PkiCaVersion),
            "multi_tenancy.enabled" => Some(Self::MultiTenancyEnabled),
            "registration.require_token_for_oidc" => Some(Self::RegistrationRequireTokenForOidc),
            "mqtt.max_clients_per_tenant" => Some(Self::MqttMaxClientsPerTenant),
            "auth.jwt_signing_key" => Some(Self::JwtSigningKey),
            "crypto.master_key_verification" => Some(Self::MasterKeyVerification),
            "smtp.host" => Some(Self::SmtpHost),
            "smtp.port" => Some(Self::SmtpPort),
            "smtp.username" => Some(Self::SmtpUsername),
            "smtp.password" => Some(Self::SmtpPassword),
            "smtp.from_address" => Some(Self::SmtpFromAddress),
            "smtp.from_name" => Some(Self::SmtpFromName),
            "smtp.tls_mode" => Some(Self::SmtpTlsMode),
            "nats.url" => Some(Self::NatsUrl),
            "audit_log.filter" => Some(Self::AuditLogFilter),
            "audit_log.retention_days" => Some(Self::AuditLogRetentionDays),
            "zeroconf.enabled" => Some(Self::ZeroconfEnabled),
            "zeroconf.url" => Some(Self::ZeroconfUrl),
            "zeroconf.pki_addr" => Some(Self::ZeroconfPkiAddr),
            _ => None,
        }
    }

    /// Returns `true` for settings that are global (not per-tenant).
    pub const fn is_global(self) -> bool {
        matches!(
            self,
            Self::TrustedProxies
                | Self::RealIpHeader
                | Self::Sans
                | Self::HttpsAddr
                | Self::ForwardedClientCertInfoHeader
                | Self::ForwardedClientCertPemHeader
                | Self::PkiAddr
                | Self::PkiActiveCaFingerprint
                | Self::PkiCaVersion
                | Self::MultiTenancyEnabled
                | Self::MqttMaxClientsPerTenant
                | Self::JwtSigningKey
                | Self::MasterKeyVerification
                | Self::NatsUrl
                | Self::ZeroconfEnabled
                | Self::ZeroconfUrl
                | Self::ZeroconfPkiAddr
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
        assert!(SettingKey::Sans.is_global());
        assert!(SettingKey::HttpsAddr.is_global());
        assert!(SettingKey::ForwardedClientCertInfoHeader.is_global());
        assert!(SettingKey::ForwardedClientCertPemHeader.is_global());
        assert!(SettingKey::PkiAddr.is_global());
        assert!(SettingKey::PkiActiveCaFingerprint.is_global());
        assert!(SettingKey::PkiCaVersion.is_global());
        assert!(SettingKey::MultiTenancyEnabled.is_global());
        assert!(SettingKey::MqttMaxClientsPerTenant.is_global());
        assert!(SettingKey::JwtSigningKey.is_global());
        // Per-tenant keys
        assert!(!SettingKey::RegistrationMode.is_global());
        assert!(!SettingKey::AuditLogFilter.is_global());
        assert!(!SettingKey::AuditLogRetentionDays.is_global());
    }
}
