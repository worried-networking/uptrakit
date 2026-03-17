# Code Review: Rust Workspace and Database

- Review date: 2026-03-17
- Scope: full workspace review across 14 dimensions (DB design, security, HA/fault-tolerance,
  architecture, code quality, coding standards, idiomatic Rust, allocation, tests, consistency,
  extensibility, maintainability, crate structure, Sentrux metrics). Frontend excluded.
  `crates/core/integration-tests` and ignored integration suites excluded from findings.
- Validation:
  - `cargo check --no-default-features --features db-sqlite` -- passed
  - `cargo check --all-features` -- passed
  - `cargo clippy --all-targets --all-features -- -D warnings` -- passed
  - `cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings` -- passed
  - `cargo test --all-features --workspace --exclude uptrakit-integration-tests` -- passed
  - `markdownlint --config .markdownlint.json '**/CODEREVIEW.md'` -- passed

## Summary

The backend remains well-engineered overall. Clean acyclicity, strong coding-standards compliance,
solid security primitives, and the non-integration test sweep all hold. This review cycle
invalidated the previously reported fresh-install migration ordering defect (confirmed safe by
cross-referencing the index migration contents with the migrations test). The highest-priority
findings are now: (1) orphaned in-progress updates with no age-based cleanup, (2) the TOCTOU race
in discovery-pipeline upserts, (3) production `unreachable!()` panics in the enrolled WebSocket
handler, and (4) Telegram bot token stored in plaintext while equivalent SMTP credentials are
encrypted. Failure-recovery gaps and maintainability pressure from large files remain the most
actionable structural work.

## Strengths

- Zero dependency cycles. Sentrux reports 0 above-diagonal edges and acyclicity score 10000.
- Coding-standards compliance is comprehensive and clean. All required `#[non_exhaustive]`
  annotations, wire-safe `Other(String)` catch-all variants, `parking_lot` lock discipline,
  HTTP client timeouts, and SSRF protection are verified present with no violations.
- Security primitives are robust: AES-256-GCM envelope encryption, Argon2id password hashing,
  strict JWT validation (issuer + audience + required claims), OIDC email verification, and
  well-structured SSRF defense in all HTTP-making plugins.
- DB migration hygiene is strong. SQLite table-recreation helpers give DDL migrations explicit
  crash-recovery states. CAS-style update dispatch and transactional batch completion are clean.
- No `unwrap()`, `panic!()`, `todo!()`, or `unimplemented!()` in production code paths across the
  entire workspace. All such calls are confined to `#[cfg(test)]` modules.
- The workspace compiles, lints (both feature configurations), and tests cleanly against this
  revision.
- Rate limiting on all auth endpoints with fail-closed local fallback when the DB store is
  unavailable.
- Wire protocol forward compatibility is comprehensive: `#[serde(other)]` on message enums,
  `Other(String)` on value enums, sequence validation, and protocol version stamping.

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
- Propagation cost: 43% -- a change to one file affects ~43% of the dependency graph on average.
  Bottleneck crates: `uptrakit-shared-types` (imported by 38+ crates), `uptrakit-web-api`.
- DSM: Clean layering (all dependencies flow downward), density 16%, 8 level breaks.
- Git stats (90-day): 2225 commits, 30 hotspots, single-author ratio 99.9%.

## Active Findings

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
  controller. If the agent never reconnects cleanly, the row stays active forever. Queued updates
  behind it are never promoted. Software states MQTT feed shows `update_in_progress: true`
  indefinitely.
- See: `crates/shared/db/CODEREVIEW.md`, `crates/shared/scheduler-engine/CODEREVIEW.md`,
  `crates/ui/web-api-queries/CODEREVIEW.md`, `crates/core/CODEREVIEW.md`

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
- See: `crates/ui/web-api-queries/CODEREVIEW.md`

### [HIGH] `unreachable!()` panics in production WebSocket handler

- Dimension: fault tolerance, maintainability
- Scope: `crates/ui/web-api/src/routes/service_ws/handler/mod.rs:420`, `:444`, `:465`
- Why it matters: three `unreachable!()` macros guard message-to-capability dispatch. If a future
  message type is added without updating all dispatch arms, the controller panics on any enrolled
  agent connection that sends that message variant.
- Failure scenario: a new service message variant is added to the wire protocol, the capability
  dispatch router is not updated, and the first enrolled agent that sends that message terminates
  the WebSocket handler with an unrecoverable panic.
