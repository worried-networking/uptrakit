# 0031 — Surface Identifier Naming Convention

**Date:** 2026-07-26 **Status:** Accepted

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
