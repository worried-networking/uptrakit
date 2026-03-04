# Enrollment Tokens API

Enrollment tokens allow services (agents, MQTT bridges, SSH agents) to enroll with
automatic approval. Each token is a named, revocable credential with optional capability
scoping, usage limits, and time-to-live.

All endpoints require the `ManageAgents` permission.

## Overview

| Endpoint | Method | Description |
| --- | --- | --- |
| `/api/v1/enrollment-tokens` | POST | Create a new enrollment token |
| `/api/v1/enrollment-tokens` | GET | List enrollment tokens (paginated) |
| `/api/v1/enrollment-tokens/{id}` | GET | Get a single enrollment token |
| `/api/v1/enrollment-tokens/{id}` | DELETE | Revoke an enrollment token (soft-delete) |

## `POST /api/v1/enrollment-tokens`

Create a new enrollment token. The response includes the plaintext token value exactly
once; it cannot be retrieved later.

**Request body** (`CreateEnrollmentTokenRequest`):

```json
{
  "name": "CI Deploy Token",
  "allowed_capabilities": ["software_discovery", "mqtt_bridge"],
  "max_uses": 10,
  "expires_in_seconds": 86400
}
```

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `name` | string | Yes | Human-readable label for the token |
| `allowed_capabilities` | string[] | No | Restrict to services with at least one overlapping capability. Omit or `null` for a wildcard token that matches any service. |
| `max_uses` | u32 | No | Maximum number of enrollments. Omit or `null` for unlimited. |
| `expires_in_seconds` | u64 | No | Token TTL in seconds from creation. Omit or `null` for no expiration. |

**Response** (`201`): `EnrollmentTokenCreatedResponse`

```json
{
  "id": "019...",
  "token": "upt_abc123def456...",
  "name": "CI Deploy Token",
  "allowed_capabilities": ["software_discovery", "mqtt_bridge"],
  "max_uses": 10,
  "current_uses": 0,
  "expires_at": "2026-02-28T00:00:00Z",
  "created_at": "2026-02-27T00:00:00Z",
  "created_by_user_id": "019..."
}
```

The `token` field contains the plaintext secret. Store it securely; it is hashed with
Argon2id before storage and cannot be recovered.

## `GET /api/v1/enrollment-tokens`

List all enrollment tokens for the tenant, ordered by creation date (newest first).

**Query parameters**: `page` (default 1), `per_page` (default 20, max 1000).

**Response** (`200`): `PaginatedResponse<EnrollmentTokenResponse>`

```json
{
  "items": [
    {
      "id": "019...",
      "name": "CI Deploy Token",
      "allowed_capabilities": ["software_discovery"],
      "max_uses": 10,
      "current_uses": 3,
      "expires_at": "2026-03-01T00:00:00Z",
      "created_at": "2026-02-27T00:00:00Z",
      "revoked_at": null,
      "created_by_user_id": "019..."
    }
  ],
  "total": 1,
  "page": 1,
  "per_page": 20,
  "total_pages": 1
}
```

## `GET /api/v1/enrollment-tokens/{id}`

Get a single enrollment token by UUID.

**Response** (`200`): `EnrollmentTokenResponse`

**Error responses**:

- `404` -- enrollment token not found.

## `DELETE /api/v1/enrollment-tokens/{id}`

Revoke an enrollment token. Sets `revoked_at` to the current time. The token remains
in the database for audit purposes but can no longer be used for enrollment.

**Response** (`200`): `MessageResponse`

```json
{
  "message": "Enrollment token revoked"
}
```

**Error responses**:

- `404` -- enrollment token not found.
- `409` -- enrollment token is already revoked.

## Enrollment Flow

When a service connects and sends `EnrollPayload` with an `enrollment_token` field:

1. The controller loads all active tokens for the tenant (not expired, not revoked,
   uses remaining).
2. For each active token, it verifies the provided secret against the stored Argon2id
   hash.
3. On a hash match, it checks capability intersection: if the token has
   `allowed_capabilities`, at least one must overlap with the service's declared
   capabilities. A `null` (wildcard) token matches any service.
4. If all checks pass, the token's `current_uses` is atomically incremented, the
   service is created with status `approved`, and the `enrollment_token_id` FK is
   recorded on the service for audit.
5. If no token matches, the service is rejected with `Forbidden("Invalid enrollment token")`.

When no `enrollment_token` is provided, the service is created with status `pending`
and requires manual approval.

## Capability Scoping

Tokens can restrict which service types they approve by listing allowed capabilities:

| Token `allowed_capabilities` | Service capabilities | Result |
| --- | --- | --- |
| `null` (wildcard) | any | Match |
| `["software_discovery"]` | `["software_discovery", "update_hooks"]` | Match (intersection: `software_discovery`) |
| `["mqtt_bridge"]` | `["software_discovery"]` | No match (empty intersection) |
| `["software_discovery", "mqtt_bridge"]` | `["mqtt_bridge"]` | Match (intersection: `mqtt_bridge`) |

## Token Lifecycle States

| State | Condition | Can enroll? |
| --- | --- | --- |
| Active | Not revoked, not expired, uses remaining | Yes |
| Revoked | `revoked_at` is set | No |
| Expired | `expires_at` is in the past | No |
| Exhausted | `current_uses >= max_uses` | No |

## Response Types

Types are defined in `crates/shared/web-api-types/src/enrollment_tokens.rs`:

| Type | Fields |
| --- | --- |
| `CreateEnrollmentTokenRequest` | `name`, `allowed_capabilities?`, `max_uses?`, `expires_in_seconds?` |
| `EnrollmentTokenCreatedResponse` | `id`, `token` (SecretString), `name`, `allowed_capabilities`, `max_uses`, `current_uses`, `expires_at`, `created_at`, `created_by_user_id` |
| `EnrollmentTokenResponse` | `id`, `name`, `allowed_capabilities`, `max_uses`, `current_uses`, `expires_at`, `created_at`, `revoked_at`, `created_by_user_id` |
| `EnrollmentTokensSummary` | `active_count` |
| `ListEnrollmentTokensQuery` | `page?`, `per_page?` |

## Key Files

| File | Purpose |
| --- | --- |
| `crates/shared/db/src/entity/enrollment_token.rs` | SeaORM entity |
| `crates/shared/db/src/migration/m20260227_000001_enrollment_tokens.rs` | Database migration |
| `crates/shared/web-api-types/src/enrollment_tokens.rs` | Request/response types |
| `crates/ui/web-api/src/routes/enrollment_tokens.rs` | Route handlers |
| `crates/ui/web-api-queries/src/queries/enrollment_tokens.rs` | Database queries |
| `crates/shared/openapi-client/src/enrollment_tokens.rs` | Typed client methods |

## Related Documentation

- [HTTP Web API](http-web-api.md) -- API overview
- [Services and Operations](services-operations.md) -- service lifecycle
- [Wire Protocol](wire-protocol.md) -- `EnrollPayload.enrollment_token` field
- [Auth and Authorization](../security/auth-and-authorization.md) -- security model
- [CLI Usage](../end-user/cli-usage.md) -- `enrollment-tokens` CLI commands
