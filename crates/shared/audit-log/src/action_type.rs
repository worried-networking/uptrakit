use std::fmt;
use std::str::FromStr;

use crate::error::{AuditLogError, Result};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AuditActionType(String);

impl AuditActionType {
    pub const AUTH_LOGIN: &'static str = "auth.login";
    pub const AUTH_LOGOUT: &'static str = "auth.logout";
    pub const AUTH_API_TOKEN_AUTHENTICATE: &'static str = "auth.api_token.authenticate";
    pub const AUTH_JWT_AUTHENTICATE: &'static str = "auth.jwt.authenticate";
    pub const AUTH_SERVICE_AUTHENTICATE: &'static str = "auth.service.authenticate";
    pub const AUTH_TOKEN_REFRESH: &'static str = "auth.token_refresh";
    pub const AUTH_DEVICE_START: &'static str = "auth.device.start";
    pub const AUTH_DEVICE_POLL: &'static str = "auth.device.poll";
    pub const AUTH_OIDC_AUTHORIZE: &'static str = "auth.oidc.authorize";
    pub const AUTH_OIDC_CALLBACK: &'static str = "auth.oidc.callback";
    pub const AUTH_OIDC_EXCHANGE: &'static str = "auth.oidc.exchange";
    pub const AUTH_OIDC_LINK: &'static str = "auth.oidc.link";
    pub const API_TOKEN_CREATE: &'static str = "api_token.create";
    pub const API_TOKEN_REVOKE: &'static str = "api_token.revoke";
    pub const ENROLLMENT_TOKEN_CREATE: &'static str = "enrollment_token.create";
    pub const ENROLLMENT_TOKEN_REVOKE: &'static str = "enrollment_token.revoke";
    pub const USER_CREATE: &'static str = "user.create";
    pub const USER_UPDATE: &'static str = "user.update";
    pub const USER_DELETE: &'static str = "user.delete";
    pub const OIDC_PROVIDER_CREATE: &'static str = "oidc_provider.create";
    pub const OIDC_PROVIDER_UPDATE: &'static str = "oidc_provider.update";
    pub const OIDC_PROVIDER_DELETE: &'static str = "oidc_provider.delete";
    pub const PLUGIN_CONFIG_CREATE: &'static str = "plugin_config.create";
    pub const PLUGIN_CONFIG_UPDATE: &'static str = "plugin_config.update";
    pub const PLUGIN_CONFIG_DELETE: &'static str = "plugin_config.delete";
    pub const PLUGIN_TYPE_SETTINGS_UPDATE: &'static str = "plugin_type_settings.update";
    pub const NOTIFICATION_CHANNEL_CREATE: &'static str = "notification_channel.create";
    pub const NOTIFICATION_CHANNEL_UPDATE: &'static str = "notification_channel.update";
    pub const NOTIFICATION_CHANNEL_DELETE: &'static str = "notification_channel.delete";
    pub const NOTIFICATION_CHANNEL_TEST: &'static str = "notification_channel.test";
    pub const NOTIFICATION_RULE_CREATE: &'static str = "notification_rule.create";
    pub const NOTIFICATION_RULE_UPDATE: &'static str = "notification_rule.update";
    pub const NOTIFICATION_RULE_DELETE: &'static str = "notification_rule.delete";
    pub const NOTIFICATION_RULE_TEST: &'static str = "notification_rule.test";
    pub const NOTIFICATION_CALLBACK: &'static str = "notification.callback";
    pub const GLOBAL_SETTING_UPDATE: &'static str = "global_setting.update";
    pub const TENANT_SETTING_UPDATE: &'static str = "tenant_setting.update";
    pub const TENANT_DATA_RESET: &'static str = "tenant.data.reset";
    pub const SYSTEM_CA_ROTATE: &'static str = "system.ca.rotate";
    pub const SYSTEM_SERVER_CERTIFICATE_RENEW: &'static str = "system.server_certificate.renew";
    pub const SCHEDULED_TASK_UPDATE: &'static str = "scheduled_task.update";
    pub const SCHEDULED_TASK_TRIGGER: &'static str = "scheduled_task.trigger";
    pub const HOST_TAG_CREATE: &'static str = "host_tag.create";
    pub const HOST_TAG_UPDATE: &'static str = "host_tag.update";
    pub const HOST_TAG_DELETE: &'static str = "host_tag.delete";
    pub const HOST_TAG_ASSIGN: &'static str = "host_tag.assign";
    pub const HOST_UPDATE: &'static str = "host.update";
    pub const HOST_DEACTIVATE: &'static str = "host.deactivate";
    pub const HOST_DISCOVER: &'static str = "host.discover";
    pub const SERVICE_UPDATE: &'static str = "service.update";
    pub const SERVICE_APPROVE: &'static str = "service.approve";
    pub const SERVICE_REJECT: &'static str = "service.reject";
    pub const SERVICE_MERGE: &'static str = "service.merge";
    pub const SERVICE_FREEZE_ENABLE: &'static str = "service.freeze.enable";
    pub const SERVICE_FREEZE_DISABLE: &'static str = "service.freeze.disable";
    pub const SERVICE_DEACTIVATE: &'static str = "service.deactivate";
    pub const SERVICE_CONFIG_STORE: &'static str = "service_config.store";
    pub const SERVICE_CONFIG_DELETE: &'static str = "service_config.delete";
    pub const SERVICE_CONFIG_DELIVER: &'static str = "service_config.deliver";
    pub const SERVICE_CERTIFICATE_ISSUE: &'static str = "service.certificate.issue";
    pub const SERVICE_CERTIFICATE_RENEW: &'static str = "service.certificate.renew";
    pub const SERVICE_ENROLLMENT_COMPLETED: &'static str = "service.enrollment.completed";
    pub const SERVICE_CREDENTIALS_DELIVER: &'static str = "service.credentials.deliver";
    pub const SERVICE_WORKLOAD_CLAIM: &'static str = "service.workload.claim";
    pub const SERVICE_WORKLOAD_RELEASE: &'static str = "service.workload.release";
    pub const SURFACE_PROVIDER_REGISTER: &'static str = "surface_provider.register";
    pub const SURFACE_ACTION_INVOKE: &'static str = "surface_action.invoke";
    pub const SOFTWARE_IGNORE_CREATE: &'static str = "software.ignore.create";
    pub const SOFTWARE_IGNORE_DELETE: &'static str = "software.ignore.delete";
    pub const DISCOVERY_ALLOWLIST_CREATE: &'static str = "discovery_allowlist.create";
    pub const DISCOVERY_ALLOWLIST_DELETE: &'static str = "discovery_allowlist.delete";
    pub const SOFTWARE_ITEM_CREATE: &'static str = "software_item.create";
    pub const SOFTWARE_ITEM_UPDATE: &'static str = "software_item.update";
    pub const SOFTWARE_ITEM_DELETE: &'static str = "software_item.delete";
    pub const SOFTWARE_ITEM_APPROVE: &'static str = "software_item.approve";
    pub const SOFTWARE_ITEM_ASSIGN_HOSTS: &'static str = "software_item.assign_hosts";
    pub const SOFTWARE_ITEM_UNASSIGN_HOST: &'static str = "software_item.unassign_host";
    pub const SOFTWARE_ITEM_UPDATE_HOST_ASSIGNMENT: &'static str =
        "software_item.update_host_assignment";
    pub const SOFTWARE_ITEM_DELETE_PLUGIN_ASSIGNMENT: &'static str =
        "software_item.delete_plugin_assignment";
    pub const SOFTWARE_ITEM_MERGE: &'static str = "software_item.merge";
    pub const SOFTWARE_ITEM_BATCH: &'static str = "software_item.batch";
    pub const SOFTWARE_VERSION_CHECK_TRIGGERED: &'static str = "software.version_check.triggered";
    pub const SOFTWARE_VERSION_CHECK_COMPLETED: &'static str = "software.version_check.completed";
    pub const SOFTWARE_UPDATE_TRIGGERED: &'static str = "software.update.triggered";
    pub const SOFTWARE_BATCH_UPDATE_TRIGGERED: &'static str = "software.batch_update.triggered";
    pub const SOFTWARE_UPDATE_STARTED: &'static str = "software.update.started";
    pub const SOFTWARE_BATCH_UPDATE_STARTED: &'static str = "software.batch_update.started";
    pub const SOFTWARE_UPDATE_FINALIZED: &'static str = "software.update.finalized";
    pub const SOFTWARE_BATCH_UPDATE_FINALIZED: &'static str = "software.batch_update.finalized";
    pub const SOFTWARE_UPDATE_STDIN_ATTENTION: &'static str = "software.update.stdin_attention";
    pub const SOFTWARE_UPDATE_INTERACTIVE_CONTROL: &'static str =
        "software.update.interactive_control";
    pub const SOFTWARE_ITEM_ENRICH: &'static str = "software_item.enrich";
    pub const SYSTEM_SERVICE_UPDATE_GATE: &'static str = "system.service.update_gate";
    pub const SYSTEM_SERVICE_MACHINE_ID_VALIDATE: &'static str =
        "system.service.machine_id.validate";
    pub const SYSTEM_SERVICE_UPDATE_FREEZE_APPLY: &'static str =
        "system.service.update_freeze.apply";
    pub const SYSTEM_SCHEDULER_AUDIT_LOG_CLEANUP: &'static str =
        "system.scheduler.audit_log_cleanup";

    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        validate_action_type(&value)?;
        Ok(Self(value))
    }

    pub fn from_static(value: &'static str) -> Self {
        Self::new(value).expect("canonical action types must validate")
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_registered(value: &str) -> bool {
        const V1_ACTIONS: &[&str] = &[
            AuditActionType::AUTH_LOGIN,
            AuditActionType::AUTH_LOGOUT,
            AuditActionType::AUTH_API_TOKEN_AUTHENTICATE,
            AuditActionType::AUTH_JWT_AUTHENTICATE,
            AuditActionType::AUTH_SERVICE_AUTHENTICATE,
            AuditActionType::AUTH_TOKEN_REFRESH,
            AuditActionType::AUTH_DEVICE_START,
            AuditActionType::AUTH_DEVICE_POLL,
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
            AuditActionType::PLUGIN_TYPE_SETTINGS_UPDATE,
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
            AuditActionType::SERVICE_FREEZE_ENABLE,
            AuditActionType::SERVICE_FREEZE_DISABLE,
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
        ];

        V1_ACTIONS.contains(&value)
    }
}

