# AGENTS.md — AI Agent Guide for Uptrakit

This file provides structured context for AI coding agents working on the Uptrakit codebase. Read this first before making any changes.

## Project summary

Uptrakit is an agent-based update tracking toolkit for self-hosted Linux environments. It tracks installed software versions across remote hosts, checks for updates, and allows **manual, user-triggered** updates. It is **not** an auto-updater.

Key components:

- **Controller** (server): API, Web UI, scheduler, MQTT/Home Assistant integration, remote provider logic.
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
│   │   └── controller/                 # uptrakit-controller                    (bin)  — central server
│   ├── providers/
│   │   ├── core/                       # uptrakit-provider-core                 (lib)  — provider trait/abstractions
│   │   ├── github/                     # uptrakit-provider-github               (lib)  — GitHub Releases provider
│   │   └── proxmox-helper-scripts/     # uptrakit-provider-proxmox-helper-scripts (lib) — PVE helper-scripts provider
│   ├── shared/
│   │   ├── core/                       # uptrakit-core                          (lib)  — shared domain models
│   │   ├── db/                         # uptrakit-shared-db                     (lib)  — SeaORM entities & migrations
│   │   └── wire/                       # uptrakit-internal-wire                 (lib)  — agent<->controller wire protocol
│   └── ui/
│       ├── cli/                        # uptrakit-cli                           (bin)  — CLI interface
│       ├── mqtt/                       # uptrakit-mqtt                          (lib)  — MQTT / Home Assistant integration
│       └── web-api/                    # uptrakit-web-api                       (lib)  — HTTP API
├── frontend/                           # SvelteKit SPA (Skeleton UI + Tailwind CSS)
│   ├── src/
│   │   ├── lib/                        # Shared modules: api client, auth store, types
│   │   └── routes/                     # SvelteKit file-based routes
│   ├── package.json                    # npm scripts: build, check
│   ├── svelte.config.js                # SvelteKit config (static adapter)
│   ├── tailwind.config.ts              # Tailwind + Skeleton theme
│   └── vite.config.ts                  # Vite config (dev proxy → controller)
├── .github/
│   ├── workflows/ci.yml                # CI: fmt check, clippy, tests, frontend check + build
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
```

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

1. Background task checks every 24 hours whether the active CA enters the 6-month rotation window.
2. On rotation: current CA files move to `ca-previous.{crt,key}`, a new CA is generated as `ca.{crt,key}`.
3. Both CAs form a trust bundle (`bundle_pem`). The controller trusts client certs signed by either CA.
4. CRLs are partitioned: each CA signs a CRL only for certificates it issued (tracked via `ca_fingerprint` column in `agent_certificates`).
5. Connected agents receive a `CaBundleUpdated` WebSocket message with the new bundle PEM.
6. Agents that were offline detect staleness via `ca_bundle_hash` in `AgentSettings` and fetch the updated bundle over HTTP.
7. New agent certs are always signed by the active CA.

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
2. **Agents initiate outbound-only connections.** Agents connect to the controller via secure WebSocket (`/api/v1/ws/agent`). They never listen on any port or accept inbound connections.
3. **Agents run unprivileged.** They run as a dedicated user (e.g. `uptrakit`). Only specific update commands are granted `NOPASSWD` sudo access.
4. **Provider split.** Remote (upstream version resolution) logic runs on the controller. Local (installed version detection + update execution) logic runs on the agent. Keep this boundary clear.
5. **No shell injection.** Any path that constructs or executes shell commands must validate inputs. Custom scripts are treated as untrusted input.
6. **No secrets in logs.** Never log tokens, passwords, API keys, or other credentials.
7. **Logging goes to journald or stdout.** No internal log storage. Full command output is not captured internally — only high-level summaries are retained for display.
8. **No overlapping update actions per host.** The scheduler must ensure that two update operations for the same host never run concurrently.
9. **No raw SQL.** Use the structures and methods provided by Sea ORM eveywhere.
10. **Cover new logic with tests.** Cover success and failure paths.
11. **Document everything.**  Any code change must be properly documented either in the code, or in the separate documentation. Any changes to the agent-controller wire protocol must be documented in `crates/shared/wire/asyncapi.yaml`.
12. **Do not add any `allow()`** without excpicit approval from the user.
13. **Do not use `unsafe`, `unwrap` or `panic!`.** Always prefer safe and graceful solutions. See the "Error handling" section in [CONTRIBUTING.md](CONTRIBUTING.md) for approved patterns (match with fallback, serialization helpers). **Approved exceptions**: `Mutex::lock().unwrap()`, `RwLock::read().unwrap()`, and `RwLock::write().unwrap()` are safe because `panic = "abort"` in the release profile makes lock poisoning impossible.

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
- **Database-backed store**: all pending device flow state is persisted to the `pending_device_flows` table (shared with OIDC flow, account link, and token exchange stores). Survives controller restarts and supports HA multi-instance deployments. Only the resulting API token is persisted to the `api_tokens` table.

## Permissions model

Authorization uses a typed `Permission` enum (`crates/ui/web-api/src/auth/permissions.rs`) rather than raw role-name strings. The enum variants are:

| Permission | Serialized name | Purpose |
| --- | --- | --- |
| `ViewSettings` | `view_settings` | Read settings, OIDC providers, auth config |
| `ManageSettings` | `manage_settings` | Modify settings, OIDC providers, auth config |
| `ViewAgents` | `view_agents` | List agents |
| `ManageAgents` | `manage_agents` | Approve, reject, delete, merge agents; manage enrollment tokens |

### Roles

| Role | Permissions |
| --- | --- |
| `admin` | All four |
| `user` | `view_agents` only |

The first registered user gets the `admin` role. Subsequent users (password or OIDC auto-created) get the `user` role by default. OIDC role mapping can override this.

### How it works

1. `get_user_permissions()` (`routes/auth.rs`) resolves a user's permissions: user → user_roles → role_permissions → permissions table.
2. The resolved `Vec<Permission>` is embedded in the JWT access token (`permissions` claim) and returned in `UserResponse.permissions`.
3. The `require_auth` middleware injects `AuthenticatedUser` with the `permissions` field decoded from the JWT.
4. Route handlers call `user.has_permission(Permission::...)` — no DB round-trip needed.
5. The frontend receives permissions as `string[]` (e.g. `["view_settings", "manage_agents"]`) and uses the `Permission` TypeScript enum for checks.

### Adding a new permission

1. Add a variant to the `Permission` enum in `permissions.rs` (with `as_str` / `parse` arms).
2. Write a DB migration to insert it into the `permissions` table and assign it to the appropriate roles.
3. Add the check in the relevant route handler(s).
4. Add the variant to the `Permission` TypeScript enum in `frontend/src/lib/types.ts`.

## DB-managed settings

Most CLI arguments are reconciled with DB-persisted values at startup. The reconciliation module (`crates/core/controller/src/reconcile.rs`) implements a generic 5-case priority logic. Settings are stored in the `setting` DB entity as JSON values.

### Settings reference

| CLI flag | DB key | Default | Runtime-changeable |
| --- | --- | --- | --- |
| `--trusted-proxy` | `network.trusted_proxies` | `[]` | Yes |
| `--real-ip-header` | `network.real_ip_header` | `X-Forwarded-For` | Yes |
| `--san` | `network.extra_sans` | `[]` | Yes |
| `--http-addr` | `network.http_addr` | `[::]:8080` | No (restart) |
| `--https-addr` | `network.https_addr` | `[::]:8443` | No (restart) |
| `--mqtt-host` | `mqtt.host` | `null` | No (restart) |
| `--mqtt-port` | `mqtt.port` | `1883` | No (restart) |
| `--mqtt-client-id` | `mqtt.client_id` | `uptrakit-controller` | No (restart) |
| `--mqtt-username` | `mqtt.username` | `null` | No (restart) |
| `--mqtt-password` | `mqtt.password` | `null` | No (restart) |
| `--mqtt-topic-prefix` | `mqtt.topic_prefix` | `uptrakit` | No (restart) |

**Not DB-managed** (bootstrap/infrastructure): `--data-dir`, `--db-url`, `--tls-cert`, `--tls-key`, `--ca-cert`, `--ca-key`, `--static-dir`.

### Reconciliation logic

For each DB-managed setting at startup:
1. DB has value + CLI provided + differs + `--force-settings-override` → use CLI, update DB
2. DB has value + CLI provided + differs + no force → use DB, log warning
3. DB has value + (CLI absent or same) → use DB
4. No DB value + CLI provided → use CLI, save to DB
5. No DB value + CLI absent → use hardcoded default, save to DB

### In-memory settings

The `Settings` struct (`crates/ui/web-api/src/settings.rs`) holds `NetworkSettings` and `MqttSettings` behind `RwLock`s. Runtime-changeable fields (proxies, header, SANs) are updated in-memory immediately when changed via the API. Restart-required fields (addresses, MQTT) are saved to DB only.

### Settings API endpoints

| Endpoint | Permission | Purpose |
| --- | --- | --- |
| `GET /api/v1/settings/network` | ViewSettings | Read network settings |
| `PUT /api/v1/settings/network` | ManageSettings | Update network settings |
| `GET /api/v1/settings/mqtt` | ViewSettings | Read MQTT settings |
| `PUT /api/v1/settings/mqtt` | ManageSettings | Update MQTT settings |

MQTT password is never exposed in API responses; a `has_password: bool` field indicates whether one is set.

## Error handling

Use [`rootcause`](https://github.com/rootcause-rs/rootcause) for error propagation and [`thiserror`](https://github.com/dtolnay/thiserror) for error enum definition. Every module boundary must define its own error type following the patterns below.

### Pattern 1: Define an error enum with a `Result<T>` alias

Each boundary (crate, module, or logical subsystem) defines its own error enum and a `Result` alias using `Report`:

```rust
use rootcause::{Report, ReportConversion, markers};
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

