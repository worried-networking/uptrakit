# ATK-06: Multi-Tenancy Isolation Bypass

| Field | Value |
| --- | --- |
| Severity | Medium |
| Attack surface | Database / API (future multi-tenancy) |
| Prerequisites | Authenticated user in one tenant; multi-tenancy enabled (future) |
| STRIDE | Information Disclosure |

## Attack description

1. Once multi-tenancy is enabled, a user authenticated in tenant A crafts API
   requests targeting resources belonging to tenant B.
2. The `TenantContext` middleware currently always returns `state.default_tenant_id`
   regardless of the request. When multi-tenancy is activated, this middleware must
   resolve the correct tenant from the authenticated user's context.
3. If the tenant resolution is incomplete or bypassed, the attacker's queries pass
   through `TenantDb` with the wrong `tenant_id`, potentially accessing cross-tenant
   data.

Specific vectors in the current architecture:

- **Notification callback endpoint.** The generic callback handler
  (`POST /api/v1/notifications/callback/{channel_type}/{channel_id}`) bypasses `TenantDb`
  and loads channels by primary key directly. It does not enforce tenant scoping
  because the endpoint is unauthenticated (external services like Telegram call it).
- **Action token lookup.** `find_log_by_action_token()` queries `notification_log`
  directly without a `tenant_id` filter, using only the action token UUID. A valid
  action token grants access to the log entry regardless of tenant.
- **Global settings.** Thirteen setting keys are stored in the global `global_settings`
  table (no `tenant_id` column). Changes to these settings affect all tenants.

## Worst-case impact

- **Cross-tenant data exposure.** An attacker in one tenant reads software items,
  hosts, plugin configs, and notification channels belonging to another tenant.
- **Cross-tenant update triggering.** The attacker triggers updates on hosts belonging
  to another tenant by manipulating software item IDs or host IDs in API requests.
- **Cross-tenant configuration modification.** The attacker modifies plugin configs,
  notification channels, or OIDC providers in another tenant, potentially injecting
  malicious webhook URLs or shell commands.
- **Global settings manipulation.** Settings like `trusted_proxies`, `pki_addr`, and
  `mqtt_max_clients_per_tenant` affect all tenants and could be modified by any tenant
  admin with the right permission.

## Current mitigations

- **Single-tenant mode is active.** Multi-tenancy is not yet enabled. The
  `TenantContext` always returns the default tenant, and all data is associated with
  this single tenant. Cross-tenant attacks are not possible today.
- **`TenantDb` query scoping.** All tenant-scoped queries go through `TenantDb`, which
  automatically applies `WHERE tenant_id = ?`. This pattern is structurally sound and
  will enforce isolation when multi-tenancy is activated.
- **`TenantScoped` trait.** Entities that should be tenant-scoped implement the
  `TenantScoped` trait, ensuring `TenantDb` knows which column to filter on.
- **Foreign key constraints.** Cross-table relationships (e.g., `notification_rules` →
  `notification_channels`) use foreign keys that are themselves tenant-scoped,
  preventing structural cross-tenant references.
- **Database-level tenant column.** All tenant-scoped tables have a `tenant_id` column
  with a foreign key to `tenants(id)` and `ON DELETE RESTRICT`, preventing tenant
  deletion while data exists.
- **Action tokens use UUIDv7.** Action tokens have 122 bits of entropy in the random
  portion, making brute-force enumeration infeasible.

## Residual risk

- **`TenantContext` is a stub.** The middleware ignores all request context and returns
  the default tenant. When multi-tenancy is enabled, a comprehensive audit of every
  route handler is needed to ensure correct tenant resolution.
- **Unauthenticated endpoints bypass tenancy.** The Telegram callback and any future
  public endpoints cannot use JWT-based tenant resolution. They must enforce tenant
  scoping through alternative means (e.g., channel-to-tenant lookup).
- **No `X-Tenant-Id` header validation.** The documented future design accepts an
  `X-Tenant-Id` header, but there is no mechanism to validate that the authenticated
  user belongs to the specified tenant.
- **Global settings are shared.** Critical infrastructure settings (trusted proxies,
  PKI address, JWT signing key) are inherently global. A tenant admin with
  `manage_global_settings` permission can affect all tenants.
- **Scheduler is per-tenant but shares plugin credentials.** The scheduler filters by
  `tenant_id` in queries, but plugin configs containing API tokens are not isolated
  per tenant in the plugin execution layer.

## Recommended improvements

- Before enabling multi-tenancy, perform a comprehensive audit of every API route to
  ensure `TenantContext` is used correctly and no queries bypass `TenantDb`.
- Implement user-to-tenant membership mapping and validate the `X-Tenant-Id` header
  against the user's tenant memberships.
- Separate `manage_global_settings` from tenant-level permissions entirely, requiring
  a super-admin role that is not assignable within any single tenant.
- Add integration tests that create two tenants and verify that API requests from one
  tenant cannot read, modify, or trigger actions in the other.
- Consider per-tenant encryption keys (derived from the master key + tenant ID) so
  that database-level cross-tenant data leaks do not expose plaintext secrets.

## References

- [Multi-tenancy Architecture](../architecture/multi-tenancy.md)
- [Notification Subsystem Security](../security/notifications-security.md#tenant-isolation)
- [Auth and Authorization](../security/auth-and-authorization.md)
- `crates/ui/web-api/src/middleware/tenant_context.rs` — `TenantContext` extractor
- `crates/ui/web-api/src/tenant_db.rs` — `TenantDb` query wrapper
