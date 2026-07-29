//! Engine-owned query module for `access_grants` — the single choke point for
//! grant writes (validation) and reads (fail-closed JSON typing).
//!
//! # Validation (every write, in order)
//!
//! 1. **Rule 1** — [`validate_patterns`]: count bound + per-pattern
//!    `can_match_any` against the live catalog.
//! 2. **Plane purity** — a grant's patterns are ALL system-plane or ALL
//!    tenant-plane, never mixed. Without this named rule, a NULL-tenant
//!    user-subject grant mixing `system.services:read` with `hosts:read`
//!    would load in EVERY tenant (`tenant_id = ? OR IS NULL`) — a
//!    cross-tenant leak. The predicate is segment-aware (dotted `system.`
//!    prefix, mirroring `Resource::is_system`), never a bare substring —
//!    a future tenant resource named `systems` must classify tenant-plane.
//! 3. **Rule 2 (single encoding per subject type)** — system-plane ⇒
//!    `tenant_id IS NULL` (any subject); role subject ⇒ NULL always;
//!    user-subject tenant-plane ⇒ non-NULL.
//! 4. **B9 phase gate** — any non-`All` selector is rejected until M2.1.
//! 5. Bounds — description length, selector ID bounds (inert while B9
//!    holds), `MAX_GRANTS_PER_SUBJECT` on insert.
//!
//! # Resolution safety argument (06 §Storage schema)
//!
//! [`load_grants_for_principal`] is ONE query (batch invariant, no N+1).
//! Role-subject rows are always `tenant_id NULL` by rule 2; tenant scoping
//! comes from the caller's `user_roles`-derived `role_ids`, so a role grant
//! is loaded exactly when the role is assigned in the active tenant. A
//! user-subject tenant-plane grant is never NULL-tenant, so it can only
//! match the active tenant.
//!
//! # Corrupt rows
//!
//! The batch read LOUD-SKIPS corrupt rows (unparseable `patterns`/`selector`
//! JSON): `tracing::error!` per row (id + subject, never the raw payload),
//! the row is dropped, and the call succeeds; [`GrantLoad::corrupt_skipped`]
//! counts the drops so M1.3's engine MUST emit an aggregate counter from it
//! (systemic corruption must not manifest only as individual denials). This
//! IS fail-closed — the model is allow-only union, so dropping an allow row
//! can only shrink authority — while a whole-call error would convert one
//! corrupt role-subject row into a mass lockout including `access:manage`
//! holders. INVARIANT TRIPWIRE: this skip is fail-closed ONLY while grants
//! are allow-only; if deny/exclusion semantics are ever introduced (none
//! planned — 08-rejected-alternatives rejects them), corrupt-row handling
//! must flip to call-fatal in the same change.
//!
//! Whole-call errors are reserved for the query itself failing. The
//! single-row [`load_grant`] surfaces corruption distinctly
//! ([`AccessGrantError::Corrupt`], never aliased to `NotFound`) so an admin
//! inspecting one grant can see it is corrupt and target [`delete_grant`],
//! which does no parsing.
//!
//! # Accepted risk (recorded)
//!
//! The `MAX_GRANTS_PER_SUBJECT` count-then-insert is not atomic (TOCTOU):
//! two concurrent inserts near the cap can jointly exceed it. Grant writes
//! are an infrequent admin path (M1.6a is the only writer) and the cap is an
//! anti-abuse soft bound, not a security invariant. Revisit only if a
//! concurrent grant-writer appears; do not build locking for it.

use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, EntityTrait, PaginatorTrait,
    QueryFilter, Set,
};
use time::OffsetDateTime;
use uptrakit_shared_types::access::bounds::{
    MAX_GRANT_DESCRIPTION_LEN, MAX_GRANTS_PER_SUBJECT, PatternSetError, validate_patterns,
};
use uptrakit_shared_types::access::{
    ActionPattern, ResourcePattern, Selector, SelectorValidationError,
};
use uuid::Uuid;

use crate::entity::access_grant::{self, GrantSubjectType};

