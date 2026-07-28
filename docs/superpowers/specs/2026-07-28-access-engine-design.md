# M1.3 — `AccessEngine` (`uptrakit-controller-core`)

Date: 2026-07-28. Status: approved design, pending plan.

Third task of the authn/authz refactoring Milestone 1 (sources of truth:
`.superpowers/authn-and-authz-refactoring/`, esp. `07-decision-and-enforcement.md` §The PDP
interface / §Caching and invalidation, `09-resolved-questions.md` §Decision engine / §Cross-cutting
(authentication) #5, `11-task-breakdown.md` §M1.3, `12-test-plan.md` §C; and the sibling specs
`2026-07-28-access-types-core-design.md` (M1.1) + `2026-07-28-access-grant-storage-design.md`
(M1.2), whose surfaces this spec consumes). Owner-settled decisions are applied, not reopened.
**Implementation sequencing**: M1.1 and M1.2 land first — this task consumes
`uptrakit_shared_types::access` and the M1.2 engine-owned query module.

## Problem / goal

Build the single decision point: `AccessEngine` in `uptrakit-controller-core` — batched grant
resolution, a bounded cache with a 60 s TTL backstop, `authorize()`/`visibility()`, the
`ControllerMessage::AccessInvalidated` wire variant with cross-instance cache invalidation, and the
temporary Permission→Action shim. Library-complete and fully tested; **zero production
construction** — M1.4a builds the engine into request handling.

## Decisions locked during grilling (owner, 2026-07-28)

1. **Library-complete in M1.3; production wiring lands in M1.4a.** M1.3 ships the engine, the
   invalidation API, and the wire variant, all tested at module level. M1.4a constructs the engine
   in `AppState`, builds `AccessContext` in `require_auth`, and adds the
   `deliver_controller_event` arm. Until then the engine has no production caller and a received
   `AccessInvalidated` hits the existing `_ => warn` wildcard
   (`crates/ui/web-api/src/event_delivery.rs:342`) — tree green, dark-ship stated honestly.
2. **Shim = pure mapping fn, zero M1.3 production callers.** Exhaustively-tested
   `Permission → &'static [Action]` in controller-core; transitional consumers arrive M1.4+;
   deleted in M1.8.
3. (From the M1.2 round, inherited context:) all M1 grants carry `Selector::All` — the B9 write
   gate guarantees it — so target/selector matching is trivially satisfied in M1.

## Decisions from the contrarian round (2026-07-28, each verified against source)

1. **Invalidation = `invalidate_all()`.** moka's granular `invalidate_entries_if` requires builder
   opt-in (`support_invalidation_closures`) and returns `Result<PredicateId, PredicateError>` —
   under the no-`unwrap` invariant the only handlings are swallow-and-log (silently skipping an
   auth invalidation — a security regression) or fall back to a flush anyway. Grant/role mutations
   are rare admin operations; a full flush per event is provably correct, uses only `()`-returning
   moka API, and needs no closure support. The wire payload still carries subject lists
   (observability + forward-compat granular invalidation — a later optimization needs no protocol
   change).
2. **No `tenant_id` in the payload.** The mechanism ignores it, and its presence is a trap: a
   *global* (`tenant_id NULL`) user grant surfaces in **every** tenant's cache entry for that user
   (the M1.2 load filter is `tenant_id = ? OR tenant_id IS NULL`), so a receiver "optimized" to
   point-drop `(user, tenant)` would leave other-tenant entries authorized after a global-grant
   revoke until the TTL backstop. Payload is subject lists only; receivers always flush.
3. **Shim maps `ManageUsers → [users:manage]` only.** Mapping to both would re-merge the
   `users:manage`/`access:manage` split for every transitional consumer during the M1.4–M1.7
   window — the exact escalation the split exists to prevent. Durable both-ness for real users is
   carried by the M1.2 seed (`settings_manager` holds both). Guard test: the shim never emits
   `access:manage`.
