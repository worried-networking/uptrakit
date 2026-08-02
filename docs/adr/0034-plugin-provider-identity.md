# 0034 — Plugin Provider Identity

**Date:** 2026-08-02
**Status:** Accepted
**Amends:** [ADR-0028](0028-single-source-plugin-interaction-registration.md), [ADR-0033](0033-effective-plugin-enablement-and-surface-visibility.md)

## Context

`declare_plugin!`'s `surfaces:` arm carried a hand-authored `provider_id` string
(`surfaces: { provider_id, registrations }`, ADR-0028) alongside the plugin's real identifier, its
`type_id`. Nothing forced the two to agree — `provider_id` was free text chosen at the call site
(commonly `"plugin.<family>.<name>"`), while every other subsystem (config, enablement, discovery
allowlists) keys off `type_id`. ADR-0033's `PluginEffectiveEnablement::plugin_provider_visible` had to
resolve the mismatch at runtime: scan every descriptor for the one whose `descriptor.surfaces.provider_id`
field equals the requested `provider_id`, then apply enablement to _that descriptor's_ `PluginTypeId` —
with an explicit caveat that "`provider_id` is never treated as a `PluginTypeId` directly", because
nothing guaranteed it could be.

Separately, `SurfaceRegistry` admission had no namespace enforcement across the three provider kinds.
Nothing stopped a plugin from authoring a `provider_id` that collided with the shape a `Service` provider
mints (`service.<name>.<uuid>`) or a future `BuiltIn` provider would use (`builtin.<name>`), and nothing
stopped a `Service`/`BuiltIn` registration from omitting its namespace prefix. The two problems compound:
an authored, unconstrained `provider_id` is both an unnecessary indirection (it duplicates `type_id` in
the common case) and an unguarded surface for cross-kind identity collision.

## Decision

### D1 — The authored `provider_id` is deleted; for `Plugin` kind it IS the type id

The `surfaces:` arm's `provider_id` field is gone; `declare_plugin!`'s single arm is now
`surfaces: { registrations }`. Derivation is structural, not authored: `PluginCatalog::surface_registrations()`
threads `descriptor.type_id` into `PluginSurfaceRegistration::to_wire(provider_id)` for every registration
it aggregates. A plugin provider's wire `provider_id` is, by construction, the owning descriptor's
`type_id` — there is no second call site where a different value could be supplied, so the two identifier
spaces ADR-0033 had to reconcile at runtime collapse into one.

### D3 — Admission enforces provider-id namespaces per source kind, fail-closed

`SurfaceRegistry::validate_registration_basics` rejects, per registration source kind:

- `Service` registrations whose `provider_id` is not `service.`-prefixed
- `BuiltIn` registrations whose `provider_id` is not `builtin.`-prefixed
- `Plugin` registrations whose `provider_id` starts with either reserved root

Every violation is a hard rejection (`SurfaceProviderRejectionCode::InvalidTransport`), fail-closed — no
kind is exempt, no root is optional. The type-id category allowlist enforced by the first-party naming
guard (`KNOWN_TYPE_ID_CATEGORIES` in `crates/plugins/infrastructure/registry/tests/surface_id_naming_guard.rs`)
reserves the same two roots at the authoring boundary: `service` and `builtin` are deliberately absent from
the allowlist, and a dedicated test (`no_descriptor_type_id_uses_a_reserved_admission_root`) asserts no
compiled-in plugin `type_id` ever starts with either root. Because D1 makes `provider_id` equal `type_id`
for plugins, the authoring-time guard and the admission-time rule enforce the same invariant from opposite
ends — a type id that slipped into a reserved root would fail the guard test long before it could reach
admission and silently drop its surfaces at boot.

### D4 — No-parse regime: identity is a function of the kind discriminant, not a grammar

`provider_id → type_id` is the identity function for `Plugin`-kind providers, gated only on
`provider_kind == Plugin` (checked by the registry call sites before consulting any visibility filter) —
never on a string grammar. There is no `FromStr`, no `ProviderId` newtype, and no ad-hoc `parse()`:
the one call site that needs a typed handle (`PluginEffectiveEnablement::plugin_provider_visible`) converts
via `PluginTypeId::new()`, an infallible constructor (`impl Into<String>`), not a fallible parse — a
provider id reaching that filter is pre-gated to `Plugin` kind by its callers, and
`effective_instance_enabled` is itself fail-closed on an unrecognized type id.

This ADR deliberately does **not** add the kind-gated `SurfaceCatalogItem::plugin_type_id()` accessor an
earlier design sketch called for. Its intended consumer was the ADR-0033 visibility filter, which this
ADR's D1 refactored to resolve `provider_id` directly (no separate typed accessor needed at that call
site) — leaving the accessor with zero callers. A zero-caller accessor is dead weight, not a completed
interface; if a future derived encoding needs the typed conversion (see below), add the accessor and the
parse together, against a real caller.

