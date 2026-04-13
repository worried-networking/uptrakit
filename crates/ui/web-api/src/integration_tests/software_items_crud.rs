use crate::test_harness::TestApp;
use crate::test_harness::fixtures::register_and_get_token;
use http_body_util::BodyExt;
use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use uptrakit_shared_db::entity::{
    host, host_software_item, host_software_item_plugin, software_item, user,
};
use uptrakit_web_api_types::permissions::Permission;
use uuid::Uuid;

async fn insert_software_item(app: &TestApp, item_id: Uuid, name: &str) -> software_item::Model {
    let now = time::OffsetDateTime::now_utc();
    software_item::ActiveModel {
        id: Set(item_id),
        tenant_id: Set(app.tenant_id),
        name: Set(name.to_string()),
        featured: Set(false),
        icon_url: Set(None),
        last_checked_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    }
    .insert(&app.db)
    .await
    .expect("insert software item")
}

async fn insert_active_user(app: &TestApp, user_id: Uuid, email: &str) -> user::Model {
    let now = time::OffsetDateTime::now_utc();
    user::ActiveModel {
        id: Set(user_id),
        email: Set(email.parse().expect("valid email")),
        first_name: Set("Test".to_string()),
        last_name: Set("Editor".to_string()),
        password_hash: Set(None),
        is_active: Set(true),
        deactivated_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&app.db)
    .await
    .expect("insert active user")
}

async fn insert_host_for_merge_test(app: &TestApp, host_id: Uuid, hostname: &str) -> host::Model {
    let now = time::OffsetDateTime::now_utc();
    host::ActiveModel {
        id: Set(host_id),
        tenant_id: Set(app.tenant_id),
        machine_id: Set(host_id.to_string()),
        hostname: Set(hostname.to_string()),
        friendly_name: Set(hostname.to_string()),
        os_type: Set(Some("linux".to_string())),
        os_version: Set(None),
        architecture: Set(None),
        ip_address: Set(None),
        host_features: Set(None),
        last_seen_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    }
    .insert(&app.db)
    .await
    .expect("insert host")
}

async fn insert_host_link(
    app: &TestApp,
    link_id: Uuid,
    host_id: Uuid,
    software_item_id: Uuid,
    qualifier: Option<&str>,
) -> host_software_item::Model {
    let now = time::OffsetDateTime::now_utc();
    host_software_item::ActiveModel {
        id: Set(link_id),
        host_id: Set(host_id),
        software_item_id: Set(software_item_id),
        qualifier: Set(qualifier.map(str::to_string)),
        plugin_config_id: Set(None),
        package_identifier: Set(None),
        installed_version: Set(Some("1.0.0".to_string())),
        installed_version_detected_at: Set(None),
        installed_display_version: Set(None),
        latest_version: Set(Some("1.1.0".to_string())),
        latest_version_fetched_at: Set(None),
        latest_release_metadata: Set(None),
        last_updated_at: Set(None),
        linked_at: Set(now),
        update_category: Set("unknown".to_string()),
        deactivated_at: Set(None),
    }
    .insert(&app.db)
    .await
    .expect("insert host link")
}

