---
title: Shared Surface Security
weight: 170
description:
  Shared surfaces security enforced by fail-closed contract admission, per-request authorization, and transport
  controls.
---

# Shared Surface Security

## Overview

Shared surfaces let plugins and connected services project UI capabilities through the controller. Security is enforced
by a fail-closed contract admission model plus per-request authorization and transport controls.

## Permission model

Authorization is evaluated at two levels, both carried on the wire as `required_action: Option<String>` — a
canonical `resource:verb` catalog action string:

- descriptor-level: `SurfaceDescriptor.required_action`
- interaction-level: `InteractionDescriptor.required_action`

Each value is parsed to a catalog `Action` by `SurfaceProxy` at **registration admission**, not at request time.
An unparseable value rejects the whole registration (`SurfaceProviderRejectionCode::SchemaOrLimitFailure`) — every
surface and interaction the provider registered in that call is absent, not just the offending one. A
parseable-but-currently-unregistered action is admitted; whether it grants access is decided later, per request, by
`AccessEngine`. The registry stores the parsed `Action` index-aligned with the normalized registration, so web-api
never re-parses the string at request time.

Enforcement points:

- `GET /api/v1/surfaces` and `GET /api/v1/surfaces/{surface_id}/providers` — authenticated-only; no static permission
  variant exists for surface listing and inventing one is out of scope, so results are visibility-filtered per
  descriptor instead
- `GET /api/v1/surfaces/{surface_id}` — descriptor action check
- `GET|POST|PUT|DELETE /api/v1/surfaces/{surface_id}/interactions/{interaction_id}` and the
  `.../{interaction_id}/{item_id}` variants — descriptor and interaction action checks, on every method

