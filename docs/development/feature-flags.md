# Feature Flags

This document catalogs the Cargo feature flags exposed by the controller and web-api crates,
their defaults, and what each one enables.

## Controller Feature Flags

| Feature                  | Default |
| ------------------------ | ------- |
| `db-sqlite`              | Yes     |
| `db-postgres`            | No      |
| `db-all`                 | No      |
| `oidc`                   | Yes     |
| `embedded-scheduler`     | Yes     |
| `embedded-agent`         | No      |
| `embedded-ssh-agent`     | No      |
| `nats`                   | No      |
| `swagger-ui`             | No      |
| `embed-frontend`         | Yes     |
| `notifications-all`      | Yes     |
| `notifications-telegram` | No      |
| `notifications-email`    | No      |
| `interactive`            | Yes     |
| `zeroconf`               | Yes     |
| `dashboard-icons`        | Yes     |
| `reset-data`             | Yes     |

### `db-sqlite`

SQLite backend.

### `db-postgres`

PostgreSQL backend.

### `db-all`

All database backends (SQLite + PostgreSQL).

### `oidc`

OpenID Connect authentication support. Disabling removes the `openidconnect` crate and all OIDC
routes/stores, significantly reducing compile-time dependencies. Propagates to
`uptrakit-web-api/oidc`.

### `embedded-scheduler`

Embeds the scheduler engine in the controller process via `EmbeddedServiceHost::add()`. Uses
`CoexistencePolicy::YieldAlways` to defer external tasks when an external scheduler connects;
internal tasks (CRL renewal, CA rotation, service cert check) always run. The yield check queries
`EmbeddedServiceNotifier::is_capability_yielded(Scheduler)`. Adds the `uptrakit-scheduler-engine`
dependency.

### `embedded-agent`

Embeds a local agent in the controller process via `EmbeddedServiceHost::add()` for single-tenant
deployments. Manages the controller host (software discovery, version checks, updates). Uses
`CoexistencePolicy::YieldAlways` with a custom `yield_check` that yields only when an external
agent reports the same `machine_id`. Provisioned as a tenant service (not system) under
`AppState.default_tenant_id`. Reuses all business logic from `uptrakit-agent-core`. Propagates the
`interactive` feature as `uptrakit-agent-core?/interactive`. Freeze file at
`<state_dir>/embedded-agent/update-freeze`. Rate limits updates to a 5-second cooldown.

### `embedded-ssh-agent`

Embeds the SSH-backed agent in the controller process via `EmbeddedServiceHost::add()` for
single-tenant deployments that manage remote hosts over SSH. Uses
`CoexistencePolicy::YieldOnSameAppName` — yields when an external `uptrakit-agent-ssh` connects.
Provisioned as a tenant service under `AppState.default_tenant_id`. Reuses all business logic from
the `uptrakit-agent-ssh-runtime` library crate; the controller dispatches via
`uptrakit_service_sdk::run_embedded_service<AgentSshHandler>`. Uses the controller's shared
database; SSH migrations are contributed at startup through `AgentSshHandler::service_migrations()`.
Ephemeral ECIES P-256 keypair for surface parameter decryption (via
`uptrakit_service_sdk::generate_p256_keypair_for_ecies`). Propagates `interactive` as
`uptrakit-agent-ssh-runtime?/interactive` and `reset-data` as
`uptrakit-agent-ssh-runtime?/reset-data`. Freeze file at
`<state_dir>/embedded-ssh-agent/update-freeze`. Rate limits updates to a 5-second cooldown.

### `nats`

Enables NATS JetStream transport for cross-controller messaging. Propagates to
`uptrakit-web-api/nats`.

### `swagger-ui`

Swagger UI at `/swagger-ui`.

### `embed-frontend`

Embeds the SvelteKit frontend build into the binary via `rust-embed`. Requires `frontend/build/`
to exist at compile time. Removes the `--static-dir` CLI argument. See
[Embedded Frontend](embedded-frontend.md).

### `notifications-all`

Enables all optional notification plugins (Telegram, email). Expands to `notifications-telegram`,
`notifications-email`, and `uptrakit-web-api/notifications-all`.

### `notifications-telegram`

Telegram notification plugin (enabled transitively via `notifications-all`). Propagates to
`uptrakit-web-api/notifications-telegram`.

### `notifications-email`

Email notification plugin via SMTP (enabled transitively via `notifications-all`). Propagates to
`uptrakit-web-api/notifications-email`.

### `interactive` (controller)

Interactive (PTY-based) update sessions with stdin forwarding. Propagates to
`uptrakit-web-api/interactive`. Adds the interactive WebSocket endpoint and
`InteractiveSessionRegistry`. See [Interactive Updates](interactive-updates.md).

### `zeroconf`

mDNS/DNS-SD zero-configuration advertising. Enables the `--zeroconf` CLI flag and the advertiser
module. Uses the `mdns-sd` crate. See [Zeroconf Discovery](zeroconf-discovery.md).

### `dashboard-icons`

Dashboard Icons enhancement plugin. Automatically assigns icon URLs to software items from the
[Dashboard Icons](https://github.com/homarr-labs/dashboard-icons) project. Propagates to
`uptrakit-web-api/dashboard-icons` + `uptrakit-plugin-infrastructure-registry/dashboard-icons`.
Uses tenant-scoped plugin type settings on `enhancement_dashboard_icons`; `enabled` defaults to
`true` when unset. See [Dashboard Icons](dashboard-icons.md).

### `reset-data` (controller)

Destructive data reset endpoint (`POST /api/v1/settings/reset-data`). Propagates to
`uptrakit-web-api/reset-data`. Requires `CanManageGlobalSettings` permission and `confirm: "RESET"`
in the request body. Broadcasts `ResetData` to connected services after clearing tenant data.

## Web-API Feature Flags

| Feature       | Default |
| ------------- | ------- |
| `oidc`        | Yes     |
| `swagger-ui`  | No      |
| `db-sqlite`   | No      |
| `db-postgres` | No      |
| `db-all`      | No      |
| `interactive` | No      |
| `reset-data`  | No      |

### `oidc` (web-api)

OpenID Connect authentication. Propagates to `uptrakit-web-api-auth/oidc`. Gates the
`openidconnect` dependency and all OIDC-specific modules (`oidc_auth`, `oidc_providers`,
`oidc_state`), routes, OpenAPI schemas, rate limit entries, and `AppState` stores. Non-OIDC types
(`AuthMethod::Oidc`, `require_token_for_oidc`, OIDC DB entities) remain unconditional.

### `swagger-ui` (web-api)

Swagger UI at `/swagger-ui`.

### `db-sqlite` (web-api)

SQLite backend. Propagates to `uptrakit-web-api-queries/db-sqlite`.

### `db-postgres` (web-api)

PostgreSQL backend. Propagates to `uptrakit-web-api-queries/db-postgres`.

### `db-all` (web-api)

All database backends (SQLite + PostgreSQL). Propagates to `uptrakit-web-api-queries/db-all`.

### `interactive` (web-api)

Interactive update WebSocket endpoint (`/api/v1/update-history/{id}/interactive`),
`InteractiveSessionRegistry`. Propagates to `uptrakit-command/interactive` via
`uptrakit-agent-core`.

### `reset-data` (web-api)

Registers the `POST /api/v1/settings/reset-data` route and the transactional data-reset query
logic. No additional dependencies.

## Cross-References

- [Coding Standards § Feature Flags](coding-standards.md#feature-flags) — the additive-only rule:
  feature flags must never subtract functionality via `#[cfg(not(feature = "X"))]`; use `cfg!()`
  in expression position instead.
