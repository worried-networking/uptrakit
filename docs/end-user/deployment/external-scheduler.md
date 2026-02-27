# External Scheduler Deployment

The `uptrakit-scheduler` binary runs the scheduler as a standalone service, separate from the
controller. It enrolls as a service via WebSocket, receives infrastructure credentials automatically,
and executes scheduled tasks (version checks, cleanup, CA rotation checks, certificate renewal)
using direct database access and NATS messaging.

## When to use the external scheduler

| Deployment | Scheduler | Notes |
| --- | --- | --- |
| Single controller, simple | Embedded (`--features embedded-scheduler`) | Everything runs in one process |
| Single controller + NATS | External binary | Decoupled scheduling, independent scaling |
| Multi-controller HA | External binary | Avoids duplicate scheduling across controllers |
| Resilient single instance | Embedded + external | Embedded auto-disables when external connects; auto-re-enables on disconnect |

## Prerequisites

- A running Uptrakit controller with NATS configured (`--nats-url`).
- Network connectivity from the scheduler to the controller (WebSocket) and to the NATS server.
- Network connectivity from the scheduler to the database.
- The controller's master encryption key must be configured (the scheduler receives it via
  `ServiceCredentials` and uses it for database encryption/decryption).

## Installation

The `uptrakit-scheduler` binary is built from the `crates/core/scheduler/` crate:

```bash
cargo build -p uptrakit-scheduler --release
```

Database backend features mirror the controller:

```bash
# SQLite (default)
cargo build -p uptrakit-scheduler --release

# PostgreSQL
cargo build -p uptrakit-scheduler --release --features db-postgres

# All backends
cargo build -p uptrakit-scheduler --release --features db-all
```

## Enrollment

The scheduler enrolls like any other Uptrakit service (agent, MQTT bridge, SSH agent):

```bash
uptrakit-scheduler --url wss://controller.example.com:8443
```

On first run:

1. The scheduler connects to the controller via WebSocket.
2. It sends an `Enroll` message with capabilities:
   `scheduler`, `database_access`, `nats_access`, `master_key_access`, `ca_management`,
   `graceful_shutdown`.
3. An administrator approves the service (or provides an enrollment token for auto-approval).
4. The scheduler requests and receives a client certificate.
5. On the next connection (mTLS), the controller sends `ServiceCredentials` containing the
   database URL, NATS URL, and master encryption key.

To auto-approve during enrollment:

```bash
uptrakit-scheduler --url wss://controller.example.com:8443 --enrollment-token <token>
```

## CLI options

| Flag | Default | Description |
| --- | --- | --- |
| `--url` | (required) | Controller WebSocket URL |
| `--poll-interval-secs` | `15` | How often the scheduler polls for due tasks (seconds) |
| `--tofu` | — | Trust-on-first-use CA pinning |
| `--ca-cert` | — | Path to controller's CA certificate |
| `--pki-addr` | — | PKI endpoint for CA certificate |
| `--config-dir` | Platform default | Override config directory |
| `--state-dir` | Platform default | Override state directory |
| `--friendly-name` | Hostname | Human-readable service name |
| `--enrollment-token` | — | Auto-approval token |
| `--force-enroll` | — | Force fresh enrollment |
| `-v`, `--verbose` | 0 | Verbosity (up to `-vvv`) |
| `--version` | — | Print build info and exit |

## Credential flow

The scheduler advertises **credential capabilities** during enrollment. The controller automatically
sends `ServiceCredentials` on each authenticated connection:

| Capability | Credential | Description |
| --- | --- | --- |
| `database_access` | `db_url` | Database connection string |
| `nats_access` | `nats_url` | NATS server URL |
| `master_key_access` | `master_key_hex` | 256-bit master encryption key (hex) |

The `scheduler` and `ca_management` capabilities are markers — they do not grant credentials but
identify the service type and permissions.

## Security considerations

The external scheduler receives sensitive infrastructure credentials:

- **Database URL**: Grants full read/write access to the application database.
- **Master encryption key**: Can decrypt all encrypted fields in the database.
- **NATS URL**: Can publish messages to the NATS event stream.

When approving the scheduler service in the admin UI, security warnings are displayed for each
credential capability. Approve only trusted scheduler instances.

The `ServiceCredentials` message is **never** published to NATS — it is delivered exclusively via
the authenticated WebSocket connection. See
[Secrets and Encryption](../../security/secrets-and-encryption.md) for the credential filtering
rationale.

## Monitoring

- The scheduler's WebSocket connection to the controller serves as **presence detection**.
- When the `embedded-scheduler` feature is enabled on the controller, the embedded scheduler
  auto-disables when an external scheduler connects and auto-re-enables when it disconnects.
- The scheduler appears in the services list with the "Scheduler" label and can be managed via
  the REST API and admin UI.
- Logs use the `uptrakit_scheduler` tracing target. Use `-v` for debug, `-vv` for all uptrakit
  debug, `-vvv` for trace.

## Graceful shutdown

On `SIGTERM` or `SIGINT`, the scheduler:

1. Cancels the scheduler engine loop.
2. Releases all database claims (so another instance can pick up the tasks).
3. Sends a `Disconnecting` message to the controller.
4. Closes the WebSocket connection.

On `SIGHUP`, the scheduler exits with a restart outcome (suitable for systemd `Restart=on-failure`).

## Systemd service example

```ini
[Unit]
Description=Uptrakit Scheduler
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/usr/local/bin/uptrakit-scheduler --url wss://controller.example.com:8443
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

## Related documentation

- [Scheduler Architecture](../../architecture/scheduler.md) — database schema and HA claim mechanism
- [Scheduler Engine (Development)](../../development/scheduler-engine.md) — engine crate internals
- [NATS Deployment](nats.md) — NATS JetStream configuration
- [Secrets and Encryption](../../security/secrets-and-encryption.md) — credential security model
- [Service Lifecycle](../../development/service-lifecycle.md) — `ServiceHandler` trait
