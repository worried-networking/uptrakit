# NATS Config Section Optional Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the `[nats]` TOML section fully optional so the controller starts without it, even when compiled with the `nats` feature.

**Architecture:** Two changes in `crates/shared/config-reload/src/config/mod.rs`: (1) add
`#[serde(default)]` to the `nats` field so serde no longer requires the TOML section; (2)
guard the `self.nats.validate()` call in `RuntimeConfig::validate()` to skip it when
`nats.url` is empty. No other files change — `NatsConfig::validate()` and `NatsReloadable`
remain untouched.

**Tech Stack:** Rust, serde + serde_json, toml crate, rootcause error handling.

---

## Files

| Action | Path                                            | What changes                                                                                 |
| ------ | ----------------------------------------------- | -------------------------------------------------------------------------------------------- |
| Modify | `crates/shared/config-reload/src/config/mod.rs` | `#[serde(default)]` on `nats` field (line 53); guard around `self.nats.validate()` (line 79) |
| Modify | `crates/shared/config-reload/tests/loader.rs`   | New `minimal_toml_without_nats()` helper + one new test                                      |

---

### Task 1: Write the failing test

**Files:**

- Modify: `crates/shared/config-reload/tests/loader.rs`

- [ ] **Step 1: Add the helper and test to `tests/loader.rs`**

  Append to the `// ── RuntimeConfig tests ──` section, after the existing
  `runtime_config_captures_unknown_keys` test (around line 273):

  ```rust
  fn minimal_toml_without_nats() -> String {
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

  #[test]
  fn runtime_validate_skips_nats_when_url_empty() {
      // NatsConfig::validate() errors on empty URL (unchanged behaviour).
      assert!(NatsConfig::default().validate().is_err());
      // RuntimeConfig::validate() must NOT propagate that error when nats.url is empty:
      // the [nats] section is optional; an absent/empty section means NATS disabled.
      let cfg: RuntimeConfig =
          toml::from_str(&minimal_toml_without_nats()).expect("parse must succeed without [nats]");
      assert!(cfg.nats.url.is_empty());
      cfg.validate()
          .expect("RuntimeConfig with no [nats] section must validate successfully");
  }
  ```

- [ ] **Step 2: Run the new test to confirm it fails**

  ```bash
  cargo test -p uptrakit-config-reload runtime_validate_skips_nats_when_url_empty -- --nocapture
  ```

  Expected: **FAIL** with either `missing field 'nats'` (serde error) or
  `config validation failed: nats.url is empty` (validate error). Both are acceptable
  at this point — either confirms the fix is needed.

---

### Task 2: Implement the fix in `mod.rs`

**Files:**

- Modify: `crates/shared/config-reload/src/config/mod.rs`

- [ ] **Step 1: Add `#[serde(default)]` to the `nats` field (line 53)**

  Change:

  ```rust
      /// NATS messaging server settings.
      pub nats: NatsConfig,
  ```

  To:

  ```rust
      /// NATS messaging server settings.
      #[serde(default)]
      pub nats: NatsConfig,
  ```

- [ ] **Step 2: Guard `self.nats.validate()` in `RuntimeConfig::validate()` (line 79)**

  Change:

  ```rust
          self.nats.validate()?;
  ```

  To:

  ```rust
          if !self.nats.url.is_empty() {
              self.nats.validate()?;
          }
  ```

  The guard skips NATS validation when the URL is empty (= NATS disabled). When the
  URL is non-empty, `NatsConfig::validate()` is still called — preserving all existing
  validation behaviour for configured NATS deployments.

- [ ] **Step 3: Run the new test to confirm it passes**

  ```bash
  cargo test -p uptrakit-config-reload runtime_validate_skips_nats_when_url_empty -- --nocapture
  ```

  Expected: **PASS**

- [ ] **Step 4: Run the full `config-reload` test suite**

  ```bash
  cargo test -p uptrakit-config-reload --all-features
  ```

  Expected: **all tests pass**. Pay attention to:
  - `nats_validates_url` — must still pass (calls `NatsConfig::validate()` directly; empty URL still errors)
  - `runtime_config_full_round_trip` — must still pass (`minimal_toml()` includes
    `[nats] url = "nats://localhost:4222"`, exercising the non-empty-URL branch of the
    new guard — both guard branches are covered)
  - `loader_sample_file_parses_and_validates` — must still pass (sample file has `[nats]` section)
  - `nats_validate_rejects_empty_url` in `controller-runtime` — must still pass (also unchanged)

- [ ] **Step 5: Run clippy**

  ```bash
  cargo clippy -p uptrakit-config-reload --all-targets --all-features
  ```

  Expected: no warnings.

- [ ] **Step 6: Run fmt**

  ```bash
  cargo fmt -p uptrakit-config-reload
  ```

- [ ] **Step 7: Commit**

  ```bash
  git commit --only crates/shared/config-reload/src/config/mod.rs \
                    crates/shared/config-reload/tests/loader.rs \
             -m "fix(config): make [nats] TOML section optional at runtime"
  ```

---

## Docs

No documentation changes required. `ARCHITECTURE.md` references `NatsAccess` (a service
credential capability) not the controller TOML config section. `CONTEXT.md` requires no
new domain terms. The spec declares no doc impact.

---

## Known Limitations (out of scope)

- **DB-persisted URL wins on subsequent starts.** If a NATS URL was previously saved to
  the settings DB (via the admin settings API), the DB value takes precedence over TOML
  on subsequent starts. Removing `[nats]` from TOML only disables NATS on a fresh
  deployment where no URL has been persisted. Clearing a persisted URL requires the admin
  settings API or directly clearing the `nats.url` key in the DB.
- **Hot-enabling NATS is silently ignored.** Adding `nats.url` to a live config when NATS
  was disabled at startup is silently dropped — no `NatsReloadable` is registered so the
  reload coordinator ignores the delta. A restart is required to enable NATS. A
  `tracing::warn!` for this case is deferred to a follow-up.
- **Whitespace-only URL is not treated as empty.** `url = "   "` passes the `is_empty()`
  guard and propagates to the NATS client, which fails hard at connection time.
