# web-api-queries Tenant-Scoping Gaps — Design

**Date:** 2026-07-11
**Status:** Draft
**Source:** `.superpowers/audit-2026-07-11.md` — HIGH "Host lookups in software-item host assignments have no
tenant filter — cross-tenant host linkage and data exposure" + MEDIUM "Raw ServiceHost::find() in
build_host_metadata joins services without a tenant filter". Both violate the same documented standard ("push
the tenant filter into the first query") in the same crate — one fix.

## Problem

Two tenant-isolation gaps in `crates/ui/web-api-queries`, both cases of a query that has `tenant_id` in scope but
does not filter on it:

1. **Host-assignment lookups (HIGH, IDOR).** `assign_hosts_in_tx` (host_assignments.rs:394),
   `update_host_assignment_in_tx` (:480), and the non-tx `assign_hosts` (:660) / `update_host_assignment` (:763)
   all load the caller-supplied `host_id` via `Host::find_by_id(host_id).filter(DeactivatedAt.is_null())` —
   **never on `tenant_id`**, despite receiving `tenant_id` as a parameter. The route handler validates only the
   software item's tenant, not each host in the request body. A tenant that learns another tenant's host UUID
   (logs, support dumps) can create `host_software_item` / `host_software_item_plugin` rows pointing at the
   foreign host; `try_load_item_hosts` then loads those hosts with no tenant filter either, so the foreign
   host's hostname / friendly_name / OS surface in the attacker tenant's software-item detail response.
   Additionally `update_host_assignment_in_tx`'s `HostSoftwareItem` lookup (:436) is not tenant-scoped through
   the validated item.
2. **`build_host_metadata` agent join (MEDIUM, defense-in-depth).** `build_host_metadata`
   (software_states.rs:609) does `ServiceHost::find().join(service).filter(HostId.is_in(host_ids))
   .filter(status).filter(deactivated_at)` — **no `service.tenant_id` filter**. `host_ids` are tenant-scoped by
   the caller, so a leak needs a `service_host` row linking a foreign-tenant service to this tenant's host
   (normal enrollment prevents it) — but the documented standard exists precisely because this defense failed
   before. If any path (merge, re-enrollment, manual DB fix) ever produces a cross-tenant link, this surfaces
   the foreign agent's `client_version`/`last_seen_at` into another tenant's MQTT software-state payload.

## Approach

Push the tenant filter into the first query at every site — the crate's documented, incident-derived standard.

### Host-assignment lookups (all four + the join-table lookup + the reload)

