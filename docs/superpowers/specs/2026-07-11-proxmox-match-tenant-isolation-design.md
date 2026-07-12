# Proxmox Match/Unmatch Tenant Isolation — Design

**Date:** 2026-07-11 (revised 2026-07-12: wire `TenantDb` through the proxmox surface chain instead of explicit filters)
**Status:** Draft
**Source:** `.superpowers/audit-2026-07-11.md` — CRITICAL "Match/unmatch surface actions bypass tenant isolation
entirely" + HIGH "apply_match uses BEGIN DEFERRED for a read-then-write transaction" (same function, folded in).

## Problem

Three Proxmox controller-surface write actions mutate `proxmox_host_mappings` without any tenant check:

1. **Dispatch drops the tenant.** In `crates/plugins/infrastructure/proxmox/src/surfaces.rs`, the `MatchHost`,
   `ApproveMatch`, and `UnmatchHost` arms of `execute_controller_surface_action_typed` are the only three that do not
   pass `tenant_id` to their executors (`execute_controller_manual_match(db, request)` etc.), even though every other
   arm threads it through.
2. **`matching.rs` trusts its IDs.** `apply_match()` loads the mapping via
   `proxmox_host_mapping::Entity::find_by_id(mapping_id)` with no tenant filter, clears conflicting mappings filtered
   only by `host_id`, and writes a caller-supplied `host_id` without validating it exists in the caller's tenant.
   `unmatch()` has the same unfiltered lookup.
3. **Impact.** A user with surface-action permission in tenant A can mutate tenant B's mappings, or attach a foreign
   `host_id` to their own mapping — after which guest-exec, update protection, and scaling would target another
   tenant's VM. The same file documents the required standard in `load_proxmox_config` ("The tenant_id filter …
   prevents IDOR"); these write paths skip it.

Three smaller defects ride along in the same code:

- `apply_match()` opens its read-then-write transaction with plain `db.begin()` (BEGIN DEFERRED on SQLite), violating
  the project's BEGIN IMMEDIATE rule (SQLITE_BUSY_SNAPSHOT under concurrent discovery writes).
- `unmatch()` runs its SELECT-then-UPDATE with no transaction at all — a TOCTOU gap independent of the tenant-isolation
  bug (a concurrent `unmatch` or re-match between the read and the write can silently lose an update).
- `handle_save_item_overrides` (surfaces.rs) validates `plugin_config_id` tenant ownership via
  `ensure_proxmox_plugin_config_exists` but never checks that `software_item_id` belongs to the tenant, so a per-item
  protection override can be written against a foreign tenant's software item.

### Root cause

The plugin surface API already hands every action a tenant-enforcing handle:
`SurfaceActionContext::tenant_db()` returns `&TenantDb`
(`crates/plugins/infrastructure/core/src/descriptor.rs`, feature `plugin-ops`, enabled unconditionally by the proxmox
crate). The vulnerability exists because the proxmox dispatch entry point (`handle_action_inner`, surfaces.rs)
immediately unwraps it — `ctx.tenant_db().db()` + `Some(ctx.tenant_id())` — and threads a raw
`(&DatabaseConnection, Option<Uuid>)` pair from there down. The tenant becomes a separately-droppable parameter, and
three arms dropped it. **No change to the plugins API is required**; the fix is to stop unwrapping.

## Approach

Thread `&TenantDb` through the proxmox surface dispatch chain and `matching.rs`, replacing the
`(&DatabaseConnection, Option<Uuid>)` pair everywhere in that chain. All `proxmox_host_mapping`, `host`, and
`software_item` queries on the touched paths go through `TenantDb` builders (`find`, `find_by_id`, `update_many`), so
the tenant filter is structurally unforgettable rather than a per-query discipline.

Feasibility facts (verified in code):

- `proxmox_host_mapping::Entity` already implements `TenantScoped`
  (`crates/plugins/infrastructure/proxmox/src/entity/proxmox_host_mapping.rs`); `host::Entity`,
  `software_item::Entity`, and `plugin_config::Entity` implement it in
  `crates/shared/db/src/entity/tenant_scoped.rs`. The proxmox crate already depends on `uptrakit-tenant-db`.
