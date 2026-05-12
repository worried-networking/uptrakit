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
    pub const USER_DELETE: RegisteredAuditAction =
        RegisteredAuditAction::new("user.delete", AuditActionKind::Stateful);
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
    pub const SERVICE_UPDATE_FREEZE_ENABLE: RegisteredAuditAction =
        RegisteredAuditAction::new("service.update_freeze.enable", AuditActionKind::Stateful);
    pub const SERVICE_UPDATE_FREEZE_DISABLE: RegisteredAuditAction =
        RegisteredAuditAction::new("service.update_freeze.disable", AuditActionKind::Stateful);
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
        RegisteredAuditAction::new(
            "system.service.update_freeze.apply",
            AuditActionKind::Event,
        );
    pub const SYSTEM_SCHEDULER_AUDIT_LOG_CLEANUP: RegisteredAuditAction =
        RegisteredAuditAction::new(
            "system.scheduler.audit_log_cleanup",
            AuditActionKind::Event,
        );
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
    pub const OAUTH_CLIENT_METADATA_CHANGED_MATERIALLY: RegisteredAuditAction = RegisteredAuditAction::new(
        "oauth.client_metadata_changed_materially",
        AuditActionKind::Event,
    );
    pub const OAUTH_CLIENT_TRUSTED: RegisteredAuditAction =
        RegisteredAuditAction::new("oauth.client_trusted", AuditActionKind::Event);
    pub const OAUTH_CLIENT_REVOKED: RegisteredAuditAction =
        RegisteredAuditAction::new("oauth.client_revoked", AuditActionKind::Event);
    pub const OAUTH_CLIENT_REGISTRATION_RATE_LIMITED: RegisteredAuditAction =
        RegisteredAuditAction::new("oauth.client_registration_rate_limited", AuditActionKind::Event);
    pub const OAUTH_CONFIG_AUDIENCE_HOSTS_CHANGED: RegisteredAuditAction =
        RegisteredAuditAction::new("oauth.config_audience_hosts_changed", AuditActionKind::Event);
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

    pub fn new(value: impl Into<String>) -> Result<Self> {
        Self::parse_any(value)
    }

    pub const fn from_static(value: RegisteredAuditAction) -> RegisteredAuditAction {
        value
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_registered(value: &str) -> bool {
        const V1_ACTIONS: &[RegisteredAuditAction] = &[
            AuditActionType::AUTH_LOGIN,
            AuditActionType::AUTH_LOGOUT,
            AuditActionType::AUTH_API_TOKEN_AUTHENTICATE,
            AuditActionType::AUTH_JWT_AUTHENTICATE,
            AuditActionType::AUTH_SERVICE_AUTHENTICATE,
            AuditActionType::AUTH_TOKEN_REFRESH,
            AuditActionType::AUTH_DEVICE_START,
            AuditActionType::AUTH_DEVICE_POLL,
            AuditActionType::AUTH_DEVICE_APPROVE,
            AuditActionType::AUTH_DEVICE_DENY,
            AuditActionType::AUTH_OIDC_AUTHORIZE,
            AuditActionType::AUTH_OIDC_CALLBACK,
            AuditActionType::AUTH_OIDC_EXCHANGE,
            AuditActionType::AUTH_OIDC_LINK,
            AuditActionType::API_TOKEN_CREATE,
            AuditActionType::API_TOKEN_REVOKE,
            AuditActionType::ENROLLMENT_TOKEN_CREATE,
            AuditActionType::ENROLLMENT_TOKEN_REVOKE,
            AuditActionType::USER_CREATE,
            AuditActionType::USER_UPDATE,
            AuditActionType::USER_DELETE,
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

        V1_ACTIONS.iter().any(|action| action.as_str() == value)
    }
}

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

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — assert!(result.is_ok/is_err()) is idiomatic in tests"
    )]

    use super::{AuditActionKind, AuditActionType};
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
}
