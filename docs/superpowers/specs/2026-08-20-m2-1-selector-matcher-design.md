# M2.1 — Selector Matcher and Write-Path Validation

**Date:** 2026-08-20
**Status:** Draft (pending `/review-spec`)
**Series:** authn/authz refactoring, Milestone 2.1 — source of truth:
`.superpowers/authn-and-authz-refactoring/` (esp. `05-action-model.md`, `06-grant-model.md`,
`07-decision-and-enforcement.md`, `11-task-breakdown.md`, `12-test-plan.md`).
**Depends on:** M1 complete (spec epic `uptrakit-spec-2026-07-28-access-engine`, closed).

## Problem / Goal

M1 froze the grant vocabulary with selectors restricted to `All`: `Selector`,
`SelectorSupport`, `Visibility`, and bounds are types-complete, but the write path rejects
every non-`All` selector via the B9 phase gate
(`crates/shared/db/src/access_grants.rs:267`), and the engine's selector matcher is a stub
(`crates/ui/controller-core/src/access/mod.rs:391`, `Selector::All => true, _ => false`).

M2.1 makes the selector machinery real — without opening the write path:

1. **Write-path validation** implements rules 3–5 (`Tags`/`Hosts` host-level,
   `Software`/`Items` item-level: capability levels, referent existence, bounds) with
   typed errors — but the B9 phase gate **stays as the final check**: a fully valid
   non-`All` grant is still rejected with `SelectorPhaseGate` until M2.3 lifts the gate
   in the same change that lands targeted enforcement (see the series invariant below).
2. **Matcher** implements the full target-coverage matrix as pure, table-testable
   functions, including the new `TargetRef::HostSoftwareItem` variant.
3. **`visibility()`** produces `Filter { tags, hosts, software, items }` via axis union,
   with `All` ⇒ `Full` and no matching grant ⇒ `None`.
4. **Engine integration**: targeted authorization evaluates non-`All` selectors against
   targets via a new async `authorize_target()`, loading the target host's tag set at
   decision time; the sync `authorize()` becomes explicitly targetless and keeps the
   proposal's **coarse-gate** semantics ("holds any grant for this action" — selectors
   are never consulted on the targetless path, per `07-decision-and-enforcement.md`).
   `authorize_target()` has no production caller in M2.1 (engine tests only); M2.3's
   fine checks are its first consumers.

**Series invariant (owner decision 2026-08-20, revised on contrarian pass 2; recorded
in the 07 and 11 clarification blocks):** no selector kind becomes writable before
**every enforcement site** for its admitting actions performs the fine check. The
invariant is stated over sites, not milestones — the enforcement-site inventory for the
five selector-capable actions (production targetless `authorize()` sites as of this
spec) is: the REST extractors (`middleware/action.rs`, incl. the `action_extractor!`
macro), `routes/users.rs`, MCP tool auth (`mcp/oauth/tool_auth.rs` — including
**write** tools such as `trigger_update`, which takes a caller-supplied `host_id`), and
surface invoke (`routes/surfaces.rs`, `enforce_required_action`). Concretely: M2.3
deletes the phase gate in the same change that lands `authorize_target()` fine checks
and visible queries across **all** of these sites — which pulls MCP-write and
surface-invoke fine checks into M2.3 and re-scopes M2.4 to list/visibility work only
(recorded as a dated clarification on `11-task-breakdown.md`, § Doc deliverables).
This rules out both failure modes the earlier draft had:
writable-but-dead grants (fail-closed targetless gate ⇒ every request 403s) and
writable-but-unenforced grants (coarse gate with no fine checks ⇒ a `Tags`-scoped
`hosts:delete` grant would confer tenant-wide delete).

**Non-goals** (subsequent milestones, unchanged): opening the write path / lifting the
phase gate (M2.3, with enforcement); compiling `Visibility` into SQL conditions /
`TenantDb::find_visible` (M2.2); REST 404-outside-visibility and batch partial-failure
enforcement (M2.3); MCP/surface **listing** (M2.4 — re-scoped to list/visibility work
only; MCP-write and surface-invoke fine checks belong to M2.3's gate-lifting change per
the series invariant above); `hosts.tags:manage` governance split
(M2.5); frontend/CLI selector affordances (M2.6). No REST contract change: grant
request/response types already carry `Selector` since M1; only error variants and their
`ApiError` mappings change.

## Authority note: `visibility()` timing

`11-task-breakdown.md` (M2.1 entry) is authoritative: `visibility()`'s full
implementation — producing `Filter`/`Full`/`None` from matching grants — is **M2.1**.
M2.2 merely _consumes_ the produced `Visibility` in queries. The code comments saying
"M2.3 adds the Filter arm" (`crates/shared/types/src/access/decision.rs` on `Visibility`,
`crates/ui/controller-core/src/access/mod.rs` on `visibility()`) are stale relative to the
task breakdown and are corrected by this milestone (see Doc deliverables).

## Design

### 1. Types (`uptrakit-shared-types::access`) — pure layer