- See: `crates/ui/web-api/CODEREVIEW.md`

### [HIGH] Telegram global bot_token is stored in plaintext

- Dimension: security
- Scope: `crates/plugins/notifications/telegram/src/extensions.rs:handle_save_global_telegram`
- Why it matters: the Telegram bot token grants full control of the bot (send messages, read
  updates, manage webhooks). The equivalent SMTP password in the email plugin is encrypted at rest
  with AAD via `uptrakit_crypto::encrypt_str`. The Telegram token is stored as a plain JSON string
  in `global_settings`.
- Failure scenario: database backup leak, SQL injection read, or shared-hosting DB access exposes
  the bot token in plaintext. An attacker can impersonate the bot, read callback data, or pivot to
  further attacks via the Telegram Bot API.
- See: `crates/plugins/CODEREVIEW.md`

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

### [MEDIUM] SSH private keys stored as plaintext TEXT in the database

- Dimension: security
- Scope: `crates/shared/db/src/migration/m20260331_000001_ssh_agent_tables.rs:41`
- Why it matters: the `ssh_hosts.private_key` column is defined as `TEXT NOT NULL` with no
  encryption. Other sensitive fields (e.g., `notification_channels.config`) use `EncryptedString`
  for at-rest encryption. SSH private keys are high-value credentials.
- Failure scenario: database backup leak or SQL injection exposes all SSH private keys in
  plaintext, enabling lateral movement to every managed SSH host.
- See: `crates/shared/db/CODEREVIEW.md`

### [MEDIUM] `register_column_aad_mappings` is duplicated between controller and scheduler

- Dimension: maintainability, security
- Scope: `crates/core/controller/src/main.rs`, `crates/core/controller/src/reencrypt.rs`,
  `crates/core/controller/src/pki.rs`, `crates/core/scheduler/src/handler.rs`
- Why it matters: the controller and scheduler each maintain their own copy of the AAD column
  registration calls. If a new encrypted column is added and only one site is updated, decryption
  or re-encryption will use the wrong AAD context, producing silent data corruption.
- See: `crates/core/CODEREVIEW.md`

### [MEDIUM] Batch path always sets `interactive: false` without checking `config_prefers_interactive`

- Dimension: correctness, consistency
- Scope: `crates/ui/web-api-queries/src/queries/update_batches/mod.rs:create_batch` (line 198)
- Why it matters: PHS targets with `prefer_interactive: true` will fail in batch mode because the
  agent does not allocate a PTY. The single-update path correctly resolves the flag.
- Failure scenario: a "host update all" batch on a Proxmox host with PHS-discovered packages
  dispatches without `interactive: true`, causing PHS update scripts to fail.
- See: `crates/ui/web-api-queries/CODEREVIEW.md`

### [MEDIUM] Incomplete multi-page `SoftwareStates` buffers in MQTT are never garbage-collected

- Dimension: fault tolerance, memory
- Scope: `crates/core/mqtt/src/tenant_manager.rs`, partial_states buffer
- Why it matters: multi-page payloads buffered per client have no TTL or cleanup pass. Repeated
  broker churn generates orphaned entries that accumulate indefinitely.
- See: `crates/core/mqtt/CODEREVIEW.md`

### [MEDIUM] CRL number counter uses `Ordering::Relaxed` in multi-controller HA

- Dimension: high availability, security
- Scope: `crates/core/controller/src/crl_manager.rs:328`
- Why it matters: each controller maintains its own `AtomicU64` counter. CRL numbers can collide
  across controllers, causing relying parties to skip revocation updates.
- See: `crates/core/controller/CODEREVIEW.md`

### [MEDIUM] Email and Telegram plugins panic on non-object config JSON

- Dimension: fault tolerance, coding standards
- Scope: `crates/plugins/notifications/email/src/lib.rs:97`,
  `crates/plugins/notifications/telegram/src/lib.rs:267`
- Why it matters: `config.as_object_mut().expect()` in the notification delivery hot path violates
  the project's no-`unwrap()`-in-production rule.
- See: `crates/plugins/CODEREVIEW.md`

### [MEDIUM] `start_paused` rule violations in service WebSocket integration tests

- Dimension: test correctness
- Scope: `crates/ui/web-api/src/integration_tests/service_ws.rs` (4 test functions)
- Why it matters: four `#[tokio::test]` functions call `tokio::time::timeout()` without
  `start_paused = true`, violating the project rule.
