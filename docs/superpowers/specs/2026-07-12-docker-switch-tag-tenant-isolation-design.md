# Docker Surface-Action Tenant Isolation — Design

**Date:** 2026-07-12
**Status:** Draft
**Source:** Session analysis of the Docker plugin's DB wiring — both Docker surface actions query
tenant-unscoped join tables using only caller-supplied UUIDs. Same vulnerability class as the Proxmox
match/unmatch spec (`2026-07-11-proxmox-match-tenant-isolation-design.md`) and the web-api-queries
scoping spec (`2026-07-11-web-api-queries-tenant-scoping-design.md`), in a third crate.

## Problem

The Docker plugin's two surface actions on `docker.item-host-actions`
(`crates/plugins/releases/docker/src/surfaces.rs`) run every query against the raw connection —
`ctx.tenant_db().db()` — with filters built exclusively from caller-supplied form params:

1. **`get-current-tag` (read, IDOR).** `handle_get_current_tag` loads
   `host_software_item_plugin` rows filtered on `HostId.eq(host_id)`,
   `SoftwareItemId.eq(software_item_id)`, `PluginType.eq("releases_docker")` — no tenant
   predicate. A tenant-A caller with `UpdateSoftware` permission who supplies tenant-B UUIDs reads
   tenant B's image reference (registry host, repository path, tag — including private registry
   layouts).
2. **`switch-tag` (write, cross-tenant mutation).** `handle_switch_tag` — inside its
   `BEGIN IMMEDIATE` transaction — loads the same plugin rows plus the `host_software_item` row
   with the same unscoped filters, then rewrites `package_identifier` on every Docker plugin row
   and clears the item's version-tracking state. The same tenant-A caller can repoint tenant B's
   container image to an arbitrary reference (e.g. a malicious registry); tenant B's next
   user-triggered update would then pull the attacker-chosen image. This is the most severe
   consequence: a cross-tenant supply-chain redirect.

Neither `host_software_item_plugin` nor `host_software_item` carries a `tenant_id` column
(`crates/shared/db/src/entity/`), so no per-table filter is possible — scoping must join through a
`TenantScoped` parent. Both entities declare `Relation::Host` (`belongs_to host::Entity`), and
`host::Entity` implements `TenantScoped` (`crates/shared/db/src/entity/tenant_scoped.rs`).

This violates the AGENTS.md rule **"Use `TenantDb` helpers for all tenant-scoped queries"** — the
join-table form of which is exactly `find_via_tenant_join` (`docs/development/coding-standards.md`,
Tenant-Safe Database Queries).

### Root cause

Identical to the Proxmox finding: the surface API hands every action a tenant-enforcing handle —
`SurfaceActionContext::tenant_db()` returns `&TenantDb`
(`crates/plugins/infrastructure/core/src/descriptor.rs:191`) — and the handler immediately unwraps
it to the raw connection (`ctx.tenant_db().db()`), discarding the tenant. No plugins-API change is
required; the fix is to stop unwrapping.

### Entry points traced (all covered by an in-handler fix)

Every dispatch path constructs the context from an authenticated tenant, so fixing the queries
inside the handlers covers all invocation routes:

- **Dashboard/REST**: `PluginSurfaceLocalExecutor` Tier-2 allowlist
  (`crates/ui/surface-proxy/src/proxy/local_executor.rs:273`) →
  `PluginOpsSurfaceActionInvoker::invoke` builds `AppStateSurfaceActionController` from the
  session's `tenant_id` (`local_executor.rs:78`); a `None` tenant is rejected before dispatch.
- **Service WS**: `handle_surface_action_request`
  (`crates/ui/web-api/src/routes/service_ws/handler/message_processor.rs:526`) uses
  `service_tenant_id` from the enrolled service identity, not payload data.

