# M1.4a — REST action-extractor + security-scheme scaffolding

Date: 2026-07-29. Status: approved design, pending plan.

Fourth task of the authn/authz refactoring Milestone 1 (sources of truth:
`.superpowers/authn-and-authz-refactoring/`, esp. `07-decision-and-enforcement.md` §Per-request
resolution / §REST route extractors / §Native security declarations, `09-resolved-questions.md`
§Decision engine and enforcement, `11-task-breakdown.md` §M1.4a, `12-test-plan.md` §D; and the
sibling specs `2026-07-28-access-types-core-design.md` (M1.1),
`2026-07-28-access-grant-storage-design.md` (M1.2), `2026-07-28-access-engine-design.md` (M1.3),
whose surfaces this spec consumes). Owner-settled decisions are applied, not reopened.
**Implementation sequencing**: M1.1–M1.3 have landed (verified on `main` at spec time:
`ecbb03639`/`9d320a242`/`11feea67e`/`68a318334`/`4d8955050`); this task is the first production
construction of the engine. M1.4b (the mechanical route sweep) repoints against the contract this
task freezes.

## Problem / goal

Wire the `AccessEngine` into REST request handling and freeze the conversion contract the M1.4b
sweep will apply mechanically: engine constructed in `AppState`; `AccessContext` built once per
authenticated request in `require_auth` and stored in request extensions; a new action-extractor
macro (same `FromRequestParts` shape and 401/403 semantics as `permission_extractor!`) checking
`engine.authorize(ctx, action, None)`; native OpenAPI security schemes (`oauth2` with a
catalog-generated scope dictionary + `developer_token` bearer) registered beside the legacy
`bearer_token`; the `deliver_controller_event` arm for `ControllerMessage::AccessInvalidated`; a
new CI gate asserting each converted operation's `oauth2` scope list matches its handler's action
extractor; and the hosts route family converted as the reference. `permission_extractor!` stays
compiled; the routes it still guards are untouched.

## Decisions locked during grilling (owner, 2026-07-29)

1. **Scheme transition keeps all three registrations.** M1.4a registers `oauth2` +
   `developer_token` **alongside** `bearer_token`. ~196 unconverted operations still declare
   `security(("bearer_token" = []))` after this task; deleting the scheme now would leave dangling
   scheme references in `openapi.json`. The `bearer_token` registration is deleted in M1.4b's
   final sub-PR (recorded there as a follow-through obligation). The task-breakdown word
   "replacing" is satisfied across M1.4a+M1.4b as a unit.
2. **CI gate is Python.** The check pairs each operation's multi-line `#[utoipa::path(...)]`
   attribute with the following handler signature and joins through two macro tables — structured
   parsing beyond the comfortable reach of the bash+rg `verify_*` family (a known failure class:
   wrapped attributes split across a line-oriented pipeline). Precedent for Python gates in both
   CI and pre-push exists (`ci/check_plugin_semantic_boundary.py` runs in `.husky/pre-push` and
   `.github/workflows/ci.yml`; `ci/verify_db_access_policy.py` walks `routes/` pairing utoipa
   attrs with handlers).
3. **Deny observability (trace + counter) ships in the extractor now.** Resolved Q6
   (`09-resolved-questions.md` §Decision engine #6: ordinary denies = debug trace + counter
   metric) is implemented nowhere yet; the extractor's 403 path is its single natural site.
   Counter labeled by the bounded `DenyReason` (matches the engine's existing label style:
   `uptrakit_access_context_loads_total{reason}` at
   `crates/ui/controller-core/src/access/mod.rs:161`). The audit-**Event** tier for sensitive
   actions remains M1.6b.

## Verified current state (2026-07-29, live tree)

- `AccessEngine` is library-complete with **zero production construction** (`AccessEngine::new`
  appears only in its own test module; no `AppState` field, no middleware use). The engine module
  doc (`crates/ui/controller-core/src/access/mod.rs:15-25`) explicitly assigns to M1.4a: `AppState`
  construction, `require_auth` context build, and the `deliver_controller_event` arm.
