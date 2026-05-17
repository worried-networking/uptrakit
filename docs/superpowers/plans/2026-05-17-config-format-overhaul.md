# Config Format Overhaul Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps
> use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `[master_key]` section and `[network.https]`/`[network.pki]` sub-tables
with flat TOML fields, add inline-key permission enforcement, and ship an annotated sample
with CI-guarding tests.

**Architecture:** Changes ripple outward from `crates/shared/config-reload` (config struct
definitions + loader) into `crates/core/controller-runtime` (startup, triage, reload) and
`crates/ui/web-api` (AppState channel type). The `SecretString` type from
`uptrakit-shared-types` replaces `MasterKeyConfig`; `PkiConfig` is deleted; `HttpsConfig`
gains `#[serde(flatten)]` so its fields appear at the `[network]` level in TOML.

**Tech Stack:** Rust / serde / toml / tokio::sync::watch / uptrakit-shared-types::SecretString

---

## Task 1 — Add `Default` to `SecretString` + add dependency to `config-reload`

**Files:**

- Modify: `crates/shared/types/src/secret_string.rs`
- Modify: `crates/shared/config-reload/Cargo.toml`

- [ ] **Step 1: Add `Default` impl to `SecretString`**

  In `crates/shared/types/src/secret_string.rs`, add after the existing `impl` block:

  ```rust
  impl Default for SecretString {
      fn default() -> Self {
          Self(String::new())
      }
  }
  ```

- [ ] **Step 2: Add `uptrakit-shared-types` to config-reload dependencies**

  In `crates/shared/config-reload/Cargo.toml`, under `[dependencies]`, add:

  ```toml
  uptrakit-shared-types = { workspace = true }
  ```

- [ ] **Step 3: Verify compilation**

  ```bash
  cargo check -p uptrakit-config-reload
  cargo check -p uptrakit-shared-types
  ```

  Expected: both compile with no errors.

- [ ] **Step 4: Commit**

  ```bash
  git commit --only crates/shared/types/src/secret_string.rs crates/shared/config-reload/Cargo.toml \
      -m "feat(config): add Default to SecretString; add shared-types dep to config-reload"
  ```

---

## Task 2 — Replace `MasterKeyConfig` with `SecretString`

**Files:**

- Delete: `crates/shared/config-reload/src/config/master_key.rs`
- Modify: `crates/shared/config-reload/src/config/mod.rs`
- Modify: `crates/shared/config-reload/src/channels.rs`

- [ ] **Step 1: Delete the master_key module**

  ```bash
  rm crates/shared/config-reload/src/config/master_key.rs
  ```