4. **Wire blast radius includes the in-crate exhaustive guardrails.** `ControllerMessage` is
   `#[non_exhaustive]`, but same-crate matches are exhaustive and compile-break on the new variant:
   `classify_controller_message_variant` (`crates/shared/wire/src/tests.rs:2822`) and
   `variant_discriminant_name` (`tests.rs:2871`), plus `make_all_controller_message_variants` and
   the variant-catalog test. `AccessInvalidated` classifies **handler-owned / controller-internal**
   (the `TokenRevoked` group — NATS controller routing, never sent over the service WS).
5. **DB loads run direct (no single-flight).** Concurrent misses for one `(user, tenant)` may
   duplicate the 2-query load — accepted: idempotent reads, last-insert-wins, bounded by one
   principal's request parallelism, negligible at the deployment scale. moka's coalescing
   `try_get_with` exists only as a blocking sync closure (cannot `await` the DB load) or on
   `future::Cache`, which `09` resolved against (sync cache).

## Scope

In: pure decision types in `uptrakit-shared-types::access`; `AccessEngine` module + shim in
`uptrakit-controller-core`; `ControllerMessage::AccessInvalidated` in `uptrakit-wire` +
asyncapi regen + `docs/api/wire-protocol.md`; `moka` workspace registration; tests. Out (deferred
to the named tasks): engine construction in `AppState`, `AccessContext` middleware,
`deliver_controller_event` arm, action extractors (M1.4a); MCP/surfaces/inline-site enforcement +
live `DynamicActionRegistry` impls (M1.5); mutation-site invalidation calls + NATS publishing +
management API (M1.6a); `me`/claims swap (M1.7); shim + `Permission` deletion (M1.8); canonical
docs + ADR + CONTEXT.md vocabulary (M1.9); selector matching, `TargetRef::HostSoftwareItem`,
`Visibility::Filter` production + visibility-aware queries (M2.x).

## Consumed contracts (pinned)

- **M1.1** (`uptrakit_shared_types::access`): `Action` (+ `pub const` `actions::*` values — the
  shim's `&'static [Action]` arrays require const-constructible actions; stated M1.1 dependency),
  `ActionPattern` + `matches(&Action)` (incl. `system.`-exclusion and dynamic-namespace semantics),
  `Selector`, bounds.
- **M1.2** (`uptrakit_shared_db::access_grants`):
  `load_grants_for_principal(db, tenant_id: Uuid, user_id: Uuid, role_ids: &[Uuid]) -> Result<GrantLoad>`
  with `GrantLoad { grants: Vec<ResolvedGrant>, corrupt_skipped: usize }` — one query;
  **corrupt rows are loud-skipped, never call-fatal** (per-row `tracing::error!`; fail-closed under
  allow-only union — dropping an allow row only shrinks authority; whole-call errors are reserved
  for the query itself failing); the `corrupt_skipped` count feeds the engine's aggregate metric
  below. (The count-bearing return was folded into the M1.2 spec during this spec's review round,
  2026-07-28 — no cross-spec amendment remains outstanding.) `ResolvedGrant { id, tenant_id,
  subject, patterns: Vec<ActionPattern>, selector: Selector }`. Any drift found at plan time goes
  back into the M1.2 plan, not absorbed silently here.
- Dynamic-ness of a concrete `Action` (authorize step 1) derives from M1.1's existing accessors —
  `action.resource()` then `plugin_type().is_some() || surface_id().is_some()` (or a
  variant-sealed `Plugin(..)`/`Surface(..)` rest-pattern match, which M1.1 permits externally) —
  no M1.1 surface amendment needed.
- `user_roles` is `TenantScoped` (`crates/shared/db/src/entity/tenant_scoped.rs`); role-ID loading
  goes through `TenantDb` per the tenant-safe-query invariant.

## New pure types (`uptrakit-shared-types::access`, new `decision.rs`)

Per `07` §The PDP interface, placed with the other pure types (no DB deps; consumable by M2's
`TenantDb` visibility integration and tests):

```rust
#[non_exhaustive] pub enum TargetRef { Host(Uuid) }          // HostSoftwareItem variant lands M2.1
#[non_exhaustive] pub enum Decision { Allow, Deny(DenyReason) }
#[non_exhaustive] pub enum DenyReason { NoGrant, OutOfScope, OutsideSelector, UnknownAction }
#[non_exhaustive] pub enum Visibility {
    Full,
    Filter { tags: BTreeSet<Uuid>, hosts: BTreeSet<Uuid>, software: BTreeSet<Uuid>, items: BTreeSet<Uuid> },
    None,
}
```

All `#[non_exhaustive]` (evolving cross-crate enums — coding-standards default). `Debug, Clone,
PartialEq, Eq`; no serde (nothing here crosses a wire in M1 — the payload carries UUID vectors; the
`me` action list is M1.7's concern). `OutsideSelector` and `Filter` are typed now but unreachable /
never produced until M2 (types-complete, behavior-restricted — the M1.1/M1.2 pattern; rustdoc says
so). **`DenyReason` is diagnostic-internal**: it feeds traces/metrics and (M1.6b) deny audit
Events, never response bodies — D3's generic-403 rule; stated in the enum's rustdoc as normative.

## `AccessEngine` (`crates/ui/controller-core/src/access/`)

```text
src/access/
├── mod.rs      # AccessEngine, AccessContext, CachedAuthority, DynamicActionRegistry, error
└── shim.rs     # actions_for_permission (temporary — deleted in M1.8)
```

`lib.rs` gains `pub mod access;`. Error boundary per the standing rule:
`AccessEngineError` (thiserror; variants ~ `RoleResolution`, `GrantResolution`, plus
`impl_report_conversion!` from M1.2's `AccessGrantError`) with
`pub type Result<T> = std::result::Result<T, rootcause::Report<AccessEngineError>>` covering every
fn in the module.

### State and cache

```rust
pub struct AccessEngine {
    db: DatabaseConnection,
    cache: moka::sync::Cache<(Uuid /* tenant */, Uuid /* user */), Arc<CachedAuthority>>,  // key order = context() param order
    registry: Option<Arc<dyn DynamicActionRegistry>>,
    ttl: Duration,                       // default ACCESS_CACHE_TTL = 60 s
}
struct CachedAuthority { grants: Vec<ResolvedGrant>, loaded_at: tokio::time::Instant }

