use std::{fmt, str::FromStr};

use crate::error::{AuditLogError, Result};

/// Classifies a `RegisteredAuditAction` as either an entity-state mutation
/// (snapshots required) or a discrete event (snapshots forbidden).
///
/// Intentionally closed: adding a third kind is a deliberate contract change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AuditActionKind {
    Stateful,
    Event,
}

impl AuditActionKind {
    /// Returns the canonical lowercase string representation of this kind.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stateful => "stateful",
            Self::Event => "event",
        }
    }
}

impl fmt::Display for AuditActionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AuditActionKind {
    type Err = ();

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value {
            "stateful" => Ok(Self::Stateful),
            "event" => Ok(Self::Event),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RegisteredAuditAction {
    value: &'static str,
    kind: AuditActionKind,
}

impl RegisteredAuditAction {
    /// Creates a new registered audit action with the given string key and kind.
    #[must_use]
    pub const fn new(value: &'static str, kind: AuditActionKind) -> Self {
        Self { value, kind }
    }

    /// Returns the canonical dot-separated string key for this action.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.value
    }

    /// Returns the [`AuditActionKind`] classification for this action.
    #[must_use]
    pub const fn kind(self) -> AuditActionKind {
        self.kind
    }
}

impl fmt::Display for RegisteredAuditAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AuditActionType(String);

impl AuditActionType {
    pub const AUTH_LOGIN: RegisteredAuditAction =
        RegisteredAuditAction::new("auth.login", AuditActionKind::Event);
    pub const AUTH_LOGOUT: RegisteredAuditAction =
        RegisteredAuditAction::new("auth.logout", AuditActionKind::Event);
    pub const AUTH_API_TOKEN_AUTHENTICATE: RegisteredAuditAction =
        RegisteredAuditAction::new("auth.api_token.authenticate", AuditActionKind::Event);
    pub const AUTH_JWT_AUTHENTICATE: RegisteredAuditAction =
        RegisteredAuditAction::new("auth.jwt.authenticate", AuditActionKind::Event);
    pub const AUTH_SERVICE_AUTHENTICATE: RegisteredAuditAction =
        RegisteredAuditAction::new("auth.service.authenticate", AuditActionKind::Event);
    pub const AUTH_SERVICE_REKEY_RESOLVED: RegisteredAuditAction =
        RegisteredAuditAction::new("auth.service.rekey_resolved", AuditActionKind::Event);
    pub const AUTH_TOKEN_REFRESH: RegisteredAuditAction =
        RegisteredAuditAction::new("auth.token_refresh", AuditActionKind::Event);
    pub const AUTH_DEVICE_START: RegisteredAuditAction =
        RegisteredAuditAction::new("auth.device.start", AuditActionKind::Event);
    pub const AUTH_DEVICE_POLL: RegisteredAuditAction =
        RegisteredAuditAction::new("auth.device.poll", AuditActionKind::Event);
    pub const AUTH_DEVICE_APPROVE: RegisteredAuditAction =
        RegisteredAuditAction::new("auth.device.approve", AuditActionKind::Event);
    pub const AUTH_DEVICE_DENY: RegisteredAuditAction =
        RegisteredAuditAction::new("auth.device.deny", AuditActionKind::Event);
    pub const AUTH_OIDC_AUTHORIZE: RegisteredAuditAction =
        RegisteredAuditAction::new("auth.oidc.authorize", AuditActionKind::Event);
    pub const AUTH_OIDC_CALLBACK: RegisteredAuditAction =
        RegisteredAuditAction::new("auth.oidc.callback", AuditActionKind::Event);
    pub const AUTH_OIDC_EXCHANGE: RegisteredAuditAction =
        RegisteredAuditAction::new("auth.oidc.exchange", AuditActionKind::Event);
    pub const AUTH_OIDC_LINK: RegisteredAuditAction =
        RegisteredAuditAction::new("auth.oidc.link", AuditActionKind::Event);
    pub const AUTH_MFA_CHALLENGE_ISSUED: RegisteredAuditAction =
        RegisteredAuditAction::new("auth.mfa.challenge_issued", AuditActionKind::Event);
    pub const AUTH_MFA_VERIFIED: RegisteredAuditAction =
        RegisteredAuditAction::new("auth.mfa.verified", AuditActionKind::Event);
    pub const AUTH_MFA_FAILED: RegisteredAuditAction =
        RegisteredAuditAction::new("auth.mfa.failed", AuditActionKind::Event);
    pub const AUTH_MFA_CHALLENGE_EXHAUSTED: RegisteredAuditAction =
        RegisteredAuditAction::new("auth.mfa.challenge_exhausted", AuditActionKind::Event);
    pub const AUTH_MFA_ENROLLED: RegisteredAuditAction =
        RegisteredAuditAction::new("auth.mfa.enrolled", AuditActionKind::Event);
    pub const AUTH_MFA_DISABLED: RegisteredAuditAction =
        RegisteredAuditAction::new("auth.mfa.disabled", AuditActionKind::Event);
    pub const AUTH_MFA_RECOVERY_USED: RegisteredAuditAction =
        RegisteredAuditAction::new("auth.mfa.recovery_used", AuditActionKind::Event);
    pub const AUTH_MFA_RECOVERY_REGENERATED: RegisteredAuditAction =
        RegisteredAuditAction::new("auth.mfa.recovery_regenerated", AuditActionKind::Event);
    pub const API_TOKEN_CREATE: RegisteredAuditAction =
        RegisteredAuditAction::new("api_token.create", AuditActionKind::Stateful);
    pub const API_TOKEN_REVOKE: RegisteredAuditAction =
        RegisteredAuditAction::new("api_token.revoke", AuditActionKind::Stateful);
    pub const ENROLLMENT_TOKEN_CREATE: RegisteredAuditAction =
        RegisteredAuditAction::new("enrollment_token.create", AuditActionKind::Stateful);
    pub const ENROLLMENT_TOKEN_REVOKE: RegisteredAuditAction =
        RegisteredAuditAction::new("enrollment_token.revoke", AuditActionKind::Stateful);
    pub const USER_CREATE: RegisteredAuditAction =
        RegisteredAuditAction::new("user.create", AuditActionKind::Stateful);
    pub const USER_UPDATE: RegisteredAuditAction =
        RegisteredAuditAction::new("user.update", AuditActionKind::Stateful);
    // USER_DELETE intentionally omitted: no hard-delete handler exists yet.
    // Re-add when a delete-user feature is implemented and the emit site is present.
    pub const OIDC_PROVIDER_CREATE: RegisteredAuditAction =
        RegisteredAuditAction::new("oidc_provider.create", AuditActionKind::Stateful);
    pub const OIDC_PROVIDER_UPDATE: RegisteredAuditAction =
        RegisteredAuditAction::new("oidc_provider.update", AuditActionKind::Stateful);
    pub const OIDC_PROVIDER_DELETE: RegisteredAuditAction =
        RegisteredAuditAction::new("oidc_provider.delete", AuditActionKind::Stateful);
    pub const PLUGIN_CONFIG_CREATE: RegisteredAuditAction =
        RegisteredAuditAction::new("plugin_config.create", AuditActionKind::Stateful);
    pub const PLUGIN_CONFIG_UPDATE: RegisteredAuditAction =
        RegisteredAuditAction::new("plugin_config.update", AuditActionKind::Stateful);
    pub const PLUGIN_CONFIG_DELETE: RegisteredAuditAction =
        RegisteredAuditAction::new("plugin_config.delete", AuditActionKind::Stateful);
    pub const PLUGIN_TYPE_SETTINGS_UPSERT: RegisteredAuditAction =
        RegisteredAuditAction::new("plugin_type_settings.upsert", AuditActionKind::Stateful);
    pub const PLUGIN_TYPE_SETTINGS_DELETE: RegisteredAuditAction =
        RegisteredAuditAction::new("plugin_type_settings.delete", AuditActionKind::Stateful);
    pub const INSTANCE_PLUGIN_TOGGLED: RegisteredAuditAction =
        RegisteredAuditAction::new("instance_plugin.toggled", AuditActionKind::Stateful);
    pub const INSTANCE_PLUGIN_CONFIG_UPSERTED: RegisteredAuditAction =
        RegisteredAuditAction::new("instance_plugin.config_upserted", AuditActionKind::Stateful);
    pub const NOTIFICATION_CHANNEL_CREATE: RegisteredAuditAction =
        RegisteredAuditAction::new("notification_channel.create", AuditActionKind::Stateful);
    pub const NOTIFICATION_CHANNEL_UPDATE: RegisteredAuditAction =
        RegisteredAuditAction::new("notification_channel.update", AuditActionKind::Stateful);
    pub const NOTIFICATION_CHANNEL_DELETE: RegisteredAuditAction =
        RegisteredAuditAction::new("notification_channel.delete", AuditActionKind::Stateful);
    pub const NOTIFICATION_CHANNEL_TEST: RegisteredAuditAction =
        RegisteredAuditAction::new("notification_channel.test", AuditActionKind::Event);
    pub const NOTIFICATION_RULE_CREATE: RegisteredAuditAction =
        RegisteredAuditAction::new("notification_rule.create", AuditActionKind::Stateful);
    pub const NOTIFICATION_RULE_UPDATE: RegisteredAuditAction =
        RegisteredAuditAction::new("notification_rule.update", AuditActionKind::Stateful);
    pub const NOTIFICATION_RULE_DELETE: RegisteredAuditAction =
        RegisteredAuditAction::new("notification_rule.delete", AuditActionKind::Stateful);
    pub const NOTIFICATION_RULE_TEST: RegisteredAuditAction =
        RegisteredAuditAction::new("notification_rule.test", AuditActionKind::Event);
    pub const NOTIFICATION_CALLBACK: RegisteredAuditAction =
        RegisteredAuditAction::new("notification.callback", AuditActionKind::Event);
    pub const GLOBAL_SETTING_UPDATE: RegisteredAuditAction =
        RegisteredAuditAction::new("global_setting.update", AuditActionKind::Stateful);
    pub const TENANT_SETTING_UPDATE: RegisteredAuditAction =
        RegisteredAuditAction::new("tenant_setting.update", AuditActionKind::Stateful);
    pub const TENANT_DATA_RESET: RegisteredAuditAction =
        RegisteredAuditAction::new("tenant.data.reset", AuditActionKind::Event);
    pub const SYSTEM_CA_ROTATE: RegisteredAuditAction =
        RegisteredAuditAction::new("system.ca.rotate", AuditActionKind::Event);
    pub const SYSTEM_SERVER_CERTIFICATE_RENEW: RegisteredAuditAction =
        RegisteredAuditAction::new("system.server_certificate.renew", AuditActionKind::Event);
    pub const SYSTEM_CONFIG_RELOAD_REQUESTED: RegisteredAuditAction =
        RegisteredAuditAction::new("system.config_reload.requested", AuditActionKind::Event);
    pub const SYSTEM_CONFIG_RELOAD_APPLIED: RegisteredAuditAction =
        RegisteredAuditAction::new("system.config_reload.applied", AuditActionKind::Event);
    pub const SYSTEM_CONFIG_RELOAD_FAILED: RegisteredAuditAction =
        RegisteredAuditAction::new("system.config_reload.failed", AuditActionKind::Event);
    pub const SYSTEM_CONFIG_RELOAD_REVERTED: RegisteredAuditAction =
        RegisteredAuditAction::new("system.config_reload.reverted", AuditActionKind::Event);
    pub const SYSTEM_CONFIG_RELOAD_REFUSED: RegisteredAuditAction =
        RegisteredAuditAction::new("system.config_reload.refused", AuditActionKind::Event);
    pub const SYSTEM_ALERT_WRITTEN: RegisteredAuditAction =
        RegisteredAuditAction::new("system.alert.written", AuditActionKind::Event);
    pub const SCHEDULED_TASK_UPDATE: RegisteredAuditAction =
        RegisteredAuditAction::new("scheduled_task.update", AuditActionKind::Stateful);
    pub const SCHEDULED_TASK_TRIGGER: RegisteredAuditAction =
        RegisteredAuditAction::new("scheduled_task.trigger", AuditActionKind::Event);
    pub const HOST_TAG_CREATE: RegisteredAuditAction =
        RegisteredAuditAction::new("host_tag.create", AuditActionKind::Stateful);
    pub const HOST_TAG_UPDATE: RegisteredAuditAction =
        RegisteredAuditAction::new("host_tag.update", AuditActionKind::Stateful);
    pub const HOST_TAG_DELETE: RegisteredAuditAction =
        RegisteredAuditAction::new("host_tag.delete", AuditActionKind::Stateful);
    pub const HOST_TAG_ASSIGN: RegisteredAuditAction =
        RegisteredAuditAction::new("host_tag.assign", AuditActionKind::Event);
    pub const HOST_UPDATE: RegisteredAuditAction =
        RegisteredAuditAction::new("host.update", AuditActionKind::Stateful);
    pub const HOST_DEACTIVATE: RegisteredAuditAction =
        RegisteredAuditAction::new("host.deactivate", AuditActionKind::Stateful);
    pub const HOST_DISCOVER: RegisteredAuditAction =
        RegisteredAuditAction::new("host.discover", AuditActionKind::Event);
    pub const SERVICE_UPDATE: RegisteredAuditAction =
        RegisteredAuditAction::new("service.update", AuditActionKind::Stateful);
    pub const SERVICE_APPROVE: RegisteredAuditAction =
        RegisteredAuditAction::new("service.approve", AuditActionKind::Stateful);
    pub const SERVICE_REJECT: RegisteredAuditAction =
        RegisteredAuditAction::new("service.reject", AuditActionKind::Stateful);
    pub const SERVICE_MERGE: RegisteredAuditAction =
        RegisteredAuditAction::new("service.merge", AuditActionKind::Event);
    // update-freeze is a fire-and-forget WS command; no DB transaction or before/after
    // snapshot is taken, so these are classified as Event (not Stateful).
    pub const SERVICE_UPDATE_FREEZE_ENABLE: RegisteredAuditAction =
        RegisteredAuditAction::new("service.update_freeze.enable", AuditActionKind::Event);
    pub const SERVICE_UPDATE_FREEZE_DISABLE: RegisteredAuditAction =
        RegisteredAuditAction::new("service.update_freeze.disable", AuditActionKind::Event);
    pub const SERVICE_DEACTIVATE: RegisteredAuditAction =
        RegisteredAuditAction::new("service.deactivate", AuditActionKind::Stateful);
    pub const SERVICE_CONFIG_STORE: RegisteredAuditAction =
        RegisteredAuditAction::new("service_config.store", AuditActionKind::Stateful);
    pub const SERVICE_CONFIG_DELETE: RegisteredAuditAction =
        RegisteredAuditAction::new("service_config.delete", AuditActionKind::Stateful);
    pub const SERVICE_CONFIG_DELIVER: RegisteredAuditAction =
        RegisteredAuditAction::new("service_config.deliver", AuditActionKind::Event);
    pub const SERVICE_CERTIFICATE_ISSUE: RegisteredAuditAction =
        RegisteredAuditAction::new("service.certificate.issue", AuditActionKind::Event);
    pub const SERVICE_CERTIFICATE_RENEW: RegisteredAuditAction =
        RegisteredAuditAction::new("service.certificate.renew", AuditActionKind::Event);
    pub const SERVICE_ENROLLMENT_COMPLETED: RegisteredAuditAction =
        RegisteredAuditAction::new("service.enrollment.completed", AuditActionKind::Event);
    pub const SERVICE_CREDENTIALS_DELIVER: RegisteredAuditAction =
        RegisteredAuditAction::new("service.credentials.deliver", AuditActionKind::Event);
    pub const SERVICE_WORKLOAD_CLAIM: RegisteredAuditAction =
        RegisteredAuditAction::new("service.workload.claim", AuditActionKind::Event);
    pub const SERVICE_WORKLOAD_RELEASE: RegisteredAuditAction =
        RegisteredAuditAction::new("service.workload.release", AuditActionKind::Event);
    pub const SURFACE_PROVIDER_REGISTER: RegisteredAuditAction =
        RegisteredAuditAction::new("surface_provider.register", AuditActionKind::Event);
    pub const SURFACE_ACTION_INVOKE: RegisteredAuditAction =
        RegisteredAuditAction::new("surface_action.invoke", AuditActionKind::Event);
    pub const SOFTWARE_IGNORE_CREATE: RegisteredAuditAction =
        RegisteredAuditAction::new("software.ignore.create", AuditActionKind::Stateful);
    pub const SOFTWARE_IGNORE_DELETE: RegisteredAuditAction =
        RegisteredAuditAction::new("software.ignore.delete", AuditActionKind::Stateful);
    pub const DISCOVERY_ALLOWLIST_CREATE: RegisteredAuditAction =
        RegisteredAuditAction::new("discovery_allowlist.create", AuditActionKind::Stateful);
    pub const DISCOVERY_ALLOWLIST_DELETE: RegisteredAuditAction =
        RegisteredAuditAction::new("discovery_allowlist.delete", AuditActionKind::Stateful);
    pub const SOFTWARE_ITEM_CREATE: RegisteredAuditAction =
        RegisteredAuditAction::new("software_item.create", AuditActionKind::Stateful);
    pub const SOFTWARE_ITEM_UPDATE: RegisteredAuditAction =
        RegisteredAuditAction::new("software_item.update", AuditActionKind::Stateful);
    pub const SOFTWARE_ITEM_DELETE: RegisteredAuditAction =
        RegisteredAuditAction::new("software_item.delete", AuditActionKind::Stateful);
    pub const SOFTWARE_ITEM_APPROVE: RegisteredAuditAction =
        RegisteredAuditAction::new("software_item.approve", AuditActionKind::Stateful);
    pub const SOFTWARE_ITEM_ASSIGN_HOSTS: RegisteredAuditAction =
        RegisteredAuditAction::new("software_item.assign_hosts", AuditActionKind::Stateful);
    pub const SOFTWARE_ITEM_UNASSIGN_HOST: RegisteredAuditAction =
        RegisteredAuditAction::new("software_item.unassign_host", AuditActionKind::Stateful);
    pub const SOFTWARE_ITEM_UPDATE_HOST_ASSIGNMENT: RegisteredAuditAction =
        RegisteredAuditAction::new(
            "software_item.update_host_assignment",
            AuditActionKind::Stateful,
        );
    pub const SOFTWARE_ITEM_DELETE_PLUGIN_ASSIGNMENT: RegisteredAuditAction =
        RegisteredAuditAction::new(
            "software_item.delete_plugin_assignment",
            AuditActionKind::Stateful,
        );
    pub const SOFTWARE_ITEM_MERGE: RegisteredAuditAction =
        RegisteredAuditAction::new("software_item.merge", AuditActionKind::Event);
    pub const SOFTWARE_ITEM_BATCH: RegisteredAuditAction =
        RegisteredAuditAction::new("software_item.batch", AuditActionKind::Event);
    pub const SOFTWARE_VERSION_CHECK_TRIGGERED: RegisteredAuditAction =
        RegisteredAuditAction::new("software.version_check.triggered", AuditActionKind::Event);
    pub const SOFTWARE_VERSION_CHECK_COMPLETED: RegisteredAuditAction =
        RegisteredAuditAction::new("software.version_check.completed", AuditActionKind::Event);
    pub const SOFTWARE_UPDATE_TRIGGERED: RegisteredAuditAction =
        RegisteredAuditAction::new("software.update.triggered", AuditActionKind::Event);
    pub const SOFTWARE_BATCH_UPDATE_TRIGGERED: RegisteredAuditAction =
        RegisteredAuditAction::new("software.batch_update.triggered", AuditActionKind::Event);
    pub const SOFTWARE_UPDATE_STARTED: RegisteredAuditAction =
        RegisteredAuditAction::new("software.update.started", AuditActionKind::Event);
    pub const SOFTWARE_BATCH_UPDATE_STARTED: RegisteredAuditAction =
        RegisteredAuditAction::new("software.batch_update.started", AuditActionKind::Event);
    pub const SOFTWARE_UPDATE_FINALIZED: RegisteredAuditAction =
        RegisteredAuditAction::new("software.update.finalized", AuditActionKind::Event);
    pub const SOFTWARE_BATCH_UPDATE_FINALIZED: RegisteredAuditAction =
        RegisteredAuditAction::new("software.batch_update.finalized", AuditActionKind::Event);
    pub const SOFTWARE_UPDATE_STDIN_ATTENTION: RegisteredAuditAction =
        RegisteredAuditAction::new("software.update.stdin_attention", AuditActionKind::Event);
    pub const SOFTWARE_UPDATE_INTERACTIVE_CONTROL: RegisteredAuditAction =
        RegisteredAuditAction::new(
            "software.update.interactive_control",
            AuditActionKind::Event,
        );
    pub const SOFTWARE_ITEM_ENRICH: RegisteredAuditAction =
        RegisteredAuditAction::new("software_item.enrich", AuditActionKind::Event);
    pub const SYSTEM_SERVICE_UPDATE_GATE: RegisteredAuditAction =
        RegisteredAuditAction::new("system.service.update_gate", AuditActionKind::Event);
    pub const SYSTEM_SERVICE_MACHINE_ID_VALIDATE: RegisteredAuditAction =
        RegisteredAuditAction::new("system.service.machine_id.validate", AuditActionKind::Event);
    pub const SYSTEM_SERVICE_UPDATE_FREEZE_APPLY: RegisteredAuditAction =
        RegisteredAuditAction::new("system.service.update_freeze.apply", AuditActionKind::Event);
    pub const SYSTEM_SCHEDULER_AUDIT_LOG_CLEANUP: RegisteredAuditAction =
        RegisteredAuditAction::new("system.scheduler.audit_log_cleanup", AuditActionKind::Event);
    pub const OAUTH_AUTHORIZE_REQUEST: RegisteredAuditAction =
        RegisteredAuditAction::new("oauth.authorize_request", AuditActionKind::Event);
    pub const OAUTH_TOKEN_ISSUED: RegisteredAuditAction =
        RegisteredAuditAction::new("oauth.token_issued", AuditActionKind::Event);
    pub const OAUTH_TOKEN_REJECTED: RegisteredAuditAction =
        RegisteredAuditAction::new("oauth.token_rejected", AuditActionKind::Event);
    pub const OAUTH_REFRESH_ROTATED: RegisteredAuditAction =
        RegisteredAuditAction::new("oauth.refresh_rotated", AuditActionKind::Event);
    pub const OAUTH_REFRESH_REPLAY_DETECTED: RegisteredAuditAction =
        RegisteredAuditAction::new("oauth.refresh_replay_detected", AuditActionKind::Event);
    pub const OAUTH_CLIENT_REGISTERED: RegisteredAuditAction =
        RegisteredAuditAction::new("oauth.client_registered", AuditActionKind::Event);
    pub const OAUTH_CLIENT_FIRST_USE: RegisteredAuditAction =
        RegisteredAuditAction::new("oauth.client_first_use", AuditActionKind::Event);
    pub const OAUTH_CLIENT_METADATA_REFRESHED: RegisteredAuditAction =
        RegisteredAuditAction::new("oauth.client_metadata_refreshed", AuditActionKind::Event);
    pub const OAUTH_CLIENT_METADATA_CHANGED_MATERIALLY: RegisteredAuditAction =
        RegisteredAuditAction::new(
            "oauth.client_metadata_changed_materially",
            AuditActionKind::Event,
        );
    pub const OAUTH_CLIENT_TRUSTED: RegisteredAuditAction =
        RegisteredAuditAction::new("oauth.client_trusted", AuditActionKind::Event);
    pub const OAUTH_CLIENT_REVOKED: RegisteredAuditAction =
        RegisteredAuditAction::new("oauth.client_revoked", AuditActionKind::Event);
    pub const OAUTH_CLIENT_REGISTRATION_RATE_LIMITED: RegisteredAuditAction =
        RegisteredAuditAction::new(
            "oauth.client_registration_rate_limited",
            AuditActionKind::Event,
        );
    pub const OAUTH_CONFIG_AUDIENCE_HOSTS_CHANGED: RegisteredAuditAction =
        RegisteredAuditAction::new(
            "oauth.config_audience_hosts_changed",
            AuditActionKind::Event,
        );
    pub const OAUTH_CIMD_PARSE_FAILED: RegisteredAuditAction =
        RegisteredAuditAction::new("oauth.cimd_parse_failed", AuditActionKind::Event);
    pub const OAUTH_CONSENT_GRANT: RegisteredAuditAction =
        RegisteredAuditAction::new("oauth.consent_grant", AuditActionKind::Event);
    pub const OAUTH_CONSENT_DENY: RegisteredAuditAction =
        RegisteredAuditAction::new("oauth.consent_deny", AuditActionKind::Event);
    pub const OAUTH_CONSENT_REVOKE: RegisteredAuditAction =
        RegisteredAuditAction::new("oauth.consent_revoke", AuditActionKind::Event);
    pub const OAUTH_RATE_LIMITED: RegisteredAuditAction =
        RegisteredAuditAction::new("oauth.rate_limited", AuditActionKind::Event);
    pub const MCP_OAUTH_AUTHENTICATE: RegisteredAuditAction =
        RegisteredAuditAction::new("mcp.oauth_authenticate", AuditActionKind::Event);

