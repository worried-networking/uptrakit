# System Services

System services are tenant-agnostic infrastructure components (MQTT bridge, external scheduler) that
operate across all tenants. Unlike regular tenant services, they are stored in a dedicated
`system_services` table and authenticated via named system enrollment tokens
(`system_enrollment_tokens` table) rather than per-tenant enrollment tokens.

## Overview

The controller supports two service tiers:

| Tier            | Table             | Scoped to                   | Enrollment token                                    | Example services                         |
| --------------- | ----------------- | --------------------------- | --------------------------------------------------- | ---------------------------------------- |
| Tenant services | `services`        | Tenant (`tenant_id` column) | Per-tenant Argon2id tokens in `enrollment_tokens`   | Agents, SSH agents, MQTT bridge (legacy) |
| System services | `system_services` | Global (no `tenant_id`)     | Named Argon2id tokens in `system_enrollment_tokens` | MQTT bridge, external scheduler          |

The MQTT bridge and external scheduler must serve all tenants simultaneously. Placing them in a
per-tenant `services` table would require associating them with an arbitrary tenant or duplicating
rows, neither of which reflects the deployment model. The `system_services` table provides an
explicit home for these infrastructure components and keeps tenant data cleanly separated.

## The `system_service` Capability as Routing Discriminant

Enrollment is routed at the wire protocol level by detecting `system_service` in the
`EnrollPayload.capabilities` set.

```text
EnrollPayload.capabilities:
  - contains "system_service"  →  do_enroll_system_service()  →  system_services table
  - does not contain it        →  do_enroll()                 →  services table
```

The routing happens inside `enroll_service()` in
`crates/ui/web-api/src/routes/service_ws/connection.rs`. The resulting `is_system: bool` flag
threads through every subsequent operation on the same connection — credential delivery, activity
tracking, certificate lookup, and status polling all branch on `is_system`.

### Current system service capability sets

| Component                                 | Capabilities                                                                                                               |
| ----------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| MQTT bridge (`uptrakit-mqtt`)             | `system_service`, `update_tracking`, `graceful_shutdown`, `workload_claims`, `ui_surfaces`                                 |
| External scheduler (`uptrakit-scheduler`) | `system_service`, `scheduler`, `database_access`, `nats_access`, `master_key_access`, `ca_management`, `graceful_shutdown` |

## Database Schema

### `system_services` table

| Column                   | Type                 | Description                                         |
| ------------------------ | -------------------- | --------------------------------------------------- |
| `id`                     | UUID (PK, v7)        | Service identifier                                  |
| `capabilities`           | TEXT                 | JSON array of capability strings                    |
| `hostname`               | TEXT                 | Hostname reported at enrollment                     |
| `friendly_name`          | TEXT                 | Human-readable display name                         |
| `ip_address`             | TEXT (nullable)      | Client IP address, refreshed on each connect        |
| `status`                 | TEXT                 | `pending`, `approved`, `rejected`, or `deactivated` |
| `enrollment_secret_hash` | TEXT (UNIQUE)        | SHA-256 hash of the enrollment secret               |
| `client_version`         | TEXT (nullable)      | Client software version                             |
| `last_seen_at`           | TIMESTAMP (nullable) | Last connect or heartbeat time                      |
| `created_at`             | TIMESTAMP            | Row creation time                                   |
| `updated_at`             | TIMESTAMP            | Last modification time                              |
| `deactivated_at`         | TIMESTAMP (nullable) | Soft-delete timestamp                               |
| `ping_interval_seconds`  | INTEGER (nullable)   | Per-service ping interval override                  |
| `cert_lifetime_hours`    | INTEGER (nullable)   | Per-service certificate lifetime override in hours  |

There is no `tenant_id` column and no `enrollment_token_id` column. System services are global and
use a separate enrollment mechanism.

### `system_service_certificates` table

