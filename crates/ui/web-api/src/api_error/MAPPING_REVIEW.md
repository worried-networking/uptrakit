# API Error Mapping Review

This document is the human-readable companion to `mappings.rs`. It records the
canonical domain-error-to-HTTP mapping for every variant in every `From` impl,
including intentional pre-migration status-code deltas.

> **Maintained automatically** — the `tests.rs` golden-file tests assert that
> this document stays consistent with the code.

## Conventions

| Column | Meaning |
| --- | --- |
| Variant | `ErrorType::Variant` form |
| Status | HTTP status code |
| Message Strategy | `static` = fixed literal; `dynamic_display` = uses `ctx.to_string()` |
| Code | Machine-readable `code` field value |
| Safety Rationale | Why dynamic display is safe (dynamic variants only) |
| Pre-migration Delta | Status change vs. the old handler (if any) |

---

## ServiceQueryError

| Variant | Status | Message Strategy | Code |
| --- | --- | --- | --- |
| `ServiceQueryError::NotFound` | 404 | static | `service.not_found` |
| `ServiceQueryError::NotPending` | 400 | static | `service.not_pending` |
| `ServiceQueryError::NotApproved` | 400 | static | `service.not_approved` |
| `ServiceQueryError::NotMergeable` | 400 | static | `service.not_mergeable` |
| `ServiceQueryError::TargetConnected` | 409 | static | `service.target_connected` |
| `ServiceQueryError::SourceNotFound` | 404 | static | `service.source_not_found` |
| `ServiceQueryError::EmbeddedService` | 400 | static | `service.embedded_service` |
| `ServiceQueryError::Db` | 500 | static | `service.database_error` |

---

## SystemServiceQueryError

| Variant | Status | Message Strategy | Code |
| --- | --- | --- | --- |
| `SystemServiceQueryError::NotFound` | 404 | static | `system_service.not_found` |
| `SystemServiceQueryError::NotPending` | 400 | static | `system_service.not_pending` |
| `SystemServiceQueryError::NotApproved` | 400 | static | `system_service.not_approved` |
| `SystemServiceQueryError::EmbeddedService` | 400 | static | `system_service.embedded_service` |
| `SystemServiceQueryError::Db` | 500 | static | `system_service.database_error` |

---

## PluginConfigError

| Variant | Status | Message Strategy | Code | Safety Rationale | Pre-migration Delta |
| --- | --- | --- | --- | --- | --- |
| `PluginConfigError::NotFound` | 404 | static | `plugin_config.not_found` | — | — |
| `PluginConfigError::EmptyName` | 400 | static | `plugin_config.empty_name` | — | — |
| `PluginConfigError::DuplicateName` | 409 | static | `plugin_config.duplicate_name` | — | — |
| `PluginConfigError::ConfigValidation` | 400 | dynamic_display | `plugin_config.config_validation` | Contains human-readable plugin schema validation message; no secrets | **500→400** — was falling through `_ =>` catch-all |
| `PluginConfigError::Db` | 500 | static | `plugin_config.internal_error` | — | — |
| `PluginConfigError::Internal` | 500 | static | `plugin_config.internal_error` | — | — |

---

## ChannelQueryError

| Variant | Status | Message Strategy | Code | Safety Rationale |
| --- | --- | --- | --- | --- |
| `ChannelQueryError::UnsupportedType` | 400 | dynamic_display | `notification_channel.unsupported_type` | Contains type string from request payload — caller-controlled |
| `ChannelQueryError::InvalidConfig` | 400 | dynamic_display | `notification_channel.invalid_config` | Contains plugin config validation message — no secrets |
| `ChannelQueryError::Db` | 500 | static | `notification_channel.database_error` | — |

---

## RuleQueryError

| Variant | Status | Message Strategy | Code | Safety Rationale |
| --- | --- | --- | --- | --- |
| `RuleQueryError::ChannelNotFound` | 404 | static | `notification_rule.channel_not_found` | — |
| `RuleQueryError::InvalidField` | 400 | dynamic_display | `notification_rule.invalid_field` | Names the request field that failed validation — caller-controlled |
| `RuleQueryError::Db` | 500 | static | `notification_rule.database_error` | — |

---

## AllowlistError

| Variant | Status | Message Strategy | Code |
| --- | --- | --- | --- |
| `AllowlistError::InvalidPluginType` | 400 | static | `allowlist.invalid_plugin_type` |
| `AllowlistError::Db` | 500 | static | `allowlist.database_error` |

---

## ScheduledTaskError

| Variant | Status | Message Strategy | Code |
| --- | --- | --- | --- |
| `ScheduledTaskError::NotFound` | 404 | static | `scheduled_task.not_found` |
| `ScheduledTaskError::InvalidInterval` | 400 | static | `scheduled_task.invalid_interval` |
| `ScheduledTaskError::Db` | 500 | static | `scheduled_task.database_error` |

