# M2.2 — Visibility-Aware Queries

**Date**: 2026-08-21
**Status**: Draft (pending review)
**Milestone**: M2.2 of the authn/authz refactoring series
(`.superpowers/authn-and-authz-refactoring/11-task-breakdown.md`, lines 173–179)
**Depends on**: M2.1 (landed — `Selector::covers()`, `Visibility::from_selectors()`,
split `authorize()`/`authorize_target()`, `AccessEngine::visibility()`)

## Problem

M2.1 gave the `AccessEngine` a pure `visibility(ctx, action) -> Visibility` that unions the
selectors of a user's matching grants into `Full` / `Filter { tags, hosts, software, items }` /
`None`. Nothing consumes it yet: every list query still returns all tenant rows. M2.3 will convert
REST enforcement sites to visibility-filtered queries with 404-not-403 semantics; it needs the
query-layer machinery first. M2.2 builds that machinery — `TenantDb` visible-query variants that
compile a `Visibility` into SQL conditions at query time — without touching any enforcement site.

Query-time compilation (rather than materializing host-id sets per request) is what makes re-tag
changes take effect immediately: the tag axis resolves against `host_tag_assignments` inside the
query itself, so an assignment added or removed between two requests is reflected on the very next
query with no cache invalidation. (The `AccessEngine`'s 60 s grant cache affects _grant_ changes
only; tag assignments are never cached.)

## Goals

- `find_visible`-family methods on `TenantDb` that append visibility conditions to
  tenant-filtered selects.
- Per-entity axis-column resolution declared like `TenantScoped::tenant_id_column()`.
- Tag/host/software/item selector axes compiled to sea_query `IN`-subquery / `IN`-list
  conditions — no raw SQL.
- `Visibility::None` short-circuit with **no DB round-trip**
  (`07-decision-and-enforcement.md` § Visibility → SQL).
- Query-level tests green on SQLite + Postgres, including re-tag immediacy and
  query-time/decision-time parity.

## Non-goals (M2.3 and later)

- Converting any enforcement site, handler, or route to visible queries.
- Lifting `SelectorPhaseGate` (non-`All` selector writes stay rejected; the M2.3 series
  invariant stays pinned by `ci/verify_access_enforcement_sites.sh`).
- 404 semantics, batch per-item not-found, fine checks in the three pinned
  `needs-fine-check` files.
- Any change to `authorize_target`, `Selector::covers()`, or the action catalog.
- Extending `ci/verify_db_access_policy.py` to flag missed visibility filters (deferred in
  `09-resolved-questions.md`, lines 253–258; trigger: first missed filter in review).

## Design

### Semantics: mirror `covers()` exactly

There is one visibility semantics with two evaluation strategies: `Selector::covers()`
(decision-time, per-target, `crates/shared/types/src/access/selector.rs:229-245`) and the SQL
compilation defined here (query-time, per-set). They must agree row-for-row, and `covers()` is the
normative side. Its deliberately fail-closed matrix — `Software`/`Items` never cover a bare
`TargetRef::Host` — dictates the compilation rule:

> An axis contributes a condition **only when the entity declares the matching column**;
> an undeclared axis contributes nothing.

A "uniform host-reachability" reading (granted item ⇒ its host row becomes visible in host lists)
was considered and rejected: it would show hosts that `authorize_target` denies for the same
grants — a query/decision divergence. In practice nothing is lost: the action catalog's
`SelectorSupport` (`crates/shared/types/src/access/catalog.rs`) already confines `Software`/`Items`
selectors to `HostAndSoftware`-level actions (`checks:trigger`, `updates:trigger`), which query
item-level entities.

### `HostScoped` trait

Declared in `uptrakit-tenant-db` (`crates/shared/tenant-db/`), sibling of `TenantScoped` — the
exact parallel the milestone asks for; the trait references no entities, so the crate's
dependency set (`sea-orm`, `uuid`) is unchanged:

