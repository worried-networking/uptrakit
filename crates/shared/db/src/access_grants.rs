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
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, DatabaseConnection,
    DatabaseTransaction, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};
use std::collections::{HashMap, HashSet};
use time::OffsetDateTime;
use uptrakit_db_tx::begin_immediate;
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
    /// The default-tenant sentinel row was missing at lock time. Hard
    /// error: a zero-row `FOR UPDATE` locks nothing (Postgres), so a
    /// missing sentinel must never pass through as "guard ran".
    #[error("lockout-guard sentinel row missing")]
    SentinelMissing,
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
    pub description: Option<String>,
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

/// Whether the pattern set reaches the `system.` plane.
///
/// Public wrapper over this module's plane classifier for the M1.6a
/// handler-side `system.access:manage` fine check — callers must never
/// re-derive the dotted-prefix rule outside this module. Propagates
/// [`AccessGrantError::PlaneMixing`] / [`AccessGrantError::UnclassifiablePattern`].
pub fn patterns_reach_system_plane(patterns: &[ActionPattern]) -> Result<bool> {
    Ok(grant_plane(patterns)? == Plane::System)
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
        description: row.description,
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

/// Management listing: the tenant's rows plus every global (`tenant_id
/// NULL`) row, optionally filtered to one exact subject. Bounded by
/// `MAX_GRANTS_PER_SUBJECT` per subject and the deployment's scale — no
/// pagination in M1 (spec §Grant CRUD).
pub async fn list_grants(
    db: &impl ConnectionTrait,
    tenant_id: Uuid,
    subject: Option<GrantSubject>,
) -> Result<GrantLoad> {
    let mut query = access_grant::Entity::find().filter(
        Condition::any()
            .add(access_grant::Column::TenantId.eq(tenant_id))
            .add(access_grant::Column::TenantId.is_null()),
    );
    if let Some(subject) = subject {
        let (subject_type, subject_id) = split_subject(subject);
        query = query
            .filter(access_grant::Column::SubjectType.eq(subject_type))
            .filter(access_grant::Column::SubjectId.eq(subject_id));
    }
    let rows = query
        .order_by_asc(access_grant::Column::CreatedAt)
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

/// Role-delete orphan cleanup: delete every role-subject grant row for
/// `role_id`. `access_grants.subject_id` carries no FK, so nothing
/// cascades — the role-delete transaction (Plan 2) calls this explicitly.
///
/// CALLER OBLIGATION: this takes no tenant argument and performs no
/// ownership check — it deletes the grants of whatever `role_id` it is
/// handed. Call it only after the caller has proved, in the same
/// transaction, that the acting tenant owns the role (Plan 2 does that via
/// `queries::roles::delete_role_rows`, which resolves the role OWN-tenant
/// scoped first). Calling it standalone, or before the ownership check,
/// erases another tenant's grants.
pub async fn delete_grants_for_role(db: &impl ConnectionTrait, role_id: Uuid) -> Result<u64> {
    let res = access_grant::Entity::delete_many()
        .filter(access_grant::Column::SubjectType.eq(GrantSubjectType::Role))
        .filter(access_grant::Column::SubjectId.eq(role_id))
        .exec(db)
        .await
        .context_to()?;
    Ok(res.rows_affected)
}

/// `db-migrate` table descriptor for `access_grants`. This module is the
/// sanctioned exception in `ci/verify_engine_owned_entities.sh` — the only
/// place permitted to name the `access_grant` entity — so
/// `crate::migrate_core_tables::core_tables()` calls this function instead of
/// naming the entity itself. Plain row copy/clean/verify; no validation
/// applies (db-migrate moves already-validated rows between backends).
#[cfg(feature = "db-migrate")]
pub fn core_table_descriptor() -> crate::migrate_core_tables::CoreTableDescriptor {
    crate::migrate_core_tables::CoreTableDescriptor::for_entity::<access_grant::Entity>(
        "access_grants",
    )
}

// ── M1.6a lockout guard ─────────────────────────────────────────────────

/// A shrinking authority mutation the lockout guard must evaluate.
///
/// `#[non_exhaustive]`: the rustdoc on [`check_lockout`] names future
/// guarded mutations (user hard-delete, tenant deactivation, credential
/// resets) — cross-crate callers construct variants but must not match
/// exhaustively. (Cross-crate construction of a non_exhaustive ENUM's
/// variants is allowed; only exhaustive matching is restricted.)
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum GuardedMutation<'a> {
    /// Delete the grant row.
    DeleteGrant { grant_id: Uuid },
    /// Replace the grant row's patterns/selector.
    UpdateGrant {
        grant_id: Uuid,
        new_patterns: &'a [ActionPattern],
        new_selector: &'a Selector,
    },
    /// Delete the role, its assignments, and its role-subject grants.
    DeleteRole { role_id: Uuid },
    /// Full-replace the user's role set in `tenant_id` (post-state
    /// evaluation — a swap of covering role A for covering role B is legal).
    SetUserRoles {
        tenant_id: Uuid,
        user_id: Uuid,
        new_role_ids: &'a [Uuid],
    },
    /// Deactivate the user.
    DeactivateUser { user_id: Uuid },
}

/// Outcome of [`check_lockout`].
///
/// Deliberately NOT `#[non_exhaustive]`: a closed verdict set (exactly two
/// planes plus pass) that callers must handle exhaustively — a new verdict
/// kind is a semantic change every caller must see, per the
/// closed-enum exception in coding-standards.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockoutVerdict {
    /// Every plane keeps at least one covering active holder.
    Permitted,
    /// A tenant would lose its last active user whose resolved authority
    /// covers `access:manage` @ selector `All`.
    TenantLockout,
    /// The global plane would lose its last active user whose global
    /// authority covers `system.access:manage`.
    SystemLockout,
}

