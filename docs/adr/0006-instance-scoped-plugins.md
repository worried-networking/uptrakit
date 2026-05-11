# 0006 — Instance-Scoped Plugins

**Date:** 2026-05-11
**Status:** Accepted

## Context

The plugin system previously recognised only one configuration scope: Tenant-Scoped plugins, where
each tenant enables and configures a plugin independently. A new class of plugin — the
Instance-Scoped Plugin — requires enable/disable and configuration to be governed at the instance
level by an administrator with `ManageGlobalSettings` authority. The first concrete example is
`dashboard-icons`, a controller-only background enrichment service.

Four decisions shaped the implementation. Each is recorded here with the rejected alternative and
the rationale.

## Decision 1 — Dedicated `instance_plugin_setting` table, not raw keys in `global_settings`

Instance plugin state is stored in a purpose-built `instance_plugin_setting` table with columns
`(plugin_type_id, enabled, config, updated_at)`, one row per instance-scoped plugin type.

The rejected alternative was raw key-value pairs in `global_settings` using a naming convention
such as `plugin.<id>.enabled` and `plugin.<id>.config`.

`global_settings` already has a typed-key contract enforced by the `SettingKey` enum. Plugin-prefixed
raw keys would bypass that contract, making the set of valid keys implicit and compiler-invisible.
Beyond maintainability, a dedicated table makes per-row change events (needed for a future hot-reload
path) and single-shot administrative queries trivial; a query like "list all enabled instance plugins"
needs no key-prefix parsing, no regex scans, and no JSON extraction.

## Decision 2 — Restart-required toggle; hot-reload deferred

In v1, toggling `enabled` persists immediately to `instance_plugin_setting`, but the catalog reads
the table only at controller boot. Constructing or destructing instance-plugin singletons requires
a controller restart.

The rejected alternative was hot-reload: broadcast an invalidation signal, lazily spawn or cancel
background tasks (such as the dashboard-icons cache refresh loop) on toggle, and reason about
partially-constructed singletons.

Hot-reload requires solving three concurrent problems: a broadcast channel whose consumers are all
plugin singletons that may themselves hold async tasks, safe spawn/cancel semantics for background
loops with their own retry and backoff state, and concurrency correctness during the window when a
plugin is being torn down while a tenant request that passed the visibility predicate is still in
flight. All three are solvable, but they compose non-trivially and none are forced by the v1 use
case.

The decoupled snapshot architecture — catalog snapshot built at boot, separate web-api snapshot
fetched per request — keeps hot-reload achievable as an additive change. No structural rewrites are
required to add a broadcast channel later.

The gap between stored state and running state is made honest by the API surface.
`InstancePluginSummary` exposes both `enabled` (the desired state stored in the table) and
`running_enabled` (the catalog snapshot captured at boot). The UI renders a "Pending restart" badge
when the two values differ, so operators always know whether a toggle has taken effect.

## Decision 3 — Reuse `ManageGlobalSettings` permission; no new permission variant

Instance plugin administration (enable/disable, config) is gated on the existing
`ManageGlobalSettings` permission.

The rejected alternative was a new `ManageInstancePlugins` variant in the `Permission` enum.

The persona is the same: instance owners who already hold `ManageGlobalSettings` are the natural
administrators for instance-scoped plugins. A new permission variant would require: a non-exhaustive
enum churn that must be forward-declared and matched at all existing wildcard arms, a frontend
role-mapping table update, and additional friction for admins who must now hold two permissions to
do a single coherent task. A future split — if operational requirements ever dictate delegating
plugin management to a role that cannot change network or SMTP settings — is an additive change.
Nothing in the current architecture prevents it.

## Decision 4 — Single visibility predicate; disabled plugins return 404

A single predicate, `crate::visibility::is_plugin_visible_to_user`, gates every plugin-listing
endpoint and the surfaces registry. When an instance-scoped plugin is disabled, tenant users
receive a 404 response on the plugin's endpoints. The 404 matches the existing "unknown plugin type"
response shape so there is no existence side-channel for tenants.

The rejected alternative was per-handler ad-hoc filtering with bespoke 404 or 403 responses
distributed across route files.

A single predicate means there is exactly one place to update when scope semantics change. Per-handler
filtering scatters the same conditional across every plugin route, making it easy for a new plugin
surface to miss the check and expose an enabled/disabled state that tenant users should not see.
The 404 (rather than 403) choice is deliberate: returning 403 acknowledges the resource exists
and leaks the fact that the plugin is installed but disabled. The 404 shape is indistinguishable
from an unrecognised plugin type, eliminating that information.

### Out-of-scope leakage vectors (v1 invariant)

The predicate covers HTTP endpoints and the surfaces registry. It does not cover:

- **AdminEvent SSE:** dashboard-icons does not emit `AdminEvent`; this invariant must be maintained
  as the plugin evolves.
- **Agent-side runtime:** dashboard-icons is controller-only; agents have no knowledge of it.
- **MQTT topics:** dashboard-icons does not publish to MQTT.
- **Persisted side effects on tenant-readable rows:** `software_item.icon_url` may carry CDN URLs
  from prior enrichment runs. Disabling the plugin stops future enrichment but does not retroactively
  clear existing URLs. This is an accepted known limitation; no provenance column tracks which rows
  were populated by which plugin. Future plugin authors must walk the §6 leakage vectors checklist
  in the design spec before shipping.

## Consequences

**Positive:**

- Single source of truth for instance-plugin admin state. The `instance_plugin_setting` table is
  the authoritative record; no inference from key naming conventions is needed.
- Tenant-facing surface area is unchanged when a plugin is enabled. The predicate blocks the
  plugin from appearing in tenant listings and surfaces only when it is disabled.
- The restart UX is honest. The `running_enabled` / `enabled` pair surfaces the difference between
  desired state and running state so operators are never surprised.
- Future plugins promoted to `Instance` scope inherit the entire mechanism — table, API routes,
  visibility predicate, catalog snapshot — without schema changes. Only the plugin type must be
  declared with `PluginScope::Instance`.

**Negative:**

- A restart is required to pick up enable/disable toggles. The "Pending restart" badge mitigates
  the confusion but does not eliminate the operational friction.
- Instance-scoped plugins have two distinct config surfaces: the per-instance config managed through
  this API, and the `type_settings` config shared with tenant-scoped plugins. The interaction between
  these surfaces is not fully documented in v1. Documentation in `docs/development/plugin-guidelines.md`
  is deferred to Plan C.
- The visibility predicate must be consciously re-applied at every new tenant-readable plugin surface.
  Plugin authors adding new endpoints, surfaces, or background events must consult the §6 leakage
  vectors checklist in the design spec. The predicate is not applied automatically by the framework.

## References

- Spec: `docs/superpowers/specs/2026-05-10-instance-scoped-plugins-design.md`
- Plans: `docs/superpowers/plans/2026-05-10-instance-scoped-plugins-{a,b,c}.md` (Plan A merged,
  Plan B is this work, Plan C is downstream)
- CONTEXT.md glossary entries: "Plugin Scope", "Instance-Scoped Plugin"
