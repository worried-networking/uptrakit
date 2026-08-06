# M1.6b — Access Catalog Endpoint + Preset Retirement

**Date**: 2026-08-06
**Status**: Approved for planning
**Milestone**: M1.6b of the authn/authz refactoring
(`.superpowers/authn-and-authz-refactoring/11-task-breakdown.md` §M1.6b)
**Baseline**: `main @ fe6775672` (M1.6a fully landed: grant/role management API, `users:manage`/`access:manage`
split, engine-backed lockout guard, OIDC role-sync guard)

## Problem / goal

Four deliverables close out M1.6b:

1. **`GET /api/v1/access/catalog`** — expose the authorization vocabulary as data: every currently valid
   action with `selector_support`, the role bundles (demoted access presets), and the scope presets.
2. **Preset retirement** — delete `GET /api/v1/access-presets`, `POST /api/v1/users/{id}/apply-preset`,
   the `apply_preset` audit site, and every live consumer (openapi-client module, CLI commands), keeping
   the tree green at every commit.
3. **Deny audit Events** — emit an audit Event for denials of `system.*` actions, `commands:manage`,
   `access:manage`, and `mcp:use`; all other denials stay trace + counter only
   (`09-resolved-questions.md` §6; test-plan row D12).
4. **`bearer_token` scheme retirement** — inherited by the last-reference rule (pending-specs tracker):
   deleting `access_presets.rs` leaves exactly three `users.rs` self-service operations on the legacy
   scheme; convert them and deregister the scheme.

Done-when (task breakdown): catalog green; CLI/openapi-client compile with the preset endpoints gone;
audit-coverage gate green.

## Owner decisions (grilling round, 2026-08-06)