- [ ] **Step 2: Rewrite `config/mod.rs`**

  Replace the entire file content with:

  ```rust
  pub mod audit;
  pub mod db;
  pub mod embedded;
  pub mod log;
  pub mod nats;
  pub mod network;
  pub mod plugins;
  pub mod scope;
  pub mod tls;
  pub mod zeroconf;

  pub use audit::AuditConfig;
  pub use db::DbConfig;
  pub use embedded::EmbeddedServicesConfig;
  pub use log::LogConfig;
  pub use nats::NatsConfig;
  pub use network::{HttpsConfig, NetworkConfig};
  pub use plugins::PluginsConfig;
  pub use scope::Scope;
  pub use tls::TlsConfig;
  pub use zeroconf::ZeroconfConfig;

  use rootcause::prelude::*;
  use serde::{Deserialize, Serialize};
  use uptrakit_shared_types::SecretString;

  use crate::error::ConfigReloadError;

  /// Top-level runtime configuration for the uptrakit Controller.
  ///
  /// Parsed from a TOML file via [`crate::loader::TomlConfigLoader`].
  #[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
  #[non_exhaustive]
  pub struct RuntimeConfig {
      /// Database connection and pool settings.
      #[serde(default)]
      pub db: DbConfig,
      /// Master key source URI: `file:/path`, `env:VAR`, or 64-char inline hex.
      #[serde(default)]
      pub master_key: SecretString,
      /// Network listener settings (HTTPS + PKI).
      pub network: NetworkConfig,
      /// TLS certificate and key settings.
      #[serde(default)]
      pub tls: TlsConfig,
      /// NATS messaging server settings.
      pub nats: NatsConfig,
      /// Audit log settings.
      #[serde(default)]
      pub audit: AuditConfig,
      /// Logging settings.
      #[serde(default)]
      pub log: LogConfig,
      /// Zero-configuration auto-discovery settings.
      #[serde(default)]
      pub zeroconf: ZeroconfConfig,
      /// Which services run embedded inside the controller binary.
      #[serde(default)]
      pub embedded_services: EmbeddedServicesConfig,
  }

  impl RuntimeConfig {
      /// Validate all config sections.
      pub fn validate(&self) -> Result<(), Report> {
          self.db.validate()?;
          validate_master_key(self.master_key.expose_secret())?;
          self.network.validate()?;
          self.tls.validate()?;
          self.nats.validate()?;
          self.audit.validate()?;
          self.log.validate()?;
          self.zeroconf.validate()?;
          self.embedded_services.validate()?;
          Ok(())
      }

      /// Collect warnings about unknown keys found in each config section.
      ///
      /// NOTE: loops for `network.https.extra` and `network.pki.extra` from the old
      /// implementation are intentionally omitted — `HttpsConfig.extra` is deleted in
      /// Task 3 (the `#[serde(flatten)]` makes `NetworkConfig.extra` the single catch-all),
      /// and `PkiConfig` is deleted entirely. `[network] unknown key` messages cover both.
      #[must_use]
      pub fn warn_about_extras(&self) -> Vec<String> {
          let mut out = Vec::new();
          for key in self.db.extra.keys() {
              out.push(format!("[db] unknown key `{key}` ignored"));
          }
          for key in self.network.extra.keys() {
              out.push(format!("[network] unknown key `{key}` ignored"));
          }
          for key in self.tls.extra.keys() {
              out.push(format!("[tls] unknown key `{key}` ignored"));
          }
          for key in self.nats.extra.keys() {
              out.push(format!("[nats] unknown key `{key}` ignored"));
          }
          for key in self.audit.extra.keys() {
              out.push(format!("[audit] unknown key `{key}` ignored"));
          }
          for key in self.log.extra.keys() {
              out.push(format!("[log] unknown key `{key}` ignored"));
          }
          for key in self.zeroconf.extra.keys() {
              out.push(format!("[zeroconf] unknown key `{key}` ignored"));
          }
          for key in self.embedded_services.extra.keys() {
              out.push(format!("[embedded_services] unknown key `{key}` ignored"));
          }
          out
      }
  }

  /// Validate the master_key config field at parse time.
  ///
  /// Structural checks only — does not open files or read env vars.
  fn validate_master_key(key: &str) -> Result<(), Report> {
      if key.is_empty() {
          bail!(ConfigReloadError::Validate("master_key is empty".into()));
      }
      if let Some(path) = key.strip_prefix("file:") {
          if path.is_empty() {
              bail!(ConfigReloadError::Validate(
                  "master_key file: path is empty (e.g. file:/etc/uptrakit/master.key)".into()
              ));
          }
      } else if let Some(var) = key.strip_prefix("env:") {
          if var.is_empty() {
              bail!(ConfigReloadError::Validate(
                  "master_key env: variable name is empty (e.g. env:UPTRAKIT_MASTER_KEY)".into()
              ));
          }
      } else if key.len() != 64 || !key.chars().all(|c| c.is_ascii_hexdigit()) {
          bail!(ConfigReloadError::Validate(format!(
              "master_key inline value must be exactly 64 hex characters (got {}); \
               use file: or env: prefix for non-inline forms",
              key.len()
          )));
      }
      Ok(())
  }
  ```

- [ ] **Step 3: Update `channels.rs`**

  Replace the file content with:

  ```rust
  //! Boot-seeded `tokio::sync::watch` fan-out channels for runtime config sections.

  use std::sync::Arc;

  use tokio::sync::watch;
  use uptrakit_shared_types::SecretString;

  use crate::config::{
      AuditConfig, DbConfig, EmbeddedServicesConfig, LogConfig, NatsConfig, NetworkConfig,
      RuntimeConfig, TlsConfig, ZeroconfConfig,
  };

  /// Senders for all runtime config sections.
  pub struct RuntimeConfigChannels {
      pub db: watch::Sender<Arc<DbConfig>>,
      pub network: watch::Sender<Arc<NetworkConfig>>,
      pub nats: watch::Sender<Arc<NatsConfig>>,
      pub tls: watch::Sender<Arc<TlsConfig>>,
      pub audit: watch::Sender<Arc<AuditConfig>>,
      /// Boot-time only. Log path changes require reexec; no live delta variant.
      pub log: watch::Sender<Arc<LogConfig>>,
      /// Boot-time only. Master key changes require reexec; no live delta variant.
      pub master_key: watch::Sender<Arc<SecretString>>,
      pub embedded_services: watch::Sender<Arc<EmbeddedServicesConfig>>,
      pub zeroconf: watch::Sender<Arc<ZeroconfConfig>>,
  }

  /// Receivers for all runtime config sections.
  pub struct RuntimeConfigReceivers {
      pub db: watch::Receiver<Arc<DbConfig>>,
      pub network: watch::Receiver<Arc<NetworkConfig>>,
      pub nats: watch::Receiver<Arc<NatsConfig>>,
      pub tls: watch::Receiver<Arc<TlsConfig>>,
      pub audit: watch::Receiver<Arc<AuditConfig>>,
      /// Boot-time only. Log path changes require reexec; no live delta variant.
      pub log: watch::Receiver<Arc<LogConfig>>,
      /// Boot-time only. Master key changes require reexec; no live delta variant.
      pub master_key: watch::Receiver<Arc<SecretString>>,
      pub embedded_services: watch::Receiver<Arc<EmbeddedServicesConfig>>,
      pub zeroconf: watch::Receiver<Arc<ZeroconfConfig>>,
  }

  impl RuntimeConfigChannels {
      #[must_use]
      pub fn from_runtime(runtime: &RuntimeConfig) -> (Self, RuntimeConfigReceivers) {
          let (db_tx, db_rx) = watch::channel(Arc::new(runtime.db.clone()));
          let (net_tx, net_rx) = watch::channel(Arc::new(runtime.network.clone()));
          let (nats_tx, nats_rx) = watch::channel(Arc::new(runtime.nats.clone()));
          let (tls_tx, tls_rx) = watch::channel(Arc::new(runtime.tls.clone()));
          let (audit_tx, audit_rx) = watch::channel(Arc::new(runtime.audit.clone()));
          let (log_tx, log_rx) = watch::channel(Arc::new(runtime.log.clone()));
          let (mk_tx, mk_rx) = watch::channel(Arc::new(runtime.master_key.clone()));
          let (emb_tx, emb_rx) = watch::channel(Arc::new(runtime.embedded_services.clone()));
          let (zc_tx, zc_rx) = watch::channel(Arc::new(runtime.zeroconf.clone()));

          let senders = Self {
              db: db_tx,
              network: net_tx,
              nats: nats_tx,
              tls: tls_tx,
              audit: audit_tx,
              log: log_tx,
              master_key: mk_tx,
              embedded_services: emb_tx,
              zeroconf: zc_tx,
          };
          let receivers = RuntimeConfigReceivers {
              db: db_rx,
              network: net_rx,
              nats: nats_rx,
              tls: tls_rx,
              audit: audit_rx,
              log: log_rx,
              master_key: mk_rx,
              embedded_services: emb_rx,
              zeroconf: zc_rx,
          };
          (senders, receivers)
      }
  }
  ```

- [ ] **Step 4: Verify compilation**

  ```bash
  cargo check -p uptrakit-config-reload
  ```

  Expected: compiles. (controller-runtime and web-api will break — fixed in later tasks.)

- [ ] **Step 5: Commit**

  ```bash
  git commit --only crates/shared/config-reload/src/config/master_key.rs \
      crates/shared/config-reload/src/config/mod.rs \
      crates/shared/config-reload/src/channels.rs \
      -m "refactor(config): replace MasterKeyConfig with SecretString"
  ```

---

## Task 3 — Flatten `NetworkConfig`: remove `PkiConfig`, serde-flatten `HttpsConfig`

**Files:**

- Modify: `crates/shared/config-reload/src/config/network.rs`

- [ ] **Step 1: Rewrite `config/network.rs`**

  Replace the entire file content with:

  ```rust
  use std::collections::HashMap;
  use std::net::SocketAddr;

  use rootcause::prelude::*;
  use serde::{Deserialize, Serialize};

  use crate::error::ConfigReloadError;

  /// Top-level network configuration.
  ///
  /// In TOML all fields appear directly under `[network]`; there are no
  /// `[network.https]` or `[network.pki]` sub-tables.
  #[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
  #[non_exhaustive]
  pub struct NetworkConfig {
      /// HTTPS listener settings (flattened to `[network]` level in TOML).
      #[serde(flatten)]
      pub https: HttpsConfig,
      /// PKI endpoint: bare bind address or `http://` advertisement URL.
      ///
      /// - Bare address (e.g. `0.0.0.0:8080`): the embedded PKI HTTP
      ///   listener binds to this address.
      /// - `http://` URL (e.g. `http://hostname:8080`): the controller
      ///   starts a plain HTTP listener on the extracted port; the full URL
      ///   is used for CA cert SANs and zeroconf advertisement.
      ///
      /// `https://` is not a valid scheme for this field.
      #[serde(default)]
      pub pki_addr: String,
      /// Unknown keys captured for `warn_about_extras`.
      #[serde(flatten)]
      pub extra: HashMap<String, toml::Value>,
  }

  /// HTTPS listener settings. Fields appear at the `[network]` TOML level
  /// via `#[serde(flatten)]` on `NetworkConfig.https`.
  #[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
  #[non_exhaustive]
  pub struct HttpsConfig {
      /// TCP address and port to listen on (e.g. `0.0.0.0:8443`).
      pub addr: String,
      /// CIDR ranges of trusted reverse proxies.
      #[serde(default)]
      pub trusted_proxies: Vec<String>,
      /// Header name carrying the real client IP (set by the proxy).
      #[serde(default = "default_real_ip")]
      pub real_ip_header: String,
      /// Header carrying forwarded client certificate info.
      #[serde(default = "default_fcc_info")]
      pub forwarded_client_cert_info_header: String,
      /// Header carrying the forwarded client certificate PEM.
      #[serde(default = "default_fcc_pem")]
      pub forwarded_client_cert_pem_header: String,
      // No `extra` field here: NetworkConfig.extra is the single flatten
      // catch-all for the entire [network] section (serde invariant: one
      // HashMap flatten per level).
  }

  fn default_real_ip() -> String {
      "x-forwarded-for".into()
  }
  fn default_fcc_info() -> String {
      "x-forwarded-client-cert".into()
  }
  fn default_fcc_pem() -> String {
      "x-forwarded-client-cert-pem".into()
  }

  impl NetworkConfig {
      /// Validate this config section.
      pub fn validate(&self) -> Result<(), Report> {
          self.https.addr.parse::<SocketAddr>().map_err(|e| {
              report!(ConfigReloadError::Validate(format!(
                  "network.addr invalid: {e}"
              )))
          })?;
          // pki_addr is a bare bind address or an http:// advertisement URL.
          // https:// is not supported (the PKI endpoint is plain HTTP only).
          if self.pki_addr.starts_with("https://") {
              bail!(ConfigReloadError::Validate(
                  "network.pki_addr must use http:// scheme, not https://".into()
              ));
          }
          // Not validated as a SocketAddr (http:// form is not a valid SocketAddr).
          // Only check for collision with the HTTPS bind address.
          if !self.pki_addr.is_empty() && self.pki_addr == self.https.addr {
              bail!(ConfigReloadError::Validate(format!(
                  "network.pki_addr ({}) collides with network.addr",
                  self.pki_addr,
              )));
          }
          Ok(())
      }
  }
  ```

- [ ] **Step 2: Verify compilation**

  ```bash
  cargo check -p uptrakit-config-reload
  ```

  Expected: compiles. (`PkiConfig` is gone; controller-runtime breaks — fixed later.)

- [ ] **Step 3: Update existing `NetworkConfig` tests in `tests/loader.rs`**

  The tests `network_parses_https_and_pki` and `network_rejects_collision` use the old
  `[https]`/`[pki]` sub-table format. Replace them with flat-field equivalents:

  Find this block (starts around line 41):

  ```rust
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
  ```

  Replace with:

  ```rust
  #[test]
  fn network_parses_flat_fields() {
      let raw = r#"
  addr = "0.0.0.0:8443"
  pki_addr = "0.0.0.0:8444"
  trusted_proxies = ["127.0.0.1/32"]
  real_ip_header = "x-forwarded-for"
  forwarded_client_cert_info_header = "x-fcc"
  forwarded_client_cert_pem_header = "x-fcc-pem"
  "#;
      let parsed: NetworkConfig = toml::from_str(raw).unwrap();
      assert_eq!(parsed.https.addr, "0.0.0.0:8443");
      assert_eq!(parsed.pki_addr, "0.0.0.0:8444");
      parsed.validate().unwrap();
  }

  #[test]
  fn network_unknown_key_captured_in_extras() {
      let raw = r#"
  addr = "0.0.0.0:8443"
  pki_addr = "0.0.0.0:8444"
  trusted_proxies = []
  real_ip_header = "x-forwarded-for"
  forwarded_client_cert_info_header = "x-fcc"
  forwarded_client_cert_pem_header = "x-fcc-pem"
  addrr = "typo"
  "#;
      let parsed: NetworkConfig = toml::from_str(raw).unwrap();
      assert!(
          parsed.extra.contains_key("addrr"),
          "typo key must land in extra, not silently dropped"
      );
  }

  #[test]
  fn network_rejects_collision() {
      let raw = r#"
  addr = "0.0.0.0:8443"
  pki_addr = "0.0.0.0:8443"
  trusted_proxies = []
  real_ip_header = "x-forwarded-for"
  forwarded_client_cert_info_header = "x-fcc"
  forwarded_client_cert_pem_header = "x-fcc-pem"
  "#;
      let parsed: NetworkConfig = toml::from_str(raw).unwrap();
      assert!(
          parsed.validate().is_err(),
          "addr and pki_addr collision must fail"
      );
  }
  ```

- [ ] **Step 4: Run config-reload tests**

  ```bash
  cargo test -p uptrakit-config-reload -- loader
  ```

  Expected: `network_parses_flat_fields`, `network_unknown_key_captured_in_extras`,
  `network_rejects_collision` all pass.

  If `network_unknown_key_captured_in_extras` **fails** (typo key is NOT in `extra`), the
  serde flatten interaction is broken. In that case, instead of `#[serde(flatten)] https`,
  expand `HttpsConfig` fields directly onto `NetworkConfig` and build `HttpsConfig` via
  `From<&NetworkConfig>` in `HttpsListenerReloadable`. File an issue and follow the fallback
  path documented in the spec before continuing.

