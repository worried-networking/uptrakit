# Audit Log Subsystem

Development guide for the semantic, mutation-first audit-log model (V2).

## Overview

Uptrakit audit logs are semantic events, not request transcripts. Each `AuditEntry` describes a
meaningful fact: a configuration was changed, an auth attempt was made, a workflow step
completed.

V2 introduces a hard classification of every registered action into one of two kinds.

**Stateful** — the action mutates a single persisted entity. The audit row captures a
`before_snapshot` and `after_snapshot` of that entity as JSON. The row is written inside the
same database transaction as the mutation, so it commits or rolls back atomically with the
change it describes.

**Event** — the action is a discrete workflow fact with no single-entity state transition.
Examples: auth outcomes, update triggers, workflow completions, delivery acts, system tasks.
Event entries carry no snapshot columns. They are emitted fire-and-forget through the async
dispatcher, consistent with V1.

The two kinds are enforced at compile time via a typestate builder. Passing a `Stateful` entry
to `emit_event` (or vice versa) is a compile error.

## The two emit paths

### `emit_event(entry)`

```rust
pub fn emit_event(&self, entry: AuditEntry<Event>);
```

Fire-and-forget through the async dispatcher. Failure logs at `error!` and is never propagated
to the caller. Use for all Event-class actions.

```rust
audit.emit_event(AuditEntry::auth_login()
    .actor_user(user_id, user_display)
    .outcome(AuditOutcome::Denied)
    .build()?);
```

### `emit_stateful(&tx, entry)`

```rust
pub async fn emit_stateful(
    &self,
    tx: &DatabaseTransaction,
    entry: AuditEntry<Stateful>,
) -> rootcause::Result<(), AuditLogError>;
```

Writes the audit row directly on the supplied transaction. If the INSERT fails, the transaction
rolls back along with the mutation it describes. Use for all Stateful-class actions.

**Transaction requirement:** the caller must open the transaction with `BEGIN IMMEDIATE`
whenever the handler reads any row before writing (the snapshot SELECT qualifies). Without
`BEGIN IMMEDIATE`, SQLite raises `SQLITE_BUSY_SNAPSHOT` (error code 5, bypasses
`busy_timeout`) when another writer commits between the snapshot read and the audit INSERT.
See the coding-standards database section for the full `begin_with_options` boilerplate.

**Journald post-commit flush:** the journald backend cannot write inside a
`DatabaseTransaction`. Obtain a `AuditCommitHook` before opening the transaction and call
`hook.flush_after_commit()` immediately after `tx.commit()` succeeds. If the transaction rolls
back or the caller returns an error, the hook is dropped without flushing — no journald entry
is emitted for a failed mutation.

```rust
let hook = audit.commit_hook();
audit.emit_stateful(&tx, AuditEntry::plugin_config_update(&before, &after)
    .actor_user(user_id, user_display)
    .outcome(AuditOutcome::Success)
    .build()?).await?;
tx.commit().await?;
hook.flush_after_commit().await;
```

## The `AuditView` derive macro

`AuditView` is a trait that projects a domain entity into a deterministic, secret-safe JSON
snapshot. It is implemented by applying the `#[derive(AuditView)]` macro to a SeaORM `Model`
struct.

```rust
pub trait AuditView {
    const TARGET_TYPE: &'static str;
    fn audit_target_id(&self) -> String;
    fn audit_target_display(&self) -> Option<String>;
    fn audit_view(&self) -> serde_json::Value;
}
```

The macro lives in `crates/shared/audit-log-derive/` and is re-exported through
`uptrakit-audit-log` — consumers depend on a single crate.

### Attributes