- `TenantDb` methods return query **builders** (`Select<E>`, `UpdateMany<E>`) that execute against any
  `ConnectionTrait`, including a transaction handle:
  `tenant_db.find_by_id::<proxmox_host_mapping::Entity, _>(id).one(&tx)`. Transactions are begun from
  `tenant_db.db().begin_with_options(…)`. (The previous draft rejected `TenantDb` on the belief that its helpers
  cannot participate in transactions — that was incorrect.)
- Removing the `Option<Uuid>` is mechanical, not semantic: grep shows **no caller** — production or test — passes
  `tenant_id = None` into any surfaces.rs handler. The production dispatch always passes `Some(ctx.tenant_id())`, and
  every test passes `Some(tenant_id)`. The `if let Some(tid)` branches in read handlers are dead `None` paths.
- `matching::manual_match` / `apply_suggested_match` / `unmatch` have no callers outside these surface handlers
  (verified by grep), so the signature change does not ripple beyond the crate.
- This also brings the touched code into conformance with AGENTS.md rule "Use `TenantDb` helpers for all
  tenant-scoped queries" — the current raw `Entity::find_by_id` calls on `TenantScoped` entities violate it.

### Alternative considered

Point-fix with explicit `.filter(Column::TenantId.eq(tenant_id))` on each query, keeping the
`(db, Option<Uuid>)` plumbing (the previous draft of this spec). Rejected: it preserves the exact mechanism that
produced the bug — a separately-droppable tenant parameter next to an unscoped connection — and its stated reasons
for avoiding `TenantDb` do not hold (the transaction objection was factually wrong; the "crate idiom" objection is
stale — `resource_scaling.rs` and `update_protection.rs` already construct and hold `TenantDb`, and the surface
context has carried one all along).

### Other write paths to `proxmox_host_mappings` (ruled in/out)

- `reset.rs` `proxmox_reset_tenant_data`: `delete_many` already filtered on `TenantId.eq(tenant_id)` — correctly
  scoped, out of scope.
- `resource_scaling.rs` restore path: loads the mapping by `record.mapping_id` unfiltered, but the scaling record is
  controller-internal (keyed by `update_history_id`, carries its own `tenant_id`); `mapping_id` is never
  user-supplied on this path — not IDOR-reachable, out of scope. Note: this exclusion is contingent on the current
  call graph, not a structural guarantee — if the restore path ever gains a user-supplied entry point, it inherits
  this spec's requirement.
- `discovery.rs` upserts: driven by a tenant-validated plugin config — out of scope (separate audit finding covers
  its prune behavior).

### Audit-log impact

Neither `matching.rs` nor the surface handlers emit stateful audit events; the only catalog entry on this path is the
generic `surface_action.invoke` Event at the web-api layer (no before-snapshot requirement). The `unmatch`
`update_many` redesign therefore breaks no audit contract.

Classification note (no change from current behavior, documented for reviewer awareness): a rejected cross-tenant
attempt surfaces as `SurfaceActionError::ControllerIntegration(String)` (via `map_controller_action_error`), which
`crates/ui/surface-proxy/src/proxy/controller_local.rs::map_surface_action_error` collapses — together with all other
`ControllerIntegration`/`PluginInternal` errors — into `SurfaceProxyError::SendFailed`, classified by
`classify_surface_proxy_error_for_audit` as `AuditOutcome::Failed` / `"provider_unavailable"`. The `surface_action.invoke`
Event therefore does record the attempt, but with the same outcome/reason as a transient controller-side failure, not
a distinct denial signal. This collapse is pre-existing (applies identically to every `ControllerIntegration` error
today), already tracked as an accepted observability gap in
`docs/adr/0018-plugin-extension-typed-boundary.md` (Consequences § observability gap), and out of scope for this fix —
this spec does not change error classification, only which rows a cross-tenant request can reach.

## Changes

