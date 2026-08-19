#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "functional test infrastructure: unwrap/expect acceptable in test helpers and assertions"
)]

mod support;

use sea_orm::sea_query::{Alias, Query};
use sea_orm::{ConnectionTrait, EntityTrait};
use uuid::Uuid;

async fn assert_cas_sentinel(db: &sea_orm::DatabaseConnection, update_history_id: Uuid) {
    let updated = uptrakit_shared_db::entity::update_history::Entity::find_by_id(update_history_id)
        .one(db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        updated.status,
        uptrakit_shared_db::entity::update_history::UpdateStatus::InProgress,
        "CAS Pending->InProgress failed: run_protection_and_dispatch exited early",
    );
}

#[tokio::test]
async fn setup_test_db_runs_core_and_proxmox_migrations() {
    let db = support::db::setup_test_db().await;
    // proxmox_host_mapping is a proxmox-side table; existence proves the
    // plugin migration ran. tenant is core-side; existence proves the core
    // migration ran. Both in one pass.
    let proxmox_host_mappings_probe = Query::select()
        .column(Alias::new("id"))
        .from(Alias::new("proxmox_host_mappings"))
        .limit(1)
        .to_owned();
    db.query_one(&proxmox_host_mappings_probe)
        .await
        .expect("proxmox_host_mappings table must exist");
    let tenants_probe = Query::select()
        .column(Alias::new("id"))
        .from(Alias::new("tenants"))
        .limit(1)
        .to_owned();
    db.query_one(&tenants_probe)
        .await
        .expect("tenants table must exist");
}

#[tokio::test]
async fn fixtures_insert_seeds_all_base_rows() {
    use sea_orm::EntityTrait;

    let db = support::db::setup_test_db().await;
    let fixtures = support::fixtures::TestFixtures::insert(&db, "http://127.0.0.1:9999").await;

    let tenant = uptrakit_shared_db::entity::tenant::Entity::find_by_id(fixtures.tenant_id)
        .one(&db)
        .await
        .unwrap();
    assert!(tenant.is_some(), "tenant row must exist");

    let history =
        uptrakit_shared_db::entity::update_history::Entity::find_by_id(fixtures.update_history_id)
            .one(&db)
            .await
            .unwrap();
    assert_eq!(
        history.unwrap().status,
        uptrakit_shared_db::entity::update_history::UpdateStatus::Pending,
        "update_history must start at Pending",
    );
}

#[tokio::test]
async fn pending_work_builder_matches_fixtures() {
    let db = support::db::setup_test_db().await;
    let fixtures = support::fixtures::TestFixtures::insert(&db, "http://127.0.0.1:9999").await;
    let work = fixtures.pending_work(&db, "2.0.0").await;
    assert_eq!(work.update_history_id, fixtures.update_history_id);
    assert_eq!(work.to_version, "2.0.0");
    assert!(!work.interactive);
    assert_eq!(work.target.item.id, fixtures.software_item_id);
    assert_eq!(work.target.host.id, fixtures.host_id);
}

#[tokio::test]
async fn build_plugin_ops_with_proxmox_returns_protection_and_hook() {
    let plugin_ops = support::stubs::build_plugin_ops(true);
    assert!(plugin_ops.controller_update_protection().is_some());
    assert!(plugin_ops.controller_update_hook().is_some());
}

#[tokio::test]
async fn build_plugin_ops_without_proxmox_returns_none() {
    let plugin_ops = support::stubs::build_plugin_ops(false);
    assert!(plugin_ops.controller_update_protection().is_none());
    assert!(plugin_ops.controller_update_hook().is_none());
}