- Engine API (`crates/ui/controller-core/src/access/mod.rs`): `new(db: DatabaseConnection)` (:121),
  `with_registry` (:134), **async** `context(&self, tenant_id: Uuid, user_id: Uuid, scope:
  Option<Vec<ActionPattern>>) -> Result<AccessContext>` (:148), sync `authorize(&self, ctx, action,
  target) -> Decision` (:227), `invalidate_subjects` (:315), `apply_remote_invalidation(&self,
  payload: &AccessInvalidatedPayload)` (:330). `AccessContext` (:107) has **no derives** — it must
  gain `Clone` to live in axum request extensions.
- `require_auth` (`crates/ui/web-api/src/middleware/require_auth.rs:126`) authenticates JWT or
  `upk_` API token and inserts `AuthenticatedUser` (+ `AuthenticatedApiTokenId`, `SetupRequired`)
  into extensions at :196-202. No engine wiring.
- `permission_extractor!` (`crates/ui/web-api/src/middleware/permission.rs:35-85`) generates 36
  `Can*` tuple structs `(pub AuthenticatedUser)` with `new()` test bypass; 401 when the
  `AuthenticatedUser` extension is missing, 403 on `!user.has_permission(perm)`, both via
  `error_response(StatusCode, &str) -> Response`.
- Security scheme: single `bearer_token` `SecurityScheme::Http` registered by `SecurityAddon`
  (`impl utoipa::Modify`, `crates/ui/web-api/src/router.rs:480-496`). Operations hand-write
  `extensions(("x-required-permission" = json!(...)))` + `security(("bearer_token" = []))`
  (163 extension occurrences across 50 files under `routes/`, 202 `#[utoipa::path]` operations —
  counts measured at spec time; re-grep, never cite these numbers as current).
- Hosts family (`crates/ui/web-api/src/routes/hosts.rs`): 6 operations —
  `list_hosts`/`get_host` (`CanViewHosts`), `update_host` (`CanUpdateHosts`), `deactivate_host` +
  `batch_hosts` (`CanDeactivateHosts`), `discover_host` (`CanTriggerChecks`).
- Catalog consts for the family exist and the shim agrees
  (`crates/ui/controller-core/src/access/shim.rs:38-43`): `actions::HOSTS_READ`,
  `actions::HOSTS_UPDATE`, `actions::HOSTS_DELETE`, `actions::CHECKS_TRIGGER`.
- utoipa 5.5.0 (pinned) verified against crate source: `SecurityScheme::OAuth2`,
  `OAuth2::new(flows)` / `with_description`, `Flow::AuthorizationCode`,
  `AuthorizationCode::new(authorization_url, token_url, scopes)` /
  `with_refresh_url`, and `Scopes: FromIterator<(I, I)>` — the scope dictionary can be built by
  iterating `CATALOG` (`uptrakit_shared_types::access::CATALOG`, entries carry
  `VerbEntry { action_str, description, .. }`).
- Live AS endpoints already exist (MCP OAuth): `/oauth/authorize`
  (`routes/oauth/authorize.rs:41`), `/api/v1/oauth/token` (`routes/oauth/token.rs:43`), RFC 8414
  metadata. The flows object points at real URLs today; no M3 dependency.
- `ControllerMessage::AccessInvalidated(AccessInvalidatedPayload)` exists
  (`crates/shared/wire/src/messages.rs:279`); no delivery arm — a received event hits the
  `deliver_controller_event` wildcard warn (`crates/ui/web-api/src/event_delivery.rs:342`).
  `ControllerResources` (`event_delivery.rs:24-39`) is built at `nats_transport.rs:276` from
  `NatsConsumerConfig` (`nats_transport.rs:51-68`), which is filled from `app_state` at
  `crates/core/controller-runtime/src/boot/serve.rs:178`.
- `AppState` is constructed at **two** sites: `AppStateBuilder::build()`
  (`crates/ui/web-api/src/app_state.rs:914`, literal at :1003) and the test harness literal
  (`crates/ui/web-api/src/test_harness/mod.rs`, `build_test_state_with_plugin_ops`, literal at
  ~:495). The plan must re-grep `AppState {` workspace-wide before freezing its edit list.