- [ ] **Step 5: Commit**

  ```bash
  git commit --only crates/shared/config-reload/src/config/network.rs \
      crates/shared/config-reload/tests/loader.rs \
      -m "refactor(config): flatten NetworkConfig — remove PkiConfig, flatten HttpsConfig"
  ```

---

## Task 4 — Add permission check to `loader.rs` + update all test TOML

**Files:**

- Modify: `crates/shared/config-reload/src/loader.rs`
- Modify: `crates/shared/config-reload/tests/loader.rs`
- Modify: `crates/shared/config-reload/tests/coordinator.rs`
- Modify: `crates/core/integration-tests/tests/helpers/containers.rs`

- [ ] **Step 1: Rewrite `loader.rs`**

  Replace the file content with:

  ```rust
  use std::path::Path;

  use rootcause::prelude::*;

  use crate::config::RuntimeConfig;
  use crate::error::ConfigReloadError;

  /// The result of loading and parsing a TOML config file.
  #[non_exhaustive]
  pub struct LoadedConfig {
      /// The parsed and validated runtime configuration.
      pub config: RuntimeConfig,
      /// Warnings about unknown keys that were ignored during parse.
      pub warnings: Vec<String>,
  }

  /// Loads and validates a TOML config file from disk.
  pub struct TomlConfigLoader;

  impl TomlConfigLoader {
      /// Read, parse, validate, and return the config at `path`.
      pub fn load(path: impl AsRef<Path>) -> Result<LoadedConfig, Report> {
          let path = path.as_ref();
          let bytes = std::fs::read_to_string(path).map_err(|e| {
              report!(ConfigReloadError::TomlIo {
                  path: path.to_path_buf(),
                  source_msg: e.to_string(),
              })
          })?;
          // Migration hint: detect old section-based format before parse fails cryptically.
          check_old_format_hint(&bytes, path)?;
          let config: RuntimeConfig = toml::from_str(&bytes).map_err(|e| {
              report!(ConfigReloadError::TomlParse {
                  path: path.to_path_buf(),
                  source_msg: e.to_string(),
              })
          })?;
          config.validate()?;
          check_config_permissions(path, &config)?;
          let warnings = config.warn_about_extras();
          Ok(LoadedConfig { config, warnings })
      }

      /// Read, parse, and validate the config at `path` without returning it.
      pub fn validate_only(path: impl AsRef<Path>) -> Result<(), Report> {
          let loaded = Self::load(path)?;
          for w in &loaded.warnings {
              eprintln!("warning: {w}");
          }
          Ok(())
      }
  }

  /// Check that the config file has restrictive permissions when it contains
  /// inline key material.
  ///
  /// Only fires when `master_key` does NOT start with `file:` or `env:` and
  /// is non-empty (i.e. the config embeds the raw hex key directly).
  /// Emit a helpful error if the config uses the old section-based format.
  ///
  /// Old format: `[master_key]\npath = "..."` or `[network.https]` / `[network.pki]` tables.
  /// New format: top-level `master_key = "file:..."` and flat `[network]` keys.
  fn check_old_format_hint(raw: &str, path: &Path) -> Result<(), Report> {
      let has_old_master_key = raw.lines().any(|l| l.trim() == "[master_key]");
      let has_old_network = raw
          .lines()
          .any(|l| l.trim() == "[network.https]" || l.trim() == "[network.pki]");
      if has_old_master_key {
          bail!(ConfigReloadError::Validate(format!(
              "config file {:?} uses the old [master_key] section format; \
               replace with a top-level field: master_key = \"file:/path/to/key\"",
              path
          )));
      }
      if has_old_network {
          bail!(ConfigReloadError::Validate(format!(
              "config file {:?} uses old [network.https] / [network.pki] sub-sections; \
               all fields are now flat under [network] — see docs/examples/controller.toml",
              path
          )));
      }
      Ok(())
  }

  #[cfg(unix)]
  fn check_config_permissions(path: &Path, config: &RuntimeConfig) -> Result<(), Report> {
      let key = config.master_key.expose_secret();
      if key.is_empty() || key.starts_with("file:") || key.starts_with("env:") {
          return Ok(());
      }
      use std::os::unix::fs::PermissionsExt;
      let mode = std::fs::metadata(path)
          .map_err(|e| {
              report!(ConfigReloadError::TomlIo {
                  path: path.to_path_buf(),
                  source_msg: e.to_string(),
              })
          })?
          .permissions()
          .mode();
      if mode & 0o077 != 0 {
          bail!(ConfigReloadError::Validate(format!(
              "config file {:?} contains an inline master key and must not be readable by \
               group or other (current mode: {:04o}); run: chmod 0600 {:?}",
              path,
              mode & 0o777,
              path
          )));
      }
      Ok(())
  }

  #[cfg(not(unix))]
  fn check_config_permissions(path: &Path, config: &RuntimeConfig) -> Result<(), Report> {
      let key = config.master_key.expose_secret();
      if !key.is_empty() && !key.starts_with("file:") && !key.starts_with("env:") {
          tracing::warn!(
              config_path = %path.display(),
              "config file contains an inline master key; \
               permission enforcement is not available on this platform — \
               ensure the file is not world-readable"
          );
      }
      Ok(())
  }
  ```