#[tokio::test]
async fn cas_sentinel_passes_when_status_is_in_progress() {
    use sea_orm::{ActiveModelTrait, Set};
    use uptrakit_shared_db::entity::update_history;

    let db = support::db::setup_test_db().await;
    let fixtures = support::fixtures::TestFixtures::insert(&db, "http://127.0.0.1:9999").await;

    let row = update_history::Entity::find_by_id(fixtures.update_history_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let mut am: update_history::ActiveModel = row.into();
    am.status = Set(update_history::UpdateStatus::InProgress);
    am.update(&db).await.unwrap();

    assert_cas_sentinel(&db, fixtures.update_history_id).await;
}

#[tokio::test]
async fn snapshot_protection_and_scaling_before_dispatch() {
    use httpmock::prelude::*;
    use uptrakit_controller_core::testing::run_protection_and_dispatch;
    use uptrakit_plugin_infrastructure_proxmox::testing as proxmox_testing;
    use uptrakit_shared_db::entity::update_history;
    use uptrakit_wire::ControllerMessage;

    let server = MockServer::start_async().await;
    let task_status_mock = server
        .mock_async(|when, then| {
            when.method(GET)
                .path_includes("/tasks/")
                .path_includes("/status");
            then.status(200).json_body(serde_json::json!({
                "data": {"status": "stopped", "exitstatus": "OK"}
            }));
        })
        .await;
    let snapshot_mock = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/api2/json/nodes/pve1/qemu/100/snapshot")
                .body_includes("snapname=upk-test-software-")
                .body_includes("description=Uptrakit");
            then.status(200)
                .json_body(serde_json::json!({"data": "UPID:pve1:001:snapshot"}));
        })
        .await;
    let scale_get_mock = server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/api2/json/nodes/pve1/qemu/100/config");
            then.status(200).json_body(serde_json::json!({
                "data": {"cores": 2, "memory": 2048, "hotplug": "cpu,memory"}
            }));
        })
        .await;
    let scale_put_mock = server
        .mock_async(|when, then| {
            when.method(PUT)
                .path("/api2/json/nodes/pve1/qemu/100/config");
            then.status(200)
                .json_body(serde_json::json!({"data": null}));
        })
        .await;

    let db = support::db::setup_test_db().await;
    let fixtures = support::fixtures::TestFixtures::insert(&db, &server.base_url()).await;
    fixtures
        .insert_proxmox_mapping(&db, "pve1", 100, "qemu")
        .await;
    fixtures.insert_protection_default_snapshot(&db).await;
    fixtures.insert_scaling_default_delta(&db, 2, 1024).await;

    let mut notif = support::stubs::TestNotificationSetup::new(fixtures.service_id).await;
    let plugin_ops = support::stubs::build_plugin_ops(true);
    let work = fixtures.pending_work(&db, "2.0.0").await;

    run_protection_and_dispatch(
        db.clone(),
        notif.notification_state.clone(),
        std::sync::Arc::new(support::stubs::NoopOutputStream),
        plugin_ops,
        work,
    )
    .await;

    assert_cas_sentinel(&db, fixtures.update_history_id).await;

    snapshot_mock.assert_calls_async(1).await;
    task_status_mock.assert_calls_async(1).await;
    scale_get_mock.assert_calls_async(1).await;
    scale_put_mock.assert_calls_async(1).await;

    let msgs = notif.captured_messages();
    assert_eq!(msgs.len(), 1, "exactly one ExecuteUpdate dispatched");
    let payload = match &msgs[0] {
        ControllerMessage::ExecuteUpdate(p) => p,
        other => panic!("expected ExecuteUpdate, got {other:?}"),
    };
    assert_eq!(payload.to_version, "2.0.0");

    let hist = update_history::Entity::find_by_id(fixtures.update_history_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        hist.pre_update_protection_status,
        Some("protected".to_string()),
    );

    assert_eq!(proxmox_testing::count_scaling_records(&db).await, 1);
    let record = proxmox_testing::first_scaling_record(&db).await;
    assert_eq!(record.restore_status, "pending");
}

#[tokio::test]
async fn backup_protection_before_dispatch() {
    use httpmock::prelude::*;
    use uptrakit_controller_core::testing::run_protection_and_dispatch;
    use uptrakit_shared_db::entity::update_history;
    use uptrakit_wire::ControllerMessage;

    let server = MockServer::start_async().await;
    let task_status_mock = server
        .mock_async(|when, then| {
            when.method(GET)
                .path_includes("/tasks/")
                .path_includes("/status");
            then.status(200).json_body(serde_json::json!({
                "data": {"status": "stopped", "exitstatus": "OK"}
            }));
        })
        .await;
    let vzdump_mock = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/api2/json/nodes/pve1/vzdump")
                .body_includes("vmid=100")
                .body_includes("storage=storage1")
                .body_includes("notes-template=Uptrakit");
            then.status(200)
                .json_body(serde_json::json!({"data": "UPID:pve1:002:backup"}));
        })
        .await;

    let db = support::db::setup_test_db().await;
    let fixtures = support::fixtures::TestFixtures::insert(&db, &server.base_url()).await;
    fixtures
        .insert_proxmox_mapping(&db, "pve1", 100, "qemu")
        .await;
    fixtures
        .insert_protection_default_backup(&db, "pve1:storage1:dir")
        .await;
    fixtures
        .insert_backup_target_cache(&db, "pve1", "storage1", "dir", "pve1:storage1:dir")
        .await;

    let mut notif = support::stubs::TestNotificationSetup::new(fixtures.service_id).await;
    let plugin_ops = support::stubs::build_plugin_ops(true);
    let work = fixtures.pending_work(&db, "2.0.0").await;

    run_protection_and_dispatch(
        db.clone(),
        notif.notification_state.clone(),
        std::sync::Arc::new(support::stubs::NoopOutputStream),
        plugin_ops,
        work,
    )
    .await;

    assert_cas_sentinel(&db, fixtures.update_history_id).await;
    vzdump_mock.assert_calls_async(1).await;
    task_status_mock.assert_calls_async(1).await;

    let msgs = notif.captured_messages();
    assert_eq!(msgs.len(), 1);
    assert!(matches!(msgs[0], ControllerMessage::ExecuteUpdate(_)));

    let hist = update_history::Entity::find_by_id(fixtures.update_history_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        hist.pre_update_protection_status,
        Some("protected".to_string()),
    );
}