| Attribute                         | Scope             | Effect                                                                                                                                                                    |
| --------------------------------- | ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `#[audit(target_type = "...")]`   | Struct (required) | Sets `TARGET_TYPE`; becomes `target_type` on the audit row                                                                                                                |
| `#[audit(skip)]`                  | Field             | Excludes the field entirely; use for internal rowids and denormalized join columns                                                                                        |
| `#[audit(include)]`               | Field             | Overrides the auto-skip allowlist to include the field                                                                                                                    |
| `#[audit(project_with = "<fn>")]` | Field             | Calls `<fn>(&FieldType) -> serde_json::Value` instead of the default `Serialize` output                                                                                   |
| `#[audit(id_field = "...")]`      | Struct            | Overrides which field's `Display` becomes `audit_target_id` (default: `id`)                                                                                               |
| `#[audit(display_field = "...")]` | Struct            | Overrides which field becomes `audit_target_display` (default: `name` if present)                                                                                         |
| `#[audit(truncatable)]`           | Field             | Last-resort size cap: if the 16 KB snapshot cap is exceeded, this field is replaced with a sentinel before `.build()` fails; prefer `project_with` for known-large fields |

**Auto-skip allowlist:** the macro automatically skips fields named `created_at`, `updated_at`,
`deleted_at`, and `deactivated_at`. Use `#[audit(include)]` to override. Other domain
timestamps (`last_login_at`, `frozen_until`, `expires_at`) are included by default.

**Secret handling:** `EncryptedString` has no `Serialize` impl and is silently excluded at
compile time — it cannot reach `serde_json` at all. `MaskedUrl` and `MaskedEmail` implement
custom `Serialize` that emits the masked form, so they project safely without any attribute.
Do not add a plain `String` field for secrets; introduce a typed wrapper instead.

### Example

```rust
#[derive(AuditView)]
#[audit(target_type = "plugin_config")]
struct Model {
    id: Uuid,
    name: String,
    plugin_type: String,
    #[audit(project_with = "mask_config_secrets_str")]
    config_json: String,
    api_endpoint: MaskedUrl,        // self-masks via custom Serialize
    secret_value: EncryptedString,  // compile-excluded (no Serialize impl)
    #[audit(skip)]
    internal_rowid: i64,
    created_at: OffsetDateTime,     // auto-skipped
    updated_at: OffsetDateTime,     // auto-skipped
}
```

The macro emits a `#[cfg(test)] fn audit_view_determinism()` test per entity that calls
`audit_view()` twice and asserts byte-equal JSON output.

## Action-kind classification rule

When adding a new audited action, apply this rule:

**Stateful** — the action mutates one persisted entity. The handler can produce a meaningful
`before` view (read from the DB before the write) and an `after` view (read after the write or
derived from the applied changes). A snapshot pair makes sense. Examples:
`plugin_config.{create,update,delete}`, `user.update`, `service.approve`,
`service.update_freeze.{enable,disable}`, `notification_channel.*` mutations, settings updates.

**Event** — the action is a discrete workflow fact. No single-entity state transition.
Examples: all `auth.*`, all `*.triggered`, `*.started`, `*.finalized`, `*.completed`,
`*.callback`, `*.test`, all `system.scheduler.*`.

**Borderline guidance:**

- Multi-entity transforms (e.g. `service.merge` — two sources collapse into one target,
  many cross-table FKs rewritten): classify as **Event**. Use `details_json` to carry source
  IDs, target ID, and a summary of merged record counts.
- Certificate issuance (`service.certificate.{issue,renew}`): classify as **Event**. Each
  issuance is a point-in-time act; the certificate row is its own materialization. No
  meaningful "before" state exists.
- Batch facts (e.g. `software.update.triggered` batch): classify as **Event**. The individual
  per-item state transitions are each their own Stateful events.

## Catalog workflow

Every state-changing site in the codebase must have an entry in
`crates/shared/audit-log/audit-catalog.toml`. Each entry has either an `action` (mapped to a
registered action name) or a `skip` (free-text justification for why no audit is required).

### Entry shapes

```toml
[[entries]]
site = "uptrakit_web_api::routes::plugin_configs::create"
action = "plugin_config.create"

[[entries]]
site = "uptrakit_web_api::routes::host_status::record_heartbeat"
skip = "heartbeat denormalization; covered by transport access log, no security event"
```

Rules:

- `action` must match a registered action in `crates/shared/audit-log/src/action_type.rs`.
- `skip` requires a free-text justification that a future reviewer can evaluate.
- GET handlers, heartbeats, telemetry counters, connection bookkeeping, and internal cache
  writes are documented with `skip` so the static analyzer's positive assertion holds.

