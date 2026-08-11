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

1. **`current-tag` (read, IDOR).** `handle_get_current_tag` loads
   `host_software_item_plugin` rows filtered on `HostId.eq(host_id)`,
   `SoftwareItemId.eq(software_item_id)`, `PluginType.eq("releases.docker")` — no tenant
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
`TenantScoped` parent. Both entities declare **two** `TenantScoped` parents:
`Relation::SoftwareItem` (`belongs_to software_item::Entity`) and `Relation::Host`
(`belongs_to host::Entity`); both `software_item::Entity` and `host::Entity` implement `TenantScoped`
(`crates/shared/db/src/entity/tenant_scoped.rs`).

**Both parents must be tenant-verified, not just one.** A single-parent join tenant-validates that
parent but leaves the other unchecked — and for `switch-tag` a single-parent join on **either**
anchor is insufficient, not merely weaker:

- The sibling `assign_hosts` write path
  (`crates/ui/web-api-queries/src/queries/software_items/host_assignments.rs`, covered by
  `2026-07-11-web-api-queries-tenant-scoping-design.md`) loads its host via an unscoped
  `Host::find_by_id(...)` and can create a `host_software_item` / `host_software_item_plugin` row
  whose `host.tenant_id != software_item.tenant_id`.
- Given such a row, **anchoring only on `host`** leaks the other tenant's `software_item` image
  reference (the read IDOR); **anchoring only on `software_item`** still lets the caller rewrite
  `package_identifier`, so the _other_ tenant's host pulls the attacker-chosen image on its next
  update (the write redirect — the highest-severity consequence). Each single-parent variant simply
  relocates which tenant is exposed; neither closes the redirect.
- Only validating **both** parents closes it. Doing so also removes any ordering dependency on the
  concurrent `assign_hosts` fix: the Docker fix stays correct whether or not that invariant is
  currently restored, rather than relying on an invariant a sibling bug violates today. There is no
  DB-layer way to enforce `host.tenant_id == software_item.tenant_id` (no cross-column FK/CHECK), so
  the reader is the only place this can be enforced for these queries.

**Scope boundary (intentional, not "class handled").** This spec hardens the two Docker write/read
handlers only. The sibling _reader_ `plugin_types_for_role`
(`crates/ui/web-api-queries/src/queries/host_software_item_plugins/mod.rs:66`) scopes the same table
through `software_item` alone and remains single-parent after this change. That is a deliberate
scope line: Docker's `switch-tag` is a _mutation_ that redirects what a host pulls (supply-chain
blast radius), warranting belt-and-suspenders locally; the version-check reader is read-only
enrichment. Promoting both-parent scoping to a shared `TenantDb` helper and applying it uniformly
(including that reader) is the correct long-term move but is a separate, tracked task (see Out of
scope), not silently folded in here.

This violates the AGENTS.md rule **"Use `TenantDb` helpers for all tenant-scoped queries"** — the
join-table form of which is exactly `find_via_tenant_join` (`docs/development/coding-standards.md`,
Tenant-Safe Database Queries).

### Root cause

Identical to the Proxmox finding: the surface API hands every action a tenant-enforcing handle —
`SurfaceActionContext::tenant_db()` returns `&TenantDb`
(`crates/plugins/infrastructure/core/src/descriptor.rs:188`) — and the handler immediately unwraps
it to the raw connection (`ctx.tenant_db().db()`), discarding the tenant. No plugins-API change is
required; the fix is to stop unwrapping.

### Entry points traced (all covered by an in-handler fix)

Every dispatch path constructs the context from an authenticated tenant, so fixing the queries
inside the handlers covers all invocation routes:

- **Dashboard/REST**: `PluginOpsSurfaceActionInvoker::invoke` parses `request.tenant_id`
  (`crates/ui/surface-proxy/src/proxy/local_executor.rs:151`) and builds
  `AppStateSurfaceActionController` from it (`local_executor.rs:83`); a missing DB/tenant is
  rejected before dispatch. `switch-tag` routes through the Tier-2 audit allowlist
  (`local_executor.rs:253`); `current-tag` falls through to the Tier-3 generic invoke
  (`local_executor.rs:316`) — both build the invoker context from the same parsed tenant.
