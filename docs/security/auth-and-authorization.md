---
title: Authentication and Authorization
weight: 40
description: Authentication methods, JWT access token claims, the action-string grant authorization model, and auth middleware behavior in Uptrakit.
---

# Authentication and Authorization

| Method                   | Scope                     | Details                                                                                                                                                                                                                                          |
| ------------------------ | ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Password (Argon2id)      | User login                | Local accounts with hashed passwords.                                                                                                                                                                                                            |
| OIDC                     | User login                | External identity providers with auto-create or account linking. Requires the `oidc` Cargo feature (enabled by default).                                                                                                                         |
| Device authorization     | CLI login                 | RFC 8628-style flow: device code, browser approval, API token issuance. Status tracked via `DeviceAuthStatus` enum (`pending`, `authorized`, `expired`).                                                                                         |
| JWT access tokens        | API requests              | Short-lived tokens that carry no authorization data; the `AccessEngine` resolves authority per request (never stored).                                                                                                                           |
| Refresh tokens           | API requests              | SHA-256 hashed, 7-day expiry, rotated on each use within a DB transaction, revoking the predecessor. Session integrity validated on every use (see below).                                                                                       |
| API tokens               | Programmatic access       | Long-lived, revocable bearer tokens stored in the database.                                                                                                                                                                                      |
| mTLS client certs        | Agent/MQTT connections    | Issued after CSR approval and validated per connection.                                                                                                                                                                                          |
| Forwarded cert headers   | Reverse proxy             | Trusted proxies forward cert info/PEM; issuer CN verified.                                                                                                                                                                                       |
| Enrollment tokens        | Service onboarding        | Multiple named tokens stored in the `enrollment_tokens` table (Argon2id hashed). Each token supports capability scoping, usage limits, and TTL. See [Enrollment Tokens API](https://github.com/worried-networking/uptrakit/tree/main/docs/api/). |
| System enrollment tokens | System service onboarding | Global (non-tenant) named tokens stored in the `system_enrollment_tokens` table (Argon2id hashed). Supports usage limits and TTL. See [System Enrollment Tokens API](https://github.com/worried-networking/uptrakit/tree/main/docs/api/).        |

## JWT Access Token Claims Contract

Every access token minted by `JwtManager::create_access_token` includes:

| Claim         | Value                                     | Purpose                                                |
| ------------- | ----------------------------------------- | ------------------------------------------------------ |
| `iss`         | `"uptrakit"`                              | Identifies the issuing deployment.                     |
| `aud`         | `["uptrakit"]`                            | Restricts token acceptance to Uptrakit instances.      |
| `sub`         | User UUID                                 | Identifies the subject user.                           |
| `exp`         | Unix timestamp                            | Token expiry (15 minutes from issuance).               |
| `jti`         | UUID                                      | Per-token unique identifier used for denylist lookups. |
| `auth_method` | `"password"` \| `"oidc"` \| `"api_token"` | How the user authenticated.                            |

`decode_access_token` validates **all three** of `exp`, `iss`, and `aud`. Tokens that lack any of
these claims, or that carry the wrong values (e.g. tokens issued by a different deployment sharing the
same signing key), are rejected with `AuthError::JwtDecode`. This prevents cross-deployment token
replay attacks in scenarios where the signing key file is accidentally shared or restored from backup.

## JWT Signing Key Storage

The JWT signing key is stored in the `settings` table under `auth.jwt_signing_key` (global scope, base64
encoded) and is encrypted at rest using AES-256-GCM via `encrypt_str()` — the same algorithm used for all
other sensitive fields (`EncryptedString`). On every read the value is decrypted transparently before use.

The JWT key uses the context-bound `ENC:v2:` format with AAD `"uptrakit:settings:jwt_signing_key"`,
preventing this ciphertext from being reused in any other encrypted column even if the master key is
compromised and an attacker attempts a relocation attack.

Legacy unencrypted keys (base64 only, written before encryption was introduced) are transparently
re-encrypted on the next read using `ENC:v2:`. Legacy `ENC:v1:` keys are accepted during decryption
for backward compatibility with existing installations. No operator intervention is required.

See [Secrets Handling and Encryption](secrets-and-encryption.md) for the encryption format and master key
requirements.

## Session Integrity Validation

Refresh token verification and rotation validate session data integrity before proceeding:

- **OIDC sessions** must have a valid `oidc_provider_id`. If the provider ID is missing or corrupt
  (e.g., `auth_method = 'oidc'` but `oidc_provider_id IS NULL`), the session is rejected with
  `AuthError::InvalidSession` and a warning is logged. The session is **never** silently downgraded
  to password authentication.
- **JWT access tokens** with `auth_method = "oidc"` are similarly rejected if `oidc_provider_id`
  is missing or unparseable, returning HTTP 401 instead of falling back to a different auth method.
- **Database constraint**: The sessions table enforces
  `CHECK(auth_method != 'oidc' OR oidc_provider_id IS NOT NULL)` to prevent invalid state at the
  storage layer.
- **Expiry cleanup**: The scheduler's `AuthCleanupExecutor` purges expired `sessions` rows
  (`expires_at < now`, no grace window) alongside other expired auth state, so stale session rows do
  not accumulate unbounded.

See also: [Secrets Handling and Encryption](secrets-and-encryption.md) for encryption-at-rest details.

## OIDC Email Verification Enforcement

`resolve_oidc_user` checks the `email_verified` claim from the OIDC ID token before performing any database
lookup. This prevents account creation or matching for addresses that the identity provider has not confirmed.

| `email_verified` claim | Behavior                                                                                                                                                                                                                   |
| ---------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `true`                 | Accepted — proceeds to user resolution                                                                                                                                                                                     |
| `false`                | **Rejected** — returns `OidcUserResolution::EmailNotVerified`; user is redirected to the login page with `error=email_not_verified`                                                                                        |
| absent / `null`        | **Rejected** — treated the same as `false`. A rogue IdP that omits the claim cannot bypass verification. Providers that omit the claim for confirmed accounts must be configured to always include `email_verified: true`. |

The check occurs at the entry point of `resolve_oidc_user` before any DB query, ensuring no account is created
or linked for an unverified email regardless of `auto_create` or role-mapping configuration.

See also: [`docs/development/coding-standards.md`](https://github.com/worried-networking/uptrakit/tree/main/docs/development/) for the security guard
placement convention.

## Database Error Propagation in Auth Handlers

Database errors in authentication and authorization handlers must always propagate as HTTP 500
Internal Server Error. **Silently defaulting on DB failure is a security defect, not a
graceful fallback.**

### Why defaults are dangerous

| Site                                  | Anti-pattern           | Effect                                                                           |
| ------------------------------------- | ---------------------- | -------------------------------------------------------------------------------- |
| `oidc_auth.rs` — count existing users | `.unwrap_or(false)`    | DB outage → assume zero users → unintended first-admin OIDC registration allowed |
| `oidc_auth.rs` — list OIDC providers  | `.unwrap_or_default()` | DB outage → empty provider list → correct behavior obscured, outage masked       |

`users.rs`'s `build_user_response` (the site formerly listed above) now propagates role-lookup failures with `?`
instead of defaulting; it mints no token and the response carries no permission/action data, so a DB outage there
surfaces as an error rather than a silently degraded response.

`register`, `login`, the OIDC mint, `me`, and the post-MFA/2FA session builders all resolve their
response's `actions` list through `effective_actions()` / `AccessEngine::allowed_actions()` (see [Enforcement
surfaces](#enforcement-surfaces)). None of them propagate a 500 for an `AccessEngine`
failure on this path: the response still returns its normal success status with `actions: []` and
`authority: "unavailable"`. This is deliberate and uniform across the family — the SPA treats any non-2xx from `me`
as an unconditional logout, so a 500 there would eject an already-logged-in user on a transient DB blip, and the
same carve-out now covers the other mint paths so a blip degrades authority instead of failing the request.

### Required pattern

All auth-path DB queries must be propagated with `?` after mapping to an appropriate error:

```rust
// ✓ Correct — DB outage surfaces as 500; access never silently granted or denied
let roles = get_user_role_summaries(state, user_id)
    .await
    .map_err(|e| {
        tracing::error!(err = %e, user_id = %user_id, "failed to load user roles");
        AuthFailure::InternalError
    })?;
```

See [Error Handling — Pattern 20](https://github.com/worried-networking/uptrakit/tree/main/docs/development/)
for the full pattern with examples.

## OIDC Link Token URL Handling

**Risk level:** Low

When OIDC account linking is required the backend redirects to
`/login?link_required=true&email=<email>#link_token=<token>`. The sensitive `link_token`
is placed in the URL fragment instead of the query string, so it is not sent to the server
and does not appear in controller access logs or `Referer` headers.

### Mitigations in place

1. **Single-use, short-lived**: the token is consumed atomically from
   `pending_account_links` on first use and expires server-side within a short window.
2. **Fragment transport**: the browser keeps the token client-side in the fragment, removing
   it from server-side request logs and request targets.
3. **Same-origin redirect**: the redirect is to the same origin (`/login`), so there
   is no cross-origin referrer leakage.
4. **`Referrer-Policy: no-referrer`** is set on the redirect response in
   `crates/ui/web-api/src/routes/oidc_auth.rs` so the token URL is not forwarded in
   `Referer` headers when the browser subsequently loads third-party resources.
5. **User already authenticated**: the token only exists after the user has successfully
   completed OIDC authentication; it does not grant initial access.

### Residual risk

The token still appears in the browser address bar and browser history until the user
leaves or rewrites the page URL. Fragment transport removes server-side logging
exposure, but a compromised or observed client can still capture the token during its
validity window.

## Redirect pinning and pending-flow snapshots

When `oauth.canonical_host` is set, the OIDC `redirect_uri` is built from it for **every** configured
provider at authorize time. Header-derived bases (`base_url_from_headers`, preferring `Origin` then
falling back to `Host`) are used only as a fallback when no canonical host is configured — they are no
longer trusted whenever a canonical host exists. The exact `redirect_uri` string used at authorize time
is stored on the `pending_oidc_flow` row, and the callback replays that stored value instead of
re-deriving it from request headers, closing the authorize/callback divergence that a purely
header-derived scheme allowed.

The return origin observed at authorize time is snapshotted alongside the redirect URI. At callback, it
is validated against the canonical host plus `oauth.accepted_audience_hosts` (`https`-only entries; the
5-entry cap enforced by `CanonicalUrlConfig::new` applies only to the MCP OAuth boot-time config, not to
this callback check), and the post-login success redirect is made absolute to that validated origin.
**`oauth.accepted_audience_hosts` currently has no API or UI write path** — it is settable only by a
direct database row edit until `uptrakit-t6je6` lands. In practice this means that, with a canonical
host configured, the validated return origin is effectively always the canonical host today.

### Purge triggers

Any provider mutation (update, deactivate, delete, activate) purges that provider's pending OIDC flows,
and a changed `oauth.canonical_host` value purges every pending flow for the deployment. Both purges run
in the same `BEGIN IMMEDIATE` transaction as the triggering mutation.

These purges **invalidate pending flows** — they do not cancel all in-progress logins. A narrow race
remains: an authorize request that lands between the settings read and the flow insert (live IdP
discovery sits in that window) can still commit a flow pinned to the old redirect URI after the purge
has already run. That login then fails at the IdP with `redirect_uri_mismatch` rather than completing.
This is an accepted best-effort gap, affecting at most one racing user per canonical-host or provider
change.

### Upgrade note for MCP deployments setting the canonical host

Deployments that already set `oauth.canonical_host` to enable MCP OAuth will see their OIDC login
`redirect_uri` change to the canonical origin. The registered redirect URI at the IdP may need updating
to match. Alternate origins (a LAN IP, a VPN hostname) keep working for the post-login return as long as
they are allowlisted in `oauth.accepted_audience_hosts` — but the canonical callback host must still be
reachable from the client's network regardless: the IdP redirects the browser to the canonical host
first, and only the subsequent return trip can land on an allowlisted alternate origin. Redirect pinning
does not rescue a split-horizon setup where the canonical host itself is unroutable from the client.

## Authorization Model

Authorization is an action-string grant model enforced by a single decision point, the `AccessEngine`. See
[ADR-0039](../adr/0039-replace-enum-rbac-with-action-string-grants-and-a-central-access-engine.md) for why the
prior closed permission enum was replaced. Every named authorization requirement in this codebase is a
`resource:verb` catalog action string (e.g. "requires the `hosts:read` action"), never an enum variant or a bare
permission name.

### Action vocabulary

An action is a `resource:verb` string. The verb side comes from a closed set defined in
`crates/shared/types/src/access/verb.rs` (`read`, `create`, `update`, `delete`, `trigger`, `approve`, `reject`,
`manage`, `use`); adding a verb is an architecture decision, not a routine change. The resource side is open: the
`access_catalog!` macro in `crates/shared/types/src/access/catalog.rs` is the single source of truth for built-in
resources, the per-resource verb validity matrix, per-action descriptions, `SelectorSupport` levels, and the typed
`actions::*` constants (e.g. `actions::HOSTS_READ`) consumed everywhere an action is referenced in code.

Two dynamic namespaces extend the vocabulary at parse time: `plugin.<plugin_type>` and `surface.<surface_id>`. Both
admit the full closed verb set grammatically, but only one is backed by a live `DynamicActionRegistry` today —
`SurfaceActionRegistry` registers `surface.<id>:use` iff a surface with that id is currently registered. Every
other dynamic action (any other verb on `surface.*`, and all of `plugin.*`) is unregistered and therefore denies —
fail closed, no dangling authority.

The OpenAPI schema for `Action` is a documented open string, not a closed enum — the resource set can grow without
a schema-breaking change. There is deliberately no `Other` catch-all: an action string that does not parse as
`resource:verb` against the closed verb set and the catalog/dynamic-namespace grammar is a parse error, and a parse
error is a deny.

### Grant model

Authority is stored as data in the `access_grants` table (`crates/shared/db/src/access_grants.rs`,
`crates/shared/db/src/entity/access_grant.rs`), owned end-to-end by the access-grants query module — it
deliberately does **not** implement `TenantScoped`, since it mixes tenant-scoped rows with global
(`tenant_id NULL`) rows; tenant scoping is the query module's own job, not the shared trait's.

Each grant carries a set of `ActionPattern`s (`ResourcePattern` × `VerbPattern`): `*` for every tenant-plane
resource, an exact resource string, or a `<stem>.*` subtree match, each paired with `*` or a single closed-set
verb. `system.`-prefixed resources are excluded from the resource-side `*` wildcard (the verb-side `*` matches
unconditionally) — a pattern must name the `system.` plane explicitly to match it. Every grant also carries a
`Selector` (`crates/shared/types/src/access/selector.rs`): `All` (no narrowing) or one narrowing axis — `Tags`,
`Hosts`, `Software`, or `Items` (host-software links). Selectors validate fully on write (see [Selector-scoped
grants](#selector-scoped-grants) below) but stay write-gated to `All` until M2.3 lifts targeted enforcement.

Roles are data, not code: `roles.tenant_id` is `NULL` for the global built-in roles and non-`NULL` for
tenant-defined custom roles, with per-scope name uniqueness enforced by a partial-unique index pair rather than a
column constraint. A grant's subject is either a user or a role (`GrantSubject`); role-subject grants are always
tenant-`NULL` since scope comes from `user_roles`, not the grant row.

The built-in roles' seed grants are frozen literal pattern strings, guarded by the
`seed_patterns_stay_valid_against_live_catalog` test — a catalog rename must never edit a seed literal in place;
it ships a forward data migration instead, so historical seed rows keep meaning what they meant when written.

### Selector-scoped grants

A catalog action's `SelectorSupport` level (`None`, `Host`, `HostAndSoftware`) bounds which selector axes it
admits — each level admits the previous levels' kinds too, so `HostAndSoftware` also accepts `Tags`/`Hosts`.
Writing a grant runs its selector through the shared validation chain in `crates/shared/db/src/access_grants.rs`
(`validate_write`), in order, after plane purity and the tenant-encoding rule (rule 2):

- **Rule 5 (+5b)** — bounds on the canonicalized id list (`Selector::canonicalize()` sorts and dedups before
  `validate()` runs) plus a minimum of one id: an empty narrowing list matches nothing, so it is rejected
  (`SelectorValidationError::EmptyIds`) rather than stored as a dead grant.
- **Rule 3 (capability level)** — every catalog action reachable by every one of the grant's patterns must admit
  the selector's axis (`validate_selector_level`); a pattern reaching a dynamically-registered action
  (`plugin.*`/`surface.*`) never admits a non-`All` selector.
- **Rule 4 (referent existence)** — every selector id must name a non-deactivated row in the grant's tenant
  (`validate_selector_referents`). This is an authoring typo-catcher, not a safety boundary — ergonomics, not
  safety: the decision-time matcher (`Selector::covers`) never re-checks it, so a referent later deactivated
  cannot retroactively widen or corrupt an already-stored grant's meaning. Global-role grants
  (`roles.tenant_id IS NULL`) have no tenant to resolve referents in and reject any non-`All` selector outright
  (`AccessGrantError::SelectorOnGlobalRole`) — narrow authority via a direct user-subject grant instead; never by
  adding a selector to a seed or other global role.
- **Gate (last)** — a selector that passes every rule above still hits `SelectorPhaseGate` as the final check in
  `validate_write`, because the M2.3 series invariant requires target-aware enforcement to be site-complete
  before the gate can lift: `ci/verify_access_enforcement_sites.sh` pins that while any production `.authorize(`
  call site is still classified `needs-fine-check` (not yet routed through `authorize_target`) in
  `ci/verify_access_enforcement_sites_inventory.txt`, `SelectorPhaseGate` must still exist in
  `access_grants.rs`. M2.3 deletes the gate arm (and its HTTP mapping) in the same change that converts every
  remaining site to `fine-checked`.

`Visibility::from_selectors` (`crates/shared/types/src/access/decision.rs`) unions the selector axes of every
grant matching an action into a read-scope verdict — `Full` if any matching grant is `All`, `None` if no grant
matches, otherwise a per-axis `Filter` (`tags`/`hosts`/`software`/`items` id sets) — consumed by
`AccessEngine::visibility()`.

### AccessEngine

`AccessEngine` (`crates/ui/controller-core/src/access/mod.rs`) is the single access decision point, split (M2.1)
into a coarse and a fine call that share one internal `decide()` core:

- **`authorize(ctx, action)`** — sync, targetless. Runs, in order: dynamic-action registry lookup (for
  `plugin.*`/`surface.*` actions) → grant match against the principal's resolved authority → token scope ceiling.
  Selector coverage is **not** evaluated here — a principal whose only matching grant carries a non-`All`
  selector is allowed at this gate. Most enforcement sites still call this coarse gate today; per the M2.3 series
  invariant, each production call site is classified `needs-fine-check` or `fine-checked` in
  `ci/verify_access_enforcement_sites_inventory.txt`.
- **`authorize_target(ctx, action, target)`** — async, target-aware. Runs the same three checks plus a fourth:
  selector coverage against a `TargetRef` (`Host` or `HostSoftwareItem`). Host-tag membership for `Selector::Tags`
  grants is resolved **at decision time**, not stored on the grant or cached — `load_host_tags` queries live
  `host_tag_assignment` rows through `TenantDb`, so a tag added to or removed from a host takes effect on the very
  next decision. The lookup only runs when a matching grant actually carries `Selector::Tags`; every other
  selector shape decides purely from ids already in memory. Fail-closed: a DB error during tag resolution
  propagates as `Err`, never silently degrades to an empty tag set. Callers MUST load `target` through `TenantDb`
  (or an equivalently tenant-scoped path) — only the `Tags` axis is itself tenant-scoped inside the engine; every
  other axis (`Hosts`/`Software`/`Items`, and `All`) trusts the caller's tenant scoping.

All four checks (dynamic registry, grant match, scope, and — for `authorize_target` — selector coverage) must
pass for `Allow`.

Resolved per-principal authority is cached (`moka::sync::Cache`, keyed by `(tenant_id, user_id)`), bounded by
entry count, with a first-party read-time staleness check against a 60-second TTL backstop rather than relying on
the cache library's own expiry. Grant/role mutations invalidate the cache locally and publish
`ControllerMessage::AccessInvalidated` for other controller instances to apply; the TTL backstop exists to bound
staleness if an invalidation event is ever lost. When the engine or its backing DB is unavailable, callers must
fail closed — HTTP 500, never a silent permit. Credentials with no scope concept (pre-M3 sessions) pass the scope
term vacuously, preserving pre-scope behavior.

### Enforcement surfaces

Most routes declare their action requirement via `action_extractor!`-generated types
(`crates/ui/web-api/src/middleware/action.rs`), which resolve through `AccessEngine` and short-circuit with `401`
when no principal is present, `403` on an engine deny (a fixed, generic denial body — no grant/selector detail),
or `500` when engine authority is unavailable. `authorize_any` OR-gates a request across several alternative
actions for endpoints that accept any one of them.

Five sites gate on the engine directly instead of through a generated extractor: plugin type settings'
`can_view_type_settings`, the plugin configs CRUD handlers, the `system_services.rs` batch handler, the
`services/batch.rs` batch handler, and `visibility.rs::is_plugin_visible_to_user` (a visibility predicate, not a
request-denying gate). The interactive-update WebSocket route checks the `updates:trigger` action before calling
`on_upgrade()`, since `require_auth` cannot run on a route whose token arrives as a query parameter rather than a
header. MCP authorization runs on the same engine: a connection-level `mcp:use` gate, plus per-tool actions
declared in each tool's `ToolAuth` and enforced by `require_tool_auth()`.

Converted routes declare a native OpenAPI `security(...)` requirement built from the catalog-generated scope
dictionary, instead of a vendor extension; dynamic surface wrappers instead carry `x-action-dynamic: true`
alongside an authenticated-only security form. `ci/verify_action_security_declarations.py` gates that every
route's declared security matches its actual enforcement.

### Runtime-valued actions

Surface `required_action` values are not known at compile time — they are supplied by the provider (plugin or
service) as **registration data**. On the wire the field is still a plain string
(`#[serde(alias = "required_permission")]` retained for backward compatibility with older wire payloads), but it
is parsed exactly once, at registration admission, to a typed `Action`
(`crates/ui/surface-proxy/src/registry.rs`). An unparseable value rejects the whole registration rather than
admitting a partially-typed surface. From then on, enforcement runs the parsed `Action` through `AccessEngine`
before dispatch, for both plugin- and service-backed surfaces alike.

### Lockout prevention

The lockout guard is re-targeted to the `access:manage` (tenant plane) and `system.access:manage` (global plane)
actions. `check_lockout` and `begin_guarded` (`crates/shared/db/src/access_grants.rs`) evaluate a pre-state vs.
simulated post-state comparison of covering holders, in memory, inside the guard's own serialized transaction —
sentinel-row locking on the default tenant's row serializes concurrent guarded mutations. The guard never calls
`AccessEngine` itself (its cache and pooled-connection reads would escape the transaction and under-count
holders), and it is skipped entirely for authority-adding mutations, which cannot shrink coverage under the
allow-only grant model. Callers must treat any non-`Permitted` verdict as an obligation not to write the guarded
mutation. A denial surfaces as HTTP 409 with reason code `lockout_access_manage` or `lockout_system_access` — see
[Access Management API](../api/access-management.md#lockout-409-semantics).

### Catalog introspection

`GET /api/v1/access/catalog` serves the authorization vocabulary as data — built-in and live dynamic actions with
their descriptions and selector support, the code-defined role bundles, and the available scope presets — so
clients never need a hardcoded copy. Canonical reference: [Access Management API](../api/access-management.md).

### Deny events

`deny_event_worthy()` (`crates/shared/types/src/access/mod.rs`) is the single shared definition of which denials
are audit-worthy, used identically by the web-api and MCP enforcement paths: denying a `system.`-plane action,
`commands:manage`, `access:manage`, or `mcp:use` emits an `access.denied` audit Event. Every other deny produces a
debug trace plus an increment of the `uptrakit_access_denies_total` counter, nothing more.

## System Service Credential Guard

Four capabilities grant access to sensitive infrastructure secrets:

| Capability          | Secret delivered                    |
| ------------------- | ----------------------------------- |
| `database_access`   | Database connection URL             |
| `nats_access`       | NATS server URL                     |
| `master_key_access` | Master AES-256 encryption key (hex) |
| `ca_management`     | Permission to request CA rotation   |

These credentials are delivered via `ServiceCredentials` after mTLS authentication and are never
published to NATS. Because they provide privileged access to the entire infrastructure, a service
must declare the `system_service` capability to request any of them.

The guard runs at enrollment time, before any database write, in `do_enroll()`:

```rust
if requests_system_creds && !has_system_service {
    bail!(AgentRouteError::Forbidden(
        "system credentials (database_access, nats_access, master_key_access, \
         ca_management) require the system_service capability"
    ));
}
```

A service that includes any of the four credential capabilities without `system_service` receives an
`ErrorCode::EnrollmentFailed` response with a descriptive message. The guard does not apply to the
system enrollment path (`do_enroll_system_service`), which is only reached when `system_service` is
already present.

See [System Services Architecture](https://github.com/worried-networking/uptrakit/tree/main/docs/architecture/) for the full enrollment flow
and two-tier service model.

## WebSocket Enrollment Secret Lookup

Services connect to the controller WebSocket endpoint at `/api/v1/ws/service`. Three authentication
paths exist:

| Path         | When              | How                                                                                                                               |
| ------------ | ----------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| mTLS         | Post-enrollment   | Client certificate issued after CSR approval; identity is cryptographically tied to the certificate serial number and service ID. |
| Bearer token | Enrollment window | `Authorization: Bearer <enrollment_secret>` header; secret is SHA-256 hashed and compared against the database.                   |
| Anonymous    | Pre-enrollment    | No credentials; only `enroll` messages are accepted.                                                                              |

### Bearer token lookup and the `service_id` query parameter

During the enrollment window (between `save_enrollment()` completing and the CA issuing and
delivering the certificate), services authenticate via their enrollment secret. The controller
resolves the service by querying:

```sql
SELECT * FROM services
WHERE enrollment_secret_hash = $1
  AND deactivated_at IS NULL
  -- optionally:
  AND id = $2
```

As a defence-in-depth measure, the service appends its known `service_id` as a URL query parameter:

```text
wss://controller:3000/api/v1/ws/service?service_id=<uuid>
```

When `service_id` is present, the DB query is narrowed to that specific service. If the secret hash
matches a different service's row (a practically impossible collision for 256-bit random secrets, but
architecturally undesirable), the controller returns `InvalidSecret` — the same error as no match —
so the caller cannot tell whether a collision occurred.

**The `service_id` filter is enforced by the service-sdk.** `connect_ws()` in
`crates/shared/service-sdk/src/ws.rs` appends `?service_id=<uuid>` to the WebSocket URL whenever
the local identity file contains a service ID. During the first enrollment (no identity yet) the
parameter is omitted and the lookup falls back to hash-only matching.

**mTLS connections do not use the `service_id` parameter.** Their identity is embedded in the client
certificate; no bearer secret lookup is performed.

See also: [Wire Protocol](https://github.com/worried-networking/uptrakit/tree/main/docs/api/) for connection sequencing.

## OAuth 2.1 for MCP

Uptrakit ships a dual-auth model for MCP access: opaque `upk_*` API tokens for non-interactive
callers (CLI, CI) and OAuth 2.1 for browser-capable MCP clients (Claude Desktop, Cursor). Auth is
prefix-dispatched at the MCP Resource Server — a `Bearer upk_` prefix routes to opaque token
validation; a `Bearer eyJ` prefix routes to JWT validation.

The cross-rejection guarantee is enforced by audience claims. Dashboard JWTs (`aud: ["uptrakit"]`,
short-lived session tokens) are rejected by the MCP Resource Server's OAuth validator. OAuth JWTs
(`aud: ["<oauth.canonical_host>/mcp"]`) are rejected by the Dashboard JWT middleware — `aud`
mismatch by design, preventing session token reuse as OAuth bearer tokens and vice versa.

See also:

- [MCP OAuth Authorization Design](../superpowers/specs/2026-05-12-mcp-oauth-authorization-design.md)
- [ADR 0010 — MCP OAuth Authorization Server Placement](../adr/0010-mcp-oauth-authorization-server-placement.md)
- [OAuth MCP Security](oauth-mcp.md)
- [OAuth MCP Development Guide](../development/oauth-mcp.md)

## Content Security Policy

The admin UI's Content Security Policy (set in `frontend/src/app.html`) includes
`img-src 'self' https:`. This allows images from any HTTPS domain, which is required
to load OIDC provider logos configured by administrators.

**Accepted risk:** The admin who configures the OIDC provider logo URL is a trusted user.
Logo URLs are validated as HTTPS-only via `isValidLogoUrl()` in `frontend/src/lib/utils.ts`
before display. `referrerpolicy="no-referrer"` is applied to logo `<img>` elements,
preventing the Uptrakit URL from leaking to logo hosts via the `Referer` header.
