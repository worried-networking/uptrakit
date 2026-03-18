pub mod api_types;
pub mod config;
pub mod error;

pub mod plugin;
pub mod tag;

pub use config::GitHubConfig;
pub use error::{GitHubError, Result};

pub use plugin::{DESCRIPTOR, GitHubPlugin, parse_owner_repo};

/// Validate a GitHub package identifier string.
///
/// A valid identifier has exactly one `/`, with non-empty `owner` and `repo`
/// parts, and neither part may contain `..`.
///
/// This function is used by the plugin registry's `validate_package_identifier`
/// dispatch to reject invalid identifiers before they reach the GitHub API.
pub fn validate_identifier(value: &str) -> std::result::Result<(), String> {
    parse_owner_repo(value)
        .map(|_| ())
        .map_err(|e| e.to_string())
}