---

## SoftwareItemQueryError

| Variant | Status | Message Strategy | Code | Safety Rationale |
| --- | --- | --- | --- | --- |
| `SoftwareItemQueryError::NotFound` | 404 | static | `software_item.not_found` | — |
| `SoftwareItemQueryError::PluginAssignmentNotFound` | 404 | static | `software_item.plugin_assignment_not_found` | — |
| `SoftwareItemQueryError::EmptyName` | 400 | static | `software_item.empty_name` | — |
| `SoftwareItemQueryError::HostNotFound` | 400 | static | `software_item.host_not_found` | Uuid payload not surfaced — generic static message |
| `SoftwareItemQueryError::PluginConfigNotFound` | 400 | static | `software_item.plugin_config_not_found` | — |
| `SoftwareItemQueryError::IncompatibleHost` | 400 | dynamic_display | `software_item.incompatible_host` | Human-readable compatibility reason from plugin checks; no secrets |
| `SoftwareItemQueryError::InvalidPackageIdentifier` | 400 | dynamic_display | `software_item.invalid_package_identifier` | Package ID string from request — caller-controlled |
| `SoftwareItemQueryError::InvalidConfigOverride` | 400 | dynamic_display | `software_item.invalid_config_override` | Config schema validation message — no secrets |
| `SoftwareItemQueryError::InvalidInlinePluginConfig` | 400 | dynamic_display | `software_item.invalid_inline_plugin_config` | Plugin config schema validation message — no secrets |
| `SoftwareItemQueryError::InvalidExecutionSite` | 400 | dynamic_display | `software_item.invalid_execution_site` | Execution site string from request — caller-controlled |
| `SoftwareItemQueryError::DuplicateItem` | 409 | static | `software_item.duplicate_item` | — |
| `SoftwareItemQueryError::DuplicateHostAssignment` | 409 | static | `software_item.duplicate_host_assignment` | — |
| `SoftwareItemQueryError::Db` | 500 | static | `software_item.database_error` | — |

---

## TriggerUpdateError

| Variant | Status | Message Strategy | Code | Pre-migration Delta |
| --- | --- | --- | --- | --- |
| `TriggerUpdateError::SoftwareItemNotFound` | 404 | static | `trigger_update.software_item_not_found` | — |
| `TriggerUpdateError::HostNotFound` | 404 | static | `trigger_update.host_not_found` | — |
| `TriggerUpdateError::HostNotAssigned` | 400 | static | `trigger_update.host_not_assigned` | — |
| `TriggerUpdateError::NoExecuteUpdatePlugin` | 400 | static | `trigger_update.no_execute_update_plugin` | — |
| `TriggerUpdateError::NoAgent` | 400 | static | `trigger_update.no_agent` | **404→400** — host exists but lacks an agent connection; precondition failure |
| `TriggerUpdateError::AgentNotApproved` | 400 | static | `trigger_update.agent_not_approved` | — |
| `TriggerUpdateError::UpdateAlreadyActive` | 409 | static | `trigger_update.update_already_active` | — |
| `TriggerUpdateError::PluginConfigNotFound` | 400 | static | `trigger_update.plugin_config_not_found` | — |
| `TriggerUpdateError::UnknownPluginType` | 400 | dynamic_display | `trigger_update.unknown_plugin_type` | Plugin type string from request — caller-controlled |
| `TriggerUpdateError::Database` | 500 | static | `trigger_update.database_error` | — |

---

## AuditLogQueryError

| Variant | Status | Message Strategy | Code | Safety Rationale |
| --- | --- | --- | --- | --- |
| `AuditLogQueryError::InvalidFilter` | 400 | dynamic_display | `audit_log.invalid_filter` | Filter validation message about the request parameter — safe to echo |
| `AuditLogQueryError::Database` | 500 | static | `audit_log.database_error` | — |

---

## DeviceFlowError

| Variant | Status | Message Strategy | Code |
| --- | --- | --- | --- |
| `DeviceFlowError::NotFound` | 404 | static | `device_flow.not_found` |
| `DeviceFlowError::AlreadyAuthorized` | 409 | static | `device_flow.already_authorized` |
| `DeviceFlowError::TokenGeneration` | 500 | static | `device_flow.token_generation_error` |
| `DeviceFlowError::Database` | 500 | static | `device_flow.database_error` |

---

## RegistrationValidationError

> Bare `From` impl (not `From<Report<RegistrationValidationError>>`).

| Variant | Status | Message Strategy | Code |
| --- | --- | --- | --- |
| `RegistrationValidationError::Closed` | 403 | static | `registration.closed` |
| `RegistrationValidationError::TokenRequired` | 403 | static | `registration.token_required` |
| `RegistrationValidationError::NoTokenConfigured` | 403 | static | `registration.no_token_configured` |
| `RegistrationValidationError::InvalidToken` | 403 | static | `registration.invalid_token` |
| `RegistrationValidationError::VerificationFailed` | 500 | static | `registration.verification_failed` |