#### 1a. `TargetRef::HostSoftwareItem`

Extend the `#[non_exhaustive]` enum in `decision.rs` per `07-decision-and-enforcement.md`:

```rust
pub enum TargetRef {
    Host(Uuid),
    HostSoftwareItem {
        /// `host_software_items.id` — the link row.
        id: Uuid,
        /// Owning host (`host_software_items.host_id`).
        host_id: Uuid,
        /// Catalog entry (`host_software_items.software_item_id`).
        software_item_id: Uuid,
    },
}
```

Callers construct it from an already-loaded link row; the engine never re-derives
`host_id`/`software_item_id`. Current `TargetRef` match sites are confined to
`controller-core/src/access/mod.rs` and `shared/types/src/access/` — the plan enumerates
every match site by repo-wide grep before implementation (non_exhaustive means downstream
wildcards compile silently; each wildcard arm must be classified deny-safe).

#### 1b. Pure coverage matcher — `Selector::covers()`

The M1 stub `selector_covers()` in controller-core moves into `shared/types` as a method
on `Selector` (the type that owns the semantics), taking the host tag set as plain data so
it stays pure and table-testable:

```rust
impl Selector {
    /// Does this selector cover `target`? `host_tags` is the *target host's*
    /// current active tag-id set (empty when tags not loaded).
    pub fn covers(&self, target: &TargetRef, host_tags: &BTreeSet<Uuid>) -> bool
}
```

The target is **required** — `covers()` is only ever evaluated on the targeted path
(`authorize_target()`, §3). The targetless `authorize()` is the coarse gate and never
consults selectors, so there is no `None` column: per `07-decision-and-enforcement.md`,
a targetless check means "holds any grant for this action" (list endpoints pair it with
`visibility()`; coarse gates like `mcp:use` have no target at all). The earlier draft's
fail-closed `None` column contradicted that contract and would have denied every
selector-scoped principal at the route extractors — removed on review.

Coverage matrix (normative, from `06-grant-model.md` / `07-decision-and-enforcement.md`):

| Selector \ target  | `Host(h)`             | `HostSoftwareItem { id, host_id, software_item_id }` |
| ------------------ | --------------------- | ---------------------------------------------------- |
| `All`              | `true`                | `true`                                               |
| `Hosts { ids }`    | `h ∈ ids`             | `host_id ∈ ids`                                      |
| `Tags { ids }`     | `host_tags ∩ ids ≠ ∅` | `host_tags ∩ ids ≠ ∅`                                |
| `Software { ids }` | `false`               | `software_item_id ∈ ids`                             |
| `Items { ids }`    | `false`               | `id ∈ ids`                                           |

Rows 3–5 × column `Host` encode C16 (autodiscovery): `checks:trigger` with
`TargetRef::Host` matches host-axis selectors only — a `Software`/`Items`-only grant can
never trigger discovery on any host.

#### 1c. Pure visibility union — `Visibility::from_selectors()`

A pure constructor next to `Visibility` in `decision.rs`; the engine's `visibility()`
delegates to it over the selectors of all grants that matched action + scope:

```rust
impl Visibility {
    /// Union the selectors of all matching grants: any `All` ⇒ `Full`;
    /// no selectors ⇒ `None`; otherwise OR each axis into `Filter`.
    pub fn from_selectors<'a>(selectors: impl Iterator<Item = &'a Selector>) -> Visibility
}
```

Axes union, never intersect (`Tags{a} ∪ Hosts{h}` ⇒ `Filter` with both sets populated).
`Filter` retains its existing `BTreeSet<Uuid>` fields; a produced `Filter` always has at
least one non-empty axis (empty-id selectors are unrepresentable post-write, rule 5b).

#### 1d. Rule 3 — pure selector-capability check

A pure helper in `shared/types::access` reusing the existing pattern-expansion machinery
(`ActionPattern::matched_catalog_actions()`, `SelectorSupport::admits()`):

```rust
/// Rule 3: a non-`All` selector is accepted only when EVERY possible match of
/// EVERY pattern admits it. A pattern that reaches the dynamic registry
/// (`reaches_dynamic()`) has possible matches of unknown selector support and
/// therefore rejects any non-`All` selector.
pub fn validate_selector_level(
    patterns: &[ActionPattern],
    selector: &Selector,
) -> Result<(), SelectorLevelError>
```

Semantics: `All` always passes. For non-`All`, every `(CatalogEntry, VerbEntry)` yielded
by every pattern's `matched_catalog_actions()` must satisfy
`verb_entry.selector_support.admits(selector)`; additionally any pattern with
`reaches_dynamic()` fails closed (dynamic actions are never selector-capable). This
yields B4 (any non-selector-capable match ⇒ reject, "split required"), B5 (item-level
selector × host-level-only action ⇒ reject), and B6 (`hosts.tags:manage` is
`SelectorSupport::None` in the catalog ⇒ reject) without special cases.

