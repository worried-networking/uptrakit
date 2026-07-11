# Proxmox Match/Unmatch Tenant Isolation — Design

**Date:** 2026-07-11
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

Two smaller defects ride along in the same code:

- `apply_match()` opens its read-then-write transaction with plain `db.begin()` (BEGIN DEFERRED on SQLite), violating
  the project's BEGIN IMMEDIATE rule (SQLITE_BUSY_SNAPSHOT under concurrent discovery writes).
- `handle_save_item_overrides` (surfaces.rs) validates `plugin_config_id` tenant ownership via
  `ensure_proxmox_plugin_config_exists` but never checks that `software_item_id` belongs to the tenant, so a per-item
  protection override can be written against a foreign tenant's software item.

## Approach

Thread the tenant through and push the filter into the first query — the codebase's documented tenant-isolation
pattern. No new abstractions; only signature changes and filters on existing queries. All changes are contained:
`matching::manual_match` / `apply_suggested_match` / `unmatch` have no callers outside these surface handlers (verified
by grep), so the signature change does not ripple.

### Alternative considered

Route `matching.rs` through `TenantDb` instead of raw filters. Rejected for now: the proxmox plugin crate consistently
uses direct `Entity::find` + explicit `TenantId` filters (e.g. `ensure_proxmox_plugin_config_exists`,
`load_proxmox_config`), and `apply_match` needs a transaction handle, which `TenantDb`'s helper methods do not wrap.
Matching the crate's existing idiom is the smaller, consistent change; a crate-wide `TenantDb` migration is a separate
refactor if ever wanted.

### Residual structural risk (named, deferred)

The mechanism that produced this bug survives: the dispatch entry point unwraps the tenant-enforcing handle
(`ctx.tenant_db().db()`) into a raw `&DatabaseConnection` and passes tenant as a separately-droppable
`Option<Uuid>` argument — the next surface action is one forgotten parameter away from the same IDOR class.
Converting the whole chain to a concrete `Uuid` (or a tenant-carrying context) is **not** folded into this fix
because the `Option` is load-bearing in read handlers (`handle_list` et al. branch on `if let Some(tid)`), so the
conversion is a semantic refactor of ~16 arms, not a mechanical rename — wrong vehicle for a security point-fix.
Tracked as a deferred follow-up (see Out of scope). This spec closes the three exploitable write paths and makes
tenant validation a local invariant of each touched handler.

### Other write paths to `proxmox_host_mappings` (ruled in/out)

- `reset.rs` `proxmox_reset_tenant_data`: `delete_many` already filtered on `TenantId.eq(tenant_id)` — correctly
  scoped, out of scope.
- `resource_scaling.rs` restore path: loads the mapping by `record.mapping_id` unfiltered, but the scaling record is
  controller-internal (keyed by `update_history_id`, carries its own `tenant_id`); `mapping_id` is never
  user-supplied on this path — not IDOR-reachable, out of scope. Note: this exclusion is contingent on the current
  call graph, not a structural guarantee — if the restore path ever gains a user-supplied entry point, it inherits
  this spec's filter requirement.
- `discovery.rs` upserts: driven by a tenant-validated plugin config — out of scope (separate audit finding covers
  its prune behavior).

### Audit-log impact

Neither `matching.rs` nor the surface handlers emit stateful audit events; the only catalog entry on this path is the
generic `surface_action.invoke` Event at the web-api layer (no before-snapshot requirement). The `unmatch`
`update_many` redesign therefore breaks no audit contract.

## Changes

### 1. `surfaces.rs` — thread `tenant_id` into the three arms

- `MatchHost` / `ApproveMatch` / `UnmatchHost` dispatch arms pass `tenant_id` like every other arm.
- `execute_controller_manual_match`, `execute_controller_approve_match`, `execute_controller_unmatch_host` gain
  `tenant_id: Option<Uuid>`.
- `handle_match`, `handle_approve_match`, `handle_unmatch` gain `tenant_id: Option<Uuid>` and resolve it first thing
  via the existing `require_tenant_id(tenant_id, "…")` helper (same as `handle_save_item_overrides` does), then pass
  the concrete `Uuid` to `matching::*`. Note: `SurfaceActionContext::tenant_id()` returns a concrete `Uuid`, so the
  `None` branch is defensive idiom-consistency with the other arms — the actual vulnerability fix is the tenant
  filter in `matching.rs`, not the `Option` unwrap.

### 2. `matching.rs` — tenant-scope every query, validate `host_id`

- `manual_match(db, tenant_id, mapping_id, host_id)`, `apply_suggested_match(db, tenant_id, mapping_id, host_id,
  method)`, `unmatch(db, tenant_id, mapping_id)`, and internal `apply_match(db, tenant_id, …)` gain a
  `tenant_id: Uuid` parameter (concrete, not `Option` — surfaces layer resolves the option).
- Mapping lookups (`apply_match` and `unmatch`) add
  `.filter(proxmox_host_mapping::Column::TenantId.eq(tenant_id))`. A foreign-tenant `mapping_id` then yields the
  existing "mapping {id} not found" error — no information leak, no new error variant needed.
