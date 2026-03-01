pub mod api_types;
pub mod config;
pub mod error;

pub mod plugin;
pub mod tag;

pub use config::GitLabConfig;
pub use error::{GitLabError, Result};

pub use plugin::{GitLabPlugin, parse_project_path};

/// Validate a GitLab package identifier string.
///
/// A valid identifier has at least one `/`, all path components are non-empty,
/// and no component may contain `..` (path traversal guard).
///
/// GitLab supports nested namespaces such as `group/subgroup/project`, so
/// identifiers may have more than one `/`.
///
/// This function is used by the plugin registry's `validate_package_identifier`
/// dispatch to reject invalid identifiers before they reach the GitLab API.
pub fn validate_identifier(value: &str) -> std::result::Result<(), String> {
    parse_project_path(value)
        .map(|_| ())
        .map_err(|e| e.to_string())
}