/// Error returned by grant storage operations.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum AccessGrantError {
    /// Validation rule 1 failed (count bound or unmatchable pattern).
    #[error("invalid grant patterns: {0}")]
    Patterns(PatternSetError),
    /// A grant mixed system-plane and tenant-plane patterns.
    #[error("grant mixes system-plane and tenant-plane patterns")]
    PlaneMixing,
    /// Validation rule 2 failed (single tenant encoding per subject type).
    #[error("invalid tenant encoding: {0}")]
    TenantEncoding(&'static str),
    /// M1 phase gate (test row B9): non-`All` selectors are rejected until
    /// M2.1 replaces this with validation rules 3–5.
    #[error("non-All selectors are not accepted until M2.1")]
    SelectorPhaseGate,
    /// Selector ID bounds exceeded.
    #[error("invalid selector: {0}")]
    Selector(SelectorValidationError),
    /// Description exceeds [`MAX_GRANT_DESCRIPTION_LEN`] characters.
    #[error("description exceeds {max} characters")]
    DescriptionTooLong { max: usize },
    /// Subject already holds [`MAX_GRANTS_PER_SUBJECT`] grants.
    #[error("subject already holds {actual} grants (maximum {max})")]
    TooManyGrants { max: usize, actual: u64 },
    /// A pattern's resource shape is unknown to the plane classifier
    /// (`ResourcePattern` is `#[non_exhaustive]` — fail closed on future
    /// variants until this module classifies them).
    #[error("pattern shape not classifiable for plane rules")]
    UnclassifiablePattern,
    /// Selector JSON encoding failed (practically unreachable).
    #[error("failed to encode selector JSON: {0}")]
    SelectorEncode(String),
    /// No grant row with the given id.
    #[error("grant not found")]
    NotFound,
    /// The single-row load found the grant but its stored JSON is
    /// unparseable. Distinct from [`Self::NotFound`] by design.
    #[error("grant row is corrupt: {0}")]
    Corrupt(&'static str),
    /// A database error occurred. No `#[from]`: all conversions route
    /// through `.context_to()` via `impl_report_conversion!` below, and
    /// error-handling.md bans carrying both (the `From` impl would be dead
    /// code).
    #[error("database error: {0}")]
    Db(sea_orm::DbErr),
}

pub type Result<T> = std::result::Result<T, rootcause::Report<AccessGrantError>>;

uptrakit_shared_macros::impl_report_conversion!(sea_orm::DbErr => AccessGrantError::Db);

/// Grant subject: a user or a role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantSubject {
    User(Uuid),
    Role(Uuid),
}

/// Input for [`insert_grant`].
#[derive(Debug, Clone)]
pub struct NewGrant<'a> {
    pub subject: GrantSubject,
    pub tenant_id: Option<Uuid>,
    pub patterns: &'a [ActionPattern],
    pub selector: Selector,
    pub description: Option<String>,
    pub created_by: Option<Uuid>,
}

/// Input for [`update_grant`]. Subject and tenant encoding are immutable —
/// re-subject/re-scope is delete + create.
#[derive(Debug, Clone)]
pub struct GrantUpdate<'a> {
    pub patterns: &'a [ActionPattern],
    pub selector: Selector,
    pub description: Option<String>,
}

/// A grant row, typed fail-closed at the module boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedGrant {
    pub id: Uuid,
    pub tenant_id: Option<Uuid>,
    pub subject: GrantSubject,
    pub patterns: Vec<ActionPattern>,
    pub selector: Selector,
}

/// Result of [`load_grants_for_principal`]. `corrupt_skipped` exists so
/// M1.3's engine can emit the aggregate corruption counter (module docs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantLoad {
    pub grants: Vec<ResolvedGrant>,
    pub corrupt_skipped: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Plane {
    System,
    Tenant,
}

/// Segment-aware plane classification (mirrors `Resource::is_system`'s
/// dotted-prefix rule; never a bare substring). Runs after rule 1, so a
/// grammar-valid non-catalog bare `system` resource has already been
/// rejected as unmatchable. The `*` wildcard never reaches the system plane.
fn pattern_plane(pattern: &ActionPattern) -> Result<Plane> {
    match pattern.resource() {
        ResourcePattern::Any => Ok(Plane::Tenant),
        ResourcePattern::Exact(s) => Ok(if s.starts_with("system.") {
            Plane::System
        } else {
            Plane::Tenant
        }),
        ResourcePattern::Subtree(stem) => Ok(if stem == "system" || stem.starts_with("system.") {
            Plane::System
        } else {
            Plane::Tenant
        }),
        // `ResourcePattern` is #[non_exhaustive] cross-crate: fail closed on
        // any future variant this rule set has not classified.
        _ => Err(report!(AccessGrantError::UnclassifiablePattern)),
    }
}