/// In-memory assignment triple (post-state simulation needs no timestamps).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Assignment {
    tenant_id: Uuid,
    user_id: Uuid,
    role_id: Uuid,
}

/// Lockout guard for shrinking authority mutations (M1.6a).
///
/// Invariant: no mutation may leave zero active users whose resolved
/// authority covers `access:manage` @ `All` in any tenant that had one
/// (tenant plane, checked PER TENANT), nor zero active users whose global
/// authority covers `system.access:manage` (global plane) — evaluated as a
/// pre-state vs simulated post-state holder comparison inside the caller's
/// transaction.
///
/// Guarded (shrinking) mutations: grant update, grant delete, role delete,
/// role-set replace (always, post-state), user deactivation, OIDC role-set
/// replace. Adding-only mutations (grant create, role create,
/// rename/description update, user activation) must NOT call this — under
/// allow-only union they cannot shrink authority.
///
/// OIDC role sync (`sync_oidc_roles`,
/// `crates/ui/web-api-auth/src/auth/authentication.rs`) is now guarded
/// (M1.6a) against covering shrinks: a mapped replace that would strip the
/// sole `access:manage`/`system.access:manage` covering holder is skipped,
/// not applied (`RoleSyncOutcome::SkippedLockout` in that module), and the
/// login proceeds with the pre-sync role set. Do not over-claim this closes
/// the whole gap: the sync's six fail-open early returns (no
/// `role_claim_path` configured, empty `role_mapping`, claim path missing
/// from the token, a claim value that is neither an array nor a string, an
/// unmapped claim value, or the mapped set resolving to zero local roles)
/// still leave the user's existing roles untouched with no signal — an
/// IdP-side de-provisioning that simply stops sending the covering claim
/// never reaches this guard at all. That de-provisioning drift stays an
/// open, named gap.
///
/// Serialization: one `SELECT … FOR UPDATE` on the DEFAULT tenant's
/// `tenants` row — a single global sentinel for both planes (role-subject
/// grants are tenant-NULL and a role can be assigned in multiple tenants;
/// per-tenant sentinels would let two related shrinks slip past each
/// other). The `&DatabaseTransaction` parameter rules out running the
/// guard on a pooled autocommit connection (where `FOR UPDATE` releases
/// per-statement); it CANNOT express the SQLite `Immediate` mode, which
/// remains a caller obligation: obtain the transaction via
/// [`begin_guarded`], or reuse an existing `Immediate` one — a Deferred
/// `begin()` compiles but serializes nothing on SQLite (sea_query drops
/// the lock clause there; on Postgres the row lock is the serialization
/// point regardless). A missing sentinel row is a hard error
/// ([`AccessGrantError::SentinelMissing`]) — a zero-row `FOR UPDATE`
/// locks nothing.
///
/// `sentinel_tenant_id` MUST be the deployment-wide DEFAULT tenant id
/// (`AppState.default_tenant_id`), never a request-derived active tenant —
/// two shrinks locking different rows would both pass, and single-tenant
/// deployments make the mistake untestable.
///
/// When both planes would be stripped by one mutation, the verdict is
/// [`LockoutVerdict::SystemLockout`] (the less recoverable plane wins).
///
/// CALLER OBLIGATION: on any verdict other than [`LockoutVerdict::Permitted`],
/// the caller MUST NOT write the guarded mutation. Committing *unrelated*
/// work in the same transaction is permitted — the OIDC role sync commits a
/// transaction that also carries user creation after a non-`Permitted`
/// verdict, leaving only the role-set write skipped (the sentinel lock is
/// then held until that commit). This function only evaluates the
/// post-state in memory; it never itself applies or rejects the mutation
/// against the database.
///
/// PROHIBITION: this guard must never call `AccessEngine` — its cache and
/// pool-connection reads escape the transaction and under-count holders.
///
/// Future obligations (each becomes a guarded mutation the day it is
/// built; none exist today): user hard-delete, tenant deactivation, admin
/// credential-reset endpoints.
pub async fn check_lockout(
    txn: &DatabaseTransaction,
    sentinel_tenant_id: Uuid,
    mutation: &GuardedMutation<'_>,
) -> Result<LockoutVerdict> {
    lock_sentinel(txn, sentinel_tenant_id).await?;

    // Typed-column loads only — no JSON operators (JSONB has no LIKE;
    // spec decision 7). ponytail: three flat full-table loads; switch to
    // subject-scoped queries if authority data ever outgrows admin scale.
    let grant_rows = access_grant::Entity::find().all(txn).await.context_to()?;
    let mut grants: Vec<ResolvedGrant> = Vec::with_capacity(grant_rows.len());
    // Corrupt rows never count as holders (fail-closed), but the module's
    // LOUD-SKIP contract still applies — same shape as
    // `load_grants_for_principal`/`list_grants`.
    for row in grant_rows {
        let (id, subject_type, subject_id) = (row.id, row.subject_type, row.subject_id);
        match resolve_row(row) {
            Ok(grant) => grants.push(grant),
            Err(reason) => {
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
    let mut active_users: HashSet<Uuid> = crate::entity::user::Entity::find()
        .filter(crate::entity::user::Column::IsActive.eq(true))
        .select_only()
        .column(crate::entity::user::Column::Id)
        .into_tuple::<Uuid>()
        .all(txn)
        .await
        .context_to()?
        .into_iter()
        .collect();
    // Cross-tenant by design: the lockout invariant spans every tenant a
    // role is assigned in, so this deliberately does not filter by tenant —
    // do not copy this shape for tenant-scoped queries (use `TenantDb`).
    let mut assignments: Vec<Assignment> = crate::entity::user_role::Entity::find()
        .all(txn)
        .await
        .context_to()?
        .into_iter()
        .map(|a| Assignment {
            tenant_id: a.tenant_id,
            user_id: a.user_id,
            role_id: a.role_id,
        })
        .collect();

    let (pre_tenant, pre_global) = covering_holders(&grants, &assignments, &active_users);
    apply_mutation(mutation, &mut grants, &mut assignments, &mut active_users);
    let (post_tenant, post_global) = covering_holders(&grants, &assignments, &active_users);

    // System plane first: when one mutation strips both planes, report the
    // less recoverable one.
    if !pre_global.is_empty() && post_global.is_empty() {
        return Ok(LockoutVerdict::SystemLockout);
    }
    for tenant_id in pre_tenant.keys() {
        if post_tenant.get(tenant_id).is_none_or(HashSet::is_empty) {
            return Ok(LockoutVerdict::TenantLockout);
        }
    }
    Ok(LockoutVerdict::Permitted)
}

/// Open the guard's serialization transaction: `Immediate` on SQLite (the
/// write lock is taken at BEGIN — sea_query drops `FOR UPDATE` there); on
/// Postgres the sentinel row lock inside [`check_lockout`] serializes.
///
/// MANDATORY for any NEW transaction opened for the guard — handlers that
/// already hold an `Immediate` transaction reuse it instead (the two
/// sanctioned cases: the users.rs role/active handlers and the OIDC sync
/// callers).
///
/// PROHIBITION: never call while another transaction is open on the same
/// pool — reuse that transaction. On SQLite's single writer a second
/// `BEGIN IMMEDIATE` returns `SQLITE_BUSY` or deadlocks against the outer
/// transaction.
pub async fn begin_guarded(db: &DatabaseConnection) -> Result<DatabaseTransaction> {
    begin_immediate(db).await.context_to()
}

/// Lock the global sentinel row. Zero rows ⇒ hard error, never pass-through.
async fn lock_sentinel(txn: &DatabaseTransaction, sentinel_tenant_id: Uuid) -> Result<()> {
    let row = crate::entity::tenant::Entity::find_by_id(sentinel_tenant_id)
        .lock_exclusive()
        .one(txn)
        .await
        .context_to()?;
    if row.is_none() {
        bail!(AccessGrantError::SentinelMissing);
    }
    Ok(())
}

/// Covering-holder sets: per-tenant `access:manage` holders and global
/// `system.access:manage` holders. Only selector-`All` rows count; coverage
/// is decided by the production matcher (`ActionPattern::matches`), which
/// for the dot-free `access` resource admits exactly
/// {`access`, `*`} × {`manage`, `*`} and for `system.access` exactly
/// {`system.access`, `system.*`} × {`manage`, `*`} — the closed sets the
/// design's completeness argument names (pinned by
/// `covering_pattern_forms_are_the_closed_sets`).
fn covering_holders(
    grants: &[ResolvedGrant],
    assignments: &[Assignment],
    active_users: &HashSet<Uuid>,
) -> (HashMap<Uuid, HashSet<Uuid>>, HashSet<Uuid>) {
    use uptrakit_shared_types::access::actions;
    let mut per_tenant: HashMap<Uuid, HashSet<Uuid>> = HashMap::new();
    let mut global: HashSet<Uuid> = HashSet::new();
    for grant in grants {
        if grant.selector != Selector::All {
            continue;
        }
        let covers_tenant = grant
            .patterns
            .iter()
            .any(|p| p.matches(&actions::ACCESS_MANAGE));
        let covers_system = grant
            .patterns
            .iter()
            .any(|p| p.matches(&actions::SYSTEM_ACCESS_MANAGE));
        if !covers_tenant && !covers_system {
            continue;
        }
        match grant.subject {
            GrantSubject::User(user_id) => {
                if !active_users.contains(&user_id) {
                    continue;
                }
                if covers_system {
                    global.insert(user_id);
                }
                if covers_tenant && let Some(tenant_id) = grant.tenant_id {
                    per_tenant.entry(tenant_id).or_default().insert(user_id);
                }
            }
            GrantSubject::Role(role_id) => {
                for a in assignments.iter().filter(|a| a.role_id == role_id) {
                    if !active_users.contains(&a.user_id) {
                        continue;
                    }
                    if covers_system {
                        global.insert(a.user_id);
                    }
                    if covers_tenant {
                        per_tenant.entry(a.tenant_id).or_default().insert(a.user_id);
                    }
                }
            }
        }
    }
    (per_tenant, global)
}

/// Simulate the mutation's post-state over the in-memory copies.
fn apply_mutation(
    mutation: &GuardedMutation<'_>,
    grants: &mut Vec<ResolvedGrant>,
    assignments: &mut Vec<Assignment>,
    active_users: &mut HashSet<Uuid>,
) {
    match mutation {
        GuardedMutation::DeleteGrant { grant_id } => {
            grants.retain(|g| g.id != *grant_id);
        }
        GuardedMutation::UpdateGrant {
            grant_id,
            new_patterns,
            new_selector,
        } => {
            for g in grants.iter_mut().filter(|g| g.id == *grant_id) {
                g.patterns = new_patterns.to_vec();
                g.selector = (*new_selector).clone();
            }
        }
        GuardedMutation::DeleteRole { role_id } => {
            grants.retain(|g| g.subject != GrantSubject::Role(*role_id));
            assignments.retain(|a| a.role_id != *role_id);
        }
        GuardedMutation::SetUserRoles {
            tenant_id,
            user_id,
            new_role_ids,
        } => {
            assignments.retain(|a| !(a.tenant_id == *tenant_id && a.user_id == *user_id));
            assignments.extend(new_role_ids.iter().map(|role_id| Assignment {
                tenant_id: *tenant_id,
                user_id: *user_id,
                role_id: *role_id,
            }));
        }
        GuardedMutation::DeactivateUser { user_id } => {
            active_users.remove(user_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use sea_orm::{ConnectOptions, Database, DatabaseConnection, QueryFilter};
    use uptrakit_shared_types::MaskedEmail;
    use uptrakit_shared_types::access::bounds::MAX_GRANTS_PER_SUBJECT;

    use super::*;
    use crate::entity::{role, tenant, user, user_role};

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

    /// Insert an active user row; returns its id. Generates a UNIQUE email
    /// per call from a fresh `Uuid::now_v7()` — `users.email` carries
    /// `#[sea_orm(unique)]` and several tests below call this helper
    /// multiple times.
    async fn active_user(db: &DatabaseConnection) -> Uuid {
        let id = Uuid::now_v7();
        let now = OffsetDateTime::now_utc();
        user::ActiveModel {
            id: Set(id),
            email: Set(MaskedEmail::new(format!("{id}@lockout-guard.test"))),
            first_name: Set("Guard".to_string()),
            last_name: Set("Holder".to_string()),
            password_hash: Set(None),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .expect("insert active user");
        id
    }

    /// Insert a second, non-default tenant row. `user_role.tenant_id` and
    /// `access_grants.tenant_id` both carry an FK to `tenants.id`, so tests
    /// that need a genuinely separate tenant bucket (not a bare
    /// `Uuid::now_v7()`, which would fail the FK) go through this.
    async fn insert_tenant(db: &DatabaseConnection, slug: &str) -> Uuid {
        let id = Uuid::now_v7();
        let now = OffsetDateTime::now_utc();
        tenant::ActiveModel {
            id: Set(id),
            name: Set(slug.to_string()),
            slug: Set(slug.to_string()),
            is_default: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert second tenant");
        id
    }

    /// Test shim: run the guard on its own [`begin_guarded`] transaction
    /// (dropped ⇒ rolled back; the guard itself never mutates). This is
    /// the production call shape — `check_lockout` requires a real
    /// `DatabaseTransaction` by type.
    ///
    /// NEVER call while another transaction handle is alive: `test_db()`
    /// pools a SINGLE connection, so a second open txn starves the pool
    /// and the test hangs with no diagnostic.
    async fn verdict_of(
        db: &DatabaseConnection,
        sentinel: Uuid,
        mutation: &GuardedMutation<'_>,
    ) -> Result<LockoutVerdict> {
        let txn = begin_guarded(db).await?;
        check_lockout(&txn, sentinel, mutation).await
    }

    /// Assign `role_id` to `user_id` in `tenant`.
    async fn assign(db: &DatabaseConnection, tenant: Uuid, user_id: Uuid, role_id: Uuid) {
        user_role::ActiveModel {
            tenant_id: Set(tenant),
            user_id: Set(user_id),
            role_id: Set(role_id),
            assigned_at: Set(OffsetDateTime::now_utc()),
        }
        .insert(db)
        .await
        .expect("assign role");
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

    #[test]
    fn patterns_reach_system_plane_classifies_by_dotted_prefix() {
        assert!(!patterns_reach_system_plane(&pats(&["hosts:read"])).expect("tenant"));
        assert!(patterns_reach_system_plane(&pats(&["system.*:*"])).expect("system"));
        assert!(
            patterns_reach_system_plane(&pats(&["system.access:manage"])).expect("system exact")
        );
        // Plane mixing propagates the module's own error, never a silent pick.
        let err = patterns_reach_system_plane(&pats(&["hosts:read", "system.*:*"]))
            .expect_err("mixed planes must be rejected");
        assert!(
            matches!(err.current_context(), AccessGrantError::PlaneMixing),
            "expected PlaneMixing, got: {err}"
        );
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

    #[tokio::test]
    async fn list_grants_returns_tenant_and_global_rows_with_subject_filter() {
        let db = test_db().await;
        let tenant = default_tenant_id(&db).await;
        let user_a = Uuid::now_v7();
        let viewer = role_id(&db, "viewer").await;
        // one tenant-plane user grant + the seeded global role grants exist
        insert_grant(
            &db,
            NewGrant {
                subject: GrantSubject::User(user_a),
                tenant_id: Some(tenant),
                patterns: &pats(&["hosts:read"]),
                selector: Selector::All,
                description: Some("probe".to_string()),
                created_by: None,
            },
        )
        .await
        .expect("insert");

        let all = list_grants(&db, tenant, None).await.expect("list");
        assert!(
            all.grants
                .iter()
                .any(|g| g.subject == GrantSubject::User(user_a))
        );
        assert!(
            all.grants
                .iter()
                .any(|g| g.subject == GrantSubject::Role(viewer)),
            "global role rows must be included"
        );

        let filtered = list_grants(&db, tenant, Some(GrantSubject::User(user_a)))
            .await
            .expect("filtered");
        assert!(
            filtered
                .grants
                .iter()
                .all(|g| g.subject == GrantSubject::User(user_a))
        );
        assert_eq!(
            filtered
                .grants
                .first()
                .and_then(|g| g.description.as_deref()),
            Some("probe")
        );
    }

    #[tokio::test]
    async fn delete_grants_for_role_removes_only_that_roles_rows() {
        let db = test_db().await;
        let tenant = default_tenant_id(&db).await;
        let viewer = role_id(&db, "viewer").await;
        let operator = role_id(&db, "operator").await;

        let deleted = delete_grants_for_role(&db, viewer).await.expect("delete");
        assert!(deleted >= 1, "viewer seed grant should be deleted");

        let remaining = list_grants(&db, tenant, Some(GrantSubject::Role(viewer)))
            .await
            .expect("list viewer");
        assert!(remaining.grants.is_empty());
        let operator_rows = list_grants(&db, tenant, Some(GrantSubject::Role(operator)))
            .await
            .expect("list operator");
        assert!(
            !operator_rows.grants.is_empty(),
            "sibling role rows untouched"
        );
    }

    #[tokio::test]
    async fn deleting_last_tenant_covering_grant_is_tenant_lockout() {
        let db = test_db().await;
        let tenant = default_tenant_id(&db).await;
        let holder = active_user(&db).await;
        let grant_id = insert_grant(
            &db,
            NewGrant {
                subject: GrantSubject::User(holder),
                tenant_id: Some(tenant),
                patterns: &pats(&["access:manage"]),
                selector: Selector::All,
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("insert");

        let verdict = verdict_of(&db, tenant, &GuardedMutation::DeleteGrant { grant_id })
            .await
            .expect("guard");
        assert_eq!(verdict, LockoutVerdict::TenantLockout);
    }

    /// E4: each of the four tenant forms alone makes its holder count —
    /// deleting that grant is a lockout. A fresh `test_db()` per form keeps
    /// iterations isolated (this module's sibling tests each build their
    /// own `test_db()`, rather than sharing one across cases).
    #[tokio::test]
    async fn every_covering_pattern_form_counts() {
        for form in ["access:manage", "access:*", "*:manage", "*:*"] {
            let db = test_db().await;
            let tenant = default_tenant_id(&db).await;
            let holder = active_user(&db).await;
            let grant_id = insert_grant(
                &db,
                NewGrant {
                    subject: GrantSubject::User(holder),
                    tenant_id: Some(tenant),
                    patterns: &pats(&[form]),
                    selector: Selector::All,
                    description: None,
                    created_by: None,
                },
            )
            .await
            .expect("insert");
            let verdict = verdict_of(&db, tenant, &GuardedMutation::DeleteGrant { grant_id })
                .await
                .expect("guard");
            assert_eq!(
                verdict,
                LockoutVerdict::TenantLockout,
                "form {form} must cover"
            );
        }
    }

    /// Directly write a non-`All` selector row via the entity (the B9 gate
    /// makes `insert_grant` reject it), then verify a second, `All`
    /// covering grant's deletion is still a lockout — i.e. the non-`All`
    /// row did not count as surviving coverage.
    #[tokio::test]
    async fn non_all_selector_never_counts_as_holder() {
        let db = test_db().await;
        let tenant = default_tenant_id(&db).await;
        let non_all_holder = active_user(&db).await;
        let now = OffsetDateTime::now_utc();
        access_grant::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(Some(tenant)),
            subject_type: Set(GrantSubjectType::User),
            subject_id: Set(non_all_holder),
            patterns: Set(patterns_json(&pats(&["access:manage"]))),
            selector: Set(selector_json(&Selector::Hosts {
                ids: vec![Uuid::now_v7()],
            })
            .expect("encode selector")),
            description: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            created_by: Set(None),
        }
        .insert(&db)
        .await
        .expect("hand-insert non-All selector row");

        let covering_holder = active_user(&db).await;
        let grant_id = insert_grant(
            &db,
            NewGrant {
                subject: GrantSubject::User(covering_holder),
                tenant_id: Some(tenant),
                patterns: &pats(&["access:manage"]),
                selector: Selector::All,
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("insert covering grant");

        let verdict = verdict_of(&db, tenant, &GuardedMutation::DeleteGrant { grant_id })
            .await
            .expect("guard");
        assert_eq!(
            verdict,
            LockoutVerdict::TenantLockout,
            "the non-All selector row must never count as a surviving holder"
        );
    }

    #[tokio::test]
    async fn role_subject_coverage_and_per_tenant_grouping() {
        // A role-subject `access:manage` grant covers via assignment; the
        // invariant is PER TENANT: with the role assigned in the default
        // tenant only, deleting the role's grant is a lockout even though
        // a *different* tenant has no holders at all (vacuous there).
        let db = test_db().await;
        let tenant = default_tenant_id(&db).await;
        let holder = active_user(&db).await;
        let settings_manager = role_id(&db, "settings_manager").await; // seed grant covers access:manage
        assign(&db, tenant, holder, settings_manager).await;

        let verdict = verdict_of(
            &db,
            tenant,
            &GuardedMutation::DeleteRole {
                role_id: settings_manager,
            },
        )
        .await
        .expect("guard");
        assert_eq!(verdict, LockoutVerdict::TenantLockout);

        // Swap coverage: a second holder via user-subject grant → role
        // delete becomes Permitted.
        let second = active_user(&db).await;
        let second_grant_id = insert_grant(
            &db,
            NewGrant {
                subject: GrantSubject::User(second),
                tenant_id: Some(tenant),
                patterns: &pats(&["access:manage"]),
                selector: Selector::All,
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("insert");
        let verdict = verdict_of(
            &db,
            tenant,
            &GuardedMutation::DeleteRole {
                role_id: settings_manager,
            },
        )
        .await
        .expect("guard");
        assert_eq!(verdict, LockoutVerdict::Permitted);

        // PER-TENANT GROUPING, actually staged: a second, real tenant with
        // its OWN independent covering holder. If `covering_holders` ever
        // collapsed its `HashMap<Uuid, HashSet<Uuid>>` into one flat
        // `HashSet`, tenant B's holder would silently "rescue" tenant A's
        // count below (and vice versa), and both assertions here would flip
        // to `Permitted`.
        let tenant_b = insert_tenant(&db, "tenant-b-lockout-guard-test").await;
        let holder_b = active_user(&db).await;
        insert_grant(
            &db,
            NewGrant {
                subject: GrantSubject::User(holder_b),
                tenant_id: Some(tenant_b),
                patterns: &pats(&["access:manage"]),
                selector: Selector::All,
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("tenant b's own independent holder");

        // Drop tenant A's non-role holder so `holder`'s role assignment is
        // once again tenant A's only covering path.
        delete_grant(&db, second_grant_id)
            .await
            .expect("remove tenant A's second holder");

        let verdict = verdict_of(
            &db,
            tenant,
            &GuardedMutation::DeleteRole {
                role_id: settings_manager,
            },
        )
        .await
        .expect("guard");
        assert_eq!(
            verdict,
            LockoutVerdict::TenantLockout,
            "tenant A loses its only holder even though tenant B has its own, unrelated holder"
        );

        // Reverse direction: tenant B losing its only holder must ALSO be a
        // lockout, even though tenant A's holder (the role assignment,
        // still intact — the previous check never committed) is untouched.
        let verdict = verdict_of(
            &db,
            tenant,
            &GuardedMutation::DeactivateUser { user_id: holder_b },
        )
        .await
        .expect("guard");
        assert_eq!(
            verdict,
            LockoutVerdict::TenantLockout,
            "tenant B loses its only holder even though tenant A's holder is untouched"
        );
    }

    /// Swapping covering role A for covering role B in one full-replace
    /// request is Permitted (post-state evaluation, never per-removal);
    /// stripping every role is a lockout.
    #[tokio::test]
    async fn set_user_roles_evaluates_post_state_swap_is_legal() {
        let db = test_db().await;
        let tenant = default_tenant_id(&db).await;
        let holder = active_user(&db).await;
        let settings_manager = role_id(&db, "settings_manager").await; // seed grant covers access:manage
        let operator = role_id(&db, "operator").await;
        assign(&db, tenant, holder, settings_manager).await;
        insert_grant(
            &db,
            NewGrant {
                subject: GrantSubject::Role(operator),
                tenant_id: None,
                patterns: &pats(&["access:manage"]),
                selector: Selector::All,
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("operator covering grant");

        let swapped = verdict_of(
            &db,
            tenant,
            &GuardedMutation::SetUserRoles {
                tenant_id: tenant,
                user_id: holder,
                new_role_ids: &[operator],
            },
        )
        .await
        .expect("guard");
        assert_eq!(
            swapped,
            LockoutVerdict::Permitted,
            "swapping to another covering role is legal"
        );

        let stripped = verdict_of(
            &db,
            tenant,
            &GuardedMutation::SetUserRoles {
                tenant_id: tenant,
                user_id: holder,
                new_role_ids: &[],
            },
        )
        .await
        .expect("guard");
        assert_eq!(
            stripped,
            LockoutVerdict::TenantLockout,
            "stripping every role removes the last holder"
        );
    }

    #[tokio::test]
    async fn deactivating_last_system_holder_is_system_lockout_independent_of_tenant_plane() {
        // E6: global plane checked independently — tenant plane fully
        // covered by a second user, yet deactivating the only
        // system.*:*-holder is SystemLockout.
        let db = test_db().await;
        let tenant = default_tenant_id(&db).await;
        let sys_holder = active_user(&db).await;
        insert_grant(
            &db,
            NewGrant {
                subject: GrantSubject::User(sys_holder),
                tenant_id: None,
                patterns: &pats(&["system.*:*"]),
                selector: Selector::All,
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("system grant");
        let tenant_holder = active_user(&db).await;
        insert_grant(
            &db,
            NewGrant {
                subject: GrantSubject::User(tenant_holder),
                tenant_id: Some(tenant),
                patterns: &pats(&["access:manage"]),
                selector: Selector::All,
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("tenant grant");

        let verdict = verdict_of(
            &db,
            tenant,
            &GuardedMutation::DeactivateUser {
                user_id: sys_holder,
            },
        )
        .await
        .expect("guard");
        assert_eq!(verdict, LockoutVerdict::SystemLockout);
    }

    #[tokio::test]
    async fn both_planes_stripped_reports_system_lockout() {
        // Verdict precedence: one user is simultaneously the last tenant
        // access:manage holder AND the last system.*:* holder — deactivating
        // them must report SystemLockout (the less recoverable plane), not
        // TenantLockout.
        let db = test_db().await;
        let tenant = default_tenant_id(&db).await;
        let only_admin = active_user(&db).await;
        insert_grant(
            &db,
            NewGrant {
                subject: GrantSubject::User(only_admin),
                tenant_id: Some(tenant),
                patterns: &pats(&["access:manage"]),
                selector: Selector::All,
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("tenant grant");
        insert_grant(
            &db,
            NewGrant {
                subject: GrantSubject::User(only_admin),
                tenant_id: None,
                patterns: &pats(&["system.*:*"]),
                selector: Selector::All,
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("system grant");

        let verdict = verdict_of(
            &db,
            tenant,
            &GuardedMutation::DeactivateUser {
                user_id: only_admin,
            },
        )
        .await
        .expect("guard");
        assert_eq!(verdict, LockoutVerdict::SystemLockout);
    }

    #[tokio::test]
    async fn guard_then_mutate_then_commit_is_atomic_and_observable() {
        // Commit-path coverage in the fast tier (verdict_of always rolls
        // back; the E10 integration legs are #[ignore]d): guard Permitted →
        // mutate in the SAME transaction → commit → the mutation stuck and
        // a re-run guard sees the post-commit world.
        let db = test_db().await;
        let tenant = default_tenant_id(&db).await;
        let user_a = active_user(&db).await;
        let user_b = active_user(&db).await;
        let grant_a = insert_grant(
            &db,
            NewGrant {
                subject: GrantSubject::User(user_a),
                tenant_id: Some(tenant),
                patterns: &pats(&["access:manage"]),
                selector: Selector::All,
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("grant a");
        let grant_b = insert_grant(
            &db,
            NewGrant {
                subject: GrantSubject::User(user_b),
                tenant_id: Some(tenant),
                patterns: &pats(&["access:manage"]),
                selector: Selector::All,
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("grant b");

        let txn = begin_guarded(&db).await.expect("begin");
        let verdict = check_lockout(
            &txn,
            tenant,
            &GuardedMutation::DeleteGrant { grant_id: grant_a },
        )
        .await
        .expect("guard");
        assert_eq!(
            verdict,
            LockoutVerdict::Permitted,
            "a second holder remains"
        );
        delete_grant(&txn, grant_a)
            .await
            .expect("delete inside the guard txn");
        txn.commit().await.expect("commit");

        let err = load_grant(&db, grant_a)
            .await
            .expect_err("deletion committed");
        assert!(
            matches!(err.current_context(), AccessGrantError::NotFound),
            "expected NotFound after committed delete, got: {err}"
        );
        let verdict = verdict_of(
            &db,
            tenant,
            &GuardedMutation::DeleteGrant { grant_id: grant_b },
        )
        .await
        .expect("guard");
        assert_eq!(
            verdict,
            LockoutVerdict::TenantLockout,
            "post-commit world: b is now the last holder"
        );
    }

    /// E3's "narrow the covering pattern" kind, at guard level.
    #[tokio::test]
    async fn narrowing_covering_pattern_via_update_is_lockout() {
        let db = test_db().await;
        let tenant = default_tenant_id(&db).await;
        let holder = active_user(&db).await;
        let grant_id = insert_grant(
            &db,
            NewGrant {
                subject: GrantSubject::User(holder),
                tenant_id: Some(tenant),
                patterns: &pats(&["access:manage"]),
                selector: Selector::All,
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("insert");

        let narrowed = pats(&["hosts:read"]);
        let verdict = verdict_of(
            &db,
            tenant,
            &GuardedMutation::UpdateGrant {
                grant_id,
                new_patterns: &narrowed,
                new_selector: &Selector::All,
            },
        )
        .await
        .expect("guard");
        assert_eq!(verdict, LockoutVerdict::TenantLockout);

        let kept = pats(&["access:manage"]);
        let verdict = verdict_of(
            &db,
            tenant,
            &GuardedMutation::UpdateGrant {
                grant_id,
                new_patterns: &kept,
                new_selector: &Selector::All,
            },
        )
        .await
        .expect("guard");
        assert_eq!(verdict, LockoutVerdict::Permitted);
    }

    #[tokio::test]
    async fn missing_sentinel_row_is_a_hard_error() {
        let db = test_db().await;
        let err = verdict_of(
            &db,
            Uuid::now_v7(), // no such tenants row
            &GuardedMutation::DeactivateUser {
                user_id: Uuid::now_v7(),
            },
        )
        .await
        .expect_err("must be a hard error, never a pass-through");
        assert!(matches!(
            err.current_context(),
            AccessGrantError::SentinelMissing
        ));
    }

    #[test]
    fn covering_pattern_forms_are_the_closed_sets() {
        use uptrakit_shared_types::access::actions;
        // The guard's coverage predicate is ActionPattern::matches. Pin the
        // exact pattern forms that cover each guarded action, so a grammar
        // extension or resource rename fails here loudly (guard is the
        // dependent).
        let tenant_forms = ["access:manage", "access:*", "*:manage", "*:*"];
        let system_forms = [
            "system.access:manage",
            "system.access:*",
            "system.*:manage",
            "system.*:*",
        ];
        for f in tenant_forms {
            assert!(
                pat(f).matches(&actions::ACCESS_MANAGE),
                "{f} must cover tenant plane"
            );
            assert!(
                !pat(f).matches(&actions::SYSTEM_ACCESS_MANAGE),
                "{f} must not cover system"
            );
        }
        for f in system_forms {
            assert!(
                pat(f).matches(&actions::SYSTEM_ACCESS_MANAGE),
                "{f} must cover system plane"
            );
        }
        for f in [
            "access.sub:manage",
            "hosts:manage",
            "system:manage",
            "system.settings:manage",
        ] {
            assert!(
                !pat(f).matches(&actions::ACCESS_MANAGE),
                "{f} must not cover tenant"
            );
            assert!(
                !pat(f).matches(&actions::SYSTEM_ACCESS_MANAGE),
                "{f} must not cover system"
            );
        }
    }
}
