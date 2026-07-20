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

Permissions are evaluated at two levels:

- descriptor-level: `SurfaceDescriptor.required_permission`
- interaction-level: `InteractionDescriptor.required_permission`

Enforcement points:

- `GET /api/v1/surfaces` and `GET /api/v1/surfaces/{surface_id}/providers` — authenticated-only; no static permission
  variant exists for surface listing and inventing one is out of scope, so results are visibility-filtered per
  descriptor instead
- `GET /api/v1/surfaces/{surface_id}` — descriptor permission check
- `POST /api/v1/surfaces/{surface_id}/interactions/{interaction_id}` — descriptor and interaction permission checks

Read and invoke enforce the dynamic descriptor/interaction permissions in-handler via `enforce_required_permission` —
this is the documented exception to the platform's typed-permission-extractor rule, because the required permission is
runtime descriptor data no static extractor can carry; the OpenAPI operations advertise it via a human-readable
dynamic `x-required-permission` extension.

Frontend filtering is convenience only; server checks are authoritative.

### Provider-origin invocation

Provider-origin (service-initiated) calls carry no user; they are gated by tenant scope plus the provider-permission
gate: an interaction with `required_permission` is denied to `CallerOrigin::Provider` unless it sets
`provider_invocable`.

`provider_invocable = true` means **any same-tenant provider with the `UiSurfaces` capability may invoke the
interaction, subject only to tenant scope** and the standard idempotency/timeout/rate controls. It is a deliberate
privilege expansion, accepted because tenant services are co-trusted (enrolled by the tenant admin), writes are
tenant-scoped and recoverable, and every provider-origin invocation is audit-attributed to the calling service.

Registration admission rejects the flag on permissioned interactions from `Service`-kind providers (Plugin/BuiltIn-owned
interactions only); per-caller narrowing is deliberately outside the flag — a future optional allowlist field composes
with it.

Handlers of flagged interactions must not treat provider origin as privileged beyond tenant membership.

## Response caching

Surface GET responses set `Cache-Control: private, no-store`; results are per-tenant and per-permission data that must
not be cached by shared caches or bfcache.

## Registration admission controls

Service and plugin providers are admitted through `SurfaceRegistry` with strict validation:

- framework generation compatibility (`supported_generation`)
- required capability coverage
- slot ID validation against central slot registry
- provider-kind/transport compatibility
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
