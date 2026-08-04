use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use uptrakit_shared_db::access_grants::{GrantSubject, delete_grant, load_grants_for_principal};
use uptrakit_shared_db::entity::service::ServiceStatus;
use uptrakit_shared_db::entity::{role, user_role};
use uptrakit_shared_types::access::actions;
use uuid::Uuid;

use crate::test_harness::TestApp;
use crate::test_harness::fixtures::{
    insert_embedded_service, insert_service, login_user, register_and_get_token, register_user,
};

/// Register the first user (owner) and re-open registration so a second
/// user can sign up. Returns the owner's access token.
async fn open_registration(app: &TestApp) -> String {
    let client = app.client();
    let owner_token = register_and_get_token(&client).await;
    let reopen = client
        .put_json(
            "/api/v1/settings/access",
            &serde_json::json!({ "mode": "open" }),
        )
        .bearer(&owner_token)
        .header("if-match", "W/\"settings-v0\"")
        .send_status()
        .await;
    assert_eq!(
        reopen,
        http::StatusCode::OK,
        "failed to re-open registration"
    );
    owner_token
}

#[tokio::test]
async fn list_services_empty_returns_200() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/services")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["items"].as_array().expect("items array").len(), 0);
    assert_eq!(body["total"], 0);
}

#[tokio::test]
async fn list_services_returns_enrolled_service() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    // Insert a service directly.
    insert_service(&app.db, app.tenant_id, ServiceStatus::Approved).await;

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/services")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["items"].as_array().expect("items array").len(), 1);
    assert_eq!(body["total"], 1);
}

#[tokio::test]
async fn approve_service_returns_200() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let svc = insert_service(&app.db, app.tenant_id, ServiceStatus::Pending).await;

    let (status, body): (_, serde_json::Value) = client
        .post_empty(&format!("/api/v1/services/{}/approve", svc.id))
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["status"], "approved");
}

#[tokio::test]
async fn reject_service_returns_200() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let svc = insert_service(&app.db, app.tenant_id, ServiceStatus::Pending).await;

    let (status, body): (_, serde_json::Value) = client
        .post_empty(&format!("/api/v1/services/{}/reject", svc.id))
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["status"], "rejected");
}

#[tokio::test]
async fn deactivate_service_returns_204() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let svc = insert_service(&app.db, app.tenant_id, ServiceStatus::Approved).await;

    let status = client
        .delete(&format!("/api/v1/services/{}", svc.id))
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn update_service_friendly_name() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let svc = insert_service(&app.db, app.tenant_id, ServiceStatus::Approved).await;

    let (status, body): (_, serde_json::Value) = client
        .put_json(
            &format!("/api/v1/services/{}", svc.id),
            &serde_json::json!({ "ping_interval_seconds": 30 }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["ping_interval_seconds"], 30);
}

#[tokio::test]
async fn get_nonexistent_service_returns_404() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let fake_id = uuid::Uuid::now_v7();
    let status = client
        .get(&format!("/api/v1/services/{fake_id}"))
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn deactivate_embedded_service_returns_400() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let svc = insert_embedded_service(&app.db, app.tenant_id).await;

    let (status, body): (_, serde_json::Value) = client
        .delete(&format!("/api/v1/services/{}", svc.id))
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::BAD_REQUEST);
    assert!(
        body["error"].as_str().unwrap_or("").contains("embedded"),
        "expected error about embedded services, got: {body}"
    );
}

#[tokio::test]
async fn get_embedded_service_shows_is_embedded() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let svc = insert_embedded_service(&app.db, app.tenant_id).await;

    let (status, body): (_, serde_json::Value) = client
        .get(&format!("/api/v1/services/{}", svc.id))
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["is_embedded"], true);
}

#[tokio::test]
async fn batch_services_engine_deny_overrides_legacy_permission_is_403() {
    let app = TestApp::new().await;
    let client = app.client();
    open_registration(&app).await;

    let (status, auth) =
        register_user(&client, "batch-legacy@test.local", "TestPassword123!").await;
    assert_eq!(
        status,
        http::StatusCode::CREATED,
        "user registration failed"
    );
    let user_id = auth.user.id;

    let service_manager_role_id = role::Entity::find()
        .filter(role::Column::Name.eq("service_manager"))
        .one(&app.db)
        .await
        .expect("query roles")
        .expect("seeded service_manager role")
        .id;

    user_role::ActiveModel {
        tenant_id: Set(app.tenant_id),
        user_id: Set(user_id),
        role_id: Set(service_manager_role_id),
        assigned_at: Set(time::OffsetDateTime::now_utc()),
    }
    .insert(&app.db)
    .await
    .expect("assign service_manager role");
    app.state.access_engine.invalidate_subjects(&[user_id], &[]);

    let load = load_grants_for_principal(
        &app.db,
        app.tenant_id,
        Uuid::nil(),
        &[service_manager_role_id],
    )
    .await
    .expect("load service_manager grants");
    let mut deleted_any = false;
    for grant in load.grants {
        if grant.subject == GrantSubject::Role(service_manager_role_id)
            && grant
                .patterns
                .iter()
                .any(|pattern| pattern.matches(&actions::SERVICES_APPROVE))
        {
            delete_grant(&app.db, grant.id)
                .await
                .expect("delete service_manager services:approve grant");
            deleted_any = true;
        }
    }
    assert!(
        deleted_any,
        "expected at least one service_manager grant row covering services:approve"
    );
    app.state
        .access_engine
        .invalidate_subjects(&[], &[service_manager_role_id]);

    let (login_status, login_auth) =
        login_user(&client, "batch-legacy@test.local", "TestPassword123!").await;
    assert_eq!(login_status, http::StatusCode::OK);
    let token = login_auth.access_token.expose_secret().to_string();

    let svc = insert_service(&app.db, app.tenant_id, ServiceStatus::Pending).await;

    let status = client
        .post_json(
            "/api/v1/services/batch",
            &serde_json::json!({ "action": "approve", "ids": [svc.id] }),
        )
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(
        status,
        http::StatusCode::FORBIDDEN,
        "engine must deny the services batch approve once the covering grant is revoked, \
         even though the legacy approve_services JWT claim is still present"
    );
}

#[tokio::test]
async fn services_without_auth_returns_401() {
    let app = TestApp::new().await;
    let client = app.client();

    let status = client.get("/api/v1/services").send_status().await;
    assert_eq!(status, http::StatusCode::UNAUTHORIZED);
}
