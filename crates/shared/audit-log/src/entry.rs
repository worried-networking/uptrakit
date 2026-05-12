use std::fmt;
use std::marker::PhantomData;

use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::action_type::AuditActionType;
use crate::error::{AuditLogError, Result};

/// Marker: this entry records a discrete event (no before/after snapshots).
#[derive(Clone, Debug)]
pub struct Event;

/// Marker: this entry records an entity-state mutation (requires before + after snapshots).
#[derive(Clone, Debug)]
pub struct Stateful;

/// Typestate marker: the `before_snapshot` field has not yet been supplied.
pub struct NeedsBefore;

/// Typestate marker: the `before_snapshot` field has been supplied.
pub struct HasBefore;

/// Typestate marker: the `after_snapshot` field has not yet been supplied.
pub struct NeedsAfter;

/// Typestate marker: the `after_snapshot` field has been supplied.
pub struct HasAfter;

/// Classifies the actor that triggered an audit-logged operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditActorType {
    User,
    ApiToken,
    Oidc,
    Service,
    System,
}

impl AuditActorType {
    /// Returns the canonical lowercase string representation.
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

/// The outcome of the audited operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditOutcome {
    Success,
    Denied,
    ValidationFailed,
    Failed,
    Partial,
}

impl AuditOutcome {
    /// Returns the canonical lowercase string representation.
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

/// A structured, validated audit log entry.
///
/// The type parameter `K` is a marker that distinguishes discrete event entries
/// ([`Event`]) from state-mutation entries ([`Stateful`]).  Use
/// [`AuditEntry::builder_event`] or [`AuditEntry::builder_stateful`] to
/// construct entries; the builder enforces that all required snapshots are
/// supplied at compile time before `.build()` is callable.
#[derive(Clone, Debug)]
pub struct AuditEntry<K> {
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
    pub before_snapshot: Option<Value>,
    pub after_snapshot: Option<Value>,
    pub correlation_id: Option<Uuid>,
    pub request_id: Option<String>,
    _kind: PhantomData<K>,
}

/// A builder for [`AuditEntry<Event>`] entries.
///
/// The two type parameters `B` and `A` are unused for event entries but are
/// kept in the signature for uniformity with the stateful builder.
pub struct AuditEntryBuilder<K, B = NeedsBefore, A = NeedsAfter> {
    entry: AuditEntry<K>,
    _state: PhantomData<(B, A)>,
}

// ---------------------------------------------------------------------------
// Constructors
// ---------------------------------------------------------------------------

impl AuditEntry<Event> {
    /// Creates a builder for a discrete-event audit entry.
    ///
    /// # Examples
    ///
    /// ```
    /// use uptrakit_audit_log::{AuditEntry, AuditActionType, Event};
    ///
    /// let entry = AuditEntry::<Event>::builder_event(AuditActionType::AUTH_LOGIN)
    ///     .build()
    ///     .expect("entry should be valid");
    ///
    /// assert_eq!(entry.action_type, AuditActionType::AUTH_LOGIN);
    /// ```
    #[must_use]
    pub fn builder_event(action: impl Into<AuditActionType>) -> AuditEntryBuilder<Event> {
        AuditEntryBuilder {
            entry: empty_entry(action.into()),
            _state: PhantomData,
        }
    }