    fn parse_any(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        validate_action_type(&value)?;
        Ok(Self(value))
    }

    /// Creates a new `AuditActionType` from a string, validating its format.
    ///
    /// # Errors
    ///
    /// Returns [`AuditLogError::Validation`] if the value is empty, exceeds 128
    /// bytes, has fewer than two dot-separated segments, contains a reserved
    /// result suffix, or contains characters outside `[a-z0-9_]`.
    #[must_use = "the validated AuditActionType must be used"]
    pub fn new(value: impl Into<String>) -> Result<Self> {
        Self::parse_any(value)
    }

    #[must_use]
    pub const fn from_static(value: RegisteredAuditAction) -> RegisteredAuditAction {
        value
    }

    /// Returns the canonical dot-separated string key for this action type.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns `true` if the given string matches a registered V1 audit action.
    #[must_use]
    pub fn is_registered(value: &str) -> bool {
        Self::lookup_registered(value).is_some()
    }

    /// Returns the [`AuditActionKind`] for this action type, or `None` if the
    /// action type is not registered.
    #[must_use]
    pub fn kind(&self) -> Option<AuditActionKind> {
        Self::lookup_registered(self.0.as_str()).map(|a| a.kind())
    }

    /// Returns every registered V1 audit action.
    #[must_use]
    pub fn variants() -> &'static [RegisteredAuditAction] {
        V1_ACTIONS
    }

