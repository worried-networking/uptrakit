use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
pub use uptrakit_web_api_types::settings_provider_github::{
    GitHubProviderSettingsResponse, UpdateGitHubProviderSettingsRequest,
};

use crate::AppState;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::permission::CanManageGlobalSettings;

const SECRET_MASK: &str = "***";

fn snapshot_to_response(
    defaults: &uptrakit_shared_db::provider_settings::GitHubProviderDefaults,
) -> GitHubProviderSettingsResponse {
    let has_auth_token = defaults
        .auth_token
        .as_deref()
        .is_some_and(|token| !token.is_empty());

    GitHubProviderSettingsResponse {
        api_base_url: defaults.api_base_url.clone().filter(|url| !url.is_empty()),
        has_auth_token,
        auth_token: has_auth_token.then(|| SECRET_MASK.to_string()),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/global-settings/providers/github",
    responses(
        (
            status = 200,
            description = "Shared GitHub provider defaults",
            body = GitHubProviderSettingsResponse
        ),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Global Settings",
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_github_provider_settings(
    State(state): State<Arc<AppState>>,
    CanManageGlobalSettings(_user): CanManageGlobalSettings,
) -> Response {
    let defaults = match uptrakit_shared_db::provider_settings::load_github_provider_defaults(
        state.db(),
    )
    .await
    {
        Ok(defaults) => defaults,
        Err(e) => {
            tracing::error!("Failed to load GitHub provider defaults: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    (StatusCode::OK, Json(snapshot_to_response(&defaults))).into_response()
}

#[utoipa::path(
    put,
    path = "/api/v1/global-settings/providers/github",
    request_body = UpdateGitHubProviderSettingsRequest,
    responses(
        (
            status = 200,
            description = "Shared GitHub provider defaults updated",
            body = GitHubProviderSettingsResponse
        ),
        (status = 400, description = "Invalid values"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Global Settings",
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_github_provider_settings(
    State(state): State<Arc<AppState>>,
    CanManageGlobalSettings(_user): CanManageGlobalSettings,
    Validated(req): Validated<UpdateGitHubProviderSettingsRequest>,
) -> Response {
    let current = match uptrakit_shared_db::provider_settings::load_github_provider_defaults(
        state.db(),
    )
    .await
    {
        Ok(defaults) => defaults,
        Err(e) => {
            tracing::error!("Failed to load GitHub provider defaults before update: {e:?}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let next = uptrakit_shared_db::provider_settings::GitHubProviderDefaults {
        auth_token: match req.auth_token.as_deref() {
            None | Some(SECRET_MASK) => current.auth_token.clone(),
            Some("") => None,
            Some(token) => Some(token.trim().to_string()),
        },
        api_base_url: match req.api_base_url.as_deref() {
            None => current.api_base_url.clone(),
            Some("") => None,
            Some(url) => Some(url.trim().to_string()),
        },
    };

    let normalized_next =
        match uptrakit_shared_db::provider_settings::normalize_github_provider_defaults(next) {
            Ok(Some(defaults)) => defaults,
            Ok(None) => uptrakit_shared_db::provider_settings::GitHubProviderDefaults::default(),
            Err(error) => {
                let problem = error.to_string();
                tracing::warn!("Rejected invalid GitHub provider defaults update: {problem}");
                return error_response(StatusCode::BAD_REQUEST, &problem);
            }
        };

    if let Err(e) = uptrakit_shared_db::provider_settings::upsert_github_provider_defaults(
        state.db(),
        normalized_next.auth_token.as_deref(),
        normalized_next.api_base_url.as_deref(),
    )
    .await
    {
        tracing::error!("Failed to save GitHub provider defaults: {e:?}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    state.global_providers().github().invalidate();

    crate::global_providers::github::emit_global_github_provider_diagnostic_if_needed(
        state.db(),
        &state.notification.event_broadcaster,
    )
    .await;

    (StatusCode::OK, Json(snapshot_to_response(&normalized_next))).into_response()
}