    /// Compatibility alias for [`builder_event`](Self::builder_event).
    ///
    /// Prefer `builder_event` in new code. This alias exists so that existing
    /// call sites compile unchanged while the migration to the typestate API
    /// proceeds incrementally.
    #[must_use]
    pub fn builder(action: impl Into<AuditActionType>) -> AuditEntryBuilder<Event> {
        Self::builder_event(action)
    }
}

impl AuditEntry<Stateful> {
    /// Creates a builder for a state-mutation audit entry.
    ///
    /// The builder requires that both `.before(…)` and `.after(…)` are called
    /// before `.build()` becomes available.
    ///
    /// # Examples
    ///
    /// ```
    /// use uptrakit_audit_log::{AuditEntry, AuditActionType, AuditView, Stateful};
    /// use uuid::Uuid;
    ///
    /// struct Stub { id: Uuid, name: String }
    /// impl AuditView for Stub {
    ///     const TARGET_TYPE: &'static str = "stub";
    ///     fn audit_target_id(&self) -> String { self.id.to_string() }
    ///     fn audit_target_display(&self) -> Option<String> { Some(self.name.clone()) }
    ///     fn audit_view(&self) -> serde_json::Value { serde_json::json!({ "name": self.name }) }
    /// }
    ///
    /// let before = Stub { id: Uuid::now_v7(), name: "old".into() };
    /// let after  = Stub { id: before.id, name: "new".into() };
    ///
    /// let entry = AuditEntry::<Stateful>::builder_stateful(AuditActionType::USER_UPDATE)
    ///     .before(&before)
    ///     .after(&after)
    ///     .build()
    ///     .expect("entry should be valid");
    ///
    /// assert_eq!(entry.before_snapshot.as_ref().unwrap()["name"], "old");
    /// assert_eq!(entry.after_snapshot.as_ref().unwrap()["name"], "new");
    /// ```
    #[must_use]
    pub fn builder_stateful(
        action: impl Into<AuditActionType>,
    ) -> AuditEntryBuilder<Stateful, NeedsBefore, NeedsAfter> {
        AuditEntryBuilder {
            entry: empty_entry(action.into()),
            _state: PhantomData,
        }
    }
}

fn empty_entry<K>(action: AuditActionType) -> AuditEntry<K> {
    AuditEntry {
        id: Uuid::now_v7(),
        tenant_id: None,
        occurred_at: OffsetDateTime::now_utc(),
        actor_type: AuditActorType::System,
        actor_id: None,
        actor_display: None,
        action_type: action,
        target_type: None,
        target_id: None,
        target_display: None,
        outcome: AuditOutcome::Success,
        details_json: None,
        before_snapshot: None,
        after_snapshot: None,
        correlation_id: None,
        request_id: None,
        _kind: PhantomData,
    }
}

// ---------------------------------------------------------------------------
// Common builder methods (all states)
// ---------------------------------------------------------------------------

impl<K, B, A> AuditEntryBuilder<K, B, A> {
    /// Scopes this entry to a specific tenant.
    pub fn tenant_scope(mut self, tenant_id: Uuid) -> Self {
        self.entry.tenant_id = Some(tenant_id);
        self
    }

    /// Clears any tenant scope, making this a system-level entry.
    pub fn system_scope(mut self) -> Self {
        self.entry.tenant_id = None;
        self
    }

    /// Sets the actor type and optional actor ID.
    pub fn actor(mut self, actor_type: AuditActorType, actor_id: Option<Uuid>) -> Self {
        self.entry.actor_type = actor_type;
        self.entry.actor_id = actor_id;
        self
    }

    /// Sets the actor to a human user with a display name.
    pub fn actor_user(self, actor_id: Uuid, display: impl Into<String>) -> Self {
        let display = display.into();
        self.actor(AuditActorType::User, Some(actor_id))
            .actor_display_opt(Some(display))
    }

    /// Sets the actor to an authenticated service.
    pub fn actor_service(self, actor_id: Uuid) -> Self {
        self.actor(AuditActorType::Service, Some(actor_id))
    }

    /// Sets the actor to the system (no actor ID).
    pub fn actor_system(self) -> Self {
        self.actor(AuditActorType::System, None)
    }

    /// Sets the optional human-readable actor display name.
    pub fn actor_display_opt(mut self, display: Option<String>) -> Self {
        self.entry.actor_display = display;
        self
    }

    /// Sets the outcome of the audited operation.
    pub fn outcome(mut self, outcome: AuditOutcome) -> Self {
        self.entry.outcome = outcome;
        self
    }

    /// Attaches a structured JSON details payload.
    pub fn details(mut self, details: Value) -> Self {
        self.entry.details_json = Some(details);
        self
    }

    /// Sets the optional inbound HTTP request ID for correlation.
    pub fn request_id_opt(mut self, req: Option<String>) -> Self {
        self.entry.request_id = req;
        self
    }

    /// Sets the correlation ID for cross-entry linking.
    pub fn correlation_id(mut self, id: Uuid) -> Self {
        self.entry.correlation_id = Some(id);
        self
    }

    /// Sets the optional correlation ID.
    pub fn correlation_id_opt(mut self, id: Option<Uuid>) -> Self {
        self.entry.correlation_id = id;
        self
    }

