# Config Format Overhaul

**Date:** 2026-05-17
**Status:** Approved — awaiting implementation plan

## Summary

Three focused changes to the controller TOML config format:

1. Flatten `[master_key]` to a top-level string supporting all three key-source URI forms.
2. Flatten `[network.https]` + `[network.pki]` sub-tables into a single `[network]` section;
   rename `pki.addr` → `pki_addr` (flattening only; semantics unchanged).
3. Add an annotated sample at `docs/examples/controller.toml`; add tests that parse the sample
   and guard the new inline-key permission check.

Backwards compatibility is not required. No other section renames.

---

## 1. Schema changes

### 1.1 `master_key` — top-level string

**Before:**

```toml
[master_key]
path = "/etc/uptrakit/master.key"
```

`path` was a bare file path (absolute). The controller prepended `file:` internally before
calling `init_master_key`.

**After:**

```toml
# file URI (recommended)
master_key = "file:/etc/uptrakit/master.key"

# environment variable
master_key = "env:UPTRAKIT_MASTER_KEY"

# inline hex (64 chars = 32 bytes); config file MUST be mode 0600 or stricter
master_key = "0a1b2c3d..."
```

Accepts the same three URI forms the CLI flag `--master-key-from` already accepts.
No normalization or prefix injection at startup — passed directly to `init_master_key`.

`MasterKeyConfig` struct is removed entirely. `RuntimeConfig.master_key` becomes
`SecretString` (from `uptrakit-shared-types`, which must be added as a new `[dependencies]`
entry in `crates/shared/config-reload/Cargo.toml` — currently absent). `SecretString` derives
`Deserialize` with `#[serde(transparent)]` so TOML string values deserialize transparently.

The inline hex form (`"0a1b2c3d..."`) embeds actual key material directly in the config struct.
Using `SecretString` ensures `Debug` auto-masks the value and prevents accidental logging.
The `file:` and `env:` forms contain no sensitive data but the field type is uniform regardless
of which form is used.

Call sites checking prefixes use `.expose_secret()`: `key.expose_secret().starts_with("file:")`.
`PartialEq` is derived on `SecretString` (compares the inner strings), so `triage.rs` can use
`prior.master_key != new.master_key` directly without `.expose_secret()`.

**Config-level validation** (in the existing `validate()` call path): the `master_key` field
must pass these format checks, which are cheap and do not require the file or env var to exist:

- Non-empty.
- `file:` form: the path component after the prefix is non-empty
  (i.e. `key.expose_secret().len() > "file:".len()`).
- `env:` form: the variable name after the prefix is non-empty
  (i.e. `key.expose_secret().len() > "env:".len()`).
- Inline form: the value is exactly 64 hex characters (verifiable via `str::len()` and `str::chars().all(|c| c.is_ascii_hexdigit())`).

This makes `validate_only` (`uptrakit-ctl config validate`) trustworthy for pre-flight checks on
all three forms. Deeper validation (resolving the file, reading the env var, decoding the hex)
remains in `init_master_key` at startup.

`RuntimeConfigChannels/Receivers` watch channel changes from
`watch::Sender<Arc<MasterKeyConfig>>` to `watch::Sender<Arc<SecretString>>` (boot-time only;
no live reload variant exists), consistent with the `Arc<T>` wrapping used by all other channels.
`AppState.master_key_config_rx` type declaration updates to `watch::Receiver<Arc<SecretString>>`.
The 13 sites that assign `config_receivers.master_key` / `config_rx.master_key` to this field
(app_state.rs, test_harness/mod.rs, lib.rs, resolve_ip.rs, require_auth.rs, auth.rs, mfa.rs,
me_2fa.rs, services.rs, settings_nats.rs, surfaces.rs, service_ws/handler/mod.rs, and the
channels.rs `from_runtime` constructor) compile automatically once the channel type changes —
no per-site edits needed beyond the `AppState` struct and `channels.rs`.

`triage.rs` comparison: `prior.master_key != new.master_key` → reason string `"master_key"`.