```rust
/// Declares how an entity's rows resolve to the visibility axes.
///
/// `host_id_column` is mandatory: every visibility-queryable entity is
/// host-addressable (the `host` entity maps it to its own `Id`). The two
/// item-axis columns default to `None`; an axis whose column is `None`
/// contributes no condition — fail-closed, mirroring `Selector::covers()`.
///
/// An axis column may be the entity's **own primary key** when the entity
/// *is* the axis object: `host` answers `host_id_column` with `Id`, and
/// `host_software_item` answers `host_software_item_id_column` with `Id`.
/// Wire each axis method to whichever column holds that axis's id — own PK
/// or FK — and never to a similarly named column that holds something else.
pub trait HostScoped: EntityTrait {
    fn host_id_column() -> Self::Column;
    fn software_item_id_column() -> Option<Self::Column> {
        None
    }
    fn host_software_item_id_column() -> Option<Self::Column> {
        None
    }
}
```

### Entity impls (M2.2 set)

In `crates/shared/db/src/entity/host_scoped.rs` (sibling of `tenant_scoped.rs`). The set is the
entities behind the list/single-get/batch endpoints M2.3 converts; stragglers are added in M2.3
when their site converts (the trait bound forces it at compile time):

| Entity                      | `host_id_column` | `software_item_id_column` | `host_software_item_id_column` |
| --------------------------- | ---------------- | ------------------------- | ------------------------------ |
| `host`                      | `Id`             | —                         | —                              |
| `host_software_item`        | `HostId`         | `SoftwareItemId`          | `Id`                           |
| `host_software_item_plugin` | `HostId`         | —                         | —                              |
| `update_history`            | `HostId`         | —                         | —                              |

`update_history` and `host_software_item_plugin` deliberately do **not** declare the
software/items axes, even though both entities carry the columns. Two reasons. First, no designed
consumer: proposal `07-decision-and-enforcement.md:149-151` scopes those axes to the
`host_software_items` list ("which items may I update" affordance filtering) — neither of these
entities is that list, so declaring the axes would add reach with no consumer. Second, on
`update_history` the software axis would break covers() parity in the **permissive** direction:
`software_item_id` is `NOT NULL` while `host_software_item_id` is `Option<Uuid>`, and for a
`NULL`-hsi row (every pre-migration row — `m20260309_000003_unified_software_tracking.rs` leaves
`host_software_item_id` NULL on copy) the only constructible target is `TargetRef::Host`, which
`Selector::Software` does **not** cover (the fail-closed arm in `selector.rs`), while
`software_item_id IN (…)` would match. Host and tag axes (both keyed on `host_id_column`) are
these entities' entire M2.2 visibility surface; extending them to the item axes requires a
NULL-parity rule (e.g. `… AND host_software_item_id IS NOT NULL`) to be designed first — same
discipline as the `Option<host_id>` stragglers (see Deferred).

`host_software_item` and `host_software_item_plugin` are **not** `TenantScoped` (no `tenant_id`
column; tenant scoping goes through the `host` join) — they are served by the join variant below.

### Query API: extension trait on `TenantDb`

`TenantDb` is a foreign type in `uptrakit-shared-db`, and the tag-axis subquery needs the
`host_tag_assignment`/`host_tag` entity identifiers that live there — so the methods arrive via
an extension trait in a new `crates/shared/db/src/visibility.rs` module, re-exported from the
crate root next to the existing `TenantDb` re-export. (Building the subquery from `uptrakit-tenant-db`
would require string-based table identifiers — drift-prone and half-raw SQL — so the compiler
lives with the entities.) The trait-method shape — rather than free functions in the
`web-api-queries` builder style — keeps call-site parity with the inherent `find` family this
extends; see Alternatives.

```rust
pub trait TenantDbVisibleExt {
    /// Tenant-filtered select over `E`, narrowed to `visibility`.
    /// `None` ⇔ nothing is visible: return an empty list / 404 without
    /// touching the database.
    fn find_visible<E: TenantScoped + HostScoped>(
        &self,
        visibility: &Visibility,
    ) -> Option<Select<E>>;

    /// Single-row variant for 404-semantics sites (M2.3). Generic over the
    /// primary-key value type, matching `TenantDb::find_by_id`
    /// (`crates/shared/tenant-db/src/tenant_db.rs:40-46`) — every M2.2 entity
    /// uses a `Uuid` PK today, but the signature must not narrow the sibling
    /// it mirrors.
    fn find_visible_by_id<E: TenantScoped + HostScoped, V>(
        &self,
        id: V,
        visibility: &Visibility,
    ) -> Option<Select<E>>
    where
        V: Into<<E::PrimaryKey as PrimaryKeyTrait>::ValueType>;

    /// Visible variant of `find_via_tenant_join` for entities without a
    /// `tenant_id` column (`host_software_item`, `host_software_item_plugin`):
    /// tenant scoping via the join, visibility conditions on `Target`'s
    /// own columns.
    fn find_visible_via_tenant_join<Target: HostScoped, Scoped: TenantScoped>(
        &self,
        relation: RelationDef,
        visibility: &Visibility,
    ) -> Option<Select<Target>>;
}
```