/// Plane of the whole grant; [`AccessGrantError::PlaneMixing`] on a mix.
/// An empty pattern list confers nothing; tenant-plane encoding applies.
fn grant_plane(patterns: &[ActionPattern]) -> Result<Plane> {
    let mut plane: Option<Plane> = None;
    for pattern in patterns {
        let this = pattern_plane(pattern)?;
        match plane {
            None => plane = Some(this),
            Some(prev) if prev != this => bail!(AccessGrantError::PlaneMixing),
            Some(_) => {}
        }
    }
    Ok(plane.unwrap_or(Plane::Tenant))
}

/// The write-path validation chain (module docs; shared by insert + update).
fn validate_write(
    subject: GrantSubject,
    tenant_id: Option<Uuid>,
    patterns: &[ActionPattern],
    selector: &Selector,
    description: Option<&str>,
) -> Result<()> {
    // Rule 1.
    validate_patterns(patterns).map_err(|e| report!(AccessGrantError::Patterns(e)))?;
    // Plane purity (explicit rule, precedes encoding).
    let plane = grant_plane(patterns)?;
    // Rule 2: single encoding per subject type.
    match (plane, subject, tenant_id) {
        (Plane::System, _, Some(_)) => {
            bail!(AccessGrantError::TenantEncoding(
                "system-plane grant requires tenant_id NULL"
            ));
        }
        (_, GrantSubject::Role(_), Some(_)) => {
            bail!(AccessGrantError::TenantEncoding(
                "role-subject grant requires tenant_id NULL"
            ));
        }
        (Plane::Tenant, GrantSubject::User(_), None) => {
            bail!(AccessGrantError::TenantEncoding(
                "user-subject tenant-plane grant requires a tenant_id"
            ));
        }
        _ => {}
    }
    // B9 phase gate. M2.1 REPLACES this arm with validation rules 3–5 —
    // NOT a one-line deletion: rule 4 (selector referents exist in the
    // grant's tenant) is DB-backed read-before-write validation, new query
    // work this module does not perform today.
    if *selector != Selector::All {
        bail!(AccessGrantError::SelectorPhaseGate);
    }
    // Bounds. Selector::validate() is inert while the B9 gate holds; it is
    // kept so M2.1 swaps the gate arm without re-adding it.
    selector
        .validate()
        .map_err(|e| report!(AccessGrantError::Selector(e)))?;
    if let Some(description) = description
        && description.chars().count() > MAX_GRANT_DESCRIPTION_LEN
    {
        bail!(AccessGrantError::DescriptionTooLong {
            max: MAX_GRANT_DESCRIPTION_LEN
        });
    }
    Ok(())
}

fn split_subject(subject: GrantSubject) -> (GrantSubjectType, Uuid) {
    match subject {
        GrantSubject::User(id) => (GrantSubjectType::User, id),
        GrantSubject::Role(id) => (GrantSubjectType::Role, id),
    }
}

fn subject_from_row(row: &access_grant::Model) -> GrantSubject {
    match row.subject_type {
        GrantSubjectType::User => GrantSubject::User(row.subject_id),
        GrantSubjectType::Role => GrantSubject::Role(row.subject_id),
    }
}

fn patterns_json(patterns: &[ActionPattern]) -> serde_json::Value {
    serde_json::Value::from(
        patterns
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<String>>(),
    )
}

fn selector_json(selector: &Selector) -> Result<serde_json::Value> {
    match serde_json::to_value(selector) {
        Ok(v) => Ok(v),
        Err(e) => Err(report!(AccessGrantError::SelectorEncode(e.to_string()))),
    }
}

/// Fail-closed row typing shared by the single and batch loads.
fn resolve_row(row: access_grant::Model) -> std::result::Result<ResolvedGrant, &'static str> {
    let subject = subject_from_row(&row);
    let patterns: Vec<ActionPattern> = match serde_json::from_value(row.patterns) {
        Ok(p) => p,
        Err(_) => return Err("unparseable patterns JSON"),
    };
    let selector: Selector = match serde_json::from_value(row.selector) {
        Ok(s) => s,
        Err(_) => return Err("unparseable selector JSON"),
    };
    Ok(ResolvedGrant {
        id: row.id,
        tenant_id: row.tenant_id,
        subject,
        patterns,
        selector,
    })
}