### 1.2 `[network]` — flattened

**Before:**

```toml
[network.https]
addr            = "0.0.0.0:8443"
trusted_proxies = ["10.0.0.0/8"]
real_ip_header  = "x-forwarded-for"
forwarded_client_cert_info_header = "x-forwarded-client-cert"
forwarded_client_cert_pem_header  = "x-forwarded-client-cert-pem"

[network.pki]
addr = "http://uptrakit.example.com:8080"
```

`network.pki.addr` has dual semantics controlled by a scheme prefix:

- **No scheme** (`0.0.0.0:8080`) — pure bind address; `PkiListenerReloadable` probes it with
  `TcpListener::bind`; embedded PKI HTTP listener is NOT started (no port extraction).
- **`http://` scheme** (`http://hostname:8080`) — advertisement URL; `startup/validation.rs`
  extracts the port via `url::Url::parse` and starts the embedded PKI HTTP listener on
  `0.0.0.0:<port>`; the field also seeds `global_settings` `network.pki_addr` for CA cert SANs
  and zeroconf advertisement.

The misleading comment in `NetworkConfig::validate()` ("public PKI URL, not a bind address") is
incorrect for the no-scheme case. Correct description: address or URL for the PKI endpoint,
driving both the embedded listener (`http://` scheme only) and the advertisement value.

**After:**

```toml
[network]
addr            = "0.0.0.0:8443"
pki_addr        = "http://uptrakit.example.com:8080"
trusted_proxies = ["10.0.0.0/8"]
real_ip_header  = "x-forwarded-for"
forwarded_client_cert_info_header = "x-forwarded-client-cert"
forwarded_client_cert_pem_header  = "x-forwarded-client-cert-pem"
```

Field name changes from `network.pki.addr` to `network.pki_addr` (section flattened). Dual
semantics are unchanged.

`PkiConfig` struct is removed; `pki_addr` becomes a direct `String` field on `NetworkConfig`.

`HttpsConfig` is **retained as an internal Rust type** — `HttpsListenerReloadable` owns a
`watch::Sender<Arc<HttpsConfig>>` channel and cannot be removed without touching that reloadable.
TOML representation changes: the `https` field on `NetworkConfig` gains `#[serde(flatten)]`,
and `HttpsConfig.extra: HashMap` is removed (unknown keys at the `[network]` level are caught by
`NetworkConfig.extra` only — one flatten catch-all per level is a serde invariant).

**Implementation note:** Before committing the flatten approach, verify that `NetworkConfig.extra`
still captures genuinely unknown keys (e.g., a typo like `addrr = "..."` in `[network]`) when
both `#[serde(flatten)] https: HttpsConfig` and `#[serde(flatten)] extra: HashMap` coexist. Add
a test that writes an unknown key in `[network]` and asserts `warn_about_extras()` emits a
warning. If the catch-all silently swallows or misses the key, prefer expanding `HttpsConfig`
fields directly onto `NetworkConfig` and constructing `HttpsConfig` from `NetworkConfig` in
`HttpsListenerReloadable` via a `From<&NetworkConfig>` conversion.

The `[network.https]` and `[network.pki]` sub-tables disappear from TOML. `NetworkConfig.https`
(flattened) and `NetworkConfig.pki_addr: String` remain in the Rust struct.

`warn_about_extras` drops the `network.https.*` and `network.pki.*` loops (those sub-structs no
longer carry their own `extra` maps).

Validation: the misleading `SocketAddr`-exclusion comment is corrected to accurately describe
the dual-scheme semantics. `pki_addr` is still not validated as a `SocketAddr`.

### 1.3 Unchanged sections

| Section               | Change                                                           |
| --------------------- | ---------------------------------------------------------------- |
| `[db]`                | none                                                             |
| `[nats]`              | none (NATS config will gain auth/TLS fields; keeping as section) |
| `[tls]`               | none                                                             |
| `[audit]`             | none                                                             |
| `[log]`               | none                                                             |
| `[zeroconf]`          | none                                                             |
| `[embedded_services]` | none (kept verbatim for clarity)                                 |