| Column              | Type                             | Description                                    |
| ------------------- | -------------------------------- | ---------------------------------------------- |
| `ca_fingerprint`    | TEXT (PK)                        | Fingerprint of the signing CA                  |
| `serial_number`     | TEXT (PK)                        | Certificate serial number                      |
| `system_service_id` | UUID (FK → `system_services.id`) | Owning system service                          |
| `not_before`        | TIMESTAMP                        | Certificate validity start                     |
| `not_after`         | TIMESTAMP                        | Certificate validity end                       |
| `revoked_at`        | TIMESTAMP (nullable)             | Revocation timestamp                           |
| `revocation_reason` | TEXT (nullable)                  | `certificate_renewed` or `service_deactivated` |
| `created_at`        | TIMESTAMP                        | Row creation time                              |
| `last_seen_at`      | TIMESTAMP (nullable)             | Last connection using this certificate         |

The FK points to `system_services`, not `services`. The two revocation reasons reflect that system
services cannot be merged (unlike tenant services), so only renewal and deactivation trigger
revocation.

## Credential Guard

System credentials — `database_access`, `nats_access`, `master_key_access`, and `ca_management` —
grant access to sensitive infrastructure secrets (database URL, NATS URL, master encryption key).
These must never be issued to tenant services.

The guard runs at enrollment time, before any database write:

```rust
const SYSTEM_CREDENTIAL_CAPS: &[&str] = &[
    "database_access",
    "nats_access",
    "master_key_access",
    "ca_management",
];

if requests_system_creds && !has_system_service {
    bail!(AgentRouteError::Forbidden(
        "system credentials require the system_service capability"
    ));
}
```

A service that includes any of the four credential capabilities in its `EnrollPayload` without also
including `system_service` receives an `ErrorCode::EnrollmentFailed` response with a message stating
that the `system_service` capability is required. The connection is not closed abnormally — the error
is a soft response.

The guard applies to the **tenant enrollment path** (`do_enroll`). The system enrollment path
(`do_enroll_system_service`) is only reached when `system_service` is already present, so the
capability intersection is correct by construction.

## Enrollment Flow

### System enrollment tokens

Multiple named system enrollment tokens are stored in the `system_enrollment_tokens` table
(`crates/shared/db/src/entity/system_enrollment_token.rs`), superseding the earlier single
global plaintext token that was stored under `SettingKey::SystemServicesEnrollmentToken` in the
`settings` table. Tokens are backend-generated random secrets, Argon2id-hashed at rest, and shown
only once at creation (in the `token` field of `SystemEnrollmentTokenCreatedResponse`). Each token
supports an optional usage limit (`max_uses`) and TTL (`expires_at`).

At enrollment time, if the service provides a token:

1. `find_active_system_tokens()` retrieves all non-revoked, non-expired tokens with remaining uses.
2. `password::verify_password()` performs Argon2id verification against each candidate.
3. On match, `current_uses` is atomically incremented and `system_enrollment_token_id` is recorded
   on the `system_services` row. This link is audit-only (no FK constraint), so a token can be
   revoked or deleted after the service has enrolled without affecting the service record.
4. A matching token produces `Approved` status; no match produces `Forbidden`; no token provided
   produces `Pending`.

