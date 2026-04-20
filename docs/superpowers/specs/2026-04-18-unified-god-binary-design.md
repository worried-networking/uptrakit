# Unified God Binary

## Summary

Add a `unified` feature flag to `uptrakit-cli` that compiles all service runtimes into a single distributable binary. Service dispatch uses early
`argv[1]` inspection before any clap parsing — each service gets its own full clap parser and tracing initialization. Individual service binaries
remain buildable independently.

## Goals

- Single binary distribution for environments where deploying multiple binaries is impractical
- Backward-compatible: existing CLI invocations unchanged
- Individual service binaries unaffected
- Controller's `embedded-*` feature flags remain orthogonal

## Non-Goals

- Busybox-style `argv[0]` dispatch (symlinks)
- Replacing the `embedded-*` pattern on the controller
- Full refactor of controller internals (only lib+bin split required)

## Design

### Dispatch Model

```text
uptrakit <cli-command> [args]     # CLI client (default, backward-compatible)
uptrakit controller [args]        # run controller
uptrakit agent [args]             # run agent daemon
uptrakit agent-ssh [args]         # run agent-ssh daemon/CLI
uptrakit mqtt [args]              # run MQTT service
uptrakit scheduler [args]         # run scheduler daemon
```

Dispatch is via early `argv[1]` inspection, before any clap parsing. If `argv[1]` matches a reserved service name, strip it and invoke that service's
`run_from_args()` with the remaining `argv`. Each service uses its own full clap parser and tracing initialization.

CLI commands are parsed only when no service name matches. This avoids:

- **Name collisions**: `scheduler` and `services` already exist as CLI `Commands` variants; they retain their meaning unchanged.
- **Global flag conflicts**: `-v/--verbose` and `--version` differ in meaning between the CLI client and services; each parser sees only its own
  flags.
- **Tracing mismatch**: services initialize service tracing; CLI initializes client tracing. These are separate code paths.

`uptrakit --help` describes the dispatch model and lists service subcommand names with a brief description via a custom help section. Full per-service
flag help is available via `uptrakit <service> --help`.

### Runtime Crate Contract

Each service exposes from its runtime crate:

1. **`pub async fn run_from_args(args: Vec<OsString>) -> ExitCode`** — parses argv, initializes service tracing, runs full service lifecycle. Returns
   `ExitCode` (not `Result<()>`) to preserve service-specific exit behavior including subcommand dispatch (e.g. `db-migrate`, `host add`).

Service binaries become thin wrappers:

```rust
fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().collect();
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
        .block_on(xxx_runtime::run_from_args(args))
}
```

God binary dispatch:

```rust
fn main() -> ExitCode {
    let mut args: Vec<OsString> = std::env::args_os().collect();

    #[cfg(feature = "unified")]
    {
        let subcmd = args.get(1).and_then(|s| s.to_str()).unwrap_or("");
        match subcmd {
            "controller" => {
                args.remove(1);
                return tokio_runtime().block_on(
                    uptrakit_controller_runtime::run_from_args(args)
                );
            }
            "agent" => { /* ... */ }
            "agent-ssh" => { /* ... */ }
            "mqtt" => { /* ... */ }
            "scheduler" => { /* ... */ }
            _ => {}
        }
    }

    // Fall through to CLI client parsing
    tokio_runtime().block_on(cli_main(args))
}
```

### New Crate: `uptrakit-controller-runtime`

**Location:** `crates/core/controller-runtime/`

**Prerequisite:** `uptrakit-controller` must be converted from a bin-only crate to a lib+bin crate. Its bootstrap logic moves to `src/lib.rs` (public
API). `src/main.rs` becomes a thin wrapper. This is required before `controller-runtime` can depend on it.

`uptrakit-controller-runtime` then provides:

- `pub async fn run_from_args(args: Vec<OsString>) -> ExitCode` — parses `ControllerArgs` (including the `db-migrate` subcommand), initializes service
  tracing, calls controller lib bootstrap.