---

## 2. Permission check

**Trigger:** `master_key` is non-empty AND does not start with `file:` or `env:` — i.e., the
config file contains inline key material.

**Location:** `TomlConfigLoader::load()`, after parse and before returning `LoadedConfig`.
Applied on every load (initial startup, file-watch reload, `validate_only`).

**Unix check:**

```rust
#[cfg(unix)]
fn check_config_permissions(path: &Path, config: &RuntimeConfig) -> Result<(), Report> {
    let key = config.master_key.expose_secret();
    if key.is_empty() || key.starts_with("file:") || key.starts_with("env:") {
        return Ok(());
    }
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)
        .map_err(|e| report!(ConfigReloadError::TomlIo {
            path: path.to_path_buf(),
            source_msg: e.to_string(),
        }))?
        .permissions()
        .mode();
    if mode & 0o077 != 0 {
        bail!(ConfigReloadError::Validate(format!(
            "config file {:?} contains an inline master key and must not be readable by \
             group or other (current mode: {:04o}); run: chmod 0600 {:?}",
            path, mode & 0o777, path
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

**What the check does NOT do:**

- Does not validate the inline value as hex (`parse_master_key_hex` at startup handles that).
- Does not check `file:` or `env:` forms (those don't embed key material in the config).
- Does not enforce access control on Windows — emits a `warn!` instead so operators have
  in-band signal that the check was skipped.

---

## 3. Config sample

**Location:** `docs/examples/controller.toml`

The sample is the primary reference for all available parameters. All parameters have inline `#`
comments describing type, default, and effect. Required parameters are uncommented; optional
parameters with defaults are shown commented-out at their default values.

The sample uses `master_key = "file:/etc/uptrakit/master.key"` (no inline key) so it can be
loaded in tests without triggering the permission check and without a real key file on disk.

Future plan: `docs/end-user/configuration.md` prose page linking to the sample (deferred).

---

## 4. Tests

All tests live in `crates/shared/config-reload/tests/loader.rs` unless noted.

### 4.1 Sample file parse guard (new)

`loader_sample_file_parses_and_validates` — calls
`TomlConfigLoader::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../docs/examples/controller.toml"))`,
asserts `Ok`, asserts no warnings (unknown keys). Acts as a CI tripwire: fails the moment the
sample diverges from the schema.

The path uses `concat!(env!("CARGO_MANIFEST_DIR"), "...")` to produce a workspace-root-relative
absolute path at compile time; the crate root is `crates/shared/config-reload/`, so three `..`
levels reach the workspace root.

The `file:` form in the sample means the permission check does not fire and no key file needs
to exist.

### 4.2 Inline master key permission checks (new, Unix-only)

`loader_inline_master_key_rejects_permissive_config` — write a minimal valid TOML with an
inline 64-char hex `master_key` to a tempfile; `chmod 0644`; assert `TomlConfigLoader::load`
returns `Err` whose message contains `"chmod 0600"`.

`loader_inline_master_key_accepts_strict_config` — same TOML, `chmod 0600`; assert
`TomlConfigLoader::load` returns `Ok`.

Both gated `#[cfg(unix)]`.

### 4.3 Updated triage tests (`crates/core/controller-runtime/src/reexec/triage.rs`)

`base_config()` helper: `master_key` changes from
`MasterKeyConfig::new("/etc/uptrakit/master.key")` to
`SecretString::new("file:/etc/uptrakit/master.key")`.

`master_key_change_requires_reexec`: asserts
`decision.reasons.contains(&"master_key")` (not `"master_key.path"`).

### 4.4 Existing tests

No existing tests reference `HttpsConfig` or `PkiConfig` by name in their public API (confirmed
by grep). The following test files contain inline TOML with `[network.https]` / `[network.pki]`
sub-tables and `[master_key]` sections that must be updated to the new flat format:

- `crates/shared/config-reload/tests/loader.rs` — `minimal_toml()` helper (≈ lines 187–229)
- `crates/shared/config-reload/tests/coordinator.rs` — inline TOML at ≈ lines 364–393