    /// Sets target type, ID, and optional display name.
    pub fn target(mut self, target_type: &str, target_id: String, display: Option<String>) -> Self {
        self.entry.target_type = Some(target_type.to_string());
        self.entry.target_id = Some(target_id);
        self.entry.target_display = display;
        self
    }

    /// Sets target fields individually, accepting `Option`s for each.
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
}

// ---------------------------------------------------------------------------
// Snapshot truncation
// ---------------------------------------------------------------------------

const MAX_SNAPSHOT_BYTES: usize = 16 * 1024;
const TRUNCATED_PREVIEW_BYTES: usize = 256;

/// Truncates oversized fields in a snapshot `Value` until it fits within
/// [`MAX_SNAPSHOT_BYTES`].
///
/// Fields listed in `truncatable_keys` are replaced with a sentinel object
/// containing `truncated: true`, `byte_count`, and a UTF-8 best-effort
/// `preview`. Truncation stops once the overall serialised size is within
/// the limit.
///
/// # Errors
///
/// Returns an error only if `serde_json::to_vec` fails, which only occurs for
/// non-self-describing types — never in practice for audit snapshots.
fn measure_json(v: &Value) -> Result<usize> {
    let bytes = serde_json::to_vec(v)
        .map_err(|err| rootcause::report!(AuditLogError::Serialization(err)))?;
    Ok(bytes.len())
}

fn apply_truncation(mut value: Value, truncatable_keys: &[&str]) -> Result<Value> {
    if measure_json(&value)? <= MAX_SNAPSHOT_BYTES {
        return Ok(value);
    }

    // Only object values have named keys to truncate.
    if !value.is_object() {
        return Ok(value);
    }

    for key in truncatable_keys {
        // Extract the field (if present), replace it, then re-measure.
        let replaced = if let Value::Object(obj) = &mut value {
            if let Some(field) = obj.get_mut(*key) {
                let serialised = serde_json::to_vec(field).unwrap_or_default();
                let preview: String = serialised
                    .iter()
                    .take(TRUNCATED_PREVIEW_BYTES)
                    .map(|b| char::from(*b))
                    .collect();
                *field = serde_json::json!({
                    "truncated": true,
                    "byte_count": serialised.len(),
                    "preview": preview,
                });
                true
            } else {
                false
            }
        } else {
            false
        };
        // Drop the mutable borrow before measuring.
        if replaced && measure_json(&value)? <= MAX_SNAPSHOT_BYTES {
            return Ok(value);
        }
    }

    Ok(value)
}

// ---------------------------------------------------------------------------
// Snapshot setters — Stateful only
// ---------------------------------------------------------------------------

impl<A> AuditEntryBuilder<Stateful, NeedsBefore, A> {
    /// Captures the `before` snapshot from an [`AuditView`] implementation.
    ///
    /// Populates `target_type`, `target_id`, and `target_display` from the
    /// view if they are not already set.
    pub fn before<V: AuditView>(mut self, view: &V) -> AuditEntryBuilder<Stateful, HasBefore, A> {
        let snapshot = view.audit_view();
        // Truncation failures fall back to Value::Null; serde_json::to_vec never
        // fails for Serialize-bound types in practice.
        self.entry.before_snapshot =
            Some(apply_truncation(snapshot, V::audit_truncatable_fields()).unwrap_or(Value::Null));
        self.entry.target_type = Some(V::TARGET_TYPE.to_string());
        self.entry.target_id = Some(view.audit_target_id());
        if self.entry.target_display.is_none() {
            self.entry.target_display = view.audit_target_display();
        }
        AuditEntryBuilder {
            entry: self.entry,
            _state: PhantomData,
        }
    }
}

impl<B> AuditEntryBuilder<Stateful, B, NeedsAfter> {
    /// Captures the `after` snapshot from an [`AuditView`] implementation.
    pub fn after<V: AuditView>(mut self, view: &V) -> AuditEntryBuilder<Stateful, B, HasAfter> {
        let snapshot = view.audit_view();
        self.entry.after_snapshot =
            Some(apply_truncation(snapshot, V::audit_truncatable_fields()).unwrap_or(Value::Null));
        AuditEntryBuilder {
            entry: self.entry,
            _state: PhantomData,
        }
    }
}

// ---------------------------------------------------------------------------
// Terminal `.build()` — only on fully-satisfied states
// ---------------------------------------------------------------------------

impl AuditEntryBuilder<Event> {
    /// Validates and finalises the builder into an [`AuditEntry<Event>`].
    ///
    /// # Errors
    ///
    /// Returns [`AuditLogError::Validation`] if any field constraint is violated,
    /// or [`AuditLogError::Serialization`] if `details_json` cannot be measured.
    pub fn build(self) -> Result<AuditEntry<Event>> {
        validate(&self.entry)?;
        Ok(self.entry)
    }
}

impl AuditEntryBuilder<Stateful, HasBefore, HasAfter> {
    /// Validates and finalises the builder into an [`AuditEntry<Stateful>`].
    ///
    /// # Errors
    ///
    /// Returns [`AuditLogError::Validation`] if any field constraint is violated,
    /// or [`AuditLogError::Serialization`] if `details_json` cannot be measured.
    pub fn build(self) -> Result<AuditEntry<Stateful>> {
        validate(&self.entry)?;
        Ok(self.entry)
    }
}

// ---------------------------------------------------------------------------
// Shared validation
// ---------------------------------------------------------------------------

/// Validates all field constraints on a built entry.
///
/// # Errors
///
/// Returns [`AuditLogError::Validation`] when a constraint is violated, or
/// [`AuditLogError::Serialization`] when a JSON field cannot be measured.
pub fn validate<K>(e: &AuditEntry<K>) -> Result<()> {
    const MAX_ACTION_TYPE_BYTES: usize = 128;
    const MAX_DETAILS_JSON_BYTES: usize = 4 * 1024;
    const MAX_ACTOR_DISPLAY_BYTES: usize = 255;
    const MAX_TARGET_TYPE_BYTES: usize = 128;
    const MAX_TARGET_DISPLAY_BYTES: usize = 255;
    const MAX_TARGET_ID_BYTES: usize = 255;
    const MAX_REQUEST_ID_BYTES: usize = 255;
    const MAX_SNAPSHOT_BYTES_LOCAL: usize = 16 * 1024;

    if e.occurred_at.offset() != time::UtcOffset::UTC {
        return Err(rootcause::report!(AuditLogError::Validation(
            "timestamps must be UTC".into()
        )));
    }
    if matches!(e.actor_type, AuditActorType::System) && e.actor_id.is_some() {
        return Err(rootcause::report!(AuditLogError::Validation(
            "system actors must not include actor_id".into()
        )));
    }
    if e.action_type.as_str().len() > MAX_ACTION_TYPE_BYTES {
        return Err(rootcause::report!(AuditLogError::Validation(
            "action_type exceeds 128 bytes".into()
        )));
    }
    if e.target_id.is_some() && e.target_type.is_none() {
        return Err(rootcause::report!(AuditLogError::Validation(
            "target_id requires target_type".into()
        )));
    }
    if e.target_display.is_some() && e.target_type.is_none() {
        return Err(rootcause::report!(AuditLogError::Validation(
            "target_display requires target_type".into()
        )));
    }
    if e.actor_display
        .as_ref()
        .is_some_and(|s| s.len() > MAX_ACTOR_DISPLAY_BYTES)
    {
        return Err(rootcause::report!(AuditLogError::Validation(
            "actor_display exceeds 255 bytes".into()
        )));
    }
    if e.target_type
        .as_ref()
        .is_some_and(|s| s.len() > MAX_TARGET_TYPE_BYTES)
    {
        return Err(rootcause::report!(AuditLogError::Validation(
            "target_type exceeds 128 bytes".into()
        )));
    }
    if e.target_display
        .as_ref()
        .is_some_and(|s| s.len() > MAX_TARGET_DISPLAY_BYTES)
    {
        return Err(rootcause::report!(AuditLogError::Validation(
            "target_display exceeds 255 bytes".into()
        )));
    }
    if e.target_id
        .as_ref()
        .is_some_and(|s| s.len() > MAX_TARGET_ID_BYTES)
    {
        return Err(rootcause::report!(AuditLogError::Validation(
            "target_id exceeds 255 bytes".into()
        )));
    }
    if e.request_id
        .as_ref()
        .is_some_and(|s| s.len() > MAX_REQUEST_ID_BYTES)
    {
        return Err(rootcause::report!(AuditLogError::Validation(
            "request_id exceeds 255 bytes".into()
        )));
    }
    if let Some(details) = &e.details_json {
        let bytes = serde_json::to_vec(details)
            .map_err(|err| rootcause::report!(AuditLogError::Serialization(err)))?;
        if bytes.len() > MAX_DETAILS_JSON_BYTES {
            return Err(rootcause::report!(AuditLogError::Validation(
                "details_json exceeds 4096 bytes".into()
            )));
        }
    }
    if let Some(s) = &e.before_snapshot {
        let bytes = serde_json::to_vec(s)
            .map_err(|err| rootcause::report!(AuditLogError::Serialization(err)))?;
        if bytes.len() > MAX_SNAPSHOT_BYTES_LOCAL {
            return Err(rootcause::report!(AuditLogError::Validation(
                "before_snapshot exceeds 16 KB".into()
            )));
        }
    }
    if let Some(s) = &e.after_snapshot {
        let bytes = serde_json::to_vec(s)
            .map_err(|err| rootcause::report!(AuditLogError::Serialization(err)))?;
        if bytes.len() > MAX_SNAPSHOT_BYTES_LOCAL {
            return Err(rootcause::report!(AuditLogError::Validation(
                "after_snapshot exceeds 16 KB".into()
            )));
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// AuditView trait
// ---------------------------------------------------------------------------

/// Deterministic, secret-safe projection of a persisted entity into snapshot JSON.
///
/// Implementations are typically generated by the `#[derive(AuditView)]` proc-macro.
pub trait AuditView {
    /// The stable type discriminator stored in `target_type` on every audit entry.
    const TARGET_TYPE: &'static str;

    /// Returns the stable identifier for this entity (e.g. UUID as a string).
    fn audit_target_id(&self) -> String;

    /// Returns a human-readable label for the entity, if available.
    fn audit_target_display(&self) -> Option<String>;

    /// Produces the redacted, secret-safe JSON snapshot of this entity.
    fn audit_view(&self) -> serde_json::Value;

    /// JSON keys in `audit_view()` whose fields are tagged `#[audit(truncatable)]`.
    ///
    /// These are the first-pass strip targets when a snapshot exceeds the size cap.
    /// Default empty — fields that are not truncatable contribute nothing here.
    fn audit_truncatable_fields() -> &'static [&'static str] {
        &[]
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — assert!(result.is_ok/is_err()) is idiomatic in tests"
    )]

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
        let entry = AuditEntry::<Event>::builder_event(AuditActionType::PLUGIN_CONFIG_CREATE)
            .details(serde_json::json!({ "blob": "x".repeat(5000) }))
            .build();
        assert!(entry.is_err());
    }

    #[test]
    fn audit_entry_requires_utc_timestamp() {
        #[expect(
            clippy::expect_used,
            reason = "test helper — both parse and build are infallible for well-formed inputs; panic is acceptable in test setup"
        )]
        let mut entry = AuditEntry::<Event>::builder_event(AuditActionType::SERVICE_MERGE)
            .build()
            .expect("valid test audit entry");
        entry.occurred_at = entry
            .occurred_at
            .to_offset(time::UtcOffset::from_hms(1, 0, 0).unwrap());
        assert!(validate(&entry).is_err());
    }

    #[test]
    fn audit_entry_allows_missing_actor_id_for_denied_pre_auth_event() {
        let entry = AuditEntry::<Event>::builder_event(AuditActionType::AUTH_LOGIN)
            .actor(AuditActorType::User, None)
            .actor_display_opt(Some("missing@example.com".to_string()))
            .outcome(AuditOutcome::Denied)
            .build();
        assert!(entry.is_ok());
    }

    #[test]
    fn audit_entry_rejects_target_id_without_target_type() {
        let entry = AuditEntry::<Event>::builder_event(AuditActionType::SERVICE_MERGE)
            .actor_system()
            .target_opt(None, Some("svc-123".to_string()), None)
            .build();
        assert!(entry.is_err());
    }

    #[test]
    fn audit_entry_builder_accepts_dynamic_action_type_inputs() {
        let action_type = "auth.login"
            .parse::<AuditActionType>()
            .expect("registered action type");

        let entry = AuditEntry::<Event>::builder_event(action_type)
            .build()
            .expect("entry builds");

        assert_eq!(entry.action_type.as_str(), "auth.login");
    }

    #[test]
    fn audit_view_projects_struct_into_json() {
        use uuid::Uuid;

        struct Demo {
            id: Uuid,
            name: String,
            count: u32,
        }

        impl AuditView for Demo {
            const TARGET_TYPE: &'static str = "demo";

            fn audit_target_id(&self) -> String {
                self.id.to_string()
            }

            fn audit_target_display(&self) -> Option<String> {
                Some(self.name.clone())
            }

            fn audit_view(&self) -> serde_json::Value {
                serde_json::json!({ "name": self.name, "count": self.count })
            }
        }

        let id = Uuid::now_v7();
        let v = Demo {
            id,
            name: "alpha".into(),
            count: 3,
        };
        assert_eq!(v.audit_target_id(), id.to_string());
        assert_eq!(v.audit_target_display(), Some("alpha".into()));
        let proj = v.audit_view();
        assert_eq!(proj["name"], "alpha");
        assert_eq!(proj["count"], 3);
    }

    #[test]
    fn stateful_builder_captures_before_and_after_snapshots() {
        use uuid::Uuid;

        struct Thing {
            id: Uuid,
            name: String,
        }

        impl AuditView for Thing {
            const TARGET_TYPE: &'static str = "thing";

            fn audit_target_id(&self) -> String {
                self.id.to_string()
            }

            fn audit_target_display(&self) -> Option<String> {
                Some(self.name.clone())
            }

            fn audit_view(&self) -> serde_json::Value {
                serde_json::json!({ "name": self.name })
            }
        }

        let id = Uuid::now_v7();
        let before = Thing {
            id,
            name: "old".into(),
        };
        let after = Thing {
            id,
            name: "new".into(),
        };

        let entry = AuditEntry::<Stateful>::builder_stateful(AuditActionType::USER_UPDATE)
            .before(&before)
            .after(&after)
            .build()
            .expect("stateful entry should be valid");

        assert_eq!(entry.before_snapshot.as_ref().unwrap()["name"], "old");
        assert_eq!(entry.after_snapshot.as_ref().unwrap()["name"], "new");
        assert_eq!(entry.target_type.as_deref(), Some("thing"));
        assert_eq!(entry.target_id.as_deref(), Some(&*id.to_string()));
    }

    #[test]
    fn stateful_builder_sets_target_type_and_id_from_before_view() {
        use uuid::Uuid;

        struct Marker {
            id: Uuid,
        }

        impl AuditView for Marker {
            const TARGET_TYPE: &'static str = "marker";

            fn audit_target_id(&self) -> String {
                self.id.to_string()
            }

            fn audit_target_display(&self) -> Option<String> {
                None
            }

            fn audit_view(&self) -> serde_json::Value {
                serde_json::json!({})
            }
        }

        let id = Uuid::now_v7();
        let m = Marker { id };
        let entry = AuditEntry::<Stateful>::builder_stateful(AuditActionType::USER_UPDATE)
            .before(&m)
            .after(&m)
            .build()
            .expect("entry should build");

        assert_eq!(entry.target_type.as_deref(), Some("marker"));
        assert_eq!(entry.target_id.as_deref(), Some(&*id.to_string()));
    }

    #[test]
    fn event_builder_does_not_expose_snapshots_method() {
        // compile-time enforcement: AuditEntry<Event>::builder_event has no
        // .before() / .after() methods.  This test exercises the event path
        // and verifies snapshots remain None.
        let entry = AuditEntry::<Event>::builder_event(AuditActionType::AUTH_LOGIN)
            .actor_user(uuid::Uuid::now_v7(), "test@example.com")
            .build()
            .expect("event entry should build");

        assert!(entry.before_snapshot.is_none());
        assert!(entry.after_snapshot.is_none());
    }

    #[test]
    fn audit_actions_macro_generates_event_constructor() {
        let b = AuditEntry::auth_login();
        let entry = b.actor_system().build().expect("event builds");
        assert_eq!(entry.action_type.as_str(), "auth.login");
        assert!(entry.before_snapshot.is_none());
    }
}
