# 0033 — Effective Plugin Enablement and Surface Visibility

**Date:** 2026-07-29
**Status:** Accepted
**Supersedes:** ADR-0006 Decision 4 (single-predicate surfaces gating)

## Context

Instance-plugin enablement had two diverging sources of truth (boot catalog vs live ArcSwap snapshot),
and ADR-0006 Decision 4's visibility gating was enforced on none of the surfaces legs (dead predicate
key on list, no filter on providers/read/invoke/provider-origin).

## Decisions

- **Effective = boot ∧ live.** A plugin is tenant-effective only when the boot catalog constructed it
  AND the live snapshot says enabled. Disable takes effect immediately; enable stays pending-restart
  (ADR-0006 Decision 2 stands — no hot-reload). Scope note: disable-is-immediate is
  controller-process-local (the snapshot is an in-memory ArcSwap); the external scheduler and MQTT
  services carry independent boot-time catalogs and are unaffected by a live toggle — this ADR gates
  the controller's surfaces legs only.
- **Two gates, two questions.** "May this user know the plugin exists?" — per-user
  `is_plugin_visible_to_user`, admin override intact, governs plugin listing/config endpoints only.
  "Is this plugin's surface functionality live?" — user-independent effective enablement, governs every
  surfaces leg, all tiers, including provider-origin. No admin override on surfaces legs.
- **Structural enforcement.** Tenant-facing `SurfaceRegistry` enumeration/resolution methods take a
  required `SurfaceProviderVisibility` parameter; `SurfaceProxy` stores the same filter at construction
  with a deny-all-plugin-providers default (fail-closed). The stored filter holds live handles and is
  the only gate on the provider-origin leg.
- **Fail-closed.** A Plugin-kind provider whose provider_id resolves to no descriptor is not visible.
- **404 unification.** A hidden surface is byte-identical to an unknown surface on read/providers/invoke;
  the read leg's previous distinct `NoTenantCompatibleProvider` response for known-but-incompatible
  surfaces is intentionally collapsed into `SurfaceNotFound` (no existence side-channel).
- **Absent row = disabled; no seeding.** Deliberate; the admin UI lists Instance plugins from
  descriptors regardless of row.
- **`delete_channel` stays unguarded** (cleanup must work for no-longer-compiled channel types); the
  surface-dispatch leg covers the disabled-plugin path upstream.
- **Registration follows construction.** `surface_registrations()` skips boot-disabled Instance
  plugins, so the visible-but-undispatchable class dies at the root. ADR-0032 coordination: the boot
  gate lives only on the catalog's runtime accessor; monotonicity guards keep reading descriptor-level
  builders and must never point at the filtered output.

## Per-tier outcome matrix

| Boot     | Live              | Non-admin surfaces legs  | Admin surfaces legs | Admin instance-plugins UI |
| -------- | ----------------- | ------------------------ | ------------------- | ------------------------- |
| enabled  | enabled           | listed + dispatchable    | same                | enabled                   |
| enabled  | disabled          | absent + 404 (immediate) | same                | disabled (took effect)    |
| disabled | enabled           | absent + 404             | same                | pending restart badge     |
| disabled | disabled / no row | absent + 404             | same                | disabled                  |

`running_enabled` intentionally reports boot state (pending-restart badge input); live-disable closes
the surface/transport gates only — a boot-constructed singleton's background tasks run until restart.

## Provider-to-descriptor resolution

`PluginEffectiveEnablement` (`crates/ui/web-api/src/visibility.rs`) implements
`SurfaceProviderVisibility::plugin_provider_visible` by scanning `PluginMetadataOps::all()` for the
descriptor whose `descriptor.surfaces.provider_id` equals the requested `provider_id`, then applying
`effective_instance_enabled` to that descriptor's `PluginTypeId`. A `provider_id` that resolves to no
descriptor is not visible (fail-closed) — `provider_id` is never treated as a `PluginTypeId` directly;
the two identifier spaces are distinct and the match is by field equality, not by reuse.

**Amended by ADR-0034:** the authored `provider_id` field is deleted; the filter now resolves
`provider_id` directly as the plugin type id (the identity is structural, so the "never treated as a
PluginTypeId" caveat no longer applies).

**Alternatives rejected:**

- **`provider_to_type` index on the catalog:** rejected in a prior review round — `all()` already
  exposes the needed set, and an index couples correctness to string literals a rename would rewrite.
  The small-N linear scan over compiled-in descriptors is not a hot path (surface resolution, not
  per-request-item dispatch) and needs no cached index.
- **Full hot-reload** (rebuild catalog on toggle): reverses ADR-0006 Decision 2; singleton teardown +
  in-flight safety + background-task cancel compose non-trivially; not forced by the use case.
- **Strict restart-required** (both legs read boot only): disabling a misbehaving plugin would have
  zero runtime effect until restart — wrong fail direction. (Scope honesty: live-disable closes the
  surface-dispatch and transport gates only; it is not a kill switch — a boot-constructed singleton's
  background tasks run until restart, per the hot-reload non-goal.)
- **Registry learns auth types:** inverts the dependency direction (surface-proxy ← web-api auth
  types); cycle or duplication.
- **Per-leg call sites + CI grep gate:** keeps the drift class that produced this bug; the prior
  docs-only mitigation for a same-class bug in this area is already disproven by recurrence.

## References

- Spec: `docs/superpowers/specs/2026-07-27-plugin-enablement-surface-visibility-design.md`
- Plan: `docs/superpowers/plans/2026-07-29-plugin-enablement-surface-visibility.md`
- CONTEXT.md glossary entry: "Effective Enablement"
