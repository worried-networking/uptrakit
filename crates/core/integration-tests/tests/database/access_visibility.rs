#![expect(
    clippy::expect_used,
    reason = "integration test code: panics are acceptable in test helpers (db_test! macro means functions are not annotated #[test])"
)]

use std::collections::BTreeSet;

use sea_orm::{ActiveModelTrait, DatabaseConnection, RelationTrait, Set};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{
    host, host_software_item, host_software_item_plugin, host_tag, host_tag_assignment,
    software_item, update_history,
};
use uptrakit_shared_db::{TenantDb, TenantDbVisibleExt};
use uptrakit_shared_types::access::Visibility;
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