    fn lookup_registered(value: &str) -> Option<RegisteredAuditAction> {
        V1_ACTIONS.iter().copied().find(|a| a.as_str() == value)
    }
}

const V1_ACTIONS: &[RegisteredAuditAction] = &[
    AuditActionType::AUTH_LOGIN,
    AuditActionType::AUTH_LOGOUT,
    AuditActionType::AUTH_API_TOKEN_AUTHENTICATE,
    AuditActionType::AUTH_JWT_AUTHENTICATE,
    AuditActionType::AUTH_SERVICE_AUTHENTICATE,
    AuditActionType::AUTH_SERVICE_REKEY_RESOLVED,
    AuditActionType::AUTH_TOKEN_REFRESH,
    AuditActionType::AUTH_DEVICE_START,
    AuditActionType::AUTH_DEVICE_POLL,
    AuditActionType::AUTH_DEVICE_APPROVE,
    AuditActionType::AUTH_DEVICE_DENY,
    AuditActionType::AUTH_OIDC_AUTHORIZE,
    AuditActionType::AUTH_OIDC_CALLBACK,
    AuditActionType::AUTH_OIDC_EXCHANGE,
    AuditActionType::AUTH_OIDC_LINK,
    AuditActionType::AUTH_MFA_CHALLENGE_ISSUED,
    AuditActionType::AUTH_MFA_VERIFIED,
    AuditActionType::AUTH_MFA_FAILED,
    AuditActionType::AUTH_MFA_CHALLENGE_EXHAUSTED,
    AuditActionType::AUTH_MFA_ENROLLED,
    AuditActionType::AUTH_MFA_DISABLED,
    AuditActionType::AUTH_MFA_RECOVERY_USED,
    AuditActionType::AUTH_MFA_RECOVERY_REGENERATED,
    AuditActionType::API_TOKEN_CREATE,
    AuditActionType::API_TOKEN_REVOKE,
    AuditActionType::ENROLLMENT_TOKEN_CREATE,
    AuditActionType::ENROLLMENT_TOKEN_REVOKE,
    AuditActionType::USER_CREATE,
    AuditActionType::USER_UPDATE,
    AuditActionType::OIDC_PROVIDER_CREATE,
    AuditActionType::OIDC_PROVIDER_UPDATE,
    AuditActionType::OIDC_PROVIDER_DELETE,
    AuditActionType::PLUGIN_CONFIG_CREATE,
    AuditActionType::PLUGIN_CONFIG_UPDATE,
    AuditActionType::PLUGIN_CONFIG_DELETE,
    AuditActionType::PLUGIN_TYPE_SETTINGS_UPSERT,
    AuditActionType::PLUGIN_TYPE_SETTINGS_DELETE,
    AuditActionType::INSTANCE_PLUGIN_TOGGLED,
    AuditActionType::INSTANCE_PLUGIN_CONFIG_UPSERTED,
    AuditActionType::NOTIFICATION_CHANNEL_CREATE,
    AuditActionType::NOTIFICATION_CHANNEL_UPDATE,
    AuditActionType::NOTIFICATION_CHANNEL_DELETE,
    AuditActionType::NOTIFICATION_CHANNEL_TEST,
    AuditActionType::NOTIFICATION_RULE_CREATE,
    AuditActionType::NOTIFICATION_RULE_UPDATE,
    AuditActionType::NOTIFICATION_RULE_DELETE,
    AuditActionType::NOTIFICATION_RULE_TEST,
    AuditActionType::NOTIFICATION_CALLBACK,
    AuditActionType::GLOBAL_SETTING_UPDATE,
    AuditActionType::TENANT_SETTING_UPDATE,
    AuditActionType::TENANT_DATA_RESET,
    AuditActionType::SYSTEM_CA_ROTATE,
    AuditActionType::SYSTEM_SERVER_CERTIFICATE_RENEW,
    AuditActionType::SYSTEM_CONFIG_RELOAD_REQUESTED,
    AuditActionType::SYSTEM_CONFIG_RELOAD_APPLIED,
    AuditActionType::SYSTEM_CONFIG_RELOAD_FAILED,
    AuditActionType::SYSTEM_CONFIG_RELOAD_REVERTED,
    AuditActionType::SYSTEM_CONFIG_RELOAD_REFUSED,
    AuditActionType::SYSTEM_ALERT_WRITTEN,
    AuditActionType::SCHEDULED_TASK_UPDATE,
    AuditActionType::SCHEDULED_TASK_TRIGGER,
    AuditActionType::HOST_TAG_CREATE,
    AuditActionType::HOST_TAG_UPDATE,
    AuditActionType::HOST_TAG_DELETE,
    AuditActionType::HOST_TAG_ASSIGN,
    AuditActionType::HOST_UPDATE,
    AuditActionType::HOST_DEACTIVATE,
    AuditActionType::HOST_DISCOVER,
    AuditActionType::SERVICE_UPDATE,
    AuditActionType::SERVICE_APPROVE,
    AuditActionType::SERVICE_REJECT,
    AuditActionType::SERVICE_MERGE,
    AuditActionType::SERVICE_UPDATE_FREEZE_ENABLE,
    AuditActionType::SERVICE_UPDATE_FREEZE_DISABLE,
    AuditActionType::SERVICE_DEACTIVATE,
    AuditActionType::SERVICE_CONFIG_STORE,
    AuditActionType::SERVICE_CONFIG_DELETE,
    AuditActionType::SERVICE_CONFIG_DELIVER,
    AuditActionType::SERVICE_CERTIFICATE_ISSUE,
    AuditActionType::SERVICE_CERTIFICATE_RENEW,
    AuditActionType::SERVICE_ENROLLMENT_COMPLETED,
    AuditActionType::SERVICE_CREDENTIALS_DELIVER,
    AuditActionType::SERVICE_WORKLOAD_CLAIM,
    AuditActionType::SERVICE_WORKLOAD_RELEASE,
    AuditActionType::SURFACE_PROVIDER_REGISTER,
    AuditActionType::SURFACE_ACTION_INVOKE,
    AuditActionType::SOFTWARE_IGNORE_CREATE,
    AuditActionType::SOFTWARE_IGNORE_DELETE,
    AuditActionType::DISCOVERY_ALLOWLIST_CREATE,
    AuditActionType::DISCOVERY_ALLOWLIST_DELETE,
    AuditActionType::SOFTWARE_ITEM_CREATE,
    AuditActionType::SOFTWARE_ITEM_UPDATE,
    AuditActionType::SOFTWARE_ITEM_DELETE,
    AuditActionType::SOFTWARE_ITEM_APPROVE,
    AuditActionType::SOFTWARE_ITEM_ASSIGN_HOSTS,
    AuditActionType::SOFTWARE_ITEM_UNASSIGN_HOST,
    AuditActionType::SOFTWARE_ITEM_UPDATE_HOST_ASSIGNMENT,
    AuditActionType::SOFTWARE_ITEM_DELETE_PLUGIN_ASSIGNMENT,
    AuditActionType::SOFTWARE_ITEM_MERGE,
    AuditActionType::SOFTWARE_ITEM_BATCH,
    AuditActionType::SOFTWARE_VERSION_CHECK_TRIGGERED,
    AuditActionType::SOFTWARE_VERSION_CHECK_COMPLETED,
    AuditActionType::SOFTWARE_UPDATE_TRIGGERED,
    AuditActionType::SOFTWARE_BATCH_UPDATE_TRIGGERED,
    AuditActionType::SOFTWARE_UPDATE_STARTED,
    AuditActionType::SOFTWARE_BATCH_UPDATE_STARTED,
    AuditActionType::SOFTWARE_UPDATE_FINALIZED,
    AuditActionType::SOFTWARE_BATCH_UPDATE_FINALIZED,
    AuditActionType::SOFTWARE_UPDATE_STDIN_ATTENTION,
    AuditActionType::SOFTWARE_UPDATE_INTERACTIVE_CONTROL,
    AuditActionType::SOFTWARE_ITEM_ENRICH,
    AuditActionType::SYSTEM_SERVICE_UPDATE_GATE,
    AuditActionType::SYSTEM_SERVICE_MACHINE_ID_VALIDATE,
    AuditActionType::SYSTEM_SERVICE_UPDATE_FREEZE_APPLY,
    AuditActionType::SYSTEM_SCHEDULER_AUDIT_LOG_CLEANUP,
    AuditActionType::OAUTH_AUTHORIZE_REQUEST,
    AuditActionType::OAUTH_TOKEN_ISSUED,
    AuditActionType::OAUTH_TOKEN_REJECTED,
    AuditActionType::OAUTH_REFRESH_ROTATED,
    AuditActionType::OAUTH_REFRESH_REPLAY_DETECTED,
    AuditActionType::OAUTH_CLIENT_REGISTERED,
    AuditActionType::OAUTH_CLIENT_FIRST_USE,
    AuditActionType::OAUTH_CLIENT_METADATA_REFRESHED,
    AuditActionType::OAUTH_CLIENT_METADATA_CHANGED_MATERIALLY,
    AuditActionType::OAUTH_CLIENT_TRUSTED,
    AuditActionType::OAUTH_CLIENT_REVOKED,
    AuditActionType::OAUTH_CLIENT_REGISTRATION_RATE_LIMITED,
    AuditActionType::OAUTH_CONFIG_AUDIENCE_HOSTS_CHANGED,
    AuditActionType::OAUTH_CIMD_PARSE_FAILED,
    AuditActionType::OAUTH_CONSENT_GRANT,
    AuditActionType::OAUTH_CONSENT_DENY,
    AuditActionType::OAUTH_CONSENT_REVOKE,
    AuditActionType::OAUTH_RATE_LIMITED,
    AuditActionType::MCP_OAUTH_AUTHENTICATE,
];