- [ ] **Step 2: Update `minimal_toml()` in `tests/loader.rs`**

  Find the `minimal_toml()` function (starts with `fn minimal_toml() -> String`) and
  replace its entire body with the new flat format:

  ```rust
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
  ```

- [ ] **Step 3: Update `MINIMAL_RUNTIME_CONFIG_TOML` in `tests/coordinator.rs`**

  Find the `MINIMAL_RUNTIME_CONFIG_TOML` constant (around line 355) and replace its value
  with:

  ```rust
  const MINIMAL_RUNTIME_CONFIG_TOML: &str = r#"
  master_key = "file:/etc/uptrakit/master.key"

  [db]
  url = "sqlite://test.db"
  pool_size = 16
  acquire_timeout_ms = 5000

  [network]
  addr = "0.0.0.0:8443"
  pki_addr = "0.0.0.0:8444"
  trusted_proxies = []
  real_ip_header = "x-forwarded-for"
  forwarded_client_cert_info_header = "x-fcc"
  forwarded_client_cert_pem_header = "x-fcc-pem"

  [tls]
  cert_path = "/etc/uptrakit/cert.pem"
  key_path = "/etc/uptrakit/key.pem"
  sans = []

  [nats]
  url = "nats://localhost:4222"

  [audit]
  filter = "all"
  retention_days = 90

  [log]
  path = "/var/log/uptrakit/controller.log"
  level = "info"

  [zeroconf]
  enabled = false
  url = ""
  pki_addr = ""

  [embedded_services]
  agent = false
  agent_ssh = false
  mqtt = false
  scheduler = false
  "#;
  ```

