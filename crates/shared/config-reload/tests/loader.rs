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
fn network_parses_https_and_pki() {
    let raw = r#"
[https]
addr = "0.0.0.0:8443"
trusted_proxies = ["127.0.0.1/32"]
real_ip_header = "x-forwarded-for"
forwarded_client_cert_info_header = "x-fcc"
forwarded_client_cert_pem_header  = "x-fcc-pem"

[pki]
addr = "0.0.0.0:8444"
"#;
    let parsed: NetworkConfig = toml::from_str(raw).unwrap();
    assert_eq!(parsed.https.addr, "0.0.0.0:8443");
    assert_eq!(parsed.pki.addr, "0.0.0.0:8444");
    parsed.validate().unwrap();
}

#[test]
fn network_rejects_collision() {
    let raw = r#"
[https]
addr = "0.0.0.0:8443"
trusted_proxies = []
real_ip_header = "x-forwarded-for"
forwarded_client_cert_info_header = "x-fcc"
forwarded_client_cert_pem_header  = "x-fcc-pem"

[pki]
addr = "0.0.0.0:8443"
"#;
    let parsed: NetworkConfig = toml::from_str(raw).unwrap();
    assert!(
        parsed.validate().is_err(),
        "https and pki on same addr must fail"
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
    let bad: TlsConfig = toml::from_str(
        r#"
cert_path = "/etc/tls/cert.pem"
key_path = ""
sans = []
"#,
    )
    .unwrap();
    assert!(bad.validate().is_err());
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
[db]
url = "sqlite://var/lib/uptrakit/controller.db"
pool_size = 16
acquire_timeout_ms = 5000

[master_key]
path = "/etc/uptrakit/master.key"

[network.https]
addr = "0.0.0.0:8443"
trusted_proxies = []
real_ip_header = "x-forwarded-for"
forwarded_client_cert_info_header = "x-fcc"
forwarded_client_cert_pem_header  = "x-fcc-pem"

[network.pki]
addr = "0.0.0.0:8444"

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
