#![expect(
    clippy::expect_used,
    reason = "test helpers: panics on setup failure are acceptable"
)]

use sea_orm::{ActiveModelTrait, Database, DatabaseConnection, Set};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{
    host, host_software_item, host_software_item_plugin, software_item, tenant,
};
use uuid::Uuid;

use crate::queries::host_software_item_plugins::plugin_types_for_role;
use crate::tenant_db::TenantDb;

const SKILL_PACKAGE: &str = "https://github.com/obra/superpowers#skills/brainstorming/SKILL.md";

/// `(db, tenant_a, tenant_b, hsi_a_id)`.
struct Fixture {
    db: DatabaseConnection,
    tenant_a: Uuid,
    tenant_b: Uuid,
    hsi_a_id: Uuid,
}

async fn bootstrap() -> Fixture {
    uptrakit_crypto::enable_plaintext_mode();

    let db = Database::connect("sqlite::memory:")
        .await
        .expect("connect sqlite");
    uptrakit_shared_db::migration::run_migrations(&db)
        .await
        .expect("migrations");

    let now = OffsetDateTime::now_utc();
    let tenant_a = Uuid::now_v7();
    let tenant_b = Uuid::now_v7();

    for (id, slug) in [(tenant_a, "tenant-a"), (tenant_b, "tenant-b")] {
        tenant::ActiveModel {
            id: Set(id),
            name: Set(slug.to_string()),
            slug: Set(slug.to_string()),
            is_default: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert tenant");
    }

    // Software items: one per tenant.
    let item_a = Uuid::now_v7();
    let item_b = Uuid::now_v7();
    for (id, tenant_id, name) in [
        (item_a, tenant_a, "skills-a"),
        (item_b, tenant_b, "skills-b"),
    ] {
        software_item::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant_id),
            name: Set(name.to_string()),
            featured: Set(false),
            icon_url: Set(None),
            last_checked_at: Set(None),
            awaiting_restart_timeout: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert software_item");
    }

    // Host under tenant A.
    let host_a = Uuid::now_v7();
    host::ActiveModel {
        id: Set(host_a),
        tenant_id: Set(tenant_a),
        machine_id: Set(format!("machine-{host_a}")),
        hostname: Set("host-a".to_string()),
        friendly_name: Set("Host A".to_string()),
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
    .insert(&db)
    .await
    .expect("insert host");

    // host_software_item under tenant A.
    let hsi_a_id = Uuid::now_v7();
    host_software_item::ActiveModel {
        id: Set(hsi_a_id),
        host_id: Set(host_a),
        software_item_id: Set(item_a),
        qualifier: Set(None),
        plugin_config_id: Set(None),
        package_identifier: Set(Some(SKILL_PACKAGE.to_string())),
        installed_version: Set(None),
        installed_version_detected_at: Set(None),
        installed_display_version: Set(None),
        latest_version: Set(None),
        latest_version_fetched_at: Set(None),
        latest_release_metadata: Set(None),
        last_updated_at: Set(None),
        linked_at: Set(now),
        update_category: Set("unknown".to_string()),
        deactivated_at: Set(None),
        last_discovered_at: Set(None),
        discovery_source: Set(None),
        missing_since: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert host_software_item");

    // host_software_item_plugin: role=detect_version, plugin_type=package_manager_skills.
    host_software_item_plugin::ActiveModel {
        id: Set(Uuid::now_v7()),
        host_id: Set(host_a),
        software_item_id: Set(item_a),
        host_software_item_id: Set(hsi_a_id),
        plugin_config_id: Set(None),
        plugin_type: Set("package_manager_skills".to_string()),
        role: Set("detect_version".to_string()),
        ordinal: Set(0),
        package_identifier: Set(SKILL_PACKAGE.to_string()),
        config: Set(None),
        execution_site: Set("auto".to_string()),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&db)
    .await
    .expect("insert host_software_item_plugin");

    Fixture {
        db,
        tenant_a,
        tenant_b,
        hsi_a_id,
    }
}

#[tokio::test]
async fn plugin_types_for_role_returns_assignment_for_detect_version() {
    let fx = bootstrap().await;
    let tenant_db = TenantDb::new(fx.db.clone(), fx.tenant_a);

    let out = plugin_types_for_role(&tenant_db, &[fx.hsi_a_id], "detect_version")
        .await
        .expect("ok");

    let assignment = out.get(&fx.hsi_a_id).expect("present");
    assert_eq!(assignment.plugin_type, "package_manager_skills");
    assert_eq!(assignment.package_identifier, SKILL_PACKAGE);
}

#[tokio::test]
async fn plugin_types_for_role_excludes_other_tenants() {
    let fx = bootstrap().await;
    let tenant_db = TenantDb::new(fx.db.clone(), fx.tenant_b);

    let out = plugin_types_for_role(&tenant_db, &[fx.hsi_a_id], "detect_version")
        .await
        .expect("ok");

    assert!(
        out.is_empty(),
        "tenant B must not see tenant A's host_software_item_plugin rows"
    );
}