- See: `crates/ui/web-api/CODEREVIEW.md`

### [MEDIUM] `uptrakit-backoff` adoption is inconsistent across outbound-request crates

- Dimension: consistency, HA/fault-tolerance
- Scope: CLI SSE streams, notification plugins, some release plugins
- Why it matters: the CLI's `--follow` SSE paths have no reconnect-with-backoff. A transient
  network blip during a long-running tail operation terminates the CLI unnecessarily.
- See: `crates/ui/cli/CROSS_CUTTING_FINDINGS.md`

### [MEDIUM] Public API surface and file size remain the main maintainability bottlenecks

- Dimension: architecture, maintainability
- Scope: `crates/ui/web-api/src/lib.rs`, `crates/core/agent-ssh/src/main.rs`,
  `crates/core/controller/src/pki.rs`, `crates/shared/extension-framework/src/lib.rs`,
  `crates/shared/wire/src/payloads.rs`
- Why it matters: Sentrux still flags large files and long functions as the dominant structural
  debt, and `uptrakit-web-api` still exports a very broad public module surface.

### [LOW] `uptrakit-shared-types` is still too broad for a high-fanout crate

- Dimension: extensibility, crate structure, maintainability
- Scope: `crates/shared/types/src/lib.rs`
- Why it matters: the crate still mixes plugin, discovery, MQTT, auth-adjacent, and update-state
  types behind one widely imported package, amplifying rebuild churn when unrelated concepts evolve.
- See: `crates/shared/types/CODEREVIEW.md`

### [LOW] `deliver_controller_event` and CLI `run` both exceed CC threshold

- Dimension: code quality, maintainability
- Scope: `crates/ui/web-api/src/event_delivery.rs:deliver_controller_event` (CC=37),
  `crates/ui/cli/src/main.rs:run` (CC=38)
- Why it matters: both functions are complex enough that adding new event types or commands under
  pressure risks accidental side-effect changes.

## Database-Wide Conclusions

- Schema evolution quality is materially good: migration helpers, repair migrations, and
  entity-level tests show careful attention to SQLite edge cases.
- The previously reported fresh-install migration ordering defect has been invalidated --
  `m20260302_000001_add_missing_indexes` only indexes tables from the initial migration, not from
  `m20260302_000002_host_packages`. The `migrations_run_on_empty_sqlite` test confirms this.
- The main remaining DB risk is operational cleanup of long-lived in-progress update state after
  multi-component failure, plus the SSH private key plaintext storage.
- Transaction boundaries are strongest in newer batch/update code. CAS-style dispatch is correct.
- The `find_or_create_software_item` TOCTOU is the highest-priority DB correctness fix.
- Migration vec ordering has diverged significantly from chronological file naming (15+ migrations
  out of date order), which increases the risk of human error during future migration authoring.

## Split/Merge Notes

- **Best split candidate**: `uptrakit-shared-types`. Plugin/discovery-specific types are the
  clearest extraction target, estimated to reduce fanout from 38 to ~25 for the residual crate.
- **Second split candidate**: `uptrakit-extension-framework` (1970 lines). Splitting into
  `extension-ui` (UI definitions) and `extension-wire` (network payloads) is trivial effort, high
  clarity gain.
- **Third candidate**: `crates/core/agent-ssh` bootstrap operations (4517 lines across four files)
  into a `bootstrap/` internal module; reduces main.rs cognitive load and is refactorable within
  one crate.
- **Merge candidate**: `uptrakit-backoff` (114 lines) and `uptrakit-config-merge` (201 lines)
  into a `uptrakit-shared-utils` umbrella; reduces workspace member count with no behavioral
  change.
- No other urgent merges recommended. Small utility crates have low maintenance cost.

## Resolved / Invalidated Findings

### [INVALIDATED] Potential migration ordering defect on fresh installs

- **Previously**: `m20260302_000001_add_missing_indexes` was suspected of referencing tables from
  `m20260302_000002_host_packages` based on vec ordering.
- **Resolution**: The index migration only references `update_history`, `host_software_items`,
  `mqtt_leases`, `service_hosts`, `sessions`, and `host_software_item_plugins` -- all created by
  the initial migration (`m20260209_000001_initial`). None of these tables come from
  `host_packages`. Confirmed by both direct code inspection and the `migrations_run_on_empty_sqlite`
  test. The vec ordering between these two specific migrations is safe. The broader migration vec
  ordering divergence is tracked as a [MEDIUM] maintainability concern in
  `crates/shared/db/CODEREVIEW.md`.

