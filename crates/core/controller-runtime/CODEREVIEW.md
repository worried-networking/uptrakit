# Code Review: `uptrakit-controller`

- Review date: 2026-03-17
- Scope: current-state review

## Summary

The controller remains the most operationally mature runtime crate in the workspace. Startup is clearly phased, master-key and PKI handling are much stronger than in older reviews, and the crate passed both clippy modes and the non-integration test sweep. The active risks are now failure-recovery completeness, long-file operational complexity, and a CRL numbering gap in multi-controller HA deployments.

## Strengths

- Startup is explicitly staged: master key, directories, database, key-ring, settings, reconciliation, and runtime services.
- PKI and migration handling have extensive unit coverage and are materially better than older review snapshots.
- The controller now benefits from cleaner crash-recovery and transactional behavior in the query layer than earlier versions did.
- Phased shutdown in `tasks.rs` is well-structured: stop HTTP, drain embedded services, scatter restart notifications, cancel token-based tasks, abort remainder, and await with per-task timeouts.
- `mtls_acceptor.rs` correctly separates mTLS and unauthenticated enrollment TLS contexts.

## Active Findings

### [HIGH] Embedded scheduling still lacks a generic stale-update cleanup executor

- Dimension: high availability, database
- Scope: controller-embedded scheduler path plus `update_history`
- Why it matters: the controller can clean up stale task claims, but not stale `InProgress` updates that survive wider failure combinations.
- Failure scenario: controller, DB, or network failure occurs after an update transitions to `InProgress`, and the originating agent never reconnects. The controller keeps the host locked indefinitely.

### [MEDIUM] Core controller logic is still concentrated in very large files and functions

- Dimension: maintainability, coding standards
- Scope: `crates/core/controller/src/main.rs` (~1100 lines), `crates/core/controller/src/pki.rs` (~900+ lines), `crates/core/controller/src/reencrypt.rs` (~924 lines), `crates/core/controller/src/crl_manager.rs` (~686 lines)
- Why it matters: the crate still carries monolithic operational code paths and an unannotated `#[allow(clippy::too_many_arguments)]` at `main.rs:667`.
- Failure scenario: a future HA or security change in startup or PKI logic has a larger review and regression surface than it should because too many responsibilities remain co-located.

### [MEDIUM] CRL number counter uses `Ordering::Relaxed` which can produce duplicate numbers in HA

- Dimension: high availability, security
- Scope: `crates/core/controller/src/crl_manager.rs:328`
- Why it matters: `crl_number` is an `AtomicU64` incremented with `fetch_add(1, Ordering::Relaxed)`. In a single-process context this is fine, but in a multi-controller HA deployment where each controller maintains its own counter, CRL numbers can collide. Relying parties that cache CRLs by number may skip a revocation update if two controllers issue the same CRL number.
- Failure scenario: controller A and controller B each issue CRL number 42. A relying party that has already cached CRL 42 from controller A ignores controller B's CRL 42, which contains a newer revocation entry.
- Fix: derive CRL numbers from a DB sequence or incorporate a controller-unique prefix.

### [MEDIUM] `reencrypt.rs` per-table upgrade helpers contain significant boilerplate duplication

- Dimension: maintainability, code quality
- Scope: `crates/core/controller/src/reencrypt.rs`
- Why it matters: the four per-table re-encryption upgrade functions (`upgrade_*`) follow an identical pagination + decrypt + re-encrypt + update pattern. Each function is ~150 lines with the same structure, differing only in entity type and column names.
- Failure scenario: a correctness fix (e.g., error handling around partial page failures or AAD changes) needs to be applied identically in four places. Missing one creates a silent encryption divergence for that table.
- Fix: extract a generic `upgrade_table<E>` helper parameterized by entity and column accessor.

### [LOW] Production journald initialization still uses `expect`

- Dimension: coding standards, resilience
- Scope: `crates/core/controller/src/main.rs:100`
- Why it matters: `tracing_journald::layer().expect("failed to connect to journald")` still panics a production code path instead of degrading gracefully. Containerized deployments without journald will crash on startup.

### [INFO] `register_column_aad_mappings` is duplicated between controller and scheduler

- Dimension: maintainability, security
- Scope: `crates/core/controller/src/main.rs`, `crates/core/controller/src/reencrypt.rs`, `crates/core/controller/src/pki.rs`, `crates/core/scheduler/src/handler.rs`
- Why it matters: the controller and scheduler each maintain their own copy of AAD column registrations. If a new encrypted column is added in only one site, the other will use wrong AAD context during decryption or re-encryption.
- Recommendation: extract a single shared `register_all_aad_mappings()` function.