- [ ] **Step 4: Add two new tests to `tests/loader.rs`**

  Append at the end of `tests/loader.rs`:

  ```rust
  // ── TomlConfigLoader tests ──────────────────────────────────────────────────

  #[cfg(unix)]
  #[test]
  fn loader_inline_master_key_rejects_permissive_config() {
      use std::os::unix::fs::PermissionsExt;
      let dir = tempfile::tempdir().unwrap();
      let path = dir.path().join("controller.toml");
      let toml = minimal_toml().replace(
          "master_key = \"file:/etc/uptrakit/master.key\"",
          "master_key = \"0000000000000000000000000000000000000000000000000000000000000000\"",
      );
      std::fs::write(&path, &toml).unwrap();
      std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

      let result = uptrakit_config_reload::loader::TomlConfigLoader::load(&path);
      assert!(result.is_err(), "permissive config with inline key must fail");
      let err = format!("{:?}", result.unwrap_err());
      assert!(
          err.contains("chmod 0600"),
          "error must mention chmod 0600, got: {err}"
      );
  }

  #[cfg(unix)]
  #[test]
  fn loader_inline_master_key_accepts_strict_config() {
      use std::os::unix::fs::PermissionsExt;
      let dir = tempfile::tempdir().unwrap();
      let path = dir.path().join("controller.toml");
      let toml = minimal_toml().replace(
          "master_key = \"file:/etc/uptrakit/master.key\"",
          "master_key = \"0000000000000000000000000000000000000000000000000000000000000000\"",
      );
      std::fs::write(&path, &toml).unwrap();
      std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

      let result = uptrakit_config_reload::loader::TomlConfigLoader::load(&path);
      assert!(result.is_ok(), "strict config with inline key must pass: {result:?}");
  }
  ```

- [ ] **Step 5: Update `containers.rs` integration test TOML**

  In `crates/core/integration-tests/tests/helpers/containers.rs`, find the `write!` block
  (around line 97) that embeds a config file. Replace:

  ```toml
  [master_key]
  path = "/tmp/dummy-overridden-by-cli"

  [network.https]
  addr = "[::]:8443"

  [network.pki]
  addr = "http://[::]:8444"
  ```

  With:

  ```toml
  master_key = "file:/tmp/dummy-overridden-by-cli"

  [network]
  addr = "[::]:8443"
  pki_addr = "http://[::]:8444"
  ```

  (The value `file:/tmp/dummy-overridden-by-cli` is a placeholder that passes `validate_master_key`
  structural checks; the real key comes from `--master-key-from` CLI argument in integration tests.)

- [ ] **Step 6: Run all config-reload tests**

  ```bash
  cargo test -p uptrakit-config-reload
  ```

  Expected: all pass (including the two new Unix permission tests on Unix systems).

- [ ] **Step 7: Commit**

  ```bash
  git commit --only crates/shared/config-reload/src/loader.rs \
      crates/shared/config-reload/tests/loader.rs \
      crates/shared/config-reload/tests/coordinator.rs \
      crates/core/integration-tests/tests/helpers/containers.rs \
      -m "feat(config): add permission check for inline master_key; update test TOML to flat format"
  ```

---

## Task 5 — Create `docs/examples/controller.toml`

**Files:**

- Create: `docs/examples/controller.toml`

- [ ] **Step 1: Create the sample file**

  Create `docs/examples/controller.toml` with content:

  ```toml
  # uptrakit controller configuration
  #
  # Required fields are uncommented. Optional fields are commented out showing
  # their default values. See docs/end-user/configuration.md for prose docs (planned).

  # Master key source (required). Three forms:
  #   file:/path   — read hex key from file (recommended)
  #   env:VAR      — read hex key from environment variable
  #   0abc...      — inline 64-char hex (requires chmod 0600 on THIS file)
  master_key = "file:/etc/uptrakit/master.key"

  # ── Database ──────────────────────────────────────────────────────────────────

  [db]
  # PostgreSQL: "postgres://user:pass@host/dbname"
  # SQLite:     "sqlite:///var/lib/uptrakit/controller.db"
  url = "sqlite:///var/lib/uptrakit/controller.db"

  # Maximum pooled connections. Default: 16
  # pool_size = 16

  # Maximum wait for an idle connection (ms). Default: 5000
  # acquire_timeout_ms = 5000

  # ── Network ───────────────────────────────────────────────────────────────────

  [network]
  # HTTPS bind address (required).
  addr = "0.0.0.0:8443"

  # PKI endpoint: bare bind address or http:// advertisement URL.
  #   "0.0.0.0:8080"                   — bind only, no embedded PKI HTTP listener
  #   "http://controller.example.com:8080" — starts PKI HTTP listener on port 8080
  # Default: "" (PKI HTTP listener disabled)
  # pki_addr = "http://controller.example.com:8080"

  # Trusted reverse proxy CIDR ranges. Default: []
  # trusted_proxies = ["10.0.0.0/8"]

  # Header carrying the real client IP. Default: "x-forwarded-for"
  # real_ip_header = "x-forwarded-for"

  # Forwarded client cert info header (DER fields). Default: "" (disabled)
  # forwarded_client_cert_info_header = "x-forwarded-client-cert"

  # Forwarded client cert PEM header. Default: "" (disabled)
  # forwarded_client_cert_pem_header = "x-forwarded-client-cert-pem"

  # ── TLS ───────────────────────────────────────────────────────────────────────

  [tls]
  # Custom TLS certificate PEM. Leave empty to use the managed CA. Default: ""
  # cert_path = "/etc/uptrakit/tls/cert.pem"

  # Matching TLS private key PEM. Must be set with cert_path. Default: ""
  # key_path = "/etc/uptrakit/tls/key.pem"

  # Additional SANs for the managed certificate. Auto-detected on first start. Default: []
  # sans = ["controller.example.com", "10.0.0.1"]

  # ── NATS ──────────────────────────────────────────────────────────────────────

  [nats]
  # NATS server URL (required when the NATS service is enabled).
  url = "nats://localhost:4222"

  # ── Audit ─────────────────────────────────────────────────────────────────────

  [audit]
  # Audit log filter: "all", "mutations", or "none". Default: "all"
  # filter = "all"

  # Audit log retention period in days. Default: 90
  # retention_days = 90

  # ── Logging ───────────────────────────────────────────────────────────────────

  [log]
  # Log file path. Empty string → stderr. Default: ""
  # path = "/var/log/uptrakit/controller.log"

  # Log level filter. Examples: "info", "debug", "uptrakit=debug,info". Default: "info"
  # level = "info"

  # ── Zeroconf ─────────────────────────────────────────────────────────────────

  [zeroconf]
  # Enable mDNS/DNS-SD advertisement for local discovery. Default: false
  # enabled = false

  # mDNS advertisement URL (required when enabled = true). Default: ""
  # url = ""

  # PKI advertisement address. Falls back to network.pki_addr. Default: ""
  # pki_addr = ""

  # ── Embedded services ─────────────────────────────────────────────────────────

  [embedded_services]
  # Run Agent inside the controller binary. Default: false
  # agent = false

  # Run Agent-SSH inside the controller binary. Default: false
  # agent_ssh = false

  # Run MQTT inside the controller binary. Default: false
  # mqtt = false

  # Run Scheduler inside the controller binary. Default: false
  # scheduler = false
  ```