async fn insert_plugin_row(
    app: &TestApp,
    plugin_row_id: Uuid,
    host_id: Uuid,
    software_item_id: Uuid,
    host_software_item_id: Uuid,
    role: &str,
    ordinal: i32,
) -> host_software_item_plugin::Model {
    insert_plugin_row_with_details(
        app,
        plugin_row_id,
        host_id,
        software_item_id,
        host_software_item_id,
        "package_manager_apt",
        role,
        ordinal,
        None,
        "pkg",
        None,
        "auto",
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn insert_plugin_row_with_details(
    app: &TestApp,
    plugin_row_id: Uuid,
    host_id: Uuid,
    software_item_id: Uuid,
    host_software_item_id: Uuid,
    plugin_type: &str,
    role: &str,
    ordinal: i32,
    plugin_config_id: Option<Uuid>,
    package_identifier: &str,
    config: Option<serde_json::Value>,
    execution_site: &str,
) -> host_software_item_plugin::Model {
    let now = time::OffsetDateTime::now_utc();
    host_software_item_plugin::ActiveModel {
        id: Set(plugin_row_id),
        host_id: Set(host_id),
        software_item_id: Set(software_item_id),
        host_software_item_id: Set(host_software_item_id),
        plugin_config_id: Set(plugin_config_id),
        plugin_type: Set(plugin_type.to_string()),
        role: Set(role.to_string()),
        ordinal: Set(ordinal),
        package_identifier: Set(package_identifier.to_string()),
        config: Set(config),
        execution_site: Set(execution_site.to_string()),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&app.db)
    .await
    .expect("insert plugin row")
}

async fn read_json_response(
    response: http::Response<axum::body::Body>,
) -> (http::StatusCode, serde_json::Value) {
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let body = serde_json::from_slice(&bytes).unwrap_or_else(|err| {
        panic!(
            "failed to deserialize JSON response: {err}\nbody: {}",
            String::from_utf8_lossy(&bytes)
        )
    });
    (status, body)
}

#[tokio::test]
async fn create_returns_201() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/software-items",
            &serde_json::json!({ "name": "My App" }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::CREATED);
    assert_eq!(body["name"], "My App");
    assert!(body["id"].as_str().is_some());
}

#[tokio::test]
async fn list_returns_created() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    // Create an item.
    client
        .post_json(
            "/api/v1/software-items",
            &serde_json::json!({ "name": "Listed App" }),
        )
        .bearer(&token)
        .send_status()
        .await;

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/software-items")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    let items = body["items"].as_array().expect("data array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Listed App");
}

#[tokio::test]
async fn list_filters_by_query_case_insensitively() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    for name in ["Node.js", "node exporter", "Redis"] {
        client
            .post_json(
                "/api/v1/software-items",
                &serde_json::json!({ "name": name }),
            )
            .bearer(&token)
            .send_status()
            .await;
    }

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/software-items?query=node")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    let items = body["items"].as_array().expect("items array");
    assert_eq!(items.len(), 2);
    assert!(items.iter().all(|item| {
        item["name"]
            .as_str()
            .map(|name| name.to_ascii_lowercase().contains("node"))
            .unwrap_or(false)
    }));
}

#[tokio::test]
async fn list_treats_percent_and_underscore_as_literals_in_query() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    for name in [
        "100% Coverage",
        "100 percent Coverage",
        "under_score",
        "underXscore",
    ] {
        client
            .post_json(
                "/api/v1/software-items",
                &serde_json::json!({ "name": name }),
            )
            .bearer(&token)
            .send_status()
            .await;
    }

    let (percent_status, percent_body): (_, serde_json::Value) = client
        .get("/api/v1/software-items?query=%25")
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(percent_status, http::StatusCode::OK);
    let percent_items = percent_body["items"]
        .as_array()
        .expect("percent items array");
    assert_eq!(percent_items.len(), 1);
    assert_eq!(percent_items[0]["name"], "100% Coverage");

    let (underscore_status, underscore_body): (_, serde_json::Value) = client
        .get("/api/v1/software-items?query=_")
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(underscore_status, http::StatusCode::OK);
    let underscore_items = underscore_body["items"]
        .as_array()
        .expect("underscore items array");
    assert_eq!(underscore_items.len(), 1);
    assert_eq!(underscore_items[0]["name"], "under_score");
}

#[tokio::test]
async fn list_filters_non_ascii_queries_case_insensitively() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    for name in ["MÜNCHEN", "Paris"] {
        client
            .post_json(
                "/api/v1/software-items",
                &serde_json::json!({ "name": name }),
            )
            .bearer(&token)
            .send_status()
            .await;
    }

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/software-items?query=m%C3%BCn")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    let items = body["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "MÜNCHEN");
}

