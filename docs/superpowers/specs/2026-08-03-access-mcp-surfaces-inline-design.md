# M1.5 — MCP, surfaces, and non-extractor enforcement sites onto the engine

Date: 2026-08-03. Status: approved design, pending plan.

Fifth task of the authn/authz refactoring Milestone 1 (sources of truth:
`.superpowers/authn-and-authz-refactoring/`, esp. `07-decision-and-enforcement.md` §MCP tools /
§Shared surfaces / §Interactive update WS and inline handler checks, `05-action-model.md` §Dynamic
namespaces, `09-resolved-questions.md` §Action model #3 / §Sequencing with pending specs #2/#4/#5,
`11-task-breakdown.md` §M1.5, `12-test-plan.md` §I + D15/F9; and the sibling specs
`2026-07-28-access-types-core-design.md` (M1.1), `2026-07-28-access-grant-storage-design.md`
(M1.2), `2026-07-28-access-engine-design.md` (M1.3),
`2026-07-29-access-rest-extractor-scaffolding-design.md` (M1.4a), whose surfaces this spec
consumes). Owner-settled decisions are applied, not reopened.

**Implementation sequencing**: M1.1–M1.4a have landed (verified on `main` at spec time — the
M1.4a train ends at `02b50abe9`; `action_extractor!`, `AccessState`, `AccessAuthority`, the
oauth2/developer_token schemes, and the hosts reference family are live). M1.5 runs **parallel to
M1.4b and M1.6a** per the task breakdown; the file-overlap rule applies — this task owns
`routes/surfaces.rs`, `routes/system_services.rs`, `routes/plugin_type_settings.rs`,
`routes/plugin_configs/crud.rs`, `routes/services/batch.rs`, `routes/interactive_ws.rs`, and the
`visibility` module for its inline conversions; the M1.4b spec
(`2026-08-03-access-route-sweep-design.md`, decision 3) locks the resolution: its final batch B6
carries the **extractor/declaration** work on these same files — `#[utoipa::path]` security
blocks, `x-action-dynamic` on the dynamic surface wrappers (eight at spec time; the set is
defined by the `dynamic:` extension grep, not a fixed count) — and lands only after M1.5
merges.

## Problem / goal

Move every non-REST-extractor enforcement surface onto the `AccessEngine`:

- **MCP** (crate `uptrakit-mcp`): the `mcp:use` connection gate on both auth paths, per-tool
  engine checks, deletion of `load_user_permissions_for_mcp`.
- **Surfaces**: `required_permission` → `required_action` (string on the wire, parsed to `Action`
  at admission, typed in the registry), the engine call inside the shared resolution path's
  permission step (404 → 403 → 405 order preserved), provider-origin fail-closed rules re-typed,
  and the declaration sweep across **every** crate declaring surface permissions.
- **Live `DynamicActionRegistry`**: the engine finally gets a registry (`with_registry`), so
  `surface.<id>:use` grants become decidable against the live surface registry.
- **Non-extractor sites**: the interactive update WS gate and every inline handler-body
  `has_permission` conversion.