- Add `.filter(host::Column::TenantId.eq(tenant_id))` to the four `Host::find_by_id` sites (host_assignments.rs
  394, 480, 660, 763). This is the established in-crate idiom (`update_dispatch.rs:864`,
  `update_batches/candidates.rs:74` already do exactly this against a raw `db`/`txn`); `TenantDb::find_by_id`
  is **not** reachable here — the `_in_tx` fns take `&DatabaseTransaction`, not a `TenantDb`. A foreign-tenant
  `host_id` then yields the existing `HostNotFound(host_id)` error — no info leak, no new error variant (matches
  the proxmox spec's not-found-on-foreign-id convention).
- Add a one-line doc comment to `ensure_host_link` stating the caller MUST have validated `host_id` belongs to
  the tenant (it accepts `host_id` unchecked and is safe today only because every caller guards upstream —
  contrarian-flagged latent footgun; cheap regression insurance, no behavior change).
- The `HostSoftwareItem` lookup in `update_host_assignment_in_tx` (:436) needs **no change** — `host_software_item`
  has no `tenant_id` column, and the lookup is already transitively tenant-scoped: `find_active_item(db,
  tenant_id, id)` validated the software item's tenant three lines earlier, and the `HostSoftwareItem::find()`
  filters on that same validated `id`. (Only the `Host::find_by_id` right after it, :480, is the real gap.)
- **Read-path reload (deeper than one line — thread `tenant_id` through the chain):** the unfiltered host load
  is in `load_host_assignment_data` (software_items/mod.rs:332), reached via
  `try_load_item_hosts → try_load_item_hosts_inner → load_host_assignment_data`, and the sibling
  `load_item_hosts` shares the same `_inner`. None of these take `tenant_id` today. Thread `tenant_id: Uuid`
  through all four signatures and both `try_load`/`load_item_hosts` call sites (host_assignments.rs:690, :899),
  then add `host::Column::TenantId.eq(tenant_id)` to the `Host::find()` in `load_host_assignment_data`. This
  scopes the read path independently of the write fix, so a pre-existing cross-tenant link (bad merge, prior
  exploit) cannot leak host details on read.

### `build_host_metadata` agent join

Pass **`&TenantDb`** into `build_host_metadata` (not `tenant_id: Uuid`) — both call sites
(`load_software_states_for_tenant`, `load_software_states_page_for_tenant`, software_states.rs:279/540) already
hold one (verified) — and scope via the crate's canonical join-table helper:
`tenant_db.find_via_tenant_join::<service_host::Entity, service::Entity>(service_host::Relation::Service.def())`
(`service_host` has no `tenant_id`; `service` does — this is the helper's documented example case, matching the
MEMORY-documented pattern). `find_via_tenant_join` returns a plain `Select<service_host::Entity>`, so the
existing `.select_only().column(...).column_as(...).into_model::<AgentInfoRow>()` projection chains onto it
unchanged (verified — same builder type). The `&TenantDb` signature is deliberate: the helper requires it, and
it's the idiomatic scoping vehicle here — no explicit-filter fallback needed (that ambiguity is resolved).

## Tests

Route/query tests via the existing `TestApp`/query test harness (DB-backed, no `start_paused` — snapshot rules):

1. **Assignment IDOR (the HIGH regression):** with two tenants, assert `assign_hosts` / `update_host_assignment`
   (both tx and non-tx variants) return `HostNotFound` when the `host_id` belongs to the other tenant — no
   `host_software_item` row written. FK constraints in the in-memory DB require seeding both tenants + hosts
   (per the harness's existing multi-parent seeding).
2. **Read-path scoping:** construct (or force via direct insert) a cross-tenant `host_software_item` link, then
   assert `try_load_item_hosts` (and `load_item_hosts`, sharing the same `_inner`) does **not** surface the
   foreign host's details — the `load_host_assignment_data` filter holds even when a bad link pre-exists.
3. **`build_host_metadata`:** seed a `service_host` row linking a foreign-tenant service to this tenant's host;
   assert the foreign agent's `client_version`/`last_seen_at` do **not** appear in the metadata for this
   tenant's hosts.
4. Happy path unchanged: same-tenant assignments and same-tenant agent info still resolve exactly as today.

## Documentation deliverables

- No doc changes beyond code: the tenant-isolation standard is already documented
  (`docs/development/coding-standards.md` — "push the tenant filter into the first query"; the raw
  `ServiceHost::find()` anti-pattern is called out there after the prior incident). This change makes the code
  conform. No API/wire/OpenAPI change for legitimate same-tenant callers; foreign-tenant requests change from
  silent success/leak to `HostNotFound` (the fix).
- No new ADR: conformance to an existing documented invariant.

## Out of scope / deferred

- Two more unfiltered `Host::find` sites found during review — `update_history.rs:247` and
  `update_batches/queries.rs:143` — are **out of scope**: their `host_ids` are already tenant-scoped upstream
  (a tenant-filtered `HostId.in_subquery` in update_history's base query; `tenant_db.find_by_id::<update_batch>`
  in the batch-detail path), so the `Host::find().is_in(host_ids)` is a name-lookup over an already-authorized
  id set, not caller-supplied-IDOR like the four `host_assignments.rs` sites. Different risk tier; a
  defense-in-depth filter there is a separate low-priority follow-up, not this fix. (`update_dispatch.rs:864`,
  `update_batches/candidates.rs:74/253` are already correctly filtered.)
- The proxmox surface-action tenant-isolation gaps (separate committed spec).
- Migrating the crate wholesale to `TenantDb` for all queries (the crate mixes `TenantDb` and raw `Entity::find`
  with explicit filters; a wholesale migration is a separate refactor).