## CODEREVIEW.md Index

| Path | Scope | Findings |
|------|-------|----------|
| `CODEREVIEW.md` (this file) | Root executive summary | 4 HIGH, 13 MEDIUM, 2 LOW |
| `crates/core/CODEREVIEW.md` | Core umbrella | 1 HIGH, 6 MEDIUM, 1 INFO |
| `crates/core/controller/CODEREVIEW.md` | Controller binary | 1 HIGH, 3 MEDIUM, 1 LOW, 1 INFO |
| `crates/core/agent-ssh/CODEREVIEW.md` | SSH agent | 3 MEDIUM, 1 LOW |
| `crates/core/agent/CODEREVIEW.md` | Agent binary | 0 (clean) |
| `crates/core/mqtt/CODEREVIEW.md` | MQTT service | 2 MEDIUM, 1 LOW |
| `crates/core/scheduler/CODEREVIEW.md` | External scheduler | 1 HIGH, 1 INFO |
| `crates/shared/CODEREVIEW.md` | Shared umbrella | 1 MEDIUM, 1 LOW |
| `crates/shared/db/CODEREVIEW.md` | Database schema | 1 HIGH, 5 MEDIUM, 4 LOW |
| `crates/shared/crypto/CODEREVIEW.md` | Crypto library | 1 MEDIUM, 1 LOW |
| `crates/shared/wire/CODEREVIEW.md` | Wire protocol | 0 (clean) |
| `crates/shared/types/CODEREVIEW.md` | Shared types | 2 LOW |
| `crates/shared/web-api-types/CODEREVIEW.md` | Web API types | 0 (clean) |
| `crates/shared/agent-core/CODEREVIEW.md` | Agent core lib | 1 MEDIUM, 1 LOW |
| `crates/shared/command/CODEREVIEW.md` | Command execution | 0 (clean) |
| `crates/shared/nats/CODEREVIEW.md` | NATS client | 0 (clean) |
| `crates/shared/openapi-client/CODEREVIEW.md` | OpenAPI client | 0 (clean) |
| `crates/shared/service-sdk/CODEREVIEW.md` | Service SDK | 0 (clean) |
| `crates/shared/scheduler-engine/CODEREVIEW.md` | Scheduler engine | 1 HIGH, 1 MEDIUM, 2 LOW |
| `crates/ui/CODEREVIEW.md` | UI umbrella | 2 HIGH, 7 MEDIUM |
| `crates/ui/web-api/CODEREVIEW.md` | Web API | 1 HIGH, 8 MEDIUM, 1 LOW |
| `crates/ui/web-api-auth/CODEREVIEW.md` | Auth module | 1 MEDIUM, 1 LOW |
| `crates/ui/web-api-queries/CODEREVIEW.md` | Query layer | 2 HIGH, 5 MEDIUM, 2 LOW |
| `crates/ui/cli/CODEREVIEW.md` | CLI binary | 1 MEDIUM, 3 LOW |
| `crates/plugins/CODEREVIEW.md` | Plugins umbrella | 1 HIGH, 4 MEDIUM, 3 LOW |
| `crates/plugins/infrastructure/core/CODEREVIEW.md` | Plugin infra core | Per file |
| `crates/plugins/infrastructure/registry/CODEREVIEW.md` | Plugin registry | Per file |
| `crates/plugins/discovery/proxmox-helper-scripts/CODEREVIEW.md` | PHS discovery | Per file |
| `crates/plugins/package-managers/apt/CODEREVIEW.md` | APT plugin | Per file |
| `crates/plugins/package-managers/homebrew/CODEREVIEW.md` | Homebrew plugin | Per file |
| `crates/plugins/package-managers/npm/CODEREVIEW.md` | NPM plugin | Per file |
| `crates/plugins/releases/docker/CODEREVIEW.md` | Docker plugin | Per file |
| `crates/plugins/releases/github/CODEREVIEW.md` | GitHub plugin | Per file |
| `crates/plugins/releases/gitlab/CODEREVIEW.md` | GitLab plugin | Per file |
| `crates/plugins/releases/forgejo/CODEREVIEW.md` | Forgejo plugin | Per file |
