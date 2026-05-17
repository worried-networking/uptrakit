use uptrakit_config_reload::RuntimeConfig;
use uptrakit_config_reload::config::{
    AuditConfig, DbConfig, EmbeddedServicesConfig, NatsConfig, NetworkConfig, TlsConfig,
    ZeroconfConfig,
};

// ── DbConfig tests ──────────────────────────────────────────────────────────

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
    good.validate().unwrap();
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

// ── NetworkConfig tests ─────────────────────────────────────────────────────

#[test]
fn network_parses_flat_fields() {
    let raw = r#"
addr = "0.0.0.0:8443"
pki_addr = "0.0.0.0:8444"
trusted_proxies = ["127.0.0.1/32"]
real_ip_header = "x-forwarded-for"
forwarded_client_cert_info_header = "x-fcc"
forwarded_client_cert_pem_header  = "x-fcc-pem"
"#;
    let parsed: NetworkConfig = toml::from_str(raw).unwrap();
    assert_eq!(parsed.https.addr, "0.0.0.0:8443");
    assert_eq!(parsed.pki_addr, "0.0.0.0:8444");
    parsed.validate().unwrap();
}

#[test]
fn network_rejects_https_scheme_pki_addr() {
    let raw = r#"
addr    = "0.0.0.0:8443"
pki_addr = "https://controller.example.com:8444"
"#;
    let parsed: NetworkConfig = toml::from_str(raw).unwrap();
    let err = parsed.validate().unwrap_err();
    assert!(
        err.to_string().contains("https://"),
        "error must mention https://"
    );
}

#[test]
fn network_rejects_collision() {
    let raw = r#"
addr    = "0.0.0.0:8443"
pki_addr = "0.0.0.0:8443"
"#;
    let parsed: NetworkConfig = toml::from_str(raw).unwrap();
    assert!(
        parsed.validate().is_err(),
        "same addr for https and pki must fail"
    );
}

// ── AuditConfig tests ───────────────────────────────────────────────────────

#[test]
fn audit_rejects_unknown_filter() {
    let bad: AuditConfig = toml::from_str("filter = \"weird\"\nretention_days = 90\n").unwrap();
    assert!(bad.validate().is_err());
}

#[test]
fn audit_accepts_known_filters() {
    for filter in ["all", "mutations", "none"] {
        let cfg: AuditConfig =
            toml::from_str(&format!("filter = \"{filter}\"\nretention_days = 90\n")).unwrap();
        cfg.validate().unwrap();
    }
}

// ── NatsConfig tests ────────────────────────────────────────────────────────

#[test]
fn nats_validates_url() {
    let good = NatsConfig::new("nats://localhost:4222");
    good.validate().unwrap();
    let bad = NatsConfig::new("");
    assert!(bad.validate().is_err());
}

// ── TlsConfig tests ─────────────────────────────────────────────────────────

#[test]
fn tls_requires_both_paths() {
    let cert_only: TlsConfig = toml::from_str(
        r#"
cert_path = "/etc/tls/cert.pem"
key_path = ""
sans = []
"#,
    )
    .unwrap();
    assert!(cert_only.validate().is_err());

    let key_only: TlsConfig = toml::from_str(
        r#"
cert_path = ""
key_path = "/etc/tls/key.pem"
sans = []
"#,
    )
    .unwrap();
    assert!(key_only.validate().is_err());
}

#[test]
fn tls_both_empty_is_managed_ca_mode() {
    TlsConfig::default().validate().unwrap();
}

#[test]
fn runtime_config_without_tls_paths_validates() {
    let toml_str = minimal_toml()
        .replace(
            "cert_path = \"/etc/uptrakit/tls/cert.pem\"",
            "cert_path = \"\"",
        )
        .replace(
            "key_path  = \"/etc/uptrakit/tls/key.pem\"",
            "key_path  = \"\"",
        );
    let cfg: RuntimeConfig = toml::from_str(&toml_str).expect("parse");
    cfg.validate()
        .expect("managed CA mode (both paths empty) must validate");
}

// ── EmbeddedServicesConfig tests ────────────────────────────────────────────

#[test]
fn embedded_services_accepts_all_false() {
    let cfg: EmbeddedServicesConfig = toml::from_str(
        r#"
agent = false
agent_ssh = false
mqtt = false
scheduler = false
"#,
    )
    .unwrap();
    cfg.validate().unwrap();
}

// ── ZeroconfConfig tests ────────────────────────────────────────────────────

#[test]
fn zeroconf_disabled_does_not_require_url() {
    let cfg: ZeroconfConfig = toml::from_str("enabled = false\n").unwrap();
    cfg.validate().unwrap();
}

#[test]
fn zeroconf_enabled_requires_url_and_pki_addr() {
    let raw = "enabled = true\nurl = \"\"\npki_addr = \"\"\n";
    let cfg: ZeroconfConfig = toml::from_str(raw).unwrap();
    assert!(cfg.validate().is_err());
}

// ── RuntimeConfig tests ─────────────────────────────────────────────────────