/// Insert a validated grant; returns the new row id.
pub async fn insert_grant(db: &impl ConnectionTrait, grant: NewGrant<'_>) -> Result<Uuid> {
    validate_write(
        grant.subject,
        grant.tenant_id,
        grant.patterns,
        &grant.selector,
        grant.description.as_deref(),
    )?;
    let (subject_type, subject_id) = split_subject(grant.subject);
    // Count-then-insert: accepted TOCTOU risk (module docs).
    let existing = access_grant::Entity::find()
        .filter(access_grant::Column::SubjectType.eq(subject_type))
        .filter(access_grant::Column::SubjectId.eq(subject_id))
        .count(db)
        .await
        .context_to()?;
    if existing >= MAX_GRANTS_PER_SUBJECT as u64 {
        bail!(AccessGrantError::TooManyGrants {
            max: MAX_GRANTS_PER_SUBJECT,
            actual: existing,
        });
    }
    let id = Uuid::now_v7();
    let now = OffsetDateTime::now_utc();
    access_grant::ActiveModel {
        id: Set(id),
        tenant_id: Set(grant.tenant_id),
        subject_type: Set(subject_type),
        subject_id: Set(subject_id),
        patterns: Set(patterns_json(grant.patterns)),
        selector: Set(selector_json(&grant.selector)?),
        description: Set(grant.description),
        created_at: Set(now),
        updated_at: Set(now),
        created_by: Set(grant.created_by),
    }
    .insert(db)
    .await
    .context_to()?;
    Ok(id)
}

/// Update a grant's patterns/selector/description; the full validation chain
/// re-runs against the STORED subject and tenant encoding.
pub async fn update_grant(
    db: &impl ConnectionTrait,
    id: Uuid,
    update: GrantUpdate<'_>,
) -> Result<()> {
    let row = access_grant::Entity::find_by_id(id)
        .one(db)
        .await
        .context_to()?
        .ok_or_else(|| report!(AccessGrantError::NotFound))?;
    let subject = subject_from_row(&row);
    validate_write(
        subject,
        row.tenant_id,
        update.patterns,
        &update.selector,
        update.description.as_deref(),
    )?;
    let mut active: access_grant::ActiveModel = row.into();
    active.patterns = Set(patterns_json(update.patterns));
    active.selector = Set(selector_json(&update.selector)?);
    active.description = Set(update.description);
    active.updated_at = Set(OffsetDateTime::now_utc());
    active.update(db).await.context_to()?;
    Ok(())
}

/// Delete a grant by id. Does no JSON parsing, so it works on corrupt rows.
pub async fn delete_grant(db: &impl ConnectionTrait, id: Uuid) -> Result<()> {
    let res = access_grant::Entity::delete_by_id(id)
        .exec(db)
        .await
        .context_to()?;
    if res.rows_affected == 0 {
        bail!(AccessGrantError::NotFound);
    }
    Ok(())
}

/// Load a single grant (M1.6a's read-before-update). Corruption is surfaced
/// as [`AccessGrantError::Corrupt`], never aliased to `NotFound`.
pub async fn load_grant(db: &impl ConnectionTrait, id: Uuid) -> Result<ResolvedGrant> {
    let row = access_grant::Entity::find_by_id(id)
        .one(db)
        .await
        .context_to()?
        .ok_or_else(|| report!(AccessGrantError::NotFound))?;
    match resolve_row(row) {
        Ok(grant) => Ok(grant),
        Err(reason) => Err(report!(AccessGrantError::Corrupt(reason))),
    }
}

/// Load every grant row for a principal in ONE query (module docs carry the
/// safety argument and the loud-skip contract).
pub async fn load_grants_for_principal(
    db: &impl ConnectionTrait,
    tenant_id: Uuid,
    user_id: Uuid,
    role_ids: &[Uuid],
) -> Result<GrantLoad> {
    let condition = Condition::any()
        .add(
            Condition::all()
                .add(access_grant::Column::SubjectType.eq(GrantSubjectType::User))
                .add(access_grant::Column::SubjectId.eq(user_id))
                .add(
                    Condition::any()
                        .add(access_grant::Column::TenantId.eq(tenant_id))
                        .add(access_grant::Column::TenantId.is_null()),
                ),
        )
        .add(
            // An empty role_ids slice renders as an always-false predicate
            // (sea-query emits `1 = 2` for an empty IN tuple).
            Condition::all()
                .add(access_grant::Column::SubjectType.eq(GrantSubjectType::Role))
                .add(access_grant::Column::SubjectId.is_in(role_ids.iter().copied())),
        );
    let rows = access_grant::Entity::find()
        .filter(condition)
        .all(db)
        .await
        .context_to()?;

    let mut grants = Vec::with_capacity(rows.len());
    let mut corrupt_skipped = 0usize;
    for row in rows {
        let (id, subject_type, subject_id) = (row.id, row.subject_type, row.subject_id);
        match resolve_row(row) {
            Ok(grant) => grants.push(grant),
            Err(reason) => {
                corrupt_skipped += 1;
                tracing::error!(
                    grant_id = %id,
                    subject_type = ?subject_type,
                    subject_id = %subject_id,
                    reason,
                    "skipping corrupt access_grants row during resolution"
                );
            }
        }
    }
    Ok(GrantLoad {
        grants,
        corrupt_skipped,
    })
}