#[tokio::test]
async fn list_filters_query_with_pagination_and_total() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    for name in ["node alpha", "node beta", "node gamma", "redis"] {
        client
            .post_json(
                "/api/v1/software-items",
                &serde_json::json!({ "name": name }),
            )
            .bearer(&token)
            .send_status()
            .await;
    }

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/software-items?query=node&page=2&per_page=1")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    let items = body["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "node beta");
    assert_eq!(body["total"], 3);
}

#[tokio::test]
async fn get_returns_detail() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (_, created): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/software-items",
            &serde_json::json!({ "name": "Detail App" }),
        )
        .bearer(&token)
        .send_json()
        .await;

    let id = created["id"].as_str().expect("id");
    let (status, body): (_, serde_json::Value) = client
        .get(&format!("/api/v1/software-items/{id}"))
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["name"], "Detail App");
}

#[tokio::test]
async fn update_name_returns_200() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (_, created): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/software-items",
            &serde_json::json!({ "name": "Old Name" }),
        )
        .bearer(&token)
        .send_json()
        .await;

    let id = created["id"].as_str().expect("id");
    let (status, body): (_, serde_json::Value) = client
        .put_json(
            &format!("/api/v1/software-items/{id}"),
            &serde_json::json!({ "name": "New Name" }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["name"], "New Name");
}

#[tokio::test]
async fn delete_returns_204_then_get_returns_404() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (_, created): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/software-items",
            &serde_json::json!({ "name": "To Delete" }),
        )
        .bearer(&token)
        .send_json()
        .await;

    let id = created["id"].as_str().expect("id");

    let status = client
        .delete(&format!("/api/v1/software-items/{id}"))
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(status, http::StatusCode::NO_CONTENT);

    let status2 = client
        .get(&format!("/api/v1/software-items/{id}"))
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(status2, http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn create_empty_name_returns_400() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let status = client
        .post_json("/api/v1/software-items", &serde_json::json!({ "name": "" }))
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn trigger_update_on_nonexistent_item_returns_404() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let item_id = Uuid::now_v7();
    let host_id = Uuid::now_v7();
    let status = client
        .post_json(
            &format!("/api/v1/software-items/{item_id}/hosts/{host_id}/update"),
            &serde_json::json!({ "to_version": "1.0.0" }),
        )
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn check_versions_on_nonexistent_item_returns_404() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let item_id = Uuid::now_v7();
    let status = client
        .post_empty(&format!("/api/v1/software-items/{item_id}/check-versions"))
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn assign_hosts_with_empty_list_returns_400() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    // Create a real item so the route reaches the validation check.
    let (_, created): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/software-items",
            &serde_json::json!({ "name": "Assignable App" }),
        )
        .bearer(&token)
        .send_json()
        .await;
    let id = created["id"].as_str().expect("id");

    let status = client
        .post_json(
            &format!("/api/v1/software-items/{id}/hosts"),
            &serde_json::json!({ "host_assignments": [] }),
        )
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn merge_execute_missing_survivor_candidate_returns_400() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let item_a = Uuid::now_v7();
    let item_b = Uuid::now_v7();
    let non_candidate_survivor = Uuid::now_v7();

    insert_software_item(&app, item_a, "Item A").await;
    insert_software_item(&app, item_b, "Item B").await;

    let status = client
        .post_json(
            "/api/v1/software-items/merge/execute",
            &serde_json::json!({
                "candidate_ids": [item_a, item_b],
                "survivor_id": non_candidate_survivor,
            }),
        )
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn merge_preview_allows_update_without_delete_but_execute_forbids_it() {
    let app = TestApp::new().await;
    let client = app.client();
    let owner_token = register_and_get_token(&client).await;

    let editor_user_id = Uuid::now_v7();
    insert_active_user(&app, editor_user_id, "editor@test.local").await;

    let update_only_token = app
        .jwt
        .create_access_token(
            editor_user_id,
            &[Permission::UpdateSoftware],
            "password",
            None,
        )
        .expect("mint update-only token");

    let survivor_id = Uuid::now_v7();
    let loser_id = Uuid::now_v7();
    insert_software_item(&app, survivor_id, "Survivor").await;
    insert_software_item(&app, loser_id, "Loser").await;

    let preview_status = client
        .post_json(
            "/api/v1/software-items/merge/preview",
            &serde_json::json!({
                "candidate_ids": [survivor_id, loser_id],
                "survivor_id": survivor_id,
            }),
        )
        .bearer(&update_only_token)
        .send_status()
        .await;
    assert_eq!(preview_status, http::StatusCode::OK);

    let execute_status = client
        .post_json(
            "/api/v1/software-items/merge/execute",
            &serde_json::json!({
                "candidate_ids": [survivor_id, loser_id],
                "survivor_id": survivor_id,
            }),
        )
        .bearer(&update_only_token)
        .send_status()
        .await;
    assert_eq!(execute_status, http::StatusCode::FORBIDDEN);

    let owner_execute_status = client
        .post_json(
            "/api/v1/software-items/merge/execute",
            &serde_json::json!({
                "candidate_ids": [survivor_id, loser_id],
                "survivor_id": survivor_id,
            }),
        )
        .bearer(&owner_token)
        .send_status()
        .await;
    assert_eq!(owner_execute_status, http::StatusCode::OK);
}

