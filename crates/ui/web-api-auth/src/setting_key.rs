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
    JwtSigningKey,
    MasterKeyVerification,
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
    /// Whether 2FA is required for all password-auth users in this tenant.
    ///
    /// DB key: `auth.two_factor_required`
    TwoFactorRequired,
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
    /// Whether the MCP OAuth server is enabled.
    ///
    /// DB key: `oauth.mcp_enabled`
    OauthMcpEnabled,
    /// Whether Dynamic Client Registration (DCR) is enabled.
    ///
    /// DB key: `oauth.dcr_enabled`
    OauthDcrEnabled,
    /// Whether Client-Initiated Metadata Discovery (CIMD) is enabled.
    ///
    /// DB key: `oauth.cimd_enabled`
    OauthCimdEnabled,
    /// Canonical host used as OAuth issuer and redirect base.
    ///
    /// DB key: `oauth.canonical_host`
    OauthCanonicalHost,
    /// Additional audience host strings accepted in bearer token validation.
    ///
    /// DB key: `oauth.accepted_audience_hosts`
    OauthAcceptedAudienceHosts,
    /// Allow multiple controller instances to share an OAuth domain (unsafe).
    ///
    /// DB key: `oauth.allow_multi_controller_unsafe`
    OauthAllowMultiControllerUnsafe,
    /// HMAC-SHA256 secret used to sign OAuth JWTs.
    ///
    /// DB key: `oauth.jwt_signing_secret`
    OauthJwtSigningSecret,
    /// Lifetime of OAuth access tokens in seconds.
    ///
    /// DB key: `oauth.access_token_ttl_secs`
    OauthAccessTokenTtlSecs,
    /// Lifetime of OAuth refresh tokens in seconds.
    ///
    /// DB key: `oauth.refresh_token_ttl_secs`
    OauthRefreshTokenTtlSecs,
    /// Maximum lifetime of an OAuth refresh token family in seconds.
    ///
    /// DB key: `oauth.refresh_family_max_ttl_secs`
    OauthRefreshFamilyMaxTtlSecs,
    /// Lifetime of OAuth authorization codes in seconds.
    ///
    /// DB key: `oauth.authorization_code_ttl_secs`
    OauthAuthorizationCodeTtlSecs,
    /// Lifetime of OAuth authorization requests (PAR) in seconds.
    ///
    /// DB key: `oauth.authorization_request_ttl_secs`
    OauthAuthorizationRequestTtlSecs,
    /// Rate limit: maximum DCR registrations per hour.
    ///
    /// DB key: `oauth.rate.dcr_per_hour`
    OauthRateDcrPerHour,
    /// Rate limit: maximum CIMD requests per minute.
    ///
    /// DB key: `oauth.rate.cimd_per_min`
    OauthRateCimdPerMin,
    /// Rate limit: maximum authorization endpoint requests per minute.
    ///
    /// DB key: `oauth.rate.authorize_per_min`
    OauthRateAuthorizePerMin,
    /// Rate limit: maximum token endpoint requests per minute.
    ///
    /// DB key: `oauth.rate.token_per_min`
    OauthRateTokenPerMin,
    /// Rate limit: maximum consent endpoint requests per minute.
    ///
    /// DB key: `oauth.rate.consent_per_min`
    OauthRateConsentPerMin,
    /// Rate limit: maximum MCP authentication failures per minute before lockout.
    ///
    /// DB key: `oauth.rate.mcp_auth_fail_per_min`
    OauthRateMcpAuthFailPerMin,
    /// Allowlist of cosmetic CIMD fields that clients may supply during registration.
    ///
    /// DB key: `oauth.cimd_cosmetic_field_allowlist`
    OauthCimdCosmeticFieldAllowlist,
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
            Self::JwtSigningKey => "auth.jwt_signing_key",
            Self::MasterKeyVerification => "crypto.master_key_verification",
            Self::NatsUrl => "nats.url",
            Self::AuditLogFilter => "audit_log.filter",
            Self::AuditLogRetentionDays => "audit_log.retention_days",
            Self::TwoFactorRequired => "auth.two_factor_required",
            Self::ZeroconfEnabled => "zeroconf.enabled",
            Self::ZeroconfUrl => "zeroconf.url",
            Self::ZeroconfPkiAddr => "zeroconf.pki_addr",
            Self::OauthMcpEnabled => "oauth.mcp_enabled",
            Self::OauthDcrEnabled => "oauth.dcr_enabled",
            Self::OauthCimdEnabled => "oauth.cimd_enabled",
            Self::OauthCanonicalHost => "oauth.canonical_host",
            Self::OauthAcceptedAudienceHosts => "oauth.accepted_audience_hosts",
            Self::OauthAllowMultiControllerUnsafe => "oauth.allow_multi_controller_unsafe",
            Self::OauthJwtSigningSecret => "oauth.jwt_signing_secret",
            Self::OauthAccessTokenTtlSecs => "oauth.access_token_ttl_secs",
            Self::OauthRefreshTokenTtlSecs => "oauth.refresh_token_ttl_secs",
            Self::OauthRefreshFamilyMaxTtlSecs => "oauth.refresh_family_max_ttl_secs",
            Self::OauthAuthorizationCodeTtlSecs => "oauth.authorization_code_ttl_secs",
            Self::OauthAuthorizationRequestTtlSecs => "oauth.authorization_request_ttl_secs",
            Self::OauthRateDcrPerHour => "oauth.rate.dcr_per_hour",
            Self::OauthRateCimdPerMin => "oauth.rate.cimd_per_min",
            Self::OauthRateAuthorizePerMin => "oauth.rate.authorize_per_min",
            Self::OauthRateTokenPerMin => "oauth.rate.token_per_min",
            Self::OauthRateConsentPerMin => "oauth.rate.consent_per_min",
            Self::OauthRateMcpAuthFailPerMin => "oauth.rate.mcp_auth_fail_per_min",
            Self::OauthCimdCosmeticFieldAllowlist => "oauth.cimd_cosmetic_field_allowlist",
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
            "auth.jwt_signing_key" => Some(Self::JwtSigningKey),
            "crypto.master_key_verification" => Some(Self::MasterKeyVerification),
            "nats.url" => Some(Self::NatsUrl),
            "audit_log.filter" => Some(Self::AuditLogFilter),
            "audit_log.retention_days" => Some(Self::AuditLogRetentionDays),
            "auth.two_factor_required" => Some(Self::TwoFactorRequired),
            "zeroconf.enabled" => Some(Self::ZeroconfEnabled),
            "zeroconf.url" => Some(Self::ZeroconfUrl),
            "zeroconf.pki_addr" => Some(Self::ZeroconfPkiAddr),
            "oauth.mcp_enabled" => Some(Self::OauthMcpEnabled),
            "oauth.dcr_enabled" => Some(Self::OauthDcrEnabled),
            "oauth.cimd_enabled" => Some(Self::OauthCimdEnabled),
            "oauth.canonical_host" => Some(Self::OauthCanonicalHost),
            "oauth.accepted_audience_hosts" => Some(Self::OauthAcceptedAudienceHosts),
            "oauth.allow_multi_controller_unsafe" => Some(Self::OauthAllowMultiControllerUnsafe),
            "oauth.jwt_signing_secret" => Some(Self::OauthJwtSigningSecret),
            "oauth.access_token_ttl_secs" => Some(Self::OauthAccessTokenTtlSecs),
            "oauth.refresh_token_ttl_secs" => Some(Self::OauthRefreshTokenTtlSecs),
            "oauth.refresh_family_max_ttl_secs" => Some(Self::OauthRefreshFamilyMaxTtlSecs),
            "oauth.authorization_code_ttl_secs" => Some(Self::OauthAuthorizationCodeTtlSecs),
            "oauth.authorization_request_ttl_secs" => Some(Self::OauthAuthorizationRequestTtlSecs),
            "oauth.rate.dcr_per_hour" => Some(Self::OauthRateDcrPerHour),
            "oauth.rate.cimd_per_min" => Some(Self::OauthRateCimdPerMin),
            "oauth.rate.authorize_per_min" => Some(Self::OauthRateAuthorizePerMin),
            "oauth.rate.token_per_min" => Some(Self::OauthRateTokenPerMin),
            "oauth.rate.consent_per_min" => Some(Self::OauthRateConsentPerMin),
            "oauth.rate.mcp_auth_fail_per_min" => Some(Self::OauthRateMcpAuthFailPerMin),
            "oauth.cimd_cosmetic_field_allowlist" => Some(Self::OauthCimdCosmeticFieldAllowlist),
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
                | Self::JwtSigningKey
                | Self::MasterKeyVerification
                | Self::NatsUrl
                | Self::ZeroconfEnabled
                | Self::ZeroconfUrl
                | Self::ZeroconfPkiAddr
                | Self::OauthMcpEnabled
                | Self::OauthDcrEnabled
                | Self::OauthCimdEnabled
                | Self::OauthCanonicalHost
                | Self::OauthAcceptedAudienceHosts
                | Self::OauthAllowMultiControllerUnsafe
                | Self::OauthJwtSigningSecret
                | Self::OauthAccessTokenTtlSecs
                | Self::OauthRefreshTokenTtlSecs
                | Self::OauthRefreshFamilyMaxTtlSecs
                | Self::OauthAuthorizationCodeTtlSecs
                | Self::OauthAuthorizationRequestTtlSecs
                | Self::OauthRateDcrPerHour
                | Self::OauthRateCimdPerMin
                | Self::OauthRateAuthorizePerMin
                | Self::OauthRateTokenPerMin
                | Self::OauthRateConsentPerMin
                | Self::OauthRateMcpAuthFailPerMin
                | Self::OauthCimdCosmeticFieldAllowlist
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
        assert!(SettingKey::JwtSigningKey.is_global());
        // Per-tenant keys
        assert!(!SettingKey::RegistrationMode.is_global());
        assert!(!SettingKey::AuditLogFilter.is_global());
        assert!(!SettingKey::AuditLogRetentionDays.is_global());
    }
}