`Option<Select<E>>` is the entire short-circuit contract: `Visibility::None` (and every other
nothing-visible outcome) returns `None`, and the compiler forces each caller to **handle** the
nothing-visible case — it cannot be forgotten. It can still be handled _wrongly_:
`.unwrap_or_else(|| tenant_db.find::<E>())` compiles and silently defeats visibility. That
bypass pattern is a review obligation for M2.3 site conversions and a named detection target of
the deferred `ci/verify_db_access_policy.py` visibility extension
(`uptrakit-def-db-policy-visibility-gate`). The methods are pure query builders: infallible, no
new error enum, no `Result`.

`find_visible_by_id` requires `TenantScoped`, so single-row 404-semantics sites for the
join-served entities (`host_software_item`, `host_software_item_plugin`) apply
`.filter(Column::Id.eq(id))` to the **unwrapped** select — same `let-else` shape as the
call-site example, since the join variant also returns `Option<Select<Target>>`:

```rust
let Some(query) = tenant_db.find_visible_via_tenant_join::<Target, Scoped>(rel, &visibility)
else {
    return Err(not_found());
};
let row = query.filter(Column::Id.eq(id)).one(tenant_db.db()).await?;
```

That composition is the reviewed idiom for those sites; a dedicated
`find_visible_by_id_via_tenant_join` is added only if M2.3 conversions show repeated use.

Call-site shape against the chain it replaces (`list_update_history`,
`crates/ui/web-api-queries/src/queries/update_history.rs:172-207` — filter → order → count →
offset/limit): one `let-else` prefix, then the existing chain continues on the unwrapped
`Select<E>` unchanged:

```rust
let Some(query) = tenant_db.find_visible::<update_history::Entity>(&visibility) else {
    // Visibility::None / nothing-contributes: no DB round-trip
    return Ok(PaginatedResponse::new(vec![], 0, pagination));
};
let query = query.filter(...); // existing filter/order chain, unchanged
let total = query.clone().count(tenant_db.db()).await?;
let rows = query.offset(offset).limit(limit).all(tenant_db.db()).await?;
```

Reachability note: under the current catalog, `software:read` — the action gating today's
`update_history` routes — is `SelectorSupport::None`, so its visibility is always `Full` or
`None`; the example's `Filter` arm is exercised by **cross-action** visibility. The designed
consumers of the software/items axes are item-level trigger visibility
(`visibility(ctx, updates:trigger)` / `checks:trigger` — "which items may I update" affordance
filtering, proposal `07-decision-and-enforcement.md:149-151`) and `hosts:read` visibility
applied to host-anchored lists. Which action each M2.3 site computes visibility for is an M2.3
mapping decision — see Security invariants.

Mutations intentionally have no visible variants: all targeted writes go through
`AccessEngine::authorize_target` at decision time (proposal 07), so visible `update_many` /
`delete_many` would be untested dead surface.

### Compilation rules

Internal to `visibility.rs`, one function producing the per-entity condition set:

