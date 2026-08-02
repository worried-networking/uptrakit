# 0031 — Surface Identifier Naming Convention

Date: 2026-07-26

## Status

Accepted

## Context

The shared charset validator for surface contracts (`validate_surface_identifier`,
`crates/shared/surfaces/src/ids.rs` — first char `[a-z]`, rest `[a-z0-9._-]`) permits every naming style at once, so
first-party providers drifted along three independent axes:

1. **kebab-case vs snake_case** — notifications used `configure_smtp`, `save_global_smtp`, surface IDs like
   `notifications.email.global_smtp`; every other provider used kebab-case.
2. **CRUD verbs baked into IDs** — `list`, `get-info`, `get_smtp`, `preload-*`, `load-*`,
   `create`/`edit`/`delete`, `mqtt.create-client`, `remove-host` — semantics now carried by the HTTP method
   ([ADR-0030](0030-surfaces-rest-method-model.md)) instead.
3. **Namespace prefixes** — MQTT prefixed every interaction (`mqtt.list-clients`) and data source
   (`mqtt.clients.primary`); proxmox prefixed data sources (`proxmox.hosts.mappings`); every other provider used
   bare IDs scoped implicitly by the surface.

The prior rule in `docs/development/surfaces.md` (data-retrieval = noun phrases, mutations = verbs) was
document-only and widely violated, which is why this ADR pairs the convention with an executable guard rather than
leaving it as prose alone.

## Decision

### Convention

Interaction IDs, data-source IDs, and surface IDs adopt a single kebab-case, HTTP-method-aware grammar. The
authoritative wording lives in `docs/development/surfaces.md` (kept in sync with this ADR); in summary:

- Interaction and data-source IDs are kebab-case only, with no underscores, dots, or provider/surface prefixes —
  the surface ID already namespaces them.
- Surface IDs are dot-separated kebab-case segments, first segment naming the provider family.
- A CRUD-shaped resource collapses onto one plural noun registered under GET/POST/PUT/DELETE, with a single GET
  registration serving both list and item read (branching on `params["id"]` presence) rather than separate
  `list-`/`get-` interactions.
- Singleton settings resources use a singular noun under GET + PUT; domain operations that are not CRUD use an
  imperative verb phrase under POST; a data source's ID and `ProviderQuery.operation_id` both equal the noun of
  the GET interaction they pair with.

### No transitional aliases; renames land atomically per binary