`SelectorLevelError` is a small typed enum (`thiserror`) naming the offending pattern and
the first non-admitting action. The write path wraps it verbatim as
`AccessGrantError::SelectorNotSupported(SelectorLevelError)`, consistent with the
existing leaf-wrapping variants (`Patterns(PatternSetError)`,
`Selector(SelectorValidationError)`).

### 2. Write path (`uptrakit-shared-db::access_grants`) — rules 3–5

`validate_write()` remains the single choke point for `insert_grant`/`update_grant`.
Rules 3–5 are fully implemented and land **in front of** the B9 phase gate, which stays
as the final selector check (series invariant, § Problem/Goal): an _invalid_ selector
now gets its accurate typed error (rules 3–5 below), and a _valid_ non-`All` selector
still gets `SelectorPhaseGate` — proof it passed every validation rule. The gate's
error message must say so explicitly — e.g. "selector validation passed; non-`All`
selectors are not yet enabled (lifts with M2.3 enforcement)" — so an operator reading
the 400 does not chase the preceding rules' diagnostics as the cause. M2.3 deletes
the gate arm, its variant, mapping, registry code, and B9 test in the same change that
lands targeted enforcement; nothing else about the write path changes then.

**Order of checks** (cheap-pure before DB): rule 5 (bounds, incl. new 5b) → rule 3
(level) → rule 4 (referent existence, DB) → B9 phase gate (any remaining non-`All`
selector). Rules 1–2 (pattern matchability, tenant-encoding) stay as in M1, evaluated
before selector rules as today.

#### Rule 5 — bounds (5a existing + 5b new)

- 5a: existing `Selector::validate()` limits (`MAX_SELECTOR_TAG_IDS = 32`,
  `MAX_SELECTOR_HOST_IDS/SOFTWARE_IDS/ITEM_IDS = 100`) — currently unreachable behind the
  phase gate; becomes live. Grant-level bounds (patterns/grant, pattern length,
  description, grants/subject) unchanged.