- [ ] **Step 2: Add sample-file test and RuntimeConfig unknown-key test to `tests/loader.rs`**

  Append at the end of `tests/loader.rs`:

  ```rust
  #[test]
  fn loader_sample_file_parses_and_validates() {
      let sample_path =
          concat!(env!("CARGO_MANIFEST_DIR"), "/../../../docs/examples/controller.toml");
      let result = uptrakit_config_reload::loader::TomlConfigLoader::load(sample_path);
      assert!(result.is_ok(), "sample file must parse and validate: {result:?}");
      let loaded = result.unwrap();
      assert!(
          loaded.warnings.is_empty(),
          "sample file must have no unknown keys: {:?}",
          loaded.warnings
      );
  }

  #[test]
  fn runtime_config_network_unknown_key_captured_in_extras() {
      let raw = minimal_toml() + "\n";
      // Insert a typo key directly into the [network] section of the full RuntimeConfig.
      let raw = raw.replace(
          "addr = \"0.0.0.0:8443\"",
          "addr = \"0.0.0.0:8443\"\nnetwork_typo_key = \"should_land_in_extra\"",
      );
      let config: RuntimeConfig = toml::from_str(&raw).expect("should parse");
      // Pin the double-flatten contract: HttpsConfig fields must be reachable through
      // the flattened NetworkConfig even when RuntimeConfig is the top-level deserializer.
      assert_eq!(
          config.network.https.addr, "0.0.0.0:8443",
          "HttpsConfig.addr must parse correctly via RuntimeConfig double-flatten"
      );
      assert!(
          config.network.extra.contains_key("network_typo_key"),
          "unknown [network] key must land in network.extra when parsed via RuntimeConfig"
      );
  }
  ```

- [ ] **Step 3: Run the sample test and the new RuntimeConfig test**

  ```bash
  cargo test -p uptrakit-config-reload loader_sample_file_parses_and_validates
  cargo test -p uptrakit-config-reload runtime_config_network_unknown_key_captured_in_extras
  ```

  Expected: both PASS.

- [ ] **Step 4: Run all config-reload tests**

  ```bash
  cargo test -p uptrakit-config-reload
  ```

  Expected: all pass.

- [ ] **Step 5: Commit**

  ```bash
  git commit --only docs/examples/controller.toml \
      crates/shared/config-reload/tests/loader.rs \
      -m "docs(config): add annotated controller.toml sample; add sample parse test"
  ```

---

## Task 6 — Update `controller-runtime`

**Files:**

- Modify: `crates/core/controller-runtime/src/reexec/triage.rs`
- Modify: `crates/core/controller-runtime/src/reload/pki_listener.rs`
- Modify: `crates/core/controller-runtime/src/lib.rs`
- Modify: `crates/core/controller-runtime/src/startup/settings.rs`
- Modify: `crates/core/controller-runtime/src/startup/validation.rs`
- No change: `crates/core/controller-runtime/src/reload/https_listener.rs` — still accesses
  `network.https.addr` / `network.https.clone()` which remain valid since `HttpsConfig` is
  kept as a named Rust struct (only its `extra` field is removed in Task 3).

### 6a — `triage.rs`

- [ ] **Step 1: Update `triage.rs` module doc comment**

  In `crates/core/controller-runtime/src/reexec/triage.rs`, replace:

  ```rust
  //! - `master_key.path` — master encryption key path; the crypto subsystem
  //!   does not support swapping keys at runtime.
  ```

  With:

  ```rust
  //! - `master_key` — master key source URI; the crypto subsystem does not
  //!   support swapping keys at runtime.
  ```

- [ ] **Step 2: Update the `decide` function**

  Replace:

  ```rust
  if prior.master_key.path != new.master_key.path {
      reasons.push("master_key.path");
  }
  ```

  With:

  ```rust
  if prior.master_key != new.master_key {
      reasons.push("master_key");
  }
  ```

- [ ] **Step 3: Update the test module**

  In the `#[cfg(test)]` block:
  1. Change the import to remove `MasterKeyConfig` and add `SecretString`:

     ```rust
     use uptrakit_config_reload::config::{DbConfig, EmbeddedServicesConfig, LogConfig, RuntimeConfig};
     use uptrakit_shared_types::SecretString;
     ```

  2. Change `base_config()`:

     ```rust
     fn base_config() -> RuntimeConfig {
         let mut cfg = RuntimeConfig::default();
         cfg.db = DbConfig::new("sqlite:///var/lib/uptrakit/test.db");
         cfg.master_key = SecretString::new("file:/etc/uptrakit/master.key");
         cfg.log = LogConfig::new("/var/log/uptrakit/controller.log", "info");
         cfg.embedded_services = EmbeddedServicesConfig::default();
         cfg
     }
     ```

  3. Rename `master_key_path_change_requires_reexec` → `master_key_change_requires_reexec`
     and update its body:

     ```rust
     #[test]
     fn master_key_change_requires_reexec() {
         let prior = base_config();
         let mut new = prior.clone();
         new.master_key = SecretString::new("file:/etc/uptrakit/new.key");
         let decision = decide(&prior, &new);
         assert!(decision.needed);
         assert!(decision.reasons.contains(&"master_key"));
     }
     ```

  4. Update `multiple_changes_reported` test — replace `MasterKeyConfig::new(...)`:

     ```rust
     #[test]
     fn multiple_changes_reported() {
         let prior = base_config();
         let mut new = prior.clone();
         new.db = DbConfig::new("sqlite:///var/lib/uptrakit/other.db");
         new.master_key = SecretString::new("file:/etc/uptrakit/new.key");
         let decision = decide(&prior, &new);
         assert!(decision.needed);
         assert_eq!(decision.reasons.len(), 2);
     }
     ```

### 6b — `pki_listener.rs`

