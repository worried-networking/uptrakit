//! Visibility-aware query builders: compile [`Visibility`] into sea_query
//! conditions appended to tenant-filtered selects.
//!
//! [`TenantDbVisibleExt`] extends [`TenantDb`] with `find_visible*` variants
//! of the inherent `find` family. The compiled row membership mirrors
//! `Selector::covers()` (`uptrakit-shared-types`) exactly for the `TargetRef`
//! each entity maps to: entities declaring fewer axes via [`HostScoped`] are
//! strictly deny-side — `covers()` may allow on an `Items`/`Software` grant
//! where `find_visible` returns nothing, never the reverse. Visibility only
//! ever narrows a tenant-scoped select; it never replaces tenant scoping.
//!
//! # Caller obligations
//!
//! - `AccessEngine::visibility` deliberately skips the dynamic-action
//!   registry gate: `find_visible` alone does not guard dynamic
//!   (`plugin.*` / `surface.*`) resources.
//! - Tag *assignments* resolve live in-query, but the tag ids inside a
//!   [`Visibility::Filter`] come from grants read through the engine's 60 s
//!   cache — a *grant* edit can lag up to 60 s. "Re-tag is immediate" must
//!   not be generalized to "visibility is immediate".
//! - `None` is a round-trip optimization, not the only nothing-visible
//!   shape: a `Some(select)` whose filter ids are all foreign or
//!   nonexistent yields zero rows. Callers must treat an empty result
//!   identically to `None`; the `Option` cannot prove completeness.
//!
//! # Bind-parameter assumption
//!
//! Axis unions are bounded by write-time validation (per-selector caps ×
//! `MAX_GRANTS_PER_SUBJECT = 200` grants): a pathological union is roughly
//! 20 000 ids per axis against SQLite's 32 766 bind-variable ceiling.
//! Unreachable at current deployment sizes, but any change to those bounds
//! must re-evaluate this margin rather than discover it as an opaque
//! execute-time database error.

use std::collections::BTreeSet;

use sea_orm::sea_query::{Condition, Expr, Query, SelectStatement};
use sea_orm::{ColumnTrait, ExprTrait, PrimaryKeyTrait, QueryFilter, RelationDef, Select};
use uptrakit_shared_types::access::Visibility;
use uptrakit_tenant_db::{HostScoped, TenantDb, TenantScoped};
use uuid::Uuid;

use crate::entity::{host_tag, host_tag_assignment};

/// Visibility-narrowed variants of the [`TenantDb`] `find` family.
///
/// Every method returns `Option<Select<_>>`: `None` ⇔ nothing is visible —
/// return an empty list / 404 without touching the database. The methods are
/// pure query builders: infallible, no `Result`.
pub trait TenantDbVisibleExt {
    /// Tenant-filtered select over `E`, narrowed to `visibility`.
    fn find_visible<E: TenantScoped + HostScoped>(
        &self,
        visibility: &Visibility,
    ) -> Option<Select<E>>;

    /// Single-row variant for 404-semantics sites. Generic over the
    /// primary-key value type, matching [`TenantDb::find_by_id`].
    fn find_visible_by_id<E, V>(&self, id: V, visibility: &Visibility) -> Option<Select<E>>
    where
        E: TenantScoped + HostScoped,
        V: Into<<E::PrimaryKey as PrimaryKeyTrait>::ValueType>;

    /// Visible variant of [`TenantDb::find_via_tenant_join`] for entities
    /// without a `tenant_id` column (`host_software_item`,
    /// `host_software_item_plugin`): tenant scoping via the join, visibility
    /// conditions on `Target`'s own columns.
    fn find_visible_via_tenant_join<Target, Scoped>(
        &self,
        relation: RelationDef,
        visibility: &Visibility,
    ) -> Option<Select<Target>>
    where
        Target: HostScoped,
        Scoped: TenantScoped;
}

impl TenantDbVisibleExt for TenantDb {
    fn find_visible<E: TenantScoped + HostScoped>(
        &self,
        visibility: &Visibility,
    ) -> Option<Select<E>> {
        match compile::<E>(self.tenant_id(), visibility) {
            Compiled::Unrestricted => Some(self.find::<E>()),
            Compiled::Narrowed(cond) => Some(self.find::<E>().filter(cond)),
            Compiled::Nothing => None,
        }
    }

    fn find_visible_by_id<E, V>(&self, id: V, visibility: &Visibility) -> Option<Select<E>>
    where
        E: TenantScoped + HostScoped,
        V: Into<<E::PrimaryKey as PrimaryKeyTrait>::ValueType>,
    {
        match compile::<E>(self.tenant_id(), visibility) {
            Compiled::Unrestricted => Some(self.find_by_id(id)),
            Compiled::Narrowed(cond) => Some(self.find_by_id(id).filter(cond)),
            Compiled::Nothing => None,
        }
    }