#[tokio::test]
async fn merge_execute_soft_deletes_losers_and_moves_links() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let survivor_id = Uuid::now_v7();
    let loser_id = Uuid::now_v7();
    let host_id = Uuid::now_v7();
    let moved_link_id = Uuid::now_v7();
    let plugin_row_id = Uuid::now_v7();

    insert_software_item(&app, survivor_id, "Survivor").await;
    insert_software_item(&app, loser_id, "Loser").await;
    insert_host_for_merge_test(&app, host_id, "merge-host").await;
    insert_host_link(&app, moved_link_id, host_id, loser_id, Some("beta")).await;
    insert_plugin_row(
        &app,
        plugin_row_id,
        host_id,
        loser_id,
        moved_link_id,
        "detect_version",
        0,
    )
    .await;

    let response = client
        .post_json(
            "/api/v1/software-items/merge/execute",
            &serde_json::json!({
                "candidate_ids": [survivor_id, loser_id],
                "survivor_id": survivor_id,
            }),
        )
        .bearer(&token)
        .send()
        .await;
    let status = response.status();
    assert_eq!(status, http::StatusCode::OK);
    let (_, body) = read_json_response(response).await;
    assert_eq!(body["survivor_id"], survivor_id.to_string());
    assert_eq!(body["deleted_ids"], serde_json::json!([loser_id]));
    assert_eq!(body["moved_link_ids"], serde_json::json!([moved_link_id]));
    assert_eq!(body["skipped_duplicate_link_ids"], serde_json::json!([]));

    let survivor = software_item::Entity::find_by_id(survivor_id)
        .one(&app.db)
        .await
        .expect("load survivor")
        .expect("survivor exists");
    assert_eq!(survivor.name, "Survivor");
    assert!(survivor.deactivated_at.is_none());

    let loser = software_item::Entity::find_by_id(loser_id)
        .one(&app.db)
        .await
        .expect("load loser")
        .expect("loser exists");
    assert!(loser.deactivated_at.is_some());

    let moved_link = host_software_item::Entity::find_by_id(moved_link_id)
        .one(&app.db)
        .await
        .expect("load moved link")
        .expect("moved link exists");
    assert_eq!(moved_link.software_item_id, survivor_id);
    assert!(moved_link.deactivated_at.is_none());

    let moved_plugin = host_software_item_plugin::Entity::find_by_id(plugin_row_id)
        .one(&app.db)
        .await
        .expect("load plugin row")
        .expect("plugin row exists");
    assert_eq!(moved_plugin.software_item_id, survivor_id);
    assert_eq!(moved_plugin.host_software_item_id, moved_link_id);
}