- `apply_match` validates the target host before assignment: `host::Entity::find_by_id(host_id)` filtered on
  `TenantId.eq(tenant_id)` and `DeactivatedAt.is_null()` (inside the transaction, same style as
  `host_assignments` fixes elsewhere). Missing → error "host {id} not found" via the existing
  `ProxmoxError::Database` reporting style used for the mapping-not-found case.
- The conflict-clearing query in `apply_match` adds the same `TenantId` filter (defense in depth — after host
  validation it can only match own-tenant rows, but the filter makes the invariant local and grep-provable).
  Same-tenant behavior is unchanged, including multi-config setups: all in-tenant mappings holding that `host_id`
  are still cleared regardless of `plugin_config_id`; the only behavioral delta is that foreign-tenant rows sharing
  a `host_id` are no longer touched — which is the fix.
- `apply_match` switches `db.begin()` →
  `db.begin_with_options(TransactionOptions { sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
  ..Default::default() })` with the standard comment, copying the exact pattern from `protection_store.rs`
  (`upsert_audit`).
- `unmatch` is collapsed to a single atomic statement: `Entity::update_many()` setting `host_id = NULL` and
  `match_method = NULL`, filtered on `Id.eq(mapping_id)` **and** `TenantId.eq(tenant_id)`; `rows_affected == 0` maps
  to the existing "mapping {id} not found" error. This removes the current unwrapped SELECT-then-UPDATE pair (a
  TOCTOU gap today) instead of wrapping it in a transaction — one statement, atomic by construction, no read.

### 3. `surfaces.rs` — `handle_save_item_overrides` software-item check

- New private helper `ensure_software_item_in_tenant(db, tenant_id, software_item_id) -> Result<(), String>` modeled
  exactly on `ensure_proxmox_plugin_config_exists`: `software_item::Entity::find_by_id(...)` +
  `TenantId.eq(tenant_id)` + `DeactivatedAt.is_null()` (the column exists on `software_item` — verified), erroring
  "software item '{id}' was not found in tenant scope".
- Called in `handle_save_item_overrides` right after `ensure_proxmox_plugin_config_exists`, before both the
  delete-override and upsert-override paths (clearing a foreign item's override is also a cross-tenant write).
- Also called in `handle_preload_item_overrides` (right after `require_tenant_id`). Traced during review: the preload
  path does **not** leak foreign-tenant override values today — `find_first_item_override_config` is unscoped, but
  the returned config id is re-validated tenant-scoped and both foreign paths collapse to the same `inherit_global`
  default response. The check is added anyway so tenant validation is a local invariant of every handler that takes a
  `software_item_id`, not an emergent property of a distant config re-validation.

## Tests

Extend the existing `matching.rs` test module (in-memory SQLite harness already present, e.g.
`manual_match_preserves_single_host_mapping_under_conflict`). The current fixtures have no multi-tenant support —
`make_mapping`/`make_host` hardcode `tenant_id: Uuid::nil()` — so extend them to take a `tenant_id` parameter (or
construct `Model` literals inline with two distinct tenant UUIDs, matching the existing DB-backed test's style):

1. `manual_match` with a `mapping_id` belonging to another tenant → error containing "not found"; mapping row
   unchanged.
2. `manual_match` with a `host_id` belonging to another tenant → error; mapping's `host_id` stays `NULL`.
3. `unmatch` with a foreign-tenant `mapping_id` → error; foreign mapping keeps its match.
4. `unmatch` twice on the same in-tenant mapping → **assert both calls return `Ok`** (pins `rows_affected` counting
   matched rows, not changed rows; a matched-but-unchanged row must not surface as "not found").
5. Happy path re-verified: existing conflict-clearing test updated for the new signatures; extend it with a second
   same-tenant mapping under a different `plugin_config_id` to prove cross-config conflict clearing still works.

In the `surfaces.rs` test module (harness already exists — `handle_save_item_overrides` tests at ~2261/2627):

1. `handle_save_item_overrides` with a foreign-tenant `software_item_id` → error "not found in tenant scope"; no
   override row written.
2. `handle_preload_item_overrides` with a foreign-tenant `software_item_id` → same error (no fabricated
   `inherit_global` response for foreign items).

## Documentation deliverables

- None beyond code/doc-comments. The tenant-isolation rule and the BEGIN IMMEDIATE rule are already documented in
  `docs/development/coding-standards.md`; this change makes the code conform to them. No wire-protocol, API-surface,
  or behavior-contract change for legitimate same-tenant callers (foreign-tenant requests change from silent success
  to "not found" — the security fix itself). No new ADR: no architectural decision, just conformance.

## Out of scope / deferred

- Crate-wide migration of the proxmox plugin to `TenantDb` helpers (separate refactor; current crate idiom is
  explicit filters).
- The other audit findings in the proxmox crate (backup-target prune on partial discovery, `randomblob()` migration,
  protection-policy BEGIN DEFERRED sites other than `apply_match`) — separate specs.
- Auditing other plugins' surface dispatch for the same dropped-tenant pattern (worth a quick grep during
  implementation; fixing anything found there is a new finding, not this spec).
- Tightening the surface dispatch chain so tenant-less DB access is unrepresentable (concrete `Uuid` or
  tenant-carrying context instead of raw `&DatabaseConnection` + `Option<Uuid>`; requires resolving the
  `if let Some(tid)` semantics in read handlers) — the structural follow-up named in "Residual structural risk".