REST API: `POST/GET /api/v1/system-enrollment-tokens`, `GET/DELETE
/api/v1/system-enrollment-tokens/{id}` (requires the `system.settings:manage` action). OpenAPI client:
`crates/shared/openapi-client/src/system_enrollment_tokens.rs`. CLI: `uptrakit
system-enrollment-tokens list|create|show|revoke`. Full request/response shapes and the
enrollment-behaviour table are documented in
[HTTP Web API](../api/http-web-api.md#system-enrollment-token-endpoints).

### Auto-approve vs Pending

```text
enrollment_token provided?
  yes → matches an active, non-expired, non-revoked token with uses remaining?
          yes → Approved
          no  → Forbidden (enrollment rejected)
  no  → Pending
```

After enrollment, if the service is immediately `Approved`, the controller sends `Enrolled` followed
by `Approved` in the same WebSocket session so the service can proceed to CSR submission without
waiting for a poll cycle.

## Certificate Lifecycle

System service certificates follow the same flow as tenant service certificates:

1. After enrollment and approval, the service generates an ECDSA P-256 keypair and submits a CSR.
2. The controller validates the CSR and signs it using the active CA.
3. The signed certificate is stored in `system_service_certificates` with FK to the owning system service.
4. The service reconnects using mTLS with the new certificate.

The `ServiceCertCheckExecutor` scheduled task (`service_cert_check`) also covers system service
certificates for proactive renewal when they approach the configured renewal window.

On deactivation (`DELETE /api/v1/system-services/{id}`), the handler:

1. Soft-deletes the `system_services` row (sets `deactivated_at`).
2. Revokes all associated `system_service_certificates` rows (sets `revoked_at`,
   `revocation_reason = service_deactivated`).
3. Fires `revocation_notify` to trigger a local CRL rebuild.
4. Publishes `RequestCrlRenewal` to NATS (if configured) for cross-controller CRL sync.
5. Unregisters the service from `ServiceConnectionRegistry`.

All three mutations are wrapped in a single database transaction (atomic semantics). If any step
fails, the transaction rolls back and the service remains active.

## WebSocket Routing: `is_system` Threading

The `is_system: bool` flag is determined at connection setup and carried through the session. It
controls which database table is queried for every subsequent operation.

### Certificate lookup

On authenticated (mTLS) connect, the controller resolves the certificate by trying the tenant table
first, then the system table:

```text
service_certificates (serial = X, service_id = Y)?
  found     → is_system = false
  not found → system_service_certificates (serial = X, system_service_id = Y)?
                found     → is_system = true
                not found → close: CertificateNotRecognized
```

When a serial is absent (reverse proxy does not forward the cert serial), the lookup falls back to a
service-ID-only query against both tables in the same order.

### Activity recording

`record_service_activity()` and `record_system_service_activity()` are separate functions that update
the corresponding table's `last_seen_at` and `ip_address` columns.

### Enrolled path (Bearer secret)

The `ConnectionType::Enrolled { service_id, is_system }` variant is populated during the Bearer
secret lookup in `service_ws/mod.rs`. The lookup queries `system_services` first via
`lookup_by_secret()` when no service-ID hint is present, or queries the indicated table directly
when a `service_id` query parameter is present.

## Two-Tier Service Model

```text
┌─────────────────────────────────────────────────────┐
│                   WebSocket /api/v1/ws/service       │
│                                                      │
│  EnrollPayload.capabilities contains system_service? │
│         yes ──────────────────────────┐              │
│         no  ───────────┐              │              │
└────────────────────────┼──────────────┼──────────────┘
                         ↓              ↓
              ┌──────────────────┐  ┌──────────────────┐
              │  Tenant Services │  │  System Services │
              │  (services table)│  │ (system_services │
              │  tenant_id FK    │  │    table)        │
              │  per-tenant      │  │  global,         │
              │  enrollment      │  │  no tenant_id    │
              │  tokens          │  │  named tokens    │
              │  (Argon2id)      │  │  (Argon2id)      │
              └──────────────────┘  └──────────────────┘
              REST: /api/v1/services   REST: /api/v1/system-services
```

| Property                     | Tenant services                        | System services                                       |
| ---------------------------- | -------------------------------------- | ----------------------------------------------------- |
| Database table               | `services`                             | `system_services`                                     |
| `tenant_id`                  | Required                               | None                                                  |
| `enrollment_token_id`        | FK to `enrollment_tokens`              | None                                                  |
| `system_enrollment_token_id` | None                                   | Audit-only link to `system_enrollment_tokens` (no FK) |
| Enrollment token storage     | Argon2id hash in `enrollment_tokens`   | Argon2id hash in `system_enrollment_tokens`           |
| Token comparison             | Argon2id verify                        | Argon2id verify                                       |
| Token returned to operator   | No (hash only)                         | Yes, once, at creation (plaintext)                    |
| Certificate table            | `service_certificates`                 | `system_service_certificates`                         |
| Merge support                | Yes                                    | No                                                    |
| REST path prefix             | `/api/v1/services`                     | `/api/v1/system-services`                             |
| Action (read)                | `services:read`                        | `system.services:read`                                |
| Action (approve/reject)      | `services:approve` / `services:reject` | `system.services:approve` / `system.services:reject`  |
| Action (update)              | `services:update`                      | `system.services:update`                              |
| Action (delete)              | `services:delete`                      | `system.services:delete`                              |

## Surfaces and Config Delivery Are Untenanted

System services are tenant-agnostic end to end — this extends to the two subsystems that used to
special-case a "default tenant" for them. See [ADR-0042](../adr/0042-untenanted-system-service-surfaces-and-a-single-runtime-owned-mqtt-definition.md)
for the full rationale.

### Shared surface registrations

A system service that participates in the shared surface runtime (currently the MQTT bridge —
see [Surfaces and Config Delivery](../development/surfaces.md)) registers with `Scope::Global` and
`Targeting::Universal`, and its `effective_tenant_binding.tenant_id` is `None`. Registration never
depends on resolving a tenant to bind to, so it can never be skipped for lack of one — unlike the
previous tenant-bound registration model, which failed to register at all when no default tenant
was available.

Because the provider registration itself carries no tenant, tenancy for interactions on that
surface is resolved per request instead: the controller stamps each
`SurfaceActionRequest.tenant_id` from the authenticated caller before dispatch
(`crates/ui/surface-proxy/src/proxy/dispatch.rs`), and the provider filters its own state (for
example, MQTT client configs) by that value. One MQTT service instance serves multiple tenants'
broker connections this way — tenancy is a property of each MQTT client config row, never of the
service or its surface registration.

`AdminEvent::SurfacesChanged` is broadcast globally (`EventBroadcaster::send_global`) when a
Global-scope provider's surfaces change, and per-tenant otherwise.

When an embedded system service yields to an external counterpart claiming the same app name, the
controller unregisters the yielded service's surface provider and fails its in-flight requests
(`evict_yielded_service_surfaces`,
`crates/ui/web-api/src/routes/service_ws/handler/surface_eviction.rs`) synchronously with the yield
handoff, so the external service's equivalent Universal registration is admitted rather than
rejected as a provider conflict.

### One MQTT service definition, two deployment modes

The MQTT service's deployment facts — app name, capability set, `ServiceScope::System`,
`YieldPolicy`, embedded shutdown timeout — are declared exactly once, in
`crates/core/mqtt-runtime/src/bootstrap.rs`. Both the standalone `uptrakit-mqtt` binary and the
controller's embedded-service host (`--features embedded-mqtt`,
`crates/core/controller-runtime/src/service_host/builtins.rs`) read these facts from that module
rather than declaring their own copies, so the two deployment modes cannot drift apart.