- No existing CI script touches `x-required-permission` (grep over `ci/`, `.github/`, `.husky/`,
  `scripts/`: zero hits) — the new gate is net-new.
- `metrics` is already a `uptrakit-web-api` dependency (Cargo.toml:86).

## Scope

In: `Clone` derive on `AccessContext` (controller-core); `access_engine` field on `AppState` +
`AccessState` sub-state; context build in `require_auth`; new `action_extractor!` macro + the four
hosts-family extractors; `SecurityAddon` scheme additions; hosts.rs reference conversion +
`./scripts/regen-api.sh` artifacts; `AccessInvalidated` delivery arm
(`event_delivery`/`ControllerResources`/`NatsConsumerConfig`/`serve.rs`); new
`ci/verify_action_security_declarations.py` + unittest + CI/pre-push wiring; harness tests
(D1/D2/D3 subset on hosts ×2 credentials, immediate-effect grant change); doc touches listed
below.

Out (deferred to the named tasks): every other route family, `x-action-dynamic` on the seven
surface wrappers, `bearer_token` scheme deletion, extension `rg` sweep (M1.4b); MCP/surfaces/
inline sites + live `DynamicActionRegistry` (M1.5 — the engine is constructed here with **no**
registry, so dynamic actions deny, which no REST extractor exercises); mutation-site invalidation
calls + NATS publishing + management API + deny audit Events (M1.6a/b); claims/`me`/frontend
(M1.7); `Permission` + `permission_extractor!` deletion (M1.8); canonical doc rewrite + ADR
(M1.9); visibility/selectors (M2.x). Streaming holders (SSE/WS) keep their current auth model
until M1.5 — the `AccessContext` doc's long-lived-holder residual is restated, not resolved, here.

## Consumed contracts (pinned)