    fn find_visible_via_tenant_join<Target, Scoped>(
        &self,
        relation: RelationDef,
        visibility: &Visibility,
    ) -> Option<Select<Target>>
    where
        Target: HostScoped,
        Scoped: TenantScoped,
    {
        match compile::<Target>(self.tenant_id(), visibility) {
            Compiled::Unrestricted => Some(self.find_via_tenant_join::<Target, Scoped>(relation)),
            Compiled::Narrowed(cond) => Some(
                self.find_via_tenant_join::<Target, Scoped>(relation)
                    .filter(cond),
            ),
            Compiled::Nothing => None,
        }
    }
}

/// Outcome of compiling a [`Visibility`] for one entity.
enum Compiled {
    /// `Visibility::Full`: no extra condition.
    Unrestricted,
    /// `Visibility::Filter` with at least one contributing axis: AND this
    /// OR-of-axes condition onto the tenant-filtered select.
    Narrowed(Condition),
    /// Nothing is visible: skip the database round-trip entirely.
    Nothing,
}

fn compile<E: HostScoped>(tenant_id: Uuid, visibility: &Visibility) -> Compiled {
    match visibility {
        Visibility::Full => Compiled::Unrestricted,
        Visibility::None => Compiled::Nothing,
        Visibility::Filter {
            tags,
            hosts,
            software,
            items,
        } => {
            let mut cond = Condition::any();
            let mut contributed = false;
            if !hosts.is_empty() {
                cond = cond.add(E::host_id_column().is_in(hosts.iter().copied()));
                contributed = true;
            }
            if !tags.is_empty() {
                cond = cond
                    .add(E::host_id_column().in_subquery(tagged_host_subquery(tenant_id, tags)));
                contributed = true;
            }
            if !software.is_empty()
                && let Some(col) = E::software_item_id_column()
            {
                cond = cond.add(col.is_in(software.iter().copied()));
                contributed = true;
            }
            if !items.is_empty()
                && let Some(col) = E::host_software_item_id_column()
            {
                cond = cond.add(col.is_in(items.iter().copied()));
                contributed = true;
            }
            if contributed {
                Compiled::Narrowed(cond)
            } else {
                // The silent-empty-list support case ("user sees nothing,
                // grants look right") must be diagnosable from logs.
                tracing::debug!(
                    entity = E::default().table_name(),
                    software_populated = !software.is_empty(),
                    items_populated = !items.is_empty(),
                    "no visibility axis contributes a condition for this entity; nothing is visible"
                );
                Compiled::Nothing
            }
        }
        other => {
            // Source enum is `#[non_exhaustive]`; wildcard arm required. An
            // unknown visibility variant must never degrade to an
            // unrestricted select — deny, never a silent allow.
            tracing::warn!(
                ?other,
                "unhandled Visibility variant; treating as nothing-visible"
            );
            Compiled::Nothing
        }
    }
}

/// Uncorrelated subquery: host ids carrying at least one **active** tag from
/// `tag_ids`, tenant-scoped through `host_tags.tenant_id`. The
/// `deactivated_at IS NULL` filter keeps parity with decision-time
/// `load_host_tags` — a deactivated tag confers nothing on either path.
fn tagged_host_subquery(tenant_id: Uuid, tag_ids: &BTreeSet<Uuid>) -> SelectStatement {
    Query::select()
        .column((
            host_tag_assignment::Entity,
            host_tag_assignment::Column::HostId,
        ))
        .from(host_tag_assignment::Entity)
        .inner_join(
            host_tag::Entity,
            Expr::col((
                host_tag_assignment::Entity,
                host_tag_assignment::Column::HostTagId,
            ))
            .equals((host_tag::Entity, host_tag::Column::Id)),
        )
        .and_where(Expr::col((host_tag::Entity, host_tag::Column::TenantId)).eq(tenant_id))
        .and_where(Expr::col((host_tag::Entity, host_tag::Column::DeactivatedAt)).is_null())
        .and_where(
            Expr::col((
                host_tag_assignment::Entity,
                host_tag_assignment::Column::HostTagId,
            ))
            .is_in(tag_ids.iter().copied()),
        )
        .to_owned()
}

#[cfg(test)]
mod tests {
    use sea_orm::{
        ActiveModelTrait, ConnectOptions, Database, DatabaseConnection, RelationTrait, Set,
    };
    use time::OffsetDateTime;