pub struct AccessContext {
    pub user_id: Uuid,
    pub tenant_id: Uuid,                 // carried for M1.6b deny-audit + logging, unused by evaluation
    authority: Arc<CachedAuthority>,
    scope: Option<Vec<ActionPattern>>,   // None = credential with no scope concept (pre-M3 session JWT)
}
// rustdoc: point-in-time snapshot. Per-request rebuild (the M1.4a contract) covers ordinary
// handlers; a long-lived streaming holder (SSE/WS) IS one request — it needs periodic
// re-`context()` or an invalidation-aware refresh, which per-request rebuild does not provide.
// Carried as an M1.4a/M1.5 residual for the streaming enforcement sites.
```

Constructor `AccessEngine::new(db)` + builder-style `with_registry(...)` / test-only TTL/capacity
overrides. Cache: `moka::sync::Cache` (per `09` §Cross-cutting #5), `max_capacity = 10_000`
entries, **no moka `time_to_live`**.

**TTL backstop is first-party read-time staleness, not moka expiry** — load-bearing rationale:
moka's expiry runs on its own quanta-based clock, which `tokio::time::advance` cannot move, so test
row C11 (`start_paused` self-heal) would hang or false-green against real moka TTL (ledger:
check which clock the code under test reads), and asserting moka's own expiry is a banned
upstream-behavior test. Instead `context()` treats a hit with
`loaded_at.elapsed() > ttl` as a miss and reloads. `tokio::time::Instant` tracks real time in
production runtimes and the paused clock in `start_paused` tests — the staleness logic is
first-party and testable. A stale entry that is never read again sits harmlessly until size
eviction: staleness only matters at read, and every read checks.

### Resolution (`context()`)

```rust
pub async fn context(&self, tenant_id: Uuid, user_id: Uuid,
                     scope: Option<Vec<ActionPattern>>) -> Result<AccessContext>
