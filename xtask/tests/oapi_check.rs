use xtask::openapi_client_check::{check, spec::SpecOp};

fn op(id: &str) -> SpecOp {
    SpecOp { operation_id: id.into(), path: "/x".into(), method: "get".into() }
}

#[test]
fn flags_spec_op_with_no_method_and_orphan_method() {
    let ops = vec![op("list_hosts"), op("get_host")];
    let methods = vec![
        "list_hosts".to_string(),
        "list_all_hosts".to_string(),
        "orphan_method".to_string(),
    ];
    let v = check::check_names(&ops, &methods);
    let d: Vec<_> = v.iter().map(|x| x.detail.clone()).collect();
    assert!(d.iter().any(|x| x.contains("get_host")), "missing method not flagged");
    assert!(d.iter().any(|x| x.contains("orphan_method")), "orphan method not flagged");
    assert!(!d.iter().any(|x| x.contains("list_all_hosts")), "companion wrongly flagged");
}

#[test]
fn flags_dead_const_and_unrouted_path() {
    let spec_paths = vec!["/api/v1/hosts".to_string(), "/api/v1/services/{}".to_string()];
    let client = vec![
        "/api/v1/hosts".to_string(),
        "/healthz".to_string(),
        "/api/v1/dead".to_string(),
    ];
    let v = check::check_paths(&spec_paths, &client);
    let d: Vec<_> = v.iter().map(|x| x.detail.clone()).collect();
    assert!(d.iter().any(|x| x.contains("/api/v1/dead")), "dead const not flagged");
    assert!(d.iter().any(|x| x.contains("/api/v1/services/{}")), "unrouted path not flagged");
    assert!(!d.iter().any(|x| x.contains("/healthz")), "PATHS_CLIENT_ONLY wrongly flagged");
}

#[test]
fn flags_stale_ledger_entries() {
    // Fixture lacks the seed ledgers' entries (`oidc_callback`, `raw_request`), so they read stale.
    let ops = vec![op("list_hosts")];
    let methods = vec!["list_hosts".to_string()];
    let templates: Vec<String> = vec![];
    let v = check::check_stale_ledgers(&ops, &methods, &templates);
    assert!(v.iter().all(|x| x.kind == "stale-ledger"));
    assert!(
        v.iter().any(|x| x.detail.contains("oidc_callback")),
        "stale SPEC_ONLY not flagged"
    );
    assert!(
        v.iter().any(|x| x.detail.contains("raw_request")),
        "stale CLIENT_ONLY not flagged"
    );
}
