# Code Review: Controller Crate

**Crate:** `crates/core/controller/`
**Date:** 2026-02-13
**Scope:** 13 source files, ~5,000 lines (excluding migrations; ~7,400 total)

---

## Architecture

**Rating: Excellent**

- Well-organized binary: CLI parsing, DB init, PKI/TLS setup, background tasks,
  HTTP server
- Watch channel architecture for CA updates (`tokio::sync::watch`)
- CAS (Compare-And-Swap) pattern for multi-instance CA rotation safety
  (`update_setting_string_cas` in `pki.rs:633` uses a `WHERE` filter on the
  expected fingerprint before swapping)
- Version-gated CRL polling (60s polls + event-driven via `Notify`)
- Graceful shutdown with `CancellationToken` and 5s per-task timeout
- Settings reconciliation with deterministic DB > CLI precedence

## Security and Safety

**Rating: Excellent**

- aws-lc-rs crypto provider (FIPS-eligible); ECDSA P-256 keys; SHA-256
  fingerprints
- mTLS with `WebPkiClientVerifier` + CRL support + hot-reload
- CA key material uses `zeroize::Zeroizing<String>` (`pki.rs:208`)
- DB URL credential masking via `sanitize_url()` (`db/mod.rs:34`)
- PKI address validation (http/https scheme only)
- Trusted proxy validation with `IpNet` CIDR support (`cli.rs:226`)
- CSR CN verification matches `agent_id`; cert lifetime capped at 730 days
  (`cert_signer.rs:19`)

## High Availability

**Rating: Excellent**

- **CA rotation**: Database-level CAS (`pki.rs:633` — compares
  `active_fingerprint` before swap) prevents concurrent rotations from
  conflicting; losing instances reload from DB.
- **CRL management**: Version-gated polling with `AtomicI64`
  (`crl_manager.rs:45`) + DB `revocation_version`. Each instance polls at
  `CRL_POLL_INTERVAL` (60s, `durations.rs`) and rebuilds CRL only when version
  changes; local revocations optimistically bump the version (`crl_manager.rs:264`).
- **Settings**: Atomic publish via `tokio::sync::watch`; cross-instance sync via
  DB `settings_version` counter polled at `SETTINGS_POLL_INTERVAL` (30s,
  `durations.rs`) in `tasks.rs`.
- **Server cert renewal**: Only occurs when cert is within
  `SERVER_CERT_RENEWAL_WINDOW_DAYS` (30 days, `durations.rs`); file-based locking
  via PKI directory.
- **Master key verification**: On startup, a sentinel token is encrypted and
  stored in the DB. Subsequent startups decrypt and verify the token, failing
  with `MasterKeyMismatch` if the key does not match (`startup.rs`).

## Code Quality

**Rating: Excellent**

- No dead code or unused imports found
- Minimal code duplication (reconciliation logic properly abstracted)
- Comprehensive test coverage in every module
- Clear separation of concerns between modules

## Coding Standards Compliance

**All Passed**

- No `#[allow()]` in source (only `#[cfg_attr(..., allow(...))]` for
  feature-gated code in `db/config.rs:13` and `db/config.rs:42` -- acceptable)
- No `unsafe` anywhere
- No `panic!()` in production code
- `unwrap()`/`.expect()` only on hardcoded constants:
  - `startup.rs` -- `.expect("valid default HTTPS addr")` on hardcoded
    `"[::]:8443"`
  - `startup.rs` -- `.expect("already validated")` on URL already validated by
    CLI parser
- No raw SQL (all SeaORM)
- Error handling consistently uses `thiserror` + `rootcause` +
  `impl_report_conversion!`
- No secrets in logs

## Findings

**0 Critical, 0 High, 1 Medium, 2 Low (2 fixed)**

### MEDIUM: `db/config.rs:65` -- safe but unclear `unwrap_or`

`.unwrap_or("unknown")` on `split("://").next()` is technically safe
(`split` always yields at least one element) but could use a constant or comment
for clarity.

```rust
url.split("://").next().unwrap_or("unknown")
```

### ~~LOW: Hardcoded durations could be named constants~~ **FIXED**

**Status:** Resolved. All hardcoded durations extracted to named constants in
`crates/core/controller/src/durations.rs`. Constants include `CA_ROTATION_WINDOW_DAYS`,
`SERVER_CERT_RENEWAL_WINDOW_DAYS`, `SERVER_CERT_VALIDITY_DAYS`, `CRL_POLL_INTERVAL`,
`SETTINGS_POLL_INTERVAL`, `CA_ROTATION_CHECK_INTERVAL`, `SERVER_CERT_RENEWAL_CHECK_INTERVAL`,
`AUTH_CLEANUP_INTERVAL`, `BACKGROUND_TASK_SHUTDOWN_TIMEOUT`, and `RESTART_NOTIFICATION_SCATTER`.
All modules (`pki.rs`, `crl_manager.rs`, `tasks.rs`) reference these constants.

### ~~LOW: `reconcile_setting_vec()` duplicates logic pattern~~ **FIXED**

**Status:** Resolved as part of the controller refactor. `reconcile_setting_vec()` and
`reconcile_socket_addr()` moved from `main.rs` to `startup.rs` alongside the reconciliation
phase function, improving locality and reducing the monolithic `run()` function.

## Extensibility Assessment

The controller is **not intended as a template** for external developers -- it is
the unique central server. However, several issues affect maintainability and
would need to be addressed before the codebase could support embedded or
alternative controller configurations.

### ~~MAJOR: `main.rs` has a monolithic ~1,200-line `run()` function~~ **FIXED**

**Status:** Resolved. The monolithic `run()` function has been decomposed into named
phase functions across three new modules:

- **`durations.rs`** — Named constants for all timing values (CA rotation window,
  CRL poll interval, settings poll interval, etc.)
- **`startup.rs`** — Phase functions: `init_master_key()`, `init_database()`,
  `verify_master_key()`, `reconcile_all_settings()`, `bootstrap_oidc()`,
  `validate_configuration()`, `init_pki_runtime()`, `init_jwt()`. Intermediate
  result structs: `ReconciledSettings`, `ValidatedConfig`, `PkiRuntime`.
- **`tasks.rs`** — `BackgroundTasks` struct for coordinated shutdown, individual
  `spawn_*` functions for each background task.

The `run()` function in `main.rs` is now ~260 lines with clear phase annotations.

### MINOR: Direct sea-orm entity operations for OIDC bootstrap in `startup.rs`

Lines ~388-495 contain raw `ActiveModel` operations for OIDC provider
bootstrapping directly in `main.rs`. This logic should be in a dedicated module
or delegated to the web-api layer. The controller binary should orchestrate, not
implement, database CRUD.

### MINOR: `uptrakit-web-api-types` dependency may be unnecessary

The `Cargo.toml` lists `uptrakit-web-api-types` but it may not be directly
imported in reviewed source files. If only needed transitively through
`uptrakit-web-api`, it should be removed as a direct dependency to reduce
confusion about actual dependency boundaries.