- 5b (**new, deviation — extension of the proposal's rule 5**): a non-`All` selector with
  an empty id list is rejected (`SelectorValidationError::EmptyIds`). Rationale: mirrors
  rule 1's "pattern must match ≥ 1 action" — a grant that provably matches nothing is a
  footgun. Owner-approved 2026-08-20.
- Canonicalization: id lists are sorted + deduplicated before validation and persistence
  (duplicates are not an error; bounds apply to the deduplicated count). Matches the
  `BTreeSet` shape of `Filter`.

#### Rule 3 — selector-capability level

`validate_write()` calls `validate_selector_level(patterns, selector)`; failure maps to a
new variant:

```rust
/// Rule 3: the selector kind is not admitted by every action the grant's
/// patterns can match.
#[error("selector not supported: {0}")]
SelectorNotSupported(SelectorLevelError),
```

(Leaf-wrapping shape per §1d — the diagnostic payload travels inside the wrapped error,
matching the file's existing `Patterns`/`Selector` variants.)

#### Rule 4 — referent existence (DB-backed, active-only)

New read-before-write queries in the `access_grants` module, all batch (`.is_in(ids)`,
single query per axis — no N+1), all scoped to the grant's tenant, all **active-only**
(owner decision 2026-08-20: you cannot build new authority on deactivated rows; a
referent that later dangles or deactivates simply matches nothing — write-time strictness
only):

| Axis       | Query                                                                                                                                                         |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Tags`     | `host_tags` WHERE `id IN ids` AND `tenant_id = grant.tenant` AND `deactivated_at IS NULL`                                                                     |
| `Hosts`    | `hosts` WHERE `id IN ids` AND `tenant_id = grant.tenant` AND `deactivated_at IS NULL`                                                                         |
| `Software` | `software_items` WHERE `id IN ids` AND `tenant_id = grant.tenant` AND `deactivated_at IS NULL`                                                                |
| `Items`    | `host_software_items` JOIN `hosts` WHERE `host_software_items.id IN ids` AND `hosts.tenant_id = grant.tenant` AND both `deactivated_at IS NULL` (link + host) |

Queries follow the tenant-safe convention: `TenantDb` helpers (or equivalent explicit
tenant filters inside this `shared/db` module) for the tenant-scoped entities;
`host_software_items` is a global table (no `tenant_id`) — tenant scoping goes through the
owning host join, consistent with the `find_via_tenant_join` convention. `software_items.enabled`
is an ops toggle, not lifecycle, and does not gate rule 4.

**Referent-tenant derivation** (owner decision 2026-08-20; corrects the earlier draft's
false "rule 4 always has a grant tenant" claim): the grant row's `tenant_id` alone does
not determine the referent tenant, because role-subject grants are always stored with
`tenant_id = NULL` regardless of plane (single-encoding rule, `06-grant-model.md`).
Rule 4 resolves the tenant as:

- **User-subject grant**: `grant.tenant_id`. Non-`All` selectors imply tenant-plane,
  selector-capable patterns (rule 3), so it is non-`NULL` per the tenant-encoding rules;
  a `NULL` here is an internal-invariant typed error, not a panic.
- **Role-subject grant**: the role row's `roles.tenant_id`. Custom role
  (`tenant_id = Some(T)`) ⇒ referents checked in `T` — the natural use case (a custom
  "web-servers operator" role scoped by `Tags`) works. Global/built-in role
  (`roles.tenant_id = NULL`) with a non-`All` selector ⇒ **rejected** with a new typed
  variant (a role that applies across tenants cannot reference tenant-specific rows):

```rust
/// A non-`All` selector on a grant whose subject is a global (cross-tenant)
/// role: selector referents are tenant-rows, so there is no tenant to
/// resolve them against.
#[error("non-All selectors are not allowed on global-role grants")]
SelectorOnGlobalRole,
```

Decision-time needs no change: role grants are loaded via the principal's `user_roles`
in the active tenant, and the tag lookup is already scoped to `ctx`'s tenant (§3).

Consequence worth stating (so the next agent doesn't rediscover the dead end): the seed
roles (`operator`, `software_manager`, `host_manager`, …) are all global, so they can
never be narrowed by a selector. The sanctioned path for "give Bob operator powers, but
only on web-tagged hosts" is a **direct user-subject grant** carrying the selector
(`06-grant-model.md` blesses these); a tenant-scoped custom role is the alternative when
the grant list should be reusable. A "clone seed role into a tenant custom role" CLI
affordance is a candidate for M2.6 (noted in the doc deliverable, not scoped here).

The set difference (requested − found) is reported in full:

```rust
/// Rule 4: selector references rows that do not exist as active rows in the
/// grant's tenant.
#[error("selector references {axis} ids that do not exist in this tenant")]
SelectorReferentsMissing { axis: SelectorAxis, missing: Vec<Uuid> },
```

`SelectorAxis` is a small `Copy` enum (`Tags | Hosts | Software | Items`) with `Display`,
in `shared/types::access` (also reused by diagnostics). One selector has exactly one axis,
so a single variant instance reports every missing id in one round-trip.

**Framing (documented as such in the auth doc):** rule 4 is authoring ergonomics — a
typo-catcher at write time — not a decision-time safety property: a referent that later
dangles or deactivates simply matches nothing, and nothing re-validates stored grants.
The part that protects the operator over time (surfacing dangling referents on grant
read) is D-series work scoped to M2.6. Two accepted consequences: `Hosts`/`Items`
selectors cannot be pre-provisioned before the host enrolls (tags are the sanctioned
pre-provisioning path — the docs already steer there), and the full `missing` list is
returned to the caller — acceptable, since `access:manage` is tenant-admin authority and
UUIDs are not guessable.

#### HTTP mapping

All three new variants (`SelectorNotSupported`, `SelectorReferentsMissing`,
`SelectorOnGlobalRole`) map to **400** in `api_error/mappings.rs`, consistent with the
existing `AccessGrantError` validation family
(`Patterns`/`PlaneMixing`/`TenantEncoding`/bounds). Rule 4 is a request-body validation
failure, not a resource lookup — never 404. `SelectorPhaseGate`'s mapping arm, registry
code, and `MAPPING_REVIEW.md` row all **stay** (the gate lifts in M2.3, which removes
them). No OpenAPI schema change (error bodies are the generic `ApiError` shape), so no
`regen-api.sh` run is required. Updated in the same change, each pinned by its own test:

- `crates/ui/web-api/src/api_error/MAPPING_REVIEW.md` — add rows for the three new
  variants; annotate the `SelectorPhaseGate` row "lifts in M2.3"
  (`mapping_review_md_exists_and_has_all_variant_names`).
- The error-code registry — add the three new codes in both `ALL_IMPL_CODES`
  (`api_error/tests.rs`) and `src/api_error/code_registry.txt`
  (`code_registry_golden_file_sorted_and_complete`).

### 3. Engine (`uptrakit-controller-core::access`)

#### Split authorization API — sync `authorize()` + async `authorize_target()`

**Deviation (owner-approved 2026-08-20, recorded as a dated clarification block in
`07-decision-and-enforcement.md`):** the proposal specifies a single
`authorize(ctx, action, target: Option<&TargetRef>) -> Decision`, "pure, synchronous,
in-memory". Decision-time tag resolution (the chosen uncached-lookup design, §3) makes the
targeted path DB-backed and fallible, so the API splits instead of going async
everywhere:

- **`pub fn authorize(&self, ctx, action) -> Decision`** — the target param is
  **removed**. Stays pure, synchronous, infallible, and is now explicitly the **coarse
  gate** of `07-decision-and-enforcement.md`: "holds any grant for this action" —
  grant-match + scope only, selectors never consulted (the coverage matrix has no
  targetless column, §1b). This is genuinely identical to today's observable
  `authorize(ctx, action, None)` behavior on real data (the write gate means only `All`
  selectors exist), and it is what keeps the route extractors correct when non-`All`
  grants appear in M2.3. All current call sites pass `None`. Production sites
  (confirmed by repo-wide grep; the `event_delivery.rs` and
  `surface_action_registry.rs` matches sit inside `#[cfg(test)]` modules and are
  test-only migrations): `mcp/auth.rs` ×2, `mcp/oauth/tool_auth.rs`,
  `middleware/action.rs` ×3 (incl. the `action_extractor!` macro),
  `web-api/visibility.rs`, `routes/surfaces.rs` (`enforce_required_action`),
  `routes/users.rs`. Of these, `mcp/oauth/tool_auth.rs` and `routes/surfaces.rs`
  coarsely gate **targeted** operations (MCP `trigger_update`, surface invoke) — the
  exact sites the series invariant pins to M2.3's gate-lifting change. The migration
  is mechanically dropping the `, None` argument; the plan enumerates every site.
- **`pub async fn authorize_target(&self, ctx, action, target: &TargetRef) ->
Result<Decision>`** — new, using the module's existing error boundary: the
  `Result<T> = std::result::Result<T, Report<AccessEngineError>>` alias, with a new
  `AccessEngineError` variant for tag-resolution DB failures — no parallel error enum.
  The variant is populated with an explicit `.context_transform(...)` at the tag-lookup
  call site (the codebase's established pattern for a second `DbErr`-driven variant in
  one target enum, e.g. `controller-runtime/src/boot/nats.rs`), **not** a second
  `impl_report_conversion!(sea_orm::DbErr => …)` invocation — the module's existing
  `impl_report_conversion!(sea_orm::DbErr => AccessEngineError::RoleResolution)` is a
  blanket trait impl fixed per `(Source, Target)` pair, so a second invocation for the
  same pair fails to compile (E0119). Loads the target host's active tag set
  only when at least one candidate grant carries a `Tags` selector, then delegates to a
  private sync core shared with `authorize()`. The type system now makes "targeted check
  without tag resolution" unrepresentable — no caller can pass a target into the
  tag-blind sync path. **No production caller in M2.1**: engine tests are the only
  consumers; M2.3's handler fine checks are the first production call sites (series
  invariant, § Problem/Goal).

