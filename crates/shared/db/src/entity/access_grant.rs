//! Engine-owned `access_grants` entity (`06-grant-model.md` §Storage schema).
//!
//! ENGINE-OWNED — no generic entity access. All reads and writes go through
//! [`crate::access_grants`]; never call `Entity::find()` (or any other
//! `EntityTrait` method) on this entity from anywhere else. The module is the
//! single validation choke point (rules 1–2, plane purity, the B9 selector
//! phase gate, bounds) and the single fail-closed JSON boundary. Enforced two
//! ways: `pub(crate)` visibility (cross-crate) and
//! `ci/verify_engine_owned_entities.sh` (in-crate).
//!
//! The table deliberately does NOT implement `TenantScoped`: it mixes tenant
//! rows with global (`tenant_id NULL`) rows, which breaks the trait's
//! non-null tenant filter contract. Tenant scoping is the query module's job.
//!
//! `subject_id` carries no FK — it is polymorphic across `users.id` /
//! `roles.id` by `subject_type`. `created_by` likewise has no FK (authors may
//! be deleted). Orphaned role-subject grants are inert (the role id appears
//! in no `user_roles` set) but M1.6a's role deletion must clean them up.

use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

/// Closed two-value subject discriminator. `DeriveActiveEnum` (TEXT storage)
/// per the fixed-string-enum rule — the CLOSED shape (`SystemServiceStatus`
/// precedent), deliberately not the plugin-extensible open-string shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub(crate) enum GrantSubjectType {
    #[sea_orm(string_value = "user")]
    User,
    #[sea_orm(string_value = "role")]
    Role,
}

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "access_grants")]
pub(crate) struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) id: Uuid,
    /// Single encoding per subject type (validation rule 2): system-plane
    /// grant ⇒ NULL (any subject); role subject ⇒ always NULL (scope comes
    /// from `roles.tenant_id` + `user_roles`); user-subject tenant-plane ⇒
    /// non-NULL.
    pub(crate) tenant_id: Option<Uuid>,
    pub(crate) subject_type: GrantSubjectType,
    pub(crate) subject_id: Uuid,
    /// JSON array of pattern strings; typed as `Vec<ActionPattern>` only at
    /// the query-module boundary (repo idiom: raw `serde_json::Value` on the
    /// entity, no `FromJsonQueryResult`).
    #[sea_orm(column_type = "JsonBinary")]
    pub(crate) patterns: serde_json::Value,
    /// `Selector` tagged JSON (`{"type":"all"}`, …); same boundary rule.
    #[sea_orm(column_type = "JsonBinary")]
    pub(crate) selector: serde_json::Value,
    pub(crate) description: Option<String>,
    pub(crate) created_at: OffsetDateTime,
    pub(crate) updated_at: OffsetDateTime,
    /// NULL for seed rows.
    pub(crate) created_by: Option<Uuid>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub(crate) enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