impl fmt::Display for AuditActionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl PartialEq<str> for AuditActionType {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for AuditActionType {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<RegisteredAuditAction> for AuditActionType {
    fn eq(&self, other: &RegisteredAuditAction) -> bool {
        self.0 == other.value
    }
}

impl PartialEq<String> for RegisteredAuditAction {
    fn eq(&self, other: &String) -> bool {
        self.value == other
    }
}

impl PartialEq<str> for RegisteredAuditAction {
    fn eq(&self, other: &str) -> bool {
        self.value == other
    }
}

#[cfg(feature = "db")]
impl From<RegisteredAuditAction> for sea_orm::Value {
    fn from(action: RegisteredAuditAction) -> Self {
        sea_orm::Value::String(Some(action.value.to_owned()))
    }
}

impl From<RegisteredAuditAction> for AuditActionType {
    fn from(value: RegisteredAuditAction) -> Self {
        Self(value.as_str().to_string())
    }
}

impl FromStr for AuditActionType {
    type Err = rootcause::Report<AuditLogError>;

    fn from_str(value: &str) -> Result<Self> {
        let action = Self::parse_any(value)?;
        if !Self::is_registered(action.as_str()) {
            return Err(rootcause::report!(AuditLogError::Validation(format!(
                "action_type is not registered: {}",
                action.as_str()
            ))));
        }
        Ok(action)
    }
}

fn validate_action_type(value: &str) -> Result<()> {
    static RESERVED_RESULT_SEGMENTS: &[&str] = &[
        "failed",
        "success",
        "denied",
        "partial",
        "error",
        "validation_failed",
    ];

    if value.is_empty() {
        return Err(rootcause::report!(AuditLogError::Validation(
            "action type must not be empty".to_string()
        )));
    }
    if value.len() > 128 {
        return Err(rootcause::report!(AuditLogError::Validation(
            "action type must be <= 128 bytes".to_string()
        )));
    }

    let segments: Vec<_> = value.split('.').collect();
    if segments.len() < 2 {
        return Err(rootcause::report!(AuditLogError::Validation(
            "action type must have at least two segments".to_string()
        )));
    }

    for segment in segments {
        if segment.is_empty() {
            return Err(rootcause::report!(AuditLogError::Validation(
                "action type contains an empty segment".to_string()
            )));
        }
        if RESERVED_RESULT_SEGMENTS.contains(&segment) {
            return Err(rootcause::report!(AuditLogError::Validation(
                "action type must not encode result in its name".to_string()
            )));
        }
        if !segment
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(rootcause::report!(AuditLogError::Validation(
                "action type segments must contain only [a-z0-9_]".to_string()
            )));
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Per-action AuditEntry constructor methods
// ---------------------------------------------------------------------------

uptrakit_audit_log_derive::audit_actions! {
    // auth — Event
    auth_login => AUTH_LOGIN, Event;
    auth_logout => AUTH_LOGOUT, Event;
    auth_api_token_authenticate => AUTH_API_TOKEN_AUTHENTICATE, Event;
    auth_jwt_authenticate => AUTH_JWT_AUTHENTICATE, Event;
    auth_service_authenticate => AUTH_SERVICE_AUTHENTICATE, Event;
    auth_service_rekey_resolved => AUTH_SERVICE_REKEY_RESOLVED, Event;
    auth_token_refresh => AUTH_TOKEN_REFRESH, Event;
    auth_device_start => AUTH_DEVICE_START, Event;
    auth_device_poll => AUTH_DEVICE_POLL, Event;
    auth_device_approve => AUTH_DEVICE_APPROVE, Event;
    auth_device_deny => AUTH_DEVICE_DENY, Event;
    auth_oidc_authorize => AUTH_OIDC_AUTHORIZE, Event;
    auth_oidc_callback => AUTH_OIDC_CALLBACK, Event;
    auth_oidc_exchange => AUTH_OIDC_EXCHANGE, Event;
    auth_oidc_link => AUTH_OIDC_LINK, Event;
    auth_mfa_challenge_issued => AUTH_MFA_CHALLENGE_ISSUED, Event;
    auth_mfa_verified => AUTH_MFA_VERIFIED, Event;
    auth_mfa_failed => AUTH_MFA_FAILED, Event;
    auth_mfa_challenge_exhausted => AUTH_MFA_CHALLENGE_EXHAUSTED, Event;
    auth_mfa_enrolled => AUTH_MFA_ENROLLED, Event;
    auth_mfa_disabled => AUTH_MFA_DISABLED, Event;
    auth_mfa_recovery_used => AUTH_MFA_RECOVERY_USED, Event;
    auth_mfa_recovery_regenerated => AUTH_MFA_RECOVERY_REGENERATED, Event;

    // tokens — Stateful
    api_token_create => API_TOKEN_CREATE, Stateful;
    api_token_revoke => API_TOKEN_REVOKE, Stateful;
    enrollment_token_create => ENROLLMENT_TOKEN_CREATE, Stateful;
    enrollment_token_revoke => ENROLLMENT_TOKEN_REVOKE, Stateful;

    // users — Stateful
    user_create => USER_CREATE, Stateful;
    user_update => USER_UPDATE, Stateful;

    // oidc providers — Stateful
    oidc_provider_create => OIDC_PROVIDER_CREATE, Stateful;
    oidc_provider_update => OIDC_PROVIDER_UPDATE, Stateful;
    oidc_provider_delete => OIDC_PROVIDER_DELETE, Stateful;

    // plugin config — Stateful
    plugin_config_create => PLUGIN_CONFIG_CREATE, Stateful;
    plugin_config_update => PLUGIN_CONFIG_UPDATE, Stateful;
    plugin_config_delete => PLUGIN_CONFIG_DELETE, Stateful;
    plugin_type_settings_upsert => PLUGIN_TYPE_SETTINGS_UPSERT, Stateful;
    plugin_type_settings_delete => PLUGIN_TYPE_SETTINGS_DELETE, Stateful;
    instance_plugin_toggled => INSTANCE_PLUGIN_TOGGLED, Stateful;
    instance_plugin_config_upserted => INSTANCE_PLUGIN_CONFIG_UPSERTED, Stateful;

    // notifications
    notification_channel_create => NOTIFICATION_CHANNEL_CREATE, Stateful;
    notification_channel_update => NOTIFICATION_CHANNEL_UPDATE, Stateful;
    notification_channel_delete => NOTIFICATION_CHANNEL_DELETE, Stateful;
    notification_channel_test => NOTIFICATION_CHANNEL_TEST, Event;
    notification_rule_create => NOTIFICATION_RULE_CREATE, Stateful;
    notification_rule_update => NOTIFICATION_RULE_UPDATE, Stateful;
    notification_rule_delete => NOTIFICATION_RULE_DELETE, Stateful;
    notification_rule_test => NOTIFICATION_RULE_TEST, Event;
    notification_callback => NOTIFICATION_CALLBACK, Event;

    // settings
    global_setting_update => GLOBAL_SETTING_UPDATE, Stateful;
    tenant_setting_update => TENANT_SETTING_UPDATE, Stateful;
    tenant_data_reset => TENANT_DATA_RESET, Event;

    // CA + server certificate
    system_ca_rotate => SYSTEM_CA_ROTATE, Event;
    system_server_certificate_renew => SYSTEM_SERVER_CERTIFICATE_RENEW, Event;

    // config reload lifecycle
    system_config_reload_requested => SYSTEM_CONFIG_RELOAD_REQUESTED, Event;
    system_config_reload_applied => SYSTEM_CONFIG_RELOAD_APPLIED, Event;
    system_config_reload_failed => SYSTEM_CONFIG_RELOAD_FAILED, Event;
    system_config_reload_reverted => SYSTEM_CONFIG_RELOAD_REVERTED, Event;
    system_config_reload_refused => SYSTEM_CONFIG_RELOAD_REFUSED, Event;

    // scheduled tasks
    scheduled_task_update => SCHEDULED_TASK_UPDATE, Stateful;
    scheduled_task_trigger => SCHEDULED_TASK_TRIGGER, Event;

    // hosts + tags
    host_tag_create => HOST_TAG_CREATE, Stateful;
    host_tag_update => HOST_TAG_UPDATE, Stateful;
    host_tag_delete => HOST_TAG_DELETE, Stateful;
    host_tag_assign => HOST_TAG_ASSIGN, Event;
    host_update => HOST_UPDATE, Stateful;
    host_deactivate => HOST_DEACTIVATE, Stateful;
    host_discover => HOST_DISCOVER, Event;

    // services
    service_update => SERVICE_UPDATE, Stateful;
    service_approve => SERVICE_APPROVE, Stateful;
    service_reject => SERVICE_REJECT, Stateful;
    service_merge => SERVICE_MERGE, Event;
    service_update_freeze_enable => SERVICE_UPDATE_FREEZE_ENABLE, Event;
    service_update_freeze_disable => SERVICE_UPDATE_FREEZE_DISABLE, Event;
    service_deactivate => SERVICE_DEACTIVATE, Stateful;
    service_config_store => SERVICE_CONFIG_STORE, Stateful;
    service_config_delete => SERVICE_CONFIG_DELETE, Stateful;
    service_config_deliver => SERVICE_CONFIG_DELIVER, Event;
    service_certificate_issue => SERVICE_CERTIFICATE_ISSUE, Event;
    service_certificate_renew => SERVICE_CERTIFICATE_RENEW, Event;
    service_enrollment_completed => SERVICE_ENROLLMENT_COMPLETED, Event;
    service_credentials_deliver => SERVICE_CREDENTIALS_DELIVER, Event;
    service_workload_claim => SERVICE_WORKLOAD_CLAIM, Event;
    service_workload_release => SERVICE_WORKLOAD_RELEASE, Event;

    // surfaces
    surface_provider_register => SURFACE_PROVIDER_REGISTER, Event;
    surface_action_invoke => SURFACE_ACTION_INVOKE, Event;

    // software item registry
    software_item_create => SOFTWARE_ITEM_CREATE, Stateful;
    software_item_update => SOFTWARE_ITEM_UPDATE, Stateful;
    software_item_delete => SOFTWARE_ITEM_DELETE, Stateful;
    software_item_approve => SOFTWARE_ITEM_APPROVE, Stateful;
    software_item_assign_hosts => SOFTWARE_ITEM_ASSIGN_HOSTS, Stateful;
    software_item_unassign_host => SOFTWARE_ITEM_UNASSIGN_HOST, Stateful;
    software_item_update_host_assignment => SOFTWARE_ITEM_UPDATE_HOST_ASSIGNMENT, Stateful;
    software_item_delete_plugin_assignment => SOFTWARE_ITEM_DELETE_PLUGIN_ASSIGNMENT, Stateful;
    software_item_merge => SOFTWARE_ITEM_MERGE, Event;
    software_item_batch => SOFTWARE_ITEM_BATCH, Event;
    software_item_enrich => SOFTWARE_ITEM_ENRICH, Event;

    software_ignore_create => SOFTWARE_IGNORE_CREATE, Stateful;
    software_ignore_delete => SOFTWARE_IGNORE_DELETE, Stateful;
    discovery_allowlist_create => DISCOVERY_ALLOWLIST_CREATE, Stateful;
    discovery_allowlist_delete => DISCOVERY_ALLOWLIST_DELETE, Stateful;

    // software workflow
    software_version_check_triggered => SOFTWARE_VERSION_CHECK_TRIGGERED, Event;
    software_version_check_completed => SOFTWARE_VERSION_CHECK_COMPLETED, Event;
    software_update_triggered => SOFTWARE_UPDATE_TRIGGERED, Event;
    software_batch_update_triggered => SOFTWARE_BATCH_UPDATE_TRIGGERED, Event;
    software_update_started => SOFTWARE_UPDATE_STARTED, Event;
    software_batch_update_started => SOFTWARE_BATCH_UPDATE_STARTED, Event;
    software_update_finalized => SOFTWARE_UPDATE_FINALIZED, Event;
    software_batch_update_finalized => SOFTWARE_BATCH_UPDATE_FINALIZED, Event;
    software_update_stdin_attention => SOFTWARE_UPDATE_STDIN_ATTENTION, Event;
    software_update_interactive_control => SOFTWARE_UPDATE_INTERACTIVE_CONTROL, Event;

    // system service runtime
    system_service_update_gate => SYSTEM_SERVICE_UPDATE_GATE, Event;
    system_service_machine_id_validate => SYSTEM_SERVICE_MACHINE_ID_VALIDATE, Event;
    system_service_update_freeze_apply => SYSTEM_SERVICE_UPDATE_FREEZE_APPLY, Event;
    system_scheduler_audit_log_cleanup => SYSTEM_SCHEDULER_AUDIT_LOG_CLEANUP, Event;

    // oauth
    oauth_authorize_request => OAUTH_AUTHORIZE_REQUEST, Event;
    oauth_token_issued => OAUTH_TOKEN_ISSUED, Event;
    oauth_token_rejected => OAUTH_TOKEN_REJECTED, Event;
    oauth_refresh_rotated => OAUTH_REFRESH_ROTATED, Event;
    oauth_refresh_replay_detected => OAUTH_REFRESH_REPLAY_DETECTED, Event;
    oauth_client_registered => OAUTH_CLIENT_REGISTERED, Event;
    oauth_client_first_use => OAUTH_CLIENT_FIRST_USE, Event;
    oauth_client_metadata_refreshed => OAUTH_CLIENT_METADATA_REFRESHED, Event;
    oauth_client_metadata_changed_materially => OAUTH_CLIENT_METADATA_CHANGED_MATERIALLY, Event;
    oauth_client_trusted => OAUTH_CLIENT_TRUSTED, Event;
    oauth_client_revoked => OAUTH_CLIENT_REVOKED, Event;
    oauth_client_registration_rate_limited => OAUTH_CLIENT_REGISTRATION_RATE_LIMITED, Event;
    oauth_config_audience_hosts_changed => OAUTH_CONFIG_AUDIENCE_HOSTS_CHANGED, Event;
    oauth_cimd_parse_failed => OAUTH_CIMD_PARSE_FAILED, Event;
    oauth_consent_grant => OAUTH_CONSENT_GRANT, Event;
    oauth_consent_deny => OAUTH_CONSENT_DENY, Event;
    oauth_consent_revoke => OAUTH_CONSENT_REVOKE, Event;
    oauth_rate_limited => OAUTH_RATE_LIMITED, Event;
    mcp_oauth_authenticate => MCP_OAUTH_AUTHENTICATE, Event
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — assert!(result.is_ok/is_err()) is idiomatic in tests"
    )]

    use super::{AuditActionKind, AuditActionType, RegisteredAuditAction};
    use std::str::FromStr;

    #[test]
    fn audit_action_type_rejects_result_encoded_names() {
        assert!("auth.login".parse::<AuditActionType>().is_ok());
        assert!("auth.login.failed".parse::<AuditActionType>().is_err());
    }

    #[test]
    fn audit_action_type_rejects_validation_failed_suffix() {
        assert!(
            "service.merge.validation_failed"
                .parse::<AuditActionType>()
                .is_err()
        );
    }

    #[test]
    fn audit_action_type_accepts_system_update_freeze_apply() {
        assert!(
            "system.service.update_freeze.apply"
                .parse::<AuditActionType>()
                .is_ok()
        );
    }

    #[test]
    fn audit_action_type_registry_includes_surface_provider_register() {
        assert!(
            AuditActionType::SURFACE_PROVIDER_REGISTER
                .as_str()
                .parse::<AuditActionType>()
                .is_ok()
        );
        assert!(AuditActionType::is_registered(
            AuditActionType::SURFACE_PROVIDER_REGISTER.as_str()
        ));
    }

    #[test]
    fn audit_action_type_registry_includes_auth_token_refresh() {
        assert!(AuditActionType::is_registered(
            AuditActionType::AUTH_TOKEN_REFRESH.as_str()
        ));
    }

    #[test]
    fn audit_action_type_registry_includes_new_auth_actions() {
        assert!(AuditActionType::is_registered(
            AuditActionType::AUTH_LOGOUT.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::AUTH_DEVICE_START.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::AUTH_DEVICE_POLL.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::AUTH_DEVICE_APPROVE.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::AUTH_DEVICE_DENY.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::AUTH_OIDC_EXCHANGE.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::AUTH_OIDC_LINK.as_str()
        ));
    }

    #[test]
    fn auth_service_rekey_resolved_is_registered() {
        assert!(
            AuditActionType::AUTH_SERVICE_REKEY_RESOLVED
                .as_str()
                .starts_with("auth.service.")
        );
        let parsed: AuditActionType = "auth.service.rekey_resolved".parse().unwrap();
        assert_eq!(parsed.as_str(), "auth.service.rekey_resolved");
    }

    #[test]
    fn audit_action_type_registry_matches_spec_taxonomy() {
        assert!(AuditActionType::is_registered(
            AuditActionType::PLUGIN_TYPE_SETTINGS_UPSERT.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::PLUGIN_TYPE_SETTINGS_DELETE.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SERVICE_UPDATE_FREEZE_ENABLE.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SERVICE_UPDATE_FREEZE_DISABLE.as_str()
        ));
    }

    #[test]
    fn audit_action_type_registry_includes_software_item_actions() {
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_ITEM_CREATE.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_ITEM_UPDATE.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_ITEM_DELETE.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_ITEM_APPROVE.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_ITEM_ASSIGN_HOSTS.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_ITEM_UNASSIGN_HOST.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_ITEM_UPDATE_HOST_ASSIGNMENT.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_ITEM_DELETE_PLUGIN_ASSIGNMENT.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_ITEM_MERGE.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_ITEM_BATCH.as_str()
        ));
    }

