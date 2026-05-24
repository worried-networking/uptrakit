use std::marker::PhantomData;
use std::sync::Arc;

use axum::Json;
use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::header::IF_MATCH;
use axum::http::request::Parts;
use rootcause::Report;
use uptrakit_config_reload::config::Scope;
use uptrakit_web_api_types::error::ErrorResponse;

use crate::app_state::AppState;
use crate::extractors::etag_source::EtagSource;
use crate::settings_store::get_settings_versions;

/// Axum extractor that enforces optimistic locking via the HTTP `If-Match` header.
///
/// `T` must implement [`EtagSource`], which provides the current ETag for the
/// resource.  The extractor:
///
/// 1. Rejects the request with `428 Precondition Required` when no `If-Match`
///    header is present.
/// 2. Rejects the request with `409 Conflict` when the client's ETag does not
///    match the current version (stale `settings_version`).
/// 3. Returns the extractor value (holding `client_etag`) when they match.
///
/// The `_marker` field uses [`PhantomData`] to bind `T` without storing it,
/// preserving zero-cost dispatch at the use site.
///
/// # Example
///
/// ```rust,ignore
/// pub async fn update_settings(
///     State(state): State<Arc<AppState>>,
///     _if_match: IfMatch<SettingsVersion>,
///     Json(body): Json<UpdateRequest>,
/// ) -> impl IntoResponse { … }
/// ```
#[non_exhaustive]
pub struct IfMatch<T: EtagSource> {
    /// The raw `If-Match` header value provided by the client.
    pub client_etag: String,
    _marker: PhantomData<T>,
}

/// Strip `W/` prefix and surrounding `"` quotes from an ETag string.
pub(crate) fn strip_etag(s: &str) -> &str {
    s.strip_prefix("W/").unwrap_or(s).trim_matches('"')
}

impl<T: EtagSource> IfMatch<T> {
    /// Construct a pre-approved [`IfMatch`] value for use in unit tests that
    /// call handler functions directly (bypassing axum's extractor pipeline).
    ///
    /// This constructor is only available in test builds.  Production code must
    /// go through the axum extractor which enforces the `If-Match` header check.
    #[cfg(test)]
    pub fn for_test() -> Self {
        Self {
            client_etag: String::new(),
            _marker: PhantomData,
        }
    }
}

impl<T: EtagSource> FromRequestParts<Arc<AppState>> for IfMatch<T> {
    type Rejection = (StatusCode, Json<ErrorResponse>);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let header = parts.headers.get(IF_MATCH).ok_or_else(|| {
            (
                StatusCode::PRECONDITION_REQUIRED,
                Json(ErrorResponse {
                    error: "if-match header is required".to_string(),
                    code: Some("if_match.required".to_string()),
                }),
            )
        })?;
        let client = header
            .to_str()
            .map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("if-match parse error: {e}"),
                        code: Some("if_match.parse_error".to_string()),
                    }),
                )
            })?
            .to_string();
        let current = T::current_etag(state).await.map_err(|e| {
            tracing::error!(error = %e, "settings version etag lookup failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "etag lookup failed".to_string(),
                    code: Some("if_match.lookup_failed".to_string()),
                }),
            )
        })?;
        if strip_etag(&client) != strip_etag(&current) {
            return Err((
                StatusCode::CONFLICT,
                Json(ErrorResponse {
                    error: "etag mismatch (stale settings_version)".to_string(),
                    code: Some("if_match.stale".to_string()),
                }),
            ));
        }
        Ok(Self {
            client_etag: client,
            _marker: PhantomData,
        })
    }
}

/// [`EtagSource`] implementation backed by [`uptrakit_config_reload::SettingsVersionCache`].
///
/// Returns a weak ETag of the form `W/"settings-v{n}"` where `n` is the
/// current per-tenant settings version counter (0 when not yet populated).
pub struct SettingsVersion;

impl EtagSource for SettingsVersion {
    async fn current_etag(state: &AppState) -> Result<String, Report> {
        // SINGLE-TENANT ASSUMPTION
        let version = state
            .settings_version_cache
            .get(Scope::Tenant(state.default_tenant_id))
            .unwrap_or(0);
        Ok(format!("W/\"settings-v{version}\""))
    }

    async fn refresh_etag(state: &AppState) -> Result<String, Report> {
        // SINGLE-TENANT ASSUMPTION
        let (tenant_v, _) = get_settings_versions(state.db(), state.default_tenant_id).await?;
        let version = u64::try_from(tenant_v).unwrap_or_else(|_| {
            tracing::warn!(
                tenant_v,
                "settings_version negative or overflow; treating as 0"
            );
            0
        });
        state
            .settings_version_cache
            .update(Scope::Tenant(state.default_tenant_id), version);
        Ok(format!("W/\"settings-v{version}\""))
    }
}

/// [`EtagSource`] backed by the global-settings version counter.
///
/// Returns a weak ETag of the form `W/"global-settings-v{n}"` where `n` is
/// `Scope::Global` from [`uptrakit_config_reload::SettingsVersionCache`].
/// Use this for endpoints that read/write `global_settings` (not per-tenant
/// `settings`), where the reconciler tracks changes under [`Scope::Global`].
pub struct GlobalSettingsVersion;

impl EtagSource for GlobalSettingsVersion {
    async fn current_etag(state: &AppState) -> Result<String, Report> {
        let version = state.settings_version_cache.get(Scope::Global).unwrap_or(0);
        Ok(format!("W/\"global-settings-v{version}\""))
    }

    async fn refresh_etag(state: &AppState) -> Result<String, Report> {
        // SINGLE-TENANT ASSUMPTION
        let (_, global_v) = get_settings_versions(state.db(), state.default_tenant_id).await?;
        let version = u64::try_from(global_v).unwrap_or_else(|_| {
            tracing::warn!(
                global_v,
                "global_version negative or overflow; treating as 0"
            );
            0
        });
        state.settings_version_cache.update(Scope::Global, version);
        Ok(format!("W/\"global-settings-v{version}\""))
    }
}
