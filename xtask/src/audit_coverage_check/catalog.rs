//! Audit catalog loader.
//!
//! Reads `audit-catalog.toml` and deserialises it into a [`Catalog`] value.
//! The catalog maps call-site patterns to registered audit actions (or
//! explicit skip decisions).

use serde::Deserialize;
use std::path::Path;

/// The top-level structure of `audit-catalog.toml`.
#[derive(Deserialize, Debug)]
#[non_exhaustive]
pub struct Catalog {
    /// All catalog entries, one per call-site decision.
    pub entries: Vec<Entry>,
}

/// A single call-site decision in the catalog.
#[derive(Deserialize, Debug)]
#[non_exhaustive]
pub struct Entry {
    /// Fully-qualified Rust path, e.g. `"uptrakit_web_api::routes::plugin_configs::create"`.
    pub site: String,
    /// Registered action key (audited), e.g. `"plugin_config.create"`.
    pub action: Option<String>,
    /// Reason this site is intentionally not audited.
    pub skip: Option<String>,
}

/// Load the audit catalog from the TOML file at `path`.
///
/// Returns an error if the file cannot be read, cannot be parsed, or contains
/// an entry that sets both `action` and `skip` (or neither).
///
/// # Errors
///
/// Returns a descriptive string if the file cannot be read, parsed, or
/// contains an entry with neither or both of `action` / `skip` set.
pub fn load(path: &Path) -> Result<Catalog, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let catalog: Catalog = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    for e in &catalog.entries {
        if e.action.is_some() == e.skip.is_some() {
            return Err(format!(
                "catalog entry '{}': must set exactly one of `action` or `skip`",
                e.site
            ));
        }
    }
    Ok(catalog)
}