- **`Visibility::Full`** → `Some(select)` unchanged — no extra condition.
- **`Visibility::None`** → `None` — no round-trip.
- **`Visibility::Filter { tags, hosts, software, items }`** → the axis conditions below are
  OR-ed under one `Condition::any()` and **appended to** (AND-ed with) the always-present
  tenant filter. Visibility only ever narrows a tenant-scoped select; it never replaces
  tenant scoping.
  - **hosts**: `E::host_id_column() IN (host_ids)`.
  - **tags**: `E::host_id_column() IN (subquery)` where the subquery is an uncorrelated
    sea_query `Query::select()` over `host_tag_assignments` inner-joined to `host_tags`, with
    `host_tags.tenant_id = <tenant>`, `host_tags.deactivated_at IS NULL`, and
    `host_tag_assignments.host_tag_id IN (tag_ids)`. The deactivation filter keeps parity with
    decision-time `load_host_tags` (`crates/ui/controller-core/src/access/mod.rs:341-353`) — a
    deactivated tag confers nothing on either path. This exact shape (joined, uncorrelated
    `IN`-subquery) has no verbatim workspace precedent; it composes two that exist:
    `in_subquery` over a single-table `Query::select()`
    (`crates/ui/web-api-queries/src/queries/update_history.rs:180-187`) and the sea_query
    inner-join subquery construction in `build_updatable_exists_subquery`
    (`crates/ui/web-api-queries/src/queries/software_items/crud.rs:492-520`, correlated
    `EXISTS` today). The milestone text's "EXISTS conditions" is satisfied semantically — the
    uncorrelated `IN`-subquery is the same semijoin.
  - **software**: `col IN (software_ids)` iff `E::software_item_id_column()` is `Some`;
    otherwise contributes nothing.
  - **items**: `col IN (item_ids)` iff `E::host_software_item_id_column()` is `Some`;
    otherwise contributes nothing.

Fail-closed edge rules:

1. An **empty axis set** contributes nothing (`from_selectors` does not produce empty axes
   today, but the compiler must not rely on that).
2. **No axis contributes any condition** (e.g. `Filter` with only `software` populated,
   queried against `host`) ⇒ return `None`. An empty `Condition::any()` must never degrade to
   an unrestricted select — this rule is what makes the compiler fail-closed standalone.
   This path emits a `tracing::debug!` naming the entity and the populated-but-undeclared
   axes — the silent-empty-list support case ("user sees nothing, grants look right") must be
   diagnosable from logs.
3. `Visibility` is `#[non_exhaustive]`: the match carries a wildcard arm that behaves as
   `None` (deny) with a `tracing::warn!`, following the workspace's wildcard-plus-warn
   precedent for `#[non_exhaustive]` enums
   (`crates/ui/web-api/src/api_error/mappings.rs:1030-1070`) — never a silent allow, never
   `unreachable!()`.

Bound-parameter note: axis unions are bounded by write-time validation (per-selector caps ×
`MAX_GRANTS_PER_SUBJECT = 200` grants), so a pathological union is theoretically ~20 000 ids per
axis against SQLite's 32 766-variable ceiling. Unreachable at this deployment's tenant sizes
(the union is further capped by ids that actually exist), but the compiler rustdoc must state
the assumption so a future bound change re-evaluates it rather than discovering it as an opaque
execute-time DB error.

### Why `Visibility::None` exists at the query layer

Three cases, in decreasing frequency: (1) cross-action filtering — a handler gated on one action
computes visibility for a different one (e.g. a list decorated with "can trigger update" per row;
zero `updates:trigger` grants ⇒ `None` ⇒ empty decoration, not 403); (2) scope-ceiling truncation
on API-token sessions for secondary actions; (3) totality/defense-in-depth — the query layer must
be total over the enum and fail closed even if a future call site forgets the coarse gate.

## Testing

**Done when**: query tests green on SQLite + Postgres.

### In-crate (fast feedback)

`crates/shared/db` behavioral tests under plain `#[cfg(test)]`, following the crate's own
`test_db()` helper precedent (`crates/shared/db/src/access_grants.rs:1084-1103`) — not the
`#[cfg(all(test, feature = "db-sqlite"))]` idiom, which belongs to `web-api-queries`/`web-api`:

- `test_db()` pattern: compile-and-run each axis against seeded rows on SQLite, plus the
  `None`/empty-`Condition` short-circuits (assert `find_visible` returns `None` — no query to run).

### Dual-backend integration matrix

New module `crates/core/integration-tests/tests/database/access_visibility.rs`, registered with a
`mod access_visibility;` line in the explicit module allowlist in
`crates/core/integration-tests/tests/database.rs` (a file under `tests/database/` is inert without
it), using `db_test!` (each case expands to `_sqlite` + `_postgres` `#[ignore]` variants; run via
`cargo test -p uptrakit-integration-tests --test database -- --ignored`). Every inclusion case
seeds **bystander rows** that must stay excluded — an assertion that only counts included rows is
vacuous.