#### Enforcement-site inventory gate (new, M2.1)

The series invariant spans a multi-milestone gap, and a prose promise is not a guard —
this spec's own review missed two sites on a hand grep. M2.1 therefore lands a
mechanical check while the phase gate still protects the window: a CI script
(`ci/verify_*` idiom; exact name in the plan) that enumerates all **production**
(non-`#[cfg(test)]`) call sites of the targetless `authorize()` and diffs them against
a committed inventory file. Each inventory row is annotated either `coarse-only` (the
actions flowing through the site are never selector-capable — e.g. `mcp:use`,
`users:manage`, `system.settings:manage`) or `needs-fine-check` (selector-capable
actions can flow through it: the extractor macro sites, MCP tool auth, surface invoke).
Any new targetless call site fails CI until deliberately classified. Three details
make the check real rather than aspirational (contrarian pass 3):

- The internal `allowed_actions()` self-call
  (`controller-core/src/access/mod.rs`) is a production targetless site and gets an
  explicit `coarse-only` row (its coarse semantics are intentional, §3
  `allowed_actions()`).
- Test detection must treat any `cfg` predicate **containing** `test` as test code
  (`surface_action_registry.rs` uses `#[cfg(all(test, feature = "db-sqlite"))]`, which
  a literal `#[cfg(test)]` match would misclassify as production).
- The script asserts the **coupling** to the gate itself: if `SelectorPhaseGate` is
  absent from `crates/shared/db/src/access_grants.rs` while any `needs-fine-check` row
  remains, the check fails. Only with this arm is "M2.3's gate lift gains a
  machine-checkable precondition" literally true — clearing every `needs-fine-check`
  annotation (by landing the fine check or migrating the site to `authorize_target()`)
  becomes a CI-enforced prerequisite of deleting the gate, not a convention.

Normative check order unchanged in the shared core (dynamic registry → grant match →
scope → target/selector). The target/selector step of `authorize_target()`:

1. Collect grants matching action + scope (as today).
2. If any such grant has a non-`All` selector and at least one selector is `Tags`, load
   the target host's active tag set once per call, **tenant-scoped like every other
   engine query**: `host_tag_assignments` (global join table, no `tenant_id`) joined to
   `host_tags` via the `TenantDb`/`find_via_tenant_join` convention against the
   context's tenant, filtered by `host_id = target.host_id()` AND
   `host_tags.deactivated_at IS NULL` — the engine is the PDP and must not trust the
   caller to have pre-checked that the target host belongs to `ctx`'s tenant; a
   cross-tenant `host_id` simply resolves to an empty tag set (and the host-axis checks
   already can't match foreign rows the grant couldn't reference, rule 4). Resolved at
   decision time, uncached. Rationale (corrected on review): not a freshness guarantee —
   grants themselves sit behind a 60 s TTL cache, so tags being fresher buys no
   end-to-end immediacy — but the **simplest correct implementation** at this
   deployment's scale: no tag-cache invalidation machinery, no new `AccessInvalidated`
   publishers. The recorded escalation path, if profiling ever warrants it, is caching
   the host→tag map behind the same invalidation channel as grants (M2.5 owns the tag
   mutation sites). (`TargetRef` gains a `host_id()` accessor: `Host(h)` ⇒ `h`,
   `HostSoftwareItem { host_id, .. }` ⇒ `host_id`.)