### Stale entries

If a handler is removed or renamed, its catalog entry becomes stale. The `audit-coverage-check`
tool detects this and fails CI with a "stale catalog entry" message. Update the catalog
alongside the handler change.

## The `audit-coverage-check` tool

The static-analysis gate lives at `crates/shared/audit-log/tools/audit-coverage-check/` and
runs in CI as:

```shell
cargo run -p audit-coverage-check
```

It runs in the same CI job as `cargo deny`. Failure is a build break.

### What it checks

**(a) Mutation-site coverage.** The tool walks every `.rs` file in the workspace using `syn`
and detects state-changing sites by AST shape: Axum route handlers reachable from a router
builder on HTTP verbs `POST`, `PUT`, `PATCH`, `DELETE`; wire-message handler arms that perform
DB writes; scheduler executor `run()` implementations; and functions annotated with
`#[audit_required]` (an escape hatch for sites the shape-based detection misses). Every
detected site must have a catalog entry. Missing entry → exits non-zero with the offending
site list.

**(b) Dead registry entries.** For each registered Stateful action, the tool performs a
token-level sweep looking for the action's typed constructor name (e.g.
`AuditEntry::plugin_config_update(`). A registered Stateful action with zero call sites fails
CI with a "dead registry entry" message.

Feature-gated handlers (`#[cfg(feature = "...")]`) are still required to have catalog entries
regardless of the building feature set. The walker treats the `cfg` attribute as metadata, not
as a reason to skip the site.

### How to interpret failures

- **"uncatalogued site"** — a mutation handler has no catalog entry. Add an `[[entries]]`
  block with either `action = "..."` or `skip = "..."`.
- **"dead registry entry"** — a registered Stateful action has no `emit_stateful(...)` call
  site. Either wire the emission at the appropriate handler or reclassify the action as Event
  if it was misclassified.

## `correlation_id` threading

`correlation_id` ties events from the same workflow together across requests, services, and
time. It is distinct from `request_id`, which correlates events within a single transport
request.

**When to mint:** at workflow entry points — HTTP handler, scheduler executor, OIDC flow
start. Use `Uuid::now_v7()`.

**When to thread:** downstream events in the same workflow inherit the correlation ID. For
example, a batch dispatch handler mints a correlation ID, stamps
`software.batch_update.triggered` with it, and passes it to each per-item trigger so
`software.update.started` and `software.update.finalized` events share the same ID.

**Single-step actions** leave `correlation_id` as `None`.

**How to use:**

```rust
let scoped = audit.with_correlation(correlation_id);
// All emits on `scoped` auto-stamp with correlation_id.
scoped.emit_event(AuditEntry::auth_login()
    .actor_user(user_id, user_display)
    .outcome(AuditOutcome::Success)
    .build()?);

scoped.emit_stateful(&tx, AuditEntry::plugin_config_update(&before, &after)
    .actor_user(user_id, user_display)
    .outcome(AuditOutcome::Success)
    .build()?).await?;
```

`scoped` is a cheap reference-counted handle; it does not allocate a new dispatcher.

## Test ergonomics

V2 splits the V1 `AuditEntry::test_stub(&str)` into two kind-specific helpers:

```rust
// Event-class test stub
AuditEntry::event_test_stub(action: &str) -> AuditEntry<Event>

// Stateful-class test stub
AuditEntry::stateful_test_stub(
    action: &str,
    target_type: &str,
    target_id: String,
    before: serde_json::Value,
    after: serde_json::Value,
) -> AuditEntry<Stateful>
```

**Secret-leak regression tests:** for every entity that embeds `EncryptedString`, `MaskedUrl`,
or `MaskedEmail`, assert that `audit_view()` output contains neither the plaintext value nor
the raw ciphertext. The auto-generated `audit_view_determinism` test covers field-order
stability; the secret-leak test is a separate, explicit assertion that must be written for
each entity.

Tests that use `emit_stateful` in a DB-backed test do not need `start_paused = true` unless
the test also calls `tokio::time::*` APIs.