```

Fresh cache hit → wrap and return. Miss/stale → exactly **two** queries:

1. Role IDs: `user_roles` rows for `(tenant_id, user_id)` via `TenantDb` (`find::<UserRole>()` +
   user filter) — one SELECT regardless of role count.
2. Grants: M1.2's `load_grants_for_principal(db, tenant_id, user_id, &role_ids)` — the one batched
   `{user_id} ∪ role_ids` query (C13).

Insert `Arc<CachedAuthority>` into the cache, return the context. Errors propagate as `Err` —
never an empty-but-authorized context (C12; the HTTP-500 mapping is M1.4a's middleware, per the
"Database Error Propagation in Auth Handlers" rule). Nothing is cached on error. Concurrent misses
may duplicate the load (contrarian decision 5 — accepted, documented in rustdoc). `scope` is
per-request state layered on the cached core, per `07` §Caching.

**Corrupt-row observability** (discharges M1.2's "M1.3 MUST add an aggregate counter/metric"
directive): when the loader reports a non-zero corrupt-skip count, `context()` emits
`metrics::counter!("uptrakit_access_corrupt_grant_rows_skipped_total").increment(count)` — name
per the repo metric idiom (`uptrakit_` prefix, `_total` suffix; precedent
`crates/ui/surface-proxy/src/registry.rs:1064`), **deliberately label-free**, a stated deviation
from the every-counter-labeled precedent set: the only per-call candidates (user/tenant IDs) are
unbounded-cardinality labels — the github.rs counters label with closed sets
(`provider`/`status`/`reason`/`consumer`), and even the surface-proxy precedent's
`surface`/`interaction` labels are registration-time-bounded rather than per-principal — while
this counter has a single fixed cause, already encoded in its name, so a constant label would be
shape-mimicry without signal. Per-principal detail rides the
companion aggregate `tracing::warn!` naming the principal. A second counter,
`metrics::counter!("uptrakit_access_context_loads_total", "reason" => "miss" | "stale")`
(bounded two-value label, the github.rs `reason` style), increments on every cache-missing/stale
successful `context()` load (error-path loads do not increment — nothing was cached). A `"miss"`
alone cannot distinguish flush-induced reloads from cold-start loads, so its companion
`metrics::counter!("uptrakit_access_invalidations_total", "origin" => "local" | "remote")`
increments in `invalidate_subjects`/`apply_remote_invalidation` — correlating the two attributes
reload bursts to flushes, the evidence the M2 granular-invalidation deferral is gated on. Systemic corruption thus surfaces as a
counter, not only a flood of per-row error logs; the skipped rows are already absent from the
returned authority (fail-closed shrink; M1.2's contract).

### Decision (`authorize()` / `visibility()`)

Both pure, synchronous, in-memory (`07`: cheap enough per batch item). Normative check order —
pinned because it makes C5/C7 reachable and the deny reasons meaningful:

```rust
pub fn authorize(&self, ctx: &AccessContext, action: &Action, target: Option<&TargetRef>) -> Decision
```

1. **Dynamic-action registry** (`plugin.*` / `surface.*` resources only): unregistered →
   `Deny(UnknownAction)`. Registry seam:
   `pub trait DynamicActionRegistry: Send + Sync { fn is_registered(&self, action: &Action) -> bool; }`
   — engine-owned narrow typed boundary (ADR-0018 pattern). `registry: None` in M1.3 ⇒ **every**
   dynamic action denies (fail-closed: nothing is registered); M1.5 injects the live
   plugin-catalog + surface-registry impls. Built-in actions skip this step (parse-time catalog membership is
   their registration). C7's "unparseable string" half is a parse failure at the boundary
   (extractor/admission — M1.4a/M1.5); a parse error is a deny and no `Action` value ever exists.
2. **Grant match**: no grant pattern matches the action → `Deny(NoGrant)`.
3. **Token scope**: `scope = None` → vacuously true (C15 — no ceiling presented; pre-M3 session
   JWTs); `Some(patterns)` → no pattern matches the action → `Deny(OutOfScope)` (an empty `Some`
   vec denies everything — an OAuth token with empty scope narrows to nothing).
4. **Target/selector**: every M1 grant carries `Selector::All` (B9 write gate), so any `target` —
   including `Some(TargetRef::Host(_))` — is covered by whichever grant matched. The arm is
   written over the selector (match `All` → covered) so M2.1 extends rather than rewrites;
   `Deny(OutsideSelector)` is unreachable until then.

`Allow` iff all pass — grant ∧ scope ∧ selector, the `07` decision rule.

```rust
pub fn visibility(&self, ctx: &AccessContext, action: &Action) -> Visibility
```

Grants matching the action **and** surviving scope intersection: any → `Full` (their selectors are
all `All` in M1), none → `Visibility::None`. Selector-union → `Filter` construction is M2.3;
the match-then-union shape is written so M2 adds the union arm only.

### Invalidation API

```rust
pub fn invalidate_subjects(&self, user_ids: &[Uuid], role_ids: &[Uuid])   // local
pub fn apply_remote_invalidation(&self, payload: &AccessInvalidatedPayload)
```

Both flush the whole cache (`cache.invalidate_all()` — contrarian decision 1) and `tracing::debug!`
the subject lists. moka's `invalidate_all` guarantees subsequent `get`s do not return entries
inserted **before** the call — so the local "effect on next request" half of C10 holds for every
cached entry, with one documented race: a load already in flight when the flush fires may insert
pre-mutation authority *after* it, serving stale data until the TTL backstop. Bounded by the same
60 s envelope already accepted for a lost NATS event; the claim is stated as "reflected on the
next request absent a concurrent in-flight load, TTL-bounded otherwise" in the rustdoc, and C10's
single-threaded test asserts the former, not literal immediacy. (An insert-time
invalidation-generation stamp would close the race — rejected as disproportionate machinery for a
window the TTL already bounds.) The engine does **not**
publish NATS: mutation sites (M1.6a) call `invalidate_subjects` locally *and* publish
`ControllerMessage::AccessInvalidated` through the existing `publish_controller_event` path
(`crates/ui/controller-core/src/notification.rs:157`, `TokenRevoked` template → JetStream subject
`uptrakit.events.controller`); remote instances route the received payload to
`apply_remote_invalidation` via the M1.4a `deliver_controller_event` arm. The 60 s TTL backstop
covers lost events (C11).

### Permission→Action shim (`shim.rs`, deleted in M1.8)

```rust
pub fn actions_for_permission(permission: &Permission) -> &'static [Action]
```

Exhaustive match over the `05-action-model.md` mapping table (`ViewServices → [SERVICES_READ]`,
…); `ManageUsers → [actions::USERS_MANAGE]` **only** (contrarian decision 3);
`Other(_) → &[]` (fail-closed: unknown legacy string confers nothing). Slice-valued so
a variant can map to multiple actions without allocation; today every arm is 0-or-1 long. Rustdoc:
temporary M1 bridge, consumers are the M1.4–M1.7 transitional sites, **no site may gate
`access:manage` through the shim** (grant-admin authorization uses the `access:manage` extractor
directly), removal site M1.8. Zero production callers in M1.3 (`pub` item in a lib crate — not
dead code); stated honestly rather than pre-wiring a consumer.

## Wire: `ControllerMessage::AccessInvalidated`

Following the `TokenRevoked` sibling shape exactly (`TokenRevokedPayload` in
`crates/shared/wire/src/payloads.rs` — re-locate at plan time, line refs drift):

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct AccessInvalidatedPayload {
    pub user_ids: Vec<Uuid>,   // subjects whose grant rows / role assignments changed
    pub role_ids: Vec<Uuid>,   // roles whose grant rows changed
}
```