- **M1.3 engine**: signatures quoted above. `context()` errors ⇒ HTTP 500 (standing "DB errors in
  auth/authz handlers must propagate as 500" rule). `authorize` is pure/sync; per-request
  evaluation is cheap.
- **Scope term (test C15 semantics)**: every credential `require_auth` accepts today (session JWT —
  no `scope` claim until M3 — and legacy `upk_` API tokens — no scopes until M4) passes
  `scope: None`: vacuously-true scope term, authority = grants alone, behavior-equivalent window
  per `07` §The PDP interface. `Some(vec![])` (deny-all ceiling) must never be produced here.
- **M1.2 storage**: seeded roles/grants exist after migrations; the TestApp first registered user
  holds the built-in roles, so engine resolution grants it the hosts-family actions. Direct grant
  manipulation in tests goes through `uptrakit_shared_db::access_grants` (`insert_grant`,
  `delete_grant`, `NewGrant`, `GrantSubject`) — the plan copies exact call shapes from that
  module's own tests (`crates/shared/db/src/access_grants.rs` test module), never paraphrases.
- **M1.1 catalog**: `CATALOG` / `actions::*` consts; `Action`/`ActionPattern` re-exported from
  `uptrakit_shared_types::access`.

## Design

### 1. `AccessContext` becomes `Clone` (controller-core)

`#[derive(Clone)]` on `AccessContext` (`crates/ui/controller-core/src/access/mod.rs:107`) — all
fields are `Uuid`/`Arc`/`Option<Vec<ActionPattern>>`; the clone is cheap (Arc bump + small vec).
Required because `axum` `Extensions::insert` needs `Clone + Send + Sync + 'static`, and the
extractor clones it out of `parts.extensions`. No other derive is added (no `Debug` — keep the
type's surface minimal; nothing formats it).

### 2. `AppState` wiring + `AccessState` sub-state

- New field `pub access_engine: Arc<uptrakit_controller_core::access::AccessEngine>` on
  `AppState`. Constructed inside `AppStateBuilder::build()` from the connection the builder
  already holds (`AccessEngine::new(db.clone())`, **no** `with_registry` — M1.5 injects one) and
  in the test-harness literal the same way. No new builder parameter: the engine is derived
  state, like the audit emitter. The plan re-runs `grep -rn "AppState {" crates/` and covers
  every literal.
- New sub-state following the `PluginOpsState` precedent (`app_state.rs:136` + `FromRef` at
  :1241): `pub struct AccessState(pub Arc<AccessEngine>);` +
  `impl FromRef<Arc<AppState>> for AccessState`. This is how the extractor reaches the engine —
  app-scoped handles live in state per the sub-state doctrine
  (`crates/ui/web-api/AGENTS.md`), not smuggled through request extensions. The sub-state list in
  `crates/ui/web-api/AGENTS.md` gains the entry.

### 3. `require_auth` builds `AccessContext`

In `require_auth` (`middleware/require_auth.rs`), after successful authentication and the
setup-gate, immediately before the existing extension inserts (:196-202):

- `state.access_engine.context(state.default_tenant_id, auth_user.user_id, None).await`
  — tenant is the deployment's single active tenant (`AppState.default_tenant_id`, the same value
  every handler uses today); scope is `None` per the pinned scope term.
- `Ok(ctx)` ⇒ `req.extensions_mut().insert(ctx)` beside `AuthenticatedUser`.
- `Err(report)` ⇒ `tracing::error!(error = %report, "access context resolution failed")` +
  `error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")` — the auth-path
  DB-failure rule; never `unwrap_or`-degrade to an empty context (that would be a silent
  authority wipe on one branch and fail-open on none — 500 is the only honest outcome).
- Cost: one moka read per request on cache hit; two batched queries per (user, tenant) per 60 s
  TTL window on miss (M1.3 measured contract). Applied to **all** `require_auth` traffic
  including not-yet-converted routes — this is intentional (M1.4b converts consumers, not the
  producer) and is what makes the immediate-effect harness test meaningful on the reference
  family alone.

`AuthenticatedUser.permissions` and the old JWT-claims path are untouched — old extractors on
unconverted routes keep working from the same middleware pass.

### 4. `action_extractor!` macro (new file `crates/ui/web-api/src/middleware/action.rs`)

Same ergonomic shape as `permission_extractor!` — explicit `Name => actions::CONST` table (macro
naming rule: `Can{Verb}{Resource}` derived from the action string; `macro_rules!` cannot
case-convert, so names are written out, as today):

```text
action_extractor! {
    CanReadHosts      => actions::HOSTS_READ,
    CanUpdateHosts    => actions::HOSTS_UPDATE,
    CanDeleteHosts    => actions::HOSTS_DELETE,
    CanTriggerChecks  => actions::CHECKS_TRIGGER,
}
```

Generated per name (mirroring `permission.rs:35-85`, differences called out):

- `#[derive(Debug)] pub struct $name(pub AuthenticatedUser);` + `pub fn new(user) -> Self` test
  bypass — same tuple payload, so converted handlers change only the type path.
- `impl<S> FromRequestParts<S> for $name where S: Send + Sync, AccessState: FromRef<S>`:
  1. `AuthenticatedUser` extension missing ⇒ 401 `"Authentication required"` (unchanged).
  2. `AccessContext` extension missing ⇒ 401 as well — it means the route is mounted outside
     `require_auth`, indistinguishable from unauthenticated; a debug-assert-style
     `tracing::error!` marks it as a wiring bug.
  3. `AccessState::from_ref(state)` → `engine.authorize(&ctx, &$action, None)`:
     `Decision::Allow` ⇒ `Ok($name(user))`; `Decision::Deny(reason)` ⇒ deny trace
     (`tracing::debug!` with `action`, `user_id`, bounded `reason` label) + counter
     `metrics::counter!("uptrakit_access_denies_total", "reason" => <static str per DenyReason
     variant>)` + 403 `error_response(StatusCode::FORBIDDEN, "Insufficient permissions")` —
     the D3 generic body, no grant/selector detail.
- The `DenyReason -> &'static str` label mapping is a small private fn in `action.rs` with an
  exhaustive match (`#[non_exhaustive]` enum from another crate ⇒ a wildcard arm mapping unknown
  variants to `"other"`; the four known variants are named).
- Name collisions with the legacy module (`CanUpdateHosts`, `CanTriggerChecks` exist in both
  `middleware::permission` and `middleware::action`) are legal — different module paths; each
  route file imports exactly one. Never glob-import either module.
- In-file unit tests mirror `permission.rs:197-295`: 401 on missing extensions, 403 on
  no-grant context, allow on granted context — driven through `from_request_parts` with a
  minimal state type implementing `FromRef` for `AccessState` and contexts built via a
  `MockDatabase`-backed engine (copy the fixture idiom from the engine's own
  `dummy_engine()`-based tests in `controller-core/src/access/mod.rs`).

`permission_extractor!` and all 36 legacy extractors stay compiled and referenced by the
unconverted route files; nothing is deleted here (M1.8 owns deletion).

### 5. Security schemes (`SecurityAddon`, `router.rs:480-496`)

The `Modify` impl registers, in addition to the existing `bearer_token` (kept — grilling
decision 1):

- `"oauth2"` — `SecurityScheme::OAuth2` with one `Flow::AuthorizationCode`:
  `AuthorizationCode::new("/oauth/authorize", "/api/v1/oauth/token", scopes)` (relative URLs —
  same-origin AS; matches how the SPA reaches these endpoints today), scopes =
  `CATALOG.iter().flat_map(|e| e.verbs.iter()).map(|v| (v.action_str, v.description)).collect::<Scopes>()`.
  Scheme description (via `OAuth2::with_description`) documents: the device flow exists but is
  not representable in the OAS flows object, and the dynamic `plugin.*`/`surface.*` namespaces
  are an open set not enumerated in the dictionary.
- `"developer_token"` — `SecurityScheme::Http` bearer for opaque `upk_` tokens; description
  states that the per-operation action requirement is enforced server-side identically (the
  `http` scheme type carries no scope field).

### 6. Reference conversion: hosts family (`routes/hosts.rs`)

Per operation: drop `extensions(("x-required-permission" = json!(...)))`, replace
`security(("bearer_token" = []))` with the native pair, swap the extractor import
(`middleware::permission::{...}` → `middleware::action::{...}`):

| Handler | Extractor (new) | `security(...)` |
| --- | --- | --- |
| `list_hosts` | `CanReadHosts` | `("oauth2" = ["hosts:read"]), ("developer_token" = [])` |
| `get_host` | `CanReadHosts` | `("oauth2" = ["hosts:read"]), ("developer_token" = [])` |
| `update_host` | `CanUpdateHosts` | `("oauth2" = ["hosts:update"]), ("developer_token" = [])` |
| `deactivate_host` | `CanDeleteHosts` | `("oauth2" = ["hosts:delete"]), ("developer_token" = [])` |
| `batch_hosts` | `CanDeleteHosts` | `("oauth2" = ["hosts:delete"]), ("developer_token" = [])` |
| `discover_host` | `CanTriggerChecks` | `("oauth2" = ["checks:trigger"]), ("developer_token" = [])` |

Handler bodies are otherwise untouched (tuple payload keeps `caller`/`_user` bindings working).
`host_tags.rs` and `software_items/host_assignments.rs` are **not** part of the reference family
(M1.4b domains). `./scripts/regen-api.sh` runs in this task; `openapi.json` +
`frontend/src/lib/api/generated/` are committed **unopened** (regen + `git add`; never read the
generated artifacts into context).

### 7. `AccessInvalidated` delivery arm

- `ControllerResources` (`event_delivery.rs:24`) gains
  `pub access_engine: Option<&'a Arc<AccessEngine>>`.
- `deliver_controller_event` gains, before the wildcard:
  `ControllerMessage::AccessInvalidated(payload)` ⇒ if the engine handle is present,
  `engine.apply_remote_invalidation(&payload)` (sync, counts + flushes internally), return
  `true`; absent handle ⇒ debug-log and `true` (never NAK — the 60 s TTL backstop self-heals,
  and redelivery cannot make a missing handle appear).
- `NatsConsumerConfig` (`nats_transport.rs:51`) gains
  `pub access_engine: Option<Arc<AccessEngine>>`; the consumer loop passes
  `access_engine.as_ref()` into `ControllerResources` (`nats_transport.rs:276`); `serve.rs:178`
  fills it with `Some(Arc::clone(&app_state.access_engine))`.
- No publisher exists until M1.6a — the arm is reachable only from another instance's future
  publishes; its test feeds `deliver_controller_event` a constructed
  `ControllerMessage::AccessInvalidated` directly and asserts the engine cache was flushed
  (observable via a subsequent `context()` reload — copy the observation idiom from the engine's
  `apply_remote_invalidation_takes_effect_on_next_context` test).
- Inventory discipline: before freezing its edit list the plan re-greps **every** construction
  site of the three widened shapes — `AppState {`, `ControllerResources {`,
  `NatsConsumerConfig {` — workspace-wide including test modules; the sites named in this spec
  are the ones found at spec time, a floor not a ceiling.

### 8. CI gate: `ci/verify_action_security_declarations.py`

Purpose (test-plan D14): a converted operation's `oauth2` scope list and its handler's action
extractor must agree — both derive from the catalog, so the check is mechanical.

Inputs, parsed at run time (no committed mirror tables — mirrors drift):

1. **Extractor map**: parse the `action_extractor!` invocation in
   `crates/ui/web-api/src/middleware/action.rs` → `{extractor name → actions::CONST ident}`.
2. **Const→string map**: parse the `access_catalog!` invocation in
   `crates/shared/types/src/access/catalog.rs` → `{CONST ident → "resource_str:verb_str"}` from
   the per-verb tuples (the invocation carries the resource string literal, the verb string
   literal, and both const idents — **never** derive the string from the const name; underscore →
   `.`/`-` mapping is ambiguous, e.g. `PLUGIN_CONFIGS_TRIGGER` ↔ `plugin-configs:trigger`).
3. **Route scan**: every `.rs` under `crates/ui/web-api/src/routes/` — for each
   `#[utoipa::path(...)]` attribute (captured by balanced-parenthesis scan over the file text,
   never line-oriented regex — attributes wrap) paired with the following `async fn` signature.

Checks (violations print `file:line` + a named rule, exit non-zero):

- **R1 scope↔extractor agreement**: an operation declaring `("oauth2" = [scopes])` with
  non-empty scopes must have exactly that action set represented by known action extractors in
  its handler params, and vice versa: a handler using an action extractor must declare the
  matching non-empty `oauth2` requirement.
- **R2 authenticated-only**: an operation declaring `("oauth2" = [])` (empty scopes) must have
  **no** action extractor.
- **R3 no mixed worlds**: no operation may carry both an `x-required-permission` extension and
  an `oauth2` requirement.
- **R4 developer_token pairing**: every operation declaring `oauth2` also declares
  `("developer_token" = [])` (the alternatives are a fixed pair until M3 revisits).
- Unconverted operations (`bearer_token` + `x-required-permission`) are ignored except by R3 —
  transition tolerance for the M1.4b window.
- **Non-vacuity guards** (green-on-empty is a defect): hard error if the extractor map parses
  empty, if the catalog map parses empty, or if zero operations declare a non-empty `oauth2`
  scope list (hosts guarantees ≥1 from this task on).

Companion unittest `ci/test_verify_action_security_declarations.py` (wired beside
`ci/test_check_plugin_semantic_boundary.py` in ci.yml) drives the checker's pure functions over
fixture snippets: one passing pair, plus one RED case per rule R1–R4 and per non-vacuity guard —
this makes D14's perturbation red-path a committed test, not a one-time demo. The plan
additionally demonstrates one live end-to-end RED (perturb a hosts scope string → non-zero exit →
revert) before wiring the gate.

Wiring: `.github/workflows/ci.yml` (semantic-boundary job — the family that installs the needed
tooling; Python needs no extra install) **and** `.husky/pre-push`, following the
`check_plugin_semantic_boundary.py` precedent of running in both. Registered in
`docs/development/quality-gates.md` (canonical list) and the root `AGENTS.md` quick-start block
in the same commit.

## Manifest changes

None expected: `uptrakit-web-api` already depends on `metrics`, `uptrakit-controller-core`
(features `["axum-integration"]`), `uptrakit-shared-db`, `uptrakit-shared-types`
(`["openapi", ...]`); utoipa security types are already imported in `router.rs`. The plan
verifies no new feature axis is needed (the scheme builder uses `CATALOG`, which is
feature-ungated). If the extractor unit tests need engine fixtures, controller-core is already a
dev-dep with `["testing"]` and sea-orm `mock` comes via controller-core's own dev surface — the
plan confirms with a compile probe rather than assuming.

## Tests

Harness (TestApp, `crates/ui/web-api/src/test_harness/`), new integration-test module
`integration_tests/access_rest_enforcement.rs` (name final at plan time; sibling idioms copied
from existing `integration_tests/*.rs`):

- **D1 (subset)** — authorized request succeeds on the hosts family, **×2 credentials**: (a) the
  first registered user's session JWT (holds seeded roles → `hosts:read` via grants);
  (b) an `upk_` API token created through the existing token endpoint for the same user. The ×2
  rule is the test plan's standing requirement for D1/D3.
- **D2 (subset)** — no credential ⇒ 401 on a hosts route (never 403).
- **D3 (subset)** — a second registered user with **no** role assignments and no direct grants ⇒
  403 with the generic body on `GET /api/v1/hosts`, ×2 credentials. Assert the body carries no
  grant/selector detail (compare against the fixed `"Insufficient permissions"` message).
- **Immediate effect** (M1 exit criterion "grant changes take effect on the next request") —
  grant the zero-grant user `hosts:read` via `access_grants::insert_grant` +
  `app.state.access_engine.invalidate_subjects(&[user_id], &[])` ⇒ next request 200; then
  `delete_grant` + `invalidate_subjects` ⇒ next request 403. No TTL wait, no re-login — this is
  the observable difference from the JWT-snapshot era.
- **Delivery arm** — direct `deliver_controller_event` test as in §7.
- **Extractor unit tests** — §4 (401/403/allow at the macro level).
- **Middleware 500 path** — engine failure ⇒ 500 is covered at engine level by M1.3's DB-error
  tests; staging a broken DB inside a live TestApp is not reachable with the current harness, so
  the REST-level 500 branch is covered by the middleware's error-arm unit shape only if the plan
  finds a cheap seam (e.g. dropping the sqlite file is not applicable in-memory). If no cheap
  seam exists, the branch ships with the tracing assertion deferred and the rationale recorded —
  do not fabricate a harness capability.
- **CI gate unittest** — §8 (rules R1–R4 + non-vacuity, committed RED fixtures).

Non-goals restated: no `start_paused` anywhere here (no test awaits tokio time; the TTL behavior
is already pinned in M1.3), no upstream-behavior tests (utoipa scheme serialization is asserted
only via the regenerated `openapi.json` staleness gate, not unit-tested).

## Verification gates

In dependency order, all foreground:

1. `cargo fmt --all`
2. `cargo clippy --all-targets --no-default-features --features db-sqlite`
3. `cargo test -p uptrakit-controller-core` (Clone derive + engine untouched surface)
4. `cargo test -p uptrakit-web-api --no-default-features --features db-sqlite`
5. `./scripts/regen-api.sh` → commit `crates/ui/web-api/openapi.json` +
   `frontend/src/lib/api/generated/` (regen only, never opened); `cd frontend && npm run check`
6. `python3 -m unittest ci/test_verify_action_security_declarations.py` +
   `python3 ci/verify_action_security_declarations.py` (green) + one demonstrated live RED
   (perturbed scope → non-zero, reverted)
7. Whole-workspace `cargo test --no-default-features --features db-sqlite` (new AppState field
   crosses crate seams: controller-runtime `serve.rs`, harness)
8. `cargo clippy --all-targets --all-features` + `cargo test --all-features` (requires
   `frontend/build/`; build frontend first)
9. `cargo xtask audit-coverage-check` (hosts handlers keep their names/methods — expected
   no-op, run to prove it), `bash ci/verify_handler_state_contract.sh`,
   `python3 ci/verify_db_access_policy.py`, `markdownlint --config .markdownlint.json '**/*.md'`

## Documentation deliverables

1. `docs/development/quality-gates.md` — add `verify_action_security_declarations.py` (canonical
   command list) — same commit as the root `AGENTS.md` quick-start block line (AGENTS.md
   maintenance rule: both in one commit).
2. Root `AGENTS.md` MUST-FOLLOW bullet **"Use typed permission extractors for route
   authorization"** (cite by bold lead-in, not position): its body currently mandates
   `permission_extractor!` + "the matching `x-required-permission` extension" — inaccurate for
   converted families from this task on. Amend the body to name both worlds (legacy:
   `permission_extractor!` + extension; converted: `action_extractor!` + native
   `security(("oauth2" = [...]))`), keeping the bold lead-in verbatim. Final wording collapses
   back to one world in M1.8/M1.9.
3. `crates/ui/web-api/AGENTS.md` — `AccessState` in the sub-state list; correct the
   handler-conventions passage that states every protected endpoint "must also carry the matching
   `x-required-permission` extension" (grep the literal — line-number cites drift) to the same
   two-world transition phrasing; a short "unconverted families keep `permission_extractor!` +
   `x-required-permission` until M1.4b" note (≤250-line budget holds).
4. `docs/security/auth-and-authorization.md` — a bounded transition subsection: the dual-world
   state (which enforcement path a route family uses is visible from its extractor import), the
   new scheme pair, and a pointer to the refactoring docs. The full rewrite stays M1.9 — this is
   a correction-in-place, not a rewrite.
5. Regenerated `openapi.json` + frontend client (committed artifacts, gate-enforced).
6. Rustdoc on every new public item (`AccessState`, macro, script header comment naming its rules
   R1–R4). No ADR (M1.9 owns it); no wire-protocol.md change (payload documented in M1.3);
   no CONTEXT.md change (vocabulary landed with M1.1).

## Alternatives considered

- **Engine handle in request extensions** (middleware inserts `Arc<AccessEngine>` beside the
  context): one line, but smuggles app-scoped state through per-request extensions against the
  repo's sub-state doctrine, and every extension read is a runtime-optional coupling. `FromRef`
  sub-state keeps the extractor's state dependency compile-checked. Rejected.
- **Lazy context build inside the extractor** (skip middleware change): avoids the per-request
  load on unconverted routes during the window, but makes the extractor async-DB-dependent,
  duplicates the load across multiple extractors on one request, and diverges from the pinned
  task contract ("built in `require_auth`, stored in request extensions"). Rejected.
- **Delete `bearer_token` scheme in M1.4a**: forces the full sweep into this task (dangling
  scheme refs otherwise) — defeats the M1.4a/M1.4b split. Rejected (grilling decision 1).
- **Bash+rg CI gate**: family precedent exists, but multi-line utoipa attributes plus two macro
  joins are exactly the wrapped-payload/line-oriented failure class already hit by earlier
  gates. Rejected (grilling decision 2).
- **Deriving const→string in the gate from const names**: lossy (`_` → `.` vs `-` ambiguity);
  parse the catalog invocation instead. Rejected.

## Deferred / out of scope (verbatim carriers)

- **M1.4b**: per-domain sweep of every remaining route module; `x-action-dynamic: true` on the
  seven surface wrappers; `bearer_token` scheme deletion in the final sub-PR; the
  `./scripts/regen-api.sh` + `rg` extension sweep at the end; D-row coverage per domain.
- **M1.5**: MCP/surfaces/interactive-WS/inline `has_permission` sites; live
  `DynamicActionRegistry` injection (`with_registry`); streaming-holder context refresh residual.
- **M1.6a/b**: mutation-site `invalidate_subjects` + NATS publishing (the delivery arm added here
  gets its first live producer); deny audit **Events**; management API; catalog endpoint.
- **M1.7**: claims removal, `me` `authority` field, frontend swap.
- **M1.8**: `permission_extractor!` + `Permission` + shim deletion; `rg`-clean.
- **M1.9**: canonical doc rewrite + ADR.
- **M3**: real scope parsing in `require_auth` (OAuth-presented tokens); until then `scope: None`
  is pinned for all REST credentials.