fn minimal_toml() -> String {
    r#"
master_key = "file:/etc/uptrakit/master.key"

[db]
url = "sqlite://var/lib/uptrakit/controller.db"
pool_size = 16
acquire_timeout_ms = 5000

[network]
addr = "0.0.0.0:8443"
pki_addr = "0.0.0.0:8444"
trusted_proxies = []
real_ip_header = "x-forwarded-for"
forwarded_client_cert_info_header = "x-fcc"
forwarded_client_cert_pem_header  = "x-fcc-pem"

[tls]
cert_path = "/etc/uptrakit/tls/cert.pem"
key_path  = "/etc/uptrakit/tls/key.pem"
sans      = []

[nats]
url = "nats://localhost:4222"

[audit]
filter = "all"
retention_days = 90

[log]
path  = "/var/log/uptrakit/controller.log"
level = "info"

[zeroconf]
enabled = true
url      = "https://controller.local:8443"
pki_addr = "controller.local:8444"

[embedded_services]
agent = false
agent_ssh = false
mqtt = false
scheduler = true
"#
    .to_string()
}

fn minimal_toml_with_typo() -> String {
    minimal_toml().replace("pool_size = 16", "pool_size = 16\npoool_size = 32")
}

#[test]
fn runtime_config_full_round_trip() {
    let cfg: RuntimeConfig = toml::from_str(&minimal_toml()).unwrap();
    cfg.validate().expect("full TOML must validate");
    assert!(cfg.warn_about_extras().is_empty());
}

#[test]
fn runtime_config_captures_unknown_keys() {
    let cfg: RuntimeConfig = toml::from_str(&minimal_toml_with_typo()).unwrap();
    let warnings = cfg.warn_about_extras();
    assert!(warnings.iter().any(|w| w.contains("poool_size")));
}

// ── TomlConfigLoader tests ──────────────────────────────────────────────────

use std::io::Write;
use tempfile::NamedTempFile;
use uptrakit_config_reload::TomlConfigLoader;

#[test]
fn loader_validate_only_passes_for_minimal_valid_toml() {
    let mut f = NamedTempFile::new().unwrap();
    writeln!(f, "{}", minimal_toml()).unwrap();
    TomlConfigLoader::validate_only(f.path()).unwrap();
}

#[test]
fn loader_validate_only_fails_for_bad_toml() {
    let mut f = NamedTempFile::new().unwrap();
    writeln!(f, "not valid toml = =").unwrap();
    assert!(TomlConfigLoader::validate_only(f.path()).is_err());
}

#[test]
fn loader_load_emits_extras_warnings() {
    let mut f = NamedTempFile::new().unwrap();
    write!(f, "{}", minimal_toml_with_typo()).unwrap();
    let loaded = TomlConfigLoader::load(f.path()).unwrap();
    assert!(!loaded.warnings.is_empty());
    assert!(loaded.warnings.iter().any(|w| w.contains("poool_size")));
}

#[test]
fn loader_sample_file_parses_and_validates() {
    let sample_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../docs/examples/controller.toml"
    );
    TomlConfigLoader::load(sample_path).unwrap_or_else(|e| {
        panic!("docs/examples/controller.toml must parse and validate: {e}");
    });
}

#[test]
fn runtime_config_network_unknown_key_captured_in_extras() {
    let raw = minimal_toml().replace(
        "addr = \"0.0.0.0:8443\"",
        "addr = \"0.0.0.0:8443\"\nnetwork_typo_key = \"value\"",
    );
    let cfg: RuntimeConfig = toml::from_str(&raw).unwrap();
    assert_eq!(
        cfg.network.https.addr, "0.0.0.0:8443",
        "double-flatten: https.addr must be accessible via NetworkConfig"
    );
    assert!(
        cfg.network.extra.contains_key("network_typo_key"),
        "unknown [network] key must land in NetworkConfig.extra"
    );
}

#[cfg(unix)]
#[test]
fn loader_inline_master_key_rejects_permissive_config() {
    use std::os::unix::fs::PermissionsExt;
    let toml_content = minimal_toml().replace(
        "master_key = \"file:/etc/uptrakit/master.key\"",
        "master_key = \"0000000000000000000000000000000000000000000000000000000000000001\"",
    );
    let mut f = NamedTempFile::new().unwrap();
    write!(f, "{toml_content}").unwrap();
    std::fs::set_permissions(f.path(), std::fs::Permissions::from_mode(0o644)).unwrap();
    let result = TomlConfigLoader::load(f.path());
    assert!(
        result.is_err(),
        "permissive config with inline key must fail"
    );
    let err = result.err().expect("expected error").to_string();
    assert!(
        err.contains("chmod 0600"),
        "error must mention chmod 0600: {err}"
    );
}

#[cfg(unix)]
#[test]
fn loader_inline_master_key_accepts_strict_config() {
    use std::os::unix::fs::PermissionsExt;
    let toml_content = minimal_toml().replace(
        "master_key = \"file:/etc/uptrakit/master.key\"",
        "master_key = \"0000000000000000000000000000000000000000000000000000000000000001\"",
    );
    let mut f = NamedTempFile::new().unwrap();
    write!(f, "{toml_content}").unwrap();
    std::fs::set_permissions(f.path(), std::fs::Permissions::from_mode(0o600)).unwrap();
    TomlConfigLoader::load(f.path())
        .expect("strict permissions with inline key must load successfully");
}