- Feature flags mirroring all of controller's own features.

### Agent-SSH: Full Library Merge into `agent-ssh-runtime`

`uptrakit-agent-ssh-runtime` exposes `SshAgentRuntime<S>` parameterized on `SshAgentRuntimeSupport` trait. The concrete `AgentSshRuntimeSupport`
implementation and all SSH primitives live in `uptrakit-agent-ssh` lib, with deep `crate::` internal dependencies throughout (`host_ops`,
`operations`, `surface_runtime`, `db`, `ssh_pool`, `client`, etc.).

**Cycle problem:** `uptrakit-agent-ssh` binary depends on `uptrakit-agent-ssh-runtime`. A reverse dep creates a Cargo cycle.

**The symbols controller imports from `uptrakit-agent-ssh`** all have transitive `crate::` deps on the rest of the lib:

- `AgentSshRuntimeSupport` → `db`, `host_ops`, `operations`, `ssh_pool`
- `surface_runtime::build_surface_registration` → `host_ops`, `operations::bootstrap`, `operations::sync`, `ssh_target`, `client`

Lifting these 6 symbols in isolation is not mechanically viable — their transitive deps must come with them. The correct approach is a **full merge**:
move the entire `uptrakit-agent-ssh` library into `uptrakit-agent-ssh-runtime` behind a new `standalone` feature.

**Solution:**

- Create `standalone` feature in `uptrakit-agent-ssh-runtime/Cargo.toml` (does not currently exist — must be added)
- Move all modules from `uptrakit-agent-ssh/src/` (except `main.rs`) into `uptrakit-agent-ssh-runtime/src/` under `standalone`: this includes
  `runtime_support`, `surface_runtime`, `host_ops`, `operations`, `db`, `ssh_pool`, `client`, `ssh_target`, `ssh_key`, `ssh_executor`,
  `ssh_transport`, `ssh_stdio_tunnel`, `host_info`, `host_cli`, `cli`, `error`, `commands/`, `remote_exec`, `ssh_pool`, etc.
- `uptrakit-agent-ssh/src/lib.rs` becomes empty or re-exports from `uptrakit-agent-ssh-runtime` (for any external consumers, if any)
- `uptrakit-agent-ssh/src/main.rs` becomes thin wrapper calling `uptrakit_agent_ssh_runtime::run_from_args(args)`

**Controller update:**

Controller already imports `SshAgentIdentity`, `SshAgentRuntime`, `SshAgentRuntimeConfig`, `SshAgentSettings`, and `ssh_agent_capabilities` from
`uptrakit-agent-ssh-runtime` — that dep already exists. After the merge, add `standalone` feature to that dep under `embedded-ssh-agent`, and switch
the remaining imports:

```rust
// Before (in ssh_agent/mod.rs and main.rs):
use uptrakit_agent_ssh::runtime_support::AgentSshRuntimeSupport;
use uptrakit_agent_ssh::{ServiceSurfaceProxy, reencrypt_ssh_to_v3,
    register_ssh_column_aad, ssh_pool};
use uptrakit_agent_ssh::surface_runtime::build_surface_registration;

// After:
use uptrakit_agent_ssh_runtime::AgentSshRuntimeSupport;
use uptrakit_agent_ssh_runtime::{ServiceSurfaceProxy, reencrypt_ssh_to_v3,
    register_ssh_column_aad, ssh_pool};
use uptrakit_agent_ssh_runtime::surface_runtime::build_surface_registration;
```

Remove `uptrakit-agent-ssh` from controller's deps under `embedded-ssh-agent`. No cycle: `agent-ssh-runtime/standalone` has no dependency on the
`uptrakit-agent-ssh` package.

This is the largest structural change in the spec: a full library merge. Behavior is unchanged — it is a crate boundary reorganization only.

### Scheduler-Runtime: `standalone` Feature Required