Renames are not shipped behind dual-registration aliases. Each binary cuts over its own registrations atomically;
because no first-party ID is registered or dispatched across a binary boundary, per-provider commits within a
binary carry no cross-provider coupling. This mirrors the no-compatibility-shim posture already established by
[ADR-0030](0030-surfaces-rest-method-model.md#b3--atomic-breaking-change-no-deprecation-window).

### Enforcement scope: first-party guard tests, third parties get guidance

Guard tests apply to first-party registrations only — the built `PluginCatalog` and the service-runtime
registration builders (`agent-ssh-runtime`, `mqtt-runtime`) are asserted against the identifier grammar and the
data-source/GET-interaction pairing rule. `validate_surface_identifier` itself is left unchanged and permissive:
tightening it would reject older service binaries at admission, and no deprecation window exists yet for that
change (see Deferred, below). Third-party and externally-registered service providers therefore receive this
convention as normative guidance in `docs/development/surfaces.md`, not as a wire-enforced requirement.

### Slot IDs are out of scope

Slot identifiers (`settings.tabs`, `host_detail.tabs`, and the other constants in
`crates/shared/surfaces/src/slot.rs`) are a separate, larger-blast-radius contract and are not touched by this
naming convention.

## Rename scope

The exhaustive, provider-by-provider rename table (current ID → new method + ID, plus every hardcoded reference
site that must move in lockstep) lives in
`docs/superpowers/specs/2026-07-20-surfaces-id-naming-convention-design.md` (§Rename table) and is not reproduced
here — it is a per-provider inventory that changes shape as providers are added, renamed, or removed, and
duplicating it would drift.

## Rejected alternatives

| Alternative                                                              | Why rejected                                                                                                                                                                                                                   |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Transitional dual-registration aliases during the rename                 | Doubles the registered-interaction surface area indefinitely for a rename with no cross-binary coupling to protect; the project's default posture on internal contract changes is a coordinated cutover, not a permanent shim. |
| Tighten `validate_surface_identifier` immediately                        | Would reject already-deployed service binaries (mqtt, agent-ssh) at admission with no deprecation window; deferred until one can be designed.                                                                                  |
| Retype permission/action literals to a typed enum as part of this change | Superseded by the access-management refactoring, which deletes the `Permission` enum outright and retypes these sites to catalog `Action` constants — typing to `Permission` first would be pure churn.                        |

## Consequences

- First-party interaction, data-source, and surface IDs across proxmox, docker, notifications, agent-ssh, and
  MQTT providers are renamed to the new grammar; the affected binaries and per-provider rename inventory are
  tracked in the spec's §Rename table, not in this ADR.
- `CONTROLLER_LOCAL_EXECUTOR_TABLE` (`crates/ui/surface-proxy/src/proxy/controller_local.rs`) and its dispatch
  submodules move in lockstep with the renamed IDs; historical audit `action_type` strings that embed the old
  action name (not the wire ID) are frozen, not renamed, preserving audit-log continuity.
- No DB migration is required — interaction IDs are not persisted, with the narrow exception of a cosmetic
  sudoers regeneration-hint string on managed hosts (`crates/core/agent-ssh-runtime/src/operations/sudoers.rs`),
  which stays stale harmlessly until the next regeneration.
- Old service binaries (pre-rename mqtt/agent-ssh) remain self-consistent against a new controller: the
  permissive wire validator (this ADR's enforcement-scope decision) still admits their old-style IDs, and the
  data-driven frontend invokes whatever is registered.

## Amendment: plugin type IDs

**Date:** 2026-07-27

Plugin type IDs (`PluginTypeId`, `crates/shared/types/src/plugin_type_id.rs`) join this convention. They were
previously fused `snake_case` (`package_manager_apt`, `infrastructure_proxmox`) or, for the three notification
channels, bare channel names (`email`, `telegram`, `webhook`) with no namespace at all.

### Grammar

Plugin type IDs are dot-separated kebab-case segments; the first segment must be one of a known category. The
category list is enforced by `KNOWN_TYPE_ID_CATEGORIES` in the guard test
`crates/plugins/infrastructure/registry/tests/surface_id_naming_guard.rs`
(`all_descriptor_type_ids_follow_dotted_kebab_grammar`), which asserts every registered descriptor's `type_id`
against it: `package-manager`, `releases`, `hook`, `infrastructure`, `generic`, `discovery`, `enhancement`,
`notifications`, `test`. This mirrors the surface/interaction/data-source grammar above but keeps its own guard
because plugin type IDs are a distinct identifier space (catalog registration, not surface routing).

### Category mapping

Every first-party plugin type ID was renamed 1:1 (26 pairs, no additions or removals):

| Category          | Old ID                             | New ID                             |
| ----------------- | ---------------------------------- | ---------------------------------- |
| `package-manager` | `package_manager_apt`              | `package-manager.apt`              |
| `package-manager` | `package_manager_homebrew`         | `package-manager.homebrew`         |
| `package-manager` | `package_manager_dnf`              | `package-manager.dnf`              |
| `package-manager` | `package_manager_npm`              | `package-manager.npm`              |
| `package-manager` | `package_manager_mas`              | `package-manager.mas`              |
| `package-manager` | `package_manager_pacman`           | `package-manager.pacman`           |
| `package-manager` | `package_manager_pkg`              | `package-manager.pkg`              |
| `package-manager` | `package_manager_apk`              | `package-manager.apk`              |
| `package-manager` | `package_manager_snap`             | `package-manager.snap`             |
| `package-manager` | `package_manager_cargo`            | `package-manager.cargo`            |
| `package-manager` | `package_manager_routeros`         | `package-manager.routeros`         |
| `package-manager` | `package_manager_skills`           | `package-manager.skills`           |
| `releases`        | `releases_github`                  | `releases.github`                  |
| `releases`        | `releases_gitlab`                  | `releases.gitlab`                  |
| `releases`        | `releases_forgejo`                 | `releases.forgejo`                 |
| `releases`        | `releases_docker`                  | `releases.docker`                  |
| `discovery`       | `discovery_proxmox_helper_scripts` | `discovery.proxmox-helper-scripts` |
| `discovery`       | `discovery_uptrakit_self_update`   | `discovery.uptrakit-self-update`   |
| `generic`         | `generic_shell`                    | `generic.shell`                    |
| `hook`            | `hook_shell`                       | `hook.shell`                       |
| `hook`            | `hook_systemd`                     | `hook.systemd`                     |
| `infrastructure`  | `infrastructure_proxmox`           | `infrastructure.proxmox`           |
| `notifications`   | `email`                            | `notifications.email`              |
| `notifications`   | `telegram`                         | `notifications.telegram`           |
| `notifications`   | `webhook`                          | `notifications.webhook`            |
| `enhancement`     | `enhancement_dashboard_icons`      | `enhancement.dashboard-icons`      |

The three notification rows are the only ones gaining a namespace rather than just reformatting one:
`email`/`telegram`/`webhook` had no category prefix at all before this amendment. This is a distinct concept
from `channel_type`, which stays the bare runtime-validated string (`"email"`/`"telegram"`/`"webhook"`) used by
the notification-dispatch subsystem (see [Notifications](../development/notifications.md)) — `channel_type` is
not renamed. The helper `notification_plugin_type(channel_type: &str) -> PluginTypeId`
(`crates/shared/types/src/plugin_type_id.rs`, re-exported from `uptrakit_shared_types`) derives the namespaced
plugin type ID from a bare `channel_type` at every conversion site, so the two concepts stay related but
distinguishable in code.

### No wire aliasing

Consistent with this ADR's "no transitional aliases" decision above: the wire `plugin_type` value is renamed
with no dual-registration compatibility shim. Satellite services (agent-ssh, MQTT) are version-locked to the
controller they connect to — an old satellite talking to a new controller is not a supported combination this
project maintains dual-name support for, matching the same posture already established for surface/interaction
IDs.

### DB value remap is best-effort, not schema validation

Unlike interaction/data-source IDs (never persisted), plugin type IDs are persisted as free-text columns across
several tables (`plugin_configs`, `plugin_type_settings`, `instance_plugin_setting`, `host_software_item_plugins`,
`tenant_discovery_allowlist`, `host_discovery_allowlist`, `notification_rules.plugin_type`). Migration
`m20260727_000001_plugin_type_id_grammar` (`crates/shared/db/src/migration/`) remaps all 26 legacy values to
their new form with one setwise `UPDATE` per (table, value-pair) — no per-row loop, no raw SQL. Any value not in
the 26-pair table (e.g. a third-party or since-removed plugin's ID) is left untouched by design: the migration
targets known first-party legacy values, not general schema validation.

## Deferred (named follow-ups, out of scope here)

- Tightening `validate_surface_identifier` for newly-registered contracts once a deprecation window can be
  designed for existing service providers.
- Slot ID normalization (see Decision, above).
- Retiring the notifications surface-CRUD duplicate path in favor of the existing REST channel-management
  family — blocked on UI-redesign rollout.

## Cross-references

- Spec: `docs/superpowers/specs/2026-07-20-surfaces-id-naming-convention-design.md`
- Depends on: [ADR-0030 — Surfaces REST Method Model](0030-surfaces-rest-method-model.md)
- Extends: [ADR-0028 — Single-Source Plugin Interaction Registration](0028-single-source-plugin-interaction-registration.md)
- Charset validator: `crates/shared/surfaces/src/ids.rs`
- Authoring documentation: [Shared Surface Runtime Development](../development/surfaces.md)
- Security documentation: [Shared Surface Security](../security/surfaces.md)
