//! `cargo xtask openapi-client-check` — assert the client tracks the OpenAPI spec.

pub mod check;
pub mod client;
pub mod ledgers;
pub mod normalize;
pub mod spec;

pub use check::Violation;

use std::path::Path;
use std::process::ExitCode;

/// Run all coverage assertions against the workspace, returning every violation.
///
/// # Errors
/// Returns an error string on ledger double-booking or on any I/O / parse failure.
pub fn run(root: &Path) -> Result<Vec<Violation>, String> {
    ledgers::validate_no_double_booking()?;
    let spec_json = std::fs::read_to_string(root.join("crates/ui/web-api/openapi.json"))
        .map_err(|e| e.to_string())?;
    let ops = spec::load(&spec_json)?;
    let client_src = root.join("crates/shared/openapi-client/src");
    let methods = client::collect_methods(&client_src)?;
    let templates = client::collect_path_templates(&client_src)?;
    // Exclude paths where every operation is in SPEC_ONLY — those have no client methods and
    // therefore no path template is expected or needed.
    let mut spec_paths: Vec<String> = ops
        .iter()
        .filter(|o| !ledgers::SPEC_ONLY.contains(&o.operation_id.as_str()))
        .map(|o| o.path.clone())
        .collect();
    spec_paths.sort();
    spec_paths.dedup();

    let mut violations = check::check_names(&ops, &methods);
    violations.extend(check::check_paths(&spec_paths, &templates));
    violations.extend(check::check_stale_ledgers(&ops, &methods, &templates));
    Ok(violations)
}

/// Subcommand entry point: run the check and map the outcome to an exit code.
#[must_use]
pub fn cli(root: &Path) -> ExitCode {
    match run(root) {
        Ok(v) if v.is_empty() => ExitCode::SUCCESS,
        Ok(v) => {
            eprintln!("openapi-client drift ({} issue(s)):", v.len());
            for x in &v {
                eprintln!("  {x}");
            }
            eprintln!(
                "\nFix: add the client method/path, or record the divergence in a ledger \
                 (xtask/src/openapi_client_check/ledgers.rs)."
            );
            ExitCode::from(1)
        }
        Err(e) => {
            eprintln!("fatal: {e}");
            ExitCode::from(2)
        }
    }
}