- Rustdoc (feeds the generated AsyncAPI, ADR-0029): controller→controller over NATS; receivers
  flush their whole access cache — the lists are diagnostic/forward-compat, **not** a promise of
  granular invalidation; no `tenant_id` by design (contrarian decision 2 — a global-grant revoke
  must invalidate every tenant's entries); both lists may be empty.
- `messages.rs`: `AccessInvalidated(AccessInvalidatedPayload)` variant with rustdoc.
- `limits.rs`: `pub const MAX_ACCESS_INVALIDATION_IDS: usize = 200;` (aligned with
  `MAX_GRANTS_PER_SUBJECT`; batch mutations touch ≤ 100 subjects). Explicit `WireValidate` arm in
  the `ControllerMessage` dispatcher (`wire_validate_impls.rs` — never the forward-compat
  catch-all, which would silently skip bounds) checking both vectors.
- In-crate exhaustive-guardrail updates (contrarian decision 4, all compile- or test-enforced):
  `classify_controller_message_variant` (handler-owned, `TokenRevoked` group),
  `variant_discriminant_name`, `make_all_controller_message_variants`, variant-catalog test —
  which hardcodes a total-count assertion ("exactly 38 entries", `tests.rs` ~3130) that must bump
  in the same change (registry-size-guard class: invisible to compile/clippy, fails only the full
  test run); plus
  an explicit test that `is_nats_publishable()` returns `true` for it (it must ride NATS; the
  deny-list is credential-oriented).
- `scripts/regen-asyncapi.sh` → commit `crates/shared/wire/asyncapi.yaml`
  (`asyncapi_yaml_is_up_to_date` golden gate, schema feature world).
- `docs/api/wire-protocol.md`: add an `access_invalidated` subsection under
  `## Controller–Controller Messages (NATS only)` (the section documenting
  `broadcast_admin_event`/`workload_claim_announcement`; note `TokenRevoked` itself is absent from
  this doc — the NATS-only section is the anchor, not a TokenRevoked entry). Standing rule: wire
  changes → AsyncAPI regen **and** wire-protocol doc.

## Manifest changes

- Root `[workspace.dependencies]`: `moka = "0.12"` — resolves 0.12.15, latest stable (verified
  crates.io 2026-07-28). Registered root-first per dependency policy.
- `crates/ui/controller-core/Cargo.toml` `[dependencies]`:
  `moka = { workspace = true, features = ["sync"] }` — the `sync` cache is a non-default feature
  in moka 0.12 (verify the exact feature name against the pinned crate at plan time); features
  declared at the consuming crate like the sibling `uuid`/`serde` convention.
- `[dependencies]`: `metrics = { workspace = true }` (already registered at root, `0.24`) — the
  corrupt-row aggregate counter; controller-core does not carry it today. Both new manifest lines
  follow the file's column-aligned `= { workspace = true }` formatting.
- `[dev-dependencies]`: `sea-orm` gains the `mock` feature (C13's transaction-log assertion);
  existing dev-deps already carry `uptrakit-shared-db` with `migration, db-sqlite` + sqlx-sqlite +
  tokio `macros, rt, time` (verified 2026-07-28) — no other test-infra additions.
- `cargo deny check` is a hard gate for the new dependency; **fallback** (owner-resolved upstream,
  `09` §Cross-cutting #5): if deny objects to moka's tree, hand-roll the bounded map with
  `parking_lot` + the same first-party TTL logic — the engine API and every test are
  cache-implementation-agnostic by construction, so the fallback swaps only the container.

## Tests

Controller-core engine tests run on in-memory SQLite through the real migrations (dev-deps
verified present); fixtures seed `tenants`/`users`/`user_roles`/`access_grants` via the M1.2
module (grants) and entities (principals). Anti-vacuity rules applied throughout: positive
cache-hit assertions accompany staleness tests, and deny tests assert the *specific* `DenyReason`
(standing rule: a 403 for the wrong reason is a red test).

§C rows owned by M1.3 — C7 unregistered-half only, C9 M1 arms only (C2/C3/C6/C8/C16, C9's
`Filter` arm, and C7's unparseable-string half — a boundary parse deny, M1.4a/M1.5 — are later
tasks'). Cache-implementation-agnosticism constraint: tests may assert first-party
TTL/staleness and invalidation effects, **never** eviction policy/ordering/`max_capacity`
behavior — that keeps the sanctioned `parking_lot` hand-roll fallback a drop-in swap.

- **C1**: allow via direct user grant; via role-inherited grant; via wildcard pattern — three
  separate cases.
- **C4**: zero matching grants → `Deny(NoGrant)`.
- **C5**: matching grant, `Some` scope excluding the action → `Deny(OutOfScope)`.
- **C7**: grant exists for `plugin.foo:manage`; engine without registry → `Deny(UnknownAction)`;
  with a stub registry not containing it → same; with a stub containing it → `Allow` (proves the
  seam, not the stub).
- **C9** (M1 arms): `All` grant ⇒ `Full`; no grant ⇒ `None`; scope-excluded grant ⇒ `None`.
- **C10** (mechanism level — endpoint-driven form completes in M1.4a/M1.6a): mutate grants via the
  M1.2 module, `invalidate_subjects` → next `context()` reflects the change;
  `apply_remote_invalidation(payload)` → same. Plus the positive control: without invalidation, a
  second `context()` within TTL serves the cached (stale) authority — proves caching exists, so
  the invalidation assertions cannot pass vacuously.
- **C11** (`start_paused`): cached entry; `tokio::time::advance(30 s)` → hit (no reload — assert
  via grant mutation not yet visible); `advance(31 s)` → stale, reload, mutation visible. Never a
  real sleep.
- **C12**: resolution against a broken DB (closed/absent-table connection) → `Err`; assert it is
  an error, not an empty-grants `Ok` (fail-closed, no `unwrap_or_default` shape anywhere). Row
  corruption is deliberately NOT this row's territory (M1.2 contract: loud-skip): a companion test
  seeds one corrupt + one valid grant row and asserts `context()` succeeds with only the valid
  grant's authority — the skip shrinks, never errors, never widens. The counter emissions
  themselves (corrupt-rows, context-loads, and invalidations alike) are thin wrappers over
  already-asserted state transitions and go unasserted — named residual, not silent. No metrics-recorder harness exists in-repo; the one counter-testability
  precedent (`github.rs`'s `RuntimeMetrics` `AtomicU64` mirror + `#[cfg(test)]` snapshot,
  `crates/ui/web-api/src/global_providers/github.rs:89`) is deliberately not adopted — a parallel
  mirror struct for a single fixed-cause counter is disproportionate machinery; revisit if the
  engine grows more metrics.
- **C13**: `MockDatabase` (sea-orm `mock` feature) principal with 5 roles → transaction log
  records exactly **2** statements (roles, grants) — no per-role queries, no stray existence
  probes. Correctness of the real SQL shape is covered by the SQLite tests; this row pins the
  round-trip count.
- **C14**: broad grant (`*:*`) + narrow scope (`[hosts:read]`) → only `hosts:read` allowed;
  narrow grant + broad scope → only the granted action allowed. Intersection both directions.
- **C15**: `scope: None` → grants alone (allow; no false 403); `scope: Some(vec![])` → every
  action `Deny(OutOfScope)`. Pins the no-scope-concept vs empty-scope distinction.
- Target arm: `All`-selector grant + `Some(TargetRef::Host(id))` → `Allow` (M1 semantics of the
  selector step).
- Shim: strum-exhaustive — every non-`Other` `Permission` variant maps to ≥ 1 action and every
  emitted action is a `CATALOG` member (asserts non-empty catalog first); `ManageUsers` maps to
  exactly `[USERS_MANAGE]` and the full shim output never contains `ACCESS_MANAGE`;
  `Other("anything")` → empty.

Wire crate: `WireValidate` at-limit accepted / over-limit rejected for both vectors;
`is_nats_publishable` true; the extended classification/discriminant/catalog tests; golden
asyncapi test green after regen (schema feature world — the schema-gated test module stays nested
inside a literal `#[cfg(test)]` per the clippy test-exemption rule).

## Verification gates

Crate-scoped lanes (pair clippy + test per feature world; run at baseline at plan time — never
pattern-guessed):

```sh
cargo fmt --all
cargo clippy --all-targets -p uptrakit-controller-core
cargo test  -p uptrakit-controller-core
cargo clippy --all-targets -p uptrakit-wire --features schema
cargo test  -p uptrakit-wire --features schema
# canonical workspace lanes (docs/development/quality-gates.md)
cargo check  --no-default-features --features db-sqlite
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo check  --all-features            # needs frontend/build (embed-frontend)
cargo clippy --all-targets --all-features
cargo test   --all-features
cargo deny check                       # new dep: moka
markdownlint --config .markdownlint.json '**/*.md'   # wire-protocol.md + this spec change
```

`scripts/regen-asyncapi.sh` runs in the same task as the wire change (golden gate).
`bash ci/verify_no_security_audit.sh`, `verify_typed_audit_actions.sh`,
`verify_handler_state_contract.sh`, `python3 ci/verify_db_access_policy.py`,
`cargo xtask audit-coverage-check` must pass untouched — no routes, no handlers, no
state-changing emit sites (the engine mutates nothing; migrations and management writes live in
M1.2/M1.6a). No OpenAPI regen (no endpoints). Commit scope: copy from
`git log --oneline -- crates/ui/controller-core` at plan time (do not guess from the directory
name).

## Documentation deliverables

- Rustdoc on every public item; `access/mod.rs` module doc: decision rule + normative check
  order, cache/TTL design (why not moka expiry), invalidation contract (flush-all semantics, who
  publishes), dark-ship state (constructed in M1.4a), pointer at
  `.superpowers/authn-and-authz-refactoring/07-decision-and-enforcement.md`.
- `docs/api/wire-protocol.md`: `access_invalidated` subsection under
  `## Controller–Controller Messages (NATS only)`.
- `crates/shared/wire/asyncapi.yaml`: regenerated, committed (never hand-edited — ADR-0029).
- **No canonical security-doc/ADR/CONTEXT.md updates in M1.3 — deliberate deferral**: the engine
  is dark until M1.4a; `docs/security/auth-and-authorization.md` + ADR + vocabulary land in M1.9
  per the milestone plan. This spec file is the M1.3 design record.

## Alternatives considered

- **moka `time_to_live` for the TTL backstop** — rejected: quanta-clock expiry is unreachable by
  `tokio::time::advance` (C11 untestable without real sleeps) and asserting it is an
  upstream-behavior test; first-party read-time staleness is testable and semantically equivalent.
- **`moka::future::Cache` + `try_get_with` (load coalescing)** — rejected: `09` resolved the sync
  cache; coalescing buys little at the target scale; the duplicate-load race is idempotent and
  documented. Revisit only with profiling evidence.
- **Granular `invalidate_entries_if`** — rejected (contrarian decision 1): opt-in closure support +
  fallible API vs the no-`unwrap` invariant; flush-all is correct and simple; payload keeps the
  subject lists so granularity remains a compatible future optimization.
- **`tenant_id` in the payload** — rejected (contrarian decision 2): unused by the mechanism and a
  cross-tenant stale-authority trap on global-grant revokes.
- **Shim mapping `ManageUsers` to both split actions** — rejected (contrarian decision 3):
  re-merges the split during the transition window; the seed carries legitimate both-ness.
- **Engine publishes NATS itself** — rejected: publishing belongs to mutation sites via the
  existing `publish_controller_event` path (single publisher pattern, `TokenRevoked` precedent);
  the engine stays a DB+cache component with no NATS dependency.
- **Hand-rolled `parking_lot` + timestamp-map cache** — held as the sanctioned fallback if
  `cargo deny` objects to moka (owner-resolved upstream), not the primary: bounded-LRU eviction is
  exactly the wheel moka ships.
- **New crate for the engine** — already rejected upstream (`09` §Decision engine #1: module in
  controller-core; workspace graph verified there).

## Deferred / out of scope (verbatim carriers)

Engine construction in `AppState` + `AccessContext` middleware + `deliver_controller_event`
`AccessInvalidated` arm + action-extractor macro + security schemes (M1.4a); live
`DynamicActionRegistry` impls (plugin catalog + surface registry) + MCP/surfaces/interactive-WS/
inline-site enforcement (M1.5); mutation-site `invalidate_subjects` + NATS publish calls +
grant/role management API + lockout guard + Stateful audit actions (M1.6a); catalog endpoint +
deny audit Events (M1.6b); `me`/JWT-claims swap + frontend action strings (M1.7); shim deletion +
`Permission`/extractor/enum-mirror-table removal (M1.8); canonical docs + ADR + CONTEXT.md
vocabulary (M1.9); selector/target matching beyond `All`, `TargetRef::HostSoftwareItem`,
`Visibility::Filter` production, visibility-aware `TenantDb` queries, granular cache invalidation
(M2.x, granular invalidation additionally gated on profiling evidence — now producible via the
`uptrakit_access_context_loads_total` counter). M1.6a residual: validate the "grant/role mutations
are rare" flush-all assumption against real role-assignment churn (bulk assignment flushes the
whole per-instance cache per mutation) before wiring the publishers.