3. Allow iff any matching grant's `selector.covers(target, &host_tags)`; otherwise
   `Deny(DenyReason::OutsideSelector)` (variant exists since M1, becomes reachable).

DB errors during the tag lookup propagate as errors (fail closed, HTTP 500 at the
surface) — never silently treated as "no tags".

**Batch constraint for M2.3 (recorded here, implemented there):** one tag lookup per
`authorize_target()` call is acceptable in M2.1 (single-target enforcement paths and
engine tests only). M2.3's batch partial-failure enforcement MUST NOT loop
`authorize_target()` over N targets with N tag queries — it batch-loads the tag sets for
all target hosts in one query (`host_id IN (…)`) and feeds the shared sync core per item,
per the workspace no-N+1 rule. The M2.3 spec inherits this as a hard requirement.

#### `allowed_actions()`

With the coarse-gate semantics the earlier draft's "under-reporting goes live" concern
dissolves: `allowed_actions()` derives from targetless matching, which now correctly
reports any held action — including one held only via a non-`All` selector grant (the
principal _does_ hold the action, coarsely; per-target nuance is what `visibility()` /
D13 summaries express). M2.1 must still: correct the stale "M2.1 (selectors) must
revisit this" comment on `allowed_actions()` (`controller-core/src/access/mod.rs`) to
say coarse semantics are intentional and per-action visibility summaries are M2.6/D13,
and **replace** the M1 pinning test
`allowed_actions_under_reports_non_all_selector_grants_until_m2_1` with one asserting
the coarse behavior (a non-`All`-selector grant's action IS reported). Richer per-action
scope reporting stays scoped to M2.6 (D13) per `11-task-breakdown.md`.

#### `visibility()`

Replaces the M1 `Full`/`None`-only logic: collect selectors of grants matching action +
scope, return `Visibility::from_selectors(...)`. Pure over the cached grant rows — no DB
work here; tag resolution stays a query-time concern for M2.2 consumers.

### 4. Testing

Per project rules: success + failure paths, non-vacuous fixtures — every reject fixture
isolates exactly one rule (mentally delete that check: the fixture must go red; a fixture
tripping two rules pins neither), and every rejection asserts the **specific typed
variant**, never merely "some error".

**Pure table tests** (`shared/types`, co-located, extending the `admits_full_matrix`
tuple-table idiom):

- `covers()` full matrix — all 10 cells of the table in §1b, including the item-level
  matrix rows named by the task: exact `(host, item)` pair allowed via `Items`; sibling
  item on the same host denied for `Items`; same software on another host denied for
  `Items` but allowed for `Software`; host outside `Hosts`/tag-set denied (C3/C6 pure
  half); `Software`/`Items` never cover a bare `Host` target (C16 pure half).
- `Visibility::from_selectors` union table — `All` anywhere ⇒ `Full`; empty ⇒ `None`;
  mixed axes ⇒ `Filter` with correct OR-ed sets (C9); duplicate ids collapse.
- `validate_selector_level` — B4/B5/B6 rows plus the dynamic-pattern reject; positive
  rows for each of the five selector-capable actions at its admitted levels.
- Rule 5b + canonicalization — empty ids rejected per axis; duplicates dedupe below
  bounds threshold (a 33-entry `Tags` list with 2 duplicates passes; 33 distinct fails).

**Write-path tests** (`shared/db::access_grants`, SQLite-migration fixture, test names
keyed to plan rows): `b9_non_all_selectors_phase_gated` is **kept** and extended — it
now asserts the gate fires _last_: a fully valid non-`All` grant (passes rules 3–5)
still yields `SelectorPhaseGate`, which doubles as the "acceptance" assertion for each
selector kind on each admitting action (reaching the gate proves no validation rule
fired; true round-trips land with M2.3's gate lift). New `b4_`/`b5_`/`b6_` (level
rejections), `b7_` (foreign-tenant referent AND deactivated referent rejected —
separate fixtures, one rule each), `b8_` extended to the now-live selector id-count
bounds and 5b (canonicalization asserted at the validation layer). Role-subject
coverage (referent-tenant derivation, §2 rule 4): custom-role grant with a selector
referencing rows in the role's tenant reaches the phase gate (rules pass); custom-role
grant referencing another tenant's rows rejected (`SelectorReferentsMissing`);
global-role grant with any non-`All` selector rejected (`SelectorOnGlobalRole`);
global-role grant with `All` still accepted end-to-end (regression guard).

**Engine tests** (`controller-core::access`, existing SQLite fixture; non-`All` grant
rows are inserted directly by the fixture, below `validate_write()` — the write gate
does not constrain engine tests), targeted rows driven through `authorize_target()`: C2
(union across grants jointly covering action ∪ axes), C3 (target matrix allows), C6
(`Deny(OutsideSelector)` per axis), C9 (`visibility()` end-to-end through the engine),
C16 (autodiscovery host-axis-only), plus decision-time tag reads (retag host → next
`authorize_target()` reflects it, no invalidation event needed) and the tag-lookup
DB-error fail-closed path. Sync `authorize()` keeps its existing targetless suite
(signature migration only) plus a new coarse-gate pinning test: a principal holding an
action only via a non-`All` selector grant is **allowed** targetless (07 semantics —
mentally flip the gate to fail-closed and the test goes red). The replaced
`allowed_actions` test (§3) asserts the matching coarse reporting.

### 5. Documentation deliverables

- `docs/security/auth-and-authorization.md` — selector-capability levels now validated,
  write-path rules 3–5 (incl. active-only referents, min-1, canonicalization, and the
  rule-4 "ergonomics, not safety" framing, §2), the phase gate's new position (last,
  lifts in M2.3 per the series invariant), decision-time tag resolution, the split
  coarse `authorize()` / targeted `authorize_target()` API, and the seed-role
  narrowing guidance (direct user-subject grants, §2). (Canonical doc for the auth
  subsystem.) The plan must sweep the whole `docs/` tree (not hand-list) for
  selector-status claims — "selectors are restricted to `All`", "non-`All` selectors
  rejected", `SelectorPhaseGate` — and update every hit to the new state (validated but
  still gated; gate lifts with M2.3 enforcement) in the same change.
