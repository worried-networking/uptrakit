# Architecture

Uptrakit is an agent-based update tracking toolkit for self-hosted Linux environments. A central **controller** orchestrates version checks and exposes a Web UI and API, while lightweight **agents** on each managed host detect installed software versions and execute user-triggered updates. Communication uses secure WebSocket with mTLS authentication.

## Component Diagram

```text
┌──────────────┐         HTTPS/WSS (Rustls)         ┌──────────────────────┐
│              │◄───────────────────────────────────── │  Agent (host A)      │
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
│              │─────── MQTT ──────────────────────── ┌──────────────────────┐
│              │                                      │  MQTT Broker         │
└──────────────┘                                      │  (Home Assistant)    │
                                                      └──────────────────────┘
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
│   └── controller/                # uptrakit-controller (bin)
├── providers/
│   ├── core/                      # uptrakit-provider-core (lib) — provider traits
│   ├── github/                    # uptrakit-provider-github (lib)
│   └── proxmox-helper-scripts/    # uptrakit-provider-proxmox-helper-scripts (lib)
├── shared/
│   ├── core/                      # uptrakit-core (lib) — shared domain models
│   ├── db/                        # uptrakit-shared-db (lib) — SeaORM entities & migrations
│   └── wire/                      # uptrakit-internal-wire (lib) — wire protocol
└── ui/
    ├── cli/                       # uptrakit-cli (bin)
    ├── mqtt/                      # uptrakit-mqtt (lib) — MQTT / HA integration
    └── web-api/                   # uptrakit-web-api (lib) — HTTP API + auth
```

For the full annotated tree with every file, see [AGENTS.md](AGENTS.md) section "Codebase layout".

## Wire Protocol

Agent-controller communication uses WebSocket over TLS with JSON-serialized messages. The protocol defines three connection types:

| Connection type | Authentication | Purpose |
| --- | --- | --- |
| Anonymous | None | Initial enrollment request |
| Enrolled | Bearer token | Certificate request during enrollment |
| Authenticated | mTLS client certificate | Normal operation (heartbeat, commands, data) |

### Agent lifecycle

1. Agent connects anonymously and sends an `enroll` message with a one-time enrollment token.
2. Controller responds with `enrolled` (token-based auth for next step) or `rejected`.
3. Agent requests a client certificate via `request_certificate`.
4. Controller issues the certificate; agent reconnects with mTLS.
5. Normal operation: `ping`/`pong` heartbeats, status updates, version reports, update commands.

### Message types

Defined in `crates/shared/wire/`: `ping`, `pong`, `enroll`, `enrolled`, `approved`, `rejected`, `request_certificate`, `certificate`, `error`, and others.

For the full message schema with payloads, see the [AsyncAPI specification](crates/shared/wire/asyncapi.yaml).

## PKI & mTLS

The controller operates an internal Certificate Authority for mutual TLS authentication with agents.

- **CA rotation**: Automatic when the managed CA enters a 6-month expiry window. Produces a dual-CA trust bundle for seamless transition.
- **CRL partitioning**: Each CA signs a CRL only for certificates it issued (tracked via `ca_fingerprint`).
- **Runtime state**: CA material is shared via a `tokio::sync::watch` channel carrying a `CaSnapshot` struct.
- **External CA**: Supported via `--ca-cert` / `--ca-key` flags, which disable managed rotation.
- **SAN sanity checks**: At startup, `--san` values are validated against the existing managed cert's SANs. Mismatched SANs trigger silent regeneration (same CA) or an error with fix instructions (different CA). `--san` is incompatible with `--tls-cert`/`--tls-key`.

For cryptographic algorithm details, see [SECURITY.md](SECURITY.md) section "Cryptographic Details". For the full operational flow (rotation steps, bundle distribution, agent update path), see [AGENTS.md](AGENTS.md) section "PKI & CA rotation".

## DB-Managed Settings

Most runtime settings are stored in the database (`setting` entity) as JSON values. At startup, `Settings::load()` issues a single `SELECT * FROM settings` query and distributes the resulting `RawSettings` map to all sub-loaders and to reconciliation — no per-key DB reads. Any unrecognised keys in the DB trigger a warning log.

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
| Network | `network.*` | Proxies, header, SANs: yes; bind addresses: restart required | `GET/PUT /api/v1/settings/network` |
| MQTT | `mqtt.*` | No (all require restart) | `GET/PUT /api/v1/settings/mqtt` |
| Registration | `registration.*` | Yes | `GET/PUT /api/v1/settings/registration` |
| Authentication | `authentication.*` | Yes | `GET/PUT /api/v1/settings/authentication` |
| Agent certificates | `agent_certificates.*` | Yes | `GET/PUT /api/v1/settings/agent-certificates` |

**Not DB-managed** (bootstrap/infrastructure): `--data-dir`, `--db-url`, `--tls-cert`, `--tls-key`, `--ca-cert`, `--ca-key`, `--static-dir`.

The `Settings` struct (in `crates/ui/web-api/src/settings.rs`) uses `RwLock` for each settings group, allowing runtime-changeable settings to take effect immediately without restart. See [AGENTS.md](AGENTS.md) section "DB-managed settings" for the full key reference.