`uptrakit-scheduler-runtime` already has a `standalone` feature containing `StandaloneSchedulerHandler` and `ServiceHandler` impl. Unified binary must
activate this feature:

```toml
# in uptrakit-cli/Cargo.toml
uptrakit-scheduler-runtime = { ..., optional = true }
```

```toml
unified = [
    # ...
    "uptrakit-scheduler-runtime?/standalone",
]
```

### Zeroconf and Interactive: Service-SDK Level

`uptrakit-agent-runtime`, `uptrakit-mqtt-runtime`, and `uptrakit-scheduler-runtime` have no `zeroconf` feature. Zeroconf is a `uptrakit-service-sdk`
feature activated by each binary's `Cargo.toml`. In the unified binary, zeroconf for all services is controlled by a single service-sdk feature:

```toml
# uptrakit-cli/Cargo.toml
unified-zeroconf = ["uptrakit-service-sdk?/zeroconf"]
unified-interactive = [
    "uptrakit-agent-runtime?/interactive",
    "uptrakit-agent-ssh-runtime?/standalone-interactive",  # if applicable
]
```

`agent-runtime` and `agent-ssh-runtime` do have `interactive` features (they control PTY/forwarder logic, not just service-sdk). Those are
individually proxied. Zeroconf is global — one flag enables it for all services simultaneously (acceptable tradeoff for unified binary).

### Runtime Crates: `service-sdk` Dependency

`uptrakit-agent-runtime` and `uptrakit-agent-ssh-runtime` currently have `uptrakit-service-sdk` only in `[dev-dependencies]`. `run_from_args` needs
service-sdk for tracing init, lifecycle, and crypto — production code. Both crates must add `uptrakit-service-sdk` as a real (non-dev) dependency,
optionally activated by a feature (`standalone` in the agent-ssh-runtime case, or unconditionally in agent-runtime since the full service lifecycle is
always needed).

### Existing Runtime Crate Changes

| Crate                        | Add                                                                                              |
| ---------------------------- | ------------------------------------------------------------------------------------------------ |
| `uptrakit-agent-runtime`     | `uptrakit-service-sdk` as prod dep; `run_from_args(Vec<OsString>) -> ExitCode`                   |
| `uptrakit-agent-ssh-runtime` | `standalone` feature + `uptrakit-service-sdk` under it; all SSH impl moved here; `run_from_args` |
| `uptrakit-mqtt-runtime`      | `pub async fn run_from_args(Vec<OsString>) -> ExitCode`                                          |
| `uptrakit-scheduler-runtime` | `pub async fn run_from_args(Vec<OsString>) -> ExitCode` (behind existing `standalone`)           |

Each service binary's `main.rs` thins to call its crate's `run_from_args`. All existing `std::process::exit()` callsites inside those `main.rs` files
must be replaced with `return ExitCode::FAILURE` so the god binary process is not killed without cleanup during service dispatch.

### Tracing Initialization

Each `run_from_args` implementation is responsible for initializing its own tracing subscriber before any service logic runs. The CLI client path
initializes its own client-focused tracing subscriber when no service name matches. These are fully separate code paths — no shared tracing setup in
the god binary `main`.

### Feature Flag Design

`uptrakit-cli/Cargo.toml`:

```toml
[dependencies]
uptrakit-controller-runtime = { workspace = true, optional = true, default-features = false }
uptrakit-agent-runtime = { workspace = true, optional = true }
uptrakit-agent-ssh-runtime = { workspace = true, optional = true, default-features = false }
uptrakit-mqtt-runtime = { workspace = true, optional = true }
uptrakit-scheduler-runtime = { workspace = true, optional = true, default-features = false }
uptrakit-service-sdk = { workspace = true, optional = true }  # for unified-zeroconf only

[features]
unified = [
    "dep:uptrakit-controller-runtime",
    "dep:uptrakit-agent-runtime",
    "dep:uptrakit-agent-ssh-runtime",
    "dep:uptrakit-mqtt-runtime",
    "dep:uptrakit-scheduler-runtime",
    "uptrakit-scheduler-runtime?/standalone",
]

# Controller features (prefixed, optional-dep gated)
controller-db-postgres = ["uptrakit-controller-runtime?/db-postgres"]
controller-db-all = ["uptrakit-controller-runtime?/db-all"]
controller-db-sqlite = ["uptrakit-controller-runtime?/db-sqlite"]
controller-oidc = ["uptrakit-controller-runtime?/oidc"]
controller-embedded-scheduler = ["uptrakit-controller-runtime?/embedded-scheduler"]
controller-embedded-agent = ["uptrakit-controller-runtime?/embedded-agent"]
controller-embedded-ssh-agent = ["uptrakit-controller-runtime?/embedded-ssh-agent"]
controller-embedded-mqtt = ["uptrakit-controller-runtime?/embedded-mqtt"]
controller-embed-frontend = ["uptrakit-controller-runtime?/embed-frontend"]
controller-nats = ["uptrakit-controller-runtime?/nats"]
controller-notifications-telegram = ["uptrakit-controller-runtime?/notifications-telegram"]
controller-notifications-email = ["uptrakit-controller-runtime?/notifications-email"]
controller-notifications-all = ["uptrakit-controller-runtime?/notifications-all"]
controller-journald = ["uptrakit-controller-runtime?/journald"]
controller-swagger-ui = ["uptrakit-controller-runtime?/swagger-ui"]
controller-reset-data = ["uptrakit-controller-runtime?/reset-data"]
controller-dashboard-icons = ["uptrakit-controller-runtime?/dashboard-icons"]
controller-zeroconf = ["uptrakit-controller-runtime?/zeroconf"]
controller-interactive = ["uptrakit-controller-runtime?/interactive"]

# Agent features
agent-interactive = ["uptrakit-agent-runtime?/interactive"]

# Agent-SSH features
agent-ssh-interactive = ["uptrakit-agent-ssh-runtime?/standalone-interactive"]
agent-ssh-reset-data = ["uptrakit-agent-ssh-runtime?/standalone-reset-data"]

# Scheduler features
scheduler-db-postgres = ["uptrakit-scheduler-runtime?/db-postgres"]

# Cross-service (service-sdk level)
# Note: uptrakit-service-sdk must be declared as optional dep in cli Cargo.toml
# even though CLI doesn't directly use it; required for the ?/feature syntax to work.
unified-zeroconf = ["dep:uptrakit-service-sdk", "uptrakit-service-sdk/zeroconf"]
```

### `controller-runtime` Feature Flags

`uptrakit-controller-runtime/Cargo.toml` has no defaults (all explicit):

```toml
[features]
default = []   # No defaults; caller picks features explicitly

db-sqlite = ["uptrakit-controller/db-sqlite"]
db-postgres = ["uptrakit-controller/db-postgres"]
db-all = ["uptrakit-controller/db-all"]
oidc = ["uptrakit-controller/oidc"]
embedded-scheduler = ["uptrakit-controller/embedded-scheduler"]
embedded-agent = ["uptrakit-controller/embedded-agent"]
embedded-ssh-agent = ["uptrakit-controller/embedded-ssh-agent"]
embedded-mqtt = ["uptrakit-controller/embedded-mqtt"]
embed-frontend = ["uptrakit-controller/embed-frontend"]
nats = ["uptrakit-controller/nats"]
notifications-telegram = ["uptrakit-controller/notifications-telegram"]
notifications-email = ["uptrakit-controller/notifications-email"]
notifications-all = ["uptrakit-controller/notifications-all"]
journald = ["uptrakit-controller/journald"]
swagger-ui = ["uptrakit-controller/swagger-ui"]
reset-data = ["uptrakit-controller/reset-data"]
dashboard-icons = ["uptrakit-controller/dashboard-icons"]
zeroconf = ["uptrakit-controller/zeroconf"]
interactive = ["uptrakit-controller/interactive"]
```