    use super::*;
    use crate::entity::{host, host_software_item, software_item, tenant};

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
        use sea_orm::EntityTrait as _;
        tenant::Entity::find()
            .one(db)
            .await
            .expect("query tenants")
            .expect("default tenant is seeded")
            .id
    }

    async fn seed_host(db: &DatabaseConnection, tenant_id: Uuid) -> Uuid {
        let now = OffsetDateTime::now_utc();
        let host_id = Uuid::now_v7();
        host::ActiveModel {
            id: Set(host_id),
            tenant_id: Set(tenant_id),
            machine_id: Set(format!("machine-{host_id}")),
            hostname: Set(format!("host-{host_id}")),
            friendly_name: Set("Visibility Fixture".to_string()),
            os_type: Set(None),
            os_version: Set(None),
            architecture: Set(None),
            ip_address: Set(None),
            host_features: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert host");
        host_id
    }

    async fn seed_tag(db: &DatabaseConnection, tenant_id: Uuid, deactivated: bool) -> Uuid {
        let now = OffsetDateTime::now_utc();
        let tag_id = Uuid::now_v7();
        host_tag::ActiveModel {
            id: Set(tag_id),
            tenant_id: Set(tenant_id),
            name: Set(format!("tag-{tag_id}")),
            color: Set("#00aa00".to_string()),
            description: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(deactivated.then_some(now)),
        }
        .insert(db)
        .await
        .expect("insert host tag");
        tag_id
    }

    async fn assign_tag(db: &DatabaseConnection, tag_id: Uuid, host_id: Uuid) {
        host_tag_assignment::ActiveModel {
            host_tag_id: Set(tag_id),
            host_id: Set(host_id),
            assigned_at: Set(OffsetDateTime::now_utc()),
        }
        .insert(db)
        .await
        .expect("insert tag assignment");
    }

    async fn seed_software_item(db: &DatabaseConnection, tenant_id: Uuid) -> Uuid {
        let now = OffsetDateTime::now_utc();
        let software_item_id = Uuid::now_v7();
        software_item::ActiveModel {
            id: Set(software_item_id),
            tenant_id: Set(tenant_id),
            name: Set(format!("sw-{software_item_id}")),
            featured: Set(false),
            icon_url: Set(None),
            last_checked_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            awaiting_restart_timeout: Set(None),
        }
        .insert(db)
        .await
        .expect("insert software item");
        software_item_id
    }

    async fn seed_hsi(db: &DatabaseConnection, host_id: Uuid, software_item_id: Uuid) -> Uuid {
        let item_id = Uuid::now_v7();
        host_software_item::ActiveModel {
            id: Set(item_id),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            qualifier: Set(None),
            plugin_config_id: Set(None),
            package_identifier: Set(None),
            installed_version: Set(None),
            installed_version_detected_at: Set(None),
            installed_display_version: Set(None),
            latest_version: Set(None),
            latest_version_fetched_at: Set(None),
            latest_release_metadata: Set(None),
            last_updated_at: Set(None),
            linked_at: Set(OffsetDateTime::now_utc()),
            update_category: Set("unknown".to_string()),
            deactivated_at: Set(None),
            last_discovered_at: Set(None),
            missing_since: Set(None),
            discovery_source: Set(None),
        }
        .insert(db)
        .await
        .expect("insert host software item");
        item_id
    }

    fn ids(v: &[Uuid]) -> BTreeSet<Uuid> {
        v.iter().copied().collect()
    }

    fn filter(tags: &[Uuid], hosts: &[Uuid], software: &[Uuid], items: &[Uuid]) -> Visibility {
        Visibility::Filter {
            tags: ids(tags),
            hosts: ids(hosts),
            software: ids(software),
            items: ids(items),
        }
    }

    async fn host_ids(query: Select<host::Entity>, db: &DatabaseConnection) -> BTreeSet<Uuid> {
        query
            .all(db)
            .await
            .expect("query hosts")
            .into_iter()
            .map(|h| h.id)
            .collect()
    }

    #[tokio::test]
    async fn full_passthrough_returns_all_tenant_rows() {
        let db = test_db().await;
        let tenant_id = default_tenant_id(&db).await;
        let h1 = seed_host(&db, tenant_id).await;
        let h2 = seed_host(&db, tenant_id).await;
        let tdb = TenantDb::new(db, tenant_id);

        let query = tdb
            .find_visible::<host::Entity>(&Visibility::Full)
            .expect("Full is visible");
        assert_eq!(host_ids(query, tdb.db()).await, ids(&[h1, h2]));
    }

    #[tokio::test]
    async fn none_short_circuits_without_query() {
        let db = test_db().await;
        let tenant_id = default_tenant_id(&db).await;
        let tdb = TenantDb::new(db, tenant_id);

        assert!(
            tdb.find_visible::<host::Entity>(&Visibility::None)
                .is_none()
        );
        assert!(
            tdb.find_visible_by_id::<host::Entity, _>(Uuid::now_v7(), &Visibility::None)
                .is_none()
        );
    }

    #[tokio::test]
    async fn hosts_axis_includes_covered_and_excludes_bystanders() {
        let db = test_db().await;
        let tenant_id = default_tenant_id(&db).await;
        let covered = seed_host(&db, tenant_id).await;
        let _bystander = seed_host(&db, tenant_id).await;
        let tdb = TenantDb::new(db, tenant_id);

        let query = tdb
            .find_visible::<host::Entity>(&filter(&[], &[covered], &[], &[]))
            .expect("hosts axis contributes");
        assert_eq!(host_ids(query, tdb.db()).await, ids(&[covered]));
    }

    #[tokio::test]
    async fn tags_axis_matches_active_assignments_only() {
        let db = test_db().await;
        let tenant_id = default_tenant_id(&db).await;
        let tagged = seed_host(&db, tenant_id).await;
        let dead_tagged = seed_host(&db, tenant_id).await;
        let _untagged = seed_host(&db, tenant_id).await;
        let tag = seed_tag(&db, tenant_id, false).await;
        let dead_tag = seed_tag(&db, tenant_id, true).await;
        assign_tag(&db, tag, tagged).await;
        assign_tag(&db, dead_tag, dead_tagged).await;
        let tdb = TenantDb::new(db, tenant_id);

        let query = tdb
            .find_visible::<host::Entity>(&filter(&[tag, dead_tag], &[], &[], &[]))
            .expect("tags axis contributes");
        assert_eq!(host_ids(query, tdb.db()).await, ids(&[tagged]));
    }

    #[tokio::test]
    async fn empty_filter_yields_nothing_visible() {
        let db = test_db().await;
        let tenant_id = default_tenant_id(&db).await;
        let tdb = TenantDb::new(db, tenant_id);

        assert!(
            tdb.find_visible::<host::Entity>(&filter(&[], &[], &[], &[]))
                .is_none()
        );
    }

    #[tokio::test]
    async fn undeclared_axes_fail_closed() {
        let db = test_db().await;
        let tenant_id = default_tenant_id(&db).await;
        let _host = seed_host(&db, tenant_id).await;
        let tdb = TenantDb::new(db, tenant_id);

        // software/items axes are undeclared on `host`: an empty
        // Condition::any() must never degrade to an unrestricted select.
        assert!(
            tdb.find_visible::<host::Entity>(&filter(
                &[],
                &[],
                &[Uuid::now_v7()],
                &[Uuid::now_v7()]
            ))
            .is_none()
        );
    }

    #[tokio::test]
    async fn item_axes_distinguish_software_from_items_via_join() {
        let db = test_db().await;
        let tenant_id = default_tenant_id(&db).await;
        let host_id = seed_host(&db, tenant_id).await;
        let sw_a = seed_software_item(&db, tenant_id).await;
        let sw_b = seed_software_item(&db, tenant_id).await;
        let hsi_a = seed_hsi(&db, host_id, sw_a).await;
        let hsi_b = seed_hsi(&db, host_id, sw_b).await;
        let tdb = TenantDb::new(db, tenant_id);

        // Software axis covers row A only; row B is the bystander.
        let by_software = tdb
            .find_visible_via_tenant_join::<host_software_item::Entity, host::Entity>(
                host_software_item::Relation::Host.def(),
                &filter(&[], &[], &[sw_a], &[]),
            )
            .expect("software axis contributes");
        let got: BTreeSet<Uuid> = by_software
            .all(tdb.db())
            .await
            .expect("query items")
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(got, ids(&[hsi_a]));

        // Items axis covers row B only — the mirror direction. A swapped
        // HostScoped column mapping fails one of these two assertions.
        let by_items = tdb
            .find_visible_via_tenant_join::<host_software_item::Entity, host::Entity>(
                host_software_item::Relation::Host.def(),
                &filter(&[], &[], &[], &[hsi_b]),
            )
            .expect("items axis contributes");
        let got: BTreeSet<Uuid> = by_items
            .all(tdb.db())
            .await
            .expect("query items")
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(got, ids(&[hsi_b]));
    }

    #[tokio::test]
    async fn find_visible_by_id_narrows_to_filter() {
        let db = test_db().await;
        let tenant_id = default_tenant_id(&db).await;
        let inside = seed_host(&db, tenant_id).await;
        let outside = seed_host(&db, tenant_id).await;
        let vis = filter(&[], &[inside], &[], &[]);
        let tdb = TenantDb::new(db, tenant_id);

        let hit = tdb
            .find_visible_by_id::<host::Entity, _>(inside, &vis)
            .expect("hosts axis contributes")
            .one(tdb.db())
            .await
            .expect("query host");
        assert!(hit.is_some());

        let miss = tdb
            .find_visible_by_id::<host::Entity, _>(outside, &vis)
            .expect("hosts axis contributes")
            .one(tdb.db())
            .await
            .expect("query host");
        assert!(miss.is_none());
    }
}