- **Service WS**: `handle_surface_action_request`
  (`crates/ui/web-api/src/routes/service_ws/handler/message_processor.rs:526`) parses the tenant
  from `payload.tenant_id` and, for **tenant** services, rejects any mismatch against the enrolled
  `service_tenant_id` (`message_processor.rs:601`). **System** services (`service_tenant_id =
  None`) skip that identity check — for them the payload-declared tenant flows through. What
  blocks Docker on this path is the provider-origin gate: both Docker interactions declare
  `required_action = software:update` (`plugin.rs:296`, `:320`, `:390`) and neither sets
  `provider_invocable`, so provider-origin invocation of an action-gated interaction is denied
  before dispatch (`crates/ui/surface-proxy/src/proxy.rs:253`). The in-handler tenant scoping is
  therefore the correct fix for every path that _can_ dispatch, and the test plan pins the
  `provider_invocable = false` assumption the system-service path relies on (test case 6).

The vulnerability is exclusively the missing tenant predicate inside the two handlers; there is no
second resolver or sibling path that re-reaches these tables for Docker (grep: `surfaces.rs` is the
plugin's only DB access).

## Approach

Scope all three query sites in `surfaces.rs` so both `TenantScoped` parents are tenant-verified.
Each query anchors on `software_item::Entity` via `TenantDb::find_via_tenant_join` (matching the
existing precedent for the same table — `crates/ui/web-api-queries/src/queries/host_software_item_plugins/mod.rs:67`,
which scopes `host_software_item_plugin` through `Relation::SoftwareItem`), then adds a second
`InnerJoin` on `Relation::Host` with an explicit `host::Column::TenantId.eq(ctx.tenant_id())`
predicate. The tenant filter becomes structurally part of query construction instead of a per-query
discipline, and neither parent's tenant is left unchecked.

Feasibility facts (verified in code):

- `find_via_tenant_join<Target, Scoped>(relation)` returns `Select<Target>` =
  `Target::find().join(InnerJoin, relation).filter(Scoped::tenant_id_column().eq(tenant_id))`
  (`crates/shared/tenant-db/src/tenant_db.rs:74`). The returned `Select` is further chainable, so
  the second `.join(JoinType::InnerJoin, …Relation::Host.def())` + `.filter(…)` compose onto it.
  `Select` executes against any `ConnectionTrait`, including the existing `BEGIN IMMEDIATE`
  transaction handle — the transaction structure of `handle_switch_tag` is unchanged (still begun
  from `ctx.tenant_db().db().begin_with_options(…)` with `SqliteTransactionMode::Immediate`).
- Anchoring on `software_item` rather than `host` matches the only existing precedent that scopes
  this exact table; the added host predicate is the defense-in-depth layer, not the anchor.
- Composing a chained manual `InnerJoin` onto a `find_via_tenant_join` result is an established
  pattern, not a novel construction:
  `crates/ui/web-api-queries/src/queries/services.rs` (:783; the chained join at :787 — line
  numbers are hints, grep `find_via_tenant_join` in that file) anchors `service_host` via `service`,
  then chains `.join(JoinType::InnerJoin, …Relation::Host.def())` with a follow-on `host::` filter.
  The Docker sites are structurally identical; the second join's filter targets `host::Column::TenantId`
  (defense-in-depth) rather than a `DeactivatedAt` predicate.
- Both inner joins are 1:1 per row (`belongs_to` on an FK), so no duplicate rows are introduced.
  SeaORM table-qualifies all generated column references, so the `host_id` column name existing on
  both the join table and — indirectly — the joined parents is not ambiguous.
- `ctx.tenant_id()` (`descriptor.rs:177`) and `TenantDb::tenant_id()` (`tenant_db.rs:25`) both
  expose the caller tenant for the second predicate.
- The Docker crate already depends on `uptrakit-tenant-db` and `uptrakit-shared-db`
  (`crates/plugins/releases/docker/Cargo.toml`); no dependency change for production code. Explicit
  import diff for `surfaces.rs` (the chains do NOT compile without these — `.join()` is a
  `QuerySelect` trait method, `.def()` needs `RelationTrait`; neither handler imports them today).
  Both handlers use **function-scoped** `use` blocks, not module-level imports:
  `handle_get_current_tag` at lines 100-103 (`use sea_orm::ColumnTrait as _;` /
  `EntityTrait as _` / `QueryFilter as _` plus the `host_software_item_plugin` entity import),
  `handle_switch_tag` in the grouped block at lines 139-143. Add to each function's block:
  `JoinType`, `QuerySelect as _`, `RelationTrait as _` (the same trait set `services.rs` uses for
  its own chained join, module-level at its lines 2-5), and extend the
  `uptrakit_shared_db::entity` import with `host` and `software_item`.
  - **Retain** `ColumnTrait as _` (the `Column::…eq()`/`host::Column::TenantId.eq()` predicates)
    and `QueryFilter as _` (the post-join `.filter(…)`). Both are still used after the refactor.
  - **Drop** `EntityTrait as _`: its _only_ uses in each handler are the three
    `host_software_item_plugin::Entity::find()` / `host_software_item::Entity::find()` calls
    (`surfaces.rs:111`, `:172`, `:189`) — the exact sites being replaced by
    `ctx.tenant_db().find_via_tenant_join::<…>()`, whose `Target::find()` happens _inside_ the
    helper. With those gone, no direct `Entity::` method remains in either body, so leaving
    `EntityTrait as _` imported trips `unused_imports` under `warnings = "deny"`. Remove it in the
    same edit that swaps the queries. (`switch-tag`'s `ActiveModelTrait`/`Set`/`TransactionTrait`
    imports stay — the write path still builds `ActiveModel`s and runs the transaction.)

### The three query sites (each read, each different — exact replacements)

1. **`handle_get_current_tag` plugin-row load (`surfaces.rs:111`)** — executes against the plain
   connection, filters include `PluginType`:

   ```rust
   let plugin_rows = ctx
       .tenant_db()
       .find_via_tenant_join::<host_software_item_plugin::Entity, software_item::Entity>(
           host_software_item_plugin::Relation::SoftwareItem.def(),
       )
       .join(JoinType::InnerJoin, host_software_item_plugin::Relation::Host.def())
       .filter(host::Column::TenantId.eq(ctx.tenant_id()))
       .filter(host_software_item_plugin::Column::HostId.eq(host_id))
       .filter(host_software_item_plugin::Column::SoftwareItemId.eq(software_item_id))
       .filter(host_software_item_plugin::Column::PluginType.eq(DOCKER_RELEASES_CONFIG_TYPE))
       .all(ctx.tenant_db().db())
       .await…
   ```

   The `find_via_tenant_join` call supplies the `software_item` tenant predicate; the chained
   `.join(...Relation::Host...)` + `.filter(host::Column::TenantId...)` adds the host one. Also
   remove the now-unused `let db = ctx.tenant_db().db();` binding at `surfaces.rs:110` — nothing
   else in `handle_get_current_tag` references it after the rewrite, and leaving it trips
   `unused_variables` under `warnings = "deny"`. (No equivalent removal at sites 2/3: `db` stays
   alive in `handle_switch_tag` for `db.begin_with_options(…)`.)

2. **`handle_switch_tag` plugin-row load (`surfaces.rs:172`)** — same two-parent join, no
   `PluginType` filter (the loop skips non-Docker rows), executes `.all(&txn)`.

3. **`handle_switch_tag` `host_software_item` load (`surfaces.rs:189`)** — Target is
   `host_software_item::Entity`; anchor relation `host_software_item::Relation::SoftwareItem.def()`,
   second join `host_software_item::Relation::Host.def()` + `host::Column::TenantId` filter,
   executes `.one(&txn)`. Preserve the existing `.one()` semantics exactly — do not switch to
   `.all()`/aggregate. The current code already loads this row with `.one()` on the same
   `(host_id, software_item_id)` filter; the qualifier column means a pair is unique only per
   partial index, so any `.one()`-ambiguity is pre-existing behavior this fix must neither introduce
   nor "improve" away.

The site-2 (plugin rows) and site-3 (`host_software_item`) loads carry independent two-parent joins
but filter the **same** caller-supplied `(host_id, software_item_id)` pair with the **same** two
tenant predicates. The tenant verdict is robust to `host_software_item` qualifier multiplicity: a
single `(host_id, software_item_id)` pair may legitimately have more than one `host_software_item`
row (the `qualifier` column is nullable and `uix_hsi_qualified` keys on `(host_id, software_item_id,
qualifier)`, so the multi-container case produces several rows; `.one()` returns whichever the result
order yields — pre-existing behavior). But **all** such rows share the same `host_id` and
`software_item_id`, hence the same two parents, hence the same tenant verdict — the two-parent filter
admits or rejects them as a set. Whichever row `.one()` returns is therefore guaranteed to be the
caller's tenant. The two joins cannot disagree for same-tenant data regardless of qualifier
multiplicity: a foreign or mismatched pair yields empty on both (site 2's empty check returns the "no
plugin assignments" error before site 3 runs); a same-tenant pair matches on both. This argument is
tenant-set-based, not index-uniqueness-based — the schema does **not** guarantee a single row per
pair, and the safety claim does not rely on it.

Unlike the `host_software_item_plugins/mod.rs` precedent (which `.select_only()`s a projection),
these sites load full `Model`s — the `switch-tag` handler converts them into `ActiveModel`s to
mutate `package_identifier`, so the full row is required; do not narrow to `.select_only()`.

The two **write** sites need no added filter: both convert models returned by the now-scoped loads
into `ActiveModel`s and update by primary key, so they can only touch rows the tenant-scoped reads
produced. The per-row update loop stays a loop — each row receives a distinct
`package_identifier` (its `#container` suffix is preserved), so the batch-update rule does not
apply.

Add one line to the module doc of `surfaces.rs` stating that no query may be _built_ from the raw
connection — queries against these tables must go through `ctx.tenant_db()` join helpers because
the tables have no `tenant_id` column. (The raw handle itself legitimately remains as the
executor and for `begin_with_options`.)

### Error contract on foreign-tenant IDs (no change to error surface)

Tenant-scoped queries return zero rows for foreign IDs, which lands on existing paths:

- `current-tag`: empty `new_image_ref` — byte-identical to today's absent-assignment response;
  no existence oracle.
- `switch-tag`: the existing `"no plugin assignments found for this host"`
  `ControllerIntegration` error, before any write. Matches the not-found-on-foreign-id convention
  established by the Proxmox and web-api-queries specs. No new error variant; the
  `SurfaceActionError` boundary and the Tier-2 audit emission
  (`emit_docker_switch_tag_audit_event`) are untouched.

Deliberately preserved behaviors (explicitly not in scope of the fix): no `deactivated_at` filter
is added for the host (current behavior accepts deactivated hosts; changing that is a separate
product decision), and the `plugin_type` skip stays in the loop for site 2. One recorded
failure-mode change: the added `INNER JOIN`s make the handlers fail-closed on a row whose `host`
or `software_item` parent is missing — runtime connections enforce `foreign_keys=true`, so such
orphans cannot exist in practice; fail-closed is the correct direction if one ever did.

Accepted side effect: a mismatched-parent row (`host.tenant_id != software_item.tenant_id`, the
shape the sibling `assign_hosts` gap can produce) becomes unreachable by **both** tenants through
these actions, surfacing as the generic "no plugin assignments" error rather than a distinct
diagnostic. No detection/repair sweep is included: the producer is closed by the
web-api-queries spec, and the single live deployment is single-tenant, so no such row can exist
in practice. If one is ever suspected, a one-off join query comparing the two parents' tenant IDs
across `host_software_items` finds it.

### Alternative considered

Upfront host-ownership guard (one `TenantDb::find_by_id::<host::Entity>` check at each handler
entry, queries unchanged). Rejected with the user: it reintroduces a separately-droppable tenant
check next to unscoped queries — the exact mechanism class the Proxmox spec revision rejected.
Honest limit of the chosen fix: it makes each **existing** query tenant-scoped by construction
(no separately-droppable guard line), but a future edit adding a **new** query to these handlers
inherits nothing under either design — that residual is exactly what the CI gate noted in Out of
scope would close, and is why it stays noted there rather than silently dropped.

## Tests

Real in-memory SQLite with the workspace's established pattern — `Database::connect("sqlite::memory:")`

- `uptrakit_shared_db::migration::run_migrations(&db)` (as in
  `crates/ui/web-api-queries/src/queries/hosts.rs:488`). Manifest change, stated precisely: a **new**
  `[dev-dependencies]` entry — the crate has `uptrakit-shared-db` under `[dependencies]` only (no
  features) and its dev-deps today are just `tokio` + `uptrakit-command`; this adds the dual-entry
  pattern `web-api-queries/Cargo.toml` already uses (plain prod dep + featured dev dep). Workspace
  registration already exists, so `cargo deny` is unaffected:

```toml
# crates/plugins/releases/docker/Cargo.toml [dev-dependencies]
uptrakit-shared-db = { workspace = true, features = ["migration", "db-sqlite"] }
```

FK constraints are enforced in these DBs, so fixtures must seed the full parent chain per tenant:
`tenant` → `host` + `software_item` → `host_software_item` → `host_software_item_plugin`
(with `plugin_type = "releases.docker"` and a `#container`-suffixed `package_identifier` on at
least one row). For test case 4's two-qualifier fixture, seed a Docker plugin row under **each**
of the two `host_software_item` rows, with **distinct** `#container` suffixes — otherwise the
"all plugin rows rewritten" half of its assertion is vacuous and the cross-qualifier fan-out is
unobservable. No encrypted columns are involved, so `enable_plaintext_mode()` is not needed.

The production `AppStateSurfaceActionController` lives in `uptrakit-surface-proxy`, which the
Docker crate cannot depend on (dependency direction: surface-proxy → plugins). Tests instead define
a ~10-line test-local impl of the `SurfaceActionController` trait
(`crates/plugins/infrastructure/core/src/roles.rs:416` — three methods: `tenant_id`, `user_id`,
`tenant_db`) holding a `TenantDb::new(db.clone(), tenant_id)`; the same pattern infra-core's own
tests use (`descriptor.rs` test module — note that precedent stubs `tenant_db()` with
`unimplemented!()` since its suite never touches the DB; this spec's version returning a real
`TenantDb` is a superset, there is no worked real-DB example to copy verbatim). Cases (two tenants, A = caller, B = victim):

1. **Cross-tenant read blocked**: `current-tag` with tenant-B `host_id`/`software_item_id`
   returns empty `new_image_ref`.
2. **Cross-tenant write blocked**: `switch-tag` with tenant-B IDs returns the
   "no plugin assignments" error AND tenant B's `host_software_item_plugin.package_identifier`,
   `host_software_item.package_identifier`, and version columns are asserted unchanged.
3. **Same-tenant read works**: `current-tag` with tenant-A IDs returns the stored reference
   with the `#container` suffix stripped.
4. **Same-tenant write works**: `switch-tag` with tenant-A IDs rewrites all Docker plugin rows
   (suffix preserved per row), clears the item's version state, sets
   `update_category = "unknown"`, and skips a seeded non-Docker plugin row. Seed the
   production-realistic multi-container shape — two `host_software_item` rows for the pair with
   distinct `qualifier` values (the `qualifier` column _is_ the container name) — and assert
   set-level current behavior: all Docker plugin rows rewritten, **exactly one** of the two
   `host_software_item` rows updated (`.one()` picks arbitrarily; which one is not asserted).
   This documents the pre-existing multi-qualifier quirk (see Out of scope) instead of masking
   it with a single-row fixture.
5. **Mismatched-parent row blocked — both orientations (both-anchor regression guard)**: assert
   **both** parent-mismatch orientations, because a single orientation only pins one of the two
   single-anchor mutants. Seed two mismatched `host_software_item` + `host_software_item_plugin`
   rows (the shape the sibling `assign_hosts` gap can produce):
   - **5a — `host∈A`, `software_item∈B`**: catches a `host`-anchored single-parent mutant (drops the
     `software_item` filter → would leak).
   - **5b — `host∈B`, `software_item∈A`**: catches a `software_item`-anchored single-parent mutant
     that keeps the chosen anchor but drops the **added** `host` defense-in-depth join → would leak.
     This is the higher-value case: the chosen fix anchors on `software_item` (line 99), so the most
     likely future regression is someone removing the "extra" chained host join believing the anchor
     alone suffices. Only 5b keeps that mutant dead.

   For each orientation a tenant-A caller supplying that host/software-item pair reads empty
   (`current-tag`) and the `switch-tag` write is refused with the "no plugin assignments" error
   and no row mutated. One orientation alone leaves a single-anchor mutant alive, so both are
   required. This guard stays meaningful even after the `assign_hosts` fix lands: the
   `host.tenant_id == software_item.tenant_id` invariant is not enforceable at the DB layer (no
   cross-table FK/CHECK ties `host.tenant_id` to `software_item.tenant_id`), so a future regression of that invariant would re-open the hole it covers.

6. **Provider-origin invocation stays gated (descriptor assertion)**: assert that both Docker
   interactions (`switch-tag`, `current-tag`) declare a `required_action` and do **not** set
   `provider_invocable`. This pins the assumption the Service-WS entry-point analysis relies on:
   a system service (which skips the enrolled-tenant identity check) must keep being denied at
   the provider-origin gate (`proxy.rs:253`) before dispatch. No DB needed — pure descriptor
   inspection alongside the existing descriptor tests in `tests.rs`.

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
- No new workspace dependency (one new `[dev-dependencies]` entry for an already-workspace-registered
  crate — see Tests) → `cargo deny check` unaffected but run per gates.

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
- Promoting two-parent tenant scoping to a shared `TenantDb` helper
  (e.g. `find_via_two_tenant_joins`) and applying it uniformly to every single-parent reader of
  these join tables — notably `plugin_types_for_role`
  (`crates/ui/web-api-queries/src/queries/host_software_item_plugins/mod.rs:66`). Committed
  trigger: the **next** occurrence of this pattern in any crate promotes it to the shared helper;
  this spec deliberately keeps the Docker fix inline (matching the `services.rs:783` composition
  precedent) rather than introducing a new shared API for two call sites.
- Multi-qualifier `switch-tag` data-consistency quirk (pre-existing, not security): for a
  multi-container pair, the handler rewrites `package_identifier` on plugin rows for **all**
  qualifiers but updates exactly one arbitrary `host_software_item` row via `.one()`, leaving the
  other qualifiers' rows with stale version state and the old item-level `package_identifier`.
  Fixing it needs a `qualifier` param or an `.all()` fan-out (or driving site 3 off the plugin
  rows' `host_software_item_id` FK) — a behavior change this security fix must not smuggle in.
  Test case 4 pins the current behavior so the quirk stays visible.
- Host `deactivated_at` filtering in these actions (pre-existing behavior, separate decision).
- Web-api-queries host-assignment scoping (covered by
  `2026-07-11-web-api-queries-tenant-scoping-design.md`).
- Any CI gate that would structurally forbid `ctx.tenant_db().db()` in plugin crates (worth
  considering after the per-crate fixes land; noted, not designed here).
