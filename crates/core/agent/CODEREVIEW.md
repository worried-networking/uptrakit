# Code Review: `uptrakit-agent`

Reviewed: `src/main.rs` (120 lines), `src/client.rs` (649 lines),
`src/error.rs` (122 lines), `src/host_info.rs`, `src/update.rs`,
`src/version_check.rs`, `src/cli.rs`, `Cargo.toml`.

## Summary

The agent crate is well-structured with proper error delegation, graceful
shutdown handling, and bounded output buffers.

## Dependency Analysis

| Dependency | Purpose | Concern |
| --- | --- | --- |
| `uptrakit-provider-registry` | Instantiate providers from JSON config | Pulls in **all** provider implementation crates transitively |
| `uptrakit-service-sdk` | Service lifecycle, enrollment, TLS | Clean; appropriate for an agent binary |
| `uptrakit-internal-wire` | Wire protocol messages | Clean; required for controller communication |
| `uptrakit-command` | Local command execution | Clean; minimal dependency |
| `uptrakit-shared-types` | Shared enums and value types | Clean; leaf crate |

## Findings

### High

#### H1: Agent depends on `provider-registry` (ACCEPTED)

The agent binary depends on `uptrakit-provider-registry`, which transitively
pulls in every concrete provider crate (GitHub, Docker Registry, Proxmox
Helper Scripts, Homebrew). The agent needs the registry to instantiate
providers from JSON config for local version detection and update execution.
This is **architecturally justified** -- the agent must be able to run any
provider locally -- but it creates a large transitive dependency footprint
that couples the agent binary to all provider implementations at compile
time.

**Impact:** Adding a new provider crate increases the agent's compile time
and binary size, even if a given agent deployment never uses that provider.

**Resolution:** Accepted tradeoff: all agents compiled with all possible providers; adding a new provider only through the registry is acceptable.

### Info

#### I1: Bounded output buffer prevents OOM

**File:** `src/update.rs`

The update execution uses a bounded output buffer (10 MB) to prevent
runaway processes from exhausting memory. This is a good defensive practice.

#### I2: Concurrent version checks with `buffer_unordered(8)`

**File:** `src/client.rs:282-308`

Version checks are run concurrently with a concurrency limit of 8. The
pre-refresh step deduplicates by provider type to avoid redundant index
refreshes. Good design.

#### I3: Graceful shutdown drains in-flight updates

**File:** `src/client.rs:479-554`

The `handle_graceful_shutdown` function properly waits for in-flight updates
with a configurable timeout, drains remaining output messages, and sends the
`Disconnecting` message to the controller. Edge cases (timeout reached,
remaining output) are handled.

#### I4: Error classification tests are thorough

**File:** `src/error.rs:59-121`

Tests cover `is_receive_closed` and `is_cert_expired` across all error
variant paths, including negative cases (e.g., `UpdateExecution` containing
"CertificateExpired" in the string should not match).

#### I5: Clean SDK usage

Uses `service-sdk` for lifecycle management, enrollment, and TLS -- clean
separation. Command execution is abstracted via `CommandExecutor` trait
injection. No direct database dependency; the agent is stateless beyond its
service identity.
