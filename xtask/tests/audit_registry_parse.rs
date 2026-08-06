use xtask::audit_coverage_check::registry;

#[test]
fn registry_load_finds_known_actions() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../crates/shared/audit-log/src/action_type.rs");
    let reg = registry::load(&path).expect("parse");
    let login = reg.actions.get("auth.login").expect("auth.login");
    assert_eq!(login.kind, registry::Kind::Event);
    let pcu = reg
        .actions
        .get("plugin_config.update")
        .expect("plugin_config.update");
    assert_eq!(pcu.kind, registry::Kind::Stateful);
    // Total should be 145 actions (142 + 3 role variants:
    // role.{create,update,delete})
    assert_eq!(reg.actions.len(), 145);
}