#[tokio::test]
async fn no_proxmox_mapping_dispatch_proceeds() {
    use httpmock::prelude::*;
    use uptrakit_controller_core::testing::run_protection_and_dispatch;
    use uptrakit_plugin_infrastructure_proxmox::testing as proxmox_testing;
    use uptrakit_wire::ControllerMessage;

    let server = MockServer::start_async().await;

    let catch_all = server
        .mock_async(|when, then| {
            when.path_includes("/");
            then.status(500);
        })
        .await;

    let db = support::db::setup_test_db().await;
    let fixtures = support::fixtures::TestFixtures::insert(&db, &server.base_url()).await;

    let mut notif = support::stubs::TestNotificationSetup::new(fixtures.service_id).await;
    let plugin_ops = support::stubs::build_plugin_ops(false);
    let work = fixtures.pending_work(&db, "2.0.0").await;

    run_protection_and_dispatch(
        db.clone(),
        notif.notification_state.clone(),
        std::sync::Arc::new(support::stubs::NoopOutputStream),
        plugin_ops,
        work,
    )
    .await;

    assert_cas_sentinel(&db, fixtures.update_history_id).await;
    catch_all.assert_calls_async(0).await;

    let msgs = notif.captured_messages();
    assert_eq!(msgs.len(), 1);
    assert!(matches!(msgs[0], ControllerMessage::ExecuteUpdate(_)));

    assert_eq!(
        proxmox_testing::count_protection_audits(&db).await,
        0,
        "no proxmox_protection_audit rows when no plugin runs",
    );
}

#[tokio::test]
async fn do_nothing_protection_scaling_still_runs() {
    use httpmock::prelude::*;
    use uptrakit_controller_core::testing::run_protection_and_dispatch;
    use uptrakit_plugin_infrastructure_proxmox::testing as proxmox_testing;
    use uptrakit_shared_db::entity::update_history;
    use uptrakit_wire::ControllerMessage;

    let server = MockServer::start_async().await;
    let scale_get_mock = server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/api2/json/nodes/pve1/qemu/100/config");
            then.status(200).json_body(serde_json::json!({
                "data": {"cores": 2, "memory": 2048, "hotplug": "cpu,memory"}
            }));
        })
        .await;
    let scale_put_mock = server
        .mock_async(|when, then| {
            when.method(PUT)
                .path("/api2/json/nodes/pve1/qemu/100/config");
            then.status(200)
                .json_body(serde_json::json!({"data": null}));
        })
        .await;

    let db = support::db::setup_test_db().await;
    let fixtures = support::fixtures::TestFixtures::insert(&db, &server.base_url()).await;
    fixtures
        .insert_proxmox_mapping(&db, "pve1", 100, "qemu")
        .await;
    fixtures.insert_protection_default_do_nothing(&db).await;
    fixtures.insert_scaling_default_absolute(&db, 4, 4096).await;

    let mut notif = support::stubs::TestNotificationSetup::new(fixtures.service_id).await;
    let plugin_ops = support::stubs::build_plugin_ops(true);
    let work = fixtures.pending_work(&db, "2.0.0").await;

    run_protection_and_dispatch(
        db.clone(),
        notif.notification_state.clone(),
        std::sync::Arc::new(support::stubs::NoopOutputStream),
        plugin_ops,
        work,
    )
    .await;

    assert_cas_sentinel(&db, fixtures.update_history_id).await;
    scale_get_mock.assert_calls_async(1).await;
    scale_put_mock.assert_calls_async(1).await;

    let msgs = notif.captured_messages();
    assert_eq!(msgs.len(), 1);
    assert!(matches!(msgs[0], ControllerMessage::ExecuteUpdate(_)));

    let hist = update_history::Entity::find_by_id(fixtures.update_history_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        hist.pre_update_protection_status,
        Some("skipped".to_string()),
    );

    assert_eq!(proxmox_testing::count_scaling_records(&db).await, 1);
}

