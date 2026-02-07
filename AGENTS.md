# AGENTS.md — AI Agent Guide for Uptrakit

This file provides structured context for AI coding agents working on the Uptrakit codebase. Read this first before making any changes.

## Project summary

Uptrakit is an agent-based update tracking toolkit for self-hosted Linux environments. It tracks installed software versions across remote hosts, checks for updates, and allows **manual, user-triggered** updates. It is **not** an auto-updater.

Key components:

- **Controller** (server): API, Web UI, scheduler, remote provider logic.
- **MQTT Service** (standalone binary): MQTT/Home Assistant integration with lease-based multi-instance tenant distribution.
- **Agents**: lightweight daemons on each managed host; outbound-only secure WebSocket to the controller; local version detection and update execution via sudo allowlists.
- **Providers**: pluggable modules that define how to detect installed versions, resolve latest versions, and perform updates.

For full project context, see [README.md](README.md). For contribution rules, see [CONTRIBUTING.md](CONTRIBUTING.md). For system design and technology choices, see [ARCHITECTURE.md](ARCHITECTURE.md). For security policy and cryptographic details, see [SECURITY.md](SECURITY.md). For the documentation catalogue, see [docs/README.md](docs/README.md).

## Codebase layout

```text
uptrakit/
├── Cargo.toml                          # Workspace root (resolver = "3", members = "crates/*/*")
├── crates/
│   ├── core/
│   │   ├── agent/                      # uptrakit-agent                         (bin)  — agent daemon
│   │   ├── controller/                 # uptrakit-controller                    (bin)  — central server
│   │   └── mqtt/                       # uptrakit-mqtt                          (bin)  — standalone MQTT service
│   ├── providers/
│   │   ├── core/                       # uptrakit-provider-core                 (lib)  — provider trait/abstractions
│   │   ├── docker-registry/            # uptrakit-provider-docker-registry      (lib)  — Docker/OCI Registry provider
│   │   ├── github/                     # uptrakit-provider-github               (lib)  — GitHub Releases provider
│   │   ├── proxmox-helper-scripts/     # uptrakit-provider-proxmox-helper-scripts (lib) — PVE helper-scripts provider
│   │   └── registry/                   # uptrakit-provider-registry             (lib)  — provider dispatch & validation
│   ├── shared/
│   │   ├── core/                       # uptrakit-core                          (lib)  — shared domain models
│   │   ├── db/                         # uptrakit-shared-db                     (lib)  — SeaORM entities & migrations
│   │   ├── directories/                # uptrakit-directories                   (lib)  — cross-platform directory management
│   │   ├── macros/                     # uptrakit-shared-macros                 (lib)  — shared declarative macros (impl_report_conversion!)
│   │   ├── web-api-types/              # uptrakit-web-api-types                 (lib)  — shared HTTP request/response types
│   │   ├── enrollment/                  # uptrakit-enrollment                    (lib)  — shared enrollment, TLS, CA bootstrap, CLI
│   │   └── wire/                       # uptrakit-internal-wire                 (lib)  — service<->controller wire protocol
│   └── ui/
│       ├── cli/                        # uptrakit-cli                           (bin)  — CLI interface
│       └── web-api/                    # uptrakit-web-api                       (lib)  — HTTP API
├── frontend/                           # SvelteKit SPA (Skeleton UI v4 + Tailwind CSS v4)
│   ├── src/
│   │   ├── lib/                        # Shared modules: api client, auth store, types
│   │   └── routes/                     # SvelteKit file-based routes
│   ├── package.json                    # npm scripts: build, check
│   ├── svelte.config.js                # SvelteKit config (static adapter)
│   └── vite.config.ts                  # Vite + Tailwind CSS v4 config (dev proxy → controller)
├── .github/
│   ├── workflows/ci.yml                # CI: fmt check, clippy, tests, reverse-proxy Docker tests, frontend check + build
│   └── dependabot.yml                  # Weekly Cargo + npm dependency updates
├── CONTRIBUTING.md
├── README.md
└── AGENTS.md                           # This file
```

All crates use **edition = "2024"**. Some specify `rust-version = "1.91"`.

## Quality gates (must pass before committing)

### Backend (Rust)

```sh
cargo fmt --all                                                      # Format
cargo check --workspace --no-default-features --features db-sqlite   # Lint with minimal features-set
cargo check --workspace --all-features                               # Lint
cargo clippy --workspace --all-targets --no-default-features --features db-sqlite -- -D warnings # Lint with Clippy over minimal features-set
cargo clippy --workspace --all-targets --all-features -- -D warnings # Lint with Clippy
cargo test --all-features                                            # Tests
cargo deny check                                                     # Validate new dependencies
# Docker integration tests (requires Docker, not part of normal CI gate):
# cargo test -p uptrakit-controller reverse_proxy -- --ignored
```

There shouldn't even be any warnings in the output of these commands.

### Frontend (SvelteKit)

```sh
cd frontend && npm install                                   # Install dependencies
cd frontend && npm run check                                 # Svelte/TypeScript type check
cd frontend && npm run build                                 # Production build
```

CI runs these same checks. A PR that fails any of them will not merge.

## Commit messages

**Conventional Commits are required.** Format:

```gitmessage
<type>(optional-scope): <description>
```

Types: `feat`, `fix`, `docs`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`.

Scopes typically match crate or component names: `agent`, `controller`, `provider-core`, `provider-github`, `provider-phs`, `mqtt`, `web`, `cli`, `wire`, `core`.

Breaking changes: add `!` after type/scope, e.g. `feat(api)!: change ws handshake payload`.

Multi-scope commits should omit scope in the first line of the commit message, but provide all the details in the body.
Make small, granular commits, focused on a single thing.

Examples:

- `feat(agent): add helper-scripts autodiscovery`
- `fix(controller): handle websocket reconnect backoff`
- `refactor(provider-github): simplify release tag normalisation`

## PKI & CA rotation

The controller manages a self-signed internal CA for mTLS agent authentication.

### Key lifetimes

| Asset | Default lifetime | Renewal/rotation window |
| --- | --- | --- |
| CA certificate | 5 years | 6 months before expiry |
| Server HTTPS cert | 90 days | 30 days before expiry |
| Agent client cert | 365 days | Configured via `renewal_window_hours` setting |

### CA rotation flow

1. Background task checks every 24 hours whether the active CA enters the 6-month rotation window. Can also be triggered on demand via `POST /api/v1/settings/rotate-ca`.
2. On rotation: current CA files move to `ca-previous.{crt,key}`, a new CA is generated as `ca.{crt,key}`.
3. Both CAs form a trust bundle (`bundle_pem`). The controller trusts client certs signed by either CA.
4. CRLs are partitioned: each CA signs a CRL only for certificates it issued (tracked via `ca_fingerprint` column in `service_certificates`).
5. Connected agents receive a `CaBundleUpdated` WebSocket message with the new bundle PEM, followed by `RequestCertRenewal` to trigger immediate cert renewal.
6. Agents that were offline detect staleness via `ca_bundle_hash` in `ServiceSettings` and fetch the updated bundle over HTTPS.
7. New agent certs are always signed by the active CA.

### PKI address and AIA/CDP extensions

When `--pki-addr` is configured, the controller embeds AIA (Authority Information Access) and CDP (CRL Distribution Points) extensions in both CA and agent certificates:

| Extension | URL |
| --- | --- |
| AIA OCSP | `{pki_addr}/api/v1/pki/ocsp` |
| AIA CA Issuers | `{pki_addr}/api/v1/pki/ca.crt` |
| CDP CRL | `{pki_addr}/api/v1/pki/ca.crl` |

`--pki-addr` accepts both `http://` and `https://` URLs. **`http://` is recommended** because Nginx only supports `http://` OCSP responder URLs — `https://` AIA URLs are silently ignored by Nginx's `ssl_ocsp` directive. When the PKI address uses `http://`, the `--pki-http` flag controls how plain HTTP serving is handled:

| `--pki-http` value | Behaviour |
| --- | --- |
| `listener` | The controller starts a plain HTTP listener on the port from `--pki-addr`, serving only PKI routes (`/healthz`, `/api/v1/pki/ca.crt`, `/api/v1/pki/ca.crl`, `/api/v1/pki/ocsp`). Required for Nginx `ssl_ocsp_responder` which only supports `http://` OCSP responder URLs. |
| `external` | PKI HTTP is handled by an external component (e.g. reverse proxy). Suppresses the warning about `http://` scheme without `--pki-http`. |
| (not set) | If `--pki-addr` uses `http://`, the controller logs a warning. |

At startup, the controller validates the existing CA certificate's embedded URLs against the reconciled `pki_addr`:
- PKI address set and matching CA extensions: OK
- PKI address set but different from CA extensions: **startup failure** (suggests updating the setting or deleting CA files)
- PKI address set but CA has no extensions: **startup failure** (suggests deleting CA files to regenerate with extensions)
- PKI address not set but CA has extensions: **startup failure** (suggests providing `--pki-addr` or deleting CA files)
- Neither set: OK

Changing the PKI address requires CA rotation (the URLs are embedded in the CA certificate). See the [reverse proxy guide](docs/reverse-proxy/README.md) for the full flow.

### OCSP responder

The controller provides an OCSP responder at `/api/v1/pki/ocsp` (both POST and GET). It accepts standard RFC 6960 OCSP requests and returns signed OCSP responses:
- **good**: certificate is valid and not revoked
- **revoked**: certificate has been revoked (includes revocation time and reason)
- **unknown**: certificate serial not found

The responder supports both SHA-1 and SHA-256 hash algorithms in requests per RFC 6960. Nginx/OpenSSL always uses SHA-1 (`1.3.14.3.2.26`) for OCSP requests. `ResponderID::ByKey` uses SHA-1 as required by RFC 6960 Section 2.3. Responses are signed with the active CA's private key using ECDSA P-256 SHA-256.

Only Nginx natively supports OCSP verification of client certificates (via `ssl_ocsp` directive, since v1.19.0). HAProxy, Envoy, Traefik, and Caddy do not.

### External CA

Pass `--ca-cert` and `--ca-key` to disable managed CA and rotation. The controller uses the provided CA as-is.

### Server cert auto-renewal

When the server HTTPS certificate (also CA-signed) approaches expiry, a background task generates a new one and hot-reloads the TLS listener. Admins can also trigger renewal manually via `POST /api/v1/settings/renew-server-certificate`.

### Server cert SAN sanity checks

At startup, the controller validates that `--san` values match the existing managed server certificate's SANs:

