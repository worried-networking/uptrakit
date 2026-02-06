# Architecture

Uptrakit is an agent-based update tracking toolkit for self-hosted Linux environments. A central **controller** orchestrates version checks and exposes a Web UI and API, while lightweight **agents** on each managed host detect installed software versions and execute user-triggered updates. Communication uses secure WebSocket with mTLS authentication.

## Component Diagram

```text
┌──────────────┐         HTTPS/WSS (Rustls)           ┌──────────────────────┐
│              │◄──────────────────────────────────── │  Agent (host A)      │
│              │         outbound-only mTLS           │  local providers     │
│              │                                      │  sudo allowlists     │
│  Controller  │◄──────────────────────────────────── └──────────────────────┘
│              │
│  - Axum API  │◄──────────────────────────────────── ┌──────────────────────┐
│  - Scheduler │         outbound-only mTLS           │  Agent (host B)      │
│  - SeaORM DB │                                      └──────────────────────┘
│  - SvelteKit │
│    (static)  │         HTTPS                        ┌──────────────────────┐
│              │◄──────────────────────────────────── │  Browser (Web UI)    │
│              │                                      └──────────────────────┘
│              │
│              │         WSS (mTLS)                   ┌──────────────────────┐
│              │◄──────────────────────────────────── │  MQTT Service        │
│              │         /api/v1/ws/service             │  Instance 1          │──── MQTT ──── Broker
│              │                                      └──────────────────────┘
│              │         WSS (mTLS)                   ┌──────────────────────┐
│              │◄──────────────────────────────────── │  MQTT Service        │
│              │         /api/v1/ws/service             │  Instance 2          │──── MQTT ──── Broker
└──────────────┘                                      └──────────────────────┘
```

All agent connections are **outbound-only** -- agents initiate connections to the controller; the controller never connects to agents.

## Technology Stack

