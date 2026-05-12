use uptrakit_config_reload::config::db::DbConfig;

#[test]
fn db_config_rejects_zero_pool_size() {
    let bad = DbConfig::with_all("sqlite://x", 0, 5000);
    assert!(bad.validate().is_err());
}

#[test]
fn db_config_rejects_zero_timeout() {
    let bad = DbConfig::with_all("sqlite://x", 16, 0);
    assert!(bad.validate().is_err());
}

#[test]
fn db_config_accepts_valid_values() {
    let good = DbConfig::with_all("sqlite://x", 16, 5000);
    assert!(good.validate().is_ok());
}

#[test]
fn db_config_captures_unknown_fields() {
    let raw = r#"
url = "sqlite://x"
pool_size = 16
acquire_timeout_ms = 5000
unknown_key = "value"
"#;
    let cfg: DbConfig = toml::from_str(raw).expect("parse should succeed; unknowns land in extra");
    assert_eq!(cfg.extra.len(), 1);
    assert!(cfg.extra.contains_key("unknown_key"));
}
