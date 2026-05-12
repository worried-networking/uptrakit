use uptrakit_config_reload::config::Scope;

#[test]
fn scope_equality_global() {
    assert_eq!(Scope::Global, Scope::Global);
}

#[test]
fn scope_equality_tenant() {
    let id = uuid::Uuid::nil();
    assert_eq!(Scope::Tenant(id), Scope::Tenant(id));
    assert_ne!(Scope::Tenant(id), Scope::Global);
}

// Task 3 tests added below.
use uptrakit_config_reload::ConfigReloadError;

#[test]
fn error_into_report_is_ok() {
    // Verify the Into<Report> conversion compiles and succeeds.
    // (Testing thiserror's Display format string is prohibited.)
    use rootcause::Report;
    let err = ConfigReloadError::TomlParse {
        path: "/etc/uptrakit/controller.toml".into(),
        source_msg: "expected `=` at line 3".into(),
    };
    let _report: Report = err.into();
    // If we reach here the conversion succeeded.
}
