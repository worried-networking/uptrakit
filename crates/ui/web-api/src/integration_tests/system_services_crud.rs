//! HTTP-level coverage for `/api/v1/system-services`, focused on the
//! engine deny taking effect immediately once a covering grant is revoked.

use uptrakit_shared_types::access::actions;
use uuid::Uuid;

use crate::test_harness::TestApp;
use crate::test_harness::fixtures::{
    link_role, open_registration, register_user, revoke_role_grants_covering, role_id_by_name,
};

#[tokio::test]
async fn batch_system_services_forbidden_after_covering_grant_revoked() {
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

    let system_administrator_role_id = role_id_by_name(&app, "system_administrator").await;
    link_role(&app, user_id, system_administrator_role_id).await;
    revoke_role_grants_covering(
        &app,
        system_administrator_role_id,
        &[actions::SYSTEM_SERVICES_APPROVE],
    )
    .await;

    // `revoke_role_grants_covering` invalidates the role's cached authority,
    // so the registration token already in hand re-resolves grants on the
    // next request — no fresh login needed.
    let token = auth.access_token.expose_secret().to_string();

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
        "engine must deny the system-services batch approve once the covering grant is revoked"
    );
}