## Don't

These patterns are banned in V2. Violating them is either a compile error or a CI failure.

- **No new `target: "security_audit"` tracing producers.** Use semantic emitters. The V1 CI
  check that fails on `target: "security_audit"` mutation logging remains active in V2.
- **No raw `action_type` string literals outside the registry, tests, fixtures, and
  migrations.** All production call sites use typed constructors generated by `audit_actions!`.
  Dynamic `AuditActionType` `FromStr` parsing is reserved for validated wire boundaries and
  test scaffolding only.
- **No service-supplied snapshots.** Snapshots are always sourced from the controller's
  authoritative database read. The wire ingress rejects any forwarded `AuditEventPayload`
  whose `action_type` resolves to `AuditActionKind::Stateful`.
- **No `emit_best_effort`.** Removed in V2. Every call site must use either `emit_stateful`
  or `emit_event`.

## Core crates

| Path                                                                | Purpose                                                                                                         |
| ------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| `crates/shared/audit-log/`                                          | `AuditActionType`, `AuditEntry<K>`, `AuditOutcome`, `AuditEmitter`, `RuntimeAuditEmitter`, dispatcher, backends |
| `crates/shared/audit-log-derive/`                                   | `AuditView` derive macro, `audit_actions!` proc-macro                                                           |
| `crates/shared/audit-log/audit-catalog.toml`                        | Action coverage catalog                                                                                         |
| `crates/shared/audit-log/tools/audit-coverage-check/`               | Static-analysis CI gate                                                                                         |
| `crates/shared/db/src/entity/audit_log.rs`                          | Tenant-scoped semantic rows                                                                                     |
| `crates/shared/db/src/entity/system_audit_log.rs`                   | System-scoped semantic rows                                                                                     |
| `crates/ui/web-api/src/routes/*`                                    | HTTP mutation producers                                                                                         |
| `crates/ui/web-api/src/routes/service_ws/handler/mod.rs`            | Service-forwarded audit event ingestion and scope validation                                                    |
| `crates/shared/scheduler-engine/src/executors/audit_log_cleanup.rs` | Retention cleanup and runtime audit emission                                                                    |

## Persistence model

`DatabaseBackend` routes by scope:

- `tenant_id = Some(...)` → `audit_logs`
- `tenant_id = None` → `system_audit_logs`

V2 columns (both tables, `system_audit_logs` omits `tenant_id`):

- `id`, `tenant_id`, `occurred_at`
- `actor_type`, `actor_id`, `actor_display`
- `action_type`, `action_kind` (`"stateful"` or `"event"`)
- `target_type`, `target_id`, `target_display`
- `outcome`
- `details_json` (optional JSON, 4 KB cap)
- `before_snapshot` (optional JSON, 16 KB cap; populated only when `action_kind = "stateful"`)
- `after_snapshot` (optional JSON, 16 KB cap; populated only when `action_kind = "stateful"`)
- `correlation_id` (UUID, nullable)
- `request_id`

`audit_logs.tenant_id` has no FK so audit history survives tenant deletion.

A `CHECK` constraint enforces that snapshot columns are populated for stateful rows and absent
for event rows. The typestate builder is the first line of defense; the constraint is the
second.

## CLI and backend configuration

Controller flags are unchanged from V1:

- `--audit-log-backend` (`db`, `journald`, `none`, repeatable)
- `--audit-log-db-url` (optional separate DB)
- `--audit-log-filter` (`all`, `mutations`, `none`)

`journald` backend emits structured events to `target: "uptrakit_audit"`.

## Read surface

Read APIs and CLI expose semantic filters:

- `actor_type`, `action_type`, `action_kind`, `outcome`, `target_type`, `target_id`
- `correlation_id`, `from`, `to`, `actor_id`, plus pagination

See:

- [Audit Logs API Reference](../api/audit-logs.md)
- [Audit Logs Security](../security/audit-logs.md)
- [Audit Logs End-User Guide](../end-user/audit-logs.md)

For the complete action catalog, see `crates/shared/audit-log/audit-catalog.toml`.
