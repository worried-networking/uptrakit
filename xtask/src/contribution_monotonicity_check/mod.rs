//! `cargo xtask contribution-monotonicity-check` — ADR-0032 Layer B.
//! Runs the registry `dump_contributions` example in two feature lanes and
//! asserts: fingerprint keys ≡ registry [features] table; baseline
//! fingerprint ≡ expected_baseline_fingerprint(); union fingerprint all-true;
//! per-plugin contribution supersets; baseline is a PROPER subset (canary).

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, ExitCode};

use serde::Deserialize;

#[derive(Deserialize)]
struct Dump {
    features: BTreeMap<String, bool>,
    plugins: BTreeMap<String, PluginDump>,
}

#[derive(Deserialize, PartialEq)]
struct PluginDump {
    surfaces: BTreeMap<String, SurfaceDump>,
    agent_interactions: Vec<String>,
    migrations: Vec<String>,
    agent_migrations: Vec<String>,
}

#[derive(Deserialize, PartialEq)]
struct SurfaceDump {
    interactions: Vec<(String, String)>,
    data_sources: Vec<String>,
}

/// Baseline = registry's effective feature set inside the lean
/// `uptrakit-controller` resolution. Derived 2026-07-27 via the ONLY
/// permitted source (package-scoped resolve):
///   cargo tree -p uptrakit-controller -e features
/// Unscoped `cargo metadata` is FORBIDDEN here — it reports the
/// workspace-unified resolve (agent-infra ON = the union shape), which would
/// make this diff vacuous (ADR-0032). Re-derive with the same command when
/// the fingerprint key-diff below forces a re-pin.
const BASELINE_FEATURES: &[&str] = &[
    "plugin-ops",
    "migrations",
    "notifications",
    "notifications-email",
    "notifications-telegram",
    "notifications-webhook",
    "dashboard-icons",
];

/// Hand-authored, independent of any derivation tooling (ADR-0032).
/// Keys must cover the registry [features] table exactly (key-diff enforced).
fn expected_baseline_fingerprint() -> BTreeMap<String, bool> {
    let mut m = BTreeMap::new();
    for k in ["default", "daemon", "ssh", "agent-infra", "test-support"] {
        m.insert(k.to_string(), false);
    }
    for k in BASELINE_FEATURES {
        m.insert((*k).to_string(), true);
    }
    m
}

/// Fingerprint keys must equal the registry crate's declared features table.
fn fingerprint_key_violations(
    fingerprint: &BTreeMap<String, bool>,
    declared: &[String],
) -> Vec<String> {
    let mut violations = Vec::new();
    for key in declared {
        if !fingerprint.contains_key(key) {
            violations.push(format!(
                "fingerprint missing declared feature key `{key}` — extend \
                 feature_fingerprint() in dump_contributions.rs"
            ));
        }
    }
    for key in fingerprint.keys() {
        if !declared.contains(key) {
            violations.push(format!(
                "fingerprint carries key `{key}` not declared in registry [features]"
            ));
        }
    }
    violations
}

/// Every contribution present in baseline must be present in union.
fn superset_violations(baseline: &Dump, union: &Dump) -> Vec<String> {
    let mut violations = Vec::new();
    for (plugin, base) in &baseline.plugins {
        let Some(uni) = union.plugins.get(plugin) else {
            violations.push(format!("{plugin}: present in baseline, absent in union"));
            continue;
        };
        for (surface_id, base_surface) in &base.surfaces {
            let Some(uni_surface) = uni.surfaces.get(surface_id) else {
                violations.push(format!(
                    "{plugin}: surface `{surface_id}` present in baseline, absent in union"
                ));
                continue;
            };
            for pair in &base_surface.interactions {
                if !uni_surface.interactions.contains(pair) {
                    violations.push(format!(
                        "{plugin}: `{surface_id}` interaction ({}, {}) dropped in union",
                        pair.0, pair.1
                    ));
                }
            }
            for ds in &base_surface.data_sources {
                if !uni_surface.data_sources.contains(ds) {
                    violations.push(format!(
                        "{plugin}: `{surface_id}` data source `{ds}` dropped in union"
                    ));
                }
            }
        }
        for (label, base_list, uni_list) in [
            (
                "agent interaction",
                &base.agent_interactions,
                &uni.agent_interactions,
            ),
            ("migration", &base.migrations, &uni.migrations),
            (
                "agent migration",
                &base.agent_migrations,
                &uni.agent_migrations,
            ),
        ] {
            for id in base_list.iter() {
                if !uni_list.contains(id) {
                    violations.push(format!("{plugin}: {label} `{id}` dropped in union"));
                }
            }
        }
    }
    violations
}