Embedded system-service bridges (scheduler, MQTT) are untenanted by construction — the shared
`spawn_system_bridge` helper always builds the bridge session with `service_tenant_id: None`, never
a per-service value.

### One config-delivery path for every embedded service

Every embedded service — agent, agent-ssh, scheduler, MQTT — receives its stored
[service config store](../development/service-config-store.md) entries through the same delivery
path and the same audit trail, whether over an external mTLS WebSocket or the in-process
connection registry, and whether or not it has any stored entries. See
[Service Config Store](../development/service-config-store.md) for the wire-level detail.

## Frontend

The frontend filters services by capability instead of type and displays `service_label` instead
of `service_type`.

## Related Documentation

- [Services and Operations](../api/services-operations.md) — REST endpoint details for both tiers
- [HTTP Web API](../api/http-web-api.md) — full endpoint reference
- [Authentication and Authorization](../security/auth-and-authorization.md) — action/grant authorization model
- [Wire Protocol](../api/wire-protocol.md) — capabilities and enrollment flow
- [Scheduler Architecture](scheduler.md) — external scheduler as a system service
- [ADR-0042](../adr/0042-untenanted-system-service-surfaces-and-a-single-runtime-owned-mqtt-definition.md) —
  untenanted system-service surfaces and the single runtime-owned MQTT definition
