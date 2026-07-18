# Shared Surface API

This document describes the runtime API used for built-in and provider-backed shared surfaces.

## Endpoints

### `GET /api/v1/surfaces`

Lists registered surfaces visible to the authenticated tenant.

Query params:

- `slot` (optional): return only surfaces in this slot
- `page` (optional): page alias filter (`settings`, `software`, `hosts`, `surfaces`)

Response: `SurfaceResponse[]`

```json
[
  {
    "surface_id": "ssh-agent.hosts",
    "label": "SSH Hosts",
    "priority": 500,
    "slot": "surface.page",
    "scope": "tenant",
    "targeting": "targeted",
    "required_permission": "manage_hosts",
    "provider_kind": "service",
    "required_capabilities": [
      "table_node",
      "targeted_targeting",
      "mutation_action"
    ],
    "root_node": {
      "kind": "text_block",
      "text": "..."
    },
    "provider_count": 1
  }
]
```

### `GET /api/v1/surfaces/{surface_id}/providers`

Lists providers for targeted surfaces.

Response: `SurfaceProviderInfo[]`

- `availability`: `available`, `disconnected`, or `incompatible_tenant`
- `encryption_metadata`: present when provider supports encrypted sensitive params

### `GET /api/v1/surfaces/{surface_id}`

Returns read model for a surface:

- descriptor
- interactions
- data sources

Response: `SurfaceReadResponse`

The previous path, with a trailing `/read` segment, was removed in this change (404; no deprecation window — all
in-repo clients updated atomically).

### `POST /api/v1/surfaces/{surface_id}/interactions/{interaction_id}`

Invokes an interaction.

Request body: `InvokeSurfaceInteractionRequest`

```json
{
  "params": {
    "host_id": "019585f4-1234-7000-8000-000000000001"
  },
  "target_provider_id": "service:ssh-agent:019585f4-1234-7000-8000-000000000001",
  "idempotency_key": "f286b304-7198-4e1f-9fcf-56f659560b38",
  "timeout_seconds": 30
}
```

`idempotency_key` is deliberately a body field, not an `Idempotency-Key` header (settled decision A3 of the
2026-07-16 spec — no header precedent in this API; server generates one when omitted).

Sensitive values are sent via `encrypted_sensitive_params`, not plaintext `params`.

## Error codes

Lookup/routing errors:

- `surface_not_found`
- `interaction_not_found`
- `target_provider_required`
- `invalid_provider`
- `no_provider`

Invocation failures:

- `forbidden`
- `invalid_request`
- `schema_validation_failed`
- `provider_unavailable`
- `duplicate_request`
- `rate_limited`
- `timeout`

## Caching

Every surface GET response carries `Cache-Control: private, no-store`. Results are per-tenant and per-permission, so
shared caches and bfcache must never serve them across users.

## Authorization

`list`/`providers` are authenticated-only, with results filtered by descriptor visibility; `read`/`invoke` enforce the
dynamic permissions declared by the surface descriptor/interaction, advertised in OpenAPI via the human-readable
`x-required-permission` extension. See [Shared Surface Security](../security/surfaces.md) for the full model.

## OpenAPI

All four endpoints are registered in `crates/ui/web-api/openapi.json` (operation ids `list_surfaces`,
`get_surface_read`, `list_surface_providers`, `invoke_surface_interaction`). Descriptor-bearing fields are documented
as free-form JSON, with the surface contract model in `crates/shared/surfaces/` remaining the canonical shape.

## Types

Canonical type ownership:

- REST request/response envelopes: `crates/shared/web-api-types/src/surfaces.rs`
- Contract model: `crates/shared/surfaces/`
- Wire barrel re-export: `crates/shared/wire/src/surfaces.rs`

## Notes

- `/api/v1/surfaces/*` is the active runtime path for shared surfaces.
- Frontend and backend use the same `/api/v1/surfaces/*` API family.
