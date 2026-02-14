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
  (`crl_manager.rs:45`) + DB `revocation_version`. Each instance polls every 60s
  (`crl_manager.rs:235`) and rebuilds CRL only when version changes; local
  revocations optimistically bump the version (`crl_manager.rs:264`).
- **Settings**: Atomic publish via `tokio::sync::watch`; cross-instance sync via
  DB `settings_version` counter polled every 30s (`main.rs:817`, `main.rs:857`).
- **Server cert renewal**: Only occurs when cert is within 30-day renewal window
  (`pki.rs:834`); file-based locking via PKI directory.

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
  - `main.rs:382` -- `.expect("valid default HTTPS addr")` on hardcoded
    `"[::]:8443"`
  - `main.rs:532` -- `.expect("already validated")` on URL already validated by
    CLI parser
- No raw SQL (all SeaORM)
- Error handling consistently uses `thiserror` + `rootcause` +
  `impl_report_conversion!`
- No secrets in logs

## Findings

**0 Critical, 0 High, 1 Medium, 2 Low**

### MEDIUM: `db/config.rs:65` -- safe but unclear `unwrap_or`

`.unwrap_or("unknown")` on `split("://").next()` is technically safe
(`split` always yields at least one element) but could use a constant or comment
for clarity.

```rust
url.split("://").next().unwrap_or("unknown")
```

### LOW: Hardcoded durations could be named constants

Multiple hardcoded durations are scattered across modules:

| Duration | Location | Value |
|---|---|---|
| CA expiry check | `pki.rs:823` | 183 days |
| Server cert renewal window | `pki.rs:834` | 30 days |
| Server cert validity | `pki.rs:750` | 90 days |
| CRL polling interval | `crl_manager.rs:235` | 60s |
| Settings polling interval | `main.rs:817`, `main.rs:857` | 30s |
| CA rotation check interval | `main.rs:938` | 24h |
| Shutdown task timeout | `main.rs:1254` | 5s |
| Auth cleanup interval | `main.rs:789` | 300s |

Extracting these to named constants would improve maintainability.

### LOW: `reconcile_setting_vec()` duplicates logic pattern

`reconcile_setting_vec()` in `main.rs:1339` duplicates the match-arm structure
from `reconcile::reconcile_setting()` in `reconcile.rs:47`. The `Vec<T>`
specialisation is necessary (empty vec = "not provided"), but consolidation into
`reconcile.rs` with a trait-based approach could reduce duplication.

## Extensibility Assessment

The controller is **not intended as a template** for external developers -- it is
the unique central server. However, several issues affect maintainability and
would need to be addressed before the codebase could support embedded or
alternative controller configurations.

### MAJOR: `main.rs` has a monolithic ~1,200-line `run()` function

The `run()` function handles master key initialization, directory resolution,
database setup, migrations, tenant loading, settings reconciliation, OIDC
bootstrap, PKI initialization, CA rotation setup, CRL management, server
certificate handling, JWT key migration, OIDC state stores, background task
spawning, server startup, signal handling, and graceful shutdown -- all in a
single function. This makes the controller extremely difficult for external
developers to understand or adapt.

**Recommendation:** Split `run()` into well-named initialization phases:
`init_master_key()`, `init_database()`, `init_settings()`, `init_pki()`,
`init_auth_stores()`, `spawn_background_tasks()`, `run_server()`.

### MINOR: Direct sea-orm entity operations for OIDC bootstrap in `main.rs`

Lines ~388-495 contain raw `ActiveModel` operations for OIDC provider
bootstrapping directly in `main.rs`. This logic should be in a dedicated module
or delegated to the web-api layer. The controller binary should orchestrate, not
implement, database CRUD.

### MINOR: `uptrakit-web-api-types` dependency may be unnecessary

The `Cargo.toml` lists `uptrakit-web-api-types` but it may not be directly
imported in reviewed source files. If only needed transitively through
`uptrakit-web-api`, it should be removed as a direct dependency to reduce
confusion about actual dependency boundaries.
