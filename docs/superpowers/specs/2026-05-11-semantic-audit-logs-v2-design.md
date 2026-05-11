# Semantic Audit Logs V2 Design

## Goal

Extend the V1 semantic audit subsystem so every audited mutation captures the
entity state it changed, every workflow is traceable end-to-end, and coverage is
enforced rather than aspirational.

V2 is the data-model release. Analytics, search, and visualization land in V3.

## Relationship to V1

The V1 spec (`docs/superpowers/specs/2026-04-17-semantic-audit-logs-design.md`)
remains authoritative for everything V2 does not explicitly modify. V2 extends
the canonical `AuditEntry` contract, the storage schema, the emitter API, and
the wire payload. V2 keeps every V1 design principle and adds two new ones.

## Scope

### V2 in scope

- Per-entity state capture via `before_snapshot` and `after_snapshot` JSON
  columns on both audit tables.
- A typed `AuditActionKind::{Stateful, Event}` classification on every
  registered action, enforced by a typestate builder so producers cannot emit a
  stateful action without a snapshot pair (or an event action with one).
- An `AuditView` derive macro that projects domain entities into snapshot JSON
  with secret-safe defaults.
- A flat `correlation_id` column tying multi-step workflows together (batch
  dispatch, OIDC flow chain, scheduler-spawned event chains).
- A catalog file plus a static-analysis CI gate that fails the build when a new
  state-changing mutation site lands without an explicit catalog decision.
- A two-path emitter API: `emit_stateful` writes the audit row inside the same
  database transaction as the mutation; `emit_event` keeps V1's async
  fire-and-forget dispatcher path.
- Minimal UI/CLI/API surface to render the new fields and filter by
  `correlation_id`.

### V3 (deferred)

Explicitly out of scope for V2 and called out in every V2 design and
implementation document:

- Analytics dashboards, time-series aggregation, and search beyond the
  existing list endpoint filter set.
- Workflow timeline view (grouped by `correlation_id`). Note: cross-table
  correlation queries (events spanning `audit_logs` and `system_audit_logs`
  under the same correlation) require API-layer fan-out or a unified
  index — assess at V3 design time.
