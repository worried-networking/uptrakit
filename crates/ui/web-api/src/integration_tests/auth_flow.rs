use crate::test_harness::TestApp;
use crate::test_harness::fixtures::{login_user, refresh_token, register_user};

#[tokio::test]
async fn register_first_user_returns_201() {
    let app = TestApp::new().await;
    let client = app.client();

    let (status, auth) = register_user(&client, "owner@test.local", "StrongPassword1!").await;

    assert_eq!(status, http::StatusCode::CREATED);
    assert!(!auth.access_token.expose_secret().is_empty());
    assert!(!auth.refresh_token.expose_secret().is_empty());
    assert_eq!(auth.token_type, "Bearer");
    assert_eq!(auth.user.email, "owner@test.local");
    // First user gets the "owner" role — all permissions should be present.
    assert!(
        !auth.user.permissions.is_empty(),
        "first user should have permissions"
    );
}

#[tokio::test]
async fn register_second_user_gets_user_role() {
    let app = TestApp::new().await;
    let client = app.client();

    // First user (gets owner role).
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

    // Second user (gets user role — fewer permissions).
    let (s2, second) = register_user(&client, "user2@test.local", "StrongPassword2!").await;
    assert_eq!(s2, http::StatusCode::CREATED);
    assert!(
        second.user.permissions.len() < first.user.permissions.len(),
        "second user should have fewer permissions than owner"
    );
}

#[tokio::test]
async fn register_duplicate_email_returns_409() {
    let app = TestApp::new().await;
    let client = app.client();

    let (s1, first) = register_user(&client, "dup@test.local", "StrongPassword1!").await;
    assert_eq!(s1, http::StatusCode::CREATED);

    // Re-open registration (initial setup closes it after first user).
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

#[tokio::test]
async fn register_invalid_email_returns_400() {
    let app = TestApp::new().await;
    let client = app.client();

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

#[tokio::test]
async fn register_short_password_returns_400() {
    let app = TestApp::new().await;
    let client = app.client();

    let status = client
        .post_json(
            "/api/v1/auth/register",
            &serde_json::json!({
                "email": "short@test.local",
                "first_name": "A",
                "last_name": "B",
                "password": "abc"
            }),
        )
        .send_status()
        .await;
    assert_eq!(status, http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn login_valid_returns_200() {
    let app = TestApp::new().await;
    let client = app.client();

    register_user(&client, "login@test.local", "StrongPassword1!").await;
    let (status, auth) = login_user(&client, "login@test.local", "StrongPassword1!").await;

    assert_eq!(status, http::StatusCode::OK);
    assert!(!auth.access_token.expose_secret().is_empty());
    assert_eq!(auth.user.email, "login@test.local");
}

#[tokio::test]
async fn login_wrong_password_returns_401() {
    let app = TestApp::new().await;
    let client = app.client();

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

#[tokio::test]
async fn login_nonexistent_user_returns_401() {
    let app = TestApp::new().await;
    let client = app.client();

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

#[tokio::test]
async fn refresh_valid_token_returns_200() {
    let app = TestApp::new().await;
    let client = app.client();

    let (_, auth) = register_user(&client, "refresh@test.local", "StrongPassword1!").await;
    let (status, refreshed) =
        refresh_token(&client, auth.refresh_token.expose_secret()).await;

    assert_eq!(status, http::StatusCode::OK);
    assert!(!refreshed.access_token.expose_secret().is_empty());
    assert!(!refreshed.refresh_token.expose_secret().is_empty());
}

#[tokio::test]
async fn refresh_invalid_token_returns_401() {
    let app = TestApp::new().await;
    let client = app.client();

    let status = client
        .post_json(
            "/api/v1/auth/refresh",
            &serde_json::json!({ "refresh_token": "totally-invalid-token" }),
        )
        .send_status()
        .await;
    assert_eq!(status, http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn refresh_rotates_token() {
    let app = TestApp::new().await;
    let client = app.client();

    let (_, auth) = register_user(&client, "rotate@test.local", "StrongPassword1!").await;
    let old_refresh = auth.refresh_token.expose_secret().to_string();

    // Use the refresh token once — should succeed.
    let (s1, new_tokens) = refresh_token(&client, &old_refresh).await;
    assert_eq!(s1, http::StatusCode::OK);

    // Using the old refresh token again should fail (it was rotated).
    let s2 = client
        .post_json(
            "/api/v1/auth/refresh",
            &serde_json::json!({ "refresh_token": old_refresh }),
        )
        .send_status()
        .await;
    assert_eq!(s2, http::StatusCode::UNAUTHORIZED);

    // The new refresh token should still work.
    let (s3, _) =
        refresh_token(&client, new_tokens.refresh_token.expose_secret()).await;
    assert_eq!(s3, http::StatusCode::OK);
}

#[tokio::test]
async fn me_with_valid_jwt_returns_200() {
    let app = TestApp::new().await;
    let client = app.client();

    let (_, auth) = register_user(&client, "me@test.local", "StrongPassword1!").await;
    let (status, user): (_, serde_json::Value) = client
        .get("/api/v1/auth/me")
        .bearer(auth.access_token.expose_secret())
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(user["email"], "me@test.local");
}

#[tokio::test]
async fn me_without_auth_returns_401() {
    let app = TestApp::new().await;
    let client = app.client();

    let status = client.get("/api/v1/auth/me").send_status().await;
    assert_eq!(status, http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn logout_revokes_tokens() {
    let app = TestApp::new().await;
    let client = app.client();

    let (_, auth) = register_user(&client, "logout@test.local", "StrongPassword1!").await;
    let access = auth.access_token.expose_secret();
    let refresh = auth.refresh_token.expose_secret().to_string();

    // Logout.
    let status = client
        .post_json(
            "/api/v1/auth/logout",
            &serde_json::json!({ "refresh_token": refresh }),
        )
        .bearer(access)
        .send_status()
        .await;
    assert_eq!(status, http::StatusCode::NO_CONTENT);

    // Refresh should now fail.
    let s2 = client
        .post_json(
            "/api/v1/auth/refresh",
            &serde_json::json!({ "refresh_token": refresh }),
        )
        .send_status()
        .await;
    assert_eq!(s2, http::StatusCode::UNAUTHORIZED);
}
