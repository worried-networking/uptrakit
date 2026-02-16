# SSH-Backed Agent Architecture

The SSH-backed agent (`uptrakit-agent-ssh`) is a new service type that connects to the controller over WebSocket (like the regular agent) but will
execute version detection and updates on remote hosts over SSH instead of locally.

## Current Scope (Skeleton)

This document describes the initial skeleton implementation, which provides:

- A new `ServiceType::SshAgent` variant in the shared type system
- Controller-side enrollment and WebSocket dispatch for SSH agents
- A standalone binary (`uptrakit-agent-ssh`) with the `ServiceHandler` trait
- A local SQLite database for storing SSH host credentials (encrypted at rest)

The skeleton does **not** include SSH transport, provider execution over SSH, or UI configuration beyond the existing services API.

## Architecture Overview

```text
┌──────────────────┐         WSS (mTLS)        ┌────────────────────┐
│  uptrakit-agent-  │ ◄────────────────────────► │  uptrakit-         │
│  ssh              │                            │  controller        │
│                   │                            │                    │
│  ┌─────────────┐  │                            │  ┌──────────────┐  │
│  │ Local SQLite │  │                            │  │ Controller DB│  │
│  │ (ssh_hosts)  │  │                            │  │ (services)   │  │
│  └─────────────┘  │                            │  └──────────────┘  │
└──────────────────┘                            └────────────────────┘
```

The SSH agent follows the same enrollment and connection lifecycle as the regular agent and MQTT service, using the shared `ServiceHandler` trait from
`uptrakit-service-sdk`.

## Self-Managed Encryption

The SSH agent manages its own encryption key independently from the controller:

| Component | Master Key | Encrypts | |---|---|---| | Controller | Controller's master key (`UPTRAKIT_MASTER_KEY`) | CA keys, OIDC secrets, MQTT
passwords | | SSH Agent | SSH agent's master key (`UPTRAKIT_MASTER_KEY`) | SSH private keys in local SQLite |

Both use the same `init_master_key()` function from `uptrakit-shared-db::crypto` and the same `EncryptedString` type (AES-256-GCM), but with
independent keys. The controller has no knowledge of the SSH agent's master key.

### Master Key Options

The SSH agent supports the same master key configuration as the controller:

- `--master-key-file <path>` — path to a file containing a 64-character hex string
- `UPTRAKIT_MASTER_KEY` environment variable — 64-character hex string
- `--allow-plaintext-secrets` — development mode (disables encryption, logs a warning)

## Local Database Schema

The SSH agent uses a local SQLite database (`agent-ssh.db` in the state directory) with the following table:

### `ssh_hosts`

| Column | Type | Description | |---|---|---| | `id` | TEXT (UUID) | Primary key | | `name` | TEXT | Friendly name for the host | | `hostname` | TEXT
| SSH hostname or IP address | | `port` | INTEGER | SSH port (default: 22) | | `username` | TEXT | SSH username | | `private_key` | TEXT |
`EncryptedString` — SSH private key (AES-256-GCM) | | `key_type` | TEXT | Key algorithm: `ed25519` or `rsa` | | `host_key_fingerprint` | TEXT | Known
host key (SHA-256), nullable | | `created_at` | INTEGER | Unix timestamp | | `updated_at` | INTEGER | Unix timestamp |

## Service Type Registration

The `SshAgent` variant is registered in:

- `ServiceType` enum (`crates/shared/types/src/service_type.rs`) — serializes as `"ssh_agent"`
- `SettingKey::SshAgentEnrollmentTokenHash` — per-tenant enrollment token setting
- Controller dispatch (`crates/ui/web-api/src/routes/service_ws.rs`) — routes to `ssh_agent_ws`
- Connection registry (`crates/ui/web-api/src/service_connections.rs`) — `register_ssh_agent()`
- Services API (`crates/ui/web-api/src/routes/services.rs`) — enrollment token key resolution
- AsyncAPI spec (`crates/shared/wire/asyncapi.yaml`) — `ssh_agent` in ServiceType enum

## Crate Structure

```text
crates/core/agent-ssh/
├── Cargo.toml
├── build.rs
└── src/
    ├── main.rs          # SshAgentHandler (ServiceHandler impl), entry point, master key init
    ├── cli.rs           # CLI args (CommonServiceArgs + --master-key-file, --allow-plaintext-secrets)
    ├── client.rs        # Authenticated loop (ping/pong, cert renewal, local DB init)
    ├── error.rs         # Error types (rootcause + thiserror)
    └── db/
        ├── mod.rs       # SQLite init (init_db) + tests
        └── migration/
            ├── mod.rs   # Migration runner
            └── m20260215_000001_initial.rs  # ssh_hosts table
```

## Related Documentation

- [Service Lifecycle](../development/service-lifecycle.md) — `ServiceHandler` trait
- [SSH Agent Secrets](../security/ssh-agent-secrets.md) — secret storage and threat model
- [Wire Protocol](../api/wire-protocol.md) — `SshAgent` service type in enrollment
- [Services and Operations](../api/services-operations.md) — shared service management API