#[tokio::test]
async fn merge_execute_skips_equivalent_survivor_link() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let survivor_id = Uuid::now_v7();
    let loser_id = Uuid::now_v7();
    let host_id = Uuid::now_v7();
    let survivor_link_id = Uuid::now_v7();
    let survivor_other_qualifier_link_id = Uuid::now_v7();
    let duplicate_loser_link_id = Uuid::now_v7();
    let survivor_detect_plugin_id = Uuid::now_v7();
    let duplicate_detect_plugin_id = Uuid::now_v7();
    let survivor_beta_execute_plugin_id = Uuid::now_v7();
    let unique_execute_plugin_id = Uuid::now_v7();

    insert_software_item(&app, survivor_id, "Survivor").await;
    insert_software_item(&app, loser_id, "Loser").await;
    insert_host_for_merge_test(&app, host_id, "duplicate-host").await;
    insert_host_link(&app, survivor_link_id, host_id, survivor_id, Some("stable")).await;
    insert_host_link(
        &app,
        survivor_other_qualifier_link_id,
        host_id,
        survivor_id,
        Some("beta"),
    )
    .await;
    insert_host_link(
        &app,
        duplicate_loser_link_id,
        host_id,
        loser_id,
        Some("stable"),
    )
    .await;
    insert_plugin_row(
        &app,
        survivor_detect_plugin_id,
        host_id,
        survivor_id,
        survivor_link_id,
        "detect_version",
        0,
    )
    .await;
    insert_plugin_row(
        &app,
        duplicate_detect_plugin_id,
        host_id,
        loser_id,
        duplicate_loser_link_id,
        "detect_version",
        0,
    )
    .await;
    insert_plugin_row(
        &app,
        survivor_beta_execute_plugin_id,
        host_id,
        survivor_id,
        survivor_other_qualifier_link_id,
        "execute_update",
        1,
    )
    .await;
    insert_plugin_row(
        &app,
        unique_execute_plugin_id,
        host_id,
        loser_id,
        duplicate_loser_link_id,
        "execute_update",
        0,
    )
    .await;

    let response = client
        .post_json(
            "/api/v1/software-items/merge/execute",
            &serde_json::json!({
                "candidate_ids": [survivor_id, loser_id],
                "survivor_id": survivor_id,
            }),
        )
        .bearer(&token)
        .send()
        .await;
    let status = response.status();
    assert_eq!(status, http::StatusCode::OK);
    let (_, body) = read_json_response(response).await;
    assert_eq!(body["survivor_id"], survivor_id.to_string());
    assert_eq!(body["deleted_ids"], serde_json::json!([loser_id]));
    assert_eq!(body["moved_link_ids"], serde_json::json!([]));
    assert_eq!(
        body["skipped_duplicate_link_ids"],
        serde_json::json!([duplicate_loser_link_id])
    );

    let survivor_link = host_software_item::Entity::find_by_id(survivor_link_id)
        .one(&app.db)
        .await
        .expect("load survivor link")
        .expect("survivor link exists");
    assert_eq!(survivor_link.software_item_id, survivor_id);
    assert_eq!(survivor_link.qualifier.as_deref(), Some("stable"));

    let skipped_link = host_software_item::Entity::find_by_id(duplicate_loser_link_id)
        .one(&app.db)
        .await
        .expect("load skipped link");
    assert!(
        skipped_link.is_none(),
        "duplicate loser link should be removed instead of moved"
    );

    let duplicate_detect_plugin =
        host_software_item_plugin::Entity::find_by_id(duplicate_detect_plugin_id)
            .one(&app.db)
            .await
            .expect("load duplicate detect plugin");
    assert!(
        duplicate_detect_plugin.is_none(),
        "redundant duplicate plugin assignment should be dropped"
    );

    let unique_execute_plugin =
        host_software_item_plugin::Entity::find_by_id(unique_execute_plugin_id)
            .one(&app.db)
            .await
            .expect("load unique execute plugin")
            .expect("unique execute plugin should survive reconciliation");
    assert_eq!(unique_execute_plugin.software_item_id, survivor_id);
    assert_eq!(
        unique_execute_plugin.host_software_item_id,
        survivor_link_id
    );
    assert_eq!(unique_execute_plugin.role, "execute_update");
    assert_eq!(unique_execute_plugin.host_id, host_id);

    let survivor_detect_plugin =
        host_software_item_plugin::Entity::find_by_id(survivor_detect_plugin_id)
            .one(&app.db)
            .await
            .expect("load survivor detect plugin")
            .expect("survivor detect plugin should remain");
    assert_eq!(survivor_detect_plugin.software_item_id, survivor_id);
    assert_eq!(
        survivor_detect_plugin.host_software_item_id,
        survivor_link_id
    );
    assert_eq!(survivor_detect_plugin.role, "detect_version");

    let survivor_beta_execute_plugin =
        host_software_item_plugin::Entity::find_by_id(survivor_beta_execute_plugin_id)
            .one(&app.db)
            .await
            .expect("load survivor beta execute plugin")
            .expect("survivor beta execute plugin should remain");
    assert_eq!(survivor_beta_execute_plugin.software_item_id, survivor_id);
    assert_eq!(
        survivor_beta_execute_plugin.host_software_item_id,
        survivor_other_qualifier_link_id
    );
    assert_eq!(survivor_beta_execute_plugin.role, "execute_update");

    let loser = software_item::Entity::find_by_id(loser_id)
        .one(&app.db)
        .await
        .expect("load loser")
        .expect("loser exists");
    assert!(loser.deactivated_at.is_some());
}

