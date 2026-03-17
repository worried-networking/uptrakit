# Code Review: Rust Workspace and Database

- Review date: 2026-03-17
- Scope: full workspace review across 14 dimensions (DB design, security, HA/fault-tolerance,
  architecture, code quality, coding standards, idiomatic Rust, allocation, tests, consistency,
  extensibility, maintainability, crate structure, Sentrux metrics). Frontend excluded.
  `crates/core/integration-tests` and ignored integration suites excluded from findings.
- Validation:
  - `frontend: npm ci && npm run build` — passed
  - `cargo check --no-default-features --features db-sqlite` — passed
  - `cargo check --all-features` — passed
  - `cargo clippy --all-targets --all-features -- -D warnings` — passed
  - `cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings` — passed
  - `cargo test --all-features --workspace --exclude uptrakit-integration-tests` — passed

## Summary

The backend remains well-engineered overall. Clean acyclicity, strong coding-standards compliance,
solid security primitives, and the non-integration test sweep all hold. This review cycle surfaced
new findings in three areas: a potential fresh-install migration ordering defect, a TOCTOU race in
the discovery pipeline, and two production `unreachable!()` panics in the enrolled WebSocket
handler. Failure-recovery gaps and maintainability pressure from large files remain the most
actionable active work.

Older resolved findings were intentionally removed. Prior findings no longer reproducible include
the old unbounded notification queue and the old npm retry gap.

## Strengths

- Zero dependency cycles. Sentrux reports 0 above-diagonal edges and acyclicity score 10000.
- Coding-standards compliance is comprehensive and clean. All required `#[non_exhaustive]`
  annotations, wire-safe `Other(String)` catch-all variants, `parking_lot` lock discipline,
  HTTP client timeouts, and SSRF protection are verified present with no violations.
- Security primitives are robust: AES-256-GCM envelope encryption, Argon2id password hashing,
  strict JWT validation, and well-structured SSRF defense in all notification plugins.
- DB migration hygiene is strong. SQLite table-recreation helpers give DDL migrations explicit
  crash-recovery states. CAS-style update dispatch and transactional batch completion are clean.
- The workspace compiles, lints (both feature configurations), and tests cleanly against this
  revision.

## Sentrux Snapshot

- Quality signal: `7018`
- Bottleneck: `modularity`
- Root-cause scores:
  - acyclicity `10000`
  - redundancy `8670`
  - depth `6154`
  - equality `5665`
  - modularity `5631`
- Rule violations still present:
  - 2 functions exceed cyclomatic complexity 35 (`cli/src/main.rs:run` CC=38,
    `event_delivery.rs:deliver_controller_event` CC=37)
  - 58 functions exceed 150 lines
  - 93 files exceed 800 lines
- Test coverage: 74 test files for 1185 source files (~18.5% by Sentrux structural heuristic).
  Acceptable as a project tradeoff; noted as maintainability context only.
- Propagation cost: 43% — a change to one file affects ~43% of the dependency graph on average.
  Bottleneck crates: `uptrakit-shared-types` (imported by 38+ crates), `uptrakit-web-api`.

## Active Findings

### [HIGH] Potential migration ordering defect on fresh installs

- Dimension: database, high availability
- Scope: `crates/shared/db/src/migration/mod.rs`, migration vec ordering
- Why it matters: `m20260302_000001_add_missing_indexes` appears before
  `m20260302_000002_host_packages` in the migrations vec. SeaORM executes migrations in vec order.
  If the indexes reference tables created by `_host_packages`, fresh-install deployments will fail
  at startup when the DB is empty.
- Failure scenario: first-time deployment on a clean database. Existing deployments are unaffected
  because the tables already exist from prior migrations. Requires cross-checking against the
  `migrations_run_on_empty_sqlite` test in `migration/mod.rs`.
- See: `crates/shared/db/CODEREVIEW.md`

### [HIGH] TOCTOU race in `find_or_create_software_item`

- Dimension: database, correctness
- Scope: `crates/ui/web-api-queries/src/queries/autodiscovery/discovery_items.rs`
- Why it matters: the three-phase upsert races between Phase 2 (tenant-wide check) and Phase 3
  (insert). The unique-constraint collision recovery falls back to `(tenant_id, name)` lookup, which
  can return the wrong item when two concurrent discovery targets for different plugin configs share
  the same software name.