| Layer | Technology | Version / Notes |
| --- | --- | --- |
| Language | Rust | Edition 2024, `rust-version = "1.91"` in some crates |
| Async runtime | [Tokio](https://tokio.rs) | 1.x, full features |
| HTTP framework | [Axum](https://github.com/tokio-rs/axum) | 0.8 (with WebSocket support) |
| TLS | [Rustls](https://github.com/rustls/rustls) | 0.23, aws-lc-rs backend |
| ORM | [SeaORM](https://www.sea-ql.org/SeaORM/) | 2.0 (rc), multi-backend |
| Error handling | [rootcause](https://github.com/rootcause-rs/rootcause) + [thiserror](https://github.com/dtolnay/thiserror) | rootcause 0.11, thiserror 2 |
| MQTT client | [rumqttc](https://github.com/bytebeamio/rumqtt) | 0.25 |
| Password hashing | [argon2](https://crates.io/crates/argon2) | 0.5, OWASP parameters |
| JWT | [jsonwebtoken](https://crates.io/crates/jsonwebtoken) | 9 |
| Frontend | [SvelteKit](https://kit.svelte.dev) | 2.x, static adapter |
| UI framework | [Skeleton UI](https://skeleton.dev) | 2.x + Tailwind CSS |

## Crate Map

The workspace uses `resolver = "3"` with `members = ["crates/*/*"]`.

```text
crates/
├── core/
│   ├── agent/                     # uptrakit-agent (bin)
│   ├── controller/                # uptrakit-controller (bin)
│   └── mqtt/                      # uptrakit-mqtt (bin) — standalone MQTT service
├── providers/
│   ├── core/                      # uptrakit-provider-core (lib) — provider traits
│   ├── docker-registry/           # uptrakit-provider-docker-registry (lib)
│   ├── github/                    # uptrakit-provider-github (lib)
│   ├── proxmox-helper-scripts/    # uptrakit-provider-proxmox-helper-scripts (lib)
│   └── registry/                  # uptrakit-provider-registry (lib) — provider dispatch
├── shared/
│   ├── core/                      # uptrakit-core (lib) — shared domain models
│   ├── db/                        # uptrakit-shared-db (lib) — SeaORM entities & migrations
│   ├── directories/               # uptrakit-directories (lib) — cross-platform directory management
│   ├── enrollment/                # uptrakit-enrollment (lib) — shared enrollment, TLS, CA, CLI
│   ├── macros/                    # uptrakit-shared-macros (lib) — shared declarative macros
│   ├── web-api-types/             # uptrakit-web-api-types (lib) — shared HTTP types
│   └── wire/                      # uptrakit-internal-wire (lib) — wire protocol
└── ui/
    ├── cli/                       # uptrakit-cli (bin)
    └── web-api/                   # uptrakit-web-api (lib) — HTTP API + auth
```

For the full annotated tree with every file, see [AGENTS.md](AGENTS.md) section "Codebase layout".

## Wire Protocol

Agent-controller and MQTT service-controller communication uses WebSocket over TLS with JSON-serialized messages. Both agents and MQTT services are modelled as a unified **service** entity and share a single `ServiceMessage`/`ControllerMessage` wire enum, with variant subsets distinguished by service type. The protocol defines three connection types:

| Connection type | Authentication | Purpose |
| --- | --- | --- |
| Anonymous | None | Initial enrollment request |
| Enrolled | Bearer token | Certificate request during enrollment |
| Authenticated | mTLS client certificate | Normal operation (heartbeat, commands, data) |

### Agent lifecycle

1. Agent connects anonymously and sends an `enroll` message with hostname, host info, and optional enrollment token.
2. Controller generates a UUIDv7 `agent_id` and responds with `enrolled` (including agent_id and enrollment secret for reconnection) or `rejected`.
3. After approval, agent generates an ECDSA P-256 keypair + CSR (with CN=agent_id) and sends `request_certificate` with `csr_pem`.
5. Controller validates the CSR and signs the certificate. Agent receives `certificate` (cert PEM only — the private key never leaves the agent).
6. Agent reconnects with mTLS.
7. Normal operation: `ping`/`pong` heartbeats, status updates, version reports, update commands.
8. On shutdown (SIGINT/SIGTERM/SIGHUP): agent waits for in-flight updates (with configurable timeout), sends `disconnecting` message, and closes gracefully.

### Message types

Defined in `crates/shared/wire/`:

**Service → Controller (shared):** `ping`, `enroll`, `request_certificate`, `renew_certificate`, `disconnecting`

**Service → Controller (agent-specific):** `report_host_info`, `version_check_results`, `update_started`, `update_output`, `update_result`

**Service → Controller (MQTT-specific):** `register`, `release_tenants`

**Controller → Service (shared):** `pong`, `enrolled`, `approved`, `rejected`, `certificate`, `error`, `service_settings`, `ca_bundle_updated`, `request_cert_renewal`, `server_restarting`

**Controller → Service (agent-specific):** `check_versions`, `execute_update`

**Controller → Service (MQTT-specific):** `registered`, `tenant_assignments`, `tenant_config_updated`, `tenant_revoked`

The previous `AgentMessage` and `MqttServiceMessage` enums have been unified into `ServiceMessage`; likewise `ControllerMessage` and `MqttControllerMessage` are now a single `ControllerMessage` enum. The former `agent_settings` and `mqtt_service_settings` variants are unified as `service_settings`.

For the full message schema with payloads, see the [AsyncAPI specification](crates/shared/wire/asyncapi.yaml).

## PKI & mTLS

The controller operates an internal Certificate Authority for mutual TLS authentication with agents and MQTT services.

- **CA rotation**: Automatic when the managed CA enters a 6-month expiry window. Can also be triggered on demand via `POST /api/v1/settings/rotate-ca`. Produces a dual-CA trust bundle for seamless transition.
- **CRL partitioning**: Each CA signs a CRL only for certificates it issued (tracked via `ca_fingerprint`). Combined PEM CRLs are served at `GET /api/v1/pki/ca.crl`.
- **OCSP responder**: `POST /api/v1/pki/ocsp` (and GET variant) provides real-time certificate revocation status per RFC 6960. Supports SHA-1 and SHA-256 hash algorithms (Nginx/OpenSSL uses SHA-1). `ResponderID::ByKey` uses SHA-1 per RFC 6960 Section 2.3. Responses are signed with the active CA's private key using ECDSA P-256 SHA-256. Only Nginx natively supports OCSP verification of client certificates (via `ssl_ocsp` directive).
- **AIA/CDP extensions**: When `--pki-addr` is configured, certificates embed Authority Information Access (OCSP, CA Issuers) and CRL Distribution Points extensions. This enables proxies like Nginx to discover and use the OCSP responder automatically.
- **Optional PKI HTTP listener**: When `--pki-http listener` is set alongside `--pki-addr`, the controller starts a plain HTTP listener serving only PKI routes. Required for Nginx `ssl_ocsp_responder` which only supports `http://` OCSP responder URLs. Use `--pki-http external` to suppress the warning when PKI HTTP is handled by a reverse proxy.
- **Runtime state**: CA material is shared via a `tokio::sync::watch` channel carrying a `CaSnapshot` struct.
- **External CA**: Supported via `--ca-cert` / `--ca-key` flags, which disable managed rotation.
- **SAN sanity checks**: At startup, `--san` values are validated against the existing managed cert's SANs. Mismatched SANs trigger silent regeneration (same CA) or an error with fix instructions (different CA). `--san` is incompatible with `--tls-cert`/`--tls-key`.
- **PKI address validation**: At startup, the controller validates that an existing managed CA's AIA/CDP URLs match the reconciled `--pki-addr`. Mismatches cause a hard startup failure with actionable error messages.

For cryptographic algorithm details, see [SECURITY.md](SECURITY.md) section "Cryptographic Details". For the full operational flow (rotation steps, bundle distribution, agent update path), see [AGENTS.md](AGENTS.md) section "PKI & CA rotation".

## DB-Managed Settings

Most runtime settings are stored in the database (`setting` entity) as JSON values. At startup, `Settings::load()` issues a single `SELECT * FROM settings WHERE tenant_id = ?` query for the default tenant and distributes the resulting `RawSettings` map to all sub-loaders and to reconciliation — no per-key DB reads. Any unrecognised keys in the DB trigger a warning log.

The controller then reconciles CLI arguments with the pre-loaded values using a 5-case priority logic:

1. **DB has value + CLI provided + differs + `--force-settings-override`**: CLI wins, DB updated.
2. **DB has value + CLI provided + differs + no force**: DB wins, warning logged.
3. **DB has value + (CLI absent or same)**: DB value used.
4. **No DB value + CLI provided**: CLI value saved to DB.
5. **No DB value + CLI absent**: Hardcoded default saved to DB.

This ensures that settings persist across restarts without requiring CLI flags after initial configuration, while still allowing one-time overrides.

### Settings categories

| Category | DB key prefix | Runtime-changeable | API endpoint |
| --- | --- | --- | --- |
| Network | `network.*` | Proxies, headers, SANs, forwarded cert headers, PKI address: yes; bind addresses: restart required | `GET/PUT /api/v1/settings/network` |
| MQTT | Dedicated `mqtt_clients` table (multiple per tenant) | Yes (via API); controller pushes changes to connected MQTT service instances | Collection-style: `GET/POST /api/v1/settings/mqtt`, `GET/PUT/DELETE /api/v1/settings/mqtt/{id}`, `GET/PUT /api/v1/settings/mqtt/limit` |
| Registration | `registration.*` | Yes | `GET/PUT /api/v1/settings/registration` (includes `require_token_for_oidc`) |
| Authentication | `authentication.*` | Yes | `GET/PUT /api/v1/settings/authentication` |
| Service certificates | `service_certificates.*` | Yes | `GET/PUT /api/v1/settings/service-certificates` |

**Not DB-managed** (bootstrap/infrastructure): `--data-dir`, `--db-url`, `--tls-cert`, `--tls-key`, `--ca-cert`, `--ca-key`, `--static-dir`, `--oidc-*` (OIDC bootstrap flags write directly to the `oidc_providers` table).

The `Settings` struct (in `crates/ui/web-api/src/settings.rs`) uses `RwLock` for each settings group, allowing runtime-changeable settings to take effect immediately without restart. See [AGENTS.md](AGENTS.md) section "DB-managed settings" for the full key reference.

## Authentication & Authorization

### User authentication

- **Password**: Argon2id with OWASP-recommended parameters.
- **OIDC**: External identity providers with auto-create or account linking. Can be bootstrapped at startup via `--oidc-*` CLI flags (see [AGENTS.md](AGENTS.md) "OIDC provider bootstrap"). When registration mode is `Invite`, new OIDC users may need to provide a registration token (always for the first user, optionally for subsequent users via `require_token_for_oidc` setting).
- **Device authorization**: RFC 8628-style flow for CLI login. The CLI requests a device code, the user approves in the browser, and the CLI receives an API token. Works with any auth method (password or OIDC).
- **Sessions**: SHA-256 hashed tokens, 7-day expiry, 30-min sliding window.
- **JWT**: Access and refresh tokens carrying resolved permissions.
- **API tokens**: Long-lived, revocable bearer tokens for programmatic access.

### Agent authentication

- **Enrollment**: Agent enrolls with one-time token (controller assigns UUIDv7 agent_id). After approval, agent generates ECDSA P-256 keypair + CSR locally for certificate issuance. The private key never leaves the agent.
- **Normal operation**: mTLS on every WebSocket connection; CRL checked per connection.
- **Reverse proxy**: When behind a trusted proxy, agent identity is extracted from forwarded headers (`X-Forwarded-Tls-Client-Cert-Info` or `X-Forwarded-Tls-Client-Cert`). Issuer CN is verified against known CA certificates. See [docs/reverse-proxy/](docs/reverse-proxy/) for deployment guides.

### Authorization (RBAC)

Uses a typed `Permission` enum -- route handlers check `user.has_permission(Permission::...)`, never raw role strings. JWT tokens carry resolved permissions; the frontend receives permissions as `string[]`.

Roles: `owner` (all permissions including global settings), `admin` (all except global settings), and `user` (`view_agents` only). First registered user gets `owner`.

For the full permissions table and instructions on adding new permissions, see [AGENTS.md](AGENTS.md) section "Permissions model".

## Database Design

SeaORM provides a multi-backend abstraction layer. The controller supports:

| Backend | Feature flag | Use case |
| --- | --- | --- |
| SQLite | `db-sqlite` (default) | Development, single-node deployments |
| PostgreSQL | `db-postgres` | Production, multi-node setups |
| MySQL | `db-mysql` | Alternative production backend |

### Entities

The data model comprises 28 entities in `crates/shared/db/src/entity/`:

`api_token`, `auth_method`, `available_version`, `host`, `host_software_item`, `mqtt_client`, `mqtt_lease`, `oidc_provider`, `pending_account_link`, `pending_device_flow`, `pending_oidc_flow`, `pending_oidc_registration`, `pending_oidc_token_exchange`, `permission`, `provider_config`, `role`, `role_permission`, `service`, `service_certificate`, `service_host`, `session`, `setting`, `software_item`, `tenant`, `update_history`, `user`, `user_oidc_link`, `user_role`

The `host` entity represents a physical or virtual machine, identified by a persistent `machine_id` (e.g. `/etc/machine-id` on Linux). The `service_host` junction table models the many-to-many relationship between services and hosts, enabling automatic host matching across service re-enrollments and hostname changes.

The `pending_*` entities store transient auth flow state (device authorization, OIDC login, account linking, token exchange, OIDC registration with token). Persisting these to the database instead of in-memory maps enables controller restarts without losing active flows and supports HA multi-instance deployments with a shared database.

The `provider_config` entity stores per-provider-type configuration (e.g. GitHub owner/repo, auth tokens, asset filters). Multiple configs can exist per provider type (e.g. tracking releases from several GitHub repositories). Configs are managed via CRUD API endpoints with secret masking (auth tokens are replaced with `"***"` in responses) and provider-specific validation.

The `software_item` entity represents a trackable piece of software linked to a provider config. Each software item has a `package_identifier` (provider-specific, e.g. distinguishing multiple assets from the same GitHub repo) and an optional `config_override` JSON blob that extends/overrides the base provider config at resolution time. A unique constraint on `(provider_config_id, package_identifier)` prevents duplicate tracking. The `host_software_item` junction table assigns software items to hosts and stores per-host state (installed version, detection timestamp, last update timestamp). The `available_version` entity stores upstream release information per software item (one-to-many), including version string, release date, release notes, and provider-specific metadata in a flexible JSON `extra` column. A CHECK constraint ensures at least one of `version` or `release_date` is non-null. Software items are managed via CRUD API endpoints at `/api/v1/software-items` with host assignment/unassignment sub-endpoints.

The `update_history` entity tracks individual software update operations. Each record represents a single update attempt for a specific software item on a specific host, with status lifecycle tracking (pending, in_progress, completed, failed). The `output` text field captures full command output for both successful and failed updates. The `initiated_by` field identifies who triggered the update (user UUID, "scheduler", or "mqtt"). Tenant scoping is implicit via the `host_id` foreign key (same pattern as `available_versions` which scopes through `software_item_id`). Records are immutable history with no soft delete. Read-only API endpoints at `/api/v1/update-history` support filtering by host_id, software_item_id, and status.

### Multi-tenancy

The database supports multi-tenancy via a `tenants` table and `tenant_id` foreign keys on scoped tables (`services`, `hosts`, `provider_configs`, `software_items`, `oidc_providers`, `user_roles`, `settings`, `mqtt_clients`). A seeded default tenant is used in single-tenant mode. Global tables (`users`, `roles`, `permissions`, `sessions`, `api_tokens`, `pending_*` including `pending_oidc_registration`) remain unscoped. The `service_certificates` and `service_hosts` tables are scoped indirectly through the `services` table. The `TenantContext` Axum extractor resolves the active tenant from the `X-Tenant-Id` header or falls back to the default tenant. See [AGENTS.md](AGENTS.md) section "Multi-tenancy" for details.

### Migrations

SeaORM migrations live alongside the entity definitions. They run automatically on controller startup, creating or updating the schema as needed.

## Provider Architecture

Providers define how software items are tracked and updated. Each provider splits into two sides:

| Concern | Runs on | Responsibility |
| --- | --- | --- |
| Remote/upstream version | Controller | Fetch latest version metadata from upstream sources |
| Local/installed version | Agent | Detect currently installed version on the host |
| Software discovery | Agent | Enumerate software the provider can manage on the local system (optional, default returns empty list) |
| Update execution | Agent | Run the update via sudo-allowlisted commands or custom scripts |

### Current providers

| Provider | Path | Description |
| --- | --- | --- |
| Provider Core | `crates/providers/core/` | Shared `Provider` trait and `ProviderCapability` enum |
| Provider Registry | `crates/providers/registry/` | Centralized provider dispatch, config validation, and secret management |
| GitHub Releases | `crates/providers/github/` | Tracks GitHub release metadata; agent installs from artifacts |
| Docker Registry | `crates/providers/docker-registry/` | Tracks container image tags from OCI/Docker registries |
| Proxmox Helper-Scripts | `crates/providers/proxmox-helper-scripts/` | Auto-discovers and manages PVE helper-script-installed apps |

The **Provider Registry** crate (`uptrakit-provider-registry`) centralizes all provider-related operations:
- Creating local and remote `Provider` instances from `ProviderType` and config JSON
- Validating provider configuration before storage
- Masking and restoring secrets in config JSON for API responses

The agent and web-api crates import only the registry — not individual provider crates — eliminating scattered string-based provider matching.

The update step can always be overridden by a custom shell script, regardless of provider.

### Provider configuration storage

Provider-specific configurations are stored in the `provider_configs` database table as JSON blobs. Each config record has:

- A user-friendly `name` (e.g. "Prometheus GitHub Releases")
- A `provider_type` discriminator (e.g. `github_releases`)
- A `config` JSON object validated against the provider's config schema on create/update
- An `enabled` flag and soft-delete via `deactivated_at`

The web API exposes CRUD endpoints at `/api/v1/provider-configs` with `ViewSettings` / `ManageSettings` permissions. Secret fields (e.g. `auth_token` for GitHub, `auth.password`/`auth.token` for Docker Registry) are masked with `"***"` in API responses and preserved on update when the masked value is sent back.

## Frontend Architecture

The Web UI is a SvelteKit single-page application using the static adapter (no SSR). The controller serves the built frontend at runtime.

- **Framework**: SvelteKit 2.x with Svelte 5
- **UI**: Skeleton UI 2.x with Tailwind CSS
- **Build**: Vite 6.x, output to `frontend/build/`
- **API client**: Shared module in `frontend/src/lib/` with typed API calls
- **Auth store**: Svelte store managing JWT tokens, permissions, and session state
- **Routing**: SvelteKit file-based routing in `frontend/src/routes/`
- **Dev proxy**: Vite proxies API requests to the controller during development

## MQTT / Home Assistant Integration

Each tracked software item is published as a Home Assistant `update` entity via MQTT auto-discovery. Entity attributes include installed version, latest version, changelog URL, release link, and more.

MQTT is handled by a separate `uptrakit-mqtt` binary (`crates/core/mqtt/`) that communicates with the controller via WebSocket with mTLS authentication -- the same enrollment and certificate model used by agents. The MQTT service has no direct database access; the controller manages all state (leases, tenant configs) and pushes configuration changes to connected instances.

### MQTT service lifecycle

1. **Enrollment**: MQTT service connects anonymously to `/api/v1/ws/service`, sends `enroll` with hostname and optional enrollment token. Controller generates a UUIDv7 service_id and responds with `enrolled`. If a valid MQTT enrollment token is provided, the service is auto-approved; otherwise manual approval is required via the REST API. All messages use the unified `ServiceMessage`/`ControllerMessage` wire enums.
2. **Certificate issuance**: After approval, the service generates an ECDSA P-256 keypair + CSR (CN=service_id) and sends `request_certificate`. Controller signs and returns the certificate.
3. **Authenticated operation**: Service reconnects with mTLS, sends `register` with instance_id, max_tenants, and active_tenants (for reconnect reconciliation). Controller responds with tenant assignments.
4. **Runtime**: Controller pushes `tenant_config_updated` and `tenant_revoked` messages when MQTT settings change via the REST API. Service sends periodic `ping` messages; the controller uses Ping receipt to update lease heartbeats and responds with `pong`.

### Lease management

Multiple MQTT service instances can run simultaneously. The controller manages centralized lease coordination:
- Instances register with a `max_tenants` capacity (0 = unlimited)
- Unclaimed MQTT clients are assigned on registration (one lease per MQTT client config, keyed by `mqtt_client_id`)
- Leases are tracked in the `mqtt_leases` table (controller-managed, not directly accessed by MQTT services)
- On instance disconnect, its leases are released (no automatic redistribution)
- Stale leases are cleaned up periodically (no heartbeat within timeout)

The MQTT client supports two transport types: plain TCP (`mqtt://`) and TLS (`mqtts://`). Connection parameters are stored in the `mqtt_clients` database table (multiple rows per tenant, up to a configurable limit) and managed via the collection-style settings API. See [AGENTS.md](AGENTS.md) section "MQTT Service (standalone binary)" for the full connection lifecycle, CLI flags, and key files.

Updates can be triggered from Home Assistant, the Web UI, or the CLI -- all paths converge on the same controller API.

## Key Design Decisions

| Decision | Rationale |
| --- | --- |
| **No automatic updates** | Users must explicitly trigger updates. The scheduler only checks for new versions. This prevents unattended breakage in homelab environments. |
| **Outbound-only agents** | Agents connect to the controller; never the reverse. Simplifies firewall rules and eliminates the need for agents to expose network services. |
| **mTLS for agents** | Client certificates provide strong identity without shared secrets. The internal CA avoids dependency on external PKI infrastructure. |
| **CSR-based certificate issuance** | Agents generate their own ECDSA P-256 keypairs and submit CSRs to the controller for signing. The private key never leaves the agent. A fresh keypair is generated for each CSR (enrollment and renewals). The controller validates CSR signatures and controls all certificate parameters (DN, EKU, validity). |
| **Permission enum over role strings** | Typed permissions catch authorization bugs at compile time and make the permission model explicit in code. |
| **SeaORM multi-backend** | SQLite for development simplicity; PostgreSQL/MySQL for production. Feature flags keep the binary lean. |
| **rootcause + thiserror** | rootcause provides `Report`-based error propagation with structured context. thiserror generates the error enums. The `impl_report_conversion!` macro (in `uptrakit-shared-macros`) reduces `ReportConversion` trait impls to one-liners. Together they enforce boundary-aware error handling without boilerplate. |
| **Rustls over OpenSSL** | Pure-Rust TLS avoids OpenSSL linking complexity and provides memory safety guarantees. aws-lc-rs backend offers FIPS-capable cryptography. |
| **SvelteKit static adapter** | No server-side rendering needed -- the controller serves the pre-built SPA. Keeps deployment simple (single binary + static files). |
| **MQTT for Home Assistant** | MQTT auto-discovery is the standard integration mechanism for Home Assistant. Native protocol avoids custom HA add-on complexity. Runs as a separate binary (`uptrakit-mqtt`) that connects to the controller via WebSocket/mTLS (same enrollment model as agents). No direct database access -- the controller pushes tenant configs and manages leases centrally. |
| **Partitioned CRLs** | Each CA signs a CRL only for its own certificates. Prevents cross-CA revocation confusion during rotation periods. |
| **HTTPS-only controller** | The controller listens on HTTPS by default. An optional plain HTTP listener (`--pki-http listener`) can be started for PKI-only endpoints (OCSP, CRL, CA cert) when needed by Nginx `ssl_ocsp_responder`. All agent and browser connections use TLS. |
| **Shared enrollment crate** | The `uptrakit-enrollment` crate provides unified service identity management, TLS configuration, CA bootstrap, WebSocket enrollment protocol, and CLI arguments shared by both agent and MQTT service binaries. This eliminates code duplication and ensures consistent enrollment behavior across service types. |
| **Flexible agent bootstrap** | Both agent and MQTT service support four CA bootstrap modes: cached CA from disk, `--ca-cert` file, `--pki-addr` fetch, `--tofu` (TOFU via HTTPS), or system trust store. A single `--url` flag replaces separate host/port/http-port args. An optional `--pki-addr` allows fetching the CA certificate from a separate PKI endpoint (including plain HTTP). |
| **Reverse proxy support** | L4 passthrough and L7 TLS termination. Agent identity forwarded via structured info or PEM headers with CA CN verification. Header stripping prevents spoofing from non-proxy clients. Docker integration tests validate all 5 supported proxies (Nginx, Traefik, Caddy, HAProxy, Envoy). |
| **Zero-downtime graceful restart** | HAProxy-style restart using `SO_REUSEPORT`. New process binds the same port, starts accepting immediately, then signals the old process (SIGUSR1). Old process stops accepting, scatters `ServerRestarting` notifications to agents over 5 seconds to avoid thundering herd, drains existing connections, then exits. Agents may not notice the restart if their connection stays open. |
| **Agent graceful shutdown** | Agents handle SIGINT/SIGTERM/SIGHUP with graceful shutdown: wait for in-flight updates (configurable timeout), notify controller, close cleanly. SIGHUP returns exit code 0 for systemd restart. Prevents abandoned updates and data loss. |