- `.superpowers/authn-and-authz-refactoring/06-grant-model.md` — dated
  "Clarification (2026-08-20, from M2.1 spec)" block: rule 5 gains the min-1 bound; rule
  4 "exist" means active (non-deactivated) rows; rule 4's referent tenant derives from
  `grant.tenant_id` (user-subject) or `roles.tenant_id` (role-subject), and global-role
  grants reject non-`All` selectors. Source-doc correction per the
  folklore-propagation ledger rule — the proposal is the first thing the next agent reads.
- `.superpowers/authn-and-authz-refactoring/07-decision-and-enforcement.md` — dated
  "Clarification (2026-08-20, from M2.1 spec)" block: the single "pure, synchronous"
  `authorize(ctx, action, target)` is superseded by the split API (sync targetless
  coarse `authorize()` + async fallible `authorize_target()`, §3); the **series
  invariant** (no selector kind writable before every enforcement site for its
  admitting actions fine-checks — stated over the enforcement-site inventory in
  § Problem/Goal, not over milestones; the phase gate lifts in M2.3's site-complete
  enforcement change); and the M2.3 batch-tag-load constraint.
- `.superpowers/authn-and-authz-refactoring/11-task-breakdown.md` — dated
  "Clarification (2026-08-20, from M2.1 spec)" block: M2.3's gate-lifting change is
  **site-complete** — it owns the MCP-write fine checks (`trigger_update`,
  `mcp/oauth/tool_auth.rs`) and the surface-invoke fine checks
  (`routes/surfaces.rs`), alongside the REST handler checks; M2.4 is re-scoped to
  list/visibility work only (visible queries for MCP list-shaped tools, surface
  listing filters). Corrects two gaps: M2.4's "surface invoke fine-checks targets"
  line was unordered relative to M2.3's gate lift, and MCP write tools were covered
  by no milestone at all. Ownership is additionally tracked as a deferred bead
  (`uptrakit-def-m23-targeted-site-fine-checks`, § summary) so it cannot be lost
  between spec cycles.
- `crates/ui/web-api/src/api_error/MAPPING_REVIEW.md` — variant rows updated with the
  mapping change (§2 HTTP mapping; pinned by test).
- Stale comment corrections (same change as the code they annotate):
  `shared/types/src/access/decision.rs` (`Visibility` "never produced until M2.3" →
  produced from M2.1), `controller-core/src/access/mod.rs` (`visibility()` "M2.3 adds the
  Filter arm" → implemented in M2.1; `allowed_actions()` "M2.1 (selectors) must revisit
  this" → coarse semantics intentional, per-action summaries M2.6/D13, with the pinning
  test replaced, §3), the B9 phase-gate comment block in `access_grants.rs` updated to
  its new position and M2.3 lift (not removed), and `decision.rs` "`HostSoftwareItem`
  lands in M2.1" line (now landed).
- `CONTEXT.md` — add glossary entries for **Selector** (grant-scoping axis:
  All/Tags/Hosts/Software/Items) and **Visibility** (Full/Filter/None read-scope result)
  if absent at implementation time; **TargetRef** stays code-level (not domain
  vocabulary).
- No new ADR: the architectural decisions (action-string grants, central engine,
  fail-closed) are recorded in ADR-0039; M2.1's choices (pure matcher + decision-time tag
  lookup, active-only referents) are direct consequences of the proposal's normative
  design and are recorded here and in the proposal clarification block — none is
  hard-to-reverse enough to clear the ADR bar.
- No wire/asyncapi change; no OpenAPI regen (no contract change); no frontend change
  (M2.6).

## Alternatives considered