- Failure scenario: two concurrent autodiscovery runs produce different targets for the same
  software name. One target's insert loses the race and the fallback silently reuses the winner's
  item, causing incorrect plugin config routing.
- See: `crates/shared/db/CODEREVIEW.md` and `crates/ui/web-api-queries/CODEREVIEW.md`

### [HIGH] `unreachable!()` panics in production WebSocket handler

- Dimension: fault tolerance, maintainability
- Scope: `crates/ui/web-api/src/routes/service_ws/handler/mod.rs`, capability dispatch arms
- Why it matters: three `unreachable!()` macros guard message-to-capability dispatch. If a future
  message type is added without updating all dispatch arms, the controller panics on any enrolled
  agent connection that sends that message variant.
- Failure scenario: a new service message variant is added to the wire protocol, the capability
  dispatch router is not updated, and the first enrolled agent that sends that message terminates
  the WebSocket handler with an unrecoverable panic.
- See: `crates/ui/web-api/CODEREVIEW.md`

### [HIGH] No generic stale-update recovery for orphaned `update_history` rows

- Dimension: high availability, fault tolerance, database
- Scope: `crates/shared/scheduler-engine/src/executors/mod.rs`,
  `crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs`,
  `crates/ui/web-api/src/routes/service_ws/handler/updates.rs`
- Why it matters: the system recovers `InProgress` rows when the same agent reconnects, but there
  is still no scheduler executor or age-based cleanup for updates orphaned by broader failure modes.
  The partial unique host lock then blocks future work on that host indefinitely.
- Failure scenario: controller crash, agent crash, host loss, DB failover, or a dead network
  partition occurs after an update is marked `InProgress` but before a terminal result reaches the
  controller. If the agent never reconnects cleanly, the row stays active forever.

### [MEDIUM] Several control-plane paths still prefer silent drop over durable convergence under backpressure

- Dimension: high availability, consistency
- Scope: `crates/ui/web-api/src/notifications/dispatcher.rs`,
  `crates/core/mqtt/src/mqtt_client.rs`, `crates/core/mqtt/src/tenant_manager.rs`,
  `crates/ui/web-api/src/service_connections.rs`
- Why it matters: bounded channels protect memory correctly, but the choice under overflow is still
  `try_send()` plus warning-only logging. Under burst load or downstream stalls, status changes,
  notification events, reconnect hints, and config fan-out can be lost with no automatic replay.
- Failure scenario: MQTT broker flap, slow controller task, or a burst of update completions fills
  the in-memory channel. The system stays alive but operators observe stale status or missing
  notifications.

### [MEDIUM] `notifications/dispatcher.rs` has no timeout on `recv()` and unmonitored spawned tasks

- Dimension: fault tolerance, observability
- Scope: `crates/ui/web-api/src/notifications/dispatcher.rs:dispatch_loop`
- Why it matters: if all event producers crash and drop their sender ends, `dispatch_loop` blocks
  indefinitely on `rx.recv().await`. Spawned delivery tasks have no supervisor; a panic leaves the
  notification log entry permanently `pending`.
- Failure scenario: event producer crashes silently; notification delivery stalls without any
  alertable signal. Stale pending log entries accumulate.

### [MEDIUM] Encryption AAD lookup falls back to empty string on unregistered columns

- Dimension: security, correctness
- Scope: `crates/shared/crypto/src/encrypted_string.rs`, `TryGetable` impl
- Why it matters: when a column name is not registered in the AAD registry, decryption proceeds
  with empty AAD instead of the correct context-bound AAD. Decryption will fail, but the error is
  non-obvious and the data becomes operationally unrecoverable.
- Failure scenario: a plugin crate encrypts a field but omits its AAD registration; the column is
  unreadable until the registration is restored.
- See: `crates/shared/crypto/CODEREVIEW.md`

### [MEDIUM] OIDC state store does not distinguish expired tokens from never-existed tokens