- Per-entity audit history view ("show every audit row that touched this
  plugin_config").
- Per-action-kind retention or per-action retention policy.
- Parent/child event linking (event-DAG navigation).
- Compliance-driven retention extensions (legal-hold, immutable archive).
- Agent-side stateful audit emission for agent-authoritative entities (e.g.
  locally-managed certificate stores, agent plugin state). V2 routes all
  stateful audit through the controller; if a future architecture
  introduces agent-authoritative persisted state, V3 will revisit the
  trust-boundary rule that today rejects forwarded Stateful action types.

### Explicitly not addressed by V2

- Automatic transport-layer interception (middleware/SeaORM-hook auto-emit).
  Rejected; the catalog plus CI gate is the substitute.
- Service-side stateful audit emission. Snapshots are always sourced from the
  controller's authoritative database state. The wire ingress rejects
  service-forwarded stateful action types.

## Design principles (additions to V1)

V2 keeps every V1 principle (semantic over transport, one pipeline,
mutation-first, emit where meaning is known, safe details only, no
backward-compatibility burden). V2 adds:

- **Evidence integrity over delivery convenience**: for stateful actions,
  the audit row commits or rolls back with the mutation it describes. A
  mutation whose audit cannot be captured does not happen.
- **Enforced coverage**: the absence of an audit emission at a
  state-changing site is a build failure, not a code-review observation.

V2 also reinforces one V1 coding-standard rule that becomes load-bearing:
every transaction that captures a snapshot pair is a read-then-write
transaction and must be opened with `begin_with_options(TransactionOptions
{ sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
..Default::default() })`. Without `BEGIN IMMEDIATE`, SQLite raises
`SQLITE_BUSY_SNAPSHOT` (error code 5, bypasses busy_timeout) when another
writer commits between the snapshot SELECT and the audit INSERT. The rule
exists in V1 coding standards already; V2 must not relax it.

## Data model

### `audit_logs` and `system_audit_logs` columns

Drop V1 rows. Create fresh V2 tables. V2 keeps every V1 column and adds three.

V2 tenant table columns:

- `id`
- `tenant_id`
- `occurred_at`
- `actor_type`
- `actor_id`
- `actor_display`
- `action_type`
- `action_kind` _(new — stored as `"stateful"` or `"event"` at the DB boundary)_
- `target_type`
- `target_id`
- `target_display`
- `outcome`
- `details_json`
- `before_snapshot` _(new — JSON, nullable; populated only when `action_kind = "stateful"`)_
- `after_snapshot` _(new — JSON, nullable; populated only when `action_kind = "stateful"`)_
- `correlation_id` _(new — UUID, nullable)_
- `request_id`

`system_audit_logs` uses the same shape minus `tenant_id` (V1 rule preserved).

The V1 compliance-oriented rule preserved: no foreign key from
`audit_logs.tenant_id` to `tenants`.

### `action_kind`

Stored as a string at the DB boundary for migration tolerance, validated
against the closed internal `AuditActionKind` enum on read. The enum is
intentionally closed (V1 closed-enum convention for `actor_type` and
`outcome`).

### Snapshot columns

`before_snapshot` and `after_snapshot` are stored as JSON (Postgres `JSONB`,
SQLite `TEXT` containing serialized JSON). Each is capped at 16 KB serialized
on the producer side; capture-time validation guarantees the column never
exceeds the cap. `details_json` keeps its V1 4 KB cap and retains its V1 role
(curated metadata, not bulk state).

Snapshots are required for `action_kind = "stateful"` and forbidden for
`action_kind = "event"`. The typestate builder enforces this at compile time;
the DB layer enforces it as a `CHECK` constraint (Postgres: `CHECK ((action_kind
= 'event' AND before_snapshot IS NULL AND after_snapshot IS NULL) OR
(action_kind = 'stateful' AND before_snapshot IS NOT NULL AND after_snapshot IS
NOT NULL))`; SQLite supports the same constraint). The constraint is the second
line of defense after the typestate builder.

### `correlation_id`

`correlation_id UUID NULL`. Producer mints with `Uuid::now_v7()` at workflow
entry (HTTP handler, scheduler executor, OIDC flow start). Downstream events
inherit. Single-step actions leave it null.

`correlation_id` is distinct from `request_id`. `request_id` correlates events
emitted during a single transport request (HTTP-time or WS-frame-time).
`correlation_id` correlates events across requests, services, and time — the
batch dispatch that fires a `software.batch_update.triggered` and the agent
that emits `software.update.finalized` hours later share `correlation_id`, not
`request_id`.

### Indexes

V1 indexes preserved. V2 adds:

Tenant table:

- `(tenant_id, correlation_id)`
- `(tenant_id, action_kind, occurred_at desc)` — supports
  "show me only stateful entries this week" without a sequential scan

System table:

- `(correlation_id)`
- `(action_kind, occurred_at desc)`

Snapshot columns are deliberately not indexed in V2. Querying snapshots by
field path is V3 analytics work.

### Migration

A single sea-orm migration drops the V1 audit tables and creates the V2
tables. Both engines run the same DDL. SQLite migrations and Postgres
migrations share one source.

Rollout posture matches V1:

- Coordinated controller cutover; not a rolling controller upgrade.
- All controller instances stop or drain before the migration runs.
- The migration drops V1 rows; no transformation.
- New controllers only start after migration completes.
- Service rollout may remain rolling because the wire payload change is
  additive.
- The migration uses no Postgres-only DDL. Snapshot columns use sea-orm's
  `ColumnDef::json_binary()` which maps to `JSONB` on Postgres and `TEXT` on
  SQLite. `CHECK` constraints work on both engines.

### Why drop V1 rows over an additive `ALTER TABLE`

V1 rolled out the same way (drop and recreate). V2's typestate builder requires
that every row has an `action_kind` classification; back-populating that on V1
rows is a guesswork exercise that undermines audit-trail integrity. Dropping
V1 rows keeps the V2 schema's guarantees deterministic from day zero. The
audit history loss is bounded — V1 has been in production for weeks, not
months — and accepted by the design owner.

## `AuditView` trait and derive macro

Each domain entity that may appear as a stateful audit target implements
`AuditView`. The trait projects the entity into a deterministic, secret-safe
JSON view that becomes the snapshot payload.

### Trait

```rust
pub trait AuditView {
    /// Stable string identifying the entity kind, e.g. "plugin_config".
    const TARGET_TYPE: &'static str;

    /// Stable identifier of the entity instance. Becomes `target_id` on the
    /// audit row.
    fn audit_target_id(&self) -> String;

    /// Human-readable label. Becomes `target_display`.
    fn audit_target_display(&self) -> Option<String>;

    /// Deterministic JSON projection of the entity's audit-relevant fields.
    /// Field order is stable across invocations.
    fn audit_view(&self) -> serde_json::Value;
}
```

### Derive macro

Lives in a new dedicated crate `uptrakit-audit-log-derive` with `[lib]
proc-macro = true`. The existing `uptrakit-shared-macros` crate hosts
declarative `macro_rules!` macros (`impl_report_conversion!`,
`wire_safe_enum!`) and cannot be converted to a proc-macro crate — a
`proc-macro = true` crate cannot export non-macro public items, which would
break the declarative exports. The new derive crate is depended on by
`uptrakit-audit-log` and re-exported through the audit-log crate's public
API so consumers depend on a single crate.

The macro is opt-in per entity — domain crates apply it to the SeaORM
`Model` struct.

```rust
#[derive(AuditView)]
#[audit(target_type = "plugin_config")]
struct Model {
    id: Uuid,
    name: String,
    plugin_type: String,
    #[audit(project_with = "mask_config_secrets_str")]
    config_json: String,
    api_endpoint: MaskedUrl,             // self-masks via Serialize
    secret_value: EncryptedString,       // compile-skip (no Serialize impl)
    enabled: bool,

    #[audit(skip)]
    internal_rowid: i64,

    // record-metadata fields auto-skipped by name allowlist
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}
```

Macro semantics:

- `#[audit(target_type = "...")]` is required at struct level; this becomes
  `TARGET_TYPE`. The `id` field's `Display` impl becomes `audit_target_id`
  unless overridden with `#[audit(id_field = "...")]`. `name`, if present,
  becomes `audit_target_display` unless overridden with `#[audit(display_field
= "...")]`.
- Each non-skipped field is projected via `serde::Serialize`. Field order
  matches struct declaration order. To preserve insertion order on the
  serialized JSON, the V2 migration enables the `serde_json/preserve_order`
  feature at the workspace level (the workspace does not currently enable
  it). The derive macro emits projections through `serde_json::Map` which
  becomes an `IndexMap` under that feature.
- `#[audit(skip)]` excludes a field entirely. Used for noise (internal FK
  rowids, denormalized join columns).
- `#[audit(project_with = "<fn>")]` replaces the field's default `Serialize`
  output with a custom projector returning `serde_json::Value`. The projector
  must accept `&FieldType` and return `serde_json::Value`; the macro emits
  the call and lets `rustc` enforce the signature (proc-macro hygiene cannot
  type-check at expansion time, so an incorrect signature surfaces as a
  normal compile error in the generated code). Determinism of the projector
  is a documentation-only contract — the macro cannot detect a
  non-deterministic projector. Used for entities with user-controlled
  nested JSON, most notably `plugin_config.config_json` which must apply
  the existing `mask_config_secrets_str` helper.
- `#[audit(include)]` overrides the auto-skip allowlist. The default skip list
  is exactly `created_at`, `updated_at`, `deleted_at`, `deactivated_at`. These
  are skipped solely by field name, not by type. `last_login_at`,
  `frozen_until`, `expires_at`, and other domain-meaningful timestamps are
  included by default.
- Fields whose type does not implement `Serialize` are silently excluded with
  a compile-time note (cargo `--message-format=json` surfaces it; no warning
  noise in normal builds). This is the trust-the-type-system path:
  `EncryptedString` has no `Serialize` impl by design, so it cannot leak.
  `MaskedUrl` and `MaskedEmail` implement custom `Serialize` that emits the
  masked form, so they project safely.

### Why not `#[audit_mask]`?

The workspace already provides three masking primitives, each of which makes
its own safety guarantee at the type system level:

- `EncryptedString` — no `Serialize` impl. Cannot reach serde at all. Any
  attempt to include it in a `serde_json::to_value` call fails at compile time.
- `MaskedUrl` — custom `Serialize` impl that emits the masked form.
- `MaskedEmail` — same pattern as `MaskedUrl`.

Adding an `#[audit_mask]` attribute would reinvent what these types already
guarantee, with worse semantics (an attribute is opt-in and easy to forget;
the type system is opt-out and refuses to compile). The macro therefore
trusts the type system. New secret-bearing fields are added by introducing
new typed wrappers, not by tagging plain `String` fields.

### Determinism

`audit_view()` must be deterministic across invocations. The macro guarantees
this by:

- Using struct declaration order for field emission.
- Forbidding any logic that depends on system state (the macro generates pure
  field projections; custom projectors via `#[audit(project_with)]` must be
  documented as deterministic — convention, not enforcement).

A test in `uptrakit-audit-log` calls `audit_view()` twice on the same instance
and asserts byte-equal JSON output. The derive macro adds a generated test per
entity (`#[cfg(test)] fn audit_view_determinism()`).

### Opaque render, no schema version

Snapshots are historical evidence. A snapshot taken in 2026 stays valid
forever even if the entity gains, loses, or renames fields. The UI renders
snapshots as opaque key-value tables, not schema-bound forms. There is no
`schema_version` field on the snapshot envelope; readers tolerate any key set.

If a field is renamed in the entity, old snapshots keep the old key. The diff
view shows both keys side-by-side ("`old_name` was X, no `new_name` present"
on the before; "`new_name` is Y, no `old_name` present" on the after). This is
the truthful representation of the historical state and is acceptable.

## Action kind classification and typestate builder

### `AuditActionKind`

```rust
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum AuditActionKind {
    /// The action mutates the state of a persisted entity. Snapshot pair is
    /// required.
    Stateful,
    /// The action describes a discrete event with no entity transition.
    /// Snapshots are forbidden.
    Event,
}

impl AuditActionKind {
    pub const fn as_str(self) -> &'static str { /* ... */ }
}
```

The enum is intentionally closed. Adding a third kind is a deliberate contract
change.

### Registry classification

Every `RegisteredAuditAction` constant carries an `AuditActionKind`. The V1
registry shape is extended:

```rust
impl RegisteredAuditAction {
    pub const fn new(value: &'static str, kind: AuditActionKind) -> Self { /* ... */ }
    pub const fn kind(self) -> AuditActionKind { /* ... */ }
}
```

All V1 constants in `crates/shared/audit-log/src/action_type.rs` are migrated
to call `RegisteredAuditAction::new("...", AuditActionKind::Stateful)` or
`::Event` as appropriate. See "Initial catalog classification" below for the
full V1 sweep.

### Typestate builder

The V1 flat `AuditEntry` becomes a kind-parameterized type
`AuditEntry<K>`. The V1 `AuditEntryBuilder` is replaced by a
kind-parameterized builder. The phantom kind is carried through `.build()`
into the value type — there is no erasure point. `emit_stateful` then
accepts `AuditEntry<Stateful>` and `emit_event` accepts `AuditEntry<Event>`,
so kind enforcement holds end-to-end at compile time.

```rust
pub struct AuditEntry<K> {
    /* fields private */
    _kind: PhantomData<K>,
}

pub struct AuditEntryBuilder<K> {
    /* private */
    _kind: PhantomData<K>,
}

pub struct Stateful;
pub struct Event;
```

`Builder<Stateful>` exposes `.before(&impl AuditView)` and
`.after(&impl AuditView)`. `Builder<Event>` exposes neither. `.build()`
returns `Result<AuditEntry<K>, AuditLogError>`. The phantom `K` survives the
build step; `emit_stateful` takes `AuditEntry<Stateful>`, `emit_event` takes
`AuditEntry<Event>`. Passing the wrong kind to either is a compile error.

`Builder<Stateful>` requires both snapshots set before `.build()` is
callable (enforced via a separate `BuilderReady<Stateful>` marker on
intermediate types — `.before()` returns a builder with a `HasBefore`
marker, `.after()` returns `HasAfter`, and `.build()` is only implemented
for the fully-populated combination). Calling `.before()`/`.after()` on
`Builder<Event>` is a compile error.

### Constructors

V2 ships two distinct procedural macros, both in the new
`uptrakit-audit-log-derive` crate:

- `#[derive(AuditView)]` — applied to each domain entity `Model`. Generates
  the per-entity `AuditView` trait impl (projection, target_id, display,
  determinism test).
- A separate `audit_actions!` proc-macro (declarative wrapper around the
  registry constants) — generates per-action constructor methods on
  `AuditEntry`. The registry classifies actions; the same macro invocation
  emits the typed constructors. Constructors for Stateful actions take
  `(&impl AuditView, &impl AuditView)` arguments; Event-action constructors
  take none.

The two macros are distinct: `AuditView` derive is entity-side and lives
with the domain entity crate; `audit_actions!` is registry-side and lives
with the audit-log crate. They do not depend on each other.

For a `Stateful` action:

```rust
impl AuditEntry {
    pub fn plugin_config_update(
        before: &plugin_config::Model,
        after: &plugin_config::Model,
    ) -> AuditEntryBuilder<Stateful> {
        // generated: sets action_type, action_kind, before_snapshot,
        // after_snapshot, target_type, target_id, target_display
        // from the AuditView impl on plugin_config::Model.
    }
}
```

For an `Event` action:

```rust
impl AuditEntry {
    pub fn auth_login() -> AuditEntryBuilder<Event> {
        // generated: sets action_type, action_kind
    }
}
```

`audit_actions!` lives in `uptrakit-audit-log-derive` and is invoked from
`crates/shared/audit-log/src/action_type.rs` once per release of the
registry. Call sites become:

```rust
audit.emit_stateful(
    &tx,
    AuditEntry::plugin_config_update(&before, &after)
        .actor_user(user_id, user_display)
        .outcome(AuditOutcome::Success)
        .correlation_id(corr_id)
        .build()?,
).await?;

audit.emit_event(
    AuditEntry::auth_login()
        .actor_user(user_id, user_display)
        .outcome(AuditOutcome::Denied)
        .request_id_opt(req_id)
        .details(json!({"reason_code": "invalid_password"}))
        .build()?,
);
```

`.build()` performs runtime validation (size caps, UTC timestamp, valid
action-type, declared-kind matches the builder typestate as a belt-and-braces
check). The typestate prevents the most common misuse class; `.build()`
catches the rest.

### Why typestate over runtime validation alone

The codebase already prefers typed enums and typestate patterns over runtime
checks (standards-snapshot rule: "prefer typed enums or newtypes over raw
String mode flags"). A `Builder<Event>` that cannot syntactically be given a
snapshot is a stronger guarantee than a `build()` that returns
`Err("snapshot forbidden")`. The typestate is the cheaper guard; runtime
validation is the residual catch.

## Emitter API

### Two emit paths

The V1 `AuditEmitter::emit_best_effort` method is removed. V2 replaces it
with two distinct methods. Every V1 producer call site (~100 sites) is
migrated to one of these two methods as part of step 4 of the implementation
order. The transition is not source-compatible — `AuditEntry` becomes
generic over `K`, the builder API changes shape, and `emit_best_effort`
disappears. Step 4 must land as a single coordinated change (no green-CI
midway); the constructor naming convention is chosen so that nearly every
existing call site translates by mechanical rewrite.

The V1 `AuditEmitter` gains two methods replacing the V1 `emit`:

```rust
impl AuditEmitter {
    /// Stateful emission. Writes the audit row inside the supplied transaction.
    /// Failure rolls back the transaction along with the mutation.
    pub async fn emit_stateful(
        &self,
        tx: &DatabaseTransaction,
        entry: AuditEntry<Stateful>,
    ) -> rootcause::Result<(), AuditLogError>;

    /// Event emission. Fire-and-forget through the existing async dispatcher.
    /// Failure logs at error! and is never propagated to the caller.
    pub fn emit_event(&self, entry: AuditEntry<Event>);
}
```

Behavior:

- `emit_stateful` performs the INSERT into `audit_logs` (or `system_audit_logs`
  depending on tenant scope) directly on the supplied transaction. The
  caller is responsible for opening that transaction with
  `begin_with_options(TransactionOptions { sqlite_transaction_mode:
Some(SqliteTransactionMode::Immediate), ..Default::default() })` whenever
  the handler reads any row prior to mutation — this is the V1 coding
  standard for read-then-write transactions, and snapshot capture (SELECT
  before + mutation + audit INSERT) is exactly such a sequence.
- The journald multiplex for stateful events is novel infrastructure that
  V2 must build. sea-orm does not expose post-commit hooks on
  `DatabaseTransaction`. The implementation buffers the entry in a
  caller-supplied accumulator (e.g. `AuditCommitHook` — a small handle
  obtained from `AuditEmitter::commit_hook()`); the caller calls
  `hook.flush_after_commit()` immediately after `tx.commit().await?`
  succeeds. On `tx.rollback()` or any caller error before commit, the hook
  is dropped without flushing. The hook's `flush_after_commit` is
  fire-and-forget for journald failures (consistent with the rest of the
  journald path).
- `emit_event` enqueues onto the V1 async dispatcher unchanged. Both the DB
  backend and the journald backend handle the entry asynchronously.

### Scoped correlation

The emitter exposes a scoped view that auto-stamps every emit with the
supplied correlation ID:

```rust
let scoped = audit.with_correlation(correlation_id);
scoped.emit_event(...);
scoped.emit_stateful(&tx, ...).await?;
```

`scoped` is a cheap reference-counted handle around the same emitter; it does
not allocate a new dispatcher.

### Failure semantics

| Failure                                                             | Stateful behavior                                                                 | Event behavior                                 |
| ------------------------------------------------------------------- | --------------------------------------------------------------------------------- | ---------------------------------------------- |
| Producer-side snapshot serialization error                          | `.build()` returns `Err`; caller rolls back the transaction                       | N/A (no snapshots)                             |
| Producer-side size-cap exceeded after `#[audit(truncatable)]` strip | `.build()` returns `Err`; caller rolls back the transaction                       | N/A                                            |
| Before-snapshot SELECT error                                        | Caller already returned `Err` before reaching `.build()`; mutation never happened | N/A                                            |
| DB INSERT into audit table errors                                   | Transaction rolls back along with the mutation                                    | Dispatcher logs `error!`; main path unaffected |
| Journald write errors                                               | Logged at `error!`; DB row stands                                                 | Logged at `error!`; main path unaffected       |
| Catalog mismatch (action not classified)                            | Compile error via missing constructor                                             | Compile error via missing constructor          |

### `#[audit(truncatable)]` field attribute

Some entity fields are routinely large (notification rule body templates,
plugin config JSON containing comments, host description fields). The
`#[audit(truncatable)]` field attribute marks them as eligible for first-pass
truncation if the snapshot otherwise exceeds 16 KB.

Truncation strategy:

- On `.build()`, the projection is serialized and measured.
- If under 16 KB, accepted as-is.
- If over, fields marked `truncatable` are replaced with the sentinel object
  `{"truncated": true, "byte_count": <original>, "preview": "<first 256 bytes>"}`.
- The projection is re-serialized.
- If still over 16 KB, `.build()` returns `Err`; the caller rolls back.

The sentinel preserves enough context for an operator to identify what was
truncated. The 256-byte preview is a hard upper bound to keep the sentinel
itself well under the cap.

**Guidance**: `#[audit(truncatable)]` is a last-resort safety valve. A
field that routinely triggers truncation produces an unhelpful diff
(`{"truncated": true}` on both sides). The preferred path for known-large
fields is `#[audit(project_with = "<fn>")]` with a projector that emits a
deterministic, audit-relevant summary (e.g. a SHA-256 of the body plus the
line/byte count, or a structured key extraction). For `plugin_config
.config_json`, the projector applies `mask_config_secrets_str` which both
masks secrets and trims comments. New entities with potentially-large
fields should ship a `project_with` summary before relying on
`truncatable`.

### Determinism of snapshot envelope

The snapshot envelope at the column level is exactly the projection produced
by `AuditView::audit_view()`. There is no wrapper object, no
`schema_version`, no `target_type` duplication inside the JSON (those live on
the row columns). Two-byte savings per row across millions of rows matter; so
does the simpler reader code path.

## Catalog and static-analysis CI gate

### Goal

Every state-changing site in the codebase is either explicitly audited or
explicitly marked as not requiring audit, with a justification. Adding a new
state-changing site without updating the catalog fails CI.

### Catalog file

`crates/shared/audit-log/audit-catalog.toml`:

```toml
[[entries]]
site = "uptrakit_web_api::routes::plugin_configs::create"
action = "plugin_config.create"

[[entries]]
site = "uptrakit_web_api::routes::plugin_configs::update"
action = "plugin_config.update"

[[entries]]
site = "uptrakit_web_api::routes::host_status::record_heartbeat"
skip = "heartbeat denormalization; covered by transport access log, no security event"

[[entries]]
site = "uptrakit_controller_runtime::scheduler::audit_log_cleanup::run"
action = "system.scheduler.audit_log_cleanup"
```

Each entry has either an `action` or a `skip`. `action` must match a
registered action. `skip` requires a free-text justification.

### Static-analysis tool

A small Rust binary at `crates/shared/audit-log/tools/audit-coverage-check/`
implements the gate:

- Built on `syn` and `walkdir`; reads every `.rs` file under the workspace.
- Identifies state-changing sites by AST shape:
  - Axum route handlers reachable from a router builder, filtered to HTTP
    verbs `POST`, `PUT`, `PATCH`, `DELETE`.
  - Wire-message handler arms in service WS dispatch that perform DB writes.
  - Scheduler executor `run()` implementations.
  - Functions named `*_handler` or matched by attribute `#[audit_required]`
    (escape hatch for sites the AST shape misses).
- For each identified site, looks up the catalog. Missing entry → exits
  non-zero with the offending site list.
- For each catalog entry with `action`, asserts the named action is
  registered.
- For each catalog entry with `skip`, asserts the `site` exists in the
  codebase (catches stale catalog entries).
- `#[cfg(feature = "...")]`-gated handlers must have catalog entries
  regardless of the building feature set. The walker parses items even when
  the gating feature is not active in the current build, so a feature-only
  handler (e.g. `notifications-email`) is still required to have an
  `action` or `skip` decision. The walker treats the `#[cfg]` attribute as
  metadata, not as a reason to skip the site.

The tool runs in CI as a workspace check (`cargo run -p audit-coverage-check`)
in the same job that runs `cargo deny`. Failure is a build break.

### Why a Rust source walker over grep/regex

The codebase prefers typed analysis (standards-snapshot rule:
"prefer typed enums or newtypes over raw String mode flags"). A `syn`-based
walker handles attribute macros, conditional compilation, and the
function-renaming/relocation churn that grep cannot. The walker is ~600 lines
of pure Rust with no novel dependencies; the existing workspace already
depends on `syn` indirectly through several macro crates.

### Complementary assertion: every Stateful action has an emit site

The walker catches one failure mode — "this looks like a mutation site, do
you have a catalog entry?" — and is prone to false positives when write
logic moves into helper functions or `web-api-queries` modules outside the
walker's pattern set. A second, much simpler check covers the inverse
failure mode: every registered `Stateful` action in the registry must have
at least one `emit_stateful(...)` call site somewhere in the workspace.
The `audit-coverage-check` binary runs both checks. The second pass is a
token-level grep across the workspace looking for the action's typed
constructor name (e.g. `AuditEntry::plugin_config_update(`); registered
actions with zero call sites fail CI with a "dead registry entry" message.
The two checks together catch both "added a mutation but forgot to audit
it" (walker) and "registered an action but never emit it" (constructor
sweep).

### Out-of-scope sites (catalog-documented as `skip`)

Specific categories that the catalog must list with `skip` reasons:

- GET handlers and read paths (transport access log covers visibility).
- Heartbeats, ping/pong, telemetry counters.
- Connection lifecycle bookkeeping (WS open/close, reconnect, keepalive).
- Internal cache writes and denormalization side effects driven by observed
  state (e.g. `service_status_cache` rows).
- Schema migrations themselves.

These appear in the catalog so the static analyzer's positive assertion holds
("every site has a decision"); the `skip` text serves as the justification a
future reviewer needs.

### Compatibility with V1's `security_audit` guardrail

V1 introduced a CI check that fails on `target: "security_audit"` mutation
logging outside an allowlist. V2 keeps that check; both guardrails run side
by side. The V1 check enforces "no parallel audit channel"; the V2 check
enforces "no silent audit gaps."

## Wire protocol

### `AuditEventPayload` extension

The V1 `ServiceMessage::AuditEvent(AuditEventPayload)` gains one field:

```rust
pub struct AuditEventPayload {
    pub action_type: AuditActionType,
    pub tenant_id: Option<Uuid>,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub target_display: Option<String>,
    pub outcome: String,
    pub details_json: Option<serde_json::Value>,
    pub request_id: Option<String>,
    pub correlation_id: Option<Uuid>,  // V2 addition
}
```

The addition is purely additive. Old services keep emitting payloads without
`correlation_id`; the field arrives `None` on the controller side. New
services may emit `correlation_id` to tie service-originated events back to
controller-originated workflow IDs (e.g. an agent emitting
`software.update.finalized` with the `correlation_id` it received from the
trigger message).

### Stateful action rejection at ingress

Services emit only event-class actions today (the V1 spec lists exactly:
`service.workload.claim`, `service.workload.release`,
`software.update.started`, `software.update.finalized`,
`system.service.update_freeze.apply`, `system.service.machine_id.validate`,
`system.service.update_gate` — all event-class).

V2 codifies this by rejecting any forwarded `AuditEventPayload` whose
`action_type` resolves to `AuditActionKind::Stateful`. The controller logs a
warning with the service identity and drops the event. The service connection
is not disconnected (consistent with V1's "invalid forwarded events must be
dropped with a warning").

Snapshots for stateful entities are always sourced from the controller's
authoritative database read. If a service-originated workflow needs to
trigger a stateful audit on the controller side, the controller-side handler
of that workflow emits the stateful event itself.

### Why service-supplied snapshots are rejected

A service-supplied "before" state is a trust-boundary leak: a compromised
service could fabricate a plausible before/after pair that makes the audit
row look pristine while the underlying DB state shows otherwise. The
controller's authoritative read is the only source-of-truth for entity state
audit evidence. This is a hard rule, not a default.

## Product surface changes

### API

Existing endpoints keep their paths:

- `GET /api/v1/audit-logs`
- `GET /api/v1/system-audit-logs`

Response DTO additions (nullable):

- `before_snapshot: Option<serde_json::Value>`
- `after_snapshot: Option<serde_json::Value>`
- `correlation_id: Option<Uuid>`
- `action_kind: String` (`"stateful"` or `"event"`)

Filter additions:

- `?correlation_id=<uuid>`
- `?action_kind=stateful|event`

V1 filters preserved (`action_type`, `actor_type`, `outcome`, `target_type`,
`target_id`, `actor_id`, `from`, `to`).

### UI

The Dashboard Audit Logs page list view is unchanged. The detail drawer gains
a "State" tab.

State tab behavior:

- Hidden entirely for event-class entries.
- Renders two key-value tables side by side ("Before" / "After") from the
  snapshot JSON.
- Computes the field-level diff client-side. Keys present in only one column
  are highlighted (added/removed). Keys with different values are highlighted
  (changed). Unchanged keys are dim.
- Renders snapshot JSON values inline for primitives; nested objects render
  as collapsible sub-tables.

Filter bar gains a `correlation_id` UUID input field. Pasting a UUID from a
related row's detail view (a copy-button next to the correlation_id display)
filters the list to the full workflow.

### CLI

`cli audit-logs list` JSON output gains the four new fields (additive).
Human-readable output gains a "State changes" section for stateful entries
only, rendering a compact diff (changed keys only, before → after on each
line). Event entries are unchanged in human output.

`cli audit-logs show <id>` gains the same State tab rendering as the UI.

## Reliability and failure semantics

V1 reliability properties for `emit_event`:

- Fire-and-forget from the caller perspective.
- Backend failures must not fail the user operation.
- At-least-once delivery; retries may produce duplicate rows.

V2 adds for `emit_stateful`:

- Atomicity with the mutation. Audit row commit ↔ mutation commit.
- Exactly-once at the database level (single INSERT inside the same
  transaction).
- Capture failures roll back the transaction.

The journald multiplex remains at-least-once for both paths. UI, CLI, and
operators continue to treat audit rows as evidence, not as exactly-once facts
in the journald copy.

Missing producer coverage in V2 is no longer a product bug — it is a build
failure. The catalog gate makes silent gaps impossible.

### Retention

V2 keeps the V1 retention model unchanged: a single `audit_log.retention_days`
setting drives the scheduler cleanup executor
(`system.scheduler.audit_log_cleanup`). Per-action and per-action-kind
retention are V3 compliance work.

V2 documents the new storage characteristics in `docs/security/audit-logs.md`
so operators can size the retention setting deliberately given the snapshot
columns. The worst-case row size moves from ~1-2 KB (V1) to ~32 KB (V2) for
stateful entries; event entries remain V1-sized.

## Initial catalog classification

Every V1 registered action must carry an `AuditActionKind`. The V2 migration
work includes classifying all ~100 existing actions in one mechanical sweep.

Classification rules:

- **Stateful**: the action mutates one persisted entity. The handler can
  produce a `before` and `after` view of that entity. Examples:
  `plugin_config.{create,update,delete}`, `host.update`, `user.update`,
  `service.update_freeze.{enable,disable}` (mutates `service` row),
  `service.{approve,reject,deactivate}`, `notification_channel.*` mutations,
  `software_item.update`, settings updates, `tenant_setting.update`.
- **Event**: the action is a discrete fact with no single-entity transition.
  Examples: all `auth.*`, all `*.triggered`, `*.started`, `*.finalized`,
  `*.completed`, `*.callback`, `*.test`, `system.scheduler.*`,
  `notification_channel.test`, `service.merge` (consumes two entities,
  produces one — complex state transition expressed via `details_json` source
  and target IDs), `service.certificate.{issue,renew}` (point-in-time act;
  the certificate row is its own materialization, recorded via separate
  `service_certificate` row writes if those become stateful audit targets in
  the future), `software_item.merge`, `software.update.*` workflow facts.

### Borderline call-outs

These deserve explicit reasoning in the spec because the classification is
not obvious:

- **`service.approve` / `service.reject` → Stateful**. Both mutate the
  `service` row's status column. The before/after snapshot captures the
  status transition and any provisioning fields set during approval.
- **`service.merge` → Event**. Two source services collapse into one target
  service; the operation rewrites many cross-table FKs. There is no clean
  "before/after of one entity" — the target service has a meaningful
  before/after, but the operation as a whole is a multi-entity transformation
  better described by `details_json` carrying source IDs, target ID, and a
  summary of merged record counts.
- **`service.certificate.issue` / `service.certificate.renew` → Event**.
  Each issuance is a point-in-time act. The certificate row is the
  materialization. The audit row's `target` is the certificate; no
  before-state exists (a fresh certificate has no prior version on the same
  row).
- **`software.update.{triggered,started,finalized}` → Event**. Workflow
  facts. Together they form a chain tied by `correlation_id`. The actual
  state transitions on `update_history` rows are captured by separate
  stateful audit events (e.g. `update_history.update` if introduced) — these
  three actions remain workflow facts.
- **`host.update` → Stateful**. Mutates `host` row; snapshots show the
  before/after of host metadata.
- **`tenant.data.reset` → Event**. Destructive cross-table operation; cannot
  be cleanly snapshotted as "one entity before/after." `details_json` carries
  the reset scope and row-count summary.
- **`software_item.batch` → Event**. Batch operation across many software
  items; per-item state transitions are individual `software_item.update`
  Stateful events. The batch action itself is the workflow fact.
- **`notification.callback` → Event**. External webhook callback; no entity
  mutation on receipt (the channel's history table may receive a row, which
  is itself a separate stateful concern only if added to the catalog later).
- **`service_config.{store,delete}` → Stateful**. Both mutate the
  `service_config` row; snapshots show the stored payload metadata (with the
  encrypted blob naturally excluded via `EncryptedString`).
- **`service_config.deliver` → Event**. Delivery is a point-in-time act
  against an existing config row; the row itself is not changed by delivery.
- **`service.credentials.deliver` → Event**. Same reasoning as
  `service_config.deliver`.
- **`service.workload.{claim,release}` → Event**. Service-side workload
  tracking; the controller has no canonical "workload entity" with a stable
  before/after to project.
- **`surface_provider.register` → Event**. Surface providers register at
  startup; registry itself is not a persisted entity audited at the row
  level.
- **`surface_action.invoke` → Event**. Each invocation is a point-in-time
  act; the surface action itself is not a persisted entity that mutates.
- **`host_tag.{create,update,delete}` → Stateful**. Each mutates a single
  `host_tag` row. `host_tag.assign` is **Event** — it records a tag-to-host
  assignment fact and may produce rows in a join table that is itself not a
  stateful target (per the V1 spec's join-table tenant-isolation rule, the
  join row is keyed by both sides, so a "before/after of one entity" is
  meaningless).
- **`discovery_allowlist.{create,delete}` → Stateful**. Both mutate
  allowlist rows; snapshots show the pattern and scope.
- **`instance_plugin.toggled` → Stateful**. The instance-scoped-plugin enable
  flag lives in `global_settings`; snapshots capture before/after of the
  enabled state.
- **`instance_plugin.config_upserted` → Stateful**. Similar reasoning;
  snapshots capture the stored config row.
- **`software.update.{stdin_attention,interactive_control}` → Event**.
  Interactive update workflow signals; no entity mutation.
- **`software_item.enrich` → Event**. Enrichment results are recorded
  separately as workflow facts; the item itself is mutated via
  `software_item.update` if data changes.

The full classification table lives in
`crates/shared/audit-log/src/action_type.rs` alongside the constants. The
implementation plan walks each action through this rule set.

## Testing strategy

### Unit tests

- `AuditView` derive macro: byte-equal determinism test (auto-generated by
  the macro per entity).
- `AuditView` derive macro: secret-leak regression. For every entity that
  embeds `EncryptedString`, `MaskedUrl`, or `MaskedEmail`, the derived
  `audit_view()` output is asserted to contain neither the plaintext nor the
  raw `db_value` ciphertext.
- `AuditView` derive macro: nested-JSON projection. For `plugin_config`,
  asserts that `config_json` containing secrets is masked via
  `mask_config_secrets_str` (the `#[audit(project_with)]` path).
- `AuditActionKind`: round-trip `as_str()` / `from_str()`.
- Typestate builder: compile-fail tests (`trybuild` crate) for:
  - calling `.before()` on `Builder<Event>`.
  - calling `.build()` on `Builder<Stateful>` without both snapshots.
  - passing `AuditEntry<Event>` to `emit_stateful` (and vice versa).
  - applying `#[audit(truncatable)]` to a field whose type has no
    `Serialize` impl.
- Validator: 16 KB cap enforced for each snapshot column independently.
- Validator: combined row size with two near-cap snapshots (e.g. 15 KB + 15
  KB) succeeds and the resulting row stays under the documented ~32 KB
  worst-case ceiling.
- Validator: `action_kind` matches the registry's declared kind for the
  given `action_type`.

### Producer tests

For each V2 stateful action representative, an integration test asserts:

- An audited mutation in a single transaction writes exactly one audit row.
- The before/after snapshots match the entity state observed pre- and
  post-mutation.
- Rolling back the transaction (forced by a synthetic failure) drops both
  the mutation and the audit row.
- Rolling back also discards the queued journald commit hook (no journald
  entry emitted after a failed transaction).

For each V2 event action representative, an integration test asserts:

- An audited event writes exactly one audit row asynchronously.
- The audit row carries no snapshot columns.
- Backend failure does not fail the user operation.

Representative coverage includes at minimum:

- `plugin_config.update` (Stateful, common shape).
- `host.update` (Stateful, denormalized fields).
- `service.approve` (Stateful, status transition).
- `service.merge` (Event with rich details).
- `auth.login` (Event, pre-auth actor).
- `software.update.started` (Event, service-forwarded).
- `software.batch_update.triggered` (Event with `outcome = partial`).
- `system.scheduler.audit_log_cleanup` (Event, scheduler-emitted).

### Wire-ingress tests

- A service forwards a payload whose `action_type` resolves to
  `AuditActionKind::Stateful`. The controller drops the event with a warning
  and does not write an audit row. The service connection stays open.
- A service forwards a payload with `correlation_id`. The controller
  accepts it and writes the audit row with that correlation_id.

### Catalog gate tests

- The `audit-coverage-check` binary detects a new state-changing handler
  added in a fixture crate without a catalog entry and exits non-zero.
- Stale catalog entries (pointing at a removed handler) are detected and
  reported.
- The walker correctly handles `#[cfg(feature = "...")]`-gated handlers.
  Feature-gated handlers must have catalog entries regardless of the
  building feature set; the walker treats a `cfg`-out path as still
  requiring a catalog decision (either an `action` reference or a `skip`
  with a feature-gated justification).

### Migration tests

- The V2 migration drops V1 tables and creates V2 tables on both Postgres
  (testcontainers) and SQLite (in-memory).
- Fresh writes through `emit_stateful` and `emit_event` succeed against the
  migrated schema.
- The `CHECK` constraint rejects a row where `action_kind = "event"` and
  `before_snapshot IS NOT NULL`.

### Frontend tests

- State tab renders with both snapshots present (Stateful row).
- State tab is hidden for Event rows.
- Diff highlighting marks added/removed/changed keys correctly across a
  representative snapshot pair (driven by Playwright on a seeded test
  fixture).
- `correlation_id` filter input round-trips through the URL query string and
  filters the list.

### Test ergonomics

- The V1 `AuditEntry::test_stub(&str)` helper changes shape because
  `AuditEntry` is now generic over `K`. The V2 replacement is two stubs:
  `AuditEntry::event_test_stub(action: &str) -> AuditEntry<Event>` and
  `AuditEntry::stateful_test_stub(action: &str, target_type: &str,
target_id: String, before: serde_json::Value, after: serde_json::Value)
-> AuditEntry<Stateful>`. V1 test code that used the old `test_stub` is
  migrated to whichever stub matches the action kind. This is a test-only
  breaking change; production code does not use `test_stub`.
- The `tokio::time::pause`/`advance` discipline from V1 is preserved.
  Stateful capture tests do not need `start_paused = true` unless the test
  also uses `tokio::time::*`.

### Compile-time benchmark

The `audit_actions!` macro generates per-action constructor methods on
`AuditEntry` (one per registered action, ~100 entries). Each constructor
is a distinct expansion point; expansion volume increases compile time
for `uptrakit-audit-log` and its direct dependents. Step 5 of the
implementation order takes a baseline `cargo build` timing snapshot for
the audit-log crate plus one downstream crate (`uptrakit-web-api`) before
landing the macro and again after, recorded in the implementation plan
notes. A regression beyond 10% on either crate is a signal to consider a
runtime dispatch table for the constructor factory instead of per-action
generic methods. Not a CI gate — measurement is taken once during the
landing and revisited only if compile times become a complaint.

## Quality gates

Every V2 commit must pass:

```shell
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo run -p audit-coverage-check
cargo deny check
markdownlint --config .markdownlint.json '**/*.md'
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

Integration tests require Docker (Postgres testcontainer) and run under
`cargo test -p uptrakit-integration-tests -- --ignored`.

V2 inherits V1's lint posture. The workspace `warnings = "deny"` rule plus
the `unfulfilled_lint_expectations = "deny"` rule already escalate any
warning to a build error workspace-wide; adding a crate-level `[lints]`
table with `clippy::pedantic = "warn"` would also escalate pedantic findings
to errors and create churn on every Clippy upgrade. The standards snapshot
records the project's stance: pedantic findings are "review input, not
blindly applied style churn." V2 keeps that stance — the audit-log crate
uses the same lint configuration as every other shared crate, and authors
are encouraged (but not gated) to run
`cargo clippy -p uptrakit-audit-log --all-targets -- -W clippy::pedantic
-W clippy::nursery` locally as review input.

## Documentation deliverables

Every deliverable below is non-optional. The implementation plan tracks each
as a separate task.

### New documents

- **`docs/adr/0007-audit-stateful-transactional-emit.md`** — Captures the
  decision to write stateful audit rows inside the mutation transaction
  (synchronous) and to split actions into Stateful vs Event with typestate
  enforcement. ADR criteria pass: hard to reverse (every producer call site
  is reshaped), surprising without context (most audit systems are async),
  real trade-off (async simplicity vs evidence integrity, with evidence
  winning). The ADR must include: (a) the operator-observable difference
  between Stateful (committed-or-not-present) and Event (best-effort, may
  be delayed or missing on crash) reliability guarantees, and (b) a
  per-transaction latency budget (target: stateful audit INSERT adds
  <5 ms to P99 transaction duration on Postgres; SQLite is dominated by
  the existing `BEGIN IMMEDIATE` write lock). A benchmark in
  `crates/shared/audit-log/benches/` measures the actual regression and
  is referenced from the ADR.

### Rewritten or updated documents

- **`docs/development/audit-logs.md`** — Producer section rewritten to cover:
  the two emit paths (`emit_stateful` vs `emit_event`), the `AuditView`
  derive macro and its attributes, the action-kind classification rule, the
  catalog file workflow, the static-analysis tool usage, the
  `correlation_id` threading pattern. The V1 audited-action catalog section
  is replaced by a pointer to the new catalog file.
- **`docs/security/audit-logs.md`** — Operator-facing. Documents the V2
  evidence-integrity properties (atomic capture for Stateful; best-effort
  for Event), the transactional guarantee, and storage sizing math given
  the new snapshot columns. Includes a worked retention example: "At 100
  stateful mutations/day × 16 KB × 90 days = ~144 MB/tenant for stateful
  rows; bump `audit_log.retention_days` down or storage up." Also includes
  a section on the V1→V2 cutover that surfaces the data-loss caveat to
  operators and provides an optional pre-migration export step
  (`pg_dump` for Postgres or a one-line `sqlite3 .dump` for SQLite,
  filtered to the audit tables) so deployments with compliance posture
  can preserve V1 history out-of-band before running the migration.
- **`docs/end-user/audit-logs.md`** — Dashboard-facing. Adds: the State tab,
  the `correlation_id` filter, how to copy a correlation_id from one row to
  another. Updates the "what's excluded" section to note V3-deferred items
  (workflow timeline, per-entity history).
- **`docs/api/audit-logs.md`** — Response DTO updated with the four new
  fields and their nullability semantics. New filter documented.
  `action_kind` semantics explained.
- **`AGENTS.md`** — Audit subsystem summary updated. Adds: Stateful/Event
  split, transactional emit rule, catalog-as-source-of-truth, V3 deferred
  list.
- **`ARCHITECTURE.md`** — V2 flow updated. New diagram: HTTP handler →
  read-before (in tx) → mutation (in tx) → read-after (in tx) → typestate
  builder → `emit_stateful` (in tx) → commit → post-commit journald
  multiplex. Service-forwarded path unchanged from V1 (Event-only).
- **`CONTEXT.md`** — No glossary changes required. V2 does not introduce new
  domain terms; "audit log," "snapshot," "stateful action," and "event
  action" are implementation vocabulary, not domain vocabulary.

### Auto-memory note

The implementation plan must update the user's `CLAUDE.md` / project memory
when the V2 emitter is wired in, replacing the existing V1 audit notes with
V2 emit-path guidance.

## Implementation order

The implementation plan should sequence work as:

1. Enable the `serde_json/preserve_order` feature at the workspace level
   (one-line `Cargo.toml` change; required so the `AuditView` macro can
   guarantee deterministic field-order JSON projections).
2. Define `AuditActionKind` enum and extend `RegisteredAuditAction` to
   carry it. Classify all V1 actions in a single mechanical pass.
3. Create the new `uptrakit-audit-log-derive` crate (`proc-macro = true`).
   Implement the `AuditView` derive macro and the `audit_actions!`
   registry-side macro. Apply `AuditView` to a single pilot entity
   (`plugin_config::Model`) to validate the macro shape. Re-export
   `AuditView` through `uptrakit-audit-log` so consumers depend on a
   single crate.
4. Add the V2 schema columns (using `ColumnDef::json_binary()` for the
   snapshot columns) and `CHECK` constraints via a sea-orm migration. Drop
   V1 tables; create V2 tables.
5. Rebuild `AuditEntry<K>` and `AuditEntryBuilder<K>` types with the
   typestate carried through `.build()`. Remove
   `AuditEmitter::emit_best_effort`. Generate per-action constructor
   methods via `audit_actions!`. This step is a coordinated breaking
   change to every V1 producer call site; the constructor naming
   convention is chosen so most sites translate by mechanical rewrite, and
   the step lands as a single atomic commit (no green-CI midway). Test
   helpers also migrate (`test_stub` splits into `event_test_stub` and
   `stateful_test_stub`).
6. Implement `AuditEmitter::emit_stateful` (transactional path including
   the `AuditCommitHook` mechanism for deferred journald flush) and
   `AuditEmitter::emit_event` (the V1 fire-and-forget dispatcher path,
   renamed). Wire `with_correlation`.
7. Apply `AuditView` to every entity referenced by a Stateful action.
   Convert every Stateful producer to `emit_stateful(&tx, ...)`.
8. Add `correlation_id` threading at the workflow heads (batch dispatch
   handler, OIDC initiation, scheduler executor entry, service enrollment
   chain).
9. Extend `AuditEventPayload` wire with `correlation_id`. Add wire-ingress
   validation that rejects forwarded Stateful action types.
10. Implement `crates/shared/audit-log/tools/audit-coverage-check/`. Seed
    `audit-catalog.toml` from the existing catalog plus a sweep of all
    mutation sites. Wire into CI.
11. Frontend State tab, correlation_id filter, diff highlighting. CLI human
    output for stateful changes.
12. Documentation deliverables (every file listed above).

## Open implementation decisions

The implementation plan resolves these — they are not grilling-stage
decisions:

- Concrete `audit-catalog.toml` schema variant (flat list vs grouped by
  crate).
- Whether the catalog ships seeded with every existing route in V2 or
  ramps up in two sub-passes (HTTP routes first, then wire-message handlers
  and runtimes).
- Truncation sentinel exact shape and whether `preview` should be UTF-8 safe
  byte-clamped or rune-clamped.
- Exact `AuditCommitHook` API shape (caller-flush vs RAII drop-flush).
