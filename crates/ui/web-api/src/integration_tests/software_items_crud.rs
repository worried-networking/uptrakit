use crate::test_harness::TestApp;
use crate::test_harness::fixtures::register_and_get_token;
use sea_orm::{ActiveModelTrait, Set};
use uptrakit_shared_db::entity::{host, host_software_item};
use uuid::Uuid;

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
