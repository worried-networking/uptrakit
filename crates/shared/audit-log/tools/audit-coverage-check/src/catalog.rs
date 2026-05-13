//! Audit catalog loader.
//!
//! Reads `audit-catalog.toml` and deserialises it into a [`Catalog`] value.
//! The catalog maps call-site patterns to registered audit actions (or
//! explicit skip decisions).

use serde::Deserialize;

/// The top-level structure of `audit-catalog.toml`.
#[derive(Deserialize, Debug)]
pub struct Catalog {
    /// All catalog entries, one per call-site decision.
    pub entries: Vec<Entry>,
}

/// A single call-site decision in the catalog.
#[derive(Deserialize, Debug)]
pub struct Entry {
    /// Unique identifier for the call site (e.g. `"web_api::routes::hosts::create"`).
    pub site: String,
    /// Registered audit action constant name to emit at this site, if any.
    pub action: Option<String>,
    /// Reason this site is intentionally skipped, if applicable.
    pub skip: Option<String>,
}

/// Load the audit catalog from the TOML file at `path`.
///
/// Returns an empty catalog (no entries) when the file does not yet exist,
/// so the tool can be run in a freshly-bootstrapped workspace without error.
///
/// # Errors
///
/// Returns a descriptive string if the file exists but cannot be read or parsed.
pub fn load(path: &std::path::Path) -> Result<Catalog, String> {
    if !path.exists() {
        return Ok(Catalog { entries: vec![] });
    }

    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;

    toml::from_str::<Catalog>(&content).map_err(|e| format!("cannot parse {}: {e}", path.display()))
}