All changes are contained in the proxmox plugin crate. **If implementation discovers that any change to
`uptrakit-plugin-infrastructure-core` (or any other crate's public API) is required after all — stop and escalate
before proceeding.**

### 1. `surfaces.rs` — dispatch takes `&TenantDb`, `Option<Uuid>` plumbing removed

Sizing note: `surfaces.rs` has roughly two dozen call sites taking `tenant_id: Option<Uuid>` today (every
`execute_controller_*` wrapper plus every handler behind it), not just the three vulnerable write arms — this is a
signature-threading change across the whole dispatch chain, not a three-function patch.

Commit ordering: land this as two commits so the security-critical delta is independently reviewable and revertible.
**Commit A** — the vulnerability closure: `handle_action_inner` and `execute_controller_surface_action_typed` (the
dispatch entry point, since the three write wrappers currently take only `db` with no `tenant_id` to thread — closing
finding 1 requires touching the entry point, not just the three arms), the three write-wrapper signatures
(`execute_controller_manual_match`/`approve_match`/`unmatch_host`) and their `handle_match`/`handle_approve_match`/
`handle_unmatch` handlers, all of `matching.rs` (host_id validation, BEGIN IMMEDIATE, atomic unmatch), and the
`handle_save_item_overrides`/`handle_preload_item_overrides` software-item check (§3). **Commit B** — the read-handler
and `require_tenant_id`-caller signature churn, which is behavior-preserving by the "no `None` caller" argument above.
Commit A alone closes the IDOR; Commit B is defense-in-depth cleanup.

- `handle_action_inner` passes `ctx.tenant_db()` instead of `(ctx.tenant_db().db(), Some(ctx.tenant_id()))`.
- `execute_controller_surface_action_typed(tenant_db: &TenantDb, surface_id, action_id, params)` — the
  `db`/`tenant_id` pair disappears from the dispatch signature. It has no callers besides `handle_action_inner`.
- Every `execute_controller_*` wrapper and every handler behind it takes `tenant_db: &TenantDb` in place of
  `db: &DatabaseConnection, tenant_id: Option<Uuid>`:
  - **Write handlers** (`handle_match`, `handle_approve_match`, `handle_unmatch`) pass `tenant_db` to
    `matching::*` — this closes finding 1.
  - **Read handlers** (`handle_list`, `handle_get_info`, `handle_list_all_unmatched`, …): the `if let Some(tid)`
    filter branches become unconditional — mapping queries switch to `tenant_db.find::<proxmox_host_mapping::Entity>()`
    (+ existing non-tenant filters). Production behavior is unchanged (tenant was always present); the dead
    `None` = unfiltered path is deleted. This deletion's safety is contingent on `SurfaceActionContext::tenant_id()`
    staying non-optional (`Uuid`, not `Option<Uuid>`) — same caveat this spec already applies to the
    `resource_scaling.rs` restore-path exclusion above; if infrastructure-core ever re-optionalizes it, these read
    paths inherit this spec's requirement.
  - Handlers that call helpers keeping raw signatures (`resolve_scope_plugin_configs`,
    `load_item_override`, `upsert_item_override`, `find_first_item_override_config`,
    `ensure_cached_backup_target_exists`, …) pass `tenant_db.db()` and `tenant_db.tenant_id()` — those helpers are
    keyed by already-tenant-validated IDs and are not rewritten here.
  - The nine `handle_preload_*`/`handle_save_*`/`handle_load_backup_target_options` handlers that currently open with
    `let tenant_id = require_tenant_id(tenant_id, "…")?;` (global defaults, item overrides, scaling global defaults,
    scaling item overrides, backup target options) also move to `tenant_db: &TenantDb` parameters. Since
    `TenantDb::tenant_id()` returns `Uuid` (not `Option<Uuid>`), these call sites drop the `require_tenant_id` call
    entirely and use `tenant_db.tenant_id()` directly wherever they currently use the unwrapped `tenant_id`.
- `require_tenant_id` (surfaces.rs) loses all callers once the above nine sites and the write/read handlers take
  `&TenantDb` — delete it.
- `ensure_proxmox_plugin_config_exists` switches to `(tenant_db: &TenantDb, plugin_config_id: Uuid)` using
  `tenant_db.find_by_id::<plugin_config::Entity, _>(…)` + the existing `PluginType`/`DeactivatedAt` filters (it is
  touched at every call site anyway).

### 2. `matching.rs` — `TenantDb` everywhere, validate `host_id`, IMMEDIATE, atomic unmatch

- `manual_match(tenant_db, mapping_id, host_id)`, `apply_suggested_match(tenant_db, mapping_id, host_id, method)`,
  `unmatch(tenant_db, mapping_id)`, and internal `apply_match(tenant_db, …)` take `tenant_db: &TenantDb` in place of
  `db: &DatabaseConnection`. Once no signature in the file references `DatabaseConnection` directly, remove its
  now-unused `use` import — the workspace denies warnings, so a leftover unused import fails the build.
- `apply_match` opens its transaction with
  `tenant_db.db().begin_with_options(TransactionOptions { sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate), ..Default::default() })`
  with the standard comment, copying the exact pattern from `protection_store.rs` (`upsert_audit`).
- Mapping lookup: `tenant_db.find_by_id::<proxmox_host_mapping::Entity, _>(mapping_id).one(&tx)`. A foreign-tenant
  `mapping_id` then yields the existing "mapping {id} not found" error — no information leak, no new error variant.
- Host validation before assignment: `tenant_db.find_by_id::<host::Entity, _>(host_id)` +
  `.filter(host::Column::DeactivatedAt.is_null())`, executed on `&tx`. Missing → "host {id} not found" via the
  existing `ProxmoxError::Database` reporting style used for the mapping-not-found case.
- Conflict-clearing: the conflict query becomes
  `tenant_db.find::<proxmox_host_mapping::Entity>().filter(HostId.eq(host_id)).filter(Id.ne(mapping_id)).all(&tx)` —
  tenant scoping is now automatic; the per-conflict `tracing::warn!` loop is retained. Same-tenant behavior is
  unchanged, including multi-config setups: all in-tenant mappings holding that `host_id` are still cleared
  regardless of `plugin_config_id`; the only behavioral delta is that foreign-tenant rows sharing a `host_id` are no
  longer touched — which is the fix.
- `unmatch` is collapsed to a single atomic statement, following the existing `update_many` call shape used in
  `crates/ui/web-api-queries/src/queries/enrollment_tokens.rs` (`revoke_enrollment_token` — the only other
  `TenantDb::update_many` call site in the repo; this is the first plugin-crate use of it):
  `tenant_db.update_many::<proxmox_host_mapping::Entity>().col_expr(Column::HostId, Expr::value(None::<Uuid>)).col_expr(Column::MatchMethod, Expr::value(None::<String>)).filter(Column::Id.eq(mapping_id)).exec(tenant_db.db())`.
  `rows_affected == 0` maps to the existing "mapping {id} not found" error. This removes the current unwrapped
  SELECT-then-UPDATE pair (a TOCTOU gap today) instead of wrapping it in a transaction — one statement, atomic by
  construction, no read.

### 3. `surfaces.rs` — `handle_save_item_overrides` software-item check

- New private helper `ensure_software_item_in_tenant(tenant_db: &TenantDb, software_item_id: Uuid) -> Result<(), String>`
  modeled on `ensure_proxmox_plugin_config_exists`:
  `tenant_db.find_by_id::<software_item::Entity, _>(software_item_id)` +
  `.filter(software_item::Column::DeactivatedAt.is_null())` (the column exists on `software_item` — verified),
  erroring "software item '{id}' was not found in tenant scope".
- Called in `handle_save_item_overrides` right after `ensure_proxmox_plugin_config_exists`, before both the
  delete-override and upsert-override paths (clearing a foreign item's override is also a cross-tenant write).
- Also called first thing in `handle_preload_item_overrides`. Traced during review: the preload path does **not**
  leak foreign-tenant override values today — `find_first_item_override_config` is unscoped, but the returned config
  id is re-validated tenant-scoped and both foreign paths collapse to the same `inherit_global` default response. The
  check is added anyway so tenant validation is a local invariant of every handler that takes a `software_item_id`,
  not an emergent property of a distant config re-validation.

## Tests

Harness facts (corrected from the previous draft): the existing `matching.rs` and `surfaces.rs` test modules use
`sea_orm::MockDatabase`, **not** in-memory SQLite. `MockDatabase` returns appended rows regardless of `WHERE`
clauses, so it cannot prove tenant filtering — it is fine for signature-conversion churn but not for isolation
assertions. Two layers:

**a. Convert existing tests (MockDatabase).** All current `matching.rs`/`surfaces.rs` tests that call the changed
functions wrap their mock connection as `TenantDb::new(db, tenant_id)` — the exact pattern already used in
`resource_scaling.rs` tests. Assertion changes: none beyond signatures, except tests that previously passed
`Some(tenant_id)` now pass it inside the `TenantDb`. The existing conflict-clearing test
(`manual_match_preserves_single_host_mapping_under_conflict`) is extended with a second same-tenant mapping under a
different `plugin_config_id` to pin cross-config conflict clearing.

**b. New tenant-isolation tests (real in-memory SQLite).** Add
`uptrakit-shared-db = { workspace = true, features = ["migration"] }` to the proxmox crate's `[dev-dependencies]`
(`uptrakit-shared-db` is already in `[workspace.dependencies]` at `Cargo.toml:123` with `default-features = false`, so
no new `[workspace.dependencies]` entry is needed — only the crate-local `features = ["migration"]` opt-in). Note:
every current consumer of the `migration` feature is a `crates/ui/*` or `crates/core/*` crate; this is the first
`crates/plugins/*` crate to pull it into `[dev-dependencies]` — a new precedent, not reuse of an established
plugin-crate pattern. Test setup: `Database::connect("sqlite::memory:")`,
run the shared-db migrations (creates `tenant`, `host`, `software_item`, `plugin_config`) plus the plugin's
controller migration (creates `proxmox_host_mappings` — pattern exists in `controller_migration.rs` tests), insert
fixture rows for **two tenants** (FK constraints are enforced: each mapping needs its `tenant` + `plugin_config`
parents, each match target its `host`; `testing::insert_host_mapping` helps).

Invariant: error-string assertions alone ("not found") are insufficient — `find_by_id` returning `None` looks
identical whether a row is absent or merely foreign, so every isolation case below MUST assert the foreign row's
unchanged post-state as the primary check, not just the error text. Cases 1–3 already specify this; keep it that way
under future edits. Cases:

1. `manual_match` with a `mapping_id` belonging to another tenant → error containing "not found"; foreign mapping row
   unchanged.
2. `manual_match` with a `host_id` belonging to another tenant → error; mapping's `host_id` stays `NULL`.
3. `unmatch` with a foreign-tenant `mapping_id` → error; foreign mapping keeps its match.
4. `unmatch` twice on the same in-tenant mapping → **assert both calls return `Ok`** (pins `rows_affected` counting
   matched rows, not changed rows; a matched-but-unchanged row must not surface as "not found").
5. Happy-path conflict clearing across two same-tenant configs re-verified on the SQLite harness (real `WHERE`
   evaluation, complementing the mock test in (a)).

In the `surfaces.rs` test module (MockDatabase is sufficient here — the check is an explicit query the handler either
makes or doesn't):

1. `handle_save_item_overrides` with a foreign-tenant `software_item_id` (mock returns no row for the
   `ensure_software_item_in_tenant` query) → error "not found in tenant scope"; no override row written.
2. `handle_preload_item_overrides` with a foreign-tenant `software_item_id` → same error (no fabricated
   `inherit_global` response for foreign items).

## Documentation deliverables

- None beyond code/doc-comments. The tenant-isolation rule (`TenantDb` helpers, AGENTS.md rule 28) and the BEGIN
  IMMEDIATE rule are already documented in `docs/development/coding-standards.md`; this change makes the code conform
  to them. No wire-protocol, API-surface, or behavior-contract change for legitimate same-tenant callers
  (foreign-tenant requests change from silent success to "not found" — the security fix itself). No new ADR: no
  architectural decision, just conformance.

## Out of scope / deferred

- Other plugins unwrapping the surface handle the same way — `crates/plugins/releases/docker/src/surfaces.rs` also
  does `ctx.tenant_db().db()` into raw queries. Worth the same conversion; auditing/fixing it is a separate finding,
  not this spec (quick grep during implementation encouraged).
- Crate-wide `TenantDb` migration of proxmox modules **off** the surface path (`discovery.rs`,
  `protection_store.rs`, `policy_store.rs`, `scaling_store.rs`) — separate refactor; this spec converts the surface
  dispatch chain and `matching.rs` only.
- The other audit findings in the proxmox crate (backup-target prune on partial discovery, `randomblob()` migration,
  protection-policy BEGIN DEFERRED sites other than `apply_match`) — separate specs.
- Making the plugins API hand out **only** `TenantDb` (removing the `.db()` escape hatch) — an
  infrastructure-core API change, explicitly excluded from this spec.