1. **`--san` is incompatible with `--tls-cert`/`--tls-key`**: the controller rejects this combination because SANs are only configurable for controller-managed certificates.
2. **SAN mismatch + same CA**: if `--san` values are not present in the existing cert's SANs and the cert was signed by the currently active CA, the cert is silently regenerated.
3. **SAN mismatch + different CA**: if the cert needing SAN regeneration was signed by a different CA (e.g. after CA rotation), the controller fails with a multi-step fix message guiding the admin through manual certificate renewal.

Shared PKI utility functions (`SanCollection`, `collect_sans`, `cert_signed_by_ca`) live in `crates/ui/web-api/src/pki_utils.rs` and are used by both the web API handlers and the controller startup logic.

### CaSnapshot sharing

Runtime CA state is shared across async tasks via a `tokio::sync::watch` channel carrying a `CaSnapshot` struct. The cert signer, CRL manager, API handlers, and background tasks all read from this channel.

## Architecture rules and invariants

These are non-negotiable design constraints. Do not violate them.

1. **Updates are never automatic.** The scheduler triggers version *checks* only. Update execution requires explicit user action (via UI, CLI, or MQTT/Home Assistant).
2. **Agents initiate outbound-only connections.** Agents connect to the controller via secure WebSocket (`/api/v1/ws/service`). They never listen on any port or accept inbound connections.
3. **Agents run unprivileged.** They run as a dedicated user (e.g. `uptrakit`). Only specific update commands are granted `NOPASSWD` sudo access.
4. **Provider split.** Remote (upstream version resolution) logic runs on the controller. Local (installed version detection + update execution) logic runs on the agent. Keep this boundary clear.
5. **No shell injection.** Any path that constructs or executes shell commands must validate inputs. Custom scripts are treated as untrusted input.
6. **No secrets in logs.** Never log tokens, passwords, API keys, or other credentials.
7. **Logging goes to journald or stdout.** No internal log storage. Full command output is not captured internally — only high-level summaries are retained for display.
8. **No overlapping update actions per host.** The scheduler must ensure that two update operations for the same host never run concurrently.
9. **No raw SQL.** Use the structures and methods provided by Sea ORM eveywhere.
10. **Cover new logic with tests.** Cover success and failure paths.
11. **Document everything.**  Any code change must be properly documented either in the code, or in the separate documentation. Any changes to the agent-controller wire protocol must be documented in `crates/shared/wire/asyncapi.yaml`.
12. **Do not add any `#[allow()]`** without explicit confirmation. There are currently no approved exceptions in the codebase; all previously allowed lints have been resolved via parameter structs, `FromStr` implementations, or dead code removal.
13. **Do not use `unsafe`, `unwrap` or `panic!`.** Always prefer safe and graceful solutions. See the "Error handling" section in [CONTRIBUTING.md](CONTRIBUTING.md) for approved patterns (match with fallback, serialization helpers). **Approved exceptions**: `Mutex::lock().unwrap()`, `RwLock::read().unwrap()`, and `RwLock::write().unwrap()` are safe because `panic = "abort"` in the release profile makes lock poisoning impossible.

## Directory management

All binaries (controller, agent, MQTT service) use the `uptrakit-directories` crate for cross-platform directory resolution. The crate uses the `directories` crate (`ProjectDirs`) to follow platform conventions:

