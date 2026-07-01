//! Coverage assertions producing `Violation`s.

use super::{ledgers, spec::SpecOp};
use std::collections::HashSet;
use std::fmt;

/// A single coverage failure.
#[derive(Debug)]
pub struct Violation {
    pub kind: &'static str,
    pub detail: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.kind, self.detail)
    }
}

fn resolve(op_id: &str) -> &str {
    ledgers::RENAME_MAP
        .iter()
        .find(|(id, _)| *id == op_id)
        .map_or(op_id, |(_, method)| *method)
}

/// Assertion 1: every spec op maps to a method (or `SPEC_ONLY`); every method
/// maps back to an op (or `CLIENT_ONLY`, or is a `list_all_` companion).
#[must_use]
pub fn check_names(ops: &[SpecOp], methods: &[String]) -> Vec<Violation> {
    let method_set: HashSet<&str> = methods.iter().map(String::as_str).collect();
    let mut violations = Vec::new();
    let mut expected: HashSet<&str> = HashSet::new();

    for op in ops {
        if ledgers::SPEC_ONLY.contains(&op.operation_id.as_str()) {
            continue;
        }
        let method = resolve(&op.operation_id);
        expected.insert(method);
        if !method_set.contains(method) {
            violations.push(Violation {
                kind: "spec-only",
                detail: format!(
                    "operationId '{}' ({} {}) has no client method '{method}'",
                    op.operation_id, op.method, op.path
                ),
            });
        }
    }

    for m in methods {
        if ledgers::CLIENT_ONLY.contains(&m.as_str())
            || ledgers::is_list_all_companion(m, methods)
            || expected.contains(m.as_str())
        {
            continue;
        }
        violations.push(Violation {
            kind: "client-only",
            detail: format!("client method '{m}' has no spec operation"),
        });
    }
    violations
}

/// Assertion 2: `paths.rs` templates ↔ spec paths (normalized), minus
/// `PATHS_CLIENT_ONLY`. Deduplicates before comparing.
#[must_use]
pub fn check_paths(spec_paths: &[String], client_templates: &[String]) -> Vec<Violation> {
    let spec_set: HashSet<&str> = spec_paths.iter().map(String::as_str).collect();
    let client_set: HashSet<&str> = client_templates.iter().map(String::as_str).collect();
    let mut violations = Vec::new();

    for &t in &client_set {
        if !spec_set.contains(t) && !ledgers::PATHS_CLIENT_ONLY.contains(&t) {
            violations.push(Violation {
                kind: "dead-path-const",
                detail: format!("paths.rs template '{t}' matches no spec path"),
            });
        }
    }
    for &p in &spec_set {
        if !client_set.contains(p) {
            violations.push(Violation {
                kind: "unrouted-path",
                detail: format!("spec path '{p}' has no paths.rs template"),
            });
        }
    }
    violations
}

/// Flag ledger entries that no longer correspond to anything in the spec or
/// client, so removing an endpoint cannot leave a dead ledger row.
#[must_use]
pub fn check_stale_ledgers(
    ops: &[SpecOp],
    methods: &[String],
    templates: &[String],
) -> Vec<Violation> {
    let op_ids: HashSet<&str> = ops.iter().map(|o| o.operation_id.as_str()).collect();
    let method_set: HashSet<&str> = methods.iter().map(String::as_str).collect();
    let template_set: HashSet<&str> = templates.iter().map(String::as_str).collect();
    let mut v = Vec::new();

    for &(id, _) in ledgers::RENAME_MAP {
        if !op_ids.contains(id) {
            v.push(Violation {
                kind: "stale-ledger",
                detail: format!("RENAME_MAP operationId '{id}' no longer in spec"),
            });
        }
    }
    for &id in ledgers::SPEC_ONLY {
        if !op_ids.contains(id) {
            v.push(Violation {
                kind: "stale-ledger",
                detail: format!("SPEC_ONLY '{id}' no longer in spec"),
            });
        }
    }
    for &m in ledgers::CLIENT_ONLY {
        if !method_set.contains(m) {
            v.push(Violation {
                kind: "stale-ledger",
                detail: format!("CLIENT_ONLY '{m}' no longer a client method"),
            });
        }
    }
    for &t in ledgers::PATHS_CLIENT_ONLY {
        if !template_set.contains(t) {
            v.push(Violation {
                kind: "stale-ledger",
                detail: format!("PATHS_CLIENT_ONLY '{t}' no longer a paths.rs template"),
            });
        }
    }
    v
}
