use async_trait::async_trait;
use axum::http::request::Parts;
use rootcause::Report;

use crate::app_state::AppState;

/// Provides the current ETag value for a given resource type.
///
/// Implementors produce a stable string that identifies the current version of
/// some resource.  The [`super::if_match::IfMatch`] extractor uses this to
/// compare against the `If-Match` header sent by the client.
///
/// # Errors
///
/// Returns an error when the version cannot be determined (e.g. a DB look-up
/// fails).  The extractor converts this into a `500 Internal Server Error`.
#[async_trait]
pub trait EtagSource: Sized + Send + Sync + 'static {
    async fn current_etag(parts: &Parts, state: &AppState) -> Result<String, Report>;
}