Every method-mapped interaction route resolves in the same order: unknown surface/interaction is `404`; then the
descriptor action, then the interaction action (`403` on deny, `500` if `AccessEngine`'s authority is
`Unavailable` — fail-closed); only then a method mismatch (`405`, with an `Allow` header). So the full resolution
order is **404 → 403/500 → 405**. The action check runs **before** the method-mismatch check specifically so an
unauthorized caller cannot probe an interaction's registered method set by comparing `403` against `405` across
methods. See [Shared Surface API](../api/surfaces.md#resolution-order-and-405-semantics) for the full resolution
order and the two distinct `Allow`-header shapes.

Read and invoke enforce the dynamic descriptor/interaction actions in-handler via `enforce_required_action`, which
runs the resolved `Action` through `AccessEngine`: `None` action allows; `Ready` authority + `Allow` decision
allows; `Ready` + deny returns `403` and increments the `uptrakit_access_denies_total` counter; `Unavailable`
authority returns `500` (fail-closed, never silently permissive). This is a documented exception to the platform's
typed-permission-extractor rule, because the required action is runtime descriptor data no static extractor can
carry; the OpenAPI operations advertise the runtime-valued requirement via the boolean `x-action-dynamic: true`
extension, paired with an authenticated-only security declaration — the enforced requirement itself lives in the
registered descriptor/interaction, not in the spec. See [Authentication and
Authorization](auth-and-authorization.md#runtime-valued-permission-extension-surfaces) for how this exception class
is distinguished from the platform's other two documented extractor exceptions.

Denied-audit entries record the failing value under the `required_action` key in `details_json` (renamed from
`required_permission`); the `reason_code` literal `missing_required_permission` and the `permission_scope` key are
deliberately unchanged.

Frontend filtering is convenience only; server checks are authoritative. **Known regression (until M1.7):** the SPA's
client-side filter still compares action strings against legacy permission names, so action-gated surfaces are
hidden in the web UI for all users regardless of their actual access. Server-side enforcement via `AccessEngine` is
unaffected — this is a display-only gap, not an authorization bypass.

### GET query strings and sensitive data

`DataLoad` interactions are dispatched over `GET`, so their params travel in the URL query string rather than a
request body. Query strings are visible in server access logs, browser history, and (for cross-origin navigations)
the `Referer` header — a materially different exposure surface than a JSON body.

This is an accepted trade because `DataLoad` params are never allowed to carry secrets: admission validation rejects
any `DataLoad` interaction that declares a non-empty `sensitive_fields` list, with the error `data-load interaction
{id} in surface {surface_id} must not declare sensitive_fields (GET params travel in query strings)`. A provider
that legitimately needs to pass a sensitive value into a load path must model it as a non-`DataLoad` interaction
kind instead (which keeps `POST`/`PUT`/`DELETE` semantics and can use `encrypted_sensitive_params`).

**Failure mode for providers that violate this rule** (e.g. an older or out-of-repo provider build predating this
admission check): registration of the offending surface is rejected outright at admission time. The surface is
simply absent from `GET /api/v1/surfaces` and every interaction on it is unreachable — there is no partial
registration and no runtime error path that a caller could trigger by invoking the interaction. The provider's
connection/registration logs show the rejection reason; end users see no trace of the surface at all.

### Provider-origin invocation

Provider-origin (service-initiated) calls carry no user; they are gated by tenant scope plus the provider-permission
gate: an interaction with `required_action` is denied to `CallerOrigin::Provider` unless it sets
`provider_invocable`.

`provider_invocable = true` means **any same-tenant provider with the `UiSurfaces` capability may invoke the
interaction, subject only to tenant scope** and the standard idempotency/timeout/rate controls. It is a deliberate
privilege expansion, accepted because tenant services are co-trusted (enrolled by the tenant admin), writes are
tenant-scoped and recoverable, and every provider-origin invocation is audit-attributed to the calling service.

Registration admission rejects the flag on permissioned interactions from `Service`-kind providers (Plugin/BuiltIn-owned
interactions only); per-caller narrowing is deliberately outside the flag — a future optional allowlist field composes
with it.

Handlers of flagged interactions must not treat provider origin as privileged beyond tenant membership.

### Effective-enablement gate

Every tenant-facing surfaces leg — list, providers, read, invoke, and provider-origin invocation —
is gated on the owning plugin's effective enablement (boot ∧ live, ADR-0033) via a required
`SurfaceProviderVisibility` filter on the registry's resolution methods. The posture is fail-closed:
a Plugin-kind provider that resolves to no compiled-in descriptor is never visible, and
`SurfaceProxy` defaults to denying all plugin providers unless the production filter is wired in.
A hidden surface's response is byte-identical to an unknown surface's (404, no existence
side-channel), for every permission tier — there is no admin override on the surfaces legs.

## Response caching

Surface GET responses set `Cache-Control: private, no-store`; results are per-tenant and per-permission data that must
not be cached by shared caches or bfcache.

## Registration admission controls

Service and plugin providers are admitted through `SurfaceRegistry` with strict validation:

- framework generation compatibility (`supported_generation`)
- required capability coverage
- slot ID validation against central slot registry
- provider-kind/transport compatibility
- provider-id namespace per source kind (`service.` / `builtin.` / neither for plugins) — fail-closed
  (ADR-0034)
- tenant-binding correctness for authenticated service context
- allowlist checks for controller queries and SSE topics
- contract shape and depth limits
- payload and interaction count limits

Invalid registrations are rejected with structured rejection reasons.

## Targeting and tenancy controls

Targeted surfaces require explicit `target_provider_id` at invocation time. Provider resolution is tenant-aware;
cross-tenant providers are excluded from availability and dispatch.

For service providers:

- tenant services must register tenant-scoped bindings matching authenticated tenant
- system services must register global scope with no tenant binding

## Sensitive parameter handling

Sensitive interaction values are not sent in plaintext `params`. Clients send `encrypted_sensitive_params` (ECIES P-256
metadata + ciphertext payload).

Controller behavior:

- validates presence/shape requirements
- forwards ciphertext opaquely to provider
- never decrypts sensitive payloads

Provider behavior:

- publishes encryption metadata in provider info
- decrypts payload locally using provider private key

## Action execution controls

Invocation path is mediated by `SurfaceProxy`:

- idempotency key handling (`duplicate_request` protection)
- per-request timeout handling
- provider disconnect and unavailability handling
- typed error-code mapping for caller-safe failure semantics
- in-flight budget/idempotency cancellation-safety: if a caller disconnects mid-request, the proxy releases the
  provider/tenant in-flight budget and the idempotency reservation on future-drop (RAII guards) plus a deadline-keyed
  backstop sweep, so a disconnecting client cannot permanently exhaust a provider's or tenant's in-flight budget or
  wedge an idempotency key

## Capability gating

UI/surface runtime participation is gated on protocol capability negotiation. Incompatible providers are excluded from
the active surface catalog; their registrations are rejected at admission time rather than silently degrading runtime
behavior.

## Trust boundaries

- Browser never talks directly to services for surface actions.
- Service-to-controller calls are authenticated over mTLS WebSocket.
- Controller mediates all provider selection and dispatch.
- NATS is not used for session-targeted surface action payload delivery.

## Key files

| File                                                     | Purpose                                                       |
| -------------------------------------------------------- | ------------------------------------------------------------- |
| `crates/shared/surfaces/src/`                            | Shared surface contract and validation policy types           |
| `crates/shared/wire/src/wire_validate_impls.rs`          | Wire-level payload validation                                 |
| `crates/ui/web-api/src/surface_registry.rs`              | Registration admission and tenant/provider indexing           |
| `crates/ui/surface-proxy/src/proxy.rs`                   | Invocation correlation, idempotency, timeout, and routing     |
| `crates/ui/web-api/src/routes/surfaces.rs`               | Authz enforcement and API error mapping                       |
| `crates/ui/web-api/src/routes/service_ws/handler/mod.rs` | Service message handling for surface registration and actions |

## See also

- [Shared Surface Development](https://github.com/worried-networking/uptrakit/tree/main/docs/development/)
- [Shared Surface Architecture](https://github.com/worried-networking/uptrakit/tree/main/docs/architecture/)
- [Shared Surface API](https://github.com/worried-networking/uptrakit/tree/main/docs/api/)
- [Auth and Authorization](auth-and-authorization.md)
