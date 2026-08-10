# 40. Provider-origin surface invocation enforces the descriptor action gate

Date: 2026-08-10

## Status

Accepted

## Context

Surface authorization is a two-layer model: a surface descriptor can carry `required_action`, and each
of its member interactions can independently carry its own `required_action`. A surface author may gate
only the descriptor and leave member interactions ungated, relying on the descriptor gate to protect
them — the HTTP path (`crates/ui/web-api/src/routes/surfaces.rs`) enforces both layers via
`enforce_required_action` on every surface read and interaction invoke, so this pattern is safe for
browser/API callers.

The provider-origin path (service-initiated invocation over the service WebSocket,
`CallerOrigin::Provider`) did not honor the same contract. The gate in `SurfaceProxy::invoke_inner`
consulted only `resolved.interaction_required_action`; it never read `resolved.descriptor_required_action`.
An interaction that relied entirely on its home surface's descriptor gate — ungated itself, no
`provider_invocable` — was therefore invocable by any same-tenant service that had registered at least
one surface provider, with no action check at all.

This gap was live, not theoretical: six first-party notification `DataLoad` reads (channel/SMTP/settings
listings across the email, telegram, and webhook plugins) sat on descriptor-gated surfaces with fully
ungated interactions, exactly the shape the gap exposed. A grep across `agent-ssh-runtime`,
`mqtt-runtime`, `service-sdk`, and the infra plugins found no first-party service actually invoking any
of the six, so the gap was latent, but it was reachable by any enrolled same-tenant service with the
`UiSurfaces` capability. Two of the six additionally crossed an instance-scoped gate
(`system.settings:manage`, the `ADR-0006` instance-scope permission under its current action-string name)
into tenant-service reach, which the two-layer model is not meant to allow.

The admission path had a matching gap on the write side: `validate_interaction_provider_rules` rejected
`provider_invocable` on a `Service`-kind interaction only when that interaction's own `required_action`
was set, leaving service-kind registrations free to set `provider_invocable` on an ungated interaction
that sat under an action-gated surface descriptor — the same escape hatch the runtime gate did not check.

## Decision

Provider-origin invocation is denied when **either** the surface descriptor or the interaction carries
`required_action`, unless the interaction sets `provider_invocable`. `SurfaceProxy::invoke_inner`
(`crates/ui/surface-proxy/src/proxy.rs`) now guards on
`resolved.descriptor_required_action.is_some() || resolved.interaction_required_action.is_some()`, so the
provider-origin path enforces the same two-layer model the HTTP path already enforced.

Registration admission is tightened to match: `validate_interaction_provider_rules`
(`crates/shared/surfaces/src/protocol.rs`) additionally rejects a `Service`-kind registration when an
interaction sets `provider_invocable` and its home surface descriptor carries `required_action`, even if
the interaction's own gate is unset. `Plugin`/`BuiltIn`-kind providers are unaffected by this admission
rule — `provider_invocable` under a gated descriptor remains registrable for them, preserving the escape
hatch for the flows it exists for (for example the proxmox `match`/`unmatched-guests` interactions, which
are invoked by services on a plugin-owned surface within tenant co-trust).

The combined effect: wherever any gate exists on a surface — descriptor, interaction, or both — a service
can only reach it in provider-origin mode through an interaction that both belongs to a plugin/built-in
provider and explicitly opts in via `provider_invocable`. A service can no longer register the opt-in on
its own gated surfaces, and the engine denies the call regardless of who registered it.

## Consequences

The six latent `DataLoad` reads flip from allowed to denied for provider-origin callers. HTTP callers are
unaffected, because the descriptor gate already ran for them on that path. No first-party service invoked
any of the six (grep-verified), so no shipped flow breaks; every proxmox/docker/ssh-agent/mqtt interaction
already gates itself at the interaction level or sets `provider_invocable` deliberately, so the change is
a no-op for them. An out-of-tree integration driving one of the six reads over a service connection would
now receive `PermissionDenied`, and an out-of-tree service registering the outlawed
`provider_invocable`-under-gated-descriptor shape would be rejected at registration instead of accepted —
both are release-notes items, not regressions of a documented contract.

A deliberate corollary follows from tightening admission alongside the engine gate: a service can no
longer provider-invoke an interaction on its **own** descriptor-gated surface at all, because the engine
denies the call and admission removes the only service-side opt-in that could have permitted it.
Descriptor-gated service-registered surfaces are user/HTTP-driven only from this point on; a service
provider that needs a provider-origin-invocable interaction must leave the owning surface's descriptor
ungated and gate (or leave open) only at the interaction level.

The runtime escape hatch stays interaction-keyed — `provider_invocable` is still read from the
interaction, not from a new descriptor-level flag — so no new field was added to the wire contract; only
the set of registrations and invocations it can protect changed. Plugin/built-in owners keep their
existing semantics unchanged.

This decision is distinct from [ADR-0006](0006-instance-scoped-plugins.md). ADR-0006 defines the
instance-scoped plugin model and the `system.settings:manage` gate that two of the six reads crossed; it
is not superseded or revisited here. This ADR concerns a different, orthogonal decision — how the
provider-origin invocation path enforces the two-layer action-gate model that already existed — and the
instance-scope gate was simply one of several gates that the provider-origin path had been failing to
enforce, not the subject of this decision.
