//! HTTP-level coverage for `/api/v1/system-services`, focused on the
//! engine-vs-legacy-claim discriminator for the batch action gate.

use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use uptrakit_shared_db::access_grants::{GrantSubject, delete_grant, load_grants_for_principal};
use uptrakit_shared_db::entity::{role, user_role};
use uptrakit_shared_types::access::actions;
use uuid::Uuid;

use crate::test_harness::TestApp;
use crate::test_harness::fixtures::{login_user, register_and_get_token, register_user};

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
async fn batch_system_services_engine_deny_overrides_legacy_permission_is_403() {
    let app = TestApp::new().await;
    let client = app.client();
    open_registration(&app).await;

    let (status, auth) = register_user(
        &client,
        "system-batch-legacy@test.local",
        "TestPassword123!",
    )
    .await;
    assert_eq!(
        status,
        http::StatusCode::CREATED,
        "user registration failed"
    );
    let user_id = auth.user.id;

    let system_administrator_role_id = role::Entity::find()
        .filter(role::Column::Name.eq("system_administrator"))
        .one(&app.db)
        .await
        .expect("query roles")
        .expect("seeded system_administrator role")
        .id;

    user_role::ActiveModel {
        tenant_id: Set(app.tenant_id),
        user_id: Set(user_id),
        role_id: Set(system_administrator_role_id),
        assigned_at: Set(time::OffsetDateTime::now_utc()),
    }
    .insert(&app.db)
    .await
    .expect("assign system_administrator role");
    app.state.access_engine.invalidate_subjects(&[user_id], &[]);

    let load = load_grants_for_principal(
        &app.db,
        app.tenant_id,
        Uuid::nil(),
        &[system_administrator_role_id],
    )
    .await
    .expect("load system_administrator grants");
    let mut deleted_any = false;
    for grant in load.grants {
        if grant.subject == GrantSubject::Role(system_administrator_role_id)
            && grant
                .patterns
                .iter()
                .any(|pattern| pattern.matches(&actions::SYSTEM_SERVICES_APPROVE))
        {
            delete_grant(&app.db, grant.id)
                .await
                .expect("delete system_administrator system.services:approve grant");
            deleted_any = true;
        }
    }
    assert!(
        deleted_any,
        "expected at least one system_administrator grant row covering system.services:approve"
    );
    app.state
        .access_engine
        .invalidate_subjects(&[], &[system_administrator_role_id]);

    let (login_status, login_auth) = login_user(
        &client,
        "system-batch-legacy@test.local",
        "TestPassword123!",
    )
    .await;
    assert_eq!(login_status, http::StatusCode::OK);
    let token = login_auth.access_token.expose_secret().to_string();

    let status = client
        .post_json(
            "/api/v1/system-services/batch",
            &serde_json::json!({ "action": "approve", "ids": [Uuid::now_v7()] }),
        )
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(
        status,
        http::StatusCode::FORBIDDEN,
        "engine must deny the system-services batch approve once the covering grant is \
         revoked, even though the legacy approve_system_services JWT claim is still present"
    );
}