#[tokio::test]
async fn dispatch_payload_has_correct_plugin_assignments() {
    use uptrakit_controller_core::testing::run_protection_and_dispatch;
    use uptrakit_wire::ControllerMessage;

    let db = support::db::setup_test_db().await;
    let fixtures = support::fixtures::TestFixtures::insert(&db, "http://127.0.0.1:9999").await;

    let mut notif = support::stubs::TestNotificationSetup::new(fixtures.service_id).await;
    let plugin_ops = support::stubs::build_plugin_ops(false);
    let work = fixtures.pending_work(&db, "2.0.0").await;

    run_protection_and_dispatch(
        db.clone(),
        notif.notification_state.clone(),
        std::sync::Arc::new(support::stubs::NoopOutputStream),
        plugin_ops,
        work,
    )
    .await;

    assert_cas_sentinel(&db, fixtures.update_history_id).await;

    let msgs = notif.captured_messages();
    assert_eq!(msgs.len(), 1);
    let payload = match &msgs[0] {
        ControllerMessage::ExecuteUpdate(p) => p,
        other => panic!("expected ExecuteUpdate, got {other:?}"),
    };

    assert_eq!(payload.to_version, "2.0.0");
    assert_eq!(payload.software_item_id, fixtures.software_item_id);

    let exec = &payload.execute_update_plugin;
    assert_eq!(exec.plugin_type, "generic.shell");
    assert_eq!(
        exec.config.get("update_command").and_then(|v| v.as_str()),
        Some("echo ok"),
        "execute_update assignment must carry shell config payload from fixtures",
    );

    let detect = payload
        .detect_version_plugin
        .as_ref()
        .expect("detect_version present");
    assert_eq!(detect.plugin_type, "generic.shell");
    assert_eq!(
        detect
            .config
            .get("version_command")
            .and_then(|v| v.as_str()),
        Some("echo 1.0.0"),
        "detect_version assignment must carry shell config payload from fixtures",
    );
}

#[tokio::test]
async fn post_update_resource_restore() {
    use httpmock::prelude::*;
    use uptrakit_plugin_infrastructure_proxmox::testing as proxmox_testing;
    use uptrakit_shared_db::entity::update_history;
    use uptrakit_web_api_queries::queries::update_dispatch::finalize_post_update_hook;

    let server = MockServer::start_async().await;
    let restore_mock = server
        .mock_async(|when, then| {
            when.method(PUT)
                .path("/api2/json/nodes/pve1/qemu/100/config")
                .body_includes("cores=2")
                .body_includes("memory=2048");
            then.status(200)
                .json_body(serde_json::json!({"data": null}));
        })
        .await;

    let db = support::db::setup_test_db().await;
    let fixtures = support::fixtures::TestFixtures::insert(&db, &server.base_url()).await;
    let mapping_id = fixtures
        .insert_proxmox_mapping(&db, "pve1", 100, "qemu")
        .await;

    proxmox_testing::insert_resource_scaling_record(
        &db,
        fixtures.tenant_id,
        fixtures.update_history_id,
        fixtures.host_id,
        fixtures.software_item_id,
        fixtures.proxmox_config_id,
        mapping_id,
        "qemu",
        2,
        2048,
        4,
        4096,
    )
    .await;

    let plugin_ops = support::stubs::build_plugin_ops(true);
    let hook = plugin_ops
        .controller_update_hook()
        .expect("proxmox update hook present");

    let record = update_history::Entity::find_by_id(fixtures.update_history_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();

    finalize_post_update_hook(&db, Some(hook), plugin_ops.as_ref(), &record)
        .await
        .expect("finalize hook ok");

    restore_mock.assert_calls_async(1).await;

    assert_eq!(proxmox_testing::count_scaling_records(&db).await, 1);
    let restored = proxmox_testing::first_scaling_record(&db).await;
    assert_eq!(restored.restore_status, "restored");
}