/// Non-vacuity canary: the diff is meaningless if the lanes are identical.
fn lanes_identical(baseline: &Dump, union: &Dump) -> bool {
    baseline.plugins == union.plugins
}

/// Positive canary (spec Decision 3 step 4: "at least one KNOWN union-only
/// contribution present"): the agent-infra axis specifically must separate
/// the lanes. Mere lane inequality is not enough — union enables many
/// features, so the maps can differ for reasons unrelated to this axis
/// while agent-infra is silently collapsed.
fn agent_infra_axis_violations(baseline: &Dump, union: &Dump) -> Vec<String> {
    let mut violations = Vec::new();
    match union.plugins.get("infrastructure.proxmox") {
        None => violations.push("union lacks infrastructure.proxmox plugin".to_string()),
        Some(p) if p.agent_interactions.is_empty() => violations.push(
            "union infrastructure.proxmox has no agent interactions — \
             agent-infra axis collapsed (ADR-0032 canary)"
                .to_string(),
        ),
        Some(_) => {}
    }
    if let Some(p) = baseline.plugins.get("infrastructure.proxmox")
        && !p.agent_interactions.is_empty()
    {
        violations.push(
            "baseline infrastructure.proxmox has agent interactions — \
             baseline lane inflated with agent-infra (ADR-0032 canary)"
                .to_string(),
        );
    }
    violations
}

/// Declared feature keys for `uptrakit-plugin-infrastructure-registry`, read
/// from `cargo metadata --format-version 1 --no-deps`. Package-scoped
/// invocation (`cargo metadata -p <pkg>`) errors with "unexpected argument",
/// so the whole workspace graph is fetched and the package located by name.
fn declared_features(root: &Path) -> Result<Vec<String>, String> {
    let output = Command::new("cargo")
        .current_dir(root)
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .map_err(|e| format!("failed to spawn `cargo metadata`: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "`cargo metadata` failed (status {}): {stderr}",
            output.status
        ));
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("failed to parse `cargo metadata` JSON: {e}"))?;
    let packages = value
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "`cargo metadata` JSON missing `packages` array".to_string())?;
    let package = packages
        .iter()
        .find(|p| {
            p.get("name").and_then(serde_json::Value::as_str)
                == Some("uptrakit-plugin-infrastructure-registry")
        })
        .ok_or_else(|| {
            "package `uptrakit-plugin-infrastructure-registry` not found in `cargo metadata` \
             output"
                .to_string()
        })?;
    let features = package
        .get("features")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| "package entry missing `features` object".to_string())?;
    Ok(features.keys().cloned().collect())
}