impl fmt::Display for AuditActionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&'static str> for AuditActionType {
    fn from(value: &'static str) -> Self {
        Self::from_static(value)
    }
}

impl FromStr for AuditActionType {
    type Err = rootcause::Report<AuditLogError>;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Self::new(s.to_string())
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
    use super::AuditActionType;

    #[test]
    fn audit_action_type_rejects_result_encoded_names() {
        assert!(AuditActionType::new("auth.login").is_ok());
        assert!(AuditActionType::new("auth.login.failed").is_err());
    }

    #[test]
    fn audit_action_type_rejects_validation_failed_suffix() {
        assert!(AuditActionType::new("service.merge.validation_failed").is_err());
    }

    #[test]
    fn audit_action_type_accepts_system_update_freeze_apply() {
        assert!(AuditActionType::new("system.service.update_freeze.apply").is_ok());
    }

    #[test]
    fn audit_action_type_registry_includes_surface_provider_register() {
        assert!(AuditActionType::new(AuditActionType::SURFACE_PROVIDER_REGISTER).is_ok());
        assert!(AuditActionType::is_registered(
            AuditActionType::SURFACE_PROVIDER_REGISTER
        ));
    }

    #[test]
    fn audit_action_type_registry_includes_auth_token_refresh() {
        assert!(AuditActionType::is_registered(
            AuditActionType::AUTH_TOKEN_REFRESH
        ));
    }

