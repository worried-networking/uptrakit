# Code Review: uptrakit-controller

## Review metadata

| Field | Value |
| --- | --- |
| Date | 2026-02-17 |
| Scope | Full crate review (`crates/core/controller/`) |
| Rating | **Excellent** |
| Reviewer | AI-assisted (Claude Code) |

## Executive summary

The controller crate is exceptionally well-engineered. No critical or high-severity
issues were found. The code demonstrates professional Rust development practices
with strong attention to safety, reliability, and HA correctness. All AGENTS.md
rules are satisfied.

## Findings

| # | Severity | Category | Location | Description | Suggested fix |
| --- | --- | --- | --- | --- | --- |
| C-4 | Info | HA | `tasks.rs:128` | Token denylist periodic purge is correctly invoked from `spawn_denylist_cleanup` on `AUTH_CLEANUP_INTERVAL`. No issue. | None needed. |
| C-5 | Info | Architecture | Overall | Excellent module separation, sophisticated startup phases, proper HA patterns with optimistic locking, graceful shutdown. | None needed. |

## Extensibility Review

No critical extensibility issues.

### Observation: does not directly depend on provider-registry

The controller depends on `uptrakit-web-api`, which in turn depends on
`uptrakit-provider-registry`. The controller itself does not import or use the registry directly.
Provider-related operations are delegated to the web-api layer. This is a clean separation.

### Extensibility positives

- **Well-structured startup phases** with 10 distinct phases and explicit intermediate types
  (`ReconciledSettings`, `ValidatedConfig`, `PkiRuntime`). Each phase has a clear input/output
  contract.
- **HA-safe scheduler** with optimistic locking via database claims and stale claim recovery.
  Multiple controller instances can run concurrently without duplicate task execution.
- **`TaskExecutor` trait** for scheduler task types -- new scheduled tasks can be added by
  implementing the trait and registering the executor.
- **Graceful shutdown** via signal handlers (`SIGTERM`, `SIGINT`, `SIGUSR1`).
- **SO_REUSEPORT support** for zero-downtime restarts.
- **Feature-gated database backends** (`db-sqlite`, `db-postgres`, `db-mysql`) propagated cleanly
  to `web-api` and `sea-orm`.
- **Master key verification** at startup ensures HA instances share the same encryption key.

## Strengths

- **Zero `unsafe`, zero `#[allow()]`, zero non-test `unwrap()`/`panic!()` / `.expect()`.**
  Production code paths never panic; all errors propagated with `rootcause`/`thiserror`.
- **CA rotation with optimistic locking** (`pki.rs:391-484`).
  Compare-and-swap guard on the active CA fingerprint prevents concurrent rotations
  across controller instances.
- **Scheduler task claiming with stale recovery** (`scheduler/claim.rs`).
  `WHERE locked_by IS NULL` prevents double-claims; 10-minute stale timeout
  (`STALE_CLAIM_SECONDS`) auto-recovers abandoned tasks.
- **Ordered graceful shutdown** (`tasks.rs:57-107`).
  Stop accepting connections, scatter `ServerRestarting` notifications, cancel
  token-based tasks, abort non-token tasks, per-task timeout.
- **Cross-instance synchronization via version polling.**
  CA version (`tasks.rs:167-233`), settings version (`tasks.rs:140-164`), and
  CRL version (`crl_manager.rs`) polling keep all instances consistent.
- **Encrypted secrets at rest.**
  CA private keys stored via `EncryptedString` (AES-256-GCM). OIDC client secrets
  encrypted in DB.
- **Comprehensive test coverage.**
  CLI parsing, proxy parsing, cert signing, scheduler claiming, CRL serial parsing,
  cron expressions, and settings reconciliation all have dedicated tests.

## AGENTS.md compliance checklist

| Rule | Status | Evidence |
| --- | --- | --- |
| No `unsafe` | Pass | No `unsafe` blocks found |
| No `unwrap()`/`panic!()` in production code | Pass | Only in `#[cfg(test)]` modules |
| No `#[allow()]` | Pass | No `#[allow()]` attributes found |
| No raw SQL | Pass | All DB access via SeaORM query builder |
| `FromStr` for string-to-type conversions | Pass | `cron_utils.rs`, CLI parsers |
| Typed error enums with `thiserror` | Pass | `AppError`, `PkiError`, `SchedulerError`, `DbError` |
| `rootcause` context propagation | Pass | `.context()`, `.context_to()`, `bail!()` throughout |
| `impl_report_conversion!` at boundaries | Pass | All error modules |
| Secrets never logged | Pass | `CaKeyStore::Debug` redacts keys; `Zeroizing` wrappers |
| Updates never automatic | Pass | Scheduler triggers version checks only |
| Agents outbound-only | Pass | Controller never initiates connections to agents |
