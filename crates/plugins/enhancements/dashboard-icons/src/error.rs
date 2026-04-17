/// Errors specific to the Dashboard Icons plugin.
#[derive(Debug, thiserror::Error)]
pub(crate) enum DashboardIconsError {
    /// HTTP request to fetch the icon index failed.
    #[error("icon index fetch failed: {0}")]
    IndexFetch(String),
}

pub(crate) type Result<T> = std::result::Result<T, rootcause::Report<DashboardIconsError>>;