- [ ] **Step 4: Rewrite `pki_listener.rs`**

  Replace the entire file with:

  ```rust
  //! PKI listener reloadable subsystem.
  //!
  //! [`PkiListenerReloadable`] distributes updated `pki_addr` strings to the
  //! running PKI listener via a [`tokio::sync::watch`] channel.

  use std::sync::Arc;
  use std::time::Duration;

  use parking_lot::Mutex;
  use rootcause::prelude::*;
  use tokio::sync::watch;
  use uptrakit_config_reload::config::NetworkConfig;
  use uptrakit_config_reload::defaults::WATCHDOG_PKI;
  use uptrakit_config_reload::delta::RuntimeConfigDelta;
  use uptrakit_config_reload::error::ConfigReloadError;
  use uptrakit_config_reload::reloadable::Reloadable;

  use crate::reload::probe::pick_probe_addr;

  /// A [`Reloadable`] that distributes updated `pki_addr` strings.
  #[non_exhaustive]
  pub(crate) struct PkiListenerReloadable {
      tx: watch::Sender<Arc<String>>,
      snapshot: Mutex<Option<Arc<String>>>,
      draining: Mutex<bool>,
  }

  impl PkiListenerReloadable {
      /// Create with the initial `pki_addr` string.
      ///
      /// The returned receiver is discarded at the call site (`_pki_rx`) —
      /// no external subscriber consumes it.
      pub(crate) fn new(initial: String) -> (Self, watch::Receiver<Arc<String>>) {
          let (tx, rx) = watch::channel(Arc::new(initial));
          let this = Self {
              tx,
              snapshot: Mutex::new(None),
              draining: Mutex::new(false),
          };
          (this, rx)
      }
  }

  impl Reloadable for PkiListenerReloadable {
      type Config = NetworkConfig;

      fn name(&self) -> &'static str {
          "pki_listener"
      }

      fn validate(&self, new: &NetworkConfig) -> Result<(), Report> {
          let current = self.tx.borrow().clone();
          if new.pki_addr == *current {
              return Ok(());
          }
          let is_draining = *self.draining.lock();
          if is_draining {
              return Ok(());
          }
          // pki_addr can be an http:// advertisement URL — not a bindable SocketAddr.
          if new.pki_addr.starts_with("http://") {
              return Ok(());
          }
          let probe = std::net::TcpListener::bind(&new.pki_addr).map_err(|e| {
              report!(ConfigReloadError::Validate(format!(
                  "network.pki_addr bind probe failed: {e}"
              )))
          })?;
          drop(probe);
          Ok(())
      }

      async fn apply(&self, new: Arc<NetworkConfig>) -> Result<(), Report> {
          let current = self.tx.borrow().clone();
          {
              let mut guard = self.snapshot.lock();
              *guard = Some(current);
          }
          tracing::info!(addr = %new.pki_addr, "pki listener config applied");
          self.tx.send(Arc::new(new.pki_addr.clone())).ok();
          Ok(())
      }

      async fn revert(&self) -> Result<(), Report> {
          let prior = self.snapshot.lock().clone();
          if let Some(prior) = prior {
              tracing::info!(addr = %*prior, "pki listener config reverted");
              self.tx.send(prior).ok();
          }
          Ok(())
      }

      async fn health_check(&self) -> Result<(), Report> {
          let cfg = self.tx.borrow().clone();
          // pki_addr may be an http:// advertisement URL — pick_probe_addr only handles
          // bare SocketAddr strings. Skip the TCP probe for URL form; the PKI HTTP
          // listener liveness is validated separately by the startup path.
          if cfg.starts_with("http://") || cfg.is_empty() {
              return Ok(());
          }
          let probe_addr = pick_probe_addr(cfg.as_str())?;
          tokio::time::timeout(
              Duration::from_secs(1),
              tokio::net::TcpStream::connect(&probe_addr),
          )
          .await
          .map_err(|_elapsed| {
              report!(ConfigReloadError::HealthFailed {
                  subsystem: "pki_listener".into(),
                  message: format!("connect to {probe_addr} timed out after 1s"),
              })
          })?
          .map_err(|e| {
              report!(ConfigReloadError::HealthFailed {
                  subsystem: "pki_listener".into(),
                  message: e.to_string(),
              })
          })?;
          tracing::debug!(addr = %probe_addr, "pki listener health check ok");
          Ok(())
      }

      fn rollback_window(&self) -> Duration {
          WATCHDOG_PKI
      }
  }

  uptrakit_config_reload::reloadable_erased_impl!(PkiListenerReloadable, RuntimeConfigDelta::Network);

  #[cfg(test)]
  mod tests {
      use super::*;

      #[tokio::test]
      async fn pki_reloadable_skip_pre_bind_on_same_addr() {
          let (r, _rx) = PkiListenerReloadable::new("127.0.0.1:0".to_string());
          let mut net = NetworkConfig::default();
          net.pki_addr = "127.0.0.1:0".to_string();
          r.validate(&net).unwrap();
      }

      #[tokio::test]
      async fn pki_reloadable_apply_updates_receiver() {
          let (r, rx) = PkiListenerReloadable::new("127.0.0.1:0".to_string());
          let mut net = NetworkConfig::default();
          net.pki_addr = "127.0.0.1:9".to_string();
          r.apply(Arc::new(net)).await.unwrap();
          assert!(rx.has_changed().unwrap());
      }

      #[tokio::test]
      async fn pki_reloadable_revert_restores_prior() {
          let (r, mut rx) = PkiListenerReloadable::new("127.0.0.1:0".to_string());

          let mut net = NetworkConfig::default();
          net.pki_addr = "127.0.0.1:9".to_string();
          r.apply(Arc::new(net)).await.unwrap();
          rx.changed().await.unwrap();

          r.revert().await.unwrap();
          assert!(rx.has_changed().unwrap());
          let restored = rx.borrow_and_update().clone();
          assert_eq!(*restored, "127.0.0.1:0");
      }

      #[tokio::test]
      async fn pki_reloadable_skip_pre_bind_while_draining() {
          let (r, _rx) = PkiListenerReloadable::new("127.0.0.1:0".to_string());
          *r.draining.lock() = true;

          let mut net = NetworkConfig::default();
          net.pki_addr = "127.0.0.1:9999".to_string();
          r.validate(&net).unwrap();
      }
  }
  ```

### 6c — `lib.rs` (master key source)

- [ ] **Step 5: Replace master key source resolution in `lib.rs`**

  In `crates/core/controller-runtime/src/lib.rs`, find the block starting with the
  `// Phase 1: Master key initialization` comment (around line 304). Replace:

  ```rust
  // Phase 1: Master key initialization — reads from --master-key-from or TOML
  // master_key.path as a fallback.
  let master_key_source = args.master_key_from.as_deref().or_else(|| {
      let p = runtime.master_key.path.as_str();
      if p.is_empty() { None } else { Some(p) }
  });
  // Build a `file:` prefixed source if we got a bare path from TOML.
  let master_key_from_toml_buf;
  let master_key_source = if let Some(src) = master_key_source {
      if !src.starts_with("file:")
          && !src.starts_with("env:")
          && !runtime.master_key.path.is_empty()
          && src == runtime.master_key.path.as_str()
      {
          master_key_from_toml_buf = format!("file:{src}");
          Some(master_key_from_toml_buf.as_str())
      } else {
          Some(src)
      }
  } else {
      None
  };
  let master_key_hex = startup::init_master_key(master_key_source)?;
  ```

  With:

  ```rust
  // Phase 1: Master key initialization — CLI flag takes precedence over TOML master_key.
  let toml_key = runtime.master_key.expose_secret();
  let master_key_source = args.master_key_from.as_deref().or_else(|| {
      if toml_key.is_empty() { None } else { Some(toml_key) }
  });
  let master_key_hex = startup::init_master_key(master_key_source)?;
  ```

