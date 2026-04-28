---
title: Shared Surface Security
weight: 170
description: Shared surfaces security enforced by fail-closed contract admission, per-request authorization, and transport controls.
---

# Shared Surface Security

## Overview

Shared surfaces let plugins and connected services project UI capabilities through the controller.
Security is enforced by a fail-closed contract admission model plus per-request authorization and
transport controls.

## Permission model

Permissions are evaluated at two levels:

- descriptor-level: `SurfaceDescriptor.required_permission`
- interaction-level: `InteractionDescriptor.required_permission`

Enforcement points:

- `GET /api/v1/surfaces` — only tenant-visible surfaces are listed
- `GET /api/v1/surfaces/{surface_id}/read` — descriptor permission check
- `POST /api/v1/surfaces/{surface_id}/interactions/{interaction_id}` — descriptor and interaction
  permission checks

Frontend filtering is convenience only; server checks are authoritative.

## Registration admission controls

Service and plugin providers are admitted through `SurfaceRegistry` with strict validation:

- framework generation compatibility (`supported_generation`)
- required capability coverage
- slot ID validation against central slot registry
- provider-kind/transport compatibility
- tenant-binding correctness for authenticated service context
- allowlist checks for controller queries, SSE topics, and direct built-in operations
- contract shape and depth limits
- payload and interaction count limits

Invalid registrations are rejected with structured rejection reasons.

## Targeting and tenancy controls

Targeted surfaces require explicit `target_provider_id` at invocation time.
Provider resolution is tenant-aware; cross-tenant providers are excluded from availability and
dispatch.

For service providers:

- tenant services must register tenant-scoped bindings matching authenticated tenant
- system services must register global scope with no tenant binding

## Sensitive parameter handling

Sensitive interaction values are not sent in plaintext `params`.
Clients send `encrypted_sensitive_params` (ECIES P-256 metadata + ciphertext payload).

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

## Capability gating

UI/surface runtime participation is gated on protocol capability negotiation.
Provider reports are also used by rollout guard logic so incompatible required providers prevent
activation of shared-surface runtime.

## Trust boundaries

- Browser never talks directly to services for surface actions.
- Service-to-controller calls are authenticated over mTLS WebSocket.
- Controller mediates all provider selection and dispatch.
- NATS is not used for session-targeted surface action payload delivery.

## Key files

| File | Purpose |
| --- | --- |
| `crates/shared/surfaces/src/` | Shared surface contract and validation policy types |
| `crates/shared/wire/src/wire_validate_impls.rs` | Wire-level payload validation |
| `crates/ui/web-api/src/surface_registry.rs` | Registration admission and tenant/provider indexing |
| `crates/ui/web-api/src/surface_proxy.rs` | Invocation correlation, idempotency, timeout, and routing |
| `crates/ui/web-api/src/routes/surfaces.rs` | Authz enforcement and API error mapping |
| `crates/ui/web-api/src/routes/service_ws/handler/mod.rs` | Service message handling for surface registration and actions |

## See also

- [Shared Surface Development](../development/extensions.md)
- [Shared Surface Architecture](../architecture/extensions.md)
- [Shared Surface API](../api/extensions.md)
- [Auth and Authorization](auth-and-authorization.md)