Fixture invariant: for `host_software_item` — the one entity declaring both
`software_item_id_column` and `host_software_item_id_column` — fixtures must make the two axes
distinguishable: seed a row whose `software_item_id` is in the software axis while its `id` is
**not** in the items axis, and the mirror row; assert both directions. A fixture where the
covered software id and covered item id always ride the same row cannot detect a swapped
`HostScoped` column mapping — the most likely impl bug.

Cases:

1. **Per-axis include/exclude** — hosts, tags, software, items: covered rows returned,
   bystanders absent (per entity class: `host`, `update_history`, and
   `host_software_item_plugin` (join variant) for the host/tag axes; `host_software_item` via
   the join variant for all four).
2. **OR-composition** — multi-axis `Filter` returns the union.
3. **`Full` passthrough** — identical row set to plain `find`.
4. **`None` short-circuit** — returns `None`; treated as empty.
5. **No-axis-contributes ⇒ empty** — `Filter { software: {x} }` against `host` returns `None`.
6. **Deactivated-tag exclusion** — tag deactivated ⇒ its assignments confer nothing;
   reactivation restores visibility.
7. **Re-tag immediacy** — remove/add a `host_tag_assignment` between two queries; second query
   reflects it immediately (no cache in the path).
8. **Tenant isolation** — host/tag/item ids belonging to another tenant listed in the filter
   still yield nothing (visibility narrows, never widens, the tenant filter).
9. **Undeclared axes fail closed** — on both `update_history` and
   `host_software_item_plugin`: `Filter { software: {x} }` where `x` matches seeded rows'
   `software_item_id` returns `None`, not those rows (rule 2: axis undeclared) — pins the
   decision to leave the item axes undeclared (covers()-parity for legacy
   `NULL`-`host_software_item_id` rows; no-consumer scoping for the plugin table).
10. **Parity with `covers()`** — targeted cases: for a fixed grant/selector fixture and seeded
    rows, assert `find_visible` row membership ≡ `Selector::covers()` verdict per row (resolve
    `host_tags` the same way `load_host_tags` does, build the matching `TargetRef` per row:
    `host` and `update_history` rows map to `TargetRef::Host` — the full declared axis surface
    of both — and `host_software_item` rows to `TargetRef::HostSoftwareItem`).
    Parity runs against `covers()` directly — same normative matcher `authorize_target` uses —
    without bootstrapping an `AccessEngine` in DB tests. Scope limitation, stated: this pins
    the SQL compiler against the normative matcher, not the engine's tag-resolution _wiring_
    (`load_host_tags` vs the subquery); the two named divergence risks on that wiring —
    deactivated-tag handling and cross-tenant tag trust — are pinned by cases 6 and 8
    respectively. Engine-in-the-loop parity (through `AccessEngine::authorize_target` per row)
    belongs to M2.3, where the engine actually enters the request path.
11. **`find_visible_by_id` nothing-visible outcomes** — the two distinct outcomes M2.3's 404
    semantics rest on, asserted for a seeded in-tenant id: `None` (visibility `None` / no axis
    contributes) and `Some(select)` yielding zero rows (id exists but sits outside the
    filter).

Postgres notes: the fresh-container-per-test harness in `tests/database_helpers/db_providers.rs`
(each Postgres case starts its own testcontainer and a `test_{uuid}` database, dropped with the
returned guard) is reused unchanged; no constraint-violation paths are exercised (pure SELECTs),
so no aborted-transaction divergence applies.

## Security invariants

- Visibility conditions are appended to tenant-filtered selects — never a replacement for
  tenant scoping, and never applied to a non-tenant-scoped query path.
- Query-time and decision-time enforcement agree row-for-row for the `TargetRef` each entity
  maps to (Entity-impls table); the parity tests pin it. Entities declaring fewer axes are
  strictly **deny-side** relative to a `HostSoftwareItem` target — `covers()` may allow on an
  `Items`/`Software` grant where `find_visible` returns nothing; never the reverse.
- All fail-closed edges (unknown variant, undeclared axis, empty contribution) deny — no
  configuration reaches an unrestricted select from a `Filter` or unknown visibility.