### 6d — `startup/settings.rs`

- [ ] **Step 6: Update `pki.addr` access in `settings.rs`**

  In `crates/core/controller-runtime/src/startup/settings.rs`, find (around line 168):

  ```rust
  // PKI addr from TOML [network.pki].
  let toml_pki_addr = runtime.network.pki.addr.clone();
  ```

  Replace with:

  ```rust
  // PKI addr from TOML network.pki_addr.
  let toml_pki_addr = runtime.network.pki_addr.clone();
  ```

### 6e — `startup/validation.rs`

- [ ] **Step 7: Update warning strings in `validation.rs`**

  In `crates/core/controller-runtime/src/startup/validation.rs`, find the warn message:

  ```text
  "network.pki.addr uses http:// scheme but has no explicit port; \
   the built-in PKI HTTP listener will not start"
  ```

  Replace with:

  ```text
  "network.pki_addr uses http:// scheme but has no explicit port; \
   the built-in PKI HTTP listener will not start"
  ```

  And:

  ```text
  "network.pki.addr URL could not be parsed; PKI HTTP listener disabled"
  ```

  Replace with:

  ```text
  "network.pki_addr URL could not be parsed; PKI HTTP listener disabled"
  ```

- [ ] **Step 8: Update `PkiListenerReloadable::new` call in `lib.rs`**

  Find (line 820):

  ```rust
  reload::pki_listener::PkiListenerReloadable::new(b.runtime.network.pki.clone())
  ```

  Replace with:

  ```rust
  reload::pki_listener::PkiListenerReloadable::new(b.runtime.network.pki_addr.clone())
  ```

- [ ] **Step 9: Verify compilation**

  ```bash
  cargo check -p uptrakit-controller-runtime
  ```

  If any remaining `PkiConfig` or `master_key.path` references remain, the compiler will
  identify them — fix each one.

- [ ] **Step 10: Run controller-runtime tests**

  ```bash
  cargo test -p uptrakit-controller-runtime
  ```

  Expected: all tests pass including the renamed `master_key_change_requires_reexec` and
  all four `pki_reloadable_*` tests.

- [ ] **Step 11: Commit**

  ```bash
  git commit --only crates/core/controller-runtime/src/reexec/triage.rs \
      crates/core/controller-runtime/src/reload/pki_listener.rs \
      crates/core/controller-runtime/src/lib.rs \
      crates/core/controller-runtime/src/startup/settings.rs \
      crates/core/controller-runtime/src/startup/validation.rs \
      -m "refactor(config): update controller-runtime for flat config format"
  ```

---

## Task 7 — Update `web-api` AppState

**Files:**

- Modify: `crates/ui/web-api/src/app_state.rs`

- [ ] **Step 1: Update `master_key_config_rx` type annotation**

  In `crates/ui/web-api/src/app_state.rs` around line 347, replace:

  ```rust
  /// Master key config watch receiver.
  pub master_key_config_rx: tokio::sync::watch::Receiver<
      std::sync::Arc<uptrakit_config_reload::config::MasterKeyConfig>,
  >,
  ```

  With:

  ```rust
  /// Master key source watch receiver (boot-time only).
  pub master_key_config_rx: tokio::sync::watch::Receiver<
      std::sync::Arc<uptrakit_shared_types::SecretString>,
  >,
  ```

- [ ] **Step 2: Verify full compilation**

  ```bash
  cargo check --all-features
  ```

  Expected: no errors. If any remaining site references `MasterKeyConfig` by name, the
  compiler will identify it — update those sites to use `uptrakit_shared_types::SecretString`
  or `uptrakit_wire::SecretString` (both are the same type).

- [ ] **Step 3: Run full test suite**

  ```bash
  cargo test --all-features
  ```

  Expected: all tests pass.

- [ ] **Step 4: Commit**

  ```bash
  git commit --only crates/ui/web-api/src/app_state.rs \
      -m "refactor(config): update AppState master_key_config_rx type to SecretString"
  ```

---

## Task 8 — Update documentation

**Files:**

- Modify: `docs/adr/0008-graceful-reload-architecture.md`
- Modify: `docs/development/coding-standards.md`

- [ ] **Step 1: Amend `docs/adr/0008-graceful-reload-architecture.md`**

  Find the **Consequences** section. Add an amendment note documenting the schema changes.
  Append to that section (exact text depends on the ADR's existing content — add a new
  subsection or list item):

  ```markdown
  **2026-05-17 amendment:** Config schema flattened for clarity:

  - `[master_key] path = "..."` → top-level `master_key = "file:..."` (`SecretString`).
    All three CLI forms (`file:`, `env:`, inline hex) are now available in TOML.
  - `[network.https]` + `[network.pki]` sub-tables merged into a flat `[network]` section.
    `network.pki.addr` renamed to `network.pki_addr`. `PkiConfig` struct removed;
    `HttpsConfig` retained as internal Rust type with `#[serde(flatten)]`.
  - Irreversibly-bound key name: `master_key` (was `master_key.path`).
  ```

- [ ] **Step 2: Update `docs/development/coding-standards.md`**

  Find the reference to `master_key.path` in the reexec trigger documentation (search for
  `master_key.path`). Replace each occurrence with `master_key`.

  Example: if you find:

  ```text
  e.g. db.url, master_key.path, log.path
  ```

  Replace with:

  ```text
  e.g. db.url, master_key, log.path
  ```

- [ ] **Step 3: Lint docs**

  ```bash
  markdownlint --config .markdownlint.json docs/adr/0008-graceful-reload-architecture.md \
               docs/development/coding-standards.md
  ```

  Fix any lint violations (line length > 150, missing blank lines around fences, etc.).

- [ ] **Step 4: Commit**

  ```bash
  git commit --only docs/adr/0008-graceful-reload-architecture.md \
      docs/development/coding-standards.md \
      -m "docs(config): amend ADR-0008 and coding-standards for flat config schema"
  ```

---

## Task 9 — Final verification

- [ ] **Step 1: Full lint + test sweep**

  ```bash
  cargo fmt --all -- --check
  cargo check --no-default-features --features db-sqlite
  cargo check --all-features
  cargo clippy --all-targets --no-default-features --features db-sqlite
  cargo clippy --all-targets --all-features
  cargo test --all-features
  cargo deny check
  markdownlint --config .markdownlint.json '**/*.md'
  ```

  Expected: no failures. Fix any clippy warnings (unused imports, renamed paths, etc.).

- [ ] **Step 2: Mark spec implemented in pending-specs tracker**

  Edit `.superpowers/pending-specs.md`. Find the "Config Format Overhaul" section and
  change its status from `NO_PLAN` to `IN_PROGRESS` (update to `MOSTLY_DONE` or a final
  status once the feature is fully merged).