When your module calls code that returns a different error type, implement `ReportConversion` so that `.context_to()` can convert automatically:

```rust
impl<T> ReportConversion<sea_orm::DbErr, markers::Mutable, T> for MyError
where
    MyError: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<sea_orm::DbErr, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(MyError::Database)
    }
}
```

For errors that don't map directly via `#[from]`, use a closure:

```rust
report.context_transform(|_| MyError::Internal("unexpected failure".to_string()))
```

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
6. **Implement `ReportConversion`** for every foreign error type your boundary may encounter.

## Testing expectations

Every behaviour change must include tests. Types of tests used:

- **Unit tests**: pure logic, version comparison, parsing.
- **Provider tests**: parsing upstream metadata, mapping to internal models.
- **API boundary tests**: request/response (de)serialisation, backwards compatibility.
- **Error path tests**: expected failures produce correct error types and messages.

Run tests with:

```sh
cargo test --all-features
# or with nextest:
cargo nextest run --all-features
```

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
| `uptrakit-provider-core` | `crates/providers/core/` | Shared provider traits and abstractions |
| `uptrakit-provider-github` | `crates/providers/github/` | GitHub Releases: controller fetches release metadata; agent installs from artifacts |
| `uptrakit-provider-proxmox-helper-scripts` | `crates/providers/proxmox-helper-scripts/` | Proxmox VE Helper-Scripts: agent auto-discovers and manages helper-script-installed apps |

The update step can always be overridden by a custom shell script, regardless of provider.

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
