# Test Coverage: uptrakit-controller

> Generated: 2026-02-20 | Tool: cargo-llvm-cov 0.8.4 | Rust: 1.93.1

## Summary

| Metric | Value |
| -------- | ------- |
| Line coverage | 58.1% (2,811 / 4,840) |
| Function coverage | 60.1% (285 / 474) |
| Test count | 126 |

## Coverage by Module

| File | Line % | Lines | Function % | Functions |
| ------ | -------- | ------- | ------------ | ----------- |
| migration/mod.rs | 100.0% | 14/14 | 100.0% | 3/3 |
| scheduler/claim.rs | 100.0% | 338/338 | 100.0% | 34/34 |
| scheduler/cron_utils.rs | 100.0% | 76/76 | 100.0% | 13/13 |
| scheduler/executors/ca_rotation_check.rs | 100.0% | 98/98 | 100.0% | 11/11 |
| scheduler/executors/event_cleanup.rs | 100.0% | 49/49 | 100.0% | 7/7 |
| migration/m20260209_000001_initial.rs | 98.3% | 119/121 | 80.0% | 4/5 |
| cli.rs | 94.6% | 210/222 | 92.3% | 24/26 |
| scheduler/mod.rs | 93.2% | 261/280 | 92.3% | 24/26 |
| reconcile.rs | 92.9% | 197/212 | 85.0% | 17/20 |
| cert_signer.rs | 91.4% | 224/245 | 82.6% | 19/23 |
| pki.rs | 83.9% | 977/1,165 | 70.5% | 93/132 |
| migration/m20260215_000001_scheduled_tasks.rs | 70.0% | 7/10 | 60.0% | 3/5 |
| db/mod.rs | 69.0% | 20/29 | 80.0% | 4/5 |
| scheduler/executors/version_check.rs | 35.7% | 41/115 | 46.2% | 6/13 |
| crl_manager.rs | 35.1% | 119/339 | 37.1% | 13/35 |
| startup.rs | 7.8% | 61/783 | 15.4% | 10/65 |
| db/config.rs | 0.0% | 0/29 | 0.0% | 0/2 |
| main.rs | 0.0% | 0/301 | 0.0% | 0/13 |
| mtls_acceptor.rs | 0.0% | 0/39 | 0.0% | 0/6 |
| scheduler/executors/auth_cleanup.rs | 0.0% | 0/23 | 0.0% | 0/2 |
| scheduler/executors/service_cert_check.rs | 0.0% | 0/8 | 0.0% | 0/2 |
| scheduler/executors/stale_lease_cleanup.rs | 0.0% | 0/6 | 0.0% | 0/3 |
| server.rs | 0.0% | 0/66 | 0.0% | 0/6 |
| tasks.rs | 0.0% | 0/272 | 0.0% | 0/17 |

## Uncovered Critical Paths

### Tier 1 — Security-Critical

- **mTLS acceptor** (`mtls_acceptor.rs`, 0% coverage, 39 lines): Mutual TLS connection acceptance and client certificate extraction.
  Risk: mTLS misconfiguration could bypass client authentication.
- **PKI module gaps** (`pki.rs`, 83.9% coverage, 1,165 lines): While well-tested overall, 188 uncovered lines include CA key
  rotation edge cases, certificate chain building, and AIA/CDP extension generation. Risk: edge cases in PKI could produce
  invalid certificates.

### Tier 2 — Business-Logic

- **Remaining scheduler executors** (0% coverage for auth_cleanup, service_cert_check, stale_lease_cleanup): These executors
  still lack tests. The `version_check` executor is partially tested (35.7%), and `ca_rotation_check` and `event_cleanup`
  executors are now fully covered at 100%.
- **Version check remaining gaps** (`scheduler/executors/version_check.rs`, 35.7% coverage, 115 lines): While `merge_config`
  tests were added, 74 uncovered lines remain in version comparison and update notification logic.
- **Task management** (`tasks.rs`, 0% coverage, 272 lines): Async task spawning, cancellation, and lifecycle management for
  background operations. Risk: task leaks or failed cancellation could cause resource exhaustion.
- **Startup orchestration** (`startup.rs`, 7.8% coverage, 783 lines): Database initialization, master key setup, CA bootstrap,
  PKI initialization, scheduler start, and graceful shutdown coordination. Risk: startup failures could leave the system in a
  partially initialized state.
- **CRL manager remaining gaps** (`crl_manager.rs`, 35.1% coverage, 339 lines): While `sign_crl` tests were added (up from
  12.5%), 220 uncovered lines remain in CRL distribution, caching, and revocation list management.
- **Settings reconciliation gaps** (`reconcile.rs`, 92.9% coverage): 15 uncovered lines in edge cases for settings change detection
  and runtime reconfiguration.

### Tier 3 — Supporting

- **Server setup** (`server.rs`, 0% coverage, 66 lines): HTTP/HTTPS server binding and TLS configuration.
- **Database config** (`db/config.rs`, 0% coverage, 29 lines): Database connection URL construction.
- **Main entry point** (`main.rs`, 0% coverage, 301 lines): CLI argument parsing and service bootstrap.

## Test Recommendations

1. **Remaining scheduler executor tests** -- Test auth_cleanup, service_cert_check, and stale_lease_cleanup executors. Covers
   remaining `scheduler/executors/*.rs` (Tier 2). Use `MockDatabase` from SeaORM.
2. **Task lifecycle tests** -- Test task spawning, completion callbacks, cancellation, and concurrent task limits. Covers `tasks.rs`
   (Tier 2). Unit-testable with mock async tasks.
3. **Startup sequence integration test** -- Test database initialization, master key verification, and CA bootstrap with in-memory
   SQLite. Covers `startup.rs` (Tier 2). Requires careful mock setup for file system and database.
4. **mTLS acceptor test** -- Test client certificate extraction and validation from TLS connections. Covers `mtls_acceptor.rs`
   (Tier 1). Requires test certificates (reuse PKI test helpers).
5. **PKI edge case tests** -- Test CA key rotation, certificate chain building with multiple CA generations, and AIA/CDP extension
   validation. Covers uncovered lines in `pki.rs` (Tier 1). Extend existing PKI test suite.
6. **CRL manager tests** -- Test CRL distribution, caching, and revocation list management. Covers remaining gaps in
   `crl_manager.rs` (Tier 2). Extend existing `sign_crl` test suite.
7. **Version check executor tests** -- Test version comparison and update notification logic. Covers remaining gaps in
   `scheduler/executors/version_check.rs` (Tier 2). Extend existing `merge_config` tests.