- No enforcement site, `SelectorPhaseGate`, or `authorize_target` behavior changes in this
  milestone; the M2.3 series invariant stays pinned by `ci/verify_access_enforcement_sites.sh`.
- **Named non-invariant (M2.3 obligation)**: M2.2 provides mechanics only — actual narrowing at
  a site depends on _which action_ that site computes visibility for. A site computing
  visibility for a `SelectorSupport::None` action (e.g. `software:read`) always gets `Full`, so
  a host-anchored list gated that way would show rows for hosts the caller's `hosts:read`
  visibility excludes. M2.3's site-conversion mapping must route host-anchored lists through a
  selector-capable action's visibility (proposal `07-decision-and-enforcement.md` § Visibility →
  SQL); this spec does not close that gap and M2.3's spec must address it explicitly. Corollary:
  item-axis grants confer **no** visibility on `update_history` / `host_software_item_plugin`
  (host axis only) — an M2.3 site wanting item-anchored history visibility must design the
  NULL-parity rule first (Entity-impls section), not just declare the columns.
- No raw SQL anywhere: sea_query builders only (`clippy.toml`-enforced).

## Documentation deliverables

1. **`docs/development/coding-standards.md`** — Tenant-Safe Database Queries section
   (~lines 1489–1547): extend the method enumeration with `find_visible` /
   `find_visible_by_id` / `find_visible_via_tenant_join`, introduce `HostScoped` parallel to
   the existing `TenantScoped` framing, and add an anti-pattern row for the new leak class
   (plain `find` on a selector-capable entity where a visible variant is required —
   cross-visibility leak, distinct from cross-tenant leak). The row must also name the
   Option-bypass with its literal tokens (`find_visible` + `unwrap_or` — the prefix catches
   `unwrap_or_else` too — plus `map_or`) so a human reviewer can grep for it ahead of the
   deferred CI gate.
2. **`docs/development/testing.md`** — database integration test listing (~lines 390–420):
   add the `access_visibility.rs` row.
3. **`AGENTS.md`** (budget-gated; edit in place, pointer-style) — two pre-existing stale spots
   this change lands on: the crate manifest lacks a `crates/shared/tenant-db/` row and the
   `db/` row still claims `TenantDb`; the Tenant-Safe rule (lines 276–278) cites the stale
   path `crates/shared/db/src/tenant_db.rs` (real: `crates/shared/tenant-db/src/tenant_db.rs`).
   Fix both; keep the rule generic (pointer to coding-standards, no method enumeration).
4. **Rustdoc** on all new public items (`HostScoped`, the extension trait and its methods,
   module-level doc for `visibility.rs` stating the covers()-parity contract). The module doc
   must also state two caller obligations: (a) `AccessEngine::visibility` deliberately skips
   the dynamic-action registry gate, so `find_visible` alone does not guard dynamic
   (`plugin.*`/`surface.*`) resources; (b) tag _assignments_ resolve live in-query, but the tag
   ids come from grants read through the engine's 60 s cache — a _grant_ edit can lag up to
   60 s ("re-tag is immediate" must not be generalized to "visibility is immediate"). Plus the
   bind-parameter assumption from the compilation-rules note.

Explicitly unaffected (verified by sweep): `CONTEXT.md` (Selector/Visibility glossary entries
from M2.1 already cover M2.2; `HostScoped` is implementation, not domain),
`docs/security/auth-and-authorization.md` (no request-visible behavior changes — same scoping
precedent as M2.1), `docs/architecture/multi-tenancy.md`, `docs/development/database-migrations.md`,
wire/OpenAPI artifacts (no REST, wire, or schema-bearing type changes — no regen scripts run).

## Dependencies

- **M2.1 (landed)** — `Selector::covers()`, `Visibility`, `SelectorSupport`, split engine API
  (spec: `docs/superpowers/specs/2026-08-20-m2-1-selector-matcher-design.md`).
- **M2.3 (forward)** — consumes this machinery at enforcement sites; adds straggler
  `HostScoped` impls as sites convert; lifts `SelectorPhaseGate` when the series invariant is
  met. Cross-cycle bead wiring recorded at registration time.
- **Soft relation** — `uptrakit-def-b0379` (deferred: migrate `web-api-queries` wholesale to
  `TenantDb` query builders) is a downstream consumer of `find_visible*`/`HostScoped`, not a
  predecessor; related via `bd dep relate`, no blocking edge.
