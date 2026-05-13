use audit_coverage_check::catalog;
use std::io::Write;

#[test]
fn catalog_round_trip() {
    let mut tmp = tempfile::NamedTempFile::new().unwrap();
    writeln!(
        tmp,
        r#"
[[entries]]
site = "x::y"
action = "auth.login"

[[entries]]
site = "x::z"
skip = "covered by access log"
"#
    )
    .unwrap();
    let cat = catalog::load(tmp.path()).expect("parse");
    assert_eq!(cat.entries.len(), 2);
    assert!(
        cat.entries
            .iter()
            .any(|e| e.site == "x::y" && e.action.as_deref() == Some("auth.login"))
    );
    assert!(
        cat.entries
            .iter()
            .any(|e| e.site == "x::z" && e.skip.is_some())
    );
}

#[test]
fn catalog_rejects_both_action_and_skip() {
    let mut tmp = tempfile::NamedTempFile::new().unwrap();
    writeln!(
        tmp,
        r#"
[[entries]]
site = "x::y"
action = "auth.login"
skip = "should not be both"
"#
    )
    .unwrap();
    let result = catalog::load(tmp.path());
    result.unwrap_err();
}

#[test]
fn catalog_rejects_neither_action_nor_skip() {
    let mut tmp = tempfile::NamedTempFile::new().unwrap();
    writeln!(
        tmp,
        r#"
[[entries]]
site = "x::y"
"#
    )
    .unwrap();
    let result = catalog::load(tmp.path());
    result.unwrap_err();
}
