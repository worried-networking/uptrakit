use rootcause::Report;

use crate::app_state::AppState;

pub trait EtagSource: Sized + Send + Sync + 'static {
    /// Returns the current ETag from the in-memory cache. Fast; used for GET responses.
    fn current_etag(state: &AppState) -> impl Future<Output = Result<String, Report>> + Send;

    /// Re-reads the version from the DB, syncs the cache, and returns the new ETag.
    /// Used after a successful mutation so the response carries the committed version.
    ///
    /// For GET-only resources this method is never called by `EtagLayer`. Implementors
    /// covering read-only resources may return `Err(report!("refresh not supported"))`.
    fn refresh_etag(state: &AppState) -> impl Future<Output = Result<String, Report>> + Send;
}