- **Matcher queries DB itself** (async `selector_covers`): rejected — kills the mandated
  pure table tests and moves query logic into `shared/types`.
- **`TargetRef` carries the host tag set**: rejected — contradicts the proposal's type
  definition and pushes a DB read onto every call site instead of one lazy engine lookup.
- **`authorize()` becomes async + `Result` everywhere** (single method): rejected —
  every targetless caller (all current call sites, incl. the `action_extractor!` macro)
  pays async/error ceremony for a tag lookup it can never trigger, and the proposal's
  sync-core contract is abandoned instead of narrowed. The split API (§3) preserves it
  for the targetless path.
- **Caller-supplied tag set** (sync `authorize()` gains a `host_tags` param): rejected —
  every future targeted call site must remember the load step; forgetting silently
  fails-closed on `Tags` grants and the type system can't catch it. `authorize_target()`
  makes the pairing unrepresentable-wrong.
- **Referent existence = row exists in any state**: rejected — permits grants that can
  never match (deactivated referents), violating the least-surprise/fail-closed bias.
- **Defer engine integration to M2.2** (pure matcher only in M2.1): rejected — the task
  breakdown puts `visibility()` full implementation in M2.1 and the test plan tags
  section C "selectors M2.1"; M2.2 stays a pure query-compilation milestone.
- **Open the write path in M2.1** (the earlier draft's scope; contrarian-review fatal
  finding, owner re-cut 2026-08-20): rejected in both variants. With a fail-closed
  targetless gate, selector grants are writable but dead — every request 403s at the
  route extractors (which stay targetless until M2.3) and stays dead after M2.3 too,
  contradicting 07's coarse-gate contract; a net UX regression versus the honest
  write-time rejection. With the coarse gate and no fine checks, selector grants are
  writable but unenforced — a `Tags`-scoped `hosts:delete` grant confers tenant-wide
  delete until M2.3: an escalation window. Hence the series invariant and keeping the
  phase gate until M2.3.
- **Cache the host→tag map in M2.1**: rejected — invalidation machinery (new
  `AccessInvalidated` publishers on tag mutation) for no measurable win at this
  deployment's scale; recorded as the escalation path in §3 instead.
- **Per-action (or per-selector-kind) phase-gate lift** (contrarian pass 2, owner
  decision 2026-08-20): rejected — lifting the gate only for actions whose full site
  set is fine-checked would let e.g. `hosts:read` ship ahead of `updates:trigger`, but
  adds per-action gate state and test surface for incremental shipping this single
  deployment does not need. The site-complete M2.3 lift keeps one gate, one lift; the
  inventory gate (§3) supplies the machine-checked precondition instead.

## Dependencies

- **M1 (hard, landed)**: `uptrakit-spec-2026-07-28-access-engine` — closed; all M2.1
  anchor points (`SelectorSupport::admits`, `matched_catalog_actions`, bounds constants,
  `validate_write` choke point, `DenyReason::OutsideSelector`) verified present at spec
  time.
- Cross-cycle sweep against open spec/plan epics: recorded in the beads registration
  (§ summary); no open epic touches `crates/shared/types/src/access/`,
  `crates/shared/db/src/access_grants.rs`, or `controller-core/src/access/`.

## Deferred / out of scope

The remaining M2.x milestones (M2.2 visibility-aware queries, M2.3 REST enforcement,
M2.4 MCP/surface listing — list/visibility only after the re-scope, M2.5 tag
governance, M2.6 UI affordances) are the proposal series' own backlog
(`11-task-breakdown.md`), each activated via its own `/spec` cycle — not deferral beads
of this spec. One exception, filed to pin ownership discovered on review: the MCP-write
(`trigger_update`) and surface-invoke fine checks were covered by no milestone and are
now owned by M2.3's gate-lifting change — tracked as deferred bead
`uptrakit-def-m23-targeted-site-fine-checks` (activated when the M2.3 spec cycle
starts) so the assignment survives outside this spec's prose.

## Success criteria (done when)

- Pure matcher/union/level table tests green (`shared/types`), full matrices covered.
- Write path rejects every rule 3–5 violation with the specific typed variant, and a
  fully valid non-`All` grant still hits `SelectorPhaseGate` last (gate-position
  assertion); B-row tests green. The gate itself lifts in M2.3, not here.
- Engine C-row tests green, incl. C16 and decision-time tag reads; `visibility()`
  produces `Filter`/`Full`/`None` correctly; coarse-gate pinning test green (targetless
  allow on a selector-only grant).
- Split API landed: every `authorize(ctx, action, None)` call site migrated to the
  targetless signature; targeted paths go through `authorize_target()` only (no
  production caller yet — engine tests only).
- Enforcement-site inventory gate landed: CI script + committed annotated inventory of
  production targetless `authorize()` sites, with `mcp/oauth/tool_auth.rs` and
  `routes/surfaces.rs` marked `needs-fine-check` — the machine-checked precondition
  M2.3's gate lift must clear.
- Quality gates pass: fmt, clippy (both feature sets), tests, plus the doc deliverables
  above landed in the same change.