/// Runs the registry's `dump_contributions` example in one feature lane and
/// parses its single-line JSON document.
fn run_dump(root: &Path, all_features: bool) -> Result<Dump, String> {
    let mut cmd = Command::new("cargo");
    cmd.current_dir(root);
    cmd.args([
        "run",
        "-p",
        "uptrakit-plugin-infrastructure-registry",
        "--example",
        "dump_contributions",
    ]);
    if all_features {
        cmd.arg("--all-features");
    } else {
        // Dedicated target dir so alternating feature sets does not thrash
        // the shared cache (spec Decision 3, CI-wiring paragraph).
        cmd.env(
            "CARGO_TARGET_DIR",
            root.join("target/contribution-baseline"),
        );
        cmd.args([
            "--no-default-features",
            "--features",
            &BASELINE_FEATURES.join(","),
        ]);
    }

    let output = cmd
        .output()
        .map_err(|e| format!("failed to spawn `cargo run`: {e}"))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(format!(
            "`cargo run` failed (status {}); stderr: {stderr}",
            output.status
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let last_line = stdout
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .ok_or_else(|| format!("no output line from dump_contributions; stderr: {stderr}"))?;
    serde_json::from_str(last_line)
        .map_err(|e| format!("failed to parse dump JSON: {e}; stderr: {stderr}"))
}

/// Subcommand entry point: run both feature lanes and assert monotonicity.
///
/// Exits with code `0` on success, `1` when a monotonicity violation is
/// found, or `2` when a fatal process/parse error prevents the checks from
/// running at all.
#[must_use]
pub fn cli(root: &Path) -> ExitCode {
    let declared = match declared_features(root) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("failed to read declared features: {e}");
            return ExitCode::from(2);
        }
    };

    let baseline = match run_dump(root, false) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("failed to run baseline dump: {e}");
            return ExitCode::from(2);
        }
    };

    let union = match run_dump(root, true) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("failed to run union dump: {e}");
            return ExitCode::from(2);
        }
    };

    let mut failed = false;

    for v in fingerprint_key_violations(&baseline.features, &declared) {
        eprintln!("baseline {v}");
        failed = true;
    }
    for v in fingerprint_key_violations(&union.features, &declared) {
        eprintln!("union {v}");
        failed = true;
    }

    let expected = expected_baseline_fingerprint();
    for (key, expected_value) in &expected {
        match baseline.features.get(key) {
            Some(actual) if actual == expected_value => {}
            Some(actual) => {
                eprintln!(
                    "baseline fingerprint mismatch: `{key}` expected {expected_value}, got {actual}"
                );
                failed = true;
            }
            None => {
                eprintln!("baseline fingerprint missing key `{key}`");
                failed = true;
            }
        }
    }
    for key in baseline.features.keys() {
        if !expected.contains_key(key) {
            eprintln!("baseline fingerprint has unexpected key `{key}`");
            failed = true;
        }
    }

    for (key, value) in &union.features {
        if !value {
            eprintln!("union fingerprint key `{key}` is false, expected true under --all-features");
            failed = true;
        }
    }

    for v in superset_violations(&baseline, &union) {
        eprintln!("{v}");
        failed = true;
    }

    if lanes_identical(&baseline, &union) {
        eprintln!(
            "baseline == union: the diff is vacuous; a non-registry-feature axis inflated the \
             baseline (ADR-0032 canary)"
        );
        failed = true;
    }

    for v in agent_infra_axis_violations(&baseline, &union) {
        eprintln!("{v}");
        failed = true;
    }

    if baseline.plugins.is_empty() || union.plugins.is_empty() {
        eprintln!("empty plugins map: dump capture/parse silently failed");
        failed = true;
    }

    if failed {
        ExitCode::FAILURE
    } else {
        println!("contribution-monotonicity-check: OK");
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dump_from(value: serde_json::Value) -> Dump {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn key_diff_flags_missing_and_extra() {
        let fingerprint: BTreeMap<String, bool> =
            serde_json::from_value(serde_json::json!({"default": false, "ghost": true})).unwrap();
        let declared = vec!["default".to_string(), "new-feat".to_string()];
        let violations = fingerprint_key_violations(&fingerprint, &declared);
        assert_eq!(violations.len(), 2);
        assert!(violations.iter().any(|v| v.contains("new-feat")));
        assert!(violations.iter().any(|v| v.contains("ghost")));
    }

    #[test]
    fn superset_detects_dropped_surface() {
        let baseline = dump_from(serde_json::json!({
            "features": {},
            "plugins": {
                "p": {
                    "surfaces": {
                        "a.b": {"interactions": [], "data_sources": []}
                    },
                    "agent_interactions": [],
                    "migrations": [],
                    "agent_migrations": []
                }
            }
        }));
        let union = dump_from(serde_json::json!({
            "features": {},
            "plugins": {
                "p": {
                    "surfaces": {},
                    "agent_interactions": [],
                    "migrations": [],
                    "agent_migrations": []
                }
            }
        }));
        let violations = superset_violations(&baseline, &union);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains('p'));
        assert!(violations[0].contains("a.b"));
    }

    #[test]
    fn superset_detects_dropped_interaction_and_migration() {
        let baseline = dump_from(serde_json::json!({
            "features": {},
            "plugins": {
                "p": {
                    "surfaces": {
                        "a.b": {
                            "interactions": [["do-thing", "POST"]],
                            "data_sources": []
                        }
                    },
                    "agent_interactions": [],
                    "migrations": ["m_1"],
                    "agent_migrations": []
                }
            }
        }));
        let union = dump_from(serde_json::json!({
            "features": {},
            "plugins": {
                "p": {
                    "surfaces": {
                        "a.b": {"interactions": [], "data_sources": []}
                    },
                    "agent_interactions": [],
                    "migrations": [],
                    "agent_migrations": []
                }
            }
        }));
        let violations = superset_violations(&baseline, &union);
        assert_eq!(violations.len(), 2);
        assert!(violations.iter().any(|v| v.contains("do-thing")));
        assert!(violations.iter().any(|v| v.contains("m_1")));
    }

    #[test]
    fn superset_accepts_union_only_additions() {
        let baseline = dump_from(serde_json::json!({
            "features": {},
            "plugins": {
                "p": {
                    "surfaces": {
                        "a.b": {"interactions": [], "data_sources": []}
                    },
                    "agent_interactions": [],
                    "migrations": [],
                    "agent_migrations": []
                }
            }
        }));
        let union = dump_from(serde_json::json!({
            "features": {},
            "plugins": {
                "p": {
                    "surfaces": {
                        "a.b": {"interactions": [], "data_sources": []},
                        "c.d": {"interactions": [], "data_sources": []}
                    },
                    "agent_interactions": [],
                    "migrations": [],
                    "agent_migrations": []
                },
                "q": {
                    "surfaces": {},
                    "agent_interactions": [],
                    "migrations": [],
                    "agent_migrations": []
                }
            }
        }));
        let violations = superset_violations(&baseline, &union);
        assert!(violations.is_empty());
    }

    #[test]
    fn identical_lanes_detected() {
        let dump = dump_from(serde_json::json!({
            "features": {},
            "plugins": {
                "p": {
                    "surfaces": {},
                    "agent_interactions": [],
                    "migrations": [],
                    "agent_migrations": []
                }
            }
        }));
        let other = dump_from(serde_json::json!({
            "features": {},
            "plugins": {
                "p": {
                    "surfaces": {},
                    "agent_interactions": [],
                    "migrations": [],
                    "agent_migrations": []
                }
            }
        }));
        assert!(lanes_identical(&dump, &other));
    }

    #[test]
    fn agent_infra_canary_flags_collapsed_axis() {
        let baseline_healthy = dump_from(serde_json::json!({
            "features": {},
            "plugins": {
                "infrastructure.proxmox": {
                    "surfaces": {},
                    "agent_interactions": [],
                    "migrations": [],
                    "agent_migrations": []
                }
            }
        }));
        let union_collapsed = dump_from(serde_json::json!({
            "features": {},
            "plugins": {
                "infrastructure.proxmox": {
                    "surfaces": {},
                    "agent_interactions": [],
                    "migrations": [],
                    "agent_migrations": []
                }
            }
        }));
        let violations = agent_infra_axis_violations(&baseline_healthy, &union_collapsed);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("agent-infra axis collapsed"));

        let baseline_inflated = dump_from(serde_json::json!({
            "features": {},
            "plugins": {
                "infrastructure.proxmox": {
                    "surfaces": {},
                    "agent_interactions": ["discovered-guests"],
                    "migrations": [],
                    "agent_migrations": []
                }
            }
        }));
        let union_healthy = dump_from(serde_json::json!({
            "features": {},
            "plugins": {
                "infrastructure.proxmox": {
                    "surfaces": {},
                    "agent_interactions": ["discovered-guests"],
                    "migrations": [],
                    "agent_migrations": []
                }
            }
        }));
        let violations = agent_infra_axis_violations(&baseline_inflated, &union_healthy);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("baseline lane inflated"));

        let violations = agent_infra_axis_violations(&baseline_healthy, &union_healthy);
        assert!(violations.is_empty());
    }
}