### Docker

No Dockerfile changes. Existing parameterized build works:

```bash
docker build -f docker/Dockerfile \
  --build-arg PACKAGE=uptrakit-cli \
  --build-arg BINARY=uptrakit \
  --build-arg FEATURES=unified,controller-embed-frontend,controller-db-all,controller-oidc,controller-zeroconf,controller-interactive,controller-embedded-scheduler,controller-nats,controller-notifications-all,unified-zeroconf,agent-interactive,agent-ssh-interactive,scheduler-db-postgres \
  -t uptrakit-unified .
```

### Reserved Name Handling

Reserved service names: `controller`, `agent`, `agent-ssh`, `mqtt`, `scheduler`. These are checked against `argv[1]` before any clap parsing. The CLI
client commands named `scheduler` and `services` (existing admin commands) are unaffected — they are only reachable when no service name matched
`argv[1]`.

The dispatch table is a static compile-time list (via `#[cfg]`). No runtime registration. Adding a new service requires a code change in `main.rs`.

## Changes Per Crate

| Crate                             | Change                                                                                                                                                                             |
| --------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **`controller-runtime`**          | **New.** Depends on controller lib. `run_from_args()`. No default features.                                                                                                        |
| `controller`                      | **Convert bin-only to lib+bin.** Bootstrap logic moves to `lib.rs`. `main.rs` becomes thin wrapper.                                                                                |
| `agent-runtime`                   | Add `run_from_args(Vec<OsString>) -> ExitCode`                                                                                                                                     |
| `agent`                           | `main.rs` thins to call `run_from_args`                                                                                                                                            |
| `agent-ssh-runtime`               | Create `standalone` feature in Cargo.toml; merge entire agent-ssh lib into it; `run_from_args` behind `standalone`                                                                 |
| `agent-ssh`                       | `main.rs` thins to wrapper; `lib.rs` becomes empty or re-exports from runtime                                                                                                      |
| `controller` (embedded-ssh-agent) | Add `standalone` to existing agent-ssh-runtime dep; switch all 6 imports from `uptrakit_agent_ssh` → `uptrakit_agent_ssh_runtime`; remove agent-ssh dep under `embedded-ssh-agent` |
| `mqtt-runtime`                    | Add `run_from_args(Vec<OsString>) -> ExitCode`                                                                                                                                     |
| `mqtt`                            | `main.rs` thins to call `run_from_args`                                                                                                                                            |
| `scheduler-runtime`               | Add `run_from_args` behind existing `standalone` feature                                                                                                                           |
| `scheduler`                       | `main.rs` thins to call `run_from_args`                                                                                                                                            |
| `cli`                             | `unified` feature; `argv[1]` pre-dispatch in `main`; custom `--help` text for service commands                                                                                     |
| Docker                            | No change                                                                                                                                                                          |
| Workspace `Cargo.toml`            | Add `uptrakit-controller-runtime` to workspace members                                                                                                                             |

## What Stays Unchanged

- Controller's `embedded-*` feature flags and in-process service composition
- Service SDK, lifecycle model, capability-based identity
- Individual binary builds (`cargo build -p uptrakit-agent`)
- All existing CLI commands and flags, including `scheduler` and `services` admin commands
- mTLS, PKI, enrollment model

## Testing

- Build with `unified` feature, verify `uptrakit --help` mentions service dispatch and all service names
- Confirm `uptrakit scheduler ...` (admin command) and `uptrakit scheduler` (daemon dispatch) are distinct and both work
- Build without `unified`, verify no service dispatch code is present
- Run each service via god binary, verify identical behavior to standalone binary (same flags, same tracing output, same exit codes)
- Verify individual service binaries still compile and work
- Verify `controller-*` feature flags propagate correctly
- Verify feature flags are no-ops without `unified`
- Verify `agent-ssh-runtime` standalone feature has no Cargo cycle
- Verify `scheduler-runtime` standalone feature activates correctly
