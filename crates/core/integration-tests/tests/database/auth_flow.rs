use crate::database_helpers::fixtures::{login_user, refresh_token, register_user};
use crate::database_helpers::harness::TestHarness;
use crate::database_helpers::macros::db_test;

async fn test_register_first_user_gets_owner(harness: &TestHarness) {
    let client = harness.client();
    let (status, auth) = register_user(&client, "owner@test.local", "StrongPassword1!").await;

    assert_eq!(status, http::StatusCode::CREATED);
    assert!(!auth.access_token.expose_secret().is_empty());
    assert!(!auth.refresh_token.expose_secret().is_empty());
    assert_eq!(auth.token_type, "Bearer");
    assert_eq!(auth.user.email, "owner@test.local");
    assert!(
        !auth.user.permissions.is_empty(),
        "first user should have permissions"
    );
}

db_test!(
    register_first_user_gets_owner,
    test_register_first_user_gets_owner
);

async fn test_register_second_user_gets_viewer(harness: &TestHarness) {
    let client = harness.client();

    let (s1, first) = register_user(&client, "owner@test.local", "StrongPassword1!").await;
    assert_eq!(s1, http::StatusCode::CREATED);

    // Re-open registration (initial setup closes it after first user).
    let reopen_status = client
        .put_json(
            "/api/v1/settings/registration",
            &serde_json::json!({ "mode": "open" }),
        )
        .bearer(first.access_token.expose_secret())
        .send_status()
        .await;
    assert_eq!(reopen_status, http::StatusCode::OK);

    let (s2, second) = register_user(&client, "user2@test.local", "StrongPassword2!").await;
    assert_eq!(s2, http::StatusCode::CREATED);
    assert!(
        second.user.permissions.len() < first.user.permissions.len(),
        "second user should have fewer permissions than owner"
    );
}

db_test!(
    register_second_user_gets_viewer,
    test_register_second_user_gets_viewer
);

async fn test_register_duplicate_email_returns_409(harness: &TestHarness) {
    let client = harness.client();

    let (s1, first) = register_user(&client, "dup@test.local", "StrongPassword1!").await;
    assert_eq!(s1, http::StatusCode::CREATED);

    client
        .put_json(
            "/api/v1/settings/registration",
            &serde_json::json!({ "mode": "open" }),
        )
        .bearer(first.access_token.expose_secret())
        .send_status()
        .await;

    let status = client
        .post_json(
            "/api/v1/auth/register",
            &serde_json::json!({
                "email": "dup@test.local",
                "first_name": "Dup",
                "last_name": "User",
                "password": "StrongPassword1!"
            }),
        )
        .send_status()
        .await;
    assert_eq!(status, http::StatusCode::CONFLICT);
}

db_test!(
    register_duplicate_email_returns_409,
    test_register_duplicate_email_returns_409
);

async fn test_register_invalid_email_returns_400(harness: &TestHarness) {
    let client = harness.client();

    let status = client
        .post_json(
            "/api/v1/auth/register",
            &serde_json::json!({
                "email": "",
                "first_name": "A",
                "last_name": "B",
                "password": "StrongPassword1!"
            }),
        )
        .send_status()
        .await;
    assert_eq!(status, http::StatusCode::BAD_REQUEST);
}

db_test!(
    register_invalid_email_returns_400,
    test_register_invalid_email_returns_400
);

async fn test_login_valid_credentials(harness: &TestHarness) {
    let client = harness.client();

    register_user(&client, "login@test.local", "StrongPassword1!").await;
    let (status, auth) = login_user(&client, "login@test.local", "StrongPassword1!").await;

    assert_eq!(status, http::StatusCode::OK);
    assert!(!auth.access_token.expose_secret().is_empty());
    assert_eq!(auth.user.email, "login@test.local");
}

db_test!(login_valid_credentials, test_login_valid_credentials);

async fn test_login_wrong_password_returns_401(harness: &TestHarness) {
    let client = harness.client();

    register_user(&client, "wrong@test.local", "StrongPassword1!").await;
    let status = client
        .post_json(
            "/api/v1/auth/login",
            &serde_json::json!({
                "email": "wrong@test.local",
                "password": "WrongPassword!!"
            }),
        )
        .send_status()
        .await;
    assert_eq!(status, http::StatusCode::UNAUTHORIZED);
}

db_test!(
    login_wrong_password_returns_401,
    test_login_wrong_password_returns_401
);

async fn test_login_nonexistent_user_returns_401(harness: &TestHarness) {
    let client = harness.client();

    let status = client
        .post_json(
            "/api/v1/auth/login",
            &serde_json::json!({
                "email": "ghost@test.local",
                "password": "StrongPassword1!"
            }),
        )
        .send_status()
        .await;
    assert_eq!(status, http::StatusCode::UNAUTHORIZED);
}

db_test!(
    login_nonexistent_user_returns_401,
    test_login_nonexistent_user_returns_401
);

async fn test_refresh_token_returns_new_access_token(harness: &TestHarness) {
    let client = harness.client();

    let (_, auth) = register_user(&client, "refresh@test.local", "StrongPassword1!").await;
    let (status, refreshed) = refresh_token(&client, auth.refresh_token.expose_secret()).await;

    assert_eq!(status, http::StatusCode::OK);
    assert!(!refreshed.access_token.expose_secret().is_empty());
    assert!(!refreshed.refresh_token.expose_secret().is_empty());
}

db_test!(
    refresh_token_returns_new_access_token,
    test_refresh_token_returns_new_access_token
);

async fn test_logout_revokes_tokens(harness: &TestHarness) {
    let client = harness.client();

    let (_, auth) = register_user(&client, "logout@test.local", "StrongPassword1!").await;
    let access = auth.access_token.expose_secret();
    let refresh = auth.refresh_token.expose_secret().to_string();

    let status = client
        .post_json(
            "/api/v1/auth/logout",
            &serde_json::json!({ "refresh_token": refresh }),
        )
        .bearer(access)
        .send_status()
        .await;
    assert_eq!(status, http::StatusCode::NO_CONTENT);

    let s2 = client
        .post_json(
            "/api/v1/auth/refresh",
            &serde_json::json!({ "refresh_token": refresh }),
        )
        .send_status()
        .await;
    assert_eq!(s2, http::StatusCode::UNAUTHORIZED);
}

db_test!(logout_revokes_tokens, test_logout_revokes_tokens);