| Platform | Config directory | State directory |
| --- | --- | --- |
| Linux | `~/.config/{app}/` (XDG) | `~/.local/state/{app}/` (XDG) |
| macOS | `~/Library/Application Support/io.uptrakit.{app}/` | `~/Library/Application Support/io.uptrakit.{app}/` |
| Windows | `{FOLDERID_RoamingAppData}\uptrakit\{app}\` | `{FOLDERID_LocalAppData}\uptrakit\{app}\` |

Where `{app}` is one of: `controller`, `agent`, `mqtt`.

### Config vs state separation

| Directory | Contents | Characteristics |
| --- | --- | --- |
| **Config** | Rarely-changing, persistent configuration | CA certificates, user-provided TLS certs |
| **State** | Runtime state that may change frequently | SQLite DB, JWT keys, service identity, private keys, issued certificates |

**Controller:**
- Config: PKI files (CA certificate/key, server TLS certificate/key)
- State: SQLite database, JWT signing key

**Agent/MQTT Service:**
- Config: Controller's CA certificate
- State: Service ID, private key, issued certificate

### CLI directory flags

All binaries support `--config-dir` and `--state-dir` CLI flags (and corresponding `UPTRAKIT_CONFIG_DIR` / `UPTRAKIT_STATE_DIR` environment variables) to override the platform defaults. Both support `~` expansion for home directory paths.

### Secure permissions

All created files and directories use secure permissions:
- **Directories**: 0o700 (owner read/write/execute only)
- **Files**: 0o600 (owner read/write only)

The `uptrakit-directories` crate provides helper functions:
- `create_secure_dir(path)` — creates directory with 0o700 permissions
- `write_secure_file(path, data)` — writes file with 0o600 permissions
- `AppDirs::resolve(app_kind, config_override, state_override)` — resolves directories for an application
- `AppDirs::ensure_dirs()` — creates both directories with secure permissions

### Key files

| File | Purpose |
| --- | --- |
| `crates/shared/directories/src/lib.rs` | Cross-platform directory resolution and secure file/directory operations |

## CLI output formatting

The CLI supports three output formats via the global `--output` / `-o` flag:

| Format | Flag value | Behaviour |
| --- | --- | --- |
| Human (default) | `human` | Columnar / free-text output identical to pre-flag behaviour |
| JSON | `json` | Compact single-line JSON, suitable for `jq` piping |
| YAML | `yaml` | YAML output via `serde_yml` |

### Implementation

- `OutputFormat` enum in `crates/ui/cli/src/output.rs` — derives `clap::ValueEnum` and `Default` (`Human`).
- `print_output<T: Serialize>(format, human_text, value)` — for typed commands (`auth status`, `auth token *`).
- `print_value(format, &serde_json::Value)` — for the `api` command which works with raw JSON values.
- Each structured command defines a serializable response struct (e.g. `AuthStatusOutput`, `TokenCreateOutput`, `TokenListOutput`, `TokenRevokeOutput`) in `commands/auth.rs`.
- `auth login` is interactive and does not support `--output`.

### Example usage

```sh
uptrakit-cli auth status                     # human-readable (default)
uptrakit-cli auth status -o json             # compact JSON
uptrakit-cli auth token list --output yaml   # YAML
uptrakit-cli api GET /api/v1/auth/me -o json # compact JSON for raw API calls
```

## Device authorization flow (CLI login)

The CLI uses an RFC 8628-style device authorization flow instead of password-based login. This allows the CLI to authenticate even when password auth is disabled (OIDC-only environments).

### Flow

1. CLI calls `POST /api/v1/auth/device` with an optional `client_name`. Returns `device_code`, `user_code`, `verification_url`, `expires_in` (600s), and `interval` (5s).
2. CLI opens `verification_url` in the user's browser and displays the `user_code`.
3. User logs in via the browser (password or OIDC) and approves the device code at `/device?code=XXXX-XXXX`.
4. CLI polls `POST /api/v1/auth/device/poll` with the `device_code` every `interval` seconds.
5. On approval, the poll response contains an API token. The CLI stores it locally.

### Endpoints

| Endpoint | Auth | Purpose |
| --- | --- | --- |
| `POST /api/v1/auth/device` | Public | Start device flow, get device code + user code |
| `POST /api/v1/auth/device/poll` | Public | Poll for authorization status |
| `POST /api/v1/auth/device/approve` | Bearer (JWT or API token) | Approve a device code (browser-side) |

### Security

- **Device code**: 32-byte crypto random (base64url), unguessable.
- **User code**: 8 uppercase consonants from a 20-char alphabet (avoids vowels to prevent offensive words), ~34.5 bits entropy, formatted `XXXX-XXXX`.
- **Rate limiting**: 429 returned if polling faster than the 5-second interval.
- **One-time use**: consuming an authorized flow removes it atomically; a second poll gets 404.
- **10-minute expiry**: flows auto-expire; cleanup runs every 5 minutes alongside OIDC state cleanup.
- **Database-backed store**: all pending device flow state is persisted to the `pending_device_flows` table (shared with OIDC flow, account link, token exchange, and OIDC registration stores). Survives controller restarts and supports HA multi-instance deployments. Only the resulting API token is persisted to the `api_tokens` table.

## Permissions model

Authorization uses a typed `Permission` enum (defined in `crates/shared/web-api-types/src/permissions.rs`, re-exported from `crates/ui/web-api/src/auth/permissions.rs`) rather than raw role-name strings. The enum variants are:

| Permission | Serialized name | Purpose |
| --- | --- | --- |
| `ViewSettings` | `view_settings` | Read settings, OIDC providers, auth config |
| `ManageSettings` | `manage_settings` | Modify settings, OIDC providers, auth config |
| `ViewAgents` | `view_agents` | List agents |
| `ManageAgents` | `manage_agents` | Approve, reject, delete, merge agents; manage enrollment tokens |
| `ManageGlobalSettings` | `manage_global_settings` | View and modify global settings (network, CA, TLS, system alerts) |

### Roles

| Role | Permissions |
| --- | --- |
| `owner` | All five (`view_settings`, `manage_settings`, `view_agents`, `manage_agents`, `manage_global_settings`) |
| `admin` | `view_settings`, `manage_settings`, `view_agents`, `manage_agents` |
| `user` | `view_agents` only |

The first registered user gets the `owner` role — whether registered via password or OIDC. Subsequent users (password or OIDC auto-created) get the `user` role by default. OIDC role mapping can override this.

### How it works

1. `get_user_permissions()` (`routes/auth.rs`) resolves a user's permissions: user → user_roles → role_permissions → permissions table.
2. The resolved `Vec<Permission>` is embedded in the JWT access token (`permissions` claim) and returned in `UserResponse.permissions`.
3. The `require_auth` middleware injects `AuthenticatedUser` with the `permissions` field decoded from the JWT.
4. Route handlers call `user.has_permission(Permission::...)` — no DB round-trip needed.
5. The frontend receives permissions as `string[]` (e.g. `["view_settings", "manage_agents"]`) and uses the `Permission` TypeScript enum for checks.

### Adding a new permission

1. Add a variant to the `Permission` enum in `crates/shared/web-api-types/src/permissions.rs` (with `as_str` / `parse` arms).
2. Write a DB migration to insert it into the `permissions` table and assign it to the appropriate roles.
3. Add the check in the relevant route handler(s).
4. Add the variant to the `Permission` TypeScript enum in `frontend/src/lib/types.ts`.

## Multi-tenancy

The codebase supports multi-tenancy at the database and API levels. Currently only **single-tenant mode** is active — multi-tenant mode is planned for a future release.

### Tenants table

The `tenants` table stores tenant records. A seeded **default tenant** (with `is_default = true`) is created by the initial migration. All data in single-tenant mode is associated with this default tenant.

| Column | Type | Notes |
| --- | --- | --- |
| `id` | UUID PK | `Uuid::now_v7()` |
| `name` | String | Human-readable name |
| `slug` | String (unique) | URL-safe identifier |
| `is_default` | Bool | Exactly one row has `true` |
| `created_at` | Timestamp | |
| `updated_at` | Timestamp | |
| `deactivated_at` | Timestamp? | Soft-delete |

### Tenant-scoped tables

The following tables have a `tenant_id UUID NOT NULL` column with a FK to `tenants(id)` ON DELETE RESTRICT:

| Table | Unique constraint change |
| --- | --- |
| `services` | — (index on `tenant_id`) |
| `hosts` | `machine_id` unique → `(tenant_id, machine_id)` |
| `provider_configs` | — (index on `tenant_id`) |
| `software_items` | `(provider_config_id, package_identifier)` → `(tenant_id, provider_config_id, package_identifier)` |
| `oidc_providers` | `slug` unique → `(tenant_id, slug)` |
| `user_roles` | PK `(user_id, role_id)` → `(tenant_id, user_id, role_id)` |
| `settings` | PK `(key)` → `(tenant_id, key)` |
| `mqtt_clients` | Non-unique index on `tenant_id` (multiple clients per tenant, limit controlled by `MqttMaxClientsPerTenant` global setting) |

### Tables NOT changed (remain global)

`users`, `roles`, `permissions`, `role_permissions`, `sessions`, `api_tokens`, `pending_*` tables, `host_software_items`, `available_versions`. Note: `service_certificates` and `service_hosts` are tenant-scoped through the `services` table FK.

### TenantContext extractor

Route handlers that operate on tenant-scoped data accept a `TenantContext` extractor (`crates/ui/web-api/src/middleware/tenant_context.rs`). It implements `FromRequestParts<Arc<AppState>>`:

1. Reads the `X-Tenant-Id` HTTP header.
2. If present and non-empty: parses as UUID, uses it as the tenant.
3. If absent: falls back to `state.default_tenant_id`.

In single-tenant mode, the header is optional — all requests default to the default tenant.

### AppState.default_tenant_id

`AppState` has a `default_tenant_id: uuid::Uuid` field, loaded at startup by querying the seeded default tenant from the DB. It is used:

- As the fallback in `TenantContext` when no header is provided.
- For global settings (via `resolve_tenant_for_key()`).
- In middleware and auth flows that don't have a per-request tenant context.

### Global vs tenant-scoped settings

`SettingKey::is_global()` returns `true` for settings that apply system-wide (not per-tenant). Global settings are always stored under `default_tenant_id`:

- `TrustedProxies`, `RealIpHeader`, `ExtraSans`, `HttpsAddr`
- `ForwardedClientCertInfoHeader`, `ForwardedClientCertPemHeader`
- `MultiTenancyEnabled`
- `MqttMaxClientsPerTenant`

The helper `resolve_tenant_for_key(key, tenant_id, default_tenant_id)` in `settings_store.rs` returns the correct tenant ID based on whether the key is global.

### Future multi-tenancy work

- Tenant management API (CRUD for tenants)
- Multi-tenant JWT (per-tenant permissions in token)
- Tenant-aware MQTT (per-tenant broker config or topic prefix)
- Tenant switching UI
- API token scoping per tenant

## DB-managed settings

Most CLI arguments are reconciled with DB-persisted values at startup. The reconciliation module (`crates/core/controller/src/reconcile.rs`) implements a generic 5-case priority logic. Settings are stored in the `setting` DB entity as JSON values.

### Settings reference

| CLI flag | DB key | Default | Runtime-changeable |
| --- | --- | --- | --- |
| `--trusted-proxy` | `network.trusted_proxies` | `[]` | Yes |
| `--real-ip-header` | `network.real_ip_header` | `X-Forwarded-For` | Yes |
| `--san` | `network.extra_sans` | `[]` | Yes |
| `--forwarded-client-cert-info-header` | `network.forwarded_client_cert_info_header` | `null` | Yes |
| `--forwarded-client-cert-pem-header` | `network.forwarded_client_cert_pem_header` | `null` | Yes |
| `--pki-addr` | `network.pki_addr` | `null` | Yes (requires CA rotation) |
| `--https-addr` | `network.https_addr` | `[::]:8443` | No (restart) |

**Not DB-managed** (bootstrap/infrastructure): `--data-dir`, `--db-url`, `--tls-cert`, `--tls-key`, `--ca-cert`, `--ca-key`, `--static-dir`, `--reuseport`, `--takeover-from`, `--shutdown-timeout-secs`.

### OIDC provider bootstrap

The controller supports bootstrapping an OIDC provider at startup via CLI flags. This solves the chicken-and-egg problem where configuring OIDC requires ManageSettings permission, but the first user needs to log in via OIDC.

| CLI flag | Default | Description |
| --- | --- | --- |
| `--oidc-issuer-url` | — | OIDC issuer URL; triggers bootstrap when set |
| `--oidc-client-id` | — | Required with `--oidc-issuer-url` |
| `--oidc-client-secret` | — | Required with `--oidc-issuer-url` |
| `--oidc-provider-name` | `SSO` | Display name for the provider |
| `--oidc-provider-slug` | `sso` | URL-safe slug (used for uniqueness check) |
| `--oidc-scopes` | `openid email profile groups` | Space-separated scopes |

**Bootstrap behavior:**
1. If no provider with matching `(slug, tenant_id)` exists: INSERT with `auto_create_users=true`
2. If a match exists and `--force-settings-override` is set: UPDATE issuer/client_id/client_secret
3. If a match exists without force: skip with info log

The client secret is never logged. The bootstrapped provider is created with `is_active=true` and `auto_create_users=true`.

When the first user logs in via OIDC (bootstrapped or otherwise), they are automatically promoted to the `owner` role and initial setup is completed (registration mode set to closed).

### OIDC registration token enforcement

When registration mode is `Invite`, OIDC-based user creation can require a registration token:

- **First user (initial setup)**: Always requires the registration token printed in controller startup logs.
- **Subsequent users**: Requires the token only if `require_token_for_oidc` is enabled in registration settings.

The `needs_token_for_oidc(is_first_user: bool) -> bool` helper on `RegistrationSettings` encapsulates this logic: returns `true` when mode is `Invite` AND (`is_first_user` OR `require_token_for_oidc`).

**Deferred-action flow** (follows the `AccountLinkStore` pattern):

1. OIDC callback detects a new user would be created AND `needs_token_for_oidc()` returns `true`.
2. OIDC claims are stored in `pending_oidc_registrations` via `OidcRegistrationStore` (10-minute TTL).
3. User is redirected to `/login?registration_token_required=true&registration_code={code}`.
4. Frontend shows a token input form.
5. User submits the token to `POST /api/v1/auth/oidc/complete-registration`.
6. Backend peeks at the pending registration via `get()` (non-destructive), validates the token. If the token is invalid, the entry remains in the store so the user can retry with a correct token.
7. On successful validation, the entry is atomically consumed via `take()`, the user is created, roles assigned, and session + JWT issued.

**New endpoint:**

| Endpoint | Auth | Purpose |
| --- | --- | --- |
| `POST /api/v1/auth/oidc/complete-registration` | Public | Complete OIDC registration with registration token |

**New setting:**

| DB key | Type | Default | Description |
| --- | --- | --- | --- |
| `registration.require_token_for_oidc` | bool | `false` | When `true` and mode is `Invite`, require registration token for OIDC user creation |

**New store:** `OidcRegistrationStore` in `crates/ui/web-api/src/auth/oidc_state.rs` — follows the same atomic-delete pattern as `AccountLinkStore`. Methods: `insert()`, `get()` (non-destructive read for pre-validation), `take()` (atomic consume), `cleanup_expired()`.

**New entity:** `pending_oidc_registration` in `crates/shared/db/src/entity/pending_oidc_registration.rs` — stores deferred OIDC registration claims (registration_code PK, provider_id, oidc_subject, email, names, mapped_roles JSON, expires_at).

### Zero-downtime graceful restart

The controller supports HAProxy-style zero-downtime restarts using `SO_REUSEPORT`. This allows a new controller process to start accepting connections while the old process drains existing ones.

**CLI flags:**

| Flag | Default | Description |
| --- | --- | --- |
| `--reuseport` | `false` | Enable `SO_REUSEPORT` socket option (required on both processes) |
| `--takeover-from <PID>` | — | PID of old process to take over from; sends SIGUSR1 to initiate graceful shutdown |
| `--shutdown-timeout-secs` | `30` | Graceful shutdown timeout (how long to drain connections) |

**Restart sequence:**

1. Old process is running with `--reuseport`
2. New process starts with `--reuseport --takeover-from <OLD_PID>`
3. New process binds to the same port (SO_REUSEPORT allows this)
4. New process starts accepting connections immediately
5. New process sends SIGUSR1 to old process
6. Old process stops accepting new connections
7. Old process scatters `ServerRestarting` notifications to agents over 5 seconds (avoids thundering herd)
8. Old process cancels background tasks and waits for drain timeout
9. Old process exits cleanly
10. New process serves all traffic

**Signal handling:**

| Signal | Action |
| --- | --- |
| SIGTERM | Initiate graceful shutdown |
| SIGINT | Initiate graceful shutdown |
| SIGUSR1 | Initiate graceful shutdown (used for takeover) |

**Wire protocol:** The `ServerRestarting` message (`ControllerMessage::ServerRestarting(ServerRestartingPayload)`) notifies agents that the controller is restarting. Agents log the message and allow the connection to close naturally; their existing reconnect logic handles the rest.

**Platform support:** `SO_REUSEPORT` is available on Linux, macOS, FreeBSD, and OpenBSD. Not available on Windows.

### Bulk loading and known-keys registry

At startup, `Settings::load(db, tenant_id)` issues a single `SELECT * FROM settings WHERE tenant_id = ?` via `load_all_settings(db, tenant_id)` and distributes the resulting `RawSettings` (`HashMap<String, serde_json::Value>`) to all sub-loaders. This replaces the previous pattern of one query per key.

After the bulk load, `warn_unrecognised_keys()` logs a warning for any DB key not recognised by `SettingKey::from_db_key()`. The `SettingKey` enum (defined in `crates/ui/web-api/src/setting_key.rs`) is the single source of truth for all known setting keys. In tests, `SettingKey::iter()` (via `strum::EnumIter`) provides iteration over every variant.

`Settings::load()` returns `(Self, RawSettings, Option<String>)` so the controller passes the same map to reconciliation without re-reading.

The `RawSettingsExt` trait (defined in `settings_store.rs`) provides a `get_setting(SettingKey) -> Option<&Value>` method for typed lookups on `RawSettings`, replacing raw `raw.get("string.key")` calls throughout the codebase.

### Reconciliation logic

`reconcile_setting()` (`crates/core/controller/src/reconcile.rs`) accepts a `SettingKey` and a `&RawSettings` map, looking up the key via `key.as_str()` — no per-key DB reads. It still needs the `DatabaseConnection` for upserts.

For each DB-managed setting at startup:

1. DB has value + CLI provided + differs + `--force-settings-override` → use CLI, update DB
2. DB has value + CLI provided + differs + no force → use DB, log warning
3. DB has value + (CLI absent or same) → use DB
4. No DB value + CLI provided → use CLI, save to DB
5. No DB value + CLI absent → use hardcoded default, save to DB

### In-memory settings

The `Settings` struct (`crates/ui/web-api/src/settings.rs`) holds `NetworkSettings` behind a `RwLock`. Runtime-changeable fields (proxies, header, SANs) are updated in-memory immediately when changed via the API. Restart-required fields (addresses) are saved to DB only.

### Settings API endpoints

| Endpoint | Permission | Purpose |
| --- | --- | --- |
| `GET /api/v1/settings/network` | ManageGlobalSettings | Read network settings |
| `PUT /api/v1/settings/network` | ManageGlobalSettings | Update network settings (includes `pki_addr`) |
| `GET /api/v1/settings/mqtt` | ViewSettings | List all MQTT client configurations |
| `POST /api/v1/settings/mqtt` | ManageSettings | Create MQTT client configuration (checks per-tenant limit) |
| `GET /api/v1/settings/mqtt/limit` | ViewSettings | Get max MQTT clients per tenant limit |
| `PUT /api/v1/settings/mqtt/limit` | ManageGlobalSettings | Update max MQTT clients per tenant limit |
| `GET /api/v1/settings/mqtt/{id}` | ViewSettings | Get a specific MQTT client configuration |
| `PUT /api/v1/settings/mqtt/{id}` | ManageSettings | Update MQTT client configuration |
| `DELETE /api/v1/settings/mqtt/{id}` | ManageSettings | Delete MQTT client configuration |
| `POST /api/v1/settings/rotate-ca` | ManageGlobalSettings | Trigger immediate CA rotation |
| `POST /api/v1/settings/renew-server-certificate` | ManageGlobalSettings | Renew server TLS certificate |
| `GET /api/v1/system/alerts` | ManageGlobalSettings | Get system alerts (CA/cert status) |

### PKI API endpoints (unauthenticated)

| Endpoint | Purpose |
| --- | --- |
| `GET /api/v1/pki/ca.crt` | Download CA certificate bundle |
| `GET /api/v1/pki/ca.crl` | Download CRL (combined PEM) |
| `POST /api/v1/pki/ocsp` | OCSP responder (RFC 6960, `application/ocsp-request` body) |
| `GET /api/v1/pki/ocsp/{encoded}` | OCSP responder (base64-encoded request in URL path) |

MQTT password is never exposed in API responses; a `has_password: bool` field indicates whether one is set.

### MQTT client configuration

MQTT settings are stored in a dedicated `mqtt_clients` table (multiple rows per tenant, up to `MqttMaxClientsPerTenant` limit) rather than in the key-value `settings` table. The table stores connection components; the URL is a computed presentation field.

**Table schema (`mqtt_clients`):**

| Column | Type | Default | Notes |
| --- | --- | --- | --- |
| `id` | UUID PK | `Uuid::now_v7()` | |
| `tenant_id` | UUID FK → tenants | | Non-unique index (multiple clients per tenant) |
| `enabled` | bool | `true` | |
| `transport` | text | `tcp` | `tcp`, `tls` |
| `host` | text | | Broker hostname |
| `port` | integer | 1883 | |
| `client_id` | text | `uptrakit-controller` | |
| `username` | text? | | |
| `password` | text? | | |
| `topic_prefix` | text | `uptrakit` | |
| `created_at` | timestamptz | | |
| `updated_at` | timestamptz | | |

**Table schema (`mqtt_leases`):**

| Column | Type | Notes |
| --- | --- | --- |
| `id` | UUID PK | `Uuid::now_v7()` |
| `tenant_id` | UUID FK → tenants | |
| `mqtt_client_id` | UUID FK → mqtt_clients (ON DELETE CASCADE) | UNIQUE (one lease per MQTT client config) |
| `instance_id` | text | MQTT service instance identifier |
| `acquired_at` | timestamptz | |
| `last_heartbeat_at` | timestamptz | |

**Global setting (`MqttMaxClientsPerTenant`):**

| DB key | Type | Default | Description |
| --- | --- | --- | --- |
| `mqtt.max_clients_per_tenant` | u16 | 10 | Maximum number of MQTT client configurations per tenant |

**MQTT URL scheme:**

| URL example | Transport | Default port |
| --- | --- | --- |
| `mqtt://broker:1883` | tcp | 1883 |
| `mqtts://broker:8883` | tls | 8883 |