---

## Proactive Impls

The following impls are included proactively. No route handler migration is
needed — they are available for future use.

### PluginTypeSettingsError

| Variant | Status | Message Strategy | Code |
| --- | --- | --- | --- |
| `PluginTypeSettingsError::NotFound` | 404 | static | `plugin_type_settings.not_found` |
| `PluginTypeSettingsError::Db` | 500 | static | `plugin_type_settings.database_error` |

### AutodiscoveryError

| Variant | Status | Message Strategy | Code |
| --- | --- | --- | --- |
| `AutodiscoveryError::Db` | 500 | static | `autodiscovery.database_error` |

### ResetDataQueryError

| Variant | Status | Message Strategy | Code |
| --- | --- | --- | --- |
| `ResetDataQueryError::Database` | 500 | static | `reset_data.database_error` |

### SystemEnrollmentTokenError

| Variant | Status | Message Strategy | Code |
| --- | --- | --- | --- |
| `SystemEnrollmentTokenError::Database` | 500 | static | `system_enrollment_token.database_error` |

### AuthError

| Variant | Status | Message Strategy | Code |
| --- | --- | --- | --- |
| `AuthError::InvalidCredentials` | 401 | static | `auth.invalid_credentials` |
| `AuthError::SessionExpired` | 401 | static | `auth.session_expired` |
| `AuthError::UserNotFound` | 401 | static | `auth.invalid_credentials` (prevents user enumeration) |
| `AuthError::UserDeactivated` | 401 | static | `auth.user_deactivated` |
| `AuthError::PasswordHash` | 500 | static | `auth.internal` |
| `AuthError::TokenGeneration` | 500 | static | `auth.internal` |
| `AuthError::Database` | 500 | static | `auth.internal` |
| `AuthError::UuidParse` | 500 | static | `auth.internal` |
| `AuthError::TimeError` | 500 | static | `auth.internal` |
| `AuthError::OidcProviderNotFound` | 400 | static | `auth.oidc_provider_not_found` |
| `AuthError::OidcDiscovery` | 500 | static | `auth.internal` |
| `AuthError::OidcTokenExchange` | 500 | static | `auth.internal` |
| `AuthError::OidcTokenValidation` | 400 | static | `auth.oidc_token_validation_failed` |
| `AuthError::OidcStateNotFound` | 400 | static | `auth.oidc_state_not_found` |
| `AuthError::OidcNoAccount` | 403 | static | `auth.oidc_no_account` |
| `AuthError::OidcLinkRequired` | 403 | static | `auth.oidc_link_required` |
| `AuthError::OidcLinkVerificationFailed` | 400 | static | `auth.oidc_link_verification_failed` |
| `AuthError::PasswordAuthDisabled` | 400 | static | `auth.password_auth_disabled` |
| `AuthError::CannotDisableOwnAuthMethod` | 409 | static | `auth.cannot_disable_own_auth_method` |
| `AuthError::NoAuthMethodsRemaining` | 409 | static | `auth.no_auth_methods_remaining` |
| `AuthError::JwtEncode` | 500 | static | `auth.internal` |
| `AuthError::JwtDecode` | 401 | static | `auth.jwt_decode_failed` |
| `AuthError::InvalidRefreshToken` | 401 | static | `auth.invalid_refresh_token` |
| `AuthError::RefreshTokenExpired` | 401 | static | `auth.refresh_token_expired` |
| `AuthError::RefreshTokenRevoked` | 401 | static | `auth.refresh_token_revoked` |
| `AuthError::ApiTokenNotFound` | 401 | static | `auth.api_token_not_found` |
| `AuthError::ApiTokenRevoked` | 401 | static | `auth.api_token_revoked` |
| `AuthError::DeviceFlowNotFound` | 404 | static | `auth.device_flow_not_found` |
| `AuthError::DeviceFlowAlreadyAuthorized` | 409 | static | `auth.device_flow_already_authorized` |
| `AuthError::Io` | 500 | static | `auth.internal` |
| `AuthError::InvalidSession` | 401 | static | `auth.invalid_session` |
| `AuthError::Internal` | 500 | static | `auth.internal` |

---

## Intentional Pre-migration Deltas

| Variant | Old Status | New Status | Rationale |
| --- | --- | --- | --- |
| `PluginConfigError::ConfigValidation` | 500 | 400 | Was falling through `_ =>` catch-all in `create_plugin_config`; validation failure is a client error |
| `TriggerUpdateError::NoAgent` | 404 | 400 | Host exists but lacks an agent connection — this is a precondition failure, not a missing resource |
