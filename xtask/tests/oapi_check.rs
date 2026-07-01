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