After this task, the only consumers of the legacy `Permission` world are the not-yet-converted
route families (M1.4b batches B1–B6; `users.rs`/`roles.rs`/`access_presets.rs` ride to
M1.6a/M1.6b per that spec's decision 2) and the machinery M1.7/M1.8 delete.

## Decisions locked during grilling (owner, 2026-08-03)

1. **Interactive-WS deny stays HTTP, not close frames.** The test plan's D15 wording ("close-frame
   semantics, not HTTP 403") mis-describes the live code: the `TriggerUpdates` check runs
   **before** `ws.on_upgrade()` (`routes/interactive_ws.rs:184-196`), so denials are plain HTTP
   401/403 responses and the existing tests assert HTTP statuses. Owner decision: preserve the
   verified current behavior (engine check pre-upgrade, HTTP error responses). D15 is satisfied as
   "deny before any WS session exists", asserted as HTTP statuses; the refactoring docs' close-frame
   wording is a factual error, not a requirement.
2. **M1.5 owns the whole inline handler-body class — five route files plus the `visibility`
   module.** The `07` doc's three-file enumeration was a floor: a fresh grep adds
   `routes/plugin_configs/crud.rs:100` (OR-of-three) and `routes/services/batch.rs:76` (batch
   action → permission match). All five convert here; M1.4b stays purely the
   extractor/declaration sweep. The plan re-runs
   `rg -n "has_permission\(" crates/ui/web-api/src/` at write time and covers every hit except
   the legacy `permission_extractor!` body (`middleware/permission.rs:73`) and the
   `AuthenticatedUser::has_permission` definition itself, both deleted in M1.8.
3. **Unparseable `required_action` from the wire → whole registration rejected.** Surface
   permission values are **not** human-configurable: every current source is a compiled constant
   (verified — `AgentInteraction.permission` is fed only by `with_permission(...)` calls in
   agent-ssh-runtime; the proxmox agent actions declare none; plugin/mqtt descriptors are
   builder literals). After the sweep they are typed `Action` constants, so garbage can only
   arrive from a service sending a bad string over the wire — a broken app. Owner decision:
   reject the registration wholesale at admission (test I5 as written), no per-row skip
   machinery, no notification channel. Let the incorrect app fail.
4. **Design-approval round**: typed internal APIs throughout (builders take `Action`, descriptors
   keep the canonical string on the wire); `get_current_user` returns a catalog-expanded action
   list; MCP per-tool enforcement unified behind one helper so declaration = enforcement.

## Verified current state (2026-08-03, live tree)

### MCP (`crates/ui/mcp`)

- Token dispatch by prefix at `src/auth.rs:107-121` (`upk_` → API token, JWT → OAuth).
- **API-token path** `validate_api_token_for_mcp` (`auth.rs:243-319`): gate at :293
  `if !auth_user.has_permission(Permission::AccessMcp)` → audit reason
  `"missing_access_mcp_permission"` → `McpAuthError::Forbidden`.
- **OAuth JWT path** `validate_oauth_access_token_for_mcp` (`auth.rs:338-445`): gate at :416
  `if !permissions.contains(&Permission::AccessMcp)` over the `Vec<Permission>` returned by
  `load_user_permissions_for_mcp` (`auth.rs:476-515`, sole caller :407 — a hand-rolled
  role→role_permission→permission join duplicated because `uptrakit-mcp` must not depend on
  `uptrakit-web-api`).
- `Forbidden` renders 403 at `auth.rs:142-145`. `McpAuthError` (`context.rs:91-106`) carries
  `MissingCredentials / JwtNotAccepted / Unauthorized / Forbidden / Internal`.
- `McpRequestContext` (`context.rs:33-66`): `{ user_id, token_id, tenant_id, permissions:
Vec<Permission>, auth_method }` + `has_permission()` (:63-65); inserted into request extensions
  by the auth layer (:125), read by tools via `FromContextPart` (:72-84).
- Per-tool declaration: `const ToolAuth { required_scopes: &'static [McpScope],
required_permissions: &'static [Permission] }` (`oauth/tool_auth.rs:10-15`);
  `require_scopes` (:31-55) enforces **only scopes** — the permission half is enforced by
  hand-written inline `ctx.has_permission(...)` per tool (`tools/history.rs:207,:283`,
  `tools/update.rs:59`; `get_current_user` declares `&[]` and checks nothing). Declared-but-
  unenforced drift is structurally possible today.
- `get_current_user` returns `permissions: Vec<String>` in its result (`tools/user.rs:29,:47-58`).
- `McpState` (`state.rs:23-37`, `#[non_exhaustive]`, sole constructor `McpState::new`): no engine
  field. Single production construction site: `crates/core/controller-runtime/src/server.rs:47`,
  with `cfg.app_state` in scope (which since M1.4a carries `access_engine`).
- `uptrakit-mcp` already depends on `uptrakit-controller-core` and `uptrakit-shared-types`.

### Engine surface consumed (M1.3/M1.4a, landed)

- `AccessEngine::context(tenant_id, user_id, scope) -> Result<AccessContext>` (async,
  `crates/ui/controller-core/src/access/mod.rs:149-173`); `authorize(&ctx, &Action,
Option<&TargetRef>) -> Decision` (pure/sync, :228); dynamic-action gate at :234-240 — a
  `plugin.*`/`surface.*` action is `Deny(UnknownAction)` unless
  `registry.is_registered(action)`; `with_registry` builder (:135) — **no production registry
  injected yet** (`AccessEngine::new` comment: "every dynamic action denies until M1.5 injects
  one"). `pub trait DynamicActionRegistry: Send + Sync { fn is_registered(&self, action:
&Action) -> bool; }` (:78-81).
- Web-api side: `AccessState(pub Arc<AccessEngine>)` sub-state (`app_state.rs:152-153`, `FromRef`
  :1286-1289); `AccessAuthority::Ready(ctx) | Unavailable` marker built in `require_auth`
  (`middleware/require_auth.rs:201-212`) for every authenticated HTTP request; the
  `action_extractor!` verdict set (401 missing user / 500 missing marker / 500 `Unavailable` /
  403 deny + `uptrakit_access_denies_total{reason}` counter / allow) in `middleware/action.rs`.
- Catalog consts verified present (`crates/shared/types/src/access/catalog.rs`): `MCP_USE`,
  `SOFTWARE_READ`, `SOFTWARE_UPDATE`, `UPDATES_TRIGGER`, `HOSTS_UPDATE`, `SETTINGS_READ`,
  `SYSTEM_SETTINGS_MANAGE`, `NOTIFICATIONS_READ`, `NOTIFICATIONS_MANAGE`,
  `SYSTEM_SERVICES_UPDATE`, `SYSTEM_SERVICES_APPROVE`, `SYSTEM_SERVICES_REJECT`,
  `SYSTEM_SERVICES_DELETE`, `SERVICES_APPROVE`, `SERVICES_REJECT`, `SERVICES_DELETE` (+ `_STR`
  twins).
- The M1.3 `Permission→Action` shim (`access/shim.rs`, `actions_for_permission`) has **zero call
  sites**; M1.5 does not use it — conversions reference catalog consts directly.

### Surfaces

- Field `required_permission: Option<String>` on `InteractionDescriptor`
  (`crates/shared/surfaces/src/interaction.rs:80-81`) and `SurfaceDescriptor`
  (`crates/shared/surfaces/src/surface.rs:335-336`; builder setter
  `required_permission(impl Into<String>)` :448-450). Wire length caps in
  `crates/shared/wire/src/wire_validate_impls.rs:537-540` / :790-793.
- **Admission parse site**: `validate_registration_basics`
  (`crates/ui/surface-proxy/src/registry.rs:634`, called from all three admission entry points
  :238/:307/:339) parses descriptor and interaction permissions to `Permission` at :786-794 /
  :797-805 and rejects on failure. **Correction to the milestone text**: the parse does _not_
  live in `validate_registration_admission_locked` (:965-994 — that fn does provider/binding
  conflict + contract-collision checks); the `Action` parse lands where the `Permission` parse
  is today, which is reached from the same admission choke points.
- Registry stores descriptors **as strings** end-to-end; the `MethodNotAllowed` lookup-error
  variant carries `descriptor_required_permission: Option<String>` +
  `interaction_required_permissions: Vec<Option<String>>` (registry.rs:1065-1075, built at
  :1082-1102) to feed the web-api 403-before-405 checks.
- Enforcement helper `enforce_required_permission(Option<&str>, &AuthenticatedUser, …)`
  (`crates/ui/web-api/src/routes/surfaces.rs:1510-1527`) — parses `Permission` (infallible
  `FromStr`, comment :1517) and calls `has_permission`. Resolution order: read path
  (surfaces.rs:214-237) 404 → 403; method-invoke path (:357-455) resolves → on
  `MethodNotAllowed` runs descriptor **and every sibling interaction** permission check
  (403) before returning 405 (anti-probe comment :376-379); resolved match checks descriptor
  then interaction (:427-455).
- Provider-origin fail-closed rules: `crates/ui/surface-proxy/src/proxy.rs:313-320`
  (Provider origin + `required_permission.is_some()` + `!provider_invocable` → denied);
  `proxy/prepared.rs:56-62` (same but **without** the `provider_invocable` escape);
  admission-time `InteractionDescriptor::validate_for_provider`
  (`interaction.rs:242-255`: `provider_invocable` + permission + service provider → rejected).
- **Surface listing is not permission-filtered today**: `list_surfaces`
  (`routes/surfaces.rs:50-63`) filters by `SurfaceProviderVisibility` only — `07`'s "listing
  filters to surfaces whose `required_action` the caller holds, as today" is wrong about
  "today"; the held-action listing filter is M2.4's deliverable (test I3's listing half lands
  there; its **invoke** half lands here).
- **Declaration sites** (spec-time inventory; plan re-greps `required_permission` +
  `with_permission` workspace-wide):
  - `crates/core/agent-ssh-runtime/src/surface_runtime.rs` — `.required_permission(
Permission::UpdateHosts.to_string())` (:258); `with_permission(Permission::UpdateHosts)`
    on `AgentInteraction`s (:163-188, :912, :993); `permission_or_none(&action.permission)`
    (:660, fn :721-728) mapping `""`/`"none"` → `None`; read-back at :2484.
  - `crates/plugins/releases/docker/src/plugin.rs` — `Permission::UpdateSoftware` (:296, :320,
    :390).
  - `crates/plugins/infrastructure/proxmox/src/plugin.rs` — `Permission::UpdateHosts`,
    `ManageGlobalSettings`, `ViewSoftware`, `UpdateSoftware` across ~25 sites (:90-:1129).
  - `crates/plugins/infrastructure/core/` — `AgentInteraction.permission: String`
    (`agent_interaction.rs:48`, `""` = none); `surface_form_authoring.rs` builder field
    `required_permission: String` (:22, setter :52).
  - `crates/core/mqtt-runtime/src/surface_runtime.rs` — raw literals
    `"update_system_services"` (:83, :793-:840). (`uptrakit-mqtt-runtime` does **not** depend
    on `uptrakit-shared-types` today.)
  - `crates/plugins/notifications/{telegram,webhook,email}/src/plugin.rs` — raw literals
    `"view_notifications"`, `"manage_notifications"`, `"manage_global_settings"` (the C5
    sites).

### Interactive update WS + inline sites

- `routes/interactive_ws.rs`: custom `?token=` auth (browser WS cannot set headers — approved
  extractor exception, comment :177-183), JWT path :156, API-token path above it; inline gate
  :184 `if !auth_user.has_permission(Permission::TriggerUpdates)` → denied audit
  (`"permission_denied"`, :193) → HTTP 403 `error_response` (:196). Upgrade happens later
  (:333). Because auth is bespoke, `require_auth` middleware does **not** insert
  `AccessAuthority` for this route — the handler must build its own context.
- Inline `has_permission` handler-body sites (`crates/ui/web-api/src/`):
  `visibility.rs:63` (`is_plugin_visible_to_user` :49-74 — `enabled ||
