# Auto-Update Policy Foundation — Refactor Spec

- **Date:** 2026-05-10
- **Status:** Draft
- **Scope:** Refactor only. No feature behaviour shipped by this spec.

## 1. Motivation

The end-state goal is automatic, scattered installation of Updates across the fleet, controlled by
an operator-defined policy. Concretely, an Operator should be able to express:

1. Apply security Updates across all Hosts carrying a given tag, scattered within a weekly time
   window.
2. The same for non-security Updates.
3. Restrict an automatic policy to a specific Plugin source (e.g. only `apt`, only `cargo`).
4. Roll a specific Software Item out across its assigned Hosts on the same scattered schedule.
5. Mark a Host that requires a restart after an Update and notify the Operator.

This spec does **not** build that feature. It identifies and lands the codebase changes that make a
future implementation straightforward, low-risk, and aligned with current coding standards. The
feature itself will be specified separately once these refactors are merged.

## 2. End-state shape (recorded, not built)

The grilling session converged on a single `UpdatePolicy` entity covering items (1)–(4):

- **Selector** — host tags (`any-of`), optional `software_item_ids`, optional `plugin_type_ids`,
  optional `categories`; AND across axes.
- **Cadence** — recurring weekly window, expressed via a child `update_policy_window` table
  (`weekday`, `start_time`, `duration_seconds`, IANA timezone shared with parent).
- **Scatter** — per-evaluation random jitter inside the remaining window, with per-policy
  `max_concurrent` cap. A separate per-Host single-flight invariant (already enforced) prevents
  overlap with operator-triggered Updates.
- **Cadence variants** — `target_version: Option<String>` (`None` = "always latest",
  `Some(v)` = pinned rollout) and `terminate_on_completion: bool` (pinned rollout terminates after
  every targeted Host reaches the pinned version).
- **Failure handling** — no retries inside the same window. Across windows, track
  `consecutive_failures` per `(policy_id, host_id, software_item_id)`; on N consecutive failures
  (default 3), suppress that triple until manually cleared (or until an Operator-triggered Update
  for that item on that Host succeeds).
- **Executor** — new `TaskExecutor` registered via the existing `Scheduler::register(...)` extension
  point. Engine already polls and claims tasks safely across controllers.
- **New typed-enum variants** — the feature commit will add `ActorType::Policy` and
  `BatchType::Policy` (or, equivalently, a new `update_policy_run` entity that supplants the
  `update_batch` row for this case — that decision belongs to the feature spec). Both are deferred
  to the same commit that adds the real consumer; this refactor spec deliberately does not
  pre-shape them.

Item (5) — restart-required marker + Operator notification — is an **orthogonal track**. The
existing model already carries `update_history.awaiting_restart_since`,
`software_item.awaiting_restart_timeout`, and an `UpdateStatus::AwaitingRestart` variant that blocks
new dispatches. The remaining work (host-level marker, notification event) is out of scope here and
will be specified separately.

## 3. Verified-present (no work required)

| Capability                                        | Where                                                                                         | Evidence                                           |
| ------------------------------------------------- | --------------------------------------------------------------------------------------------- | -------------------------------------------------- |
| Per-Host single-flight on active Updates          | `update_history` unique index `uix_update_history_host_active` + `has_active_update_for_host` | `update_dispatch.rs`                               |
| Queueing fallback when Host busy                  | `trigger_update_for_host` inserts `Queued` row                                                | `update_triggers.rs::trigger_update_for_host`      |
| `AwaitingRestart` blocks new dispatch             | `has_active_update_for_host` counts the awaiting-restart row                                  | `test_has_active_update_includes_awaiting_restart` |
| Scheduler executor extensibility                  | `Scheduler::register(task_type: ScheduledTaskType, executor: Box<dyn TaskExecutor>)`          | `scheduler-runtime/src/scheduler.rs`               |
| Typed actor at controller-core boundary           | `ActorInfo { actor_type: ActorType, actor_id: String }`, `#[non_exhaustive]`                  | `controller-core/src/update/mod.rs`                |
| `UpdateDispatcher` trait                          | Single entry point for one-Host dispatch from any source                                      | `controller-core/src/update/mod.rs`                |
| `UpdateCategory` wire-safe enum                   | `Security` / `Bugfix` / `Feature` / `Unknown` / `Other(String)`                               | `shared/types/src/update_category.rs`              |
| Per-Host candidate discovery with category filter | `find_outdated_items_for_host`                                                                | `update_batches/candidates.rs`                     |
| Per-Item candidate discovery across Hosts         | `find_outdated_hosts_for_item`                                                                | `update_batches/candidates.rs`                     |

