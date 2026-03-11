# Multi-tenancy

The codebase supports multi-tenancy at the database and API levels. Currently only **single-tenant mode** is active —
multi-tenant mode is planned for a future release.

## Tenants table

The `tenants` table stores tenant records. A seeded **default tenant** (with `is_default = true`) is created by the
initial migration. All data in single-tenant mode is associated with this default tenant.

| Column | Type | Notes |
| --- | --- | --- |
| `id` | UUID PK | `Uuid::now_v7()` |
| `name` | String | Human-readable name |
| `slug` | String (unique) | URL-safe identifier |
| `is_default` | Bool | Exactly one row has `true` |
| `created_at` | Timestamp | |
| `updated_at` | Timestamp | |
| `deactivated_at` | Timestamp? | Soft-delete |

## Tenant-scoped tables

The following tables have a `tenant_id UUID NOT NULL` column with a FK to `tenants(id)` ON DELETE RESTRICT:

| Table | Unique constraint change |
| --- | --- |
| `services` | — (index on `tenant_id`) |
| `hosts` | `machine_id` unique → `(tenant_id, machine_id)` |
| `plugin_configs` | — (index on `tenant_id`) |
| `software_items` | `(plugin_config_id, package_identifier)` → `(tenant_id, plugin_config_id, package_identifier)` |
| `oidc_providers` | `slug` unique → `(tenant_id, slug)` |
| `user_roles` | PK `(user_id, role_id)` → `(tenant_id, user_id, role_id)` |
| `settings` | PK `(key)` → `(tenant_id, key)` |
| `mqtt_clients` | Non-unique index on `tenant_id` (multiple clients per tenant, limit controlled by `MqttMaxClientsPerTenant` global setting) |
| `settings_version` | PK `tenant_id` (one row per tenant, version counters for cross-instance cache invalidation) |

## Tables NOT changed (remain global)

`users`, `roles`, `permissions`, `role_permissions`, `sessions`, `api_tokens`, `global_settings`, `pending_*` tables,
`host_software_items`, `available_versions`. Note: `service_certificates` and `service_hosts` are tenant-scoped through
the `services` table FK.

## TenantContext extractor

Route handlers that operate on tenant-scoped data accept a `TenantContext` extractor
(`crates/ui/web-api/src/middleware/tenant_context.rs`). It implements `FromRequestParts<Arc<AppState>>`:

1. Reads the `X-Tenant-Id` HTTP header.
1. If present and non-empty: parses as UUID, uses it as the tenant.
1. If absent: falls back to `state.default_tenant_id`.

In single-tenant mode, the header is optional — all requests default to the default tenant.

## AppState.default_tenant_id

`AppState` has a `default_tenant_id: uuid::Uuid` field, loaded at startup by querying the seeded default tenant from the
DB. It is used:

- As the fallback in `TenantContext` when no header is provided.
- For per-tenant settings and data scoping.
- In middleware and auth flows that don't have a per-request tenant context.

## Global vs tenant-scoped settings

Global settings are stored in a dedicated `global_settings` table (PK: `key`, no `tenant_id` column). Per-tenant
settings remain in the `settings` table (PK: `(tenant_id, key)`).

`SettingKey::is_global()` returns `true` for the 13 system-wide settings:

- **Network:** `TrustedProxies`, `RealIpHeader`, `Sans`, `HttpsAddr`,
  `ForwardedClientCertInfoHeader`, `ForwardedClientCertPemHeader`, `PkiAddr`
- **PKI:** `PkiActiveCaFingerprint`, `PkiCaVersion`
- **System:** `MultiTenancyEnabled`
- **MQTT:** `MqttMaxClientsPerTenant`
- **Auth:** `JwtSigningKey` (encrypted)
- **Crypto:** `MasterKeyVerification` (encrypted)

Global settings are read/written via dedicated functions (`load_global_setting`, `upsert_global_setting`,
etc.) that operate directly on the `global_settings` table without a `tenant_id`. At startup,
`Settings::load()` queries both tables and merges the results.

## Future multi-tenancy work

- Tenant management API (CRUD for tenants)
- Multi-tenant JWT (per-tenant permissions in token)
- Tenant-aware MQTT (per-tenant broker config or topic prefix)
- Tenant switching UI
- API token scoping per tenant
