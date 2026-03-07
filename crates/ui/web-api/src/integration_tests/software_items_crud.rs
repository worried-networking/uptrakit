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
            host_id: Set(host_id),
            software_item_id: Set(item_id),
            installed_version: Set(None),
            installed_version_detected_at: Set(None),
            latest_version: Set(None),
            latest_version_fetched_at: Set(None),
            latest_release_metadata: Set(None),
            last_updated_at: Set(None),
            linked_at: Set(now),
            update_category: Set("unknown".to_string()),
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