has_permission(ManageGlobalSettings)`, shared by route handlers and the surface-registry
  visibility path); `routes/system_services.rs:475-481` (batch action string →
  `Approve/Reject/RemoveSystemServices` → check); `routes/services/batch.rs:70-76` (same shape,
  tenant tier); `routes/plugin_type_settings.rs:82-85` (`ViewSettings ||
ManageGlobalSettings`); `routes/plugin_configs/crud.rs:100-105` (`ViewSoftware ||
ViewSettings || ManageGlobalSettings`); `routes/surfaces.rs:1519` (inside
  `enforce_required_permission`, §Surfaces above).

## Scope

In: MCP conversion (engine in `McpState`, both gates, per-tool actions + shared helper,
`load_user_permissions_for_mcp` deletion, `get_current_user` action list);
`required_permission` → `required_action` re-type across `uptrakit-surfaces`, `uptrakit-wire`
validation, surface-proxy admission/registry/proxy, web-api enforcement, and every declaration
crate; the live `DynamicActionRegistry` adapter + `with_registry` wiring; the interactive-WS
gate conversion; the five inline route files + `visibility` module; regen artifacts
(`./scripts/regen-api.sh`, `scripts/regen-asyncapi.sh`); tests per §Tests; doc touches per
§Documentation deliverables.

Out (deferred to the named tasks): the dynamic surface wrappers' OpenAPI declarations
(`x-action-dynamic: true`; eight at spec time) and all other route-family `#[utoipa::path]`
conversions (M1.4b, `2026-08-03-access-route-sweep-design.md` — batch B6 lands after this task);
surface **listing** filtered by held action + MCP list-tool visibility (M2.4); deny audit
**Events** for `mcp:use` (M1.6b — the counter/trace tier ships here); claims/`me`/frontend
(M1.7 — the SPA's generated types pick up the `required_action` field rename via the regen
committed here, but gating logic changes are M1.7's); `Permission` +
`AuthenticatedUser::has_permission` + shim deletion (M1.8); canonical doc rewrite + ADR (M1.9);
MCP OAuth scope intersection — `McpScope` stays the only MCP scope mechanism until M3.3, so the
engine's scope term receives `None` on every MCP path; selector fine-checks incl. the WS attach
check (M2.3). The long-lived-session residual is restated, not resolved: the WS session checks
authority once at handshake; mid-session grant revocation does not terminate it (unchanged from
today; M2.3 adds the attach-time fine check).

## Design

### 1. MCP: engine into `McpState`, gates onto `authorize`

- `McpState::new` gains a parameter `access_engine: Arc<AccessEngine>`; field
  `pub access_engine: Arc<AccessEngine>`. The single production call site
  (`controller-runtime/src/server.rs:47`) passes `Arc::clone(&cfg.app_state.access_engine)` —
  the **same instance** as REST (one cache, one NATS invalidation listener; never a second
  engine). Test fixtures construct from their existing DB handle
  (`AccessEngine::new(db.clone())` or clone the harness engine); the plan re-greps
  `McpState::new(` for every call site.
- Both validators build the context after authentication succeeds:
  `state.access_engine.context(state.default_tenant_id, user_id, None).await`. Scope is `None`
  on **both** paths: `upk_` tokens carry no scopes until M4, and MCP OAuth JWT scopes are
  `McpScope` values (`mcp:read`/`mcp:write`), which are _not_ action patterns — feeding them to
  the engine would mistranslate them into a deny-all action ceiling. `require_scopes` remains
  the sole `McpScope` enforcement until M3.3 re-scopes MCP.
- Context-build failure → `McpAuthError::Internal` (renders 500) — fail-closed, matching the
  REST extractor's `Unavailable` ⇒ 500 rule; never a fallback to the legacy permission load.
- The gate on both paths becomes
  `engine.authorize(&ctx, &actions::MCP_USE, None)`; `Decision::Deny(_)` → the existing audit
  emission with reason `"missing_access_mcp_permission"` (string kept — it names the same
  semantic fact and audit consumers key on it) + `McpAuthError::Forbidden`, plus the
  `uptrakit_access_denies_total{reason}` counter (M1.4a convention; same
  `metrics::counter!(...).increment(1)` shape).
- `load_user_permissions_for_mcp` is deleted with its sole caller's plumbing; the OAuth path no
  longer loads permissions at all.

### 2. MCP: request context + unified tool enforcement

- `McpRequestContext.permissions: Vec<Permission>` is replaced by `access: AccessContext`
  (`Clone` since M1.4a); `has_permission()` is deleted. The constructor signature changes
  accordingly; `auth_method`, ids, and `FromContextPart` stay.
- `ToolAuth.required_permissions: &'static [Permission]` → `required_actions: &'static
[Action]` (`GET_CURRENT_USER_AUTH` → `&[]`; history tools → `&[actions::SOFTWARE_READ]`;
  `TRIGGER_UPDATE_AUTH` → `&[actions::UPDATES_TRIGGER]`).
- New helper beside `require_scopes` (same module, same error style):
  `require_tool_auth(state: &McpState, ctx: &McpRequestContext, auth: &ToolAuth) ->
Result<(), ErrorData>` — checks scopes via the existing `require_scopes` logic, then
  `engine.authorize(&ctx.access, action, None)` for every entry in `required_actions`
  (deny → the same 403-shaped `ErrorData` the inline checks produce today + deny counter). Every
  tool calls this **one** helper; the hand-written per-tool `ctx.has_permission` lines are
  deleted. This closes the latent declared-but-unenforced drift: a future tool whose `ToolAuth`
  declares an action gets enforcement by construction.
- Tools reach the engine through `self.state` (the handler owns `McpState`); `authorize` is
  pure/sync, so per-call cost is in-memory evaluation.

### 3. MCP: `get_current_user` result shape

`GetCurrentUserResult.permissions: Vec<String>` → `actions: Vec<String>`: the concrete built-in
actions the caller may perform, computed by iterating `CATALOG` and keeping every action where
`engine.authorize(&ctx.access, action, None)` is `Allow` (wildcard grants therefore expand;
dynamic `plugin.*`/`surface.*` entries are excluded — registry-driven expansion is M1.7's `me`
machinery, and MCP follows `me`'s final shape then). Serde field renamed; the MCP integration
test updates. Breaking output change, allowed by the milestone rules (MCP clients re-authorize
across M1–M3 anyway).

### 4. Surfaces: `required_action` on the shared types

- Rename field + serde key on both descriptors: `required_permission` → `required_action`
  (`Option<String>` stays — string on the wire per `09` §Sequencing #2; schemars output stays a
  plain optional string). The builder setter becomes
  `required_action(action: Action)` storing `action.to_string()` — declaration sites are
  compile-checked against the catalog while the wire type is unchanged. (No `Option<Action>`
  field on the wire type: the "actions never cross the service wire as a type" property is
  load-bearing for the no-`Other` decision.)
- `uptrakit-wire` `WireValidate` impls: field rename only, same length caps.
- `AgentInteraction.permission: String` (infrastructure-core) → `required_action:
Option<Action>`; the `with_permission` builder method becomes `with_required_action(Action)`;
  `permission_or_none` in agent-ssh-runtime is deleted — the descriptor assignment (pinned)
  becomes `descriptor.required_action = action.required_action.map(|a| a.to_string())`.
  `surface_form_authoring.required_permission: String` → `required_action: Option<Action>` with
  the same treatment.
- Declaration sweep (all values map per `05`'s normative table):
  - agent-ssh-runtime: `Permission::UpdateHosts` → `actions::HOSTS_UPDATE`.
  - docker: `Permission::UpdateSoftware` → `actions::SOFTWARE_UPDATE`.
  - proxmox: `UpdateHosts`/`ManageGlobalSettings`/`ViewSoftware`/`UpdateSoftware` →
    `HOSTS_UPDATE`/`SYSTEM_SETTINGS_MANAGE`/`SOFTWARE_READ`/`SOFTWARE_UPDATE`.
  - mqtt-runtime: `"update_system_services"` → `actions::SYSTEM_SERVICES_UPDATE` (manifest gains
    `uptrakit-shared-types = { workspace = true }` — already a workspace dependency).
  - notifications email/webhook/telegram: `"view_notifications"` → `NOTIFICATIONS_READ`,
    `"manage_notifications"` → `NOTIFICATIONS_MANAGE`, `"manage_global_settings"` →
    `SYSTEM_SETTINGS_MANAGE`.
- Rename sweep discipline (ledger): inventory by site class — exact field uses, serde
  fixtures/goldens, test-harness stub descriptors (`test_harness/mod.rs:157,:188`), e2e/frontend
  generated types (regen), doc prose (`docs/security/surfaces.md`, `docs/development/surfaces.md`,
  root `AGENTS.md`), and the `asyncapi.yaml` regen. The plan runs
  `rg -n "required_permission" --hidden -g '!target'` workspace-wide and classifies every hit.

### 5. Surfaces: admission parse + typed registry

- In `validate_registration_basics` (registry.rs:786-805) the two `parse::<Permission>()` checks
  become `parse::<Action>()` (real `FromStr` with `ParseActionError` — note the legacy parse was
  infallible, so this is the first time admission can actually reject; test I5 becomes
  meaningful). Unparseable descriptor or interaction `required_action` → the existing
  `SchemaOrLimitFailure` rejection path, whole registration refused (owner decision 3), both the
  service-WS and plugin/built-in registration entry points (:238/:307/:339).
- The registry stores the parsed value: the stored surface/interaction entries gain
  `required_action: Option<Action>` populated at admission (parse once, evaluate many).
  `uptrakit-surface-proxy` already depends on `uptrakit-shared-types`, so `Action` is available
  without new edges. The `MethodNotAllowed` error variant re-types to
  `descriptor_required_action: Option<Action>` /
  `interaction_required_actions: Vec<Option<Action>>`, so the web-api 403-before-405 sibling
  checks receive typed values and never re-parse.
- Dynamic `surface.*`/`plugin.*` values are parseable `Action`s and therefore admissible as
  `required_action` — decision-time registry membership (not admission) decides whether they
  ever allow (`05` §Dynamic namespaces: fail closed, no dangling authority).

### 6. Surfaces: engine call in the shared resolution path

`enforce_required_permission` (routes/surfaces.rs:1510-1527) becomes
`enforce_required_action(required: Option<&Action>, authority: &AccessAuthority, engine:
&AccessEngine, surface_id: &str, access_kind: &'static str) -> Option<Response>`:

- `None` action → allow (`None`), as today.
- `AccessAuthority::Ready(ctx)` → `engine.authorize(ctx, required, None)`; `Deny(_)` → the
  existing 403 body + deny counter; `Allow` → `None`.
- `AccessAuthority::Unavailable` (or marker missing while a principal exists) → 500 fail-closed,
  mirroring the `action_extractor!` verdict set. Surfaces routes run under `require_auth`, so
  the marker is present on every authenticated request since M1.4a.

Call sites (read path :214-237, method-invoke path :357-455 including the MethodNotAllowed
sibling sweep) keep their exact order: 404 (unknown surface/interaction) → 403 (descriptor, then
interaction — before any method disclosure) → 405. The `AuthenticatedUser` parameter drops where
the context supersedes it; audit emissions keep their current actor plumbing.

### 7. Provider-origin rules, re-typed as-is

`proxy.rs:313-320` and `prepared.rs:56-62` re-type `required_permission.is_some()` →
`required_action.is_some()`; `validate_for_provider` likewise. **Behavior preserved exactly**,
including the existing divergence (the prepared/dispatch path has no `provider_invocable`
escape; the resolution path does). The divergence is noted for a future look, not redesigned
here — these are boolean structural gates on the provider side with no principal, so no engine
call belongs in them.

### 8. Live `DynamicActionRegistry` adapter

- New type in web-api (the only crate that sees both the trait's home and the surface registry;
  `uptrakit-surface-proxy` must not grow a `uptrakit-controller-core` edge):
  `struct SurfaceActionRegistry(Arc<SurfaceRegistry>);` implementing
  `DynamicActionRegistry::is_registered`: `Resource::Surface(id)` with `Verb::Use` → true iff a
  surface with that id is currently registered (a cheap registry lookup — the plan picks the
  narrowest existing read API or adds a `has_surface(&str) -> bool`); **everything else false**
  — other verbs on `surface.*` deny (nothing defines their meaning yet), and `plugin.*` has
  zero registered actions in v1 (`09` §Action model #3: grammar + hook only; a `declare_plugin!`
  declaration point is added when a plugin first needs one).
- Wired in `AppStateBuilder::build()`: the engine construction becomes
  `AccessEngine::new(db).with_registry(Arc::new(SurfaceActionRegistry(registry.clone())))` —
  ordering note: the builder must resolve the surface registry (today defaulted at
  app_state.rs:1109) **before** constructing the engine; the plan restructures the build
  order accordingly and re-greps every `AccessEngine::new(` fixture site to decide per-site
  whether the registry matters (test fixtures that never touch dynamic actions stay
  registry-less).
- `is_registered` is tenant-blind (trait has no tenant parameter): a surface registered by any
  provider makes `surface.<id>:use` decidable instance-wide. Accepted for v1 — single-tenant is
  the only tested mode (CONTEXT.md), and grants themselves stay tenant-scoped; recorded as a
  residual for the multi-tenant future.

### 9. Interactive update WS

In `routes/interactive_ws.rs`, after the bespoke token authentication and before any update
lookup:

- `state.access_engine.context(state.default_tenant_id, user_id, None).await` — the handler
  builds its own context because `require_auth` never ran here. Failure → 500 (fail-closed,
  same rule as everywhere).
- Gate: `engine.authorize(&ctx, &actions::UPDATES_TRIGGER, None)`; deny → the existing denied
  audit emission (`"permission_denied"`) + HTTP 403 `error_response` + deny counter. No
  credential → 401 as today. **HTTP pre-upgrade semantics preserved** (owner decision 1); no
  close-frame machinery.
- The handshake-time check is coarse (`target: None`); the attach-time fine check on the
  update's owning host is M2.3 (`07` §Interactive update WS, second bullet).

### 10. Inline handler-body conversions

All five sites read `AccessAuthority` from request extensions (present — these routes run under
`require_auth`) and `AccessState` for the engine; `Unavailable` → 500 fail-closed; denies emit
the shared counter. Conversions:

- `routes/plugin_type_settings.rs:82-85`: `authorize(ctx, SETTINGS_READ, None)` OR
  `authorize(ctx, SYSTEM_SETTINGS_MANAGE, None)` (two calls; allow if either allows).
- `routes/plugin_configs/crud.rs:100-105`: three calls — `SOFTWARE_READ`, `SETTINGS_READ`,
  `SYSTEM_SETTINGS_MANAGE`; allow on any.
- `routes/system_services.rs:475-481`: the action-string match maps to
  `SYSTEM_SERVICES_APPROVE` / `SYSTEM_SERVICES_REJECT` / `SYSTEM_SERVICES_DELETE`, then one
  `authorize` call replaces `has_permission`.
- `routes/services/batch.rs:70-76`: same shape with `SERVICES_APPROVE` / `SERVICES_REJECT` /
  `SERVICES_DELETE`.
- `visibility.rs` `is_plugin_visible_to_user`: the predicate's permission input re-types from
  `&AuthenticatedUser` to a caller-computed `caller_is_instance_admin: bool` — pinned over an
  `AccessContext` parameter because the predicate is shared with the surface-registry visibility
  path, which has no HTTP request; the fact is computed where a context exists. Route-handler
  callers compute it via `authorize(ctx, SYSTEM_SETTINGS_MANAGE, None)`; the plan enumerates
  every caller of the predicate and names each call site's source for the bool. The visibility
  semantics (`enabled || instance-admin`) are unchanged.

The batch-permission denial audit emissions at those sites keep their existing action types and
reasons — only the decision source changes.

## Cutover parity (shared obligation with M1.4b's B0)

M1.5's conversions are production authz cutovers of the same class M1.4b's B0 guards: the M1.2
migrations seed `access_grants` only for the eight built-in roles, so a custom (non-built-in)
role carrying legacy permissions with a live assignment has **no** engine grant — after M1.5,
such a principal would 403 on MCP, permission-gated surfaces, the interactive WS, and the five
inline endpoints. M1.4b runs its B0 audit strictly before its B1; M1.5 may land **first**.
Therefore: whichever of M1.5 / M1.4b-B1 lands first runs the B0 audit (non-built-in roles with
both `role_permissions` rows and a `user_roles` assignment — query per the M1.4b spec §B0) and
records the result in its plan/commit; if M1.4b's B0 already ran and recorded empty, M1.5 cites
that record instead of re-running. A non-empty result triggers B0's backfill deliverable before
M1.5's enforcement commits land. Same one-shot-validity caveat: a custom role created after the
audit re-triggers it.

## Manifest changes

`crates/core/mqtt-runtime/Cargo.toml` gains `uptrakit-shared-types = { workspace = true }`
(already registered in `[workspace.dependencies]`; needed for the catalog consts). No other new
edges expected: mcp/agent-ssh-runtime/notifications/docker/proxmox/infrastructure-core/
surface-proxy all already depend on `uptrakit-shared-types`; web-api already depends on
`uptrakit-surface-proxy` and `uptrakit-controller-core`. The plan verifies with
`cargo check` per touched crate before freezing tasks.

## Tests

Rows cited from `12-test-plan.md`; endpoint tests on the TestApp harness, MCP tests on the MCP
crate's existing sqlite harness, registry tests inline in surface-proxy. Zero-grant principals
are staged the M1.4a way (delete `user_roles` rows + `invalidate_subjects`) — registration is
not a zero-grant fixture.

- **MCP gate (F9 subset, both paths)**: principal without an `mcp:use`-covering grant → 403 on
  connection for (a) `upk_` API token, (b) OAuth JWT — identical outcome both paths; with the
  grant → connection succeeds. Audit reason preserved.
- **MCP tool deny/allow**: user with `mcp:use` but no `updates:trigger` → `trigger_update`
  denied through `require_tool_auth`; with both → tool executes. One history tool covered the
  same way (`software:read`). `get_current_user`: succeeds with zero tool actions; `actions`
  field contains exactly the catalog-expanded allow set (wildcard grant expands; no
  legacy permission names).
- **MCP immediate effect**: revoke the covering grant + `invalidate_subjects` → next MCP call
  403 (shared engine cache proven live on the MCP path).
- **Admission rejection (I5)**: a registration whose descriptor (and separately an interaction)
  carries an unparseable `required_action` (`"update_hosts"`, `"not-an-action"`) is rejected on
  the service path and the plugin/built-in path; a parseable-but-unregistered dynamic action
  (`surface.ghost:use`) **admits** (registry membership is decision-time).
- **Surface invoke deny (I3, invoke half)**: TestApp — surface/interaction gated by an action
  the caller lacks → read 403 and invoke 403; sibling-permission 403-before-405 order pinned
  (existing `surfaces_method_routes.rs` fixtures re-typed; the
  `__nonexistent_test_permission__` fixture becomes a valid-but-unheld action, since
  unparseable strings now die at admission).
- **Provider-origin (I7)**: existing surface-proxy provider-origin/`provider_invocable` tests
  re-typed and kept green (behavior unchanged).
- **Catalog guard (I8)**: a test iterating every compiled registration (ssh, mqtt, docker,
  proxmox, notifications, built-ins) asserting each declared `required_action` parses and is
  catalog-valid — with typed builders this is compile-checked, so the guard's real job is
  covering any residual string seam and future regressions; follow the
  `plugin provider-id identity` guard precedent (`b10b28215`).
- **Dynamic registry flip (extends C7)**: engine + `SurfaceActionRegistry` over a real
  `SurfaceRegistry`: grant `surface.<id>:use` → deny while unregistered, allow once the surface
  registers, deny again after deregistration.
- **Interactive WS (D15, as HTTP)**: no credential → 401; authenticated principal whose
  `updates:trigger` grant was removed (+invalidate) → 403 with the generic body, no upgrade;
  with grant → 101 Switching Protocols (existing success tests keep passing). Assertions are
  HTTP-status-based per owner decision 1.
- **Inline sites**: per converted file, one deny (403) + one allow through the new calls —
  batch system-services/services deny uses a principal missing the mapped action; the OR-logic
  endpoints get an allow via the _weaker_ arm (e.g. `settings:read` only) proving OR is
  preserved.
- **Unit**: `enforce_required_action` verdict table (None-action allow / allow / deny /
  `Unavailable` 500), mirroring its current unit tests (surfaces.rs:1636-1676).

Non-goals: no `start_paused` (no new time-dependent logic); no upstream-behavior tests; listing
filter tests are M2.4; deny audit-Event tests are M1.6b.

## Verification gates

In dependency order, all foreground: `cargo fmt --all`; `cargo clippy --all-targets
--no-default-features --features db-sqlite`; per-crate tests for the touched crates
(`uptrakit-mcp`, `uptrakit-surface-proxy`, `uptrakit-surfaces`, `uptrakit-web-api`,
runtime/plugin crates); `./scripts/regen-api.sh` (descriptor field rename reaches REST
responses; regen + `git add`, artifacts never opened) + `cd frontend && npm run check`;
`scripts/regen-asyncapi.sh` (surfaces types are schema-reached; golden gate);
whole-workspace `cargo test --no-default-features --features db-sqlite`; `cargo clippy
--all-targets --all-features` + `cargo test --all-features` (frontend build first);
`python3 ci/verify_action_security_declarations.py` (no `#[utoipa::path]` security blocks
change here — run to prove it); `cargo xtask audit-coverage-check` (audit sites keep their
action types — run to prove it); `bash ci/verify_handler_state_contract.sh`;
`python3 ci/verify_db_access_policy.py`; `markdownlint --config .markdownlint.json '**/*.md'`.

## Documentation deliverables

1. Root `AGENTS.md` — MUST-FOLLOW bullet **"Surface permissions are enforced at read/invoke
   time."** (cite by bold lead-in): re-word to `required_action` + engine enforcement; keep the
   provider-origin sentence accurate.
2. `docs/security/surfaces.md` — the four `required_permission` mentions (:20-21, :39, :69) plus
   surrounding prose: field rename, admission `Action` parse (rejection now possible), engine
   enforcement via `enforce_required_action`.
3. `docs/development/surfaces.md` (:113 and siblings) — same rename in the authoring guidance;
   builders now take catalog `Action` constants.
4. `docs/security/auth-and-authorization.md` — extend the M1.4a transition subsection: MCP,
   surfaces, interactive WS, and the inline sites now enforce through the engine; correct the
   close-frame description of the interactive WS to HTTP pre-upgrade semantics.
5. Refactoring-docs errata (local edits — `.superpowers/` is gitignored, these are working
   sources, not committed files): `07-decision-and-enforcement.md` — close-frame wording
   (decision 1), admission-site name (`validate_registration_basics`), and the surface-listing
   "as today" claim; `12-test-plan.md` — D15 note (HTTP semantics per owner decision).
6. Regenerated `openapi.json` + frontend client + `asyncapi.yaml` (committed artifacts,
   gate-enforced).
7. Rustdoc on every new/changed public item (`SurfaceActionRegistry`, `require_tool_auth`,
   `enforce_required_action`, re-typed fields). No ADR (M1.9 owns it); no CONTEXT.md change (no
   glossary term changes).

## Alternatives considered

- **Second engine instance inside MCP** (own cache over the same DB): avoids the `McpState`
  parameter thread-through, but forks the cache — a grant revocation invalidated via NATS would
  leave MCP stale for up to the TTL. Rejected: one engine, one cache.
- **Keep `Vec<Permission>` in `McpRequestContext` via the shim**: minimal diff, but keeps the
  JWT-snapshot resolution asymmetry the milestone exists to kill, and adds the only
  `actions_for_permission` call site the day before M1.8 deletes it. Rejected.
- **Per-row skip of unparseable wire `required_action` at admission**: rejected by owner
  (decision 3) — values are not human-configurable; a service sending garbage is a broken app
  and fails loudly.
- **`Option<Action>` as the descriptor wire field**: typed end-to-end but puts `Action` on the
  service wire, breaking the "string on the wire" property that justifies the no-`Other`
  design. Rejected; typed builder + string wire field gives compile-checked declarations with
  the same wire shape.
- **`DynamicActionRegistry` impl inside `uptrakit-surface-proxy`**: puts the impl beside the
  registry but requires a new `uptrakit-controller-core` dependency edge from surface-proxy
  (trait home), inverting the current layering for one trait impl. Rejected; web-api adapter.
- **Close-frame deny semantics for the interactive WS**: matches the test plan's literal text
  but changes verified current behavior and adds post-upgrade close choreography for zero
  security gain (deny still precedes any session). Rejected by owner (decision 1).

## Deferred / out of scope (verbatim carriers)

- **M1.4b** (`2026-08-03-access-route-sweep-design.md`): route-family `#[utoipa::path]`
  conversions incl. the dynamic surface wrappers' empty-scoped requirement +
  `x-action-dynamic: true` (eight at spec time), as six commit batches; its exit `rg` sweep
  asserts only the three M1.6-handed files (`users.rs`, `roles.rs`, `access_presets.rs`) retain
  `x-required-permission`/`bearer_token`. Files owned by M1.5 (listed in the sequencing note)
  receive their declaration work in M1.4b batch B6, which lands after this task. The
  `bearer_token` scheme deregistration rides with M1.6b (last-reference rule), not M1.4b.
- **M1.6a/b**: mutation-site invalidation publishing; deny audit **Events** (`mcp:use` among
  the four Event-tier actions); management API; catalog endpoint.
- **M1.7**: claims removal, `me` action list + `authority` field, frontend gating swap; MCP
  `get_current_user` follows `me`'s final dynamic-action expansion then.
- **M1.8**: `Permission`, `AuthenticatedUser::has_permission`, `permission_extractor!`, shim
  deletion; `rg`-clean.
- **M1.9**: canonical doc rewrite + ADR.
- **M2.3/M2.4**: WS attach-time fine check (D16/K11); surface listing + MCP list tools on
  visibility (I3 listing half).
- **M3.3**: MCP re-scope to `mcp:use` + per-tool action scopes; engine scope intersection for
  OAuth-presented MCP tokens (until then `scope: None` on every MCP path).
- **Multi-tenant `is_registered`**: tenant-blind in v1; revisit with the multi-tenant posture.
