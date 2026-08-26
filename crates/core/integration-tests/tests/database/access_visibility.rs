#![expect(
    clippy::expect_used,
    reason = "integration test code: panics are acceptable in test helpers (db_test! macro means functions are not annotated #[test])"
)]

use std::collections::BTreeSet;

use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, RelationTrait, Set,
};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{
    host, host_software_item, host_software_item_plugin, host_tag, host_tag_assignment,
    software_item, tenant, update_history,
};
use uptrakit_shared_db::{TenantDb, TenantDbVisibleExt};
use uptrakit_shared_types::access::{Selector, TargetRef, Visibility};
use uuid::Uuid;

use crate::database_helpers::fixtures::insert_host;
use crate::database_helpers::harness::TestHarness;
use crate::database_helpers::macros::db_test;

fn tenant_db(harness: &TestHarness) -> TenantDb {
    TenantDb::new(harness.db.clone(), harness.tenant_id)
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

async fn remove_tag(db: &DatabaseConnection, tag_id: Uuid, host_id: Uuid) {
    host_tag_assignment::Entity::delete_by_id((tag_id, host_id))
        .exec(db)
        .await
        .expect("delete tag assignment");
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

async fn seed_hsi_plugin(
    db: &DatabaseConnection,
    host_id: Uuid,
    software_item_id: Uuid,
    host_software_item_id: Uuid,
) -> Uuid {
    let now = OffsetDateTime::now_utc();
    let id = Uuid::now_v7();
    host_software_item_plugin::ActiveModel {
        id: Set(id),
        host_id: Set(host_id),
        software_item_id: Set(software_item_id),
        host_software_item_id: Set(host_software_item_id),
        plugin_config_id: Set(None),
        plugin_type: Set("generic-shell".to_string()),
        role: Set("detect_version".to_string()),
        ordinal: Set(0),
        package_identifier: Set(format!("pkg-{id}")),
        config: Set(None),
        execution_site: Set("auto".to_string()),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .expect("insert host software item plugin");
    id
}

async fn seed_update_history(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
    software_item_id: Uuid,
    host_software_item_id: Option<Uuid>,
) -> Uuid {
    let id = Uuid::now_v7();
    update_history::ActiveModel {
        id: Set(id),
        tenant_id: Set(tenant_id),
        host_id: Set(host_id),
        software_item_id: Set(software_item_id),
        host_software_item_id: Set(host_software_item_id),
        from_version: Set(None),
        to_version: Set(None),
        status: Set(update_history::UpdateStatus::Completed),
        output: Set(String::new()),
        output_bytes: Set(0),
        actor_type: Set("user".to_string()),
        actor_id: Set("visibility-test".to_string()),
        execution_owner_service_id: Set(None),
        execution_owner_instance_id: Set(None),
        started_at: Set(None),
        completed_at: Set(None),
        awaiting_restart_since: Set(None),
        created_at: Set(OffsetDateTime::now_utc()),
        update_category: Set("unknown".to_string()),
        batch_id: Set(None),
        interactive: Set(false),
        output_truncated: Set(false),
        pre_update_protection_status: Set(None),
        pre_update_protection_summary: Set(None),
        recovery_hint: Set(None),
        timeout_seconds: Set(None),
    }
    .insert(db)
    .await
    .expect("insert update history");
    id
}

async fn visible_host_ids(harness: &TestHarness, vis: &Visibility) -> Option<BTreeSet<Uuid>> {
    let tdb = tenant_db(harness);
    match tdb.find_visible::<host::Entity>(vis) {
        None => None,
        Some(query) => Some(
            query
                .all(tdb.db())
                .await
                .expect("query hosts")
                .into_iter()
                .map(|h| h.id)
                .collect(),
        ),
    }
}

async fn visible_hsi_ids(harness: &TestHarness, vis: &Visibility) -> Option<BTreeSet<Uuid>> {
    let tdb = tenant_db(harness);
    match tdb.find_visible_via_tenant_join::<host_software_item::Entity, host::Entity>(
        host_software_item::Relation::Host.def(),
        vis,
    ) {
        None => None,
        Some(query) => Some(
            query
                .all(tdb.db())
                .await
                .expect("query host software items")
                .into_iter()
                .map(|r| r.id)
                .collect(),
        ),
    }
}

async fn test_hosts_axis_on_host(harness: &TestHarness) {
    let covered = insert_host(&harness.db, harness.tenant_id).await.id;
    let _bystander = insert_host(&harness.db, harness.tenant_id).await.id;

    let got = visible_host_ids(harness, &filter(&[], &[covered], &[], &[]))
        .await
        .expect("hosts axis contributes");
    assert_eq!(got, ids(&[covered]));
}

db_test!(hosts_axis_on_host, test_hosts_axis_on_host);

async fn test_tags_axis_on_host(harness: &TestHarness) {
    let tagged = insert_host(&harness.db, harness.tenant_id).await.id;
    let _bystander = insert_host(&harness.db, harness.tenant_id).await.id;
    let tag = seed_tag(&harness.db, harness.tenant_id, false).await;
    assign_tag(&harness.db, tag, tagged).await;

    let got = visible_host_ids(harness, &filter(&[tag], &[], &[], &[]))
        .await
        .expect("tags axis contributes");
    assert_eq!(got, ids(&[tagged]));
}

db_test!(tags_axis_on_host, test_tags_axis_on_host);

async fn test_all_four_axes_on_host_software_item(harness: &TestHarness) {
    let h1 = insert_host(&harness.db, harness.tenant_id).await.id;
    let h2 = insert_host(&harness.db, harness.tenant_id).await.id;
    let sw_a = seed_software_item(&harness.db, harness.tenant_id).await;
    let sw_b = seed_software_item(&harness.db, harness.tenant_id).await;
    let hsi_a = seed_hsi(&harness.db, h1, sw_a).await;
    let hsi_b = seed_hsi(&harness.db, h2, sw_b).await;

    // hosts axis: h1's row only.
    let got = visible_hsi_ids(harness, &filter(&[], &[h1], &[], &[]))
        .await
        .expect("hosts axis contributes");
    assert_eq!(got, ids(&[hsi_a]));

    // tags axis: tag h2 only.
    let tag = seed_tag(&harness.db, harness.tenant_id, false).await;
    assign_tag(&harness.db, tag, h2).await;
    let got = visible_hsi_ids(harness, &filter(&[tag], &[], &[], &[]))
        .await
        .expect("tags axis contributes");
    assert_eq!(got, ids(&[hsi_b]));

    // Axis-distinguishable fixture: software axis covers sw_a (row A) while
    // the items axis covers hsi_b (row B) — a swapped HostScoped column
    // mapping fails one of the two directions.
    let got = visible_hsi_ids(harness, &filter(&[], &[], &[sw_a], &[]))
        .await
        .expect("software axis contributes");
    assert_eq!(got, ids(&[hsi_a]));

    let got = visible_hsi_ids(harness, &filter(&[], &[], &[], &[hsi_b]))
        .await
        .expect("items axis contributes");
    assert_eq!(got, ids(&[hsi_b]));
}

db_test!(
    all_four_axes_on_host_software_item,
    test_all_four_axes_on_host_software_item
);

async fn test_host_axes_on_update_history(harness: &TestHarness) {
    let h1 = insert_host(&harness.db, harness.tenant_id).await.id;
    let h2 = insert_host(&harness.db, harness.tenant_id).await.id;
    let sw = seed_software_item(&harness.db, harness.tenant_id).await;
    let covered = seed_update_history(&harness.db, harness.tenant_id, h1, sw, None).await;
    let _bystander = seed_update_history(&harness.db, harness.tenant_id, h2, sw, None).await;

    let tdb = tenant_db(harness);
    let rows = tdb
        .find_visible::<update_history::Entity>(&filter(&[], &[h1], &[], &[]))
        .expect("hosts axis contributes")
        .all(tdb.db())
        .await
        .expect("query update history");
    let got: BTreeSet<Uuid> = rows.into_iter().map(|r| r.id).collect();
    assert_eq!(got, ids(&[covered]));
}

db_test!(
    host_axes_on_update_history,
    test_host_axes_on_update_history
);

async fn test_host_axes_on_host_software_item_plugin(harness: &TestHarness) {
    let h1 = insert_host(&harness.db, harness.tenant_id).await.id;
    let h2 = insert_host(&harness.db, harness.tenant_id).await.id;
    let sw = seed_software_item(&harness.db, harness.tenant_id).await;
    let hsi_1 = seed_hsi(&harness.db, h1, sw).await;
    let hsi_2 = seed_hsi(&harness.db, h2, sw).await;
    let covered = seed_hsi_plugin(&harness.db, h1, sw, hsi_1).await;
    let bystander = seed_hsi_plugin(&harness.db, h2, sw, hsi_2).await;

    let tdb = tenant_db(harness);
    let rows = tdb
        .find_visible_via_tenant_join::<host_software_item_plugin::Entity, host::Entity>(
            host_software_item_plugin::Relation::Host.def(),
            &filter(&[], &[h1], &[], &[]),
        )
        .expect("hosts axis contributes")
        .all(tdb.db())
        .await
        .expect("query plugins");
    let got: BTreeSet<Uuid> = rows.into_iter().map(|r| r.id).collect();
    assert_eq!(got, ids(&[covered]));

    // tags axis on the plugin's host: tag h2, expect only h2's plugin row.
    let tag = seed_tag(&harness.db, harness.tenant_id, false).await;
    assign_tag(&harness.db, tag, h2).await;
    let rows = tdb
        .find_visible_via_tenant_join::<host_software_item_plugin::Entity, host::Entity>(
            host_software_item_plugin::Relation::Host.def(),
            &filter(&[tag], &[], &[], &[]),
        )
        .expect("tags axis contributes")
        .all(tdb.db())
        .await
        .expect("query plugins by tag");
    let got: BTreeSet<Uuid> = rows.into_iter().map(|r| r.id).collect();
    assert_eq!(got, ids(&[bystander]));
}

db_test!(
    host_axes_on_host_software_item_plugin,
    test_host_axes_on_host_software_item_plugin
);

async fn test_or_composition_returns_union(harness: &TestHarness) {
    let by_host = insert_host(&harness.db, harness.tenant_id).await.id;
    let by_tag = insert_host(&harness.db, harness.tenant_id).await.id;
    let _bystander = insert_host(&harness.db, harness.tenant_id).await.id;
    let tag = seed_tag(&harness.db, harness.tenant_id, false).await;
    assign_tag(&harness.db, tag, by_tag).await;

    let got = visible_host_ids(harness, &filter(&[tag], &[by_host], &[], &[]))
        .await
        .expect("both axes contribute");
    assert_eq!(got, ids(&[by_host, by_tag]));
}

db_test!(
    or_composition_returns_union,
    test_or_composition_returns_union
);

async fn test_full_passthrough_matches_plain_find(harness: &TestHarness) {
    let h1 = insert_host(&harness.db, harness.tenant_id).await.id;
    let h2 = insert_host(&harness.db, harness.tenant_id).await.id;

    let got = visible_host_ids(harness, &Visibility::Full)
        .await
        .expect("Full is visible");
    let tdb = tenant_db(harness);
    let plain: BTreeSet<Uuid> = tdb
        .find::<host::Entity>()
        .all(tdb.db())
        .await
        .expect("plain find")
        .into_iter()
        .map(|h| h.id)
        .collect();
    assert_eq!(got, plain);
    assert!(got.is_superset(&ids(&[h1, h2])));
}

db_test!(
    full_passthrough_matches_plain_find,
    test_full_passthrough_matches_plain_find
);

async fn test_none_short_circuits(harness: &TestHarness) {
    let _seeded = insert_host(&harness.db, harness.tenant_id).await.id;
    assert!(visible_host_ids(harness, &Visibility::None).await.is_none());
}

db_test!(none_short_circuits, test_none_short_circuits);

async fn test_no_contributing_axis_yields_none(harness: &TestHarness) {
    let _seeded = insert_host(&harness.db, harness.tenant_id).await.id;
    // software axis undeclared on `host` ⇒ nothing contributes ⇒ None.
    assert!(
        visible_host_ids(harness, &filter(&[], &[], &[Uuid::now_v7()], &[]))
            .await
            .is_none()
    );
}

db_test!(
    no_contributing_axis_yields_none,
    test_no_contributing_axis_yields_none
);

async fn test_deactivated_tag_confers_nothing(harness: &TestHarness) {
    let tagged = insert_host(&harness.db, harness.tenant_id).await.id;
    let tag = seed_tag(&harness.db, harness.tenant_id, true).await;
    assign_tag(&harness.db, tag, tagged).await;
    let vis = filter(&[tag], &[], &[], &[]);

    let got = visible_host_ids(harness, &vis)
        .await
        .expect("tags axis contributes");
    assert!(got.is_empty(), "deactivated tag must confer nothing");

    // Reactivation restores visibility immediately.
    host_tag::Entity::update_many()
        .col_expr(
            host_tag::Column::DeactivatedAt,
            sea_orm::sea_query::Expr::value(None::<OffsetDateTime>),
        )
        .filter(host_tag::Column::Id.eq(tag))
        .exec(&harness.db)
        .await
        .expect("reactivate tag");
    let got = visible_host_ids(harness, &vis)
        .await
        .expect("tags axis contributes");
    assert_eq!(got, ids(&[tagged]));
}

db_test!(
    deactivated_tag_confers_nothing,
    test_deactivated_tag_confers_nothing
);

async fn test_retag_reflected_immediately(harness: &TestHarness) {
    let h1 = insert_host(&harness.db, harness.tenant_id).await.id;
    let h2 = insert_host(&harness.db, harness.tenant_id).await.id;
    let tag = seed_tag(&harness.db, harness.tenant_id, false).await;
    assign_tag(&harness.db, tag, h1).await;
    let vis = filter(&[tag], &[], &[], &[]);

    let got = visible_host_ids(harness, &vis)
        .await
        .expect("tags axis contributes");
    assert_eq!(got, ids(&[h1]));

    remove_tag(&harness.db, tag, h1).await;
    assign_tag(&harness.db, tag, h2).await;

    let got = visible_host_ids(harness, &vis)
        .await
        .expect("tags axis contributes");
    assert_eq!(got, ids(&[h2]));
}

db_test!(
    retag_reflected_immediately,
    test_retag_reflected_immediately
);

async fn seed_foreign_tenant(db: &DatabaseConnection) -> Uuid {
    let id = Uuid::now_v7();
    let now = OffsetDateTime::now_utc();
    tenant::ActiveModel {
        id: Set(id),
        name: Set(format!("foreign-{id}")),
        slug: Set(id.to_string()),
        is_default: Set(false),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    }
    .insert(db)
    .await
    .expect("insert foreign tenant");
    id
}

async fn test_foreign_tenant_ids_yield_nothing(harness: &TestHarness) {
    let foreign_tenant = seed_foreign_tenant(&harness.db).await;
    let foreign_host = insert_host(&harness.db, foreign_tenant).await.id;
    let foreign_tag = seed_tag(&harness.db, foreign_tenant, false).await;
    assign_tag(&harness.db, foreign_tag, foreign_host).await;
    let foreign_sw = seed_software_item(&harness.db, foreign_tenant).await;
    let foreign_hsi = seed_hsi(&harness.db, foreign_host, foreign_sw).await;
    let own = insert_host(&harness.db, harness.tenant_id).await.id;
    // Cross-tenant tag trust pin: a foreign-tenant tag assigned to an IN-tenant
    // host (FK-legal — host_tag_assignments has no tenant column) must confer
    // nothing. Without tagged_host_subquery's host_tags.tenant_id predicate,
    // `own` would leak; the outer tenant filter cannot catch it (own is in-tenant).
    assign_tag(&harness.db, foreign_tag, own).await;

    let got = visible_host_ids(harness, &filter(&[foreign_tag], &[foreign_host], &[], &[]))
        .await
        .expect("axes contribute");
    assert!(
        got.is_empty(),
        "foreign host/tag ids must yield nothing — a non-empty set means the \
         tag subquery trusted a foreign tenant's tag id"
    );

    let got = visible_hsi_ids(harness, &filter(&[], &[], &[foreign_sw], &[foreign_hsi]))
        .await
        .expect("axes contribute");
    assert!(got.is_empty(), "foreign item ids must yield nothing");

    let got = visible_host_ids(harness, &filter(&[], &[own], &[], &[]))
        .await
        .expect("host axis contributes");
    assert_eq!(got, BTreeSet::from([own]));
}

db_test!(
    foreign_tenant_ids_yield_nothing,
    test_foreign_tenant_ids_yield_nothing
);

async fn test_undeclared_axes_fail_closed(harness: &TestHarness) {
    let h = insert_host(&harness.db, harness.tenant_id).await.id;
    let sw = seed_software_item(&harness.db, harness.tenant_id).await;
    let hsi = seed_hsi(&harness.db, h, sw).await;
    let _uh = seed_update_history(&harness.db, harness.tenant_id, h, sw, Some(hsi)).await;
    let _plugin = seed_hsi_plugin(&harness.db, h, sw, hsi).await;
    let vis = filter(&[], &[], &[sw], &[]);
    let tdb = tenant_db(harness);

    assert!(
        tdb.find_visible::<update_history::Entity>(&vis).is_none(),
        "update_history must not honor the software axis"
    );
    assert!(
        tdb.find_visible_via_tenant_join::<host_software_item_plugin::Entity, host::Entity>(
            host_software_item_plugin::Relation::Host.def(),
            &vis,
        )
        .is_none(),
        "host_software_item_plugin must not honor the software axis"
    );
}

db_test!(
    undeclared_axes_fail_closed,
    test_undeclared_axes_fail_closed
);

/// Resolve a host's active tag ids the way decision-time `load_host_tags`
/// does: assignments joined to same-tenant, non-deactivated tags.
async fn active_host_tags(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
) -> BTreeSet<Uuid> {
    use sea_orm::{JoinType, QuerySelect, RelationTrait};
    host_tag_assignment::Entity::find()
        .join(
            JoinType::InnerJoin,
            host_tag_assignment::Relation::HostTag.def(),
        )
        .filter(host_tag::Column::TenantId.eq(tenant_id))
        .filter(host_tag::Column::DeactivatedAt.is_null())
        .filter(host_tag_assignment::Column::HostId.eq(host_id))
        .all(db)
        .await
        .expect("load host tags")
        .into_iter()
        .map(|a| a.host_tag_id)
        .collect()
}

async fn test_parity_with_covers(harness: &TestHarness) {
    let h_tagged = insert_host(&harness.db, harness.tenant_id).await.id;
    let h_direct = insert_host(&harness.db, harness.tenant_id).await.id;
    let h_bystander = insert_host(&harness.db, harness.tenant_id).await.id;
    let tag = seed_tag(&harness.db, harness.tenant_id, false).await;
    assign_tag(&harness.db, tag, h_tagged).await;
    let sw_a = seed_software_item(&harness.db, harness.tenant_id).await;
    let sw_b = seed_software_item(&harness.db, harness.tenant_id).await;
    let hsi_a = seed_hsi(&harness.db, h_bystander, sw_a).await;
    let hsi_b = seed_hsi(&harness.db, h_bystander, sw_b).await;
    let sw_c = seed_software_item(&harness.db, harness.tenant_id).await;
    let hsi_c = seed_hsi(&harness.db, h_bystander, sw_c).await;
    let uh_1 = seed_update_history(&harness.db, harness.tenant_id, h_tagged, sw_a, None).await;
    let uh_2 = seed_update_history(&harness.db, harness.tenant_id, h_bystander, sw_a, None).await;

    let selectors = [
        Selector::Hosts {
            ids: vec![h_direct],
        },
        Selector::Tags { ids: vec![tag] },
        Selector::Software { ids: vec![sw_a] },
        Selector::Items { ids: vec![hsi_b] },
    ];
    let vis = Visibility::from_selectors(selectors.iter());
    let covered = |target: &TargetRef, tags: &BTreeSet<Uuid>| {
        selectors.iter().any(|s| s.covers(target, tags))
    };

    let visible = visible_host_ids(harness, &vis)
        .await
        .expect("filter contributes");
    for host_id in [h_tagged, h_direct, h_bystander] {
        let tags = active_host_tags(&harness.db, harness.tenant_id, host_id).await;
        assert_eq!(
            visible.contains(&host_id),
            covered(&TargetRef::Host(host_id), &tags),
            "host {host_id} parity"
        );
    }

    let visible = visible_hsi_ids(harness, &vis)
        .await
        .expect("filter contributes");
    for (id, host_id, software_item_id) in [
        (hsi_a, h_bystander, sw_a),
        (hsi_b, h_bystander, sw_b),
        (hsi_c, h_bystander, sw_c),
    ] {
        let tags = active_host_tags(&harness.db, harness.tenant_id, host_id).await;
        let target = TargetRef::HostSoftwareItem {
            id,
            host_id,
            software_item_id,
        };
        assert_eq!(
            visible.contains(&id),
            covered(&target, &tags),
            "host_software_item {id} parity"
        );
    }

    let tdb = tenant_db(harness);
    let visible: BTreeSet<Uuid> = tdb
        .find_visible::<update_history::Entity>(&vis)
        .expect("filter contributes")
        .all(tdb.db())
        .await
        .expect("query update history")
        .into_iter()
        .map(|r| r.id)
        .collect();
    for (id, host_id) in [(uh_1, h_tagged), (uh_2, h_bystander)] {
        let tags = active_host_tags(&harness.db, harness.tenant_id, host_id).await;
        assert_eq!(
            visible.contains(&id),
            covered(&TargetRef::Host(host_id), &tags),
            "update_history {id} parity"
        );
    }
}

db_test!(parity_with_covers, test_parity_with_covers);

async fn test_find_visible_by_id_outcomes(harness: &TestHarness) {
    let inside = insert_host(&harness.db, harness.tenant_id).await.id;
    let outside = insert_host(&harness.db, harness.tenant_id).await.id;
    let tdb = tenant_db(harness);

    assert!(
        tdb.find_visible_by_id::<host::Entity, _>(outside, &Visibility::None)
            .is_none()
    );
    assert!(
        tdb.find_visible_by_id::<host::Entity, _>(outside, &filter(&[], &[], &[outside], &[]))
            .is_none(),
        "undeclared-axis-only filter must be None, not an open select"
    );

    let vis = filter(&[], &[inside], &[], &[]);
    let miss = tdb
        .find_visible_by_id::<host::Entity, _>(outside, &vis)
        .expect("hosts axis contributes")
        .one(tdb.db())
        .await
        .expect("query host");
    assert!(miss.is_none());
    let hit = tdb
        .find_visible_by_id::<host::Entity, _>(inside, &vis)
        .expect("hosts axis contributes")
        .one(tdb.db())
        .await
        .expect("query host");
    assert!(hit.is_some());
}

db_test!(
    find_visible_by_id_outcomes,
    test_find_visible_by_id_outcomes
);