---

## 5. Documentation deliverables

| Artifact                                        | Action                                                          |
| ----------------------------------------------- | --------------------------------------------------------------- |
| `docs/examples/controller.toml`                 | New — annotated sample, all params                              |
| `CONTEXT.md`                                    | Already updated — irreversibly-bound key set: `master_key`      |
| `docs/adr/0008-graceful-reload-architecture.md` | Amendment — note schema changes in Consequences                 |
| `docs/development/coding-standards.md`          | Update reexec trigger example: `master_key.path` → `master_key` |
| `docs/end-user/configuration.md`                | Deferred — future prose page                                    |

---

## 6. Files touched

| File                                                        | Change                                                                                                                                                                                                                         |
| ----------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `crates/shared/config-reload/Cargo.toml`                    | Add `uptrakit-shared-types = { workspace = true }` to `[dependencies]` (needed for `SecretString`)                                                                                                                             |
| `crates/shared/config-reload/src/config/master_key.rs`      | Delete                                                                                                                                                                                                                         |
| `crates/shared/config-reload/src/config/network.rs`         | Remove `PkiConfig`; add `pki_addr: String`; `#[serde(flatten)]` on `https`; remove `HttpsConfig.extra`                                                                                                                         |
| `crates/shared/config-reload/src/config/mod.rs`             | `RuntimeConfig.master_key: SecretString`; remove `PkiConfig` export; update `warn_about_extras` (remove `master_key.extra`, `network.https.extra`, `network.pki.extra` loops)                                                  |
| `crates/shared/config-reload/src/channels.rs`               | `master_key` channel: `watch::Sender<Arc<SecretString>>`                                                                                                                                                                       |
| `crates/shared/config-reload/src/loader.rs`                 | Add `check_config_permissions`; call in `load()`                                                                                                                                                                               |
| `crates/core/controller-runtime/src/lib.rs`                 | Remove `file:` prefix injection; read `runtime.master_key` directly                                                                                                                                                            |
| `crates/core/controller-runtime/src/reexec/triage.rs`       | `prior.master_key.path` → `prior.master_key`; reason `"master_key"`                                                                                                                                                            |
| `crates/core/controller-runtime/src/reload/pki_listener.rs` | Remove `PkiConfig` import; internal sender type changes from `Arc<PkiConfig>` to `Arc<String>` (no external consumers — `_pki_rx` discarded at call site); validate/apply/revert/health_check change `.pki.addr` → `.pki_addr` |
| `crates/core/controller-runtime/src/startup/settings.rs`    | `runtime.network.pki.addr` → `runtime.network.pki_addr`                                                                                                                                                                        |
| `crates/core/controller-runtime/src/startup/validation.rs`  | Update warn-message strings from `"network.pki.addr"` → `"network.pki_addr"`                                                                                                                                                   |
| `crates/ui/web-api/src/app_state.rs`                        | `master_key_config_rx`: `watch::Receiver<Arc<SecretString>>`                                                                                                                                                                   |
| `crates/shared/config-reload/tests/loader.rs`               | Add 3 new tests; update `minimal_toml()` TOML to flat format                                                                                                                                                                   |
| `crates/shared/config-reload/tests/coordinator.rs`          | Update inline TOML to flat format                                                                                                                                                                                              |
| `docs/examples/controller.toml`                             | New — annotated sample                                                                                                                                                                                                         |
| `docs/adr/0008-graceful-reload-architecture.md`             | Amendment in Consequences                                                                                                                                                                                                      |
| `docs/development/coding-standards.md`                      | Update reexec example: `master_key.path` → `master_key`                                                                                                                                                                        |

---

## 7. Out of scope

- Backwards-compatible migration path (not required).
- NATS auth/TLS fields (future work; `[nats]` section kept to accommodate them).
- Windows permission enforcement.
- `docs/end-user/configuration.md` prose page (deferred).
- Startup refactor into testable phases (prerequisite for integration-level startup test; deferred).