The API accepts either a `url` field (parsed into components) or individual `transport`/`host`/`port` fields. The response always includes the computed `url`.

### MQTT Service (standalone binary)

MQTT is handled by a separate `uptrakit-mqtt` binary (`crates/core/mqtt/`) that connects to the controller via mTLS WebSocket. The controller pushes tenant assignments and configuration updates; the MQTT service no longer has direct database access. Multiple instances can run simultaneously with centralized lease coordination managed by the controller.

**CLI flags (`uptrakit-mqtt`):**

The agent and MQTT service share a common set of CLI flags via `CommonServiceArgs` (defined in `uptrakit-enrollment::cli`). Service-specific flags are listed separately.

**Common flags (shared with agent via `CommonServiceArgs`):**

| Flag | Env var | Default | Description |
| --- | --- | --- | --- |
| `--url` | | (required) | Controller URL (e.g., `https://controller:8443`). Port defaults to 443. |
| `--tofu` | | `false` | Trust the controller's TLS certificate on first connection (TOFU). Conflicts with `--ca-cert` and `--pki-addr`. |
| `--ca-cert` | | | Path to a PEM-encoded CA certificate file |
| `--pki-addr` | | | Optional URL for PKI endpoints (CA certificate, OCSP). Supports `http://` and `https://`. |
| `--config-dir` | `UPTRAKIT_CONFIG_DIR` | platform-specific | Config directory for CA certificate |
| `--state-dir` | `UPTRAKIT_STATE_DIR` | platform-specific | State directory for service identity (service_id, keypair, certificate) |
| `--friendly-name` | | hostname | Human-readable display name |
| `--enrollment-token` | `UPTRAKIT_ENROLLMENT_TOKEN` | | Enrollment token for auto-approval |
| `--force-enroll` | | `false` | Force fresh enrollment, discarding existing state (preserves cached CA certificate) |

**MQTT-specific flags:**

| Flag | Default | Description |
| --- | --- | --- |
| `--max-tenants` | `0` | Max tenants per instance (0 = unlimited) |
| `--ping-interval` | `15` | Ping interval in seconds |

**Connection lifecycle (shared with agent via `uptrakit-enrollment`):**
1. **CA bootstrap**: Cached CA → `--ca-cert` file → `--pki-addr` fetch → `--tofu` TOFU → system trust (via `uptrakit_enrollment::ca::bootstrap_ca`)
2. **Enrollment**: Connect to `/api/v1/ws/service` anonymously, send `Enroll` with `service_type: "mqtt"`, hostname/friendly_name/enrollment_token (no `host_info`), receive `Enrolled` with service_id and enrollment_secret (saved to state dir) (via `uptrakit_enrollment::ws::run_enrollment`)
3. **Certificate issuance**: Reconnect with bearer token (enrollment_secret), send CSR, receive signed certificate (saved to state dir) (via `uptrakit_enrollment::ws::resume_enrollment`)
4. **Authenticated operation**: Reconnect with mTLS, send `Register`, receive `TenantAssignments`, run MQTT clients

**Instance identification:**
- Each instance generates a unique ID: `{hostname}-{uuid_v7_first_8_chars}`
- The controller manages leases centrally via the `mqtt_leases` table

**Main loop (event-driven, not polling):**
1. Receive `TenantAssignments` → start/update MQTT clients (keyed by `mqtt_client_id`)
2. Receive `TenantConfigUpdated` → hot-reload MQTT client configuration
3. Receive `TenantRevoked` → stop MQTT client (by `mqtt_client_id`)
4. Receive `CaBundleUpdated` → update local CA certificate
5. Receive `RequestCertRenewal` → trigger certificate renewal
6. Receive `ServerRestarting` → prepare for reconnect
7. Send `Ping` periodically (controller uses Ping receipt to update lease heartbeats)

**Wire protocol:**

Agents and MQTT services share a unified wire protocol (`ServiceMessage` / `ControllerMessage`) defined in `crates/shared/wire/src/lib.rs`. `ServiceMessage` contains both agent-specific variants (`ReportHostInfo`, `VersionCheckResults`, `UpdateStarted`, `UpdateOutput`, `UpdateResult`) and MQTT-specific variants (`Register`, `ReleaseTenants`), plus shared variants (`Enroll`, `RequestCertificate`, `RenewCertificate`, `Ping`, `Disconnecting`). `ControllerMessage` is fully shared. The `service_ws.rs` module is the single public WebSocket entry point; `agent_ws.rs` and `mqtt_ws.rs` are `pub(crate)` internal implementation modules.

**Enrollment and approval:**

MQTT services use the unified service entity:
- Single `services` table with `service_type` column (`Agent`/`Mqtt`)
- Single `service_certificates` table for all service types
- MQTT enrollment tokens are settings-based (key `mqtt_enrollment.token_hash` via `SettingKey::MqttEnrollmentTokenHash`), separate from agent enrollment tokens
- Approval via unified REST API: `POST /api/v1/services/{id}/approve` (permission: `ManageAgents`)
- If a valid enrollment token is provided, the service is auto-approved

**REST API endpoints (unified services API):**

| Method | Path | Permission | Description |
| --- | --- | --- | --- |
| GET | `/api/v1/services?type=mqtt&status=...` | ViewAgents | List MQTT services |
| POST | `/api/v1/services/{id}/approve` | ManageAgents | Approve a pending service |
| POST | `/api/v1/services/{id}/reject` | ManageAgents | Reject a pending service |
| DELETE | `/api/v1/services/{id}` | ManageAgents | Deactivate a service |
| POST | `/api/v1/services/enrollment-token?type=mqtt` | ManageAgents | Create MQTT enrollment token |
| DELETE | `/api/v1/services/enrollment-token?type=mqtt` | ManageAgents | Revoke MQTT enrollment token |
| GET | `/api/v1/services/enrollment-token/status?type=mqtt` | ManageAgents | Check MQTT enrollment token status |