#[cfg(test)]
mod tests {
    use sea_orm::{ConnectOptions, Database, DatabaseConnection, QueryFilter};
    use uptrakit_shared_types::access::bounds::MAX_GRANTS_PER_SUBJECT;

    use super::*;
    use crate::entity::{role, tenant};

    async fn test_db() -> DatabaseConnection {
        let mut opt = ConnectOptions::new("sqlite::memory:");
        opt.max_connections(1).min_connections(1);
        let db = Database::connect(opt).await.expect("connect to test db");
        crate::migration::run_migrations(&db)
            .await
            .expect("run migrations");
        db
    }

    async fn default_tenant_id(db: &DatabaseConnection) -> Uuid {
        tenant::Entity::find()
            .one(db)
            .await
            .expect("query tenants")
            .expect("default tenant is seeded")
            .id
    }

    async fn role_id(db: &DatabaseConnection, name: &str) -> Uuid {
        role::Entity::find()
            .filter(role::Column::Name.eq(name))
            .one(db)
            .await
            .expect("query roles")
            .expect("seed role exists")
            .id
    }

    fn pat(s: &str) -> ActionPattern {
        s.parse().expect("pattern parses")
    }

    fn pats(strs: &[&str]) -> Vec<ActionPattern> {
        strs.iter().map(|s| pat(s)).collect()
    }

