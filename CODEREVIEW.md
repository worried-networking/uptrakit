# Code Review: Rust Workspace and Database

- Review date: 2026-03-17
- Scope: current-state review of the Rust workspace and database-related code only. `crates/core/integration-tests` and ignored integration suites were intentionally excluded from findings.
- Validation:
  - `frontend: npm ci && npm run build` (required by `embed-frontend`) - passed
  - `cargo check --no-default-features --features db-sqlite` - passed
  - `cargo check --all-features` - passed
  - `cargo clippy --all-targets --all-features -- -D warnings` - passed
  - `cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings` - passed
  - `cargo test --all-features --workspace --exclude uptrakit-integration-tests` - passed

## Summary

The backend remains well-engineered overall: the dependency graph is acyclic, the security posture around HTTP clients and secrets is materially stronger than in older reviews, and the compile, lint, and non-integration test sweep is clean. The active issues are now concentrated in failure recovery and long-term maintainability rather than in broad correctness regressions.

Older resolved findings were intentionally removed in this refresh. Examples that no longer reproduce include the old unbounded notification queue, the old npm retry gap, and the old generic-shell test gap.

## Strengths

- The workspace still has clean layering. Sentrux reports zero above-diagonal dependency edges and zero cyclicity score regressions.
- DB migration hygiene is strong. `crates/shared/db/src/migration/helpers.rs` gives SQLite table-recreation migrations explicit crash-recovery states instead of ad hoc DDL.
- Critical write paths increasingly use transactional or CAS-style guards, for example batch completion in `crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs`.
- HTTP client safety is broadly solid. The release and notification plugins use the shared client builder with SSRF protection and explicit timeouts.
- Validation discipline is strong: both `cargo check` modes, both clippy modes, and the non-integration workspace tests all pass on the reviewed revision.

## Sentrux Snapshot

- Quality signal: `7020`
- Bottleneck: `modularity`
- Root-cause scores:
  - acyclicity `10000`
  - redundancy `8651`
  - depth `6154`
  - equality `5685`
  - modularity `5632`
- Rule violations still present:
  - 2 functions exceed cyclomatic complexity 35
  - 56 functions exceed 150 lines
  - 93 files exceed 800 lines
- Coverage signal: 72 test files for 1169 source files (`~17.3%` by Sentrux's structural heuristic). This is acceptable as a project tradeoff, but still useful as maintainability context.
- Git signal: single-author ratio remains extremely high (`~0.999`), so bus factor remains a long-term maintainability risk even when code quality is otherwise good.

## Active Findings

### [HIGH] No generic stale-update recovery for orphaned `update_history` rows

- Dimension: high availability, fault tolerance, database
- Scope: `crates/shared/scheduler-engine/src/executors/mod.rs`, `crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs`, `crates/ui/web-api/src/routes/service_ws/handler/updates.rs`
- Why it matters: the system does recover `InProgress` rows when the same agent reconnects, but there is still no scheduler executor or other generic age-based cleanup for updates orphaned by broader failure modes. The partial unique host lock then blocks future work on that host indefinitely.
- Failure scenario: controller crash, agent crash, host loss, DB failover, or a dead network partition occurs after an update is marked `InProgress` but before a terminal result reaches the controller. If the agent never reconnects cleanly, the row stays active forever.

### [MEDIUM] Several control-plane paths still prefer silent drop over durable convergence under backpressure

- Dimension: high availability, consistency
- Scope: `crates/ui/web-api/src/notifications/dispatcher.rs`, `crates/core/mqtt/src/mqtt_client.rs`, `crates/core/mqtt/src/tenant_manager.rs`, `crates/ui/web-api/src/service_connections.rs`
- Why it matters: the code now bounds memory, which is correct, but it often does so with `try_send()` plus warning-only logging. Under burst load or downstream stalls, status changes, notification events, reconnect hints, and app-wide config fan-out can be lost rather than retried or reconciled.
- Failure scenario: MQTT broker flap, slow controller task, laggy NATS consumer, or a burst of update completions fills the in-memory channel. The system stays alive, but operators observe stale status or missing notifications with no automatic replay.

### [MEDIUM] Public API surface and file size remain the main maintainability bottlenecks

- Dimension: architecture, maintainability
- Scope: `crates/ui/web-api/src/lib.rs`, `crates/core/agent-ssh/src/main.rs`, `crates/core/controller/src/pki.rs`, `crates/shared/extension-framework/src/lib.rs`, `crates/shared/wire/src/payloads.rs`
- Why it matters: Sentrux still flags large files and long functions as the dominant structural debt, and `uptrakit-web-api` still exports a very broad public module surface. This makes internal refactors harder and raises the cost of changing serialization, routing, and startup logic safely.
- Failure scenario: a future resilience change spans multiple large files or public modules; review and regression risk rise because too much behavior is co-located and too much internal API is visible by default.

### [LOW] `uptrakit-shared-types` is still too broad for a high-fanout crate

- Dimension: extensibility, crate structure, maintainability
- Scope: `crates/shared/types/src/lib.rs`
- Why it matters: the crate still mixes plugin, discovery, MQTT, auth-adjacent, and update-state types behind one widely imported package. That keeps ownership fuzzy and amplifies rebuild churn when unrelated concepts evolve.
- Failure scenario: adding a plugin-only or transport-only type forces rebuilds and review touchpoints across much of the workspace even though most crates do not need that concept.

## Database-Wide Conclusions

- Schema evolution quality is materially good: the migration helpers, repair migrations, and entity-level tests show careful attention to SQLite edge cases and rolling schema repair.
- The main remaining DB risk is not migration correctness; it is operational cleanup of long-lived in-progress update state after multi-component failure.
- Transaction boundaries are strongest in the newer batch/update code than in the older monolithic migration files, which is the right direction for future work.

## Split/Merge Notes

- Best split candidate: `uptrakit-shared-types`. Plugin/discovery-specific types are the clearest extraction target.
- Second split candidate: `uptrakit-extension-framework` if it continues to grow as a single-file schema crate.
- No urgent merge is recommended. Small utility crates such as `backoff` and `build-info` have low maintenance cost and still provide clean ownership boundaries.
