use std::marker::PhantomData;
use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::header::IF_MATCH;
use axum::http::request::Parts;
use uptrakit_config_reload::config::Scope;

use crate::app_state::AppState;
use crate::extractors::etag_source::EtagSource;

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
pub struct IfMatch<T: EtagSource> {
    /// The raw `If-Match` header value provided by the client.
    pub client_etag: String,
    _marker: PhantomData<T>,
}

impl<T: EtagSource> IfMatch<T> {
    /// Strip `W/` prefix and surrounding `"` quotes from an ETag string.
    fn strip_etag(s: &str) -> &str {
        s.trim_start_matches("W/").trim_matches('"')
    }

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
    type Rejection = (StatusCode, String);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let header = parts.headers.get(IF_MATCH).ok_or((
            StatusCode::PRECONDITION_REQUIRED,
            "If-Match header is required".to_string(),
        ))?;
        let client = header
            .to_str()
            .map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    format!("If-Match parse error: {e}"),
                )
            })?
            .to_string();
        let current = T::current_etag(parts, state).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("etag lookup failed: {e}"),
            )
        })?;
        if Self::strip_etag(&client) != Self::strip_etag(&current) {
            return Err((
                StatusCode::CONFLICT,
                "ETag mismatch (stale settings_version)".to_string(),
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

#[async_trait::async_trait]
impl EtagSource for SettingsVersion {
    async fn current_etag(_parts: &Parts, state: &AppState) -> Result<String, rootcause::Report> {
        let version = state
            .settings_version_cache
            .get(Scope::Tenant(state.default_tenant_id))
            .unwrap_or(0);
        Ok(format!("W/\"settings-v{version}\""))
    }
}