## Authentication & Authorization

### User authentication

- **Password**: Argon2id with OWASP-recommended parameters.
- **OIDC**: External identity providers with auto-create or account linking.
- **Device authorization**: RFC 8628-style flow for CLI login. The CLI requests a device code, the user approves in the browser, and the CLI receives an API token. Works with any auth method (password or OIDC).
- **Sessions**: SHA-256 hashed tokens, 7-day expiry, 30-min sliding window.
- **JWT**: Access and refresh tokens carrying resolved permissions.
- **API tokens**: Long-lived, revocable bearer tokens for programmatic access.

### Agent authentication

- **Enrollment**: One-time token, then mTLS client certificate issuance.
- **Normal operation**: mTLS on every WebSocket connection; CRL checked per connection.

### Authorization (RBAC)

Uses a typed `Permission` enum -- route handlers check `user.has_permission(Permission::...)`, never raw role strings. JWT tokens carry resolved permissions; the frontend receives permissions as `string[]`.

Roles: `admin` (all permissions) and `user` (`view_agents` only). First registered user gets `admin`.

For the full permissions table and instructions on adding new permissions, see [AGENTS.md](AGENTS.md) section "Permissions model".

## Database Design

SeaORM provides a multi-backend abstraction layer. The controller supports:

| Backend | Feature flag | Use case |
| --- | --- | --- |
| SQLite | `db-sqlite` (default) | Development, single-node deployments |
| PostgreSQL | `db-postgres` | Production, multi-node setups |
| MySQL | `db-mysql` | Alternative production backend |

### Entities

The data model comprises 17 entities in `crates/shared/db/src/entity/`:

`agent`, `agent_certificate`, `api_token`, `auth_method`, `oidc_provider`, `pending_account_link`, `pending_device_flow`, `pending_oidc_flow`, `pending_oidc_token_exchange`, `permission`, `role`, `role_permission`, `session`, `setting`, `user`, `user_oidc_link`, `user_role`

The `pending_*` entities store transient auth flow state (device authorization, OIDC login, account linking, token exchange). Persisting these to the database instead of in-memory maps enables controller restarts without losing active flows and supports HA multi-instance deployments with a shared database.

### Migrations

SeaORM migrations live alongside the entity definitions. They run automatically on controller startup, creating or updating the schema as needed.

## Provider Architecture

Providers define how software items are tracked and updated. Each provider splits into two sides:

| Concern | Runs on | Responsibility |
| --- | --- | --- |
| Remote/upstream version | Controller | Fetch latest version metadata from upstream sources |
| Local/installed version | Agent | Detect currently installed version on the host |
| Update execution | Agent | Run the update via sudo-allowlisted commands or custom scripts |

### Current providers

| Provider | Path | Description |
| --- | --- | --- |
| Provider Core | `crates/providers/core/` | Shared traits and abstractions |
| GitHub Releases | `crates/providers/github/` | Tracks GitHub release metadata; agent installs from artifacts |
| Proxmox Helper-Scripts | `crates/providers/proxmox-helper-scripts/` | Auto-discovers and manages PVE helper-script-installed apps |

The update step can always be overridden by a custom shell script, regardless of provider.

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

Each tracked software item is published as a Home Assistant `update` entity via MQTT auto-discovery (`crates/ui/mqtt/`). Entity attributes include installed version, latest version, changelog URL, release link, and more.

Updates can be triggered from Home Assistant, the Web UI, or the CLI -- all paths converge on the same controller API.

## Key Design Decisions

| Decision | Rationale |
| --- | --- |
| **No automatic updates** | Users must explicitly trigger updates. The scheduler only checks for new versions. This prevents unattended breakage in homelab environments. |
| **Outbound-only agents** | Agents connect to the controller; never the reverse. Simplifies firewall rules and eliminates the need for agents to expose network services. |
| **mTLS for agents** | Client certificates provide strong identity without shared secrets. The internal CA avoids dependency on external PKI infrastructure. |
| **Permission enum over role strings** | Typed permissions catch authorization bugs at compile time and make the permission model explicit in code. |
| **SeaORM multi-backend** | SQLite for development simplicity; PostgreSQL/MySQL for production. Feature flags keep the binary lean. |
| **rootcause + thiserror** | rootcause provides `Report`-based error propagation with structured context. thiserror generates the error enums. Together they enforce boundary-aware error handling without boilerplate. |
| **Rustls over OpenSSL** | Pure-Rust TLS avoids OpenSSL linking complexity and provides memory safety guarantees. aws-lc-rs backend offers FIPS-capable cryptography. |
| **SvelteKit static adapter** | No server-side rendering needed -- the controller serves the pre-built SPA. Keeps deployment simple (single binary + static files). |
| **MQTT for Home Assistant** | MQTT auto-discovery is the standard integration mechanism for Home Assistant. Native protocol avoids custom HA add-on complexity. |
| **Partitioned CRLs** | Each CA signs a CRL only for its own certificates. Prevents cross-CA revocation confusion during rotation periods. |
