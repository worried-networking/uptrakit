//! `cargo xtask audit-coverage-check` — ensure every state-changing site has an
//! audit-catalog decision.
//!
//! Runs three static-analysis passes over the workspace source tree:
//!
//! 1. **Catalog coverage**: every detected state-changing call site must have a
//!    catalog entry.
//! 2. **Registry cross-reference**: every catalog entry that names an action
//!    must refer to a registered action constant.
//! 3. **Emit sweep**: every registered `Stateful` action must have at least one
//!    `emit` call site in the source tree.

pub mod catalog;
pub mod emit_sweep;
pub mod registry;
pub mod walker;

use std::path::Path;
use std::process::ExitCode;

/// Subcommand entry point: run the three coverage passes against `root`.
///
/// Exits with code `0` on success, `1` when coverage violations are found, or
/// `2` when a fatal I/O or parse error prevents the analysis from running.
#[must_use]
pub fn cli(root: &Path) -> ExitCode {
    let catalog_path = root.join("crates/shared/audit-log/audit-catalog.toml");
    let action_type_path = root.join("crates/shared/audit-log/src/action_type.rs");

    let catalog = match catalog::load(&catalog_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("failed to read catalog: {e}");
            return ExitCode::from(2);
        }
    };
    let registry = match registry::load(&action_type_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("failed to read registry: {e}");
            return ExitCode::from(2);
        }
    };

    let mut failed = false;

    match walker::scan(root, &catalog, &registry) {
        Ok(report) => {
            for s in &report.missing_catalog_entry {
                eprintln!("missing catalog entry: {s}");
                failed = true;
            }
            for s in &report.unknown_action {
                eprintln!("catalog action not registered: {s}");
                failed = true;
            }
            for s in &report.stale_skip {
                eprintln!("stale catalog skip (site not found): {s}");
                failed = true;
            }
        }
        Err(e) => {
            eprintln!("walker failed: {e}");
            return ExitCode::from(2);
        }
    }

    match emit_sweep::scan(root, &registry) {
        Ok(report) => {
            for a in &report.stateful_actions_without_emit_site {
                eprintln!("registered Stateful action with no emit call site: {a}");
                failed = true;
            }
        }
        Err(e) => {
            eprintln!("emit sweep failed: {e}");
            return ExitCode::from(2);
        }
    }

    if failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