**Key files:**

| File | Purpose |
| --- | --- |
| `crates/shared/web-api-types/src/mqtt_transport.rs` | `MqttTransport` enum (Tcp/Tls) |
| `crates/shared/web-api-types/src/mqtt_url.rs` | `MqttUrl` parsing and formatting |
| `crates/shared/web-api-types/src/settings_mqtt.rs` | API request/response types |
| `crates/shared/wire/src/lib.rs` | Unified wire protocol messages (`ServiceMessage` / `ControllerMessage`) |
| `crates/shared/db/src/entity/service.rs` | SeaORM entity for service identity (agents and MQTT) |
| `crates/shared/db/src/entity/service_certificate.rs` | SeaORM entity for service certificates |
| `crates/shared/db/src/entity/mqtt_client.rs` | SeaORM entity for MQTT config |
| `crates/shared/db/src/entity/mqtt_lease.rs` | SeaORM entity for leases (managed by controller) |
| `crates/shared/enrollment/` | `uptrakit-enrollment` crate: shared enrollment, identity, TLS, CA bootstrap, WebSocket protocol, and CLI args |
| `crates/ui/web-api/src/mqtt_client_store.rs` | MQTT client config CRUD store |
| `crates/ui/web-api/src/service_connections.rs` | `ServiceConnectionRegistry` for all connected services |
| `crates/ui/web-api/src/mqtt_lease_coordinator.rs` | Centralized lease management logic |
| `crates/ui/web-api/src/routes/settings_mqtt.rs` | MQTT config API route handlers |
| `crates/ui/web-api/src/routes/service_ws.rs` | Unified WebSocket entry point (`/api/v1/ws/service`) |
| `crates/ui/web-api/src/routes/mqtt_ws.rs` | Internal MQTT WebSocket handler (`pub(crate)`) |
| `crates/ui/web-api/src/routes/services.rs` | Unified service management REST endpoints |
| `crates/core/mqtt/src/main.rs` | Entry point, enrollment flow, authenticated main loop |
| `crates/core/mqtt/src/cli.rs` | CLI argument definitions |
| `crates/core/mqtt/src/controller_client.rs` | WebSocket client for controller communication |
| `crates/core/mqtt/src/tenant_manager.rs` | Per-MQTT-client lifecycle management (push-based, keyed by `mqtt_client_id`) |
| `crates/core/mqtt/src/mqtt_client.rs` | MQTT broker connection logic |
| `crates/core/mqtt/src/error.rs` | Application error types |

## Error handling