    fn user_grant<'a>(
        tenant_id: Option<Uuid>,
        user_id: Uuid,
        patterns: &'a [ActionPattern],
    ) -> NewGrant<'a> {
        NewGrant {
            subject: GrantSubject::User(user_id),
            tenant_id,
            patterns,
            selector: Selector::All,
            description: None,
            created_by: None,
        }
    }

    /// B1: valid user-subject tenant grant and role-subject global grant
    /// insert and load back typed.
    #[tokio::test]
    async fn b1_valid_grants_round_trip_typed() {
        let db = test_db().await;
        let tenant_id = default_tenant_id(&db).await;
        let user_id = Uuid::now_v7();

        let patterns = pats(&["hosts:read", "updates:trigger"]);
        let grant_id = insert_grant(&db, user_grant(Some(tenant_id), user_id, &patterns))
            .await
            .expect("valid user grant inserts");
        let loaded = load_grant(&db, grant_id).await.expect("load back");
        assert_eq!(loaded.subject, GrantSubject::User(user_id));
        assert_eq!(loaded.tenant_id, Some(tenant_id));
        assert_eq!(loaded.patterns, patterns);
        assert_eq!(loaded.selector, Selector::All);

        let viewer = role_id(&db, "viewer").await;
        let role_patterns = pats(&["notifications:read"]);
        let role_grant = NewGrant {
            subject: GrantSubject::Role(viewer),
            tenant_id: None,
            patterns: &role_patterns,
            selector: Selector::All,
            description: Some("extra viewer read".to_string()),
            created_by: None,
        };
        let role_grant_id = insert_grant(&db, role_grant)
            .await
            .expect("role grant inserts");
        let loaded = load_grant(&db, role_grant_id).await.expect("load back");
        assert_eq!(loaded.subject, GrantSubject::Role(viewer));
        assert_eq!(loaded.tenant_id, None);
    }

    /// B2: `system.*` patterns with `tenant_id = NULL` accepted (any subject).
    #[tokio::test]
    async fn b2_system_plane_null_tenant_accepted() {
        let db = test_db().await;
        let patterns = pats(&["system.services:read"]);
        insert_grant(&db, user_grant(None, Uuid::now_v7(), &patterns))
            .await
            .expect("system-plane user grant with NULL tenant is legal");
    }

    /// B3: three typed rejections — system pattern with tenant; tenant-plane
    /// user grant with NULL tenant; mixed-plane grant.
    #[tokio::test]
    async fn b3_encoding_and_plane_rejections() {
        let db = test_db().await;
        let tenant_id = default_tenant_id(&db).await;

        let system = pats(&["system.services:read"]);
        let err = insert_grant(&db, user_grant(Some(tenant_id), Uuid::now_v7(), &system))
            .await
            .expect_err("system pattern with tenant must be rejected");
        assert!(
            matches!(err.current_context(), AccessGrantError::TenantEncoding(_)),
            "expected TenantEncoding, got: {err}"
        );

        let tenant_plane = pats(&["hosts:read"]);
        let err = insert_grant(&db, user_grant(None, Uuid::now_v7(), &tenant_plane))
            .await
            .expect_err("tenant-plane user grant with NULL tenant must be rejected");
        assert!(
            matches!(err.current_context(), AccessGrantError::TenantEncoding(_)),
            "expected TenantEncoding, got: {err}"
        );

        let mixed = pats(&["system.services:read", "hosts:read"]);
        let err = insert_grant(&db, user_grant(None, Uuid::now_v7(), &mixed))
            .await
            .expect_err("mixed-plane grant must be rejected");
        assert!(
            matches!(err.current_context(), AccessGrantError::PlaneMixing),
            "expected PlaneMixing, got: {err}"
        );
    }

    /// B9: each non-`All` selector variant rejected with SelectorPhaseGate.
    #[tokio::test]
    async fn b9_non_all_selectors_phase_gated() {
        let db = test_db().await;
        let tenant_id = default_tenant_id(&db).await;
        let patterns = pats(&["checks:trigger"]);
        let selectors = [
            Selector::Tags {
                ids: vec![Uuid::now_v7()],
            },
            Selector::Hosts {
                ids: vec![Uuid::now_v7()],
            },
            Selector::Software {
                ids: vec![Uuid::now_v7()],
            },
            Selector::Items {
                ids: vec![Uuid::now_v7()],
            },
        ];
        for selector in selectors {
            let grant = NewGrant {
                subject: GrantSubject::User(Uuid::now_v7()),
                tenant_id: Some(tenant_id),
                patterns: &patterns,
                selector: selector.clone(),
                description: None,
                created_by: None,
            };
            let err = insert_grant(&db, grant)
                .await
                .expect_err("non-All selector must be phase-gated");
            assert!(
                matches!(err.current_context(), AccessGrantError::SelectorPhaseGate),
                "expected SelectorPhaseGate for {selector:?}, got: {err}"
            );
        }
    }

    /// B11 + resolution: global role-subject grant with tenant-plane patterns
    /// is legal and resolves exactly when the role id is passed; the one-call
    /// union returns direct-user + role + global-user rows and excludes
    /// foreign users' and unassigned roles' rows.
    #[tokio::test]
    async fn b11_and_resolution_union() {
        let db = test_db().await;
        let tenant_id = default_tenant_id(&db).await;
        let user_id = Uuid::now_v7();
        let foreign_user = Uuid::now_v7();
        let viewer = role_id(&db, "viewer").await;
        let operator = role_id(&db, "operator").await;

        let direct = pats(&["hosts:read"]);
        let direct_id = insert_grant(&db, user_grant(Some(tenant_id), user_id, &direct))
            .await
            .expect("direct user grant");
        let global = pats(&["system.audit:read"]);
        let global_id = insert_grant(&db, user_grant(None, user_id, &global))
            .await
            .expect("global user grant");
        let role_patterns = pats(&["notifications:read"]);
        let role_grant_id = insert_grant(
            &db,
            NewGrant {
                subject: GrantSubject::Role(viewer),
                tenant_id: None,
                patterns: &role_patterns,
                selector: Selector::All,
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("B11: global role grant with tenant-plane patterns is legal");
        // Noise rows that must NOT resolve for our principal.
        insert_grant(&db, user_grant(Some(tenant_id), foreign_user, &direct))
            .await
            .expect("foreign user grant");
        insert_grant(
            &db,
            NewGrant {
                subject: GrantSubject::Role(operator),
                tenant_id: None,
                patterns: &role_patterns,
                selector: Selector::All,
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("unassigned role grant");

        let load = load_grants_for_principal(&db, tenant_id, user_id, &[viewer])
            .await
            .expect("resolution succeeds");
        assert_eq!(load.corrupt_skipped, 0, "no corrupt rows seeded");
        let mut got: Vec<Uuid> = load.grants.iter().map(|g| g.id).collect();
        got.sort();
        // Note: the viewer role also carries its Task 2 SEED grant — the
        // resolution returns it too. Filter to the ids this test created.
        let mut expected = vec![direct_id, global_id, role_grant_id];
        expected.sort();
        for id in &expected {
            assert!(got.contains(id), "resolution must include {id}");
        }
        let foreign_ids: Vec<Uuid> = load
            .grants
            .iter()
            .filter(|g| {
                g.subject == GrantSubject::User(foreign_user)
                    || g.subject == GrantSubject::Role(operator)
            })
            .map(|g| g.id)
            .collect();
        assert!(
            foreign_ids.is_empty(),
            "foreign user / unassigned role rows must not resolve: {foreign_ids:?}"
        );

        // Without the role id, the role grant must not resolve.
        let without_role = load_grants_for_principal(&db, tenant_id, user_id, &[])
            .await
            .expect("resolution with empty roles succeeds");
        assert!(
            !without_role.grants.iter().any(|g| g.id == role_grant_id),
            "role grant must not resolve without the role id"
        );
    }

    /// Corrupt-row loud skip: the call SUCCEEDS, returns the valid rows,
    /// counts the skip, and omits the corrupt one. In-crate test bypasses the
    /// module through the entity deliberately, to stage what the module's
    /// write path can never produce.
    #[tokio::test]
    async fn resolution_loud_skips_corrupt_rows() {
        let db = test_db().await;
        let tenant_id = default_tenant_id(&db).await;
        let user_id = Uuid::now_v7();
        let good = pats(&["hosts:read"]);
        let good_id = insert_grant(&db, user_grant(Some(tenant_id), user_id, &good))
            .await
            .expect("good grant");

        access_grant::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(Some(tenant_id)),
            subject_type: Set(GrantSubjectType::User),
            subject_id: Set(user_id),
            patterns: Set(serde_json::json!(["not a pattern ["])),
            selector: Set(serde_json::json!({"type": "all"})),
            description: Set(None),
            created_at: Set(OffsetDateTime::now_utc()),
            updated_at: Set(OffsetDateTime::now_utc()),
            created_by: Set(None),
        }
        .insert(&db)
        .await
        .expect("hand-insert corrupt row");

        let load = load_grants_for_principal(&db, tenant_id, user_id, &[])
            .await
            .expect("resolution must succeed despite the corrupt row");
        assert_eq!(load.corrupt_skipped, 1, "one corrupt row must be counted");
        assert_eq!(load.grants.len(), 1, "only the valid grant returns");
        assert!(
            load.grants.iter().any(|g| g.id == good_id),
            "the valid grant must be returned"
        );
    }

    /// Single-row corruption is Corrupt, never NotFound.
    #[tokio::test]
    async fn load_grant_surfaces_corrupt_distinctly() {
        let db = test_db().await;
        let corrupt_id = Uuid::now_v7();
        access_grant::ActiveModel {
            id: Set(corrupt_id),
            tenant_id: Set(None),
            subject_type: Set(GrantSubjectType::User),
            subject_id: Set(Uuid::now_v7()),
            patterns: Set(serde_json::json!("not-an-array")),
            selector: Set(serde_json::json!({"type": "all"})),
            description: Set(None),
            created_at: Set(OffsetDateTime::now_utc()),
            updated_at: Set(OffsetDateTime::now_utc()),
            created_by: Set(None),
        }
        .insert(&db)
        .await
        .expect("hand-insert corrupt row");

        let err = load_grant(&db, corrupt_id).await.expect_err("must error");
        assert!(
            matches!(err.current_context(), AccessGrantError::Corrupt(_)),
            "expected Corrupt, got: {err}"
        );
        let err = load_grant(&db, Uuid::now_v7())
            .await
            .expect_err("must error");
        assert!(
            matches!(err.current_context(), AccessGrantError::NotFound),
            "expected NotFound, got: {err}"
        );
        // delete_grant does no parsing — it must succeed on the corrupt row.
        delete_grant(&db, corrupt_id)
            .await
            .expect("delete works on corrupt rows");
    }

    /// MAX_GRANTS_PER_SUBJECT at the boundary: at-bound accepted, over-bound
    /// rejected. Seeded via ONE batch entity insert, not 200 module calls.
    #[tokio::test]
    async fn max_grants_per_subject_boundary() {
        let db = test_db().await;
        let tenant_id = default_tenant_id(&db).await;
        let user_id = Uuid::now_v7();
        let now = OffsetDateTime::now_utc();

        let rows: Vec<access_grant::ActiveModel> = (0..MAX_GRANTS_PER_SUBJECT - 1)
            .map(|_| access_grant::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(Some(tenant_id)),
                subject_type: Set(GrantSubjectType::User),
                subject_id: Set(user_id),
                patterns: Set(serde_json::json!(["hosts:read"])),
                selector: Set(serde_json::json!({"type": "all"})),
                description: Set(None),
                created_at: Set(now),
                updated_at: Set(now),
                created_by: Set(None),
            })
            .collect();
        access_grant::Entity::insert_many(rows)
            .exec(&db)
            .await
            .expect("batch-seed grants");

        let patterns = pats(&["hosts:read"]);
        // Insert #200 (at bound: 199 existing) accepted.
        insert_grant(&db, user_grant(Some(tenant_id), user_id, &patterns))
            .await
            .expect("insert at the bound must be accepted");
        // Insert #201 rejected.
        let err = insert_grant(&db, user_grant(Some(tenant_id), user_id, &patterns))
            .await
            .expect_err("insert over the bound must be rejected");
        assert!(
            matches!(
                err.current_context(),
                AccessGrantError::TooManyGrants { .. }
            ),
            "expected TooManyGrants, got: {err}"
        );
    }

    /// B8 subset live in M1.2: the write path enforces the 16-pattern count
    /// bound and the description length bound.
    #[tokio::test]
    async fn b8_write_path_bounds() {
        let db = test_db().await;
        let tenant_id = default_tenant_id(&db).await;

        let seventeen: Vec<ActionPattern> = (0..17).map(|_| pat("hosts:read")).collect();
        let err = insert_grant(&db, user_grant(Some(tenant_id), Uuid::now_v7(), &seventeen))
            .await
            .expect_err("17 patterns must be rejected");
        assert!(
            matches!(err.current_context(), AccessGrantError::Patterns(_)),
            "expected Patterns, got: {err}"
        );

        let patterns = pats(&["hosts:read"]);
        let grant = NewGrant {
            subject: GrantSubject::User(Uuid::now_v7()),
            tenant_id: Some(tenant_id),
            patterns: &patterns,
            selector: Selector::All,
            description: Some("x".repeat(MAX_GRANT_DESCRIPTION_LEN + 1)),
            created_by: None,
        };
        let err = insert_grant(&db, grant)
            .await
            .expect_err("over-length description must be rejected");
        assert!(
            matches!(
                err.current_context(),
                AccessGrantError::DescriptionTooLong { .. }
            ),
            "expected DescriptionTooLong, got: {err}"
        );
    }

    /// update_grant re-validates against the STORED subject/tenant and
    /// persists valid updates; missing id is NotFound.
    #[tokio::test]
    async fn update_grant_revalidates_and_persists() {
        let db = test_db().await;
        let tenant_id = default_tenant_id(&db).await;
        let patterns = pats(&["hosts:read"]);
        let id = insert_grant(&db, user_grant(Some(tenant_id), Uuid::now_v7(), &patterns))
            .await
            .expect("insert");

        // A system-plane update against the stored non-NULL tenant must fail.
        let system = pats(&["system.services:read"]);
        let err = update_grant(
            &db,
            id,
            GrantUpdate {
                patterns: &system,
                selector: Selector::All,
                description: None,
            },
        )
        .await
        .expect_err("system-plane update on tenant grant must be rejected");
        assert!(
            matches!(err.current_context(), AccessGrantError::TenantEncoding(_)),
            "expected TenantEncoding, got: {err}"
        );

        let widened = pats(&["hosts:read", "checks:trigger"]);
        update_grant(
            &db,
            id,
            GrantUpdate {
                patterns: &widened,
                selector: Selector::All,
                description: Some("widened".to_string()),
            },
        )
        .await
        .expect("valid update persists");
        let loaded = load_grant(&db, id).await.expect("load back");
        assert_eq!(loaded.patterns, widened);

        let err = update_grant(
            &db,
            Uuid::now_v7(),
            GrantUpdate {
                patterns: &widened,
                selector: Selector::All,
                description: None,
            },
        )
        .await
        .expect_err("missing id must be NotFound");
        assert!(
            matches!(err.current_context(), AccessGrantError::NotFound),
            "expected NotFound, got: {err}"
        );
    }
}
