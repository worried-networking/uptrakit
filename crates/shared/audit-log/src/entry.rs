use std::fmt;

use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::action_type::{AuditActionType, RegisteredAuditAction};
use crate::error::{AuditLogError, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditActorType {
    User,
    ApiToken,
    Oidc,
    Service,
    System,
}

impl AuditActorType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::ApiToken => "api_token",
            Self::Oidc => "oidc",
            Self::Service => "service",
            Self::System => "system",
        }
    }
}

impl fmt::Display for AuditActorType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditOutcome {
    Success,
    Denied,
    ValidationFailed,
    Failed,
    Partial,
}

impl AuditOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Denied => "denied",
            Self::ValidationFailed => "validation_failed",
            Self::Failed => "failed",
            Self::Partial => "partial",
        }
    }
}

impl fmt::Display for AuditOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for AuditOutcome {
    type Error = ();

    fn try_from(value: &str) -> std::result::Result<Self, Self::Error> {
        match value {
            "success" => Ok(Self::Success),
            "denied" => Ok(Self::Denied),
            "validation_failed" => Ok(Self::ValidationFailed),
            "failed" => Ok(Self::Failed),
            "partial" => Ok(Self::Partial),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct AuditEntry {
    pub id: Uuid,
    pub tenant_id: Option<Uuid>,
    pub occurred_at: OffsetDateTime,
    pub actor_type: AuditActorType,
    pub actor_id: Option<Uuid>,
    pub actor_display: Option<String>,
    pub action_type: AuditActionType,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub target_display: Option<String>,
    pub outcome: AuditOutcome,
    pub details_json: Option<Value>,
    pub request_id: Option<String>,
}

pub struct AuditEntryBuilder {
    entry: AuditEntry,
}

impl AuditEntry {
    pub fn builder(action_type: RegisteredAuditAction) -> AuditEntryBuilder {
        Self::builder_dynamic(action_type.into())
    }

    pub fn builder_dynamic(action_type: AuditActionType) -> AuditEntryBuilder {
        AuditEntryBuilder {
            entry: AuditEntry {
                id: Uuid::now_v7(),
                tenant_id: None,
                occurred_at: OffsetDateTime::now_utc(),
                actor_type: AuditActorType::System,
                actor_id: None,
                actor_display: None,
                action_type,
                target_type: None,
                target_id: None,
                target_display: None,
                outcome: AuditOutcome::Success,
                details_json: None,
                request_id: None,
            },
        }
    }

    pub fn test_stub(action_type: &str) -> Self {
        Self::builder_dynamic(
            AuditActionType::parse_wire(action_type.to_string()).expect("valid action type"),
        )
        .build()
        .expect("valid test audit entry")
    }

    pub fn validate(&self) -> Result<()> {
        const MAX_ACTION_TYPE_BYTES: usize = 128;
        const MAX_DETAILS_JSON_BYTES: usize = 4096;
        const MAX_ACTOR_DISPLAY_BYTES: usize = 255;
        const MAX_TARGET_TYPE_BYTES: usize = 128;
        const MAX_TARGET_DISPLAY_BYTES: usize = 255;
        const MAX_TARGET_ID_BYTES: usize = 255;
        const MAX_REQUEST_ID_BYTES: usize = 255;

        if self.occurred_at.offset() != time::UtcOffset::UTC {
            return Err(rootcause::report!(AuditLogError::Validation(
                "timestamps must be UTC".to_string()
            )));
        }
        if matches!(self.actor_type, AuditActorType::System) && self.actor_id.is_some() {
            return Err(rootcause::report!(AuditLogError::Validation(
                "system actors must not include actor_id".to_string()
            )));
        }
        if self.action_type.as_str().len() > MAX_ACTION_TYPE_BYTES {
            return Err(rootcause::report!(AuditLogError::Validation(
                "action_type exceeds 128 bytes".to_string()
            )));
        }
        if self.target_id.is_some() && self.target_type.is_none() {
            return Err(rootcause::report!(AuditLogError::Validation(
                "target_id requires target_type".to_string()
            )));
        }
        if self.target_display.is_some() && self.target_type.is_none() {
            return Err(rootcause::report!(AuditLogError::Validation(
                "target_display requires target_type".to_string()
            )));
        }
        if self
            .actor_display
            .as_ref()
            .is_some_and(|s| s.len() > MAX_ACTOR_DISPLAY_BYTES)
        {
            return Err(rootcause::report!(AuditLogError::Validation(
                "actor_display exceeds 255 bytes".to_string()
            )));
        }
        if self
            .target_type
            .as_ref()
            .is_some_and(|s| s.len() > MAX_TARGET_TYPE_BYTES)
        {
            return Err(rootcause::report!(AuditLogError::Validation(
                "target_type exceeds 128 bytes".to_string()
            )));
        }
        if self
            .target_display
            .as_ref()
            .is_some_and(|s| s.len() > MAX_TARGET_DISPLAY_BYTES)
        {
            return Err(rootcause::report!(AuditLogError::Validation(
                "target_display exceeds 255 bytes".to_string()
            )));
        }
        if self
            .target_id
            .as_ref()
            .is_some_and(|s| s.len() > MAX_TARGET_ID_BYTES)
        {
            return Err(rootcause::report!(AuditLogError::Validation(
                "target_id exceeds 255 bytes".to_string()
            )));
        }
        if self
            .request_id
            .as_ref()
            .is_some_and(|s| s.len() > MAX_REQUEST_ID_BYTES)
        {
            return Err(rootcause::report!(AuditLogError::Validation(
                "request_id exceeds 255 bytes".to_string()
            )));
        }
        if let Some(details) = &self.details_json {
            let serialized = serde_json::to_vec(details)
                .map_err(|err| rootcause::report!(AuditLogError::Serialization(err)))?;
            if serialized.len() > MAX_DETAILS_JSON_BYTES {
                return Err(rootcause::report!(AuditLogError::Validation(
                    "details_json exceeds 4096 bytes".to_string()
                )));
            }
        }

        Ok(())
    }
}

impl AuditEntryBuilder {
    pub fn tenant_scope(mut self, tenant_id: Uuid) -> Self {
        self.entry.tenant_id = Some(tenant_id);
        self
    }

    pub fn system_scope(mut self) -> Self {
        self.entry.tenant_id = None;
        self
    }

    pub fn actor(mut self, actor_type: AuditActorType, actor_id: Option<Uuid>) -> Self {
        self.entry.actor_type = actor_type;
        self.entry.actor_id = actor_id;
        self
    }

    pub fn actor_service(self, actor_id: Uuid) -> Self {
        self.actor(AuditActorType::Service, Some(actor_id))
    }

    pub fn actor_system(self) -> Self {
        self.actor(AuditActorType::System, None)
    }

    pub fn actor_display_opt(mut self, actor_display: Option<String>) -> Self {
        self.entry.actor_display = actor_display;
        self
    }

    pub fn target(
        mut self,
        target_type: &str,
        target_id: String,
        target_display: Option<String>,
    ) -> Self {
        self.entry.target_type = Some(target_type.to_string());
        self.entry.target_id = Some(target_id);
        self.entry.target_display = target_display;
        self
    }

    pub fn target_opt(
        mut self,
        target_type: Option<String>,
        target_id: Option<String>,
        target_display: Option<String>,
    ) -> Self {
        self.entry.target_type = target_type;
        self.entry.target_id = target_id;
        self.entry.target_display = target_display;
        self
    }

    pub fn outcome(mut self, outcome: AuditOutcome) -> Self {
        self.entry.outcome = outcome;
        self
    }

    pub fn details(mut self, details_json: Value) -> Self {
        self.entry.details_json = Some(details_json);
        self
    }

    pub fn request_id_opt(mut self, request_id: Option<String>) -> Self {
        self.entry.request_id = request_id;
        self
    }

    pub fn build(self) -> Result<AuditEntry> {
        self.entry.validate()?;
        Ok(self.entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actor_type_as_str_round_trip() {
        assert_eq!(AuditActorType::User.as_str(), "user");
        assert_eq!(AuditActorType::ApiToken.as_str(), "api_token");
        assert_eq!(AuditActorType::Oidc.as_str(), "oidc");
    }

    #[test]
    fn actor_type_display() {
        assert_eq!(AuditActorType::User.to_string(), "user");
        assert_eq!(AuditActorType::ApiToken.to_string(), "api_token");
        assert_eq!(AuditActorType::Oidc.to_string(), "oidc");
    }

    #[test]
    fn audit_actor_type_includes_service_and_system() {
        assert_eq!(AuditActorType::Service.as_str(), "service");
        assert_eq!(AuditActorType::System.as_str(), "system");
    }

    #[test]
    fn audit_entry_rejects_oversized_details_payload() {
        let mut entry = AuditEntry::test_stub("plugin_config.create");
        entry.details_json = Some(serde_json::json!({ "blob": "x".repeat(5000) }));
        assert!(entry.validate().is_err());
    }

    #[test]
    fn audit_entry_requires_utc_timestamp() {
        let mut entry = AuditEntry::test_stub("service.merge");
        entry.occurred_at = entry
            .occurred_at
            .to_offset(time::UtcOffset::from_hms(1, 0, 0).unwrap());
        assert!(entry.validate().is_err());
    }

    #[test]
    fn audit_entry_allows_missing_actor_id_for_denied_pre_auth_event() {
        let entry = AuditEntry::builder(AuditActionType::AUTH_LOGIN)
            .actor(AuditActorType::User, None)
            .actor_display_opt(Some("missing@example.com".to_string()))
            .outcome(AuditOutcome::Denied)
            .build();
        assert!(entry.is_ok());
    }

    #[test]
    fn audit_entry_rejects_target_id_without_target_type() {
        let entry = AuditEntry::builder(AuditActionType::SERVICE_MERGE)
            .actor_system()
            .target_opt(None, Some("svc-123".to_string()), None)
            .build();
        assert!(entry.is_err());
    }
}