    #[test]
    fn audit_action_type_registry_includes_service_config_actions() {
        assert!(AuditActionType::is_registered(
            AuditActionType::SERVICE_CONFIG_STORE.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SERVICE_CONFIG_DELETE.as_str()
        ));
    }

    #[test]
    fn audit_action_type_registry_includes_software_lifecycle_actions() {
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_VERSION_CHECK_TRIGGERED.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_VERSION_CHECK_COMPLETED.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_UPDATE_FINALIZED.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_BATCH_UPDATE_FINALIZED.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_UPDATE_STDIN_ATTENTION.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_ITEM_ENRICH.as_str()
        ));
    }

    #[test]
    fn audit_action_type_new_accepts_registered_action_inputs() {
        assert!(AuditActionType::new("auth.login").is_ok());
        assert!(AuditActionType::new("auth.login".to_string()).is_ok());
    }

    #[test]
    fn audit_action_type_from_str_validates_registry() {
        assert!(AuditActionType::from_str("auth.login").is_ok());
        assert!(AuditActionType::from_str("auth.login.failed").is_err());
    }

    #[test]
    fn audit_action_kind_as_str_round_trip() {
        assert_eq!(AuditActionKind::Stateful.as_str(), "stateful");
        assert_eq!(AuditActionKind::Event.as_str(), "event");
        assert_eq!(
            AuditActionKind::from_str("stateful"),
            Ok(AuditActionKind::Stateful)
        );
        assert_eq!(
            AuditActionKind::from_str("event"),
            Ok(AuditActionKind::Event)
        );
        assert!(AuditActionKind::from_str("other").is_err());
    }

    #[test]
    fn registered_action_carries_kind() {
        assert_eq!(AuditActionType::AUTH_LOGIN.kind(), AuditActionKind::Event);
        assert_eq!(
            AuditActionType::PLUGIN_CONFIG_UPDATE.kind(),
            AuditActionKind::Stateful
        );
    }

    #[test]
    fn mfa_audit_actions_are_registered() {
        assert!(AuditActionType::is_registered(
            AuditActionType::AUTH_MFA_CHALLENGE_ISSUED.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::AUTH_MFA_VERIFIED.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::AUTH_MFA_FAILED.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::AUTH_MFA_CHALLENGE_EXHAUSTED.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::AUTH_MFA_ENROLLED.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::AUTH_MFA_DISABLED.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::AUTH_MFA_RECOVERY_USED.as_str()
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::AUTH_MFA_RECOVERY_REGENERATED.as_str()
        ));
    }

    #[test]
    fn oauth_actions_have_stable_strings() {
        let expected: &[(RegisteredAuditAction, &str)] = &[
            (
                AuditActionType::OAUTH_AUTHORIZE_REQUEST,
                "oauth.authorize_request",
            ),
            (AuditActionType::OAUTH_TOKEN_ISSUED, "oauth.token_issued"),
            (
                AuditActionType::OAUTH_TOKEN_REJECTED,
                "oauth.token_rejected",
            ),
            (
                AuditActionType::OAUTH_REFRESH_ROTATED,
                "oauth.refresh_rotated",
            ),
            (
                AuditActionType::OAUTH_REFRESH_REPLAY_DETECTED,
                "oauth.refresh_replay_detected",
            ),
            (
                AuditActionType::OAUTH_CLIENT_REGISTERED,
                "oauth.client_registered",
            ),
            (
                AuditActionType::OAUTH_CLIENT_FIRST_USE,
                "oauth.client_first_use",
            ),
            (
                AuditActionType::OAUTH_CLIENT_METADATA_REFRESHED,
                "oauth.client_metadata_refreshed",
            ),
            (
                AuditActionType::OAUTH_CLIENT_METADATA_CHANGED_MATERIALLY,
                "oauth.client_metadata_changed_materially",
            ),
            (
                AuditActionType::OAUTH_CLIENT_TRUSTED,
                "oauth.client_trusted",
            ),
            (
                AuditActionType::OAUTH_CLIENT_REVOKED,
                "oauth.client_revoked",
            ),
            (
                AuditActionType::OAUTH_CLIENT_REGISTRATION_RATE_LIMITED,
                "oauth.client_registration_rate_limited",
            ),
            (
                AuditActionType::OAUTH_CONFIG_AUDIENCE_HOSTS_CHANGED,
                "oauth.config_audience_hosts_changed",
            ),
            (
                AuditActionType::OAUTH_CIMD_PARSE_FAILED,
                "oauth.cimd_parse_failed",
            ),
            (AuditActionType::OAUTH_CONSENT_GRANT, "oauth.consent_grant"),
            (AuditActionType::OAUTH_CONSENT_DENY, "oauth.consent_deny"),
            (
                AuditActionType::OAUTH_CONSENT_REVOKE,
                "oauth.consent_revoke",
            ),
            (AuditActionType::OAUTH_RATE_LIMITED, "oauth.rate_limited"),
            (
                AuditActionType::MCP_OAUTH_AUTHENTICATE,
                "mcp.oauth_authenticate",
            ),
        ];
        for (action, name) in expected {
            assert_eq!(action.as_str(), *name);
            assert!(
                AuditActionType::variants().iter().any(|v| v == action),
                "{name} missing from variants()",
            );
        }
    }
}
