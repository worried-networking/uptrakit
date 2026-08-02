# 0028 — Single-Source Plugin Interaction Registration

Date: 2026-07-16

## Status

Accepted

## Context

The controller carried two parallel per-plugin interaction declarations that never agreed with each
other. The **legacy** system was a `SurfaceActionDescriptor` list plus a `handle_action` fn pointer,
declared via `declare_plugin!(surface_actions: { actions, handle_action })` and `owned_surface_ids:`,
routed by `PluginCatalog` with longest-prefix `starts_with` matching on `surface_id`. The **registered**
system was a hand-authored list of `InteractionDescriptor`s per plugin, declared via
`declare_plugin!(surfaces: { registrations })` and consumed by `SurfaceRegistry`. Only the registered
system gated resolvability (whether the frontend could see and admit an interaction); only the legacy
system drove dispatch (whether an inbound request actually reached a handler). Nothing linked them.

That gap is how `proxmox.hosts`' `list-all-unmatched` interaction stayed dispatchable-but-unresolvable
for 15 months: it existed in the legacy `handle_action` dispatch table and the prefix-routing map, but was
never added to the plugin's registered `InteractionDescriptor` list, so `SurfaceRegistry` never admitted
it and no caller could reach it through the registry-resolved path. The only guard against this drift
class was a single parity test scoped to the proxmox crate
(`every_legacy_dispatchable_action_is_a_registered_interaction`), not a structural guarantee.

A 2026-07-16 audit (`docs/superpowers/specs/2026-07-16-interaction-system-unification-design.md`) found
the legacy `.actions` lists were production-dead on the controller side — nothing served
`SurfaceActionDescriptor` to the frontend or HTTP API — while still being the **only** live dispatch path,
and the only consumer of several fields (`row_visible_when`, `batch_action`, `icon`) that fed nothing on
controller-plugin actions but were live on agent-collected ones. Dispatch itself was heterogeneous across
plugins: proxmox matched a private 17-variant enum, docker matched `(surface_id, action_id)` tuples,
notification plugins matched `action_id` alone against only a subset of their action lists — with their
CRUD interactions actually executed by controller-side code in `local_executor.rs`, never reaching the
plugin at all. One dispatch entry point bypassed registry resolution entirely: the public, unauthenticated
notification callback route dispatched a pseudo-action string (`"handle_callback"`) directly through the
legacy prefix router, reachable only because prefix routing never consulted registrations.

See `docs/superpowers/specs/2026-07-16-interaction-system-unification-design.md` for the full audit,
decision grilling, and design; `docs/superpowers/plans/2026-07-16-interaction-unification-c-cutover-guard-docs.md`
for the implementation plan (task series A–C).

## Decision

### One `RegisteredInteraction` per interaction; transport is derived, never authored

`crates/plugins/infrastructure/core/src/registration.rs` defines `InteractionDelivery` (`PluginHandled(handler)`
or `ControllerExecutor`) and `RegisteredInteraction`, which pairs a wire `InteractionDescriptor` with an
`InteractionDelivery`. `RegisteredInteraction::new()` is the only constructor; it overwrites
`descriptor.transport` from the delivery, so a second, independently-authored transport field can never
exist to drift from the delivery again. A plugin's `PluginSurface` holds its `SurfaceDescriptor` plus a
`Vec<RegisteredInteraction>`; a `PluginSurfaceRegistration` holds a plugin's `Vec<PluginSurface>`. `to_wire()`
strips delivery information to produce the wire `SurfaceRegistration` that `SurfaceRegistry` already
consumes — descriptor and dispatch are now authored at the same call site, so they cannot disagree.

### `declare_plugin!` folds to one `surfaces:` arm; the legacy arms are deleted