Any future scheme that encodes more than the bare type id into `provider_id` (a prefix, a suffix, any
structured value beyond identity) must introduce the typed parse at that point — this ADR's no-parse
posture applies to the current bare-id model only, not as a permanent ban on ever adding one.

### Singleton identity, not per-instance

Plugin providers are singletons keyed by `type_id`: a surface is declared once, at the plugin _type_
(descriptor) level, not per running instance or per named config. A plugin with multiple named
configuration profiles — several GitHub release-source profiles, for example — still has exactly one
surface descriptor and hence one provider identity; which profile a call targets is a within-provider
concern (config selection, `ProviderQuery` params), not a separate identity.

Per-tenant or per-instance plugin provider identities — the pattern `Service` providers already use,
minting `service.<name>.<uuid>` per connected instance — are a recorded **non-goal**. Supporting it would
require a derived-encoding migration (a `provider_id` shape beyond the bare type id) plus the typed parse
D4 defers; nothing in this ADR forecloses that future work, but nothing in the current model needs it.

This is worth disambiguating from a same-named but orthogonal axis: `PluginScope::Instance` names a
_visibility_ concern — a single per-deployment enablement bit keyed by `type_id`, gating whether the one
provider is effectively enabled at all (ADR-0033). The non-goal above is a _minting_ concern — whether more
than one provider row could exist per plugin type. The word "instance" appears on both axes; they do not
otherwise interact.

## Compatibility

- **No persisted value changes.** `provider_id` was never a stored column — surface registrations are
  computed at catalog-build time from compiled-in descriptors, not persisted. There is nothing to migrate.
- **No wire _schema_ change.** `ProviderIdentity`'s fields (`provider_id`, `provider_kind`,
  `provider_namespace`) are unchanged; only the value a plugin registration carries changes (from an
  arbitrary authored string to the type id), which every existing consumer of `ProviderIdentity` already
  treats as an opaque string. No `asyncapi.yaml` update is needed: surface registration is not part of the
  documented service↔controller wire-protocol message family that file covers.
- **Stale client state self-heals.** A browser tab holding an old-shaped `target_provider_id` from before
  this change re-fetches the surface catalog on reload; there is no client-side cache to invalidate
  explicitly.
- **Doc placement.** Identity value rules (namespace prefixes, the bare-id model) live in
  [Shared Surface Runtime Development](../development/surfaces.md) and
  [Shared Surface Security](../security/surfaces.md); `docs/api/wire-protocol.md` is intentionally not
  edited by this ADR — it documents the service↔controller wire protocol and does not cover the
  surface-registration message family this ADR concerns.

## Consequences

- Plugin authors no longer choose or maintain a `provider_id`; the `surfaces:` arm shrinks to
  `{ registrations }`, and a plugin's provider identity can never drift from its `type_id` because there is
  no second place to author it.
- ADR-0033's visibility filter no longer needs a fail-closed field-equality scan over descriptors to
  reconcile two identifier spaces — `provider_id` is resolved directly as the plugin type id, and the
  filter's own caveat ("never treated as a `PluginTypeId` directly") is retired (see amendment note on
  ADR-0033).
- Admission now actively rejects namespace collisions across provider kinds instead of relying on
  convention; the authoring-side naming guard and the admission-side rule are two independent enforcement
  points for the same reserved-root invariant, so a regression at either layer is caught before the other
  is needed.
- Per-instance plugin provider identity remains unsupported. If it is ever needed, it requires a new
  derived `provider_id` encoding plus the typed parse this ADR deliberately defers — not a relaxation of
  the identity function itself.

## Cross-references

- Amends: [ADR-0028 — Single-Source Plugin Interaction Registration](0028-single-source-plugin-interaction-registration.md)
  (arm shape)
- Amends: [ADR-0033 — Effective Plugin Enablement and Surface Visibility](0033-effective-plugin-enablement-and-surface-visibility.md)
  (provider-to-descriptor resolution)
- Related: [ADR-0031 — Surface Identifier Naming Convention](0031-surface-identifier-naming.md) (plugin
  type id grammar and category allowlist)
- Structural derivation: `crates/plugins/infrastructure/core/src/catalog.rs`
  (`PluginCatalog::surface_registrations()`)
- Registration authoring: `crates/plugins/infrastructure/core/src/registration.rs`
  (`PluginSurfaceRegistration::to_wire`), `crates/plugins/infrastructure/core/src/descriptor.rs`
  (`PluginSurfaceRegistrationOps`)
- Admission enforcement: `crates/ui/surface-proxy/src/registry.rs`
  (`SurfaceRegistry::validate_registration_basics`)
- Visibility resolution: `crates/ui/web-api/src/visibility.rs` (`PluginEffectiveEnablement::plugin_provider_visible`)
- Naming/admission guards: `crates/plugins/infrastructure/registry/tests/surface_id_naming_guard.rs`
- Development documentation: [Shared Surface Runtime Development](../development/surfaces.md)
- Security documentation: [Shared Surface Security](../security/surfaces.md)
- `CONTEXT.md` — Surface Provider glossary entry
