# Proxmox Discovered-Guest Flow Restoration + Provider-Invocable Interactions

Date: 2026-07-15
Status: Draft (pending implementation plan)

## Problem

Two user-visible features of the SSH agent's Proxmox integration are broken:

1. `POST /api/v1/surfaces/ssh-agent.hosts/interactions/list-discovered-guests` returns `{"options": []}` even though
   `proxmox_host_mappings` holds unmatched guests (verified: 15 unmatched rows in the reporting deployment's DB, correct
   tenant, correct blob-UUID encoding — the controller-side query would return them).
2. "Bootstrap Discovered Guest" is unusable: its MultiSelect draws options from the same empty list, and even a
   successfully bootstrapped guest would never be matched to its mapping (the deferred-match drain is also broken).

## Root cause (verified)

Both breaks trace to the shared-surface-runtime migration. Commit `87e4e89ff` (2026-04-14, "refactor: remove legacy
extension wire messages") removed the un-gated `ServiceMessage::ExtensionRequest` dispatch, forcing the agent's nested
service→controller calls through the gated path introduced in `7435ac81b` (2026-04-13). The agent-side Proxmox plugin
makes exactly three nested calls, all via `InfraActionInvoker` → `ServiceSurfaceProxy` →
`ServiceMessage::SurfaceActionRequest` with `CallerOrigin::Provider`, and each now fails:

| Call site | Target | Failure |
| --- | --- | --- |
| `agent/surface_actions.rs:44` (`handle_list_discovered_guests`) | `proxmox.hosts` / `list-all-unmatched` | `InteractionNotFound` — the interaction was **never registered** in `proxmox_hosts_surface().interactions` (`plugin.rs`, which registers only `list`, `discover`, `test-connection`, `approve-match`, `match`, `unmatch`); it exists only in the legacy `SurfaceActionDescriptor` library (`surfaces.rs:342`) and the dispatch map (`surfaces.rs:176`) |
| `agent/surface_actions.rs:93` (`handle_bootstrap_proxmox_guest` re-fetch) | same | same |
| `agent/plugin.rs:317` (`on_post_report_hosts` drain) | `proxmox.hosts` / `match` | registered, but denied by the provider-permission gate: `CallerOrigin::Provider` + `required_permission.is_some()` → `PermissionDenied` (`crates/ui/surface-proxy/src/proxy.rs:276-282`, duplicated verbatim at `proxy/prepared.rs:55-61`) |

`handle_list_discovered_guests` then swallows every failure into `{"options": []}` at `tracing::debug!` level
(`agent/surface_actions.rs:51-70`), which is why a hard controller-side rejection presented as a silently empty
dropdown for three months.

Registry resolution point: `crates/ui/surface-proxy/src/registry.rs:477-482` (`.ok_or(SurfaceRegistryLookupError::InteractionNotFound)`).

Secondary defects found during investigation:

- `handle_list_all_unmatched` caps `per_page` at 200 (`surfaces.rs:1056`:
  `request.per_page.unwrap_or(50).clamp(1, 1000).min(200)`) while both agent callers request 1000 — tenants with >200
  unmatched guests would get a silently truncated list, and bootstrap would fail guests beyond the cap with
  "guest not found in discovered guests list". The surface-proxy result caps do **not** protect or conflict here:
  `MAX_RESULT_ROWS = 200` (`proxy.rs:42`) applies only to a top-level array or a `"rows"` key (`proxy.rs:1009-1025`),
  and this payload is an object keyed `"items"` (`surfaces.rs:1118`); the 1 MiB `MAX_RESULT_BYTES` cap bounds ~1000
  rows at roughly 250 KiB.
- `on_post_report_hosts` retries every `proxmox_pending_matches` row on every `ReportHosts` forever — a permanently
  failing row (deleted mapping, gone host) is unbounded retry noise. (No live backlog exists: the reporting
  deployment has 0 pending rows, because bootstrap never succeeded.)
- **The Proxmox plugin's agent-local migrations are dead code — fresh installs never get the drain's tables.**
  `CreateProxmoxHostState`/`CreateProxmoxPendingMatches` (`agent/migration.rs:12`, `:127`) are referenced nowhere: the
  module doc-comment claims wiring "via `PluginBase::service_migrations()`", but no such hook exists in
  `infrastructure-core`, and `AgentSshHandler::service_migrations()` (`crates/core/agent-ssh-runtime/src/handler.rs:172-178`)
  returns only the runtime's own `Migrator::migrations()` list (`db/migration/mod.rs:22-38`), which creates the
  *legacy* `pending_proxmox_matches` table, not `proxmox_pending_matches`. Wiring existed historically
  (`f3b89f071` "fix(agent-ssh): run plugin migrations at startup") and was dropped by the 2026-03 descriptor/catalog
  unification refactors. Existing deployments carry the tables from that era (verified: the reporting DB has both
  `proxmox_pending_matches` and `proxmox_host_state`, with neither `m20260308_00000{1,2}_create_proxmox_*` recorded in
  `seaql_migrations`); a fresh install today would fail the first successful bootstrap at
  `db_ops::insert_pending_match` with "no such table: proxmox_pending_matches".
- No test covers the nested provider→controller invoke path; no test asserts the provider-permission-gate denial; no
  parity check ties the legacy dispatch map to registered interactions; `docs/development/surfaces.md` promises
  "Service-initiated action calls are supported" with no permission caveat, and `docs/security/surfaces.md` documents
  permission enforcement only for user HTTP endpoints.

## Design

### D1 — Wire contract: `provider_invocable` opt-in on `InteractionDescriptor`

Add to `InteractionDescriptor` (`crates/shared/surfaces/src/interaction.rs:44-71`, `#[non_exhaustive]`). The serde
attribute copies the in-file bool idiom of `render_previous_response` (`interaction.rs:38`); the rustdoc comment is a
deliberate first instance — no field in this struct is documented today, and a security-relevant flag warrants it:

```rust
/// Allows same-tenant provider-origin (service-initiated) invocation of this
/// interaction even when `required_permission` is set. Fail-closed: absent on
/// the wire deserializes to `false`.
#[serde(default, skip_serializing_if = "std::ops::Not::not")]
pub provider_invocable: bool,
```

- `InteractionDescriptor::new` (`interaction.rs:131-153`) initializes it to `false`.
- **No new `WireValidate` limit constant is needed**: the field is a `bool` and carries no `Vec<T>`/`String` content;
  `validate_surface_interaction` (`crates/shared/wire/src/wire_validate_impls.rs`) needs no change. Stated here so
  review does not reflexively demand one.
- Admission hardening (fail-closed policy surface): `InteractionDescriptor::validate_for_provider(provider_kind)`
  (`interaction.rs:175-255`) rejects `provider_invocable == true` when `required_permission.is_some()` **and** the
  registering provider kind is `ProviderKind::Service`, via a new `InteractionValidationError` variant (e.g.
  `ProviderInvocableForbiddenForServiceProviders`) — every existing check in that function maps to its own variant;
  callers stringify at `protocol.rs:291-292`. The flag is honored only for in-tree `Plugin`/`BuiltIn`
  interactions for now; relaxing later for service-owned interactions is an additive change. Rationale: for
  plugin-owned interactions, flag authorship and enforcement ship in the same binary (the controller); for
  service-owned permissioned interactions, honoring a remotely-registered flag would let service B invoke service A's
  privileged interaction on co-tenancy alone — defer that policy question until a real use case exists.
- **Two distinct policy axes — do not conflate.** The admission rule above governs *who may declare* a flagged
  interaction (Plugin/BuiltIn only). The invoke-time gate (D2) governs *who may call* one, and it is deliberately
  caller-identity-blind: flag set ⇒ **any** same-tenant provider may invoke. Per-caller narrowing is outside this
  field's expressive range **by design** — if a future tenant runs mutually-distrusting services, the additive path is
  a new optional field (e.g. `provider_allowlist: Option<Vec<ProviderId>>`, absent = any tenant provider) composing
  with this bool, not a breaking bool→enum mutation of it — note that unlike today's bool, that future `Vec` field
  will need a `WireValidate` limit constant. Stated here so a future maintainer neither overloads this field nor
  assumes it can express caller scoping.

### D2 — Gate: extract and extend

The provider-permission gate exists byte-identically in `proxy.rs:276-282` and `proxy/prepared.rs:55-61`. Extract into
one shared function in the `surface-proxy` crate (e.g. alongside `caller_origin_for_request`), called from both sites:

```rust
if matches!(&caller_origin, surfaces::CallerOrigin::Provider { .. })
    && resolved.interaction.required_permission.is_some()
    && !resolved.interaction.provider_invocable
{
    return Err(SurfaceProxyError::PermissionDenied(
        "provider-initiated requests cannot satisfy user permission gates".to_string(),
    ));
}
```

The user HTTP path is untouched: `invoke_surface_interaction` (`crates/ui/web-api/src/routes/surfaces.rs`) continues to
enforce **both** `descriptor.required_permission` and `interaction.required_permission` against the authenticated user.
`provider_invocable` has no effect on user-origin or `BuiltInSystem`-origin calls.

Security semantics (this is the whole argument; it goes in the docs, D8): `provider_invocable = true` on a permissioned
interaction means **any same-tenant provider with the `UiSurfaces` capability may invoke it, subject only to tenant
scope** (`message_processor.rs` rejects cross-tenant requests) plus existing rate/idempotency/timeout controls. Stated
honestly, flagging `match` **is** a privilege expansion, not a no-op: the mapping side of a match is authored by the
controller-side `discover` action querying the PVE API with stored credentials (`surfaces.rs:574-579` →
`handle_discover`) — agent-ssh does not create that data — and after this change any same-tenant service (MQTT,
scheduler, future services), not just agent-ssh, can bind any unmatched guest to any host in its tenant. Accepted
because: tenant services are co-trusted (enrolled by the tenant admin, and a service can already fabricate the entire
host side via `ReportHosts`), the write is tenant-scoped and recoverable via `unmatch`, and every provider-origin
invocation is audit-attributed to the calling service. This framing — expansion, accepted, why — is what goes into
`docs/security/surfaces.md`, not a "grants nothing new" claim.

### D3 — Register the missing interaction; flag opt-ins; REST-friendly ids

Interaction ids are renamed to REST-friendly noun phrases while we touch them (user decision). Naming convention,
recorded in `docs/development/surfaces.md`: **data-retrieval interactions use noun phrases** (`discovered-guests`,
`unmatched-guests`); **mutations keep verbs** (`bootstrap-proxmox-guest`, `match`). Renaming other pre-existing ids
(e.g. `list`) is deferred to the unification follow-up (F1).

1. **`proxmox.hosts` / `unmatched-guests`** (rename of never-registered `list-all-unmatched` — no HTTP caller ever
   reached it, so no compatibility surface). Add to `proxmox_hosts_surface().interactions`
   (`crates/plugins/infrastructure/proxmox/src/plugin.rs:228-351`), copying the in-file construction idiom:

   ```rust
   {
       let mut i = surfaces::InteractionDescriptor::new(
           surfaces::InteractionId::new("unmatched-guests").expect("literal"),
           surfaces::InteractionKind::DataLoad,
           "Unmatched Guests",
           surfaces::InteractionTransport::ControllerLocal,
       );
       i.required_permission = Some(Permission::UpdateHosts.to_string());
       i.result_schema = Some(surfaces::SchemaContract::Any);
       i.provider_invocable = true;
       i
   },
   ```

   Rename in the same change (single source of truth on the new id — all sites are in-tree):
   - legacy library descriptor `surfaces.rs:342` and dispatch map key `surfaces.rs:176`
   - both agent invoke sites `agent/surface_actions.rs:46`, `:95` (+ log message strings `:59`, `:66`)
   - test assertion `surfaces.rs:2033`
   - docs `docs/development/proxmox-plugin.md` (grep `list-all-unmatched`), `docs/architecture/ssh-agent.md:922`

2. **`proxmox.hosts` / `match`**: set `i.provider_invocable = true;` on the existing registration
   (`plugin.rs:282-330`). Id unchanged (verb is correct for a mutation; live user-facing FormSubmit).

3. **`ssh-agent.hosts` / `discovered-guests`** (rename of `list-discovered-guests`; user-requested). This id is owned
   by the agent-ssh binary — descriptor, form select-source reference, and dispatch all ship together, so the rename
   is self-consistent per binary version. Rename at:
   - descriptor `agent/plugin.rs:40` and select-source reference `agent/plugin.rs:612`
     (`FormSelectSourceDescriptor::Action { action_id }`)
   - dispatch arm + module docs `agent/surface_actions.rs:7`, `:25`, `:35`, `:90`
   - infra-core unit test `crates/plugins/infrastructure/core/src/surface_form_authoring.rs:1164`
   - docs `docs/development/proxmox-plugin.md:56`, `docs/architecture/ssh-agent.md:728`, `:888`, `:922`
   - CHANGELOG hits are historical; leave them.

   Frontend hardcodes nothing (select-source `action_id` is data-driven from the registration); the generated OpenAPI
   client is unaffected (interaction ids are runtime path *data*, not OpenAPI paths — no `regen-api.sh` needed for the
   renames themselves).

   **Version-skew note**: agent-ssh is a separately-versioned, manually-updated binary — controller/agent skew is the
   normal steady state, not an edge case. A new agent-ssh against an old controller (or vice versa) fails the nested
   call with `InteractionNotFound` — which is exactly today's behavior in **every** version combination, now surfaced
   as an error (D4) instead of a fake empty list. The renames are safe **only** because this specific path is
   non-functional in all combinations today, so no rename can regress a working path. This argument does NOT
   generalize: renaming a *working* interaction id (D9's deferred `list`, `discover`, …) is a breaking change on a
   non-lockstep binary and requires transitional dual-registration (old + new id for a release) — recorded as a D9
   constraint.

### D4 — De-swallow list errors (intentional behavior change)

`handle_list_discovered_guests` (`agent/surface_actions.rs:38-71`): on nested-call failure (`Err(_)` or
`success == false`), return an error `SurfaceActionResponse` via the module's existing `make_error_response` helper —
the same shape `handle_bootstrap_proxmox_guest` already uses (`:101-111`) — and log at `tracing::warn!` following the
file's message conventions. `{"options": []}` remains only for a genuine empty result. This **is** a behavior change:
previously-masked failures now surface to the operator; that is the point (the April regression was invisible for
three months because of this swallow). Verify during implementation that the Dashboard renders a select-source
interaction error as a load failure (not an empty dropdown); if it does not, that is a frontend deliverable of this
spec, not a reason to keep the swallow.

### D5 — Honor requested page size

`handle_list_all_unmatched` (`surfaces.rs:1056`): drop the `.min(200)` so the effective limit is
`request.per_page.unwrap_or(50).clamp(1, 1000)`. Safety argument recorded in Root cause (proxy row cap keys on
`"rows"`/top-level arrays; byte cap bounds the payload). Update the pagination note in
`docs/development/proxmox-plugin.md:169` if it states the old cap.

### D6 — Re-wire agent-plugin migrations; drain poison-row protection

**Prerequisite — re-wire the dead migration set** (see Root cause: fresh installs lack the tables):

- Collect the plugin's agent-local migrations descriptor-driven, mirroring how controller migrations are already
  collected (`declare_plugin!` `migrations:` → `PluginDescriptor.migrations`): add an agent-migrations source to the
  descriptor, and extend `db/migration/mod.rs::Migrator::migrations()` to append them **after** the runtime's own list.
  Right-sizing: proxmox is the **only** plugin in the tree with an agent-local migration module (verified:
  `find crates/plugins -path '*agent/migration.rs'` → one file), so build the mechanism to what today needs — one
  descriptor field, one collection point — and leave generalization to the D9 unification; do not present a framework
  validated by a single consumer.
- **Ordering is load-bearing, not stylistic**: the runtime's own `m20260307_000002` creates the *legacy*
  `pending_proxmox_matches` table, and `CreateProxmoxPendingMatches` copies **from** it — the plugin migrations must
  run after the runtime list or the data copy sees nothing. Appending also complies with "never reorder existing
  migration entries" (the runtime list is already non-chronological; append-order is the established idiom). State
  this dependency in a comment at the append site so a later reorder doesn't silently drop the copy.
- The extension must land in `db/migration/mod.rs::Migrator::migrations()` itself (not only `handler.rs`): standalone
  agent-ssh runs migrations via `db/mod.rs:31` → `run_migrations` directly, while embedded deployments go through
  `AgentSshHandler::service_migrations()` — both bottom out in `Migrator::migrations()`, so that is the single point
  covering both deployment shapes.
- Safe on existing DBs by construction: both plugin migrations are `if_not_exists` creates + `INSERT OR IGNORE` data
  copies (`agent/migration.rs:28`, `:93`, `:142`, `:186`), and their names
  (`m20260308_00000{1,2}_create_proxmox_*`) are unrecorded in existing `seaql_migrations` — first run after the fix
  executes them idempotently.
- Fix the false module doc-comment (`agent/migration.rs:3-4`) that cites the nonexistent
  `PluginBase::service_migrations()` hook — point it at the real collection path introduced here.
- Test: fresh in-memory agent DB after `run_migrations` contains `proxmox_pending_matches` **and**
  `proxmox_host_state`; second run is a no-op.

**Poison-row protection** — `proxmox_pending_matches` (entity `agent/entity.rs:40-57`, ops `agent/db_ops.rs:89-138`):

- New migration adding the `attempts` integer column (not null, default 0) via the `Table::alter().add_column(...)`
  sea_query builder — precedent in this same plugin: `controller_migration.rs:187-194`. Do **not** copy
  `agent/migration.rs`'s raw-SQL statements; those are the approved data-copy exceptions, not the column-add idiom
  (database-migrations.md: builders everywhere, raw SQL only for the documented exceptions).
- `on_post_report_hosts` (`agent/plugin.rs:295-354`): bound each drain cycle to `MAX_DRAIN_PER_CYCLE: usize = 50`
  rows; on per-row failure, increment `attempts`; when `attempts` reaches `MAX_MATCH_ATTEMPTS = 10`, delete the row
  and `tracing::warn!` a dead-letter message naming mapping_id/host_id so the operator can re-run matching manually.
  Both constants function-local at point of use, per the sibling idiom (`MAX_CONCURRENT_BOOTSTRAPS` inside
  `handle_bootstrap_proxmox_guest`, `agent/surface_actions.rs:81`); `agent/plugin.rs` has no module-level consts.
  No backoff machinery — `ReportHosts` cadence already spaces retries.
- Success path unchanged (invoke `match`, delete row).

### D7 — Tests (success + failure per AGENTS.md)

surface-proxy crate:

- Provider origin + `required_permission = Some` + `provider_invocable = false` → `PermissionDenied` (the gate's first
  direct positive-denial test; today it is only documented by tests that work around it).
- Same but `provider_invocable = true` → dispatch proceeds.
- Wire compat: registration JSON **without** the field deserializes to `false` and is denied.
- Admission: `validate_for_provider(ProviderKind::Service)` rejects flag+permission; `ProviderKind::Plugin` accepts.

proxmox crate:

- **Parity guard test** (kills the drift class that caused this bug). Pair derivation must be stated because
  `SurfaceActionDescriptor` carries only a bare `action_id` (`surface_form_authoring.rs:249-251`) — there is no
  action→surface mapping to iterate (that missing link *is* the bug class). Mechanism: iterate every action id in the
  legacy `surface_actions()` library (`surfaces.rs:214`); for each, scan all surfaces in
  `proxmox_surface_registrations()` for a registered `InteractionDescriptor` with that id; assert exactly one surface
  carries it **and** `resolve_controller_surface_action(that_surface_id, action_id)` returns `Some`. This derives the
  pair from the registrations rather than hand-authoring a surface/action table (which would recreate the drift-prone
  parallel list inside the test). Green-on-empty protection: assert the iterated set is non-empty **and** contains
  named known members (`unmatched-guests`, `match`) before asserting parity. If genuinely non-registered actions
  exist, each exclusion must be an explicit allowlist entry with an inline justification — derive the exact membership
  empirically at implementation time (do not hand-author from memory), and RED the test by removing one registration.
- Agent handler behavioral tests with a mock `InfraActionInvoker`. **No double exists today** — the trait
  (`agent_infra.rs:214-225`) has only production impls; neither `infrastructure-core/src/testing.rs` (command-executor
  doubles only) nor `proxmox/src/testing.rs` (DB fixtures only) covers it. Add a `RecordingActionInvoker` (records
  `(surface_id, action_id, params)`, returns queued responses/errors) to `infrastructure-core::testing` — gated
  `#[cfg(all(feature = "testing", feature = "agent-infra"))]`, because the trait lives behind `agent-infra`
  (`lib.rs:1-2`) and the `testing = []` feature (Cargo.toml:22) does not pull it in; do not make `testing` imply
  `agent-infra` (would bloat every existing testing consumer). Named manifest deliverable: the proxmox crate adds
  `uptrakit-plugin-infrastructure-core = { workspace = true, features = ["testing", "agent-infra"] }` under
  `[dev-dependencies]` — its current dependency requests only `plugin-ops` (proxmox `Cargo.toml:23`), so without the
  dev-dep entry the mock-invoker tests do not compile. Do not hand-roll a private per-test mock. Tests:
  `handle_list_discovered_guests` maps
  items → options on success, returns an error response on invoker error and on `success == false`; drain increments
  `attempts` on failure, dead-letters at the cap, respects the per-cycle cap.
- `handle_list_all_unmatched` honors `per_page = 1000` (seed >200 unmatched rows — distinct `proxmox_vmid` per row to
  respect the table's upsert key; multiple `host_id = NULL` rows are fine under the host-uniqueness index — assert no
  truncation).

web-api crate (`routes/service_ws/handler/tests.rs`, existing `handle_surface_action_request` harness):

- Provider-origin e2e: request for `proxmox.hosts`/`unmatched-guests` resolves, passes the gate, executes, and the
  audit row's actor identifies the **service** (not a user). Audit attribution already exists and is provider-tested —
  `emit_surface_action_invoke_audit_event` sets `.actor_service(ctx.service_id)` (`audit_surface.rs:227-286`), and
  `surface_action_success_emits_success_tenant_audit_row` (`tests.rs:550`) already asserts actor fields for a
  `CallerOrigin::Provider` payload — so this is an *extension* of that existing test to the two target interactions,
  not a fix hunt. (Cross-subsystem precedent, if needed: MQTT-triggered updates record `actor_type = "uptrakit-mqtt"`,
  `ActorType::Mqtt`.)
- Provider-origin request for a permissioned, un-flagged interaction still yields the denial + audit row.
- Provider-origin `proxmox.hosts`/`match` **reaches and completes its handler** (assert the mapping's `host_id` is
  set), not merely resolves and passes the gate — locks in that `handle_match` needs no user identity, so a future
  change adding one fails a test instead of the manual verification step.

### D8 — Documentation deliverables

| File | Change |
| --- | --- |
| `docs/security/surfaces.md` | Extend the permission-model section: provider-origin invocation policy — gate predicate, `provider_invocable` semantics ("any same-tenant `UiSurfaces` provider, tenant scope only"), the Plugin/BuiltIn-only admission rule, and the instruction that handlers of flagged interactions must not treat provider origin as privileged beyond tenant membership |
| `docs/development/surfaces.md` | Correct the "Service-initiated action calls are supported" claim: supported **iff** the target interaction is unpermissioned or `provider_invocable`; document the field and the noun/verb id naming convention |
| `docs/architecture/surfaces.md` | One paragraph on the provider-origin gate in the dispatch model section |
| `AGENTS.md` | "Surface permissions are enforced at read/invoke time" stub gains the provider-origin clause + doc link (invariant changes require the stub update) |
| `docs/development/proxmox-plugin.md` | Rename ids; update the action table and the pagination note |
| `docs/architecture/ssh-agent.md` | Rename ids in the surface-action table and flow description |
| `crates/shared/wire/asyncapi.yaml` | Surface registration payloads are **not currently modeled** (verified: zero hits for `SurfaceRegistration`/`InteractionDescriptor`/`interactions`). Record this pre-existing gap in the spec's implementation notes rather than silently skipping; adding full surface-message modeling is out of scope |
| `docs/api/wire-protocol.md` | No `InteractionDescriptor` field inventory exists (only a `UiSurfaces` capability row); no change required beyond D8 rows above — verified by grep |

Rustdoc: the new field, the shared gate fn, and the changed handlers get doc comments; every function whose signature
or failure semantics change gets its doc-comment re-checked in the same edit.

### D9 — Follow-up (deferred, registered in backlog): unify the two interaction systems

**Problem being deferred**: the controller has two parallel per-plugin interaction declarations —
(a) the legacy `SurfaceActionDescriptor` library + `ControllerSurfaceAction` dispatch map
(`surfaces.rs:165-230` in the proxmox plugin; equivalents in other plugins), consumed via
`declare_plugin!(surface_actions.actions)` → `catalog.rs` local dispatch, and
(b) registered `InteractionDescriptor`s (`surfaces.registrations`) consumed by the `SurfaceRegistry`. Only (b) gates
resolvability; only (a) drives `handle_surface_action` dispatch. Nothing links them — that gap is exactly how
`list-all-unmatched` stayed dispatchable-but-unresolvable for 15 months of refactors.

**Follow-up shape**: make registered `InteractionDescriptor`s the single source of truth. Sketch:

1. Extend the registration types (or a parallel plugin-local table keyed by `InteractionId`) to carry what the legacy
   descriptors add today: handler routing (the `ControllerSurfaceAction` enum), sudo/timeout metadata, and the
   agent-side conversion input (the agent-ssh runtime converts `SurfaceActionDescriptor`s to registered interactions
   in `build_interactions()` at `surface_runtime.rs:560-635` — `InteractionDescriptor::new` at `:616`, always
   `InteractionTransport::ProviderProxied` and `ProviderKind::Service`, so nothing this conversion produces may carry
   `provider_invocable` under D1's admission rule; the conversion inverts once descriptors die).
2. Migrate plugin-by-plugin: derive `resolve_controller_surface_action` from a match on `InteractionId` colocated with
   the registration; delete the plugin's `surface_actions()` list once its dispatch is derived. Generalize D6's
   single-consumer agent-migrations collection here if a second plugin ever needs it.
   **Constraint**: any renaming of *working* interaction ids (`list`, `discover`, …) requires transitional
   dual-registration (old + new id for at least one release) — agent-ssh is not released in lockstep with the
   controller, so a bare rename of a functioning id breaks mixed-version deployments (see D3's version-skew note for
   why the current renames are exempt).
3. Delete the `SurfaceActionDescriptor` type, the `catalog.rs` `.actions` plumbing, and the parity guard test (D7)
   once the drift class is structurally impossible.

Scope: all infra plugins + notification/docker plugins with controller-local actions, `catalog.rs`, agent-ssh runtime
conversion, and the local-executor allowlist tiers. Risks: the allowlist tiers (`local_executor.rs:126-340`) key
behavior (audit, db routing) per action — each migrated action must keep its tier. This is a separate spec; until it
lands, the D7 parity test is the guard.

## Alternatives considered

- **Permission-None minimalism** (no new field; null `required_permission` on `match`, register `unmatched-guests`
  unpermissioned; users stay gated by the surface-descriptor `UpdateHosts`): rejected — couples write authorization to
  the coarser descriptor grain (only coincidentally equivalent today), destroys the truthful interaction-level
  declaration the frontend and HTTP path key on, and a future maintainer re-adding the permission silently re-breaks
  the drain.
- **`BuiltInSystem` origin for agent nested calls**: rejected — launders a genuinely remote service call as a system
  principal, erasing the caller identity from audit.
- **Unify the two systems now**: deferred to D9 by user decision — right end-state, too large to couple to the fix.

## Out of scope

- The two-system unification (D9 — follow-up spec).
- Renaming pre-existing interaction ids beyond the two touched here (`list`, `discover`, …) — follows D9.
- Modeling surface registration payloads in `asyncapi.yaml` (pre-existing gap, recorded).
- Relaxing the Service-provider-kind admission rule for `provider_invocable` (no use case yet).

## Verification

- Scoped clippy over the touched crates (confirm exact `-p` names against each crate's `Cargo.toml` `name` at plan
  time — the workspace has inconsistently named crates):

  ```sh
  cargo clippy --all-targets -p uptrakit-surfaces -p uptrakit-surface-proxy \
    -p uptrakit-plugin-infrastructure-proxmox --features agent-infra \
    -p uptrakit-agent-ssh-runtime -p uptrakit-web-api
  ```

  The `--features agent-infra` (confirm the exact feature name and per-`-p` applicability at plan time) is
  load-bearing: the proxmox crate's `default = []` and `#[cfg(feature = "agent-infra")]`-gated `mod agent`
  (`lib.rs:16-17`) mean a bare scoped clippy never compiles `agent/surface_actions.rs`, `agent/plugin.rs`, or
  `agent/migration.rs` — the exact files D3/D4/D6/D7 touch.
- `cargo test` for the same crates with the same feature flags; full `cargo test --all-features` before push
  (requires `frontend/build/`)
- Re-run the consumer-inventory greps and assert zero stale hits outside CHANGELOGs/historical specs:
  `grep -rn "list-discovered-guests\|list-all-unmatched" --include="*.rs" --include="*.md" crates/ docs/development docs/architecture docs/security`
- `markdownlint --config .markdownlint.json` on every touched doc
- Manual: on the reporting deployment, the `ssh-agent.hosts` bootstrap form lists the 15 unmatched guests; bootstrapping
  one creates the host and the next `ReportHosts` drains the pending match (mapping's `host_id` set, row deleted).