    #[test]
    fn audit_action_type_registry_includes_new_auth_actions() {
        assert!(AuditActionType::is_registered(AuditActionType::AUTH_LOGOUT));
        assert!(AuditActionType::is_registered(
            AuditActionType::AUTH_DEVICE_START
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::AUTH_DEVICE_POLL
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::AUTH_OIDC_EXCHANGE
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::AUTH_OIDC_LINK
        ));
    }

    #[test]
    fn audit_action_type_registry_includes_software_item_actions() {
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_ITEM_CREATE
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_ITEM_UPDATE
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_ITEM_DELETE
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_ITEM_APPROVE
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_ITEM_ASSIGN_HOSTS
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_ITEM_UNASSIGN_HOST
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_ITEM_UPDATE_HOST_ASSIGNMENT
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_ITEM_DELETE_PLUGIN_ASSIGNMENT
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_ITEM_MERGE
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_ITEM_BATCH
        ));
    }

    #[test]
    fn audit_action_type_registry_includes_service_config_actions() {
        assert!(AuditActionType::is_registered(
            AuditActionType::SERVICE_CONFIG_STORE
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SERVICE_CONFIG_DELETE
        ));
    }

    #[test]
    fn audit_action_type_registry_includes_software_lifecycle_actions() {
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_VERSION_CHECK_TRIGGERED
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_VERSION_CHECK_COMPLETED
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_UPDATE_FINALIZED
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_BATCH_UPDATE_FINALIZED
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_UPDATE_STDIN_ATTENTION
        ));
        assert!(AuditActionType::is_registered(
            AuditActionType::SOFTWARE_ITEM_ENRICH
        ));
    }
}