#[tokio::test]
async fn merge_preview_and_execute_reject_conflicting_duplicate_link_plugin_assignments() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let survivor_id = Uuid::now_v7();
    let loser_id = Uuid::now_v7();
    let host_id = Uuid::now_v7();
    let survivor_link_id = Uuid::now_v7();
    let duplicate_loser_link_id = Uuid::now_v7();

    insert_software_item(&app, survivor_id, "Survivor").await;
    insert_software_item(&app, loser_id, "Loser").await;
    insert_host_for_merge_test(&app, host_id, "conflict-host").await;
    insert_host_link(&app, survivor_link_id, host_id, survivor_id, Some("stable")).await;
    insert_host_link(
        &app,
        duplicate_loser_link_id,
        host_id,
        loser_id,
        Some("stable"),
    )
    .await;
    insert_plugin_row_with_details(
        &app,
        Uuid::now_v7(),
        host_id,
        survivor_id,
        survivor_link_id,
        "package_manager_apt",
        "detect_version",
        0,
        None,
        "pkg-a",
        Some(serde_json::json!({"channel": "stable"})),
        "auto",
    )
    .await;
    insert_plugin_row_with_details(
        &app,
        Uuid::now_v7(),
        host_id,
        loser_id,
        duplicate_loser_link_id,
        "package_manager_apt",
        "detect_version",
        0,
        None,
        "pkg-b",
        Some(serde_json::json!({"channel": "beta"})),
        "auto",
    )
    .await;

    for endpoint in [
        "/api/v1/software-items/merge/preview",
        "/api/v1/software-items/merge/execute",
    ] {
        let response = client
            .post_json(
                endpoint,
                &serde_json::json!({
                    "candidate_ids": [survivor_id, loser_id],
                    "survivor_id": survivor_id,
                }),
            )
            .bearer(&token)
            .send()
            .await;
        let (status, body) = read_json_response(response).await;

        assert_eq!(status, http::StatusCode::BAD_REQUEST, "{endpoint}");
        assert_eq!(body["code"], "software_item.invalid_merge_request");
    }
}

