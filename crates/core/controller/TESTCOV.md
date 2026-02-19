# Test Coverage: uptrakit-controller

> Generated: 2026-02-19 | Tool: cargo-llvm-cov 0.8.4 | Rust: 1.93.1

## Summary

| Metric | Value |
| -------- | ------- |
| Line coverage | 51.2% (2,250 / 4,398) |
| Function coverage | 52.2% (228 / 437) |
| Test count | 106 (94 unit + 12 reverse proxy integration) |

## Coverage by Module

| File | Line % | Lines | Function % | Functions |
| ------ | -------- | ------- | ------------ | ----------- |
| scheduler/cron_utils.rs | 100.0% | 76/76 | 100.0% | 13/13 |
| migration/mod.rs | 100.0% | 14/14 | 100.0% | 3/3 |
| migration/m20260209_000001_initial.rs | 98.3% | 119/121 | 80.0% | 4/5 |
| cli.rs | 94.6% | 210/222 | 92.3% | 24/26 |
| reconcile.rs | 92.9% | 197/212 | 85.0% | 17/20 |
| scheduler/claim.rs | 92.6% | 313/338 | 94.1% | 32/34 |
| cert_signer.rs | 91.4% | 224/245 | 82.6% | 19/23 |
| pki.rs | 83.7% | 975/1,165 | 69.7% | 92/132 |
| migration/m20260215_000001_scheduled_tasks.rs | 70.0% | 7/10 | 60.0% | 3/5 |
| db/mod.rs | 69.0% | 20/29 | 80.0% | 4/5 |
| crl_manager.rs | 12.5% | 34/272 | 23.3% | 7/30 |
| startup.rs | 8.3% | 61/739 | 15.9% | 10/63 |
| db/config.rs | 0.0% | 0/32 | 0.0% | 0/2 |
| embedded_frontend.rs | 0.0% | 0/35 | 0.0% | 0/6 |
| main.rs | 0.0% | 0/301 | 0.0% | 0/13 |
| mtls_acceptor.rs | 0.0% | 0/39 | 0.0% | 0/6 |
| scheduler/executors/auth_cleanup.rs | 0.0% | 0/23 | 0.0% | 0/2 |
| scheduler/executors/ca_rotation_check.rs | 0.0% | 0/11 | 0.0% | 0/2 |
| scheduler/executors/event_cleanup.rs | 0.0% | 0/5 | 0.0% | 0/2 |
| scheduler/executors/service_cert_check.rs | 0.0% | 0/8 | 0.0% | 0/2 |
| scheduler/executors/stale_lease_cleanup.rs | 0.0% | 0/6 | 0.0% | 0/3 |
| scheduler/executors/version_check.rs | 0.0% | 0/84 | 0.0% | 0/8 |
| scheduler/mod.rs | 0.0% | 0/77 | 0.0% | 0/9 |
| server.rs | 0.0% | 0/62 | 0.0% | 0/6 |
| tasks.rs | 0.0% | 0/272 | 0.0% | 0/17 |

## Uncovered Critical Paths

### Tier 1 — Security-Critical

- **CRL manager** (`crl_manager.rs`, 12.5% coverage, 272 lines): Certificate Revocation List generation, signing, and distribution.
  Risk: untested CRL logic could fail to revoke compromised certificates, leaving them valid.
- **mTLS acceptor** (`mtls_acceptor.rs`, 0% coverage, 39 lines): Mutual TLS connection acceptance and client certificate extraction.
  Risk: mTLS misconfiguration could bypass client authentication.
- **PKI module gaps** (`pki.rs`, 83.7% coverage, 1,165 lines): While well-tested overall, 190 uncovered lines include CA key
  rotation edge cases, certificate chain building, and AIA/CDP extension generation. Risk: edge cases in PKI could produce
  invalid certificates.

### Tier 2 — Business-Logic

- **Scheduler run loop** (`scheduler/mod.rs`, 0% coverage, 77 lines): Main scheduler polling loop, task dispatch, and error
  recovery. Risk: scheduler failures could silently stop version checks.
- **All scheduler executors** (0% coverage across 6 files, 137 lines total): Version checking, CA rotation checks, certificate
  expiration checks, auth cleanup, event cleanup, and stale lease cleanup. Risk: untested executors could fail silently or
  produce incorrect results.
- **Task management** (`tasks.rs`, 0% coverage, 272 lines): Async task spawning, cancellation, and lifecycle management for
  background operations. Risk: task leaks or failed cancellation could cause resource exhaustion.
- **Startup orchestration** (`startup.rs`, 8.3% coverage, 739 lines): Database initialization, master key setup, CA bootstrap,
  PKI initialization, scheduler start, and graceful shutdown coordination. Risk: startup failures could leave the system in a
  partially initialized state.
- **Settings reconciliation gaps** (`reconcile.rs`, 92.9% coverage): 15 uncovered lines in edge cases for settings change detection
  and runtime reconfiguration.

### Tier 3 — Supporting

- **Server setup** (`server.rs`, 0% coverage, 62 lines): HTTP/HTTPS server binding and TLS configuration.
- **Database config** (`db/config.rs`, 0% coverage, 32 lines): Database connection URL construction.
- **Embedded frontend** (`embedded_frontend.rs`, 0% coverage, 35 lines): Static file serving from embedded binary.
- **Main entry point** (`main.rs`, 0% coverage, 301 lines): CLI argument parsing and service bootstrap.

## Test Recommendations

1. **CRL generation and signing tests** — Test CRL creation with revoked certificates, empty CRL, and CRL signing with CA key.
   Covers `crl_manager.rs` (Tier 1). Reuse PKI test infrastructure for CA key/cert setup.
2. **Scheduler executor unit tests** — Test each executor in isolation: version_check (mock provider responses),
   ca_rotation_check (mock CA state), service_cert_check (mock certificate store), auth_cleanup (mock expired sessions),
   event_cleanup (mock old events), stale_lease_cleanup (mock expired leases). Covers all `scheduler/executors/*.rs` (Tier 2).
3. **Scheduler run loop test** — Test the polling loop with mock executors, verifying task dispatch, claim, and error recovery.
   Covers `scheduler/mod.rs` (Tier 2). Use in-memory SQLite with seeded scheduled_task rows.
4. **Task lifecycle tests** — Test task spawning, completion callbacks, cancellation, and concurrent task limits. Covers `tasks.rs`
   (Tier 2). Unit-testable with mock async tasks.
5. **Startup sequence integration test** — Test database initialization, master key verification, and CA bootstrap with in-memory
   SQLite. Covers `startup.rs` (Tier 2). Requires careful mock setup for file system and database.
6. **mTLS acceptor test** — Test client certificate extraction and validation from TLS connections. Covers `mtls_acceptor.rs`
   (Tier 1). Requires test certificates (reuse PKI test helpers).
7. **PKI edge case tests** — Test CA key rotation, certificate chain building with multiple CA generations, and AIA/CDP extension
   validation. Covers uncovered lines in `pki.rs` (Tier 1). Extend existing PKI test suite.
