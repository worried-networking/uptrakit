//! Source-tree walker for audit emit call sites.
//!
//! Walks the workspace source tree, identifies state-changing call sites, and
//! cross-references them against the [`Catalog`] and [`Registry`] to produce a
//! [`WalkReport`].

use crate::{catalog::Catalog, registry::Registry};

/// Results from scanning the source tree for audit call-site coverage.
#[derive(Debug)]
pub struct WalkReport {
    /// Call sites that were detected but have no catalog entry.
    pub missing_catalog_entry: Vec<String>,
    /// Catalog entries whose `action` value does not match any registered action.
    pub unknown_action: Vec<String>,
    /// Catalog entries whose `site` pattern was not found anywhere in the source tree.
    pub stale_skip: Vec<String>,
}

/// Scan the workspace rooted at `root` for audit call-site coverage.
///
/// Cross-references detected call sites with `catalog` and `registry` to
/// populate a [`WalkReport`].
///
/// This is a stub implementation that always returns an empty report.
/// The full implementation will use `walkdir` and `syn` to parse source files.
///
/// # Errors
///
/// Returns a descriptive string if the workspace tree cannot be traversed.
pub fn scan(
    _root: &std::path::Path,
    _catalog: &Catalog,
    _registry: &Registry,
) -> Result<WalkReport, String> {
    Ok(WalkReport {
        missing_catalog_entry: vec![],
        unknown_action: vec![],
        stale_skip: vec![],
    })
}