/// Deactivated hosts must not be counted in `host_count` on either the list
/// or the detail endpoint.
#[tokio::test]
async fn host_count_excludes_deactivated_hosts() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    // Create a software item.
    let (_, created): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/software-items",
            &serde_json::json!({ "name": "Counter App" }),
        )
        .bearer(&token)
        .send_json()
        .await;
    let item_id: Uuid = created["id"].as_str().expect("id").parse().expect("uuid");

    // Insert two hosts directly (bypasses service/enrollment setup).
    // Use the full UUID as machine_id to guarantee uniqueness even when both
    // hosts are created within the same millisecond (UUID v7 timestamp prefix
    // would otherwise collide on the unique constraint).
    let now = time::OffsetDateTime::now_utc();
    let host1_id = Uuid::now_v7();
    let host2_id = Uuid::now_v7();
    for host_id in [host1_id, host2_id] {
        host::ActiveModel {
            id: Set(host_id),
            tenant_id: Set(app.tenant_id),
            machine_id: Set(host_id.to_string()),
            hostname: Set(format!("host-{host_id}")),
            friendly_name: Set(format!("Host {host_id}")),
            os_type: Set(Some("linux".to_string())),
            os_version: Set(None),
            architecture: Set(None),
            ip_address: Set(None),
            host_features: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(&app.db)
        .await
        .expect("insert host");
    }

    // Link both hosts to the software item directly via the join table.
    for host_id in [host1_id, host2_id] {
        host_software_item::ActiveModel {
            id: Set(Uuid::now_v7()),
            host_id: Set(host_id),
            software_item_id: Set(item_id),
            qualifier: Set(None),
            installed_version: Set(None),
            installed_version_detected_at: Set(None),
            installed_display_version: Set(None),
            latest_version: Set(None),
            latest_version_fetched_at: Set(None),
            latest_release_metadata: Set(None),
            last_updated_at: Set(None),
            linked_at: Set(now),
            update_category: Set("unknown".to_string()),
            plugin_config_id: Set(None),
            package_identifier: Set(None),
            deactivated_at: Set(None),
        }
        .insert(&app.db)
        .await
        .expect("insert host_software_item link");
    }

    // Deactivate host2 via the REST API.
    let del_status = client
        .delete(&format!("/api/v1/hosts/{host2_id}"))
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(del_status, http::StatusCode::NO_CONTENT);

    // List endpoint: host_count must reflect only the active host.
    let (list_status, list_body): (_, serde_json::Value) = client
        .get("/api/v1/software-items")
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(list_status, http::StatusCode::OK);
    let items = list_body["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0]["host_count"], 1,
        "deactivated host must not be counted on the list endpoint"
    );

    // Detail endpoint: host_count and hosts array must also exclude the deactivated host.
    let (detail_status, detail_body): (_, serde_json::Value) = client
        .get(&format!("/api/v1/software-items/{item_id}"))
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(detail_status, http::StatusCode::OK);
    assert_eq!(
        detail_body["host_count"], 1,
        "deactivated host must not be counted on the detail endpoint"
    );
    assert_eq!(
        detail_body["hosts"].as_array().expect("hosts array").len(),
        1,
        "deactivated host must not appear in the hosts list on the detail endpoint"
    );
}

#[tokio::test]
async fn approve_non_pending_item_returns_409() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    // Create an item — discovery_state defaults to None (not pending).
    let (_, created): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/software-items",
            &serde_json::json!({ "name": "Non-Pending App" }),
        )
        .bearer(&token)
        .send_json()
        .await;
    let id = created["id"].as_str().expect("id");

    let status = client
        .post_empty(&format!("/api/v1/software-items/{id}/approve"))
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::CONFLICT);
}