None of these need changes. The refactors below build on top of them.

## 4. In-scope refactors

Each refactor stands alone and lands as a separate small commit per
`docs/development/commit-messages.md`. Commits use Conventional Commits with scope.

Implementation order: **R1 must close before R4** (the typed-actor change ripples through every
write site, and R1's inventory is what proves the set of in-flight strings). R2 and R3 are
independent. If R1's inventory and R3's call-site sweep both touch the same scheduler-runtime
executor file (unlikely — audit-log cleanup doesn't call candidate queries today), sequence R1
first on those files to avoid merge churn.

### R1 — Extend `ActorType` enum and consolidate ad-hoc actor strings

**File:** `crates/ui/web-api-queries/src/queries/update_types.rs`

Today `ActorType` covers `User`, `ApiToken`, `Scheduler`. Grep across the workspace reveals
additional `actor_type` strings written to `update_history.actor_type` and
`update_batches.actor_type` that bypass the enum:

- `"service"` — used by service-WS update flows
- `"system_service"` — used by an unattended/system path in `update_history.rs` live code
- `"uptrakit-mqtt"` — used by the MQTT service triggering an Update; also surfaces via
  `service_app_name` in service-WS handlers
- `"system"` — written by `crates/core/scheduler-runtime/src/executors/audit_log_cleanup.rs` and
  possibly other scheduler-runtime executors. **Note:** the audit-log family (`system_audit_log`,
  `tenant_audit_log`) uses a separate `AuditActorType` enum and is **out of scope** — only
  `update_history.actor_type` and `update_batches.actor_type` writes are subject to this
  refactor. Both columns may receive `"system"` from scheduler executors that bypass the
  `CreateUpdateRecordParams` / `CreateBatchParams` path; verify during inventory.

Work:

1. **Inventory** every literal string written to `update_history.actor_type` or
   `update_batches.actor_type`. The search must cover at minimum:
   - `crates/ui/web-api/`
   - `crates/ui/web-api-queries/`
   - `crates/ui/mcp/`
   - `crates/ui/controller-core/`
   - `crates/core/scheduler-runtime/` (production executors)
   - Test fixtures may keep string literals if the resulting fixture is clearer; new tests prefer
     the enum.
2. **Add enum variants** for canonical actor types currently expressed as raw strings:
   - `Service` (`"service"`)
   - `SystemService` (`"system_service"`)
   - `Mqtt` (`"uptrakit-mqtt"` — keep `as_str()` returning `"uptrakit-mqtt"` for backwards
     compatibility with any reader filtering on the literal; document the legacy spelling in
     `coding-standards.md`)
   - `System` (`"system"`)

   Do **not** add `Policy` here. The future policy executor adds that variant in the same commit
   that introduces the executor. Pre-adding it churns every exhaustive `match` site for a variant
   with no consumer.

3. **Closed enum, no `Other(String)`**.
   - Keep `as_str(self) -> &'static str` returning the same on-disk strings (no DB migration).
   - Add `FromStr` returning `Err(ParseActorTypeError::Invalid)` on unknown strings (mirrors
     `UpdateCategory::FromStr`). Do **not** add `From<String>` and do **not** add an
     `Other(String)` variant — `actor_type` is not a wire type and the inventory closes the set.
     If a stray literal surfaces later, treat it as a bug, not a forward-compat case.
4. **Update audit-log read paths** that surface the raw `actor_type` string column so the UI/CLI
   continue to render correctly. If readers parse via the enum, ensure the new variants render.

**Acceptance:** every `actor_type` literal in production code paths writing
`update_history.actor_type` or `update_batches.actor_type` goes through `ActorType`. All quality
gates from §9 pass.

### R2 — `find_hosts_with_any_tag` helper

**File:** `crates/ui/web-api-queries/src/queries/host_tags.rs`

Today the module exposes CRUD for `host_tag` and `host_tag_assignment` plus a batch loader, but no
"find every Host with at least one of these tags" query. The future policy executor needs this on
the hot path.

Work:

1. Add `pub async fn find_hosts_with_any_tag(tenant_db: &TenantDb, tag_ids: &[Uuid])
-> Result<Vec<host::Model>, sea_orm::DbErr>` (matches the return shape used by other helpers in
   this module — `host::Model` is the canonical internal-query return type per the surrounding
   code; mapping to a UI response type is the caller's job). No `mode` parameter and no
   `TagMatchMode` enum: when `AllOf` semantics are needed in the future, add a separate function
   (`find_hosts_with_all_tags`) with its own signature. The two SeaORM query shapes do not share
   structure cleanly; encoding the choice in an enum saves nothing today and inflates the API.
2. Enforce tenant isolation through `TenantDb`. The **primary** isolation comes from starting the
   query as `tenant_db.find::<host::Entity>()` (or whichever `TenantDb` entry point this module
   already uses), which injects `host.tenant_id = ?` automatically. `host_tag_assignment` itself
   has **no** `tenant_id` column, so a second tenant filter on the parent `host_tag.tenant_id` is
   added as a belt-and-suspenders check (and to scope the join to tags owned by the current
   tenant). This mirrors how `load_host_tags_batch` scopes assignments today. Do **not** call
   `Host::find()` directly — that bypasses isolation.
3. Filter out deactivated hosts (`host.deactivated_at IS NULL`).
4. Empty `tag_ids` returns `Ok(vec![])` — never "all hosts". Document this in the body doc comment.
5. **Use SeaORM `EntityTrait::find()` + `.join(...)` + `.filter(...)` + `.distinct()`**, not raw
   SQL. The rest of `host_tags.rs` uses SeaORM exclusively; raw SQL would be inconsistent.
   Sketch:

   ```rust
   tenant_db
       .find::<host::Entity>()
       .join(JoinType::InnerJoin, host_tag_assignment::Relation::Host.def().rev())
       .join(JoinType::InnerJoin, host_tag_assignment::Relation::HostTag.def())
       .filter(host_tag::Column::TenantId.eq(tenant_db.tenant_id()))
       .filter(host_tag_assignment::Column::HostTagId.is_in(tag_ids.iter().copied()))
       .filter(host::Column::DeactivatedAt.is_null())
       .distinct()
       .all(tenant_db.db())
       .await
   ```

   The exact `TenantDb` accessor names and join-relation direction may differ — match the
   conventions already in this module.

6. Unit tests in the module: empty input, single tag, multiple tags, tag belonging to another
   tenant excluded, deactivated host excluded.
7. Doc comment includes a one-line N+1 advisory: callers enumerating outdated items per host
   should consider `find_outdated_hosts_for_item` when the item axis is known, to avoid running
   one candidate query per host.

**Acceptance:** function exists, tested, callable from a hypothetical future executor without
further refactor.

### R3 — Pluralize `categories` and add `plugin_type_ids` filter on candidate queries

**File:** `crates/ui/web-api-queries/src/queries/update_batches/candidates.rs`

The candidate-discovery helpers (`find_outdated_items_for_host`, `find_outdated_hosts_for_item`)
already accept a single `category_filter: Option<&str>`. The future policy executor wants both
plural `categories` (e.g. `{Security, Bugfix}`) and a plugin-source filter
(`{apt, cargo}`). Convert one filter shape and add the other in the same refactor for consistency.

Work:

1. Replace `category_filter: Option<&str>` with `categories: Option<&[UpdateCategory]>` in both
   helpers. Filter the `host_software_item.update_category` column via `IN (..)` of the
   `categories.iter().map(UpdateCategory::as_str)` slice.
2. Add `plugin_type_ids: Option<&[PluginTypeId]>` to both helpers. Filter the
   `host_software_item_plugin` join (`role = "execute_update"`) by `plugin_type IN (..)`.
   `PluginTypeId` is a newtype wrapping `Cow<'static, str>`
   (`crates/shared/types/src/plugin_type_id.rs`); compare against the DB string column via
   `plugin_type_id.as_str()`.
3. **Empty-slice semantics — fail fast, not silent.** Both `Some(&[])` cases (`categories` and
   `plugin_type_ids`) are caller bugs in production paths: `None` means "no filter on this axis"
   while `Some(&[])` would mean "match nothing on this axis." To prevent silent-policy bugs:
   - In each helper, on `Some(slice)` where `slice.is_empty()`:
     - `debug_assert!(!slice.is_empty(), "categories: empty slice is a caller bug; pass None to disable filter")`
       (substitute the actual parameter name — `"categories"` or `"plugin_type_ids"` — in each
       call) — explodes loudly in tests, no production cost.
     - `tracing::warn!` with the parameter name (production observability).
     - Return `Ok(vec![])` immediately (no DB query).
   - Document this contract in the helper's doc comment, including the **caller obligation**:
     callers must collapse post-filter empties to `None` _before_ invoking the helper, not pass
     `Some(&empty_vec)`. The `debug_assert` enforces this contract.
   - Until a future HTTP `Validate` impl lands (deferred to feature spec), this helper's
     debug-assert + warn-and-empty is the sole defence. That is intentional and correct: the
     helpers are safe standalone, and the feature spec will add the boundary `Validate` as the
     first line of defence once the HTTP surface exists.
4. Update existing call sites to pass `None` for both new shapes. Where current callers pass
   `Some("security")`, convert to `Some(&[UpdateCategory::Security])`.
5. Add at least one test per query per filter axis: positive match, negative match, empty-slice
   warn-and-empty.

**Acceptance:** both filter shapes work, current callers compile, new tests cover positive,
negative, and empty-slice paths. All quality gates from §9 pass.

### R4 — Typed `actor_type` in dispatch and batch params

**Files:**

- `crates/ui/web-api-queries/src/queries/update_dispatch.rs` — `CreateUpdateRecordParams`
- `crates/ui/web-api-queries/src/queries/update_triggers.rs` — `TriggerUpdateParams`
- `crates/ui/web-api-queries/src/queries/update_batches/mod.rs` — `CreateBatchParams` (also carries
  `pub actor_type: &'a str` at line ~76)

Change `pub actor_type: &'a str` to `pub actor_type: ActorType` in all three structs.

For `actor_id`: change it to `actor_id: String` (owned) in the same edit. The structs are
constructed once per dispatch and the borrow saves nothing meaningful — and in `TriggerUpdateParams`
the sibling `to_version` is already `String`. Owning `actor_id` lets the struct drop its `'a`
lifetime entirely, which is the idiomatic shape. If a call site has a borrowed string at
construction time, it `.to_string()`s once at the boundary.

Inside the implementation, `actor_type.as_str().to_string()` replaces the current
`actor_type.to_string()` at the SeaORM `Set(...)` call.

Call-site sweep:

- `update_triggers.rs::trigger_update_for_host` callers
- `update_batches/mod.rs` batch dispatch sites (including the `actor_type: params.actor_type` line
  at ~215)
- `controller-core/src/update/controller.rs` — translates `ActorInfo` into dispatch params
- `mcp/src/tools/update.rs` (already uses typed `ActorType` via `ActorInfo`)
- `crates/ui/web-api/src/actions/update_batches.rs`
- Any web-api route that fans into `TriggerUpdateParams`

Existing tests call `actor_type: ActorType::User.as_str()` — drop the `.as_str()` after the change.
Where literal `"user"` appears in tests, swap for `ActorType::User`.

**Acceptance:** no `actor_type: &str` field remains in the three in-scope params structs. No `'a`
lifetime parameter exists solely to support `actor_id: &'a str` in those structs. All quality gates
from §9 pass.

## 5. Out of scope (deferred)

These items are explicitly out of scope for this spec. A separate feature spec will cover them.

- `update_policy`, `update_policy_window`, `update_policy_suppression` schema and migrations
- `AutoUpdatePolicyExecutor` (the `TaskExecutor` implementation)
- New `ScheduledTaskType` variant for the policy task
- `ActorType::Policy` variant — lands in the same commit as the policy executor
- `BatchType::Policy` variant (or a replacement `update_policy_run` entity) — feature-spec decision
- Window evaluation logic, scatter algorithm, concurrency cap enforcement
- HTTP routes, request types, `Validate` impls, and OpenAPI surface for policy CRUD
- Dashboard surface (forms, list views, suppression management)
- Restart-required marker on `host` (today only on `update_history`)
- New `NotificationEventType` variants for restart-required / policy-rollout-complete
- An `AllOf` tag-match query helper (added when needed, with its own signature)
- Convergence of `ActorType` and `AuditActorType` (different tables, different write paths)
- **Pinned-version candidate query** — the future `target_version` axis from §2 needs a different
  semantic shape (`available_version == target AND current_version != target`) than the existing
  "is outdated?" candidate query. Expect a sibling helper (e.g. `find_pinned_targets_for_host`)
  rather than further widening `find_outdated_items_for_host`. Feature-spec decision.

## 6. Risks and open questions

### Inventory blast radius (R1)

Cleaning up ad-hoc `actor_type` strings may touch audit-log readers in the UI, CLI, MCP tools, or
Surfaces metadata. If the inventory reveals readers that match on specific literals, the safest
path is to **preserve those literals** in `ActorType::as_str()` and document the legacy values in
`docs/development/coding-standards.md`. The refactor's goal is type safety on the write path; we do
not need to renormalize on-disk values.

### Direct `Set(...)` writes outside params structs (R4)

R4 changes three params structs to use typed `ActorType`. Production code paths that write
`update_history.actor_type` or `update_batches.actor_type` should funnel through these structs. If
the inventory in R1 finds a direct `Set("...".to_string())` write on either column outside the
three structs (i.e. a code path that constructs the `ActiveModel` itself), treat it as a bug:
either route it through the typed entry point or open a follow-up to do so. Such sites are not
expected on the production write path, but R1's inventory is the proof.

### `Option<&[PluginTypeId]>` and `Option<&[UpdateCategory]>` ergonomics (R3)

A borrowed slice keeps the query API zero-allocation but is awkward for HTTP types. The HTTP layer
will own `Vec<PluginTypeId>` / `Vec<UpdateCategory>` and pass `Some(&vec)` at the call site. This
is consistent with how `exclude_item_ids: Option<&[Uuid]>` works today. Once the future feature
spec adds the HTTP `Validate` impl, that becomes the first line of defence against empty slices;
until then, the helper's debug-assert + warn-and-empty is the sole defence (see R3 step 3).

### `mqtt` actor string (R1)

`"uptrakit-mqtt"` is a service binary name leaking into a semantic actor type. Adding `Mqtt`
variant with canonical `"mqtt"` would be cleaner, but renaming the on-disk value requires checking
every reader. Conservative default: keep `"uptrakit-mqtt"` as the `as_str` output, document the
oddity. A follow-up cleanup can normalize later.

### `UpdateCategory::Other(String)` case-drift (R3, deferred)

`UpdateCategory::Other(String)` filtering in R3 uses exact-string match (`IN (..)` of
`as_str()` values). If a plugin writes `update_category = "kernel"` and another writes
`"Kernel"`, a policy matching one will silently miss the other. This is a pre-existing wire
shape question, not a new bug introduced by this refactor. The feature spec should decide
whether to normalize at the plugin write boundary (preferred) or in the policy filter.

## 7. Snapshot conformance check

The following snapshot rules apply and are satisfied or explicitly addressed:

| Rule (from `.superpowers/standards-snapshot.md`)                      | Compliance                                                                                                              |
| --------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| Prefer typed enums or newtypes over raw String mode flags             | R1, R3, R4 enforce this on the dispatch path                                                                            |
| Prefer typed request/response/config structs over `serde_json::Value` | No new structs introduced; existing typed shape preserved                                                               |
| Use `rootcause::Report`, `report!`, `.context_to()` for errors        | R2, R3 inherit existing module conventions; no new error types                                                          |
| `#[non_exhaustive]` on extensible public enums                        | `ActorType` and `BatchType` are exempt per `coding-standards.md` §"Typed enums for internal write-path discriminators"  |
| Public functions returning `Result` include `# Errors` section        | R2 helper requires a `# Errors` doc comment                                                                             |
| Tests use `start_paused = true` only when tokio time APIs are used    | New tests in R2/R3 do not touch time; no `start_paused` needed                                                          |
| Markdown line length 150 chars                                        | Enforced by `markdownlint`; this file conforms                                                                          |
| Conventional Commits with scope; small granular commits               | Each refactor lands as a separate commit, e.g. `refactor(actor-type): consolidate ad-hoc actor strings into typed enum` |
| Tenant isolation via `TenantDb` for join tables                       | R2 (`find_hosts_with_any_tag`) uses `TenantDb` with primary + belt-and-suspenders filtering                             |
| Database query patterns (`BEGIN IMMEDIATE`, multi-statement atomic)   | R2/R3 are read-only single-statement queries; rule not triggered                                                        |

No deviations.

## 8. Documentation deliverables

Implementation work for this spec must touch the following docs:

- **This spec file** — `docs/superpowers/specs/2026-05-10-auto-update-policy-foundation-spec.md`
  (committed in `docs(specs):` commit).
- **`docs/development/coding-standards.md`** — if R1's inventory keeps any legacy actor string
  (e.g. `"uptrakit-mqtt"`), add a one-paragraph entry under "Typed enums for internal write-path
  discriminators" noting the canonical strings and any preserved-for-compat values.
- **Public doc comments** — every new public function (`find_hosts_with_any_tag`) carries a
  `# Errors` section per `rust-idioms.md`. The empty-slice warn-and-empty contract in R3 is
  documented in each helper's body doc.
- **No ADR** — none of R1–R4 introduces a hard-to-reverse architectural decision; all extend
  existing typed-enum patterns already documented in coding-standards.
- **No CONTEXT.md update** — no new domain terms (`UpdatePolicy` lands with the feature spec, not
  this refactor).
- **No OpenAPI / `uptrakit-web-api-types` changes** — R4 affects internal-only params structs in
  `uptrakit-web-api-queries`; R3 changes internal helper signatures. No externally observable
  behaviour changes from this spec.

## 9. Acceptance gate

This spec is complete when:

1. All four refactors (R1–R4) merged as separate commits. R1 lands before R4.
2. The full backend quality gate suite from `docs/development/quality-gates.md` passes. At minimum:
   - `cargo fmt --all`
   - `cargo check --no-default-features --features db-sqlite`
   - `cargo check --all-features`
   - `cargo clippy --all-targets --no-default-features --features db-sqlite`
   - `cargo clippy --all-targets --all-features`
   - `cargo test --all-features`
   - `cargo deny check`
   - `python3 ci/check_plugin_semantic_boundary.py`
   - `bash ci/verify_no_security_audit.sh`
   - `bash ci/verify_typed_audit_actions.sh`
   - `bash ci/verify_handler_state_contract.sh`
   - `python3 ci/verify_db_access_policy.py`
   - `markdownlint --config .markdownlint.json '**/*.md'`

   Workspace lints already enforce `warnings = "deny"` and `clippy::all = "deny"`; no
   `-- -D warnings` flag is needed on clippy commands.

3. No `actor_type: &str` field remains in `CreateUpdateRecordParams`, `TriggerUpdateParams`, or
   `CreateBatchParams`. No `'a` lifetime parameter exists solely to support `actor_id: &'a str`
   in those structs.
4. `find_hosts_with_any_tag` is callable from any tenant-scoped query context and applies the
   `host_tag.tenant_id` filter via the join (verified by a same-tag-id-different-tenant exclusion
   test).
5. Both candidate queries accept and apply `categories: Option<&[UpdateCategory]>` and
   `plugin_type_ids: Option<&[PluginTypeId]>` filters, with the empty-slice warn-and-empty contract
   in place.
6. The codebase is ready to accept a future implementation plan for the `UpdatePolicy` feature
   without further refactor of the dispatch path.