1. **Scope presets: marker, no expansion.** The read-only preset (id `all-reads` — renamed during
   review, see §1's naming convention) is served with its concrete expanded action list
   (caller-independent, computed from the built-in catalog); `all my current actions` is served as a
   marker with no action list — the M4.2 client resolves it against `me` (whose expansion machinery is
   M1.7 work). The contract is stable across M1.7/M4.2; no expansion machinery is pulled forward.
2. **Deny Events: one uniform `access.denied` action everywhere.** Emitted at every qualifying deny site
   including both MCP gates. The MCP gates *keep* their existing `Denied` auth-audit rows (different
   concern: authn-flow outcome vs authz deny) — two rows per MCP deny is accepted.
3. **CLI: `set-roles` gains `--names`.** No new command. `uptrakit users set-roles <id>` accepts either
   positional role UUIDs (as today) or `--names viewer,operator` (client resolves names via the existing
   roles list endpoint, then calls the same `update_user_roles` path). Tier application = catalog lookup
   + one command.
4. **`bearer_token` dies in M1.6b.** The three self-service ops (`initiate_email_change`,
   `change_password`, `cancel_email_change`) convert to the authenticated-only declaration shape
   (`security(("oauth2" = []), ("developer_token" = []))`, matching `update_profile`); the scheme
   registration and the scope-map `"unconverted"` class are deleted.

## Design

### 1. Catalog endpoint

**Route**: `GET /api/v1/access/catalog`, new file `crates/ui/web-api/src/routes/access_catalog.rs`,
tag `"Access"` (sibling of `access_grants.rs`). Authenticated-but-ungoverned per the corpus
(`07-decision-and-enforcement.md` §Catalog introspection): `security(("oauth2" = []),
("developer_token" = []))`, no action extractor — the handler takes the authenticated-user extension
only. Registered via `routes!()` + `paths(...)` + `components(schemas(...))` in `router.rs` like every
sibling. `db_access_policy.toml` class: `no-db` (the handler reads only in-memory state — the static
catalog, the surface registry snapshot, and static bundle/preset data).

**Reconnaissance disclosure is accepted for v1** (restated so it is not re-litigated): live `surface.*`
entries disclose which integrations the instance runs to any authenticated caller. The corpus accepts
this — the threat model's attacker is an authenticated under-privileged principal in a homelab/small-fleet
deployment, the disclosure is integration *presence* only, and the consent screen needs the vocabulary
pre-grant. Deferred trigger recorded in the corpus: scope the dynamic portion per-caller if the posture
shifts toward untrusted tenant users.

**Response shape** (DTOs in `uptrakit-web-api-types`, new module `access_catalog.rs`; exact field
naming at implementers' discretion, structure binding):

```text
AccessCatalogResponse
  resources:     [CatalogResourceEntry]   -- actions grouped by resource, built-in + live dynamic
  role_bundles:  [RoleBundleEntry]
  scope_presets: [ScopePresetEntry]

CatalogResourceEntry
  resource: String                        -- "hosts", "settings.auth", "surface.proxmox.hosts", …
  actions:  [CatalogActionEntry]

CatalogActionEntry
  action:           String                -- "hosts:read"
  verb:             String                -- "read"
  description:      String
  selector_support: SelectorSupport       -- reuse the existing shared-types enum: it already carries
                                          -- serde snake_case ("none"|"host"|"host_and_software") and
                                          -- a ToSchema derive behind the openapi feature

RoleBundleEntry
  name: String, description: String, roles: [String]   -- seed-role names; advisory, applied via
                                                        -- standard role assignment

ScopePresetEntry
  name: String, description: String
  kind: "static" | "caller_actions"       -- serde snake_case enum + wire-safe fallback (see below)
  actions: Option<[String]>               -- present when "static"; absent for "caller_actions";
                                          -- unspecified for unknown kinds (fallback)
```

+ **Built-in actions** come from `uptrakit_shared_types::access::CATALOG` (`CatalogEntry`/`VerbEntry`
  already carry `action_str`, `description`, `selector_support`) — the same const the `oauth2` scope
  dictionary in `SecurityAddon` iterates, so the two cannot drift (both read one source; no extraction
  needed).
+ **Dynamic actions**: see §2. Grouped under their own resource entries (`surface.<id>`), verb `use`,
  `selector_support: none`, a generic description. `plugin.*` entries are naturally absent: no plugin
  declares action verbs yet (`declare_plugin!` verb declarations are post-v1 per the tracker) and no
  plugin `DynamicActionRegistry` impl exists — the catalog serves exactly what the engine can authorize,
  never more. When the first plugin registry impl lands, its actions appear through the same enumeration
  seam with no endpoint change.
+ **Role bundles**: served from the renamed `RoleBundle` type (§3) — name, description, seed-role names
  for the five tiers (`read_only`, `operator`, `manager`, `administrator`, `owner`).
+ The `kind` discriminator enum is deliberately open-ended (M4.2 calls presets "extensible"), and it is
  **wire-serialized** — `#[non_exhaustive]` alone does not make old clients tolerate a third kind (an
  unknown string fails the whole response deserialization in `uptrakit-openapi-client` and the generated
  TS client). Decided now, not deferred (sharpened in review round 2): the enum uses **`wire_safe_enum!`**
  — the repo's mandated mechanism for wire-serialized string enums, with in-crate precedents
  (`oauth/scope.rs`, `oauth/device.rs`, `notifications/event_types.rs`) — giving `Other(String)`
  catch-all, infallible serde, strict `FromStr`, and a wire-string `ToSchema`; never a hand-rolled
  `#[serde(other)]`. A per-type test (modeled on `scope.rs`'s `deserialize_infallible_for_unknown_string`,
  the sanctioned idiom) asserts an unknown `kind` string deserializes to the fallback instead of erroring.
  **Client contract for the fallback, documented on the schema**: an unknown kind means "do not offer
  this preset" — clients must never route it into caller-expansion (the broadest interpretation). No
  picker exists in this milestone, so that rule is a schema-doc obligation carried to M4.2's consumer,
  not a test row here; the M1.6b test asserts only that the unknown string deserializes to the
  fallback variant.
+ **Naming convention across sections**: role-bundle names keep the existing role-name style
  (`read_only`, `operator`, … — they are displayed beside the snake_case role names they bundle); scope
  presets use kebab-case ids. To avoid the near-collision between the role bundle `read_only` (roles to
  assign) and a scope preset "read-only" (an action ceiling) in one response, the scope preset's **id**
  is **`all-reads`** with a "Read-only access" display description — the corpus's "read-only" is the
  concept's name, not a binding identifier.
+ **Scope presets**: two code-defined entries. `all-reads`: `kind: static`, `actions` = an **explicit
  reviewed const list** of concrete `:read` action strings (never a `*:read` pattern, per the corpus
  "clients never receive a preset that silently includes a future action"). The initial list is every
  built-in catalog `read` action as of this spec — system-plane reads (`system.audit:read`, …)
  included, which is safe because a scope is a ceiling over the caller's grants, never a grant. A
  **bidirectional** guard test diffs the const against `CATALOG`'s `read`-verb actions: it fails when a
  new read action appears unlisted (forcing a reviewed add-or-exclude decision, with an explicit
  exclusion list for the reviewed-out case) **and** when a const entry no longer exists in `CATALOG`
  (a removed/renamed read action must not leave a stale string the resources section no longer carries
  and M4.2 minting could not parse).
  `all-my-current-actions`: `kind: caller_actions`, `actions: None` — semantics documented on the
  schema: the client expands it against the caller's effective action list (`me`, M1.7) at consumption
  time (token creation, M4.2). **Recorded constraint for M1.7/M4.2**: because expansion is client-side,
  either `me` must report the *acting credential's* effective (scope-intersected) actions, or M4.2's
  token creation must intersect the minted scope server-side — otherwise a narrowed-scope credential
  could mint a token broader than itself (still ceilinged by the user's grants, but a scope escalation
  relative to the parent credential).

### 2. Dynamic-action enumeration

`DynamicActionRegistry` (`crates/ui/controller-core/src/access/mod.rs`) today has only
`is_registered(&Action) -> bool`. It gains an enumeration method:

```rust
/// Every dynamic action currently registered — the catalog's dynamic section.
/// Must agree with `is_registered`: an action is in this list iff `is_registered` returns true.
fn registered_actions(&self) -> Vec<Action>;
```

+ `AccessEngine` exposes it (`pub fn dynamic_actions(&self) -> Vec<Action>`, empty when no registry is
  injected) so the catalog handler reads through the engine — the engine stays the single authority for
  the dynamic vocabulary, and the catalog matches deny/allow reality by construction.
+ `SurfaceActionRegistry` (`crates/ui/web-api/src/surface_action_registry.rs`) implements it by
  enumerating surface IDs **tenant-blind** from `SurfaceRegistry` (a new listing method on the registry,
  mirroring `has_surface`'s tenant-blind semantics — *not* `list_surfaces_for_tenant`, which would make
  the catalog's contents disagree with `is_registered`). Emits `surface.<id>:use` per registered surface.
  The listing filters on the **same non-empty-provider predicate `has_surface` uses** (not on raw key
  presence, which agrees only incidentally via cleanup), and honors the registry's documented
  non-reentrancy: clone the ID set under the lock, build `Action`s after release.
+ **Grammar mismatch is real and must fail closed**: `validate_surface_identifier` accepts IDs
  (underscores, consecutive/trailing hyphens, …) that the `Action` resource grammar rejects, so
  `registered_actions` is fallibly constructive. Build each `Action` through the **same parse path the
  read side uses** and skip IDs that do not parse — such a surface's `:use` action is unparseable
  everywhere, so `is_registered` can never return true for it, and skipping preserves the iff contract
  (no `unwrap`, no lossy normalization — a normalized ID would advertise an action the engine denies).
  Unit test: register a surface with an `_`-containing ID; assert it appears in neither
  `registered_actions` nor `is_registered`. (Tightening registration admission to the action grammar is
  deliberately out of scope — noted under Deferred.)
+ **Cross-tenant disclosure, stated precisely**: tenant-blind enumeration serves every registered
  surface ID — including surfaces registered by *another tenant's* services — to any authenticated
  caller of any tenant. This is a strictly wider statement than "instance integration presence" and is
  accepted for v1 on the same grounds (single-tenant deployment posture; the corpus's deferred trigger —
  untrusted tenant users — covers it). What actually prevents cross-tenant *use* is the dispatch-side
  filter, not grant scoping: surface resolution selects providers via
  `list_targeted_providers_for_surface(surface_id, tenant_id, visibility)` and rejects
  tenant-incompatible providers — a tenant-A grant naming a tenant-B surface ID passes the tenant-blind
  `is_registered` but can never dispatch. Cite that mechanism, not "grants are tenant-scoped", so a
  future refactor of the dispatch filter knows it is load-bearing for this acceptance.
+ The alternative — the handler going around the engine straight to `SurfaceRegistry`/`PluginOps` — was
  rejected: it reintroduces the tenant/visibility mismatch the `SurfaceActionRegistry` docs call out and
  lets catalog and engine drift independently.
+ Consumer inventory for the trait change (breaking: new required method): the one production impl plus
  every test double implementing the trait in `controller-core` and `web-api` tests — the plan must grep
  `impl DynamicActionRegistry` workspace-wide and list each site.

### 3. Preset retirement — deletion inventory

The corpus decision (`06-grant-model.md` §Presets demote to catalog metadata): tiers stop being a
server-side mechanism; role assignment is the single write path; the `apply_preset` audit site dies with
the endpoint (role-assignment Stateful audit covers what preset application used to record).

**Type rename**: `AccessPreset` (`crates/shared/types/src/access_preset.rs`) survives as the role-bundle
data source and is renamed **`RoleBundle`** (module `role_bundle.rs`), matching the corpus vocabulary.
Keep `all()`, `roles()`, `description()`, `as_str()`/`Display`. `FromStr` + `ParseAccessPresetError`
existed for the apply-preset request parse; after the deletions the plan greps for surviving callers and
deletes them if callerless (expected: callerless — `warnings = deny` forces the call). Update the
`docs/development/coding-standards.md` allowlist line naming `AccessPreset` and the doc-comment mention
in migration `m20260424_000001_access_mcp_permission.rs` (comment-only edit, no migration logic).

**web-api** (`crates/ui/web-api`):

+ `src/routes/access_presets.rs` — whole file (both handlers, `emit_user_preset_audit`, the legacy
  guard-copy helpers `roles_grant_manage_users_check`/`count_other_manage_users`, inline tests). This
  closes M1.6a's documented "inconsistent guard window". Nothing needs re-pointing: the modern
  role-assignment path (`update_user_roles`) already carries the engine-backed lockout guard, the
  system-plane fine check, and cache invalidation.
+ `src/routes/mod.rs` — `pub mod access_presets;`.
+ `src/router.rs` — the two `paths(...)` entries, the two `.routes(routes!(...))` lines, and the
  `components(schemas(...))` entries for `AccessPresetResponse` and `ApplyPresetRequest`.
+ `db_access_policy.toml` — the whole `[routes."access_presets.rs"]` table.
+ `audit-catalog.toml` (crates/shared/audit-log) — the `apply_preset` entry
  (`site = "uptrakit_web_api::routes::access_presets::apply_preset"`, `action = "user.update"`) plus its
  banner comment. The `user.update` action registration stays (other live sites in `users.rs`).
+ `scope-map.golden.json` — regen (`UPDATE_SCOPE_MAP=1`); the two `"unconverted"` rows disappear.
+ These two operations are the **last `x-required-permission` sites in shipped code** — after this
  deletion the extension is gone from the codebase ahead of M1.8's rg sweep.

**web-api-types** (`crates/shared/web-api-types`):

+ `src/access_presets.rs` (whole file, `AccessPresetResponse`) + its `lib.rs` module line.
+ `ApplyPresetRequest` item in `src/users.rs` (item only; the file stays).

**openapi-client** (`crates/shared/openapi-client`):

+ `src/access_presets.rs` (whole file) + `lib.rs` module line; the types re-export module for
  `access_presets`.
+ `apply_preset` method in `src/users.rs` + its co-located serialization test.
+ `paths.rs`: the `access_presets` module and the `users::apply_preset` fn.
+ **Add**: `get_access_catalog()` method + a `paths` const for `/api/v1/access/catalog` in the same
  commit as the endpoint (ADR-0026 drift guard: new endpoint ⇒ client method + paths const;
  `cargo xtask openapi-client-check` gates it). No ledger entries exist for the preset endpoints —
  symmetric deletion needs no ledger edit.

**CLI** (`crates/ui/cli`):

+ Delete: `AccessPresetsCommands` + the top-level `AccessPresets` command variant + `dispatch_access_presets`,
  `UsersCommands::ApplyPreset` + `apply_preset()` + `list_presets()`, the
  `HumanOutput for Vec<AccessPresetResponse>` impl, the preset imports, the parse tests
  (`users_apply_preset_parses`, `access_presets_list_parses` + banner) and the rendering tests
  (`preset_list_empty`, `preset_list_has_rows`). Update the `Users` command help text ("Manage users,
  roles, and access presets" → drop presets).
+ Extend `UsersCommands::SetRoles`: positional `role_ids: Vec<Uuid>` becomes optional; add
  `--names <name>[,<name>…]` (comma-separated role names); exactly-one-form enforced via a required
  clap `ArgGroup` over the two args (the repo's only existing group precedent, `UpdateFreeze`'s
  `enable`/`disable`, is at-most-one — this is the first required group and the first positional-list
  member, both standard clap; the plan's parse tests must cover the neither-form and both-forms
  rejections explicitly).
  The `--names` path calls the existing roles list endpoint, resolves names → IDs (error on any
  unresolved name, listing the misses), then calls the existing `set_roles` fn — one request path, no
  new endpoint. Parse tests for both forms + the mutual-exclusion rejection; a rendering/behavior test
  per the existing CLI test idiom (flat `Cli::try_parse_from` assertions in `src/tests.rs`).

**Frontend**: regen only (`./scripts/regen-api.sh`) — verified: no non-generated consumer of
`listAccessPresets`/`applyPreset`/`AccessPresetResponse` exists under `frontend/src`.

### 4. Deny audit Events

**New Event action** `access.denied`, registered in the three standard places in
`crates/shared/audit-log/src/action_type.rs` (const, registry array, `audit_actions!` row —
`access_denied => ACCESS_DENIED, Event;`), precedent `user_role.sync_lockout_prevented`.

**Qualifying predicate** — a single shared function in `uptrakit-shared-types::access` (exact name at
implementers' discretion, e.g. `deny_event_worthy(&Action) -> bool`): true iff the action's resource is
`system.*` (`Resource::is_system()`) or the action is `commands:manage`, `access:manage`, or `mcp:use`.
One definition, consumed by both `uptrakit-web-api` and `uptrakit-mcp` — never duplicated (a duplicated
privilege-classification block is a drift hole).

**OR-gate rule**: several deny sites gate one operation on an OR of actions (`authorize_any` and inline
equivalents). An OR-gate denial qualifies for the Event **iff every alternative qualifies** — the
operation is then reachable only through sensitive authority. A mixed gate (e.g. `list_plugin_types`'s
`[software:read, settings:read, system.settings:manage]`) is an ordinary operation with a sensitive
allow-*alternative*; its denial is an ordinary denial (D12's negative leg), not a `system.*` denial.
All-qualifying gates emit one Event whose details carry the full alternatives list. (Correction,
review round 2: the `system_services` batch gate is NOT a multi-alternative example — the handler
selects a single-element action slice from the request body *before* the gate, so it enters the funnel
as a single qualifying action. No multi-alternative all-qualifying gate exists in production today;
the rule governs future gates, and its all-qualifying-multi branch stays unpinned until one exists.)

Two consequences stated explicitly rather than left emergent:

+ **A mixed gate's sensitive alternative is deny-unobservable by design** — a principal probing
  `system.settings:manage` through `list_plugin_types` produces no Event. Accepted for v1: the
  alternative rule ("any alternative qualifies") would fire an Event on every under-privileged browse of
  an ordinary route, violating D12's negative leg. The two pinned D12 OR-gate tests (one mixed-negative,
  one all-qualifying-positive) are the guard against a gate-membership change silently flipping a
  route's classification unnoticed.
+ **Duplicate rows at already-audited deny sites are accepted, like the MCP decision**: some qualifying
  deny branches already emit a domain-level `Denied` row today (verified: `system_services.rs`'s
  batch OR-gate emits `emit_system_service_audit(…, Denied, "insufficient_permissions")` in the same
  branch). The plan inventories qualifying sites with pre-existing `Denied` emissions and keeps both
  rows — the domain row answers "what happened to this resource", `access.denied` is the uniform deny
  query surface; suppressing domain rows is out of scope.

**Emission sites** (the corpus scope: Events for the qualifying denials only; everything else stays
debug-trace + `uptrakit_access_denies_total`; no per-principal repeat detection in v1). The wiring is
funnel-based, not a spec-frozen site list, derived in two steps at plan time: **(1) grep the counter
literal `uptrakit_access_denies_total` workspace-wide** — its incrementers are the complete set of
*counter owners* (`record_access_deny` in web-api plus the three inline MCP sites, incl.
`oauth/tool_auth.rs`; completeness over *deny paths* is what step 2 supplies — the `_ =>` arms this
milestone fixes deny without incrementing);
**(2) grep `record_access_deny` callers** for the web-api fan-out (current hits: the
`action_extractor!` deny arm, `authorize_any`, `require_system_access`,
`routes/surfaces.rs::enforce_required_action`, `routes/users.rs::update_profile` — the last two are
hand-rolled inline `authorize` matches, *not* `authorize_any` callers, and must be wired individually):

1. **Central helper in `middleware/action.rs`** — one `#[uptrakit_audit_log::audit_required]` fn
   (precedent: `handle_role_sync_outcome`) that, given the emitter, the `AccessContext`, the denied
   action(s), and the `DenyReason`, increments the counter and emits the Event iff the predicate
   (single action) / OR-gate rule (action list) holds. **One audit-catalog row** keys this helper.
   Wire it into every funnel branch:
   + the `action_extractor!` deny arm (the macro's state bound gains `AuditEmitterState: FromRef<S>` —
     the sub-state already exists on `AppState`); while in the macro, close the pre-existing gap where
     the non-exhaustive `_ =>` arm skips `record_access_deny` — route it through the same funnel;
   + `authorize_any` (signature widens to accept the emitter + scope inputs; callers re-derived by grep
     at plan time — currently `interactive_ws.rs`, `plugin_configs/crud.rs::list_plugin_types`,
     `plugin_type_settings.rs`, `services/batch.rs`, `system_services.rs`);
   + `require_system_access` (same widening; callers: `users.rs`, `roles.rs`, `access_grants.rs`; its
     own `Decision` `_ =>` arm has the identical skip-the-funnel gap — same treatment);
   + the two hand-rolled inline matches (`surfaces.rs::enforce_required_action`,
     `users.rs::update_profile`), whose deny arms get the same funnel treatment. Note
     `update_profile`'s inline check denies `users:manage` — non-qualifying, so wiring it yields
     counter uniformity only, never an Event, given the current action.

   **Which wildcard arm, precisely**: only wildcards over the non-exhaustive `Decision` enum are deny
   paths that belong in the funnel. The *outer* fallbacks over `AccessAuthority` (e.g.
   `enforce_required_action`'s outer `_ =>`, `require_system_access`'s `authority.ready()` else-branch)
   are the **engine-unavailable 500 path** — they stay `Failed`/500 and must never enter the funnel, or
   an outage fires deny dashboards (contradicting the not-emitted rule below). The plan names the exact
   arm per site.
2. **MCP gates** (`crates/ui/mcp/src/auth.rs`, both `mcp:use` auth paths, **plus the per-tool
   action-gate deny arm in `oauth/tool_auth.rs`** — its actions are non-qualifying today, but wiring it
   through the shared predicate now means a future tool declaring a qualifying action is covered instead
   of silently bypassing the funnel forever): emit `access.denied` alongside the existing `Denied`
   auth-audit rows (owner decision 2). These emissions cannot reuse the web-api helper (dependency
   direction — `uptrakit-mcp` does not depend on `uptrakit-web-api`); they share only the predicate from
   `uptrakit-shared-types`. The enclosing fns are **not** discoverable by the audit-coverage walker
   (plain helpers — not utoipa/axum handlers, not executors, not `#[audit_required]`) and have no
   existing catalog rows, so each new emission site gets `#[uptrakit_audit_log::audit_required]` (on the
   fn or a small local emit helper) **plus its own `audit-catalog.toml` row** in the same task. The
   MCP sites **emit only** — their existing inline counter increments stay untouched (mirroring the
   web-api helper's increment-and-emit shape there would double-count every MCP deny, and the metric
   has no test to catch it).

**Flood posture, costed**: qualifying denials emit unconditionally — the corpus resolved "no
per-principal repeat detection in v1", so a client hammering a qualifying gate (an `mcp:use`-less MCP
client retrying, or renders of a surface whose provider declared a qualifying `required_action`) writes
one Event per attempt into the audit store. Two distinct modes, both accepted for the v1 deployment
posture (single-owner, small-fleet): the **row-count** mode (store growth — rate × retention, default
90 days via the configurable `audit_log.n_days`) and the **process-memory** mode (the audit
dispatcher's channel is unbounded by design — "never dropped due to backpressure" — so a sustained
retry loop grows the queue in RSS if the backend falls behind). Unauthenticated-input-driven emissions
on this channel are not new (failed-login and MCP auth `Denied` rows predate this milestone); this
milestone adds one more such path and roughly doubles the MCP one. The corpus's repeat-detection
deferral is the revisit trigger, and the memory mode is the one to monitor. Do not add a suppression
window in this milestone — that would re-litigate the resolved question.

**Event shape**: `builder_event(ACCESS_DENIED)`, outcome `Denied`, actor = the denied user,
target `("action", <action string>)` (for OR-gates: the first qualifying alternative as target, all
alternatives in details), details `{ "action": …, "reason": <DenyReason::as_str> }`.
Scope: `system_scope()` when the denied action's resource is `system.*`; `tenant_scope(active_tenant)`
otherwise. **Not emitted** for: 401s (no principal), engine-unavailable 500s (`Failed`, not `Denied` —
preserving the interactive-WS convention that outages must not fire deny dashboards), or any
non-qualifying action's denial (D12's negative leg).

### 5. `bearer_token` retirement

+ Convert `initiate_email_change`, `change_password`, `cancel_email_change` (`routes/users.rs`) to
  `security(("oauth2" = []), ("developer_token" = []))` — annotation-only; their inline self-service
  authorization logic is untouched.
+ Delete the `bearer_token` registration from `SecurityAddon` in `router.rs`.
+ Delete the now-dead `"bearer_token" ⇒ "unconverted"` branch in `integration_tests/scope_map.rs`;
  regen the scope-map golden (the three ops move to the authenticated-only class; the `"unconverted"`
  class empties out).
+ Update the stale survivors sentence in `crates/ui/web-api/AGENTS.md` and the `bearer_token` example
  snippet in `docs/development/coding-standards.md`.
+ `./scripts/regen-api.sh`: `openapi.json` drops the `bearer_token` security scheme and the five
  operations' security arrays change; frontend regen in the same commit.

### Alternatives considered

+ **Scope presets fully expanded per-caller now** — rejected: pulls M1.7's grant→action expansion into
  M1.6b for a consumer that arrives in M4.2. **Deferring the section entirely** — rejected: diverges from
  the corpus three-section shape and E8.
+ **Catalog reads `SurfaceRegistry`/`PluginOps` directly** — rejected: catalog and engine could drift;
  tenant-filtered listing would disagree with `is_registered`.
+ **Counting MCP's existing `Denied` rows as the `mcp:use` deny Event** — rejected: deny observability
  split across three action names; uniform `access.denied` is one dashboard/query surface.
+ **Dedicated `users apply-bundle` CLI command** — rejected: adds a bundle-aware command M2.6 may
  reshape; `set-roles --names` + catalog lookup covers tier application.
+ **Deferring `bearer_token` deregistration to M1.7** — rejected: the tracker's last-reference rule
  assigns it here, and only three annotation-swap conversions block it.

## Testing

Rows from `12-test-plan.md` owned here: **E8, E9, D12** (+ the CLI command test from the task
breakdown). Standing rules apply: `TestApp` harness, success + failure paths, no upstream-behavior
tests. **Principal staging**: every deny-side test stages its principal via `stage_user_with_grant` /
`stage_zero_role_user` — never `register_user`/`register_and_get_token` (first registration triggers
owner bootstrap and silently grants every role, making deny tests vacuous; this trap has recurred even
in briefs that quoted it).

+ **E8 — catalog shape** (`integration_tests/access_catalog.rs`, new):
  + Three sections present. Built-in leg: assert the actions set equals the `CATALOG`-derived
    expectation **and** pin positive literals (e.g. `hosts:read` present with
    `selector_support: "host"`, `updates:trigger` with `"host_and_software"`, `settings.auth:manage`
    with `"none"`) plus a negative literal (no `hosts:approve`).
  + Role bundles: five tiers; pin one full bundle (e.g. `owner` roles list).
  + Scope presets: `all-reads` is `static` with a non-empty concrete-`:read` list (pin `hosts:read`
    in, `updates:trigger` out); `all-my-current-actions` is `caller_actions` with no list; the
    reviewed-list guard test reds in **both** directions — a new `read`-verb action in `CATALOG`
    unlisted, and a const entry no longer present in `CATALOG`; an unknown `kind` string deserializes
    to the wire-safe fallback in the client types instead of erroring.
  + Authenticated-only: no credential → 401; a zero-grant principal (`stage_zero_role_user`) → 200.
  + Dynamic leg: register a surface via `SurfaceRegistry::register_service` → `surface.<id>:use`
    appears; `unregister_service` → gone. (`register_service`/`unregister_service` is the sanctioned
    register→deregister pair — `bootstrap_plugin` has no deregistration counterpart.) A surface with an
    `_`-containing ID appears in neither `registered_actions` nor the catalog (grammar-mismatch
    fail-closed leg). Pinned literals are the load-bearing assertions; the set-equality check is
    supplementary (it shares its derivation with production).
+ **E9 — routes gone, not stubbed**: `GET /api/v1/access-presets` and
  `POST /api/v1/users/{id}/apply-preset` return 404/405 through the full router.
+ **D12 — deny Events** (positive and negative):
  + Staged principal with an unrelated grant hits an `access:manage`-gated route → 403 + one
    `access.denied` row (assert action string, reason, target, tenant scope).
  + `system.*` deny (e.g. via a `require_system_access` path) → `access.denied` with system scope.
  + `mcp:use` deny (MCP crate test beside `get_current_user_mcp.rs`) → `access.denied` emitted; the
    pre-existing auth-audit row still present.
  + Negative: same staged principal denied on an ordinary action (e.g. `hosts:read`-gated route) →
    **no** `access.denied` row.
  + Negative (OR-gate rule): a mixed OR-gate denial (e.g. `list_plugin_types`, whose alternatives mix
    `software:read`/`settings:read` with `system.settings:manage`) → **no** `access.denied` row; a
    system-plane denial through a `system_services` route (a single-element slice through the OR
    funnel — see the correction above) → one system-scoped row.
  + Negative (outage ≠ deny): a request hitting a qualifying gate under `AccessAuthority::Unavailable`
    → 500 and **no** `access.denied` row — pins the engine-unavailable exclusion so the
    wrong-wildcard-arm implementation cannot ship green. This pins one funnel site; the plan targets a
    two-level-match site (`enforce_required_action` or `require_system_access`) where the mis-wire risk
    lives. Unit-level staging precedent already exists —
    `require_system_access_unavailable_authority_is_500` (`middleware/action.rs`) constructs
    `AccessAuthority::Unavailable` directly — so the unit-level shape is the expected outcome
    (full-router staging would require a failing engine); the remaining funnel sites' outage arms are
    recorded as unpinned.
  + Deleting the qualifying-predicate gate must red at least one of these (the negative + positive pair
    together make the predicate load-bearing).
+ **CLI**: parse tests for `set-roles` with UUIDs, with `--names`, and the both/neither rejections;
  name-resolution failure surfaces the unresolved names.
+ **Registry trait change**: every `DynamicActionRegistry` impl (production + test doubles) gains
  `registered_actions`; the `SurfaceActionRegistry` unit tests extend to cover
  enumeration-matches-`is_registered`.
+ **Goldens tripped by this work** — in the causing task's own gate list (they live in sibling crates
  and are invisible to scoped `-p` runs): the audit-registry parse/count test in
  `xtask/tests/audit_registry_parse.rs` (new `access.denied` action), `scope-map.golden.json`,
  `openapi_json_is_up_to_date` (note: `#[cfg(all(feature = "oidc", nats, reset-data))]` — run it under
  a feature set where it actually executes and assert a non-zero test count), and the frontend
  generated-client staleness gate.

## Quality gates

Standard for touched areas: `cargo fmt --all`; clippy + tests on both canonical feature worlds
(`--no-default-features --features db-sqlite` and `--all-features` with `frontend/build/` present);
`./scripts/regen-api.sh` (openapi.json + frontend generated + openapi-client sync) in the same commits
as route changes; `cargo xtask audit-coverage-check` and `cargo xtask openapi-client-check`;
`ci/verify_action_security_declarations.py`; `python3 ci/verify_db_access_policy.py`;
`bash ci/verify_typed_audit_actions.sh`; markdownlint scoped to changed files. No wire-type changes —
asyncapi untouched. **No new external dependencies.**

**Sequencing**: three of the four deliverables mutate the same generated artifacts (`openapi.json`,
`frontend/src/lib/api/generated/`, `scope-map.golden.json`) under the regen-in-originating-commit rule —
the plan's tasks run **sequentially in one branch**, recommended order: deny Events first (touches no
generated artifact), then catalog, then preset deletion, then `bearer_token` retirement. No parallel
worktrees over these tasks. The `"bearer_token" ⇒ "unconverted"` branch deletion in `scope_map.rs` is
checklist-enforced, not compiler-enforced (a string match arm survives `warnings = deny`) — it goes on
the retirement task's explicit step list.

## Documentation deliverables (this milestone's surgical edits; M1.9 owns the full rewrites)

Task-breakdown rule: doc edits land inside the task that causes them. M1.6b excises every reference to
the deleted endpoints/commands and documents what it adds; M1.9 does the full model rewrite.

+ `docs/api/access-management.md` — delete the "Preset endpoint (interim state)" section (it names
  M1.6b as its own executioner); add the catalog endpoint section (shape, auth class, the three
  sections, the `caller_actions` marker semantics); document `set-roles --names`.
+ `docs/api/user-management.md`, `docs/api/http-web-api.md` — remove the preset endpoint rows; add the
  catalog row.
+ `docs/security/auth-and-authorization.md` — excise the preset-endpoint mentions (targeted lines only).
+ `docs/end-user/user-management.md` — replace preset CLI guidance with `set-roles`/`--names` + catalog
  bundle lookup.
+ `docs/README.md` — fix the two index references if they name the deleted material.
+ `crates/ui/web-api/AGENTS.md` — legacy-survivors sentence (also stale re `roles.rs`).
+ `docs/development/coding-standards.md` — `AccessPreset` allowlist line → `RoleBundle`; `bearer_token`
  example snippet.
+ Rustdoc on the new public items (catalog DTOs, `RoleBundle`, `registered_actions`, the deny-Event
  helper) per the document-everything invariant.
+ **No new ADR**: the preset demotion and deny-observability decisions are corpus-recorded
  (`06-grant-model.md`, `09-resolved-questions.md`); the model-replacement ADR is M1.9's deliverable.

## Out of scope / deferred

+ `GET /api/v1/permissions` deletion — **M1.8** (its doc comment already points here for functional
  supersession).
+ `Permission` enum, `permission_extractor!` contents, `has_permission` remnants — M1.8.
+ Grant/role/selector CLI authoring affordances beyond `set-roles --names` (pattern autocomplete,
  selector steering) — M2.6.
+ Scope-preset consumption (token-creation UX) — M4.2; caller-actions expansion — M1.7 (`me`).
+ Per-caller scoping of the catalog's dynamic section — deferred with its corpus trigger.
+ Plugin `DynamicActionRegistry` impl — with the first plugin declaring action verbs (post-v1).
+ Per-principal deny repeat detection — explicitly none in v1.
+ Tightening surface-ID registration admission to the `Action` resource grammar (today
  `validate_surface_identifier` admits IDs whose `:use` action is unparseable, making those surfaces
  permanently ungrantable — `registered_actions` skips them fail-closed; an admission-time rejection is
  a separate, wire-affecting decision).
+ Suppressing pre-existing domain-level `Denied` rows at qualifying deny sites (duplicates accepted;
  see §4).

## Ledger conformance notes (for the plan author)

+ Deny tests: no bootstrap-privileged principals (row 16); stage via fixtures, re-login after strips.
+ `DynamicActionRegistry` trait change: run the `impl` grep at plan time; list every double (row 18).
+ Audit catalog: the web-api helper fn needs a row; the MCP emission sites are walker-invisible plain
  helpers with no existing rows — each gets `#[audit_required]` + its own row (verified against the
  walker's four detection kinds; row 31).
+ Goldens in sibling crates (audit-registry count, scope-map, openapi staleness) go in the causing
  task's own gate list; feature-gated goldens need a world where they run, with non-zero test count
  asserted (row 57).
+ Deletion sweeps: delete ranges start at doc comments/attributes; no commit leaves a callerless
  private fn under `warnings = deny`; grep deleted symbols across `cfg(test)` and doc-comment lines
  workspace-wide (rows 20, 21).
+ Every planned commit leaves the whole tree green; regen artifacts land in the originating commit —
  no expected-red windows (rows 43, 52).