The vulnerability is exclusively the missing tenant predicate inside the two handlers; there is no
second resolver or sibling path that re-reaches these tables for Docker (grep: `surfaces.rs` is the
plugin's only DB access).

## Approach

Scope all three query sites in `surfaces.rs` through `TenantDb::find_via_tenant_join`, joining each
join-table entity to `host::Entity` via its declared `Relation::Host`. The tenant filter becomes
structurally part of query construction instead of a per-query discipline — the same shape the
revised Proxmox spec adopted after rejecting point-fix filters.

Feasibility facts (verified in code):

- `find_via_tenant_join<Target, Scoped>(relation)` returns `Select<Target>` =
  `Target::find().join(InnerJoin, relation).filter(Scoped::tenant_id_column().eq(tenant_id))`
  (`crates/shared/tenant-db/src/tenant_db.rs:74`). `Select` executes against any
  `ConnectionTrait`, including the existing `BEGIN IMMEDIATE` transaction handle — the transaction
  structure of `handle_switch_tag` is unchanged (still begun from
  `ctx.tenant_db().db().begin_with_options(…)` with `SqliteTransactionMode::Immediate`).
- The inner join is 1:1 per row (`belongs_to` on an FK), so no duplicate rows are introduced.
- The Docker crate already depends on `uptrakit-tenant-db` and `uptrakit-shared-db`
  (`crates/plugins/releases/docker/Cargo.toml`); no dependency change for production code.

### The three query sites (each read, each different — exact replacements)

1. **`handle_get_current_tag` plugin-row load (`surfaces.rs:181`)** — executes against the plain
   connection, filters include `PluginType`:

   ```rust
   let plugin_rows = ctx
       .tenant_db()
       .find_via_tenant_join::<host_software_item_plugin::Entity, host::Entity>(
           host_software_item_plugin::Relation::Host.def(),
       )
       .filter(host_software_item_plugin::Column::HostId.eq(host_id))
       .filter(host_software_item_plugin::Column::SoftwareItemId.eq(software_item_id))
       .filter(host_software_item_plugin::Column::PluginType.eq(DOCKER_RELEASES_CONFIG_TYPE))
       .all(ctx.tenant_db().db())
       .await…
   ```

2. **`handle_switch_tag` plugin-row load (`surfaces.rs:242`)** — same join, no `PluginType`
   filter (the loop skips non-Docker rows), executes `.all(&txn)`.

3. **`handle_switch_tag` `host_software_item` load (`surfaces.rs:259`)** — Target is
   `host_software_item::Entity`, relation `host_software_item::Relation::Host.def()`, executes
   `.one(&txn)`.

The two **write** sites need no added filter: both convert models returned by the now-scoped loads
into `ActiveModel`s and update by primary key, so they can only touch rows the tenant-scoped reads
produced. The per-row update loop stays a loop — each row receives a distinct
`package_identifier` (its `#container` suffix is preserved), so the batch-update rule does not
apply.

Add one line to the module doc of `surfaces.rs` stating that all queries must go through
`ctx.tenant_db()` join helpers because these tables have no `tenant_id` column.

### Error contract on foreign-tenant IDs (no change to error surface)

Tenant-scoped queries return zero rows for foreign IDs, which lands on existing paths:

- `get-current-tag`: empty `new_image_ref` — byte-identical to today's absent-assignment response;
  no existence oracle.
- `switch-tag`: the existing `"no plugin assignments found for this host"`
  `ControllerIntegration` error, before any write. Matches the not-found-on-foreign-id convention
  established by the Proxmox and web-api-queries specs. No new error variant; the
  `SurfaceActionError` boundary and the Tier-2 audit emission
  (`emit_docker_switch_tag_audit_event`) are untouched.

Deliberately preserved behaviors (explicitly not in scope of the fix): no `deactivated_at` filter
is added for the host (current behavior accepts deactivated hosts; changing that is a separate
product decision), and the `plugin_type` skip stays in the loop for site 2.

### Alternative considered

Upfront host-ownership guard (one `TenantDb::find_by_id::<host::Entity>` check at each handler
entry, queries unchanged). Rejected with the user: it reintroduces a separately-droppable tenant
check next to unscoped queries — the exact mechanism class the Proxmox spec revision rejected — and
a future edit adding a third query would silently inherit nothing.

## Tests

Real in-memory SQLite with the workspace's established pattern — `Database::connect("sqlite::memory:")`

- `uptrakit_shared_db::migration::run_migrations(&db)` (as in
  `crates/ui/web-api-queries/src/queries/hosts.rs:485`). New dev-dependency features only:

```toml
# crates/plugins/releases/docker/Cargo.toml [dev-dependencies]
uptrakit-shared-db = { workspace = true, features = ["migration", "db-sqlite"] }
```

FK constraints are enforced in these DBs, so fixtures must seed the full parent chain per tenant:
`tenant` → `host` + `software_item` → `host_software_item` → `host_software_item_plugin`
(with `plugin_type = "releases_docker"` and a `#container`-suffixed `package_identifier` on at
least one row). No encrypted columns are involved, so `enable_plaintext_mode()` is not needed.

The production `AppStateSurfaceActionController` lives in `uptrakit-surface-proxy`, which the
Docker crate cannot depend on (dependency direction: surface-proxy → plugins). Tests instead define
a ~10-line test-local impl of the `SurfaceActionController` trait
(`crates/plugins/infrastructure/core/src/roles.rs:391` — three methods: `tenant_id`, `user_id`,
`tenant_db`) holding a `TenantDb::new(db.clone(), tenant_id)`; the same pattern infra-core's own
tests use (`descriptor.rs` test module). Cases (two tenants, A = caller, B = victim):

1. **Cross-tenant read blocked**: `get-current-tag` with tenant-B `host_id`/`software_item_id`
   returns empty `new_image_ref`.
2. **Cross-tenant write blocked**: `switch-tag` with tenant-B IDs returns the
   "no plugin assignments" error AND tenant B's `host_software_item_plugin.package_identifier`,
   `host_software_item.package_identifier`, and version columns are asserted unchanged.
3. **Same-tenant read works**: `get-current-tag` with tenant-A IDs returns the stored reference
   with the `#container` suffix stripped.
4. **Same-tenant write works**: `switch-tag` with tenant-A IDs rewrites all Docker plugin rows
   (suffix preserved per row), clears the item's version state, sets
   `update_category = "unknown"`, and skips a seeded non-Docker plugin row.

No tokio time APIs are involved → no `start_paused`. Existing unit tests in `surfaces.rs` (action
descriptors, param parsing, suffix helpers) are unaffected.

## Quality gates / mechanical checklist

- `cargo fmt --all`; `cargo clippy --all-targets --no-default-features --features db-sqlite`;
  `cargo clippy --all-targets --all-features`; `cargo test --all-features` (frontend build first
  for `embed-frontend`).
- No REST endpoint or wire-protocol change → no `./scripts/regen-api.sh`, no `asyncapi.yaml`, no
  openapi-client change.
- No new audit action: the existing Tier-2 `emit_docker_switch_tag_audit_event` continues to record
  attempts and outcomes; foreign-ID attempts now record the error outcome. No `audit-catalog.toml`
  change (no new state-changing site).
- No new workspace dependency (feature additions on an existing workspace dev-dependency only) →
  `cargo deny check` unaffected but run per gates.

## Documentation deliverables

- This spec.
- Module-doc line in `crates/plugins/releases/docker/src/surfaces.rs` (tenant-join requirement).
- No other doc updates: the change implements an already-documented standard
  (`docs/development/coding-standards.md` Tenant-Safe Database Queries;
  `docs/security/surfaces.md` action-permission model is unchanged), alters no externally
  observable API contract, config, or architecture. `CONTEXT.md`, ADRs, wire docs untouched.

## Out of scope

- Sweep of other plugins' surface handlers for the same raw-`db()` pattern (Proxmox already has its
  own spec; a workspace-wide audit is a separate task).
- Host `deactivated_at` filtering in these actions (pre-existing behavior, separate decision).
- Web-api-queries host-assignment scoping (covered by
  `2026-07-11-web-api-queries-tenant-scoping-design.md`).
- Any CI gate that would structurally forbid `ctx.tenant_db().db()` in plugin crates (worth
  considering after the per-crate fixes land; noted, not designed here).