- Dimension: security
- Scope: `crates/ui/web-api-auth/src/auth/oidc_state.rs:OidcFlowStore::take`
- Why it matters: an expired CSRF state token returns `None` (same as an unknown token). Callers
  cannot provide a user-facing "session expired, please try again" message, and any implicit trust
  on the `None` path could mask replay attempts.
- Failure scenario: an attacker captures a state token and presents it after expiry. The handler
  returns a generic "not found" rather than distinguishing the expiry case.
- See: `crates/ui/web-api-auth/CODEREVIEW.md`

### [MEDIUM] Public API surface and file size remain the main maintainability bottlenecks

- Dimension: architecture, maintainability
- Scope: `crates/ui/web-api/src/lib.rs`, `crates/core/agent-ssh/src/main.rs`,
  `crates/core/controller/src/pki.rs`, `crates/shared/extension-framework/src/lib.rs`,
  `crates/shared/wire/src/payloads.rs`
- Why it matters: Sentrux still flags large files and long functions as the dominant structural
  debt, and `uptrakit-web-api` still exports a very broad public module surface.
- Failure scenario: a future resilience change spans multiple large files; review and regression
  risk rise because too much behavior is co-located and too much internal API is visible externally.

### [MEDIUM] `start_paused` rule violations in service WebSocket integration tests

- Dimension: test correctness
- Scope: `crates/ui/web-api/src/integration_tests/service_ws.rs` (4 test functions)
- Why it matters: four `#[tokio::test]` functions call `tokio::time::timeout()` without
  `start_paused = true`, violating the project rule. Tests using any tokio time API must declare
  `start_paused = true`.
- Failure scenario: tests flake non-deterministically on slow CI runners or loaded machines.
- See: `crates/ui/web-api/CODEREVIEW.md`

### [LOW] `uptrakit-shared-types` is still too broad for a high-fanout crate

- Dimension: extensibility, crate structure, maintainability
- Scope: `crates/shared/types/src/lib.rs`
- Why it matters: the crate still mixes plugin, discovery, MQTT, auth-adjacent, and update-state
  types behind one widely imported package, amplifying rebuild churn when unrelated concepts evolve.
- Failure scenario: adding a plugin-only or transport-only type forces rebuilds and review
  touchpoints across much of the workspace even though most crates do not need that concept.

### [LOW] `deliver_controller_event` and CLI `run` both exceed CC threshold

- Dimension: code quality, maintainability
- Scope: `crates/ui/web-api/src/event_delivery.rs:deliver_controller_event` (CC=37),
  `crates/ui/cli/src/main.rs:run` (CC=38)
- Why it matters: both functions are complex enough that adding new event types or commands under
  pressure risks accidental side-effect changes.
- See: `crates/ui/web-api/CODEREVIEW.md` and `crates/ui/cli/CODEREVIEW.md`

## Database-Wide Conclusions

- Schema evolution quality is materially good: migration helpers, repair migrations, and
  entity-level tests show careful attention to SQLite edge cases.
- The main remaining DB risk is not migration correctness in the steady state; it is (a) fresh-
  install migration ordering and (b) operational cleanup of long-lived in-progress update state
  after multi-component failure.
- Transaction boundaries are strongest in newer batch/update code. CAS-style dispatch is correct.
- The `find_or_create_software_item` TOCTOU is the highest-priority DB correctness fix.

## Split/Merge Notes

- **Best split candidate**: `uptrakit-shared-types`. Plugin/discovery-specific types are the
  clearest extraction target, estimated to reduce fanout from 38 to ~25 for the residual crate.
- **Second split candidate**: `uptrakit-extension-framework`. Splitting into `extension-ui` (UI
  definitions) and `extension-wire` (network payloads) is trivial effort, high clarity gain.
- **Third candidate**: `crates/core/agent-ssh` bootstrap operations (4517 lines across four files)
  into a `bootstrap/` internal module; reduces main.rs cognitive load and is refactorable within
  one crate.
- **Merge candidate**: `uptrakit-backoff` (114 lines) and `uptrakit-config-merge` (201 lines)
  into a `uptrakit-shared-utils` umbrella; reduces workspace member count with no behavioral
  change.
- No other urgent merges recommended. Small utility crates have low maintenance cost.