The `surface_actions: { actions, handle_action }` and `owned_surface_ids:` arms are gone. The single
`surfaces: { provider_id, registrations }` arm takes a `fn() -> Vec<PluginSurfaceRegistration>`. A plugin
developer touches only their own crate — the registrations fn and its handler shims — never a global
routing table. (Amended by ADR-0034: the arm is now `surfaces: { registrations }` — the provider id
derives from the descriptor's `type_id`.)

### Exact-id routing replaces longest-prefix routing

`PluginCatalog::build` derives an exact-id dispatch map, `BTreeMap<(String, String), InteractionHandler>`,
from one `registrations()` call per plugin, containing only `PluginHandled` entries. Admission rejects a
build where the **same surface id** is registered by two different plugins (a build-time error, not a
routing ambiguity); the same plugin re-registering the same surface id across calls is fine. This is
strictly tighter than the prefix-overlap check it replaces: exact-id routing is safe because every
dispatched `surface_id` comes from a `ResolvedSurfaceAction` — i.e., post-registry-resolution — and no
dynamic or parameterized surface ids exist. `PluginSurfaceActionOps::handle_surface_action` looks up the
exact pair; a miss produces the same error shape the prefix router produced for an unroutable action.

### `ControllerExecutor` delivery requires a const-table row, enforced bidirectionally

Some interactions are executed entirely by controller-side code (notification channel CRUD, for example)
with no plugin handler at all; these declare `InteractionDelivery::ControllerExecutor`. Which
`(surface_id, interaction_id)` pairs are allowed to execute this way is a single source of truth,
`CONTROLLER_LOCAL_EXECUTOR_TABLE` (`crates/ui/surface-proxy/src/proxy/controller_local.rs`), tagged per
row with an `ExecutorTier` (`ControllerExecutes` for Tier 1, `PluginWithAudit` for Tier 2; Tier 3 — plugin
invoke, no audit — is the fallthrough with no table row). A bidirectional integration test,
`crates/ui/web-api/tests/interaction_executor_guard.rs`, proves the table and the catalog's unified
registrations agree in both directions: every table row has a matching registration with the expected
delivery kind, and every `ControllerExecutor`-delivery registration has a Tier-1 table row — a registered
but tableless `ControllerExecutor` interaction is registered but unexecutable, and now fails loud instead
of silently. This is the exact drift class the deleted proxmox parity test used to guard, generalized and
made structural. The guard proves interaction existence and delivery kind, not audit-tier correctness:
Tiers 2 and 3 both map to `PluginHandled`, so a row moved between an audited and unaudited tier is
invisible to it — that narrower class stays hand-authored, same as before.

### Agent-side interactions invert to a single authoring builder, gated by a wire-fixture equivalence test

`AgentInteraction` (`crates/plugins/infrastructure/core/src/agent_interaction.rs`) replaces hand-written
`SurfaceActionDescriptor`s on the agent side with one builder carrying the descriptor-building inputs,
placement metadata the wire `InteractionDescriptor` cannot express (`AgentInteractionPlacement`:
`Internal`, `Primary`, `Row`), and the agent-side handler. `PluginDescriptor` gains an `agent_surfaces:`
`declare_plugin!` arm (`fn() -> Vec<AgentInteraction>`) mirroring the `agent_migrations` field precedent.
The agent-ssh runtime's own built-in interactions and the proxmox agent module's `agent_interactions()`
table both author through this one builder — placement metadata (`AgentInteractionPlacement::Primary`,
row-visibility conditions) replaces the previous hardcoded id-literal filters used to decide surface
placement. Because the wire `InteractionDescriptor` cannot represent placement, a golden-fixture
wire-equivalence test pins the pre-refactor `ssh-agent.hosts` wire `SurfaceRegistration` JSON output
(interaction set, kinds, workflow steps, timeouts, permissions, `visible_when`, batch flags) and asserts
the new authoring path reproduces it exactly — this is the agent-side replacement for the deleted parity
test, catching metadata loss the wire type itself cannot express.

### `handle_callback` becomes a `NotificationTransport` trait method, off the surface dispatch path

The notification callback route (`/api/v1/notifications/callback/{channel_type}/{channel_id}`) is not a
surface interaction: it is a public, unauthenticated inbound webhook (Telegram Bot API and similar),
verified per-channel-type inside the plugin, not through the authenticated user-invoke path. Registering
it as an interaction would have expanded its surface area rather than fixed the drift. Instead,
`NotificationTransport` (`crates/plugins/infrastructure/core/src/roles.rs`) gained a `handle_callback`
method with a default implementation returning "callback not supported for channel type '...'"; telegram
overrides it with its existing verification logic. The route resolves the transport by `channel_type` via
the existing `plugin_ops.transport(&channel_type_id)` lookup and calls the trait method directly — the
`"handle_callback"` pseudo-action string and the fabricated `format!("notifications.{channel_type}")`
surface id are gone. This removes the last out-of-band caller of `handle_surface_action`, restoring the
invariant the exact-id dispatch map depends on: dispatch equals registrations, with no entry point outside
the model.

## Consequences

- A plugin developer adding, removing, or changing an interaction edits exactly one place in their own
  crate — the `surfaces:` (or `agent_surfaces:`) registrations fn — instead of keeping a descriptor list
  and a dispatch table in sync by hand.
- The `list-all-unmatched`-class drift (registered somewhere but not dispatchable, or dispatchable but not
  registered) is now structurally impossible for controller-side interactions: registration and dispatch
  derive from the same call, and `ControllerExecutor` delivery is guarded bidirectionally by a real test
  rather than a per-plugin parity check someone has to remember to add.
- `SurfaceActionDescriptor`, `SurfaceActionLibrary`, `ApiSubmitDescriptor`, `ControllerSurfaceAction`,
  `resolve_controller_surface_action`, the thirteen `.with_api_submit(...)` call sites, and the
  longest-prefix routing machinery are deleted. The unreachable Tier 1b `("proxmox.hosts", "add-config")`
  allowlist entry (no `add-config` interaction was ever registered, so it could never execute) is deleted
  with it — deleting it changes no audit posture, since plugin-config creation already flows through the
  REST route in every deployment, not through this surface action.
- Agent-side interaction ordering and metadata (placement, workflow steps, timeouts, `visible_when`, batch
  flags) now depend on the wire-fixture equivalence test to catch regressions, since the wire type itself
  cannot represent placement — a fixture drift is the only signal for silent metadata loss on that path.
- The bidirectional executor guard proves existence and delivery kind, not audit-tier value; an interaction
  silently moved from an audited plugin tier to the unaudited fallthrough tier (or vice versa) is not
  caught by this ADR's guard and remains a hand-authored correctness class in `local_executor.rs`.

### Out of scope (deferred)

- Interaction id renames (`list`, `discover`, …) and the transitional dual-registration machinery that
  would be needed to support them — only needed if a rename ever happens.
- Permission typing: `Option<String>` → `Option<Permission>` on descriptors and gates (own spec).
- A `DirectBuiltInApi` delivery variant — zero plugin users today; `InteractionDelivery` is
  `#[non_exhaustive]`, so adding it later is additive.
- Modeling surface registration payloads in `asyncapi.yaml` (pre-existing gap, not introduced or closed by
  this change).

## Cross-references

- Spec: `docs/superpowers/specs/2026-07-16-interaction-system-unification-design.md`
- Prior spec (origin of the `list-all-unmatched` finding):
  `docs/superpowers/specs/2026-07-15-proxmox-guest-flow-provider-invocable-design.md`
- Plan: `docs/superpowers/plans/2026-07-16-interaction-unification-c-cutover-guard-docs.md`
- Registration types: `crates/plugins/infrastructure/core/src/registration.rs`
- Agent-side authoring: `crates/plugins/infrastructure/core/src/agent_interaction.rs`
- Catalog admission and exact-id dispatch: `crates/plugins/infrastructure/core/src/catalog.rs`
- Controller-local executor table: `crates/ui/surface-proxy/src/proxy/controller_local.rs`
- Bidirectional executor guard: `crates/ui/web-api/tests/interaction_executor_guard.rs`
- Notification callback trait method: `crates/plugins/infrastructure/core/src/roles.rs`
  (`NotificationTransport::handle_callback`), `crates/ui/web-api/src/routes/notifications.rs`
- `declare_plugin!` macro: `crates/plugins/infrastructure/core/src/macros.rs`
- `CONTEXT.md` — Plugin, Surface, Interaction glossary entries