/// The `updatable` query parameter filters by update availability at the DB layer,
/// so total counts and pagination are correct regardless of page size.
#[tokio::test]
async fn list_with_updatable_filter() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    // Create two software items.
    let (_, item_a): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/software-items",
            &serde_json::json!({ "name": "App A" }),
        )
        .bearer(&token)
        .send_json()
        .await;
    let item_a_id: Uuid = item_a["id"].as_str().expect("id").parse().expect("uuid");

    let (_, item_b): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/software-items",
            &serde_json::json!({ "name": "App B" }),
        )
        .bearer(&token)
        .send_json()
        .await;
    let item_b_id: Uuid = item_b["id"].as_str().expect("id").parse().expect("uuid");

    // Insert a host.
    let now = time::OffsetDateTime::now_utc();
    let host_id = Uuid::now_v7();
    host::ActiveModel {
        id: Set(host_id),
        tenant_id: Set(app.tenant_id),
        machine_id: Set(host_id.to_string()),
        hostname: Set(format!("host-{host_id}")),
        friendly_name: Set(format!("Host {host_id}")),
        os_type: Set(Some("linux".to_string())),
        os_version: Set(None),
        architecture: Set(None),
        ip_address: Set(None),
        host_features: Set(None),
        last_seen_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    }
    .insert(&app.db)
    .await
    .expect("insert host");

    // App A: update available (installed 1.0, latest 2.0).
    host_software_item::ActiveModel {
        id: Set(Uuid::now_v7()),
        host_id: Set(host_id),
        software_item_id: Set(item_a_id),
        qualifier: Set(None),
        installed_version: Set(Some("1.0".to_string())),
        installed_version_detected_at: Set(None),
        installed_display_version: Set(None),
        latest_version: Set(Some("2.0".to_string())),
        latest_version_fetched_at: Set(None),
        latest_release_metadata: Set(None),
        last_updated_at: Set(None),
        linked_at: Set(now),
        update_category: Set("unknown".to_string()),
        plugin_config_id: Set(None),
        package_identifier: Set(None),
        deactivated_at: Set(None),
    }
    .insert(&app.db)
    .await
    .expect("insert host_software_item for App A");

    // App B: up to date (installed 3.0, latest 3.0).
    host_software_item::ActiveModel {
        id: Set(Uuid::now_v7()),
        host_id: Set(host_id),
        software_item_id: Set(item_b_id),
        qualifier: Set(None),
        installed_version: Set(Some("3.0".to_string())),
        installed_version_detected_at: Set(None),
        installed_display_version: Set(None),
        latest_version: Set(Some("3.0".to_string())),
        latest_version_fetched_at: Set(None),
        latest_release_metadata: Set(None),
        last_updated_at: Set(None),
        linked_at: Set(now),
        update_category: Set("unknown".to_string()),
        plugin_config_id: Set(None),
        package_identifier: Set(None),
        deactivated_at: Set(None),
    }
    .insert(&app.db)
    .await
    .expect("insert host_software_item for App B");

    // updatable=true → only App A.
    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/software-items?updatable=true")
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(status, http::StatusCode::OK);
    let items = body["items"].as_array().expect("items array");
    assert_eq!(
        items.len(),
        1,
        "only App A should appear with updatable=true"
    );
    assert_eq!(items[0]["name"], "App A");
    assert_eq!(body["total"], 1, "total must reflect DB-side filter");
    assert!(items[0]["update_available"].as_bool().unwrap_or(false));

    // updatable=false → only App B.
    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/software-items?updatable=false")
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(status, http::StatusCode::OK);
    let items = body["items"].as_array().expect("items array");
    assert_eq!(
        items.len(),
        1,
        "only App B should appear with updatable=false"
    );
    assert_eq!(items[0]["name"], "App B");
    assert_eq!(body["total"], 1, "total must reflect DB-side filter");
    assert!(!items[0]["update_available"].as_bool().unwrap_or(true));

    // No filter → both items.
    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/software-items")
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(status, http::StatusCode::OK);
    let items = body["items"].as_array().expect("items array");
    assert_eq!(items.len(), 2, "both items should appear with no filter");
    assert_eq!(body["total"], 2);
}