Use [`rootcause`](https://github.com/rootcause-rs/rootcause) for error propagation and [`thiserror`](https://github.com/dtolnay/thiserror) for error enum definition. Every module boundary must define its own error type following the patterns below.

### Import convention

Prefer importing the `rootcause::prelude` module. It provides `Report`, `markers`, `report!`, `bail!`, `ResultExt` (for `.context()` and `.context_to()`), and `IteratorExt`. When implementing `ReportConversion`, use the `impl_report_conversion!` macro from `uptrakit-shared-macros` (it handles the `ReportConversion` import internally):

```rust
use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_shared_macros::impl_report_conversion;
```

### Pattern 1: Define an error enum with a `Result<T>` alias

Each boundary (crate, module, or logical subsystem) defines its own error enum and a `Result` alias using `Report`:

```rust
use rootcause::prelude::*;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MyError {
    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, Report<MyError>>;
```

Real example: [`crates/ui/web-api/src/auth/error.rs`](crates/ui/web-api/src/auth/error.rs) (`AuthError`), [`crates/core/controller/src/db/error.rs`](crates/core/controller/src/db/error.rs) (`DbError`).

### Pattern 2: Implement `ReportConversion` for cross-boundary error conversion

When your module calls code that returns a different error type, implement `ReportConversion` so that `.context_to()` can convert automatically. Use the `impl_report_conversion!` macro from `uptrakit-shared-macros`:

```rust
use uptrakit_shared_macros::impl_report_conversion;

// Simple variant mapping (source error maps directly via #[from]):
impl_report_conversion!(sea_orm::DbErr => MyError::Database);

// Multiple conversions in one block:
impl_report_conversion! {
    sea_orm::DbErr => MyError::Database,
    std::io::Error => MyError::Io,
}

// Closure-based for errors that don't map directly (e.g. Box wrapping):
impl_report_conversion!(tungstenite::Error => MyError, |e| MyError::WebSocket(Box::new(e)));
```

Each macro invocation expands to a full `impl<T> ReportConversion<Source, markers::Mutable, T> for Target` block with the appropriate `context_transform` call.

### Pattern 3: Use `context_to()` in function bodies

Call `.context_to()?` on any `Result` whose error type has a `ReportConversion` impl for your boundary:

```rust
let user = users::Entity::find_by_id(id)
    .one(db)
    .await
    .context_to()?           // converts sea_orm::DbErr → MyError::Database
    .ok_or_else(|| report!(MyError::NotFound(format!("user {id}"))))?;
```

### Pattern 4: Use `report!()` to create reports directly

```rust
return Err(report!(MyError::NotFound("item not found".to_string())));
```

### Pattern 5: Adding parent context with `.context()`

Used when wrapping a low-level error with a higher-level description. Creates a parent node in the error tree:

```rust
db::connect(&db_config.url)
    .await
    .context(AppError::Database)?;
```

### Pattern 6: `context_transform()` with closures

For non-`#[from]` error conversions where you need to compute the target variant:

```rust
hostname::get()
    .context_transform(|e| PkiError::Hostname(e.to_string()))?;
```

Unlike `.context()` which creates a parent node, `.context_transform()` replaces the context type in place (single-node structure).

### Pattern 7: `map_err` with `report!()` for one-off conversions

When there's no `ReportConversion` impl and adding one isn't justified:

```rust
serde_json::from_str(json_str)
    .map_err(|e| report!(CliError::Other(format!("Invalid JSON: {e}"))))?;
```

### Pattern 8: Error inspection with `.current_context()`

Pattern-match on typed `Report` errors for semantic handling (e.g., retry logic):

```rust
if let Err(e) = operation().await {
    match e.current_context() {
        MyError::Transient => retry(),
        MyError::Fatal(msg) => return Err(e),
    }
}
```

### Anti-patterns

These are error handling patterns that MUST NOT be used:

- **`Result<T, String>`** — always define a typed error enum with `Report<E>`.
- **`Result<T, (StatusCode, &str)>`** — use typed errors; map to HTTP status at the handler level.
- **Reusing unrelated error variants** (e.g. `PkiError::Hostname` for a database error) — add a new variant.
- **`format!("error: {e}")` losing the error chain** — use `#[from]`, `context_transform()`, or `context_to()` to preserve the original error.
- **Bare error enums without `Report`** — every boundary error type should use `pub type Result<T> = std::result::Result<T, Report<MyError>>`.

### Mutex and RwLock locks

The release profile uses `panic = "abort"`, so lock poisoning **cannot occur in production**. `.unwrap()` is allowed on `Mutex::lock()`, `RwLock::read()`, and `RwLock::write()`:

```rust
let guard = store.lock().unwrap();
```

Do NOT use `.map_err()` to convert `PoisonError` into an application error — this adds unnecessary complexity since poisoning is impossible with `panic = "abort"`.

### Rules summary

1. **Every boundary has its own error enum.** Do not reuse error types across crate boundaries.
2. **Derive `Debug` and `Error`** (via thiserror) on all error enums.
3. **Use structured context** -- prefer typed variants (`NotFound(String)`) over generic string errors.
4. **No secrets in error messages.** Never include tokens, passwords, keys, or credentials.
5. **Use `Report<MyError>` as the error type**, not bare `MyError`. The `Result<T>` alias enforces this.
6. **Implement `ReportConversion`** (via `impl_report_conversion!` macro) for every foreign error type your boundary may encounter.

## API error responses

All web API error responses use a consistent JSON format defined by `ErrorResponse` (`crates/shared/web-api-types/src/error.rs`):

```json
{
  "error": "Human-readable error message",
  "code": "optional_machine_readable_code"
}
```

The `code` field is optional (`#[serde(skip_serializing_if = "Option::is_none")]`) and used only where a machine-readable code adds value (e.g. `"not_found"` for the 404 fallback, `"agent_version_too_old"` for version checks).

### Helper functions

`crates/ui/web-api/src/error_response.rs` provides two helpers:

- `error_response(status, message) -> Response` — most common case, no code field
- `error_response_with_code(status, message, code) -> Response` — includes a machine-readable code

All route handlers, middleware rejections, and custom `IntoResponse` impls use these helpers instead of constructing raw tuples.

### Convention

- **Do**: `error_response(StatusCode::BAD_REQUEST, "Invalid input")`
- **Do not**: `(StatusCode::BAD_REQUEST, "Invalid input").into_response()`
- **Do not**: construct `Json(serde_json::json!({"error": "..."}))` manually
- The 404 fallback uses `error_response_with_code` for the JSON path; the HTML path returns a plain-text "Not Found".
- WebSocket endpoints (`service_ws.rs`) are excluded — they use protocol-level error handling, not JSON responses.

### Frontend integration

The frontend (`frontend/src/lib/api.ts`) uses `extractErrorMessage(res)` to parse error responses. It tries to parse JSON and extract the `error` field; falls back to the raw response text if parsing fails. The `ErrorResponse` TypeScript interface is defined in `frontend/src/lib/types.ts`.

## Pagination

All list endpoints that can grow unboundedly use server-side pagination with a consistent response envelope. Shared types are defined in `crates/shared/web-api-types/src/pagination.rs`.

### Query parameters

| Parameter | Type | Default | Description |
| --- | --- | --- | --- |
| `page` | u64 | 1 | Page number (1-indexed) |
| `per_page` | u64 | 20 | Items per page (clamped to 1–1000) |

### Response envelope

All paginated endpoints return a `PaginatedResponse<T>`:

```json
{
  "items": [...],
  "total": 42,
  "page": 1,
  "per_page": 20,
  "total_pages": 3
}
```

### Paginated endpoints

| Endpoint | Query struct | Notes |
| --- | --- | --- |
| `GET /api/v1/services` | `ListServicesQuery` (includes `page`/`per_page`) | Filterable by `type`, `status` |
| `GET /api/v1/hosts` | `PaginationParams` | |
| `GET /api/v1/software-items` | `PaginationParams` | |
| `GET /api/v1/update-history` | `UpdateHistoryQuery` (includes `page`/`per_page`) | Filterable by `host_id`, `software_item_id`, `status` |
| `GET /api/v1/provider-configs` | `PaginationParams` | |

### Endpoints NOT paginated (already bounded)

| Endpoint | Reason |
| --- | --- |
| `GET /api/v1/settings/mqtt` | Bounded by `MqttMaxClientsPerTenant` (default 10) |
| `GET /api/v1/auth/api-tokens` | Per-user, typically small |

### Adding pagination to a new endpoint

For endpoints with an existing query struct, add `page: Option<u64>` and `per_page: Option<u64>` fields directly (serde_urlencoded does not support `#[serde(flatten)]`) and provide a `pagination()` helper method. For endpoints without an existing query struct, use `Query<PaginationParams>` as a new extractor.

### Key files

| File | Purpose |
| --- | --- |
| `crates/shared/web-api-types/src/pagination.rs` | `PaginationParams`, `ResolvedPagination`, `PaginatedResponse<T>` |

## Host entity

A `Host` represents a physical or virtual machine, decoupled from the `Agent` process identity. Hosts are identified by `machine_id` — a persistent system identifier (`/etc/machine-id` on Linux, `IOPlatformUUID` on macOS).

### Database tables

- **`hosts`**: `id` (UUID PK), `machine_id` (unique), `hostname`, `friendly_name`, `os_type?`, `os_version?`, `architecture?`, `ip_address?`, `last_seen_at?`, `created_at`, `updated_at`, `deactivated_at?`
- **`service_hosts`**: junction table with composite PK `(service_id, host_id)` and `linked_at` timestamp. FKs cascade on delete.

### Wire protocol additions

All UUID-typed ID fields (`service_id`, `software_item_id`, `update_history_id`, `mqtt_client_id`, `tenant_id`) in wire protocol structs use `uuid::Uuid` instead of `String`. This ensures UUID validation at serde deserialization time rather than requiring manual `Uuid::parse_str()` in consumers. The JSON format remains the same (hyphenated UUID strings).

Three wire payload fields use typed enums instead of raw strings — invalid values are rejected at deserialization:
- `EnrollPayload.service_type: ServiceType` — `Agent` or `Mqtt` (serialized as `"agent"` / `"mqtt"`)
- `EnrolledPayload.status: EnrollmentStatus` — `Pending` or `Approved` (serialized as `"pending"` / `"approved"`)
- `ExecuteUpdatePayload.shell: Option<HookShell>` — `Bash`, `Sh`, or `PowerShell` (serialized as `"bash"` / `"sh"` / `"powershell"`); defaults to `Bash`

Other crates (`web-api-types`, `db`) keep their own parallel enums; conversion happens in consuming crates (e.g. `wire_hook_shell()` in the web-api crate).

- `HostInfo` struct: `machine_id`, `os_type?`, `os_version?`, `architecture?`
- `EnrollPayload` includes `service_type: ServiceType` and `host_info: Option<HostInfo>` (required for agents, absent for MQTT). The `service_id` is generated by the controller.
- `RequestCertificatePayload` includes `csr_pem: String` — a fresh CSR for certificate issuance after approval
- `RenewCertificatePayload` includes `csr_pem: String` — a fresh CSR for certificate renewal
- `CertificatePayload` contains `cert_pem: String` and `not_after: UtcDateTime` (no `key_pem` — the private key never leaves the agent)
- `ReportHostInfo(ReportHostInfoPayload)` variant in `ServiceMessage` (agent-specific) — sent by authenticated agents immediately after mTLS WebSocket connect
- `RenewCertificate(RenewCertificatePayload)` variant in `ServiceMessage` — service requests certificate renewal with a fresh CSR (early or on-demand)
- `ServiceSettings(ServiceSettingsPayload)` variant in `ControllerMessage` — pushed after authentication with `renewal_window_hours`, `ca_bundle_hash`, and `shutdown_timeout_seconds` (`Option<u32>`)
- `CaBundleUpdated(CaBundleUpdatedPayload)` variant in `ControllerMessage` — pushed after CA rotation with the new bundle PEM
- `RequestCertRenewal(RequestCertRenewalPayload)` variant in `ControllerMessage` — pushed after CA rotation or PKI address change to prompt services to renew certificates immediately; includes a human-readable `reason` field
- `CheckVersions(CheckVersionsPayload)` variant in `ControllerMessage` — requests installed version detection from agents
- `VersionCheckResults(VersionCheckResultsPayload)` variant in `ServiceMessage` (agent-specific) — agent response with detected versions or errors
- `ReportHostInfoPayload` includes `agent_version: String` — agent binary version (from `CARGO_PKG_VERSION`)
- `ExecuteUpdate(ExecuteUpdatePayload)` variant in `ControllerMessage` — triggers a software update on the agent (boxed to avoid large enum variant)
- `UpdateStarted(UpdateStartedPayload)` variant in `ServiceMessage` (agent-specific) — agent acknowledges update start with detected from_version
- `UpdateOutput(UpdateOutputPayload)` variant in `ServiceMessage` (agent-specific) — agent streams update output (stdout, stderr, pre/post-hook, system)
- `UpdateResult(UpdateResultPayload)` variant in `ServiceMessage` (agent-specific) — agent reports final update status with accumulated output
- `Register(MqttRegisterPayload)` variant in `ServiceMessage` (MQTT-specific) — MQTT service registers with the controller (includes `active_mqtt_clients: Vec<Uuid>`)
- `ReleaseTenants(MqttReleaseTenantsPayload)` variant in `ServiceMessage` (MQTT-specific) — MQTT service releases MQTT client leases (by `mqtt_client_ids`)
- `ServerRestarting(ServerRestartingPayload)` variant in `ControllerMessage` — sent during graceful restart to notify services; includes a human-readable `reason` field
- `Disconnecting(DisconnectingPayload)` variant in `ServiceMessage` — service notifies controller before graceful disconnect (includes `DisconnectReason`: `shutdown` or `restart`; optional `active_mqtt_clients: Vec<Uuid>` for MQTT)

### Agent graceful shutdown

Agents support graceful shutdown to ensure in-flight updates complete before disconnecting. The shutdown behavior is controlled by signal handlers and a configurable timeout.

**Signals:**
- **SIGINT/SIGTERM**: Triggers graceful shutdown with `LoopOutcome::Shutdown`
- **SIGHUP**: Triggers graceful shutdown with `LoopOutcome::Restart` (exits cleanly for external restart by systemd/supervisors)

**Shutdown sequence:**
1. Signal received → set `shutting_down` flag
2. If update in progress:
   - Continue streaming output to controller
   - Wait for update completion (with `shutdown_timeout_seconds` timeout)
   - Send `UpdateResult` on completion or timeout
3. Send `Disconnecting { reason: shutdown|restart }` to controller
4. Close WebSocket gracefully
5. Return appropriate `LoopOutcome`

**Configuration:**
- `shutdown_timeout_seconds` in `ServiceSettingsPayload` (default: 120 seconds, `Option<u32>`)
- Controller pushes this value after authentication
- Agent waits up to this duration for in-flight updates to complete

**Wire protocol:**
- `DisconnectReason` enum: `shutdown` (SIGINT/SIGTERM) or `restart` (SIGHUP)
- `DisconnectingPayload { reason: DisconnectReason }` — sent before closing connection

### Agent version tracking

Agents report their binary version via the `agent_version` field in `ReportHostInfoPayload`. The controller stores this in the `services.client_version` column and enforces a minimum version check:

- **Minimum version**: Hardcoded `MIN_AGENT_VERSION` constant in `crates/ui/web-api/src/routes/agent_ws.rs` (currently `"0.0.1"`; note: `agent_ws.rs` is now a `pub(crate)` internal module)
- **Enforcement**: On `ReportHostInfo`, if the agent's version is below the minimum (semver comparison), the controller sends an `Error { code: "agent_version_too_old" }` message and closes the connection
- **API exposure**: The `client_version` field is included in `ServiceResponse` (REST API)

### Version check wire protocol

The controller can request installed version detection from agents:

1. **Controller → Agent**: `CheckVersions(CheckVersionsPayload)` containing a list of `VersionCheckAssignment` items
2. **Agent processes**: For each assignment, the agent dispatches to the appropriate `LocalProvider` based on `provider_type`
3. **Agent → Controller**: `VersionCheckResults(VersionCheckResultsPayload)` containing results with optional `installed_version` or `error`
4. **Controller stores**: Updates `host_software_items.installed_version` and `installed_version_detected_at` for successful results

**VersionCheckAssignment fields:**
- `software_item_id`: UUID of the software item
- `name`: Display name for logging
- `provider_type`: Provider discriminator (`github_releases`, `docker_registry`, `proxmox_helper_scripts`)
- `package_identifier`: Provider-specific identifier
- `config`: Provider configuration as JSON

**Local Provider stubs**: Each provider crate implements a local `Provider` struct with `detect_installed_version()` returning `Ok(None)` (stub) and `execute_update()` returning an error. The agent dispatches via a `match` on `provider_type` in `crates/core/agent/src/version_check.rs`.

### Agent host info collection

`crates/core/agent/src/host_info.rs` provides `collect_host_info() -> HostInfo`:
- `machine_id`: Linux `/etc/machine-id`, macOS `IOPlatformUUID`, fallback `"unknown"`
- `os_type`: `std::env::consts::OS`
- `os_version`: Linux `/etc/os-release` PRETTY_NAME, macOS `sw_vers`
- `architecture`: `std::env::consts::ARCH`

### Controller host logic

`find_or_create_host_and_link()` in the agent WebSocket handler area:
- Skips if `machine_id == "unknown"`
- Finds host by `machine_id` → updates mutable fields (hostname, IP, OS info, `last_seen_at`)
- Or creates new host with `friendly_name` defaulting to hostname
- Upserts `service_host` link (insert if not exists)
- Called during enrollment and on `ReportHostInfo` messages
- Non-fatal on failure

### REST API

| Method | Path | Permission | Action |
|--------|------|------------|--------|
| GET | `/api/v1/hosts` | ViewAgents | List non-deactivated hosts with linked agents |
| GET | `/api/v1/hosts/{id}` | ViewAgents | Get single host with linked agents |
| PUT | `/api/v1/hosts/{id}` | ManageAgents | Update friendly_name |
| DELETE | `/api/v1/hosts/{id}` | ManageAgents | Soft-delete (set deactivated_at) |

## Software item entity

A `SoftwareItem` defines what to track: a named piece of software linked to a `ProviderConfig`. Each item can be assigned to multiple hosts via the `HostSoftwareItem` junction table, which stores per-host state (installed version, detection timestamp).

### Database tables

- **`software_items`**: `id` (UUID PK), `name`, `provider_config_id` (FK → `provider_configs.id`, ON DELETE RESTRICT), `package_identifier` (default `""`), `config_override?` (JSON), `enabled` (default `true`), `last_checked_at?`, `created_at`, `updated_at`, `deactivated_at?`
  - Unique constraint: `(provider_config_id, package_identifier)` — prevents duplicate tracking of the same package from the same source
  - Indexes: `idx_software_items_provider_config_id`, `idx_software_items_deactivated_at`
- **`host_software_items`**: junction table with composite PK `(host_id, software_item_id)`, `installed_version?`, `installed_version_detected_at?`, `last_updated_at?`, `linked_at`. FKs cascade on delete.
- **`available_versions`**: `id` (UUID PK), `software_item_id` (FK → `software_items.id`, ON DELETE CASCADE), `version?`, `release_date?`, `release_notes?` (text), `extra?` (JSON — provider-specific metadata such as tag, is_prerelease, release_url), `created_at`, `updated_at`
  - CHECK constraint: at least one of `version` or `release_date` must be non-null
  - Index: `idx_available_versions_software_item_id`

### Relationships

- `SoftwareItem` belongs_to `ProviderConfig` (many:1 — multiple items can share one config)
- `ProviderConfig` has_many `SoftwareItem`
- `SoftwareItem` has_many `AvailableVersion` (one:many — upstream release records per item)
- `SoftwareItem` ↔ `Host` via `HostSoftwareItem` junction (many:many)
- `package_identifier` distinguishes items within a shared config (e.g. different assets from the same GitHub repo)
- `config_override` extends/overrides the base ProviderConfig at resolution time (e.g. different `asset_patterns` or `tag_strip_prefix`)

### REST API

| Method | Path | Permission | Status | Description |
|--------|------|------------|--------|-------------|
| POST | `/api/v1/software-items` | ManageSettings | 201 | Create a new software item |
| GET | `/api/v1/software-items` | ViewSettings | 200 | List all active software items (with host count) |
| GET | `/api/v1/software-items/{id}` | ViewSettings | 200 | Get software item with assigned hosts + installed versions |
| PUT | `/api/v1/software-items/{id}` | ManageSettings | 200 | Update name, enabled, package_identifier, config_override |
| DELETE | `/api/v1/software-items/{id}` | ManageSettings | 204 | Soft-delete |
| POST | `/api/v1/software-items/{id}/hosts` | ManageSettings | 200 | Assign to additional host(s) |
| DELETE | `/api/v1/software-items/{id}/hosts/{host_id}` | ManageSettings | 204 | Unassign from a host |

### Validation rules

- `name` must not be empty
- `provider_config_id` must reference an active (non-deactivated) provider config
- `(provider_config_id, package_identifier)` must be unique among active items
- `config_override`, if provided, is validated by merging with the base config and running provider-specific validation
- Host IDs in assignment requests must reference active (non-deactivated) hosts
- `provider_config_id` cannot be changed after creation

## Update history entity

An `UpdateHistory` record tracks a single software update operation for a specific software item on a specific host. Records are immutable — once created they are not modified or soft-deleted.

### Database table

- **`update_history`**: `id` (UUID PK), `host_id` (FK → `hosts.id`, ON DELETE CASCADE), `software_item_id` (FK → `software_items.id`, ON DELETE CASCADE), `from_version?` (version before update, null if unknown), `to_version` (target version), `status` (string-backed enum: pending, in_progress, completed, failed), `output` (text, NOT NULL — full command output for success or failure), `initiated_by` (string, NOT NULL — user UUID, "scheduler", or "mqtt"), `started_at`, `completed_at?`, `created_at`
  - Indexes: `idx_update_history_host_id`, `idx_update_history_software_item_id`, `idx_update_history_status`, `idx_update_history_host_software_item` (composite)

### Status enum

The `UpdateStatus` enum is defined in two places:

- **Entity level** (`crates/shared/db/src/entity/update_history.rs`): `DeriveActiveEnum` with `sea_orm(rs_type = "String")`. Variants: `Pending`, `InProgress`, `Completed`, `Failed`.
- **API level** (`crates/shared/web-api-types/src/update_history.rs`): `serde(rename_all = "snake_case")` with `as_str()` / `from_str()` methods. Conversion between DB and API enums happens in the route handler's `db_status_to_api` helper.

### Tenant scoping

No direct `tenant_id` column. Tenant scoping is implicit via `host_id` FK — the host table has `tenant_id`. The list endpoint loads all tenant host IDs and filters with `is_in()`. The get endpoint verifies the record's host belongs to the requesting tenant.

### Relationships

- `UpdateHistory` belongs_to `Host` (many:1)
- `UpdateHistory` belongs_to `SoftwareItem` (many:1)
- `Host` has_many `UpdateHistory`
- `SoftwareItem` has_many `UpdateHistory`

### REST API

| Method | Path | Permission | Description |
|--------|------|------------|-------------|
| GET | `/api/v1/update-history` | ViewSettings | List records (filterable by host_id, software_item_id, status) |
| GET | `/api/v1/update-history/{id}` | ViewSettings | Get single record |

Responses include denormalized `host_name` and `software_item_name` fields.

### Key files

| File | Purpose |
|------|---------|
| `crates/shared/db/src/entity/update_history.rs` | SeaORM entity with `UpdateStatus` enum |
| `crates/core/controller/src/migration/m20260203_000018_create_update_history.rs` | DB migration |
| `crates/shared/web-api-types/src/update_history.rs` | API types (response, query, status enum) |
| `crates/ui/web-api/src/routes/update_history.rs` | Route handlers + unit tests |

## Update hooks

Update hooks allow running commands before and after software updates. They support two configuration formats: structured hooks (with predefined templates) and legacy format (plain command arrays).

### Configuration format

Hooks are configured in the provider config or software item's `config_override` under a `hooks` key:

```json
{
  "hooks": {
    "pre_update": { ... },
    "post_update": { ... }
  }
}
```

Each hook phase (`pre_update`, `post_update`) can use:

1. **Predefined templates** — structured actions that map directly to commands
2. **Custom commands** — arbitrary shell commands

### Predefined hook templates

#### Systemd service

Manages systemd services with explicit actions:

```json
{
  "hooks": {
    "pre_update": {
      "predefined": {
        "systemd_service": {
          "service_name": "myapp",
          "action": "stop"
        }
      }
    },
    "post_update": {
      "predefined": {
        "systemd_service": {
          "service_name": "myapp",
          "action": "start"
        }
      }
    }
  }
}
```

**Available actions:** `start`, `stop`, `restart`, `reload`

Maps to: `systemctl {action} {service_name}`

#### Docker Compose

Manages docker-compose deployments with explicit actions:

```json
{
  "hooks": {
    "pre_update": {
      "predefined": {
        "docker_compose": {
          "action": "down",
          "project_dir": "/opt/myapp"
        }
      }
    },
    "post_update": {
      "predefined": {
        "docker_compose": {
          "action": "up",
          "project_dir": "/opt/myapp"
        }
      }
    }
  }
}
```

**Available actions:** `up`, `down`, `restart`, `pull`

**Optional fields:**
- `project_dir` — directory to run the command in
- `compose_file` — path to compose file (uses `-f` flag)

Maps to: `cd {project_dir} && docker-compose [-f {compose_file}] {action} [-d]` (the `-d` flag is added automatically for `up`)

### Custom commands

For commands not covered by predefined templates:

```json
{
  "hooks": {
    "pre_update": {
      "commands": ["echo 'Starting backup'", "backup.sh"],
      "shell": "bash"
    },
    "post_update": {
      "commands": ["systemctl restart myapp"],
      "shell": "bash"
    }
  }
}
```

### Shell types

The `shell` field controls which shell interpreter and fail-early settings are used:

| Shell | Fail-early settings | Description |
|-------|---------------------|-------------|
| `bash` (default) | `set -euo pipefail` | Exit on error, undefined vars, pipe failures |
| `sh` | `set -eu` | POSIX-compatible exit on error, undefined vars |
| `powershell` | `$ErrorActionPreference = 'Stop'` | Future Windows support |

Commands are wrapped with fail-early settings before execution to ensure hooks fail fast on errors.

### Merge strategy

When both provider config and software item `config_override` define hooks:

1. If override has a `hooks` key, it completely replaces the base config's hooks
2. If override doesn't have `hooks`, fall back to base config's hooks
3. Legacy format (`pre_update_commands`, `post_update_commands`) is supported for backward compatibility

### Phase markers in output

Hook output includes clear phase markers for debugging:

```
[pre-hook] Starting pre-update hooks...
[pre-hook] Running: systemctl stop myapp
[pre-hook] (exit code 0)
[update] Executing update to version 2.0.0...
[post-hook] Starting post-update hooks...
[post-hook] Running: systemctl start myapp
[post-hook] (exit code 0)
[update] Update completed successfully
```

### Key files

| File | Purpose |
|------|---------|
| `crates/shared/web-api-types/src/update_hooks.rs` | Hook configuration types (`HookShell`, `PredefinedHook`, `HooksConfig`) |
| `crates/ui/web-api/src/update_hooks.rs` | Hook resolution and merge logic |
| `crates/core/agent/src/update.rs` | Hook execution with shell wrapper |
| `crates/shared/wire/asyncapi.yaml` | Wire protocol documentation (includes `shell` field) |

## Testing expectations

Every behaviour change must include tests. Types of tests used:

- **Unit tests**: pure logic, version comparison, parsing.
- **Provider tests**: parsing upstream metadata, mapping to internal models.
- **API boundary tests**: request/response (de)serialisation, backwards compatibility.
- **Error path tests**: expected failures produce correct error types and messages.
- **Docker integration tests**: reverse proxy tests using real containers (see below).

Run tests with:

```sh
cargo test --all-features
# or with nextest:
cargo nextest run --all-features
```

### Reverse proxy integration tests

Docker-based integration tests in `crates/core/controller/tests/reverse_proxy/` validate that the controller's middleware correctly extracts `ServiceIdentity` (unified identity extractor, replacing the former `AgentIdentity` and `MqttServiceIdentity`) from forwarded headers when behind real reverse proxies. Each test uses `testcontainers` to spin up a Docker container.

```text
crates/core/controller/tests/
  reverse_proxy.rs              -- test binary entry point
  reverse_proxy/
    pki.rs                      -- TestPki: CA + server cert + agent cert generation (rcgen)
    server.rs                   -- TestServer: lightweight Axum HTTPS server with real middleware
    ocsp_responder.rs           -- OcspResponder: HTTP and HTTPS OCSP responder for testing
    nginx.rs                    -- Nginx L7 test (nginx:latest)
    traefik.rs                  -- Traefik L7 test (traefik:v3)
    caddy.rs                    -- Caddy L7 test (caddy:latest)
    haproxy.rs                  -- HAProxy L7 test (haproxy:latest)
    envoy.rs                    -- Envoy L7 test (envoyproxy/envoy:v1.31-latest)
    nginx_crl.rs                -- Nginx CRL revocation test
    haproxy_crl.rs              -- HAProxy CRL revocation test
    envoy_crl.rs                -- Envoy CRL revocation test
    nginx_ocsp.rs               -- Nginx OCSP revocation tests (HTTP, HTTPS, AIA)
```

All tests are `#[ignore]` with descriptive messages and never run in normal `cargo test`. They require Docker.

```sh
# Run all reverse proxy tests
cargo test -p uptrakit-controller reverse_proxy -- --ignored

# Run a single proxy test
cargo test -p uptrakit-controller reverse_proxy::nginx -- --ignored
```

A dedicated `reverse-proxy-tests` CI job runs these on `ubuntu-latest` (Docker pre-installed).

## Provider architecture

Each software item is associated with a provider. A provider defines:

| Concern | Runs on | Responsibility |
| --- | --- | --- |
| Remote/upstream version | Controller | Fetch latest version metadata (version string, release URL, changelog URL, publish timestamp, channel, notes) |
| Local/installed version | Agent | Detect currently installed version |
| Update execution | Agent | Run the update (via sudo-allowlisted commands or custom script) |

Provider crates:

| Crate | Path | Purpose |
| --- | --- | --- |
| `uptrakit-provider-core` | `crates/providers/core/` | Shared provider trait (`Provider`) and abstractions |
| `uptrakit-provider-registry` | `crates/providers/registry/` | Centralized provider dispatch, config validation, and secret management |
| `uptrakit-provider-docker-registry` | `crates/providers/docker-registry/` | Docker/OCI Registry: controller tracks container image tags via semver filtering or digest change detection |
| `uptrakit-provider-github` | `crates/providers/github/` | GitHub Releases: controller fetches release metadata; agent installs from artifacts |
| `uptrakit-provider-proxmox-helper-scripts` | `crates/providers/proxmox-helper-scripts/` | Proxmox VE Helper-Scripts: agent auto-discovers and manages helper-script-installed apps |

The **Provider Registry** crate centralizes all provider operations:
- `ProviderRegistry::create_local_provider()` — creates local `Provider` instances from `ProviderType` and config
- `ProviderRegistry::create_remote_provider()` — creates remote `Provider` instances from `ProviderType` and config
- `ProviderRegistry::validate_config()` — validates provider configuration JSON
- `ProviderRegistry::mask_secrets()` / `restore_secrets()` — handles secret masking for API responses

The agent and web-api crates import only `uptrakit-provider-registry` — not individual provider crates. This eliminates scattered string-based provider matching and keeps all dispatch logic in one place.

The update step can always be overridden by a custom shell script, regardless of provider.

### Provider capabilities

The `ProviderCapability` enum defines optional features a provider may support:

| Capability | Trait method | Description |
| --- | --- | --- |
| `DiscoverLocalSoftware` | `discover_software()` | Enumerate software the provider can manage on the local system |
| `RefreshPackageIndex` | `refresh_package_index()` | Refresh/sync the local package database from remote sources (e.g. `apt update`) |

No existing provider implements `RefreshPackageIndex` yet; it is available as a foundation for future providers (e.g. apt, homebrew).

### Software discovery

The `Provider` trait includes an optional `discover_software()` method that allows providers to enumerate software they can manage on the local system. Providers that support this capability declare `ProviderCapability::DiscoverLocalSoftware` in their `capabilities()` method. The method returns a `Vec<DiscoveredSoftware>`, where each entry contains:

| Field | Type | Description |
| --- | --- | --- |
| `package_identifier` | `String` | Provider-specific identifier (maps to `SoftwareItem.package_identifier` in DB) |
| `name` | `String` | Human-readable display name |
| `installed_version` | `Option<Version>` | Currently installed version, if detected |
| `extra` | `Option<serde_json::Value>` | Arbitrary provider-specific metadata (e.g., install path, detection method) |

The default implementation returns an empty list. Providers that support discovery (e.g., Proxmox Helper-Scripts) override this method to scan the local system.

### GitHub Releases provider (`uptrakit-provider-github`)

Fetches release metadata from the GitHub API and converts it into `UpstreamRelease` values.

**Config fields (`GitHubConfig`):**

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `owner` | String | Yes | — | GitHub repository owner |
| `repo` | String | Yes | — | GitHub repository name |
| `auth_token` | String | No | `null` | Personal access token (for private repos / higher rate limits) |
| `api_base_url` | String | No | `https://api.github.com` | API base URL (for GitHub Enterprise) |
| `include_prereleases` | bool | No | `false` | Whether to include pre-release versions |
| `tag_strip_prefix` | String | No | `"v"` | Prefix to strip from tag names to extract version strings |
| `asset_patterns` | Vec\<String\> | No | `[]` | Regex patterns to filter release assets (empty = include all) |

**Behaviour:**
- Drafts are always skipped
- Rate limit headers are checked; warnings logged when remaining < 10
- 403/429 responses with `x-ratelimit-remaining: 0` return a rate-limit error
- Asset filtering uses regex matching against asset names

### Docker Registry provider (`uptrakit-provider-docker-registry`)

Tracks container image tags from OCI/Docker registries. Supports Docker Hub, GHCR, and any OCI Distribution Spec-compliant registry. This is a **RemoteProvider only** (controller-side); agent-side container discovery is not implemented.

**Config fields (`DockerRegistryConfig`):**

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `image` | String | Yes | -- | Full image reference (e.g. `nginx`, `ghcr.io/owner/repo`) |
| `registry` | Option\<String\> | No | inferred from `image` | Override registry hostname |
| `auth` | Option\<DockerAuth\> | No | `null` | Authentication credentials |
| `tracking_mode` | TrackingMode | No | `semver_tags` | `semver_tags` or `digest_tracking` |
| `tag_patterns` | Vec\<String\> | No | `[]` | Regex patterns to filter tags (semver mode, OR logic) |
| `tag_strip_prefix` | String | No | `"v"` | Prefix to strip before semver parsing |
| `include_prereleases` | bool | No | `false` | Include pre-release versions |
| `tracked_tag` | Option\<String\> | No | `"latest"` | Tag to track (digest mode) |
| `page_size` | u32 | No | `1000` | Max tags per API request |

**DockerAuth** (tagged enum with `#[serde(tag = "type")]`):
- `basic`: `username` + `password`
- `bearer`: `token`

**Tracking modes:**

- **SemverTags** (default): Lists tags from the registry, filters by `tag_patterns` (OR logic, empty = all), strips `tag_strip_prefix`, parses as semver (non-semver tags excluded), filters pre-releases unless `include_prereleases`, sorts descending by version. Each tag becomes an `UpstreamRelease` (no `release_notes`, no `published_at`, no `assets`).
- **DigestTracking**: Gets the manifest digest for `tracked_tag` (default `"latest"`). Returns a single `UpstreamRelease` with the digest as the version string. Useful for detecting when a mutable tag has been updated.

**Registry resolution:**
- `nginx` -> `registry-1.docker.io` / `library/nginx`
- `user/repo` -> `registry-1.docker.io` / `user/repo`
- `ghcr.io/owner/repo` -> `ghcr.io` / `owner/repo`
- `registry.example.com/path/repo` -> `registry.example.com` / `path/repo`

**Auth flow:** Uses OCI Distribution Spec token authentication. On 401, parses the `WWW-Authenticate: Bearer` challenge header, fetches a token from the realm endpoint (with Basic/Bearer credentials if configured), caches the token with expiry tracking, and retries the original request.

**Secret masking:** `auth.password` and `auth.token` fields are replaced with `"***"` in GET responses. On PUT, masked values are restored from the existing DB record.

### Provider configuration management

Provider-specific configurations are stored in the `provider_configs` table and managed via CRUD API endpoints:

| Method | Path | Permission | Action |
|--------|------|------------|--------|
| GET | `/api/v1/provider-configs` | ViewSettings | List all non-deactivated configs |
| GET | `/api/v1/provider-configs/{id}` | ViewSettings | Get a specific config |
| POST | `/api/v1/provider-configs` | ManageSettings | Create a new config |
| PUT | `/api/v1/provider-configs/{id}` | ManageSettings | Update a config (partial) |
| DELETE | `/api/v1/provider-configs/{id}` | ManageSettings | Soft-delete a config |

**Config validation:** On create/update, the JSON config is deserialized into the provider-specific config type (e.g. `GitHubConfig`) and validated. Unknown `provider_type` values return 400.

**Secret masking:** `auth_token` fields in config JSON are replaced with `"***"` in GET responses. On PUT, if `auth_token` is `"***"`, the existing value from the DB is preserved.

**Supported provider types:** `github_releases`, `docker_registry`.

When adding or changing a provider, document in the same PR:

- How installed version is detected (agent side)
- How upstream/latest version is determined (controller side)
- Version comparison rules (semver, tag prefixes, build metadata handling)
- Update mechanism, required privileges, and failure modes
- Required config fields with examples

## Home Assistant / MQTT integration

Each tracked software item becomes a Home Assistant `update` entity via MQTT auto-discovery. Entity attributes include: installed version, latest version, changelog URL, release link, and more. Updates can be triggered from Home Assistant, the Web UI, or the CLI.

## Dependencies policy

- Avoid heavy dependencies without strong justification.
- Prefer well-maintained crates with clear track records.
- Crates affecting command execution, untrusted input parsing, crypto, or networking receive extra scrutiny.

### Workspace vs crate-local placement

A dependency belongs in `[workspace.dependencies]` (root `Cargo.toml`) only when **two or more crates** use it. Single-consumer dependencies go directly in the crate's own `Cargo.toml`.

**Adding a new dependency:**

1. Check how many crates will use it.
2. If only one crate needs it, add it to that crate's `[dependencies]` with an explicit version.
3. If two or more crates need it, add it to `[workspace.dependencies]` and reference it with `{ workspace = true }` in each consumer.

**Promotion (crate-local to workspace):** when a second crate starts using an existing crate-local dependency, move the version spec to `[workspace.dependencies]` and replace both crate entries with `{ workspace = true }`.

**Demotion (workspace to crate-local):** when a dependency's last second consumer is removed, move the version spec back into the sole remaining consumer's `Cargo.toml` and delete the `[workspace.dependencies]` entry.

## Release profile

The workspace uses an optimised release profile:

```toml
[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
panic = "abort"
strip = true
```