- No new external dependencies; no Cargo.toml changes (all code lands in crates with the
  needed deps already present).

## Alternatives considered

- **Always-false condition for `None`** — uniform `Select<E>` return, but executes a pointless
  round-trip; violates proposal 07. Rejected.
- **`VisibleSelect` wrapper enum** — more ceremony than `Option`; revisit only if M2.3 finds
  repeated empty-handling boilerplate. Rejected for now.
- **Uniform host-reachability for software/items axes** — diverges from `covers()`
  (fail-closed on bare hosts); would require changing M2.1's reviewed matrix. Rejected.
- **Two traits (`HostScoped` + `ItemScoped`)** — more type-level precision, two impl surfaces
  to sync and noisier bounds. Rejected.
- **Correlated `EXISTS` for the tag axis** — matches milestone wording literally, and its join
  construction is the workspace's one existing joined-subquery precedent
  (`build_updatable_exists_subquery`, `software_items/crud.rs:492-520`). Same semantics; rejected
  to keep one evaluation shape across all four axes — every axis yields a `col IN (…)` condition,
  uniformly OR-able under `Condition::any()`, rather than mixing `IN`-list and `EXISTS` forms in
  one combinator. Not rejected on precedent-dominance grounds (both forms coexist in the
  workspace).
- **Free functions instead of an extension trait** — `pub fn find_visible<E>(tenant_db, vis)`
  in `visibility.rs`, matching the free-function style of `web-api-queries` builders
  (`list_update_history` etc.). Those are domain query builders, though; `find_visible*`
  parallels `TenantDb`'s own inherent method family (`find` / `find_by_id` /
  `find_via_tenant_join`), and method syntax keeps M2.3 conversions mechanical and greppable
  (`tenant_db.find::<E>()` → `tenant_db.find_visible::<E>(vis)`). An extension trait is the only
  way to get method syntax on the foreign `TenantDb` type from `uptrakit-shared-db`. Rejected.
- **Property-based parity testing** — strongest guarantee, but new machinery for a finite
  4-axis matrix that targeted cases enumerate. Rejected.
- **Machinery in `uptrakit-tenant-db`** — the tag subquery would need string-based table
  identifiers (entities live downstream in `uptrakit-shared-db`). Only the `HostScoped` trait
  declaration lands there (dependency-free, true `TenantScoped` parallel) — a placement chosen
  for milestone-text conformance ("declared like `tenant_id_column()`") and symmetry, not a
  technical constraint: impls, compiler, and every bound site live in `uptrakit-shared-db`.
  The compiler and extension trait live with the entities. Rejected as full placement. Accepted consequence,
  stated as permanent API shape: `find_visible*` can never move upstream into `TenantDb`'s
  inherent methods (circular dep), so `find`/`find_by_id` stay inherent while `find_visible*`
  stays an extension trait — call sites import both, and every future `TenantDb` query helper
  picks a side by the same entity-dependency test.

## Deferred / out of scope

- **`ci/verify_db_access_policy.py` visibility extension** — flag handlers that run plain
  `find` where a visible variant is required. Deferred per proposal
  `09-resolved-questions.md:253-258`; trigger: first missed visibility filter found in review.
  (Bead: `uptrakit-def-db-policy-visibility-gate`.)
- **Straggler `HostScoped` impls** — `service_host`, `host_tag_assignment`, and
  `host_discovery_allowlist` are not implemented in M2.2; each is added in M2.3+ when (if) an
  enforcement site needs it, forced by the trait bound. The `Option<host_id>` entities
  (`notification_rule`, `software_ignore`) are **outside the current `HostScoped` shape
  entirely** — `host_id_column()` is mandatory and non-nullable, and for these entities
  `NULL` means _tenant-wide_ (a plain `host_id IN (…)` would hide exactly the rows that govern
  the caller's hosts). Activating them requires an explicit NULL-policy design decision (likely
  `… OR host_id IS NULL`, or a separate mechanism), not a mechanical impl — recorded in the
  bead so it cannot be "just impl'd" under M2.3 time pressure.
  (Bead: `uptrakit-def-host-scoped-stragglers`.)
