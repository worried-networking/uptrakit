use audit_coverage_check::{emit_sweep, registry};

/// Verify that every registered Stateful action has at least one emit call site
/// in the real workspace source tree (Plan C Task 7).
#[test]
fn no_stateful_actions_missing_emit_sites_in_workspace() {
    // From crates/shared/audit-log/tools/audit-coverage-check/ → workspace root (5 up).
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../../")
        .canonicalize()
        .expect("workspace root must exist");

    let registry_path = workspace_root.join("crates/shared/audit-log/src/action_type.rs");
    let registry = registry::load(&registry_path).expect("parse action registry");

    let report = emit_sweep::scan(&workspace_root, &registry).expect("emit sweep");

    assert!(
        report.stateful_actions_without_emit_site.is_empty(),
        "Stateful actions with no emit call site: {:#?}",
        report.stateful_actions_without_emit_site
    );
}
