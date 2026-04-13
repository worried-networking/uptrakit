# Service Platform Unification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the mixed standalone-plus-embedded service architecture with a unified service platform that hosts one runtime implementation per service for `agent`, `agent-ssh`, `scheduler`, and `mqtt`.

**Architecture:** Introduce a new shared `service-platform` crate that defines service definitions, runtime lifecycle, host/session contracts, and declarative yielding. Extract each service into a dedicated runtime crate, keep `service-sdk` as standalone protocol plumbing, and refactor the controller into an embedded host that runs the same service runtimes over in-process transport.

**Tech Stack:** Rust 2024 workspace crates, Tokio, existing `uptrakit-service-sdk`, `uptrakit-internal-wire`, SeaORM, scheduler-engine, existing controller embedded transport and service registry infrastructure.

---

## File Structure

### New crates

- Create: `crates/shared/service-platform/Cargo.toml`
- Create: `crates/shared/service-platform/src/lib.rs`
- Create: `crates/shared/service-platform/src/definition.rs`
- Create: `crates/shared/service-platform/src/context.rs`
- Create: `crates/shared/service-platform/src/runtime.rs`
- Create: `crates/shared/service-platform/src/session.rs`
- Create: `crates/shared/service-platform/src/yielding.rs`
- Create: `crates/shared/service-platform/src/standalone.rs`
- Create: `crates/shared/service-platform/tests/platform_smoke.rs`
- Create: `crates/core/agent-runtime/Cargo.toml`
- Create: `crates/core/agent-runtime/src/lib.rs`
- Create: `crates/core/agent-runtime/src/definition.rs`
- Create: `crates/core/agent-runtime/src/runtime.rs`
- Create: `crates/core/agent-runtime/src/events.rs`
- Create: `crates/core/agent-ssh-runtime/Cargo.toml`
- Create: `crates/core/agent-ssh-runtime/src/lib.rs`
- Create: `crates/core/agent-ssh-runtime/src/definition.rs`
- Create: `crates/core/agent-ssh-runtime/src/runtime.rs`
- Create: `crates/core/agent-ssh-runtime/src/events.rs`
- Create: `crates/core/scheduler-runtime/Cargo.toml`
- Create: `crates/core/scheduler-runtime/src/lib.rs`
- Create: `crates/core/scheduler-runtime/src/definition.rs`
- Create: `crates/core/scheduler-runtime/src/runtime.rs`
- Create: `crates/core/mqtt-runtime/Cargo.toml`
- Create: `crates/core/mqtt-runtime/src/lib.rs`
- Create: `crates/core/mqtt-runtime/src/definition.rs`
- Create: `crates/core/mqtt-runtime/src/runtime.rs`
- Create: `crates/core/controller/src/service_host/mod.rs`
- Create: `crates/core/controller/src/service_host/embedded_host.rs`
- Create: `crates/core/controller/src/service_host/builtins.rs`
- Create: `crates/core/controller/src/service_host/yielding.rs`

### Existing shared crates to modify

- Modify: `Cargo.toml`
- Modify: `crates/shared/service-sdk/src/lib.rs`
- Modify: `crates/shared/service-sdk/src/main_helper.rs`
- Modify: `crates/shared/service-sdk/src/lifecycle.rs`
- Modify: `crates/shared/service-sdk/src/shared_types.rs`

### Existing controller files to modify

- Modify: `crates/core/controller/Cargo.toml`
- Modify: `crates/core/controller/src/main.rs`
- Modify: `crates/core/controller/src/embedded/mod.rs`
- Modify: `crates/core/controller/src/embedded/types.rs`
- Modify: `crates/core/controller/src/agent/mod.rs`
- Modify: `crates/core/controller/src/ssh_agent/mod.rs`
- Modify: `crates/core/controller/src/scheduler/mod.rs`

### Existing standalone binaries to modify

- Modify: `crates/core/agent/Cargo.toml`
- Modify: `crates/core/agent/src/main.rs`
- Modify: `crates/core/agent-ssh/Cargo.toml`
- Modify: `crates/core/agent-ssh/src/main.rs`
- Modify: `crates/core/scheduler/Cargo.toml`
- Modify: `crates/core/scheduler/src/main.rs`
- Modify: `crates/core/scheduler/src/handler.rs`
- Modify: `crates/core/mqtt/Cargo.toml`
- Modify: `crates/core/mqtt/src/main.rs`

### Existing service logic likely moved or slimmed down

- Modify: `crates/shared/agent-core/Cargo.toml`
- Modify: `crates/core/agent/src/client.rs`
- Modify: `crates/core/agent-ssh/src/host_cli.rs` only if launch-time logic must remain binary-local
- Modify: `docs/development/service-lifecycle.md`
- Modify: `docs/architecture/embedded-services.md`

## Task 1: Create The Shared Service Platform Skeleton

**Files:**
- Create: `crates/shared/service-platform/Cargo.toml`
- Create: `crates/shared/service-platform/src/lib.rs`
- Create: `crates/shared/service-platform/src/definition.rs`
- Create: `crates/shared/service-platform/src/context.rs`
- Create: `crates/shared/service-platform/src/runtime.rs`
- Create: `crates/shared/service-platform/src/session.rs`
- Create: `crates/shared/service-platform/src/yielding.rs`
- Test: `crates/shared/service-platform/tests/platform_smoke.rs`
- Modify: `Cargo.toml`

- [ ] **Step 1: Add the new workspace crate manifest**

```toml
[package]
name = "uptrakit-service-platform"
description = "Shared service platform abstractions for standalone and embedded Uptrakit services"
edition = "2024"
license.workspace = true
authors.workspace = true
repository.workspace = true
version.workspace = true

[dependencies]
async-trait = { workspace = true }
tokio = { workspace = true, features = ["sync", "time"] }
tracing = { workspace = true }
uuid = { workspace = true, features = ["serde", "v7"] }
uptrakit-internal-wire = { workspace = true }
rootcause = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 2: Define the platform public surface in `lib.rs`**

```rust
pub mod context;
pub mod definition;
pub mod runtime;
pub mod session;
pub mod yielding;
pub mod standalone;

pub use context::{ServiceContext, ServiceScope};
pub use definition::{ServiceDefinition, ServiceKind};
pub use runtime::{RuntimeControl, ServiceRuntime};
pub use session::ServiceSession;
pub use yielding::{RuntimeYieldState, YieldHook, YieldPolicy};
```

- [ ] **Step 3: Define `ServiceKind` and `ServiceDefinition`**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServiceKind {
    Agent,
    AgentSsh,
    Scheduler,
    Mqtt,
}

pub struct ServiceDefinition<R> {
    pub kind: ServiceKind,
    pub app_name: &'static str,
    pub capabilities: fn() -> std::collections::BTreeSet<uptrakit_internal_wire::Capability>,
    pub scope: crate::context::ServiceScope,
    pub yield_policy: crate::yielding::YieldPolicy,
    pub build: fn() -> R,
}
```

- [ ] **Step 4: Define the runtime traits and yield hooks**

```rust
#[async_trait::async_trait]
pub trait ServiceRuntime: Send {
    async fn activate(
        &mut self,
        session: &mut dyn crate::session::ServiceSession,
        ctx: &mut crate::context::ServiceContext,
    ) -> rootcause::Result<()>;

    async fn run_until_stopped(
        &mut self,
        session: &mut dyn crate::session::ServiceSession,
        ctx: &mut crate::context::ServiceContext,
        control: &mut dyn RuntimeControl,
    ) -> rootcause::Result<()>;

    async fn drain(
        &mut self,
        session: &mut dyn crate::session::ServiceSession,
        ctx: &mut crate::context::ServiceContext,
    );

    async fn abort(&mut self, ctx: &mut crate::context::ServiceContext);
}

pub trait YieldHook {
    fn on_yield_start(&mut self);
    fn on_yield_stop(&mut self);
}
```

- [ ] **Step 5: Define the yield policy types**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YieldPolicy {
    SameServiceSameHost,
    SameServiceAnywhere,
    Never,
}

#[derive(Debug, Default)]
pub struct RuntimeYieldState {
    yielded: std::sync::atomic::AtomicBool,
}
```

- [ ] **Step 6: Add a smoke test that the crate exports the intended surface**

```rust
#[test]
fn platform_types_are_constructible() {
    use uptrakit_service_platform::{ServiceKind, YieldPolicy};

    assert_eq!(ServiceKind::Agent as u8, ServiceKind::Agent as u8);
    assert!(matches!(YieldPolicy::SameServiceAnywhere, YieldPolicy::SameServiceAnywhere));
}
```

- [ ] **Step 7: Run crate-level verification**

Run: `cargo check -p uptrakit-service-platform`
Expected: PASS

Run: `cargo test -p uptrakit-service-platform`
Expected: PASS

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml crates/shared/service-platform
git commit -m "feat(platform): add shared service platform skeleton"
```

### Task 2: Add Standalone Host Integration On Top Of `service-sdk`

**Files:**
- Modify: `crates/shared/service-sdk/src/lib.rs`
- Modify: `crates/shared/service-sdk/src/main_helper.rs`
- Modify: `crates/shared/service-sdk/src/lifecycle.rs`
- Modify: `crates/shared/service-sdk/src/shared_types.rs`
- Create: `crates/shared/service-platform/src/standalone.rs`
- Test: `crates/shared/service-platform/tests/platform_smoke.rs`

- [ ] **Step 1: Add a standalone runner facade in `service-platform`**

```rust
pub async fn run_standalone<R>(
    binary_name: &str,
    args: &uptrakit_service_sdk::cli::CommonServiceArgs,
    runtime: &mut R,
) where
    R: crate::runtime::ServiceRuntime + Send,
{
    let _ = binary_name;
    let _ = args;
    let _ = runtime;
}
```

- [ ] **Step 2: Introduce a temporary adapter from `ServiceRuntime` to existing `ServiceHandler`**

```rust
struct RuntimeHandlerAdapter<'a, R> {
    runtime: &'a mut R,
}

impl<'a, R> RuntimeHandlerAdapter<'a, R> {
    fn new(runtime: &'a mut R) -> Self {
        Self { runtime }
    }
}
```

- [ ] **Step 3: Keep `service-sdk` as standalone plumbing by adding only the adapter seam**

```rust
pub use main_helper::{init_crypto, print_build_info, run_lifecycle_and_handle_errors};
```

Expected code change: do not broaden `service-sdk` responsibilities. The adapter should live in `service-platform`,
while `service-sdk` remains the websocket/enrollment/lifecycle implementation.

- [ ] **Step 4: Add a smoke test proving `run_standalone` links against `service-sdk`**

```rust
#[test]
fn standalone_runner_symbol_is_available() {
    let _ = uptrakit_service_platform::standalone::run_standalone::<DummyRuntime>;
}

struct DummyRuntime;
```

- [ ] **Step 5: Run verification**

Run: `cargo check -p uptrakit-service-platform -p uptrakit-service-sdk`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/shared/service-platform/src/standalone.rs crates/shared/service-sdk
git commit -m "refactor(platform): add standalone host adapter seam"
```

### Task 3: Refactor Controller Embedded Hosting Into A Generic Service Host

**Files:**
- Create: `crates/core/controller/src/service_host/mod.rs`
- Create: `crates/core/controller/src/service_host/embedded_host.rs`
- Create: `crates/core/controller/src/service_host/builtins.rs`
- Create: `crates/core/controller/src/service_host/yielding.rs`
- Modify: `crates/core/controller/src/main.rs`
- Modify: `crates/core/controller/src/embedded/mod.rs`
- Modify: `crates/core/controller/src/embedded/types.rs`
- Modify: `crates/core/controller/Cargo.toml`
- Test: `crates/core/controller/src/embedded/mod.rs`

- [ ] **Step 1: Add a controller-local embedded host module that wraps the existing embedded transport and registry wiring**

```rust
pub(crate) struct BuiltinServiceHost {
    embedded: std::sync::Arc<crate::embedded::EmbeddedServiceHost>,
}

impl BuiltinServiceHost {
    pub(crate) fn new(embedded: std::sync::Arc<crate::embedded::EmbeddedServiceHost>) -> Self {
        Self { embedded }
    }
}
```

- [ ] **Step 2: Define controller-side yield matching based on declarative policy**

```rust
pub(crate) fn matches_yield_policy(
    policy: uptrakit_service_platform::YieldPolicy,
    app_name: &str,
    local_machine_id: Option<&str>,
    info: &crate::embedded::types::ExternalServiceInfo,
) -> bool {
    match policy {
        uptrakit_service_platform::YieldPolicy::SameServiceSameHost => {
            info.service_app_name.as_deref() == Some(app_name)
                && info.machine_id.as_deref() == local_machine_id
        }
        uptrakit_service_platform::YieldPolicy::SameServiceAnywhere => {
            info.service_app_name.as_deref() == Some(app_name)
        }
        uptrakit_service_platform::YieldPolicy::Never => false,
    }
}
```

- [ ] **Step 3: Create built-in service descriptors for controller startup**

```rust
pub(crate) struct BuiltinRegistration {
    pub label: &'static str,
    pub app_name: &'static str,
    pub yield_policy: uptrakit_service_platform::YieldPolicy,
}
```

- [ ] **Step 4: Replace ad hoc yield closures in `main.rs` with descriptor-driven registration**

Make these concrete substitutions in `spawn_background_tasks()`:

- replace the embedded agent `CoexistencePolicy::Custom(...)` closure with a helper that maps
  `YieldPolicy::SameServiceSameHost` to a controller-side predicate using `service_app_name == "uptrakit-agent"` and
  `machine_id == local_machine_id`
- replace the embedded scheduler `YieldOnSameAppName` call site with a helper that maps
  `YieldPolicy::SameServiceAnywhere` to `service_app_name == "uptrakit-scheduler"`
- replace the embedded SSH agent `YieldOnSameAppName` call site with a helper that maps
  `YieldPolicy::SameServiceAnywhere` to `service_app_name == "uptrakit-agent-ssh"`
- add the missing embedded MQTT built-in registration and map `YieldPolicy::SameServiceAnywhere` to
  `service_app_name == "uptrakit-mqtt"`

- [ ] **Step 5: Keep the existing `EmbeddedServiceHost` transport plumbing, but hide direct use behind `service_host`**

Replace direct `embedded_host.add(...)` calls in `spawn_background_tasks()` with controller-local helpers such as:

```rust
service_host::builtins::register_agent(...).await?;
service_host::builtins::register_agent_ssh(...).await?;
service_host::builtins::register_scheduler(...).await?;
service_host::builtins::register_mqtt(...).await?;
```

- [ ] **Step 6: Add focused tests for yield-policy mapping**

```rust
#[test]
fn same_service_anywhere_matches_by_app_name_only() {}

#[test]
fn same_service_same_host_requires_machine_id_match() {}
```

- [ ] **Step 7: Run verification**

Run: `cargo test -p uptrakit-controller embedded::`
Expected: PASS

Run: `cargo check -p uptrakit-controller --no-default-features --features db-sqlite,embedded-scheduler,embedded-agent,embedded-ssh-agent`
Expected: PASS

- [ ] **Step 8: Commit**

```bash
git add crates/core/controller/src/service_host crates/core/controller/src/main.rs crates/core/controller/src/embedded crates/core/controller/Cargo.toml
git commit -m "refactor(controller): add generic built-in service host"
```

### Task 4: Extract `agent-runtime`

**Files:**
- Create: `crates/core/agent-runtime/Cargo.toml`
- Create: `crates/core/agent-runtime/src/lib.rs`
- Create: `crates/core/agent-runtime/src/definition.rs`
- Create: `crates/core/agent-runtime/src/runtime.rs`
- Create: `crates/core/agent-runtime/src/events.rs`
- Modify: `crates/core/agent/Cargo.toml`
- Modify: `crates/core/agent/src/main.rs`
- Modify: `crates/core/controller/src/agent/mod.rs`
- Test: `crates/core/agent/src/main.rs`

- [ ] **Step 1: Create the new runtime crate manifest**

```toml
[package]
name = "uptrakit-agent-runtime"
edition = "2024"
license.workspace = true
authors.workspace = true
repository.workspace = true
version.workspace = true

[dependencies]
tokio = { workspace = true, features = ["sync", "time", "fs"] }
tracing = { workspace = true }
uptrakit-service-platform = { workspace = true }
uptrakit-agent-core = { workspace = true }
uptrakit-command = { workspace = true }
uptrakit-internal-wire = { workspace = true }
```

- [ ] **Step 2: Move the service behavior into `AgentRuntime`**

```rust
pub struct AgentRuntime {
    machine_id: String,
    in_flight_update: Option<uptrakit_agent_core::client::InFlightUpdate>,
    last_update_accepted: Option<std::time::Instant>,
}
```

Move these concrete responsibilities out of `crates/core/agent/src/main.rs` and
`crates/core/controller/src/agent/mod.rs` into `crates/core/agent-runtime/src/runtime.rs`:

- `on_connected` register/report logic
- `on_settings` delayed initial `ReportHosts` send
- `on_message` handling for `CheckVersions`, `ExecuteUpdate`, `DiscoverSoftware`, `ExecuteBatchUpdate`,
  `SetUpdateFreeze`, `TestPluginConfig`, and `UpdateStdinData`
- `poll_service_event` and `on_service_event` handling for in-flight update output/completion/attention
- freeze-file checks and `UPDATE_COOLDOWN` enforcement

- [ ] **Step 3: Declare the runtime definition with same-host yield policy**

```rust
pub fn definition() -> uptrakit_service_platform::ServiceDefinition<AgentRuntime> {
    uptrakit_service_platform::ServiceDefinition {
        kind: uptrakit_service_platform::ServiceKind::Agent,
        app_name: "uptrakit-agent",
        capabilities: agent_capabilities,
        scope: uptrakit_service_platform::ServiceScope::Tenant,
        yield_policy: uptrakit_service_platform::YieldPolicy::SameServiceSameHost,
        build: AgentRuntime::new,
    }
}
```

- [ ] **Step 4: Reduce the standalone binary to launcher code**

Target `crates/core/agent/src/main.rs` end state:

```rust
#[tokio::main]
async fn main() {
    let args = cli::Args::parse();
    let mut runtime = uptrakit_agent_runtime::AgentRuntime::new();
    uptrakit_service_platform::standalone::run_standalone("uptrakit-agent", &args.common, &mut runtime).await;
}
```

- [ ] **Step 5: Reduce the controller embedded agent module to host wiring or delete it**

Delete `run_embedded_agent(...)` from `crates/core/controller/src/agent/mod.rs`. If any code must remain in that file,
limit it to controller-only adapter helpers such as local executor construction or host-specific dependency assembly.

- [ ] **Step 6: Run verification**

Run: `cargo check -p uptrakit-agent-runtime -p uptrakit-agent`
Expected: PASS

Run: `cargo test -p uptrakit-agent`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add crates/core/agent-runtime crates/core/agent/Cargo.toml crates/core/agent/src/main.rs crates/core/controller/src/agent/mod.rs
git commit -m "refactor(agent): extract unified agent runtime"
```

### Task 5: Extract `agent-ssh-runtime`

**Files:**
- Create: `crates/core/agent-ssh-runtime/Cargo.toml`
- Create: `crates/core/agent-ssh-runtime/src/lib.rs`
- Create: `crates/core/agent-ssh-runtime/src/definition.rs`
- Create: `crates/core/agent-ssh-runtime/src/runtime.rs`
- Create: `crates/core/agent-ssh-runtime/src/events.rs`
- Modify: `crates/core/agent-ssh/Cargo.toml`
- Modify: `crates/core/agent-ssh/src/main.rs`
- Modify: `crates/core/controller/src/ssh_agent/mod.rs`
- Test: `crates/core/agent-ssh/src/main.rs`

- [ ] **Step 1: Create the runtime crate and pull SSH service state into `SshAgentRuntime`**

```rust
pub struct SshAgentRuntime {
    service_id: Option<uuid::Uuid>,
    tenant_id: Option<uuid::Uuid>,
    in_flight_updates: std::collections::HashMap<String, uptrakit_agent_ssh::client::SshInFlightUpdate>,
    last_update_per_host: std::collections::HashMap<String, std::time::Instant>,
}
```

- [ ] **Step 2: Move event polling and background channels into the runtime crate**

Move these concrete responsibilities from `crates/core/agent-ssh/src/main.rs` and
`crates/core/controller/src/ssh_agent/mod.rs` into `crates/core/agent-ssh-runtime/src/runtime.rs`:

- `SshAgentEvent`, `poll_updates`, and `poll_reload_tick`
- `on_connected`, `on_settings`, `on_message`, `poll_service_event`, `on_service_event`, `on_extension_request`, and
  `on_extension_response`
- host reload ticker setup and snapshot diff handling
- background result and aggregate update event channels
- freeze checks and per-host rate limiting
- extension proxy and infrastructure bundle orchestration

- [ ] **Step 3: Declare the runtime definition with yield-anywhere policy**

```rust
pub fn definition() -> uptrakit_service_platform::ServiceDefinition<SshAgentRuntime> {
    uptrakit_service_platform::ServiceDefinition {
        kind: uptrakit_service_platform::ServiceKind::AgentSsh,
        app_name: "uptrakit-agent-ssh",
        capabilities: uptrakit_agent_ssh::client::ssh_agent_capabilities,
        scope: uptrakit_service_platform::ServiceScope::Tenant,
        yield_policy: uptrakit_service_platform::YieldPolicy::SameServiceAnywhere,
        build: SshAgentRuntime::new,
    }
}
```

- [ ] **Step 4: Reduce `crates/core/agent-ssh/src/main.rs` to launch code**

```rust
let mut runtime = uptrakit_agent_ssh_runtime::SshAgentRuntime::new();
uptrakit_service_platform::standalone::run_standalone("uptrakit-agent-ssh", &args.common, &mut runtime).await;
```

- [ ] **Step 5: Remove the full embedded SSH agent loop from the controller**

Delete `run_embedded_ssh_agent(...)` from `crates/core/controller/src/ssh_agent/mod.rs`. If the file survives, keep
only controller-specific adapters such as injected DB handles, extension bridge helpers, or controller-owned notifier
construction.

- [ ] **Step 6: Run verification**

Run: `cargo check -p uptrakit-agent-ssh-runtime -p uptrakit-agent-ssh`
Expected: PASS

Run: `cargo test -p uptrakit-agent-ssh`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add crates/core/agent-ssh-runtime crates/core/agent-ssh/Cargo.toml crates/core/agent-ssh/src/main.rs crates/core/controller/src/ssh_agent/mod.rs
git commit -m "refactor(agent-ssh): extract unified ssh agent runtime"
```

### Task 6: Extract `scheduler-runtime`

**Files:**
- Create: `crates/core/scheduler-runtime/Cargo.toml`
- Create: `crates/core/scheduler-runtime/src/lib.rs`
- Create: `crates/core/scheduler-runtime/src/definition.rs`
- Create: `crates/core/scheduler-runtime/src/runtime.rs`
- Modify: `crates/core/scheduler/Cargo.toml`
- Modify: `crates/core/scheduler/src/main.rs`
- Modify: `crates/core/scheduler/src/handler.rs`
- Modify: `crates/core/controller/src/scheduler/mod.rs`
- Test: `crates/core/scheduler/src/handler.rs`

- [ ] **Step 1: Create `SchedulerRuntime` and move scheduler startup phases into it**

```rust
pub struct SchedulerRuntime {
    runtime: Option<SchedulerEngineHandle>,
    service_id: Option<uuid::Uuid>,
    poll_interval_secs: u64,
}
```

- [ ] **Step 2: Make service credential handling part of the runtime, not the standalone handler**

Move these concrete responsibilities from `crates/core/scheduler/src/handler.rs` into
`crates/core/scheduler-runtime/src/runtime.rs`:

- `ControllerMessage::ServiceCredentials` handling
- master key initialization and AAD registration
- database connection setup
- data key ring initialization
- NATS connection and stream setup
- scheduler notifier construction through injected host dependencies
- `stop_scheduler(true/false)` drain and abort logic

- [ ] **Step 3: Define controller-only injected dependencies instead of keeping a second embedded scheduler implementation**

```rust
pub struct SchedulerHostDeps {
    pub notifier_factory: std::sync::Arc<dyn Fn() -> std::sync::Arc<dyn uptrakit_scheduler_engine::SchedulerNotifier> + Send + Sync>,
}
```

- [ ] **Step 4: Declare the runtime definition with yield-anywhere policy**

```rust
pub fn definition(poll_interval_secs: u64) -> uptrakit_service_platform::ServiceDefinition<SchedulerRuntime> {
    uptrakit_service_platform::ServiceDefinition {
        kind: uptrakit_service_platform::ServiceKind::Scheduler,
        app_name: "uptrakit-scheduler",
        capabilities: scheduler_capabilities,
        scope: uptrakit_service_platform::ServiceScope::System,
        yield_policy: uptrakit_service_platform::YieldPolicy::SameServiceAnywhere,
        build: move || SchedulerRuntime::new(poll_interval_secs),
    }
}
```

- [ ] **Step 5: Reduce the standalone scheduler binary to a launcher**

Target `crates/core/scheduler/src/main.rs` end state:

```rust
let mut runtime = uptrakit_scheduler_runtime::SchedulerRuntime::new(args.poll_interval_secs);
uptrakit_service_platform::standalone::run_standalone("uptrakit-scheduler", &args.common, &mut runtime).await;
```

- [ ] **Step 6: Replace controller `spawn_background_tasks()` scheduler assembly with runtime hosting**

Delete the block in `spawn_background_tasks()` that manually:

- builds `ControllerSchedulerNotifier`
- constructs `Scheduler::new(...)`
- registers executors inline
- calls `sched.run(tokens.drain, tokens.abort).await`

Replace it with a built-in runtime registration that injects the controller notifier/executor factories into
`scheduler-runtime`.

- [ ] **Step 7: Run verification**

Run: `cargo check -p uptrakit-scheduler-runtime -p uptrakit-scheduler -p uptrakit-controller`
Expected: PASS

Run: `cargo test -p uptrakit-scheduler`
Expected: PASS

- [ ] **Step 8: Commit**

```bash
git add crates/core/scheduler-runtime crates/core/scheduler/Cargo.toml crates/core/scheduler/src/main.rs crates/core/scheduler/src/handler.rs crates/core/controller/src/scheduler/mod.rs crates/core/controller/src/main.rs
git commit -m "refactor(scheduler): extract unified scheduler runtime"
```

### Task 7: Extract `mqtt-runtime`

**Files:**
- Create: `crates/core/mqtt-runtime/Cargo.toml`
- Create: `crates/core/mqtt-runtime/src/lib.rs`
- Create: `crates/core/mqtt-runtime/src/definition.rs`
- Create: `crates/core/mqtt-runtime/src/runtime.rs`
- Modify: `crates/core/mqtt/Cargo.toml`
- Modify: `crates/core/mqtt/src/main.rs`
- Test: `crates/core/mqtt/src/main.rs`

- [ ] **Step 1: Create `MqttRuntime` and move the current handler state into it**

```rust
pub struct MqttRuntime {
    tenant_mgr: TenantManager,
    configs: Vec<ParsedMqttClientConfig>,
    granted_keys: std::collections::BTreeSet<String>,
}
```

- [ ] **Step 2: Move startup/config/claim sequencing into the runtime crate**

Move these concrete responsibilities from `crates/core/mqtt/src/main.rs` into
`crates/core/mqtt-runtime/src/runtime.rs`:

- `on_connected`, `on_settings`, `on_message`, `on_service_config_ack`, `on_extension_request`, `poll_service_event`,
  `on_service_event`, and `on_shutdown`
- service config delivery and config update reconciliation
- workload claim coordination and rejected-client stop handling
- tenant manager state propagation for `SoftwareStates` and `HostConnectivityUpdated`
- MQTT event channel handling and broker/client manager orchestration

- [ ] **Step 3: Declare the runtime definition with yield-anywhere policy**

```rust
pub fn definition() -> uptrakit_service_platform::ServiceDefinition<MqttRuntime> {
    uptrakit_service_platform::ServiceDefinition {
        kind: uptrakit_service_platform::ServiceKind::Mqtt,
        app_name: "uptrakit-mqtt",
        capabilities: mqtt_capabilities,
        scope: uptrakit_service_platform::ServiceScope::System,
        yield_policy: uptrakit_service_platform::YieldPolicy::SameServiceAnywhere,
        build: MqttRuntime::new,
    }
}
```

- [ ] **Step 4: Reduce the standalone binary to a launcher**

```rust
let mut runtime = uptrakit_mqtt_runtime::MqttRuntime::new();
uptrakit_service_platform::standalone::run_standalone("uptrakit-mqtt", &args.common, &mut runtime).await;
```

- [ ] **Step 5: Add controller embedded MQTT hosting support**

Add an embedded MQTT registration helper under `crates/core/controller/src/service_host/builtins.rs` and call it from
`spawn_background_tasks()` so the controller hosts `uptrakit-mqtt` through `mqtt-runtime`, using
`YieldPolicy::SameServiceAnywhere`.

- [ ] **Step 6: Run verification**

Run: `cargo check -p uptrakit-mqtt-runtime -p uptrakit-mqtt -p uptrakit-controller`
Expected: PASS

Run: `cargo test -p uptrakit-mqtt`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add crates/core/mqtt-runtime crates/core/mqtt/Cargo.toml crates/core/mqtt/src/main.rs crates/core/controller/src/main.rs
git commit -m "refactor(mqtt): extract unified mqtt runtime"
```

### Task 8: Remove Legacy Embedded Loops And Normalize Built-In Registration

**Files:**
- Modify: `crates/core/controller/src/main.rs`
- Modify: `crates/core/controller/src/agent/mod.rs`
- Modify: `crates/core/controller/src/ssh_agent/mod.rs`
- Modify: `crates/core/controller/src/embedded/mod.rs`
- Modify: `crates/core/controller/src/service_host/builtins.rs`
- Test: `crates/core/controller/src/embedded/mod.rs`

- [ ] **Step 1: Replace service-specific embedded registrations with definition-driven built-ins**

```rust
register_builtin(&agent_runtime::definition(), BuiltinMode::Embedded);
register_builtin(&agent_ssh_runtime::definition(), BuiltinMode::Embedded);
register_builtin(&scheduler_runtime::definition(poll_interval), BuiltinMode::Embedded);
register_builtin(&mqtt_runtime::definition(), BuiltinMode::Embedded);
```

- [ ] **Step 2: Delete the obsolete custom embedded loops**

Expected deletions:

- `run_embedded_agent(...)`
- `run_embedded_ssh_agent(...)`

If temporary compatibility shims remain, they must become thin wrappers that immediately delegate into the runtime
crates rather than hosting independent business logic.

- [ ] **Step 3: Make the built-in service list explicit and centralized**

```rust
pub(crate) fn builtin_services(...) -> Vec<BuiltinRegistration> {
    vec![
        /* one entry per built-in service */
    ]
}
```

- [ ] **Step 4: Run verification**

Run: `cargo check -p uptrakit-controller --all-features`
Expected: PASS

Run: `cargo test -p uptrakit-controller`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/core/controller/src/main.rs crates/core/controller/src/embedded/mod.rs crates/core/controller/src/service_host crates/core/controller/src/agent/mod.rs crates/core/controller/src/ssh_agent/mod.rs
git commit -m "refactor(controller): host built-ins through unified service definitions"
```

### Task 9: Update Documentation And Verification Coverage

**Files:**
- Modify: `docs/development/service-lifecycle.md`
- Modify: `docs/superpowers/specs/2026-04-13-service-platform-unification-design.md` only if implementation reality diverges
- Modify: `crates/shared/service-platform/tests/platform_smoke.rs`
- Modify: `crates/core/controller/src/embedded/mod.rs`

- [ ] **Step 1: Document the new layering and yield policies**

Add a section like:

```md
## Unified Service Platform

- one runtime crate per service
- standalone and embedded hosts run the same runtime
- built-in `agent` yields only to same-host external `agent`
- built-in `agent-ssh`, `scheduler`, and `mqtt` yield to any external instance of the same service
```

- [ ] **Step 2: Add verification coverage for yield-policy mapping and host registration**

Required assertions:

- `agent` same-host matching requires both app name and machine ID
- `agent-ssh` matching only requires same app name
- `scheduler` matching only requires same app name
- `mqtt` matching only requires same app name

- [ ] **Step 3: Run full verification**

Run: `cargo fmt --all`
Expected: PASS

Run: `cargo check --no-default-features --features db-sqlite`
Expected: PASS

Run: `cargo check --all-features`
Expected: PASS

Run: `cargo clippy --all-targets --no-default-features --features db-sqlite`
Expected: PASS

Run: `cargo clippy --all-targets --all-features`
Expected: PASS

Run: `cargo test --all-features`
Expected: PASS

Run: `bash ci/verify_handler_state_contract.sh`
Expected: PASS or deliberately removed with replacement documented in the same change

Run: `python3 ci/verify_db_access_policy.py`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add docs/development/service-lifecycle.md crates/shared/service-platform/tests/platform_smoke.rs crates/core/controller/src/embedded/mod.rs
git commit -m "docs: describe unified service platform and yielding"
```

## Testing Notes

- Defer Docker-backed integration tests until the platform and all four runtimes compile and link cleanly.
- If controller feature interactions around `embed-frontend` cause `--all-features` failures, build the frontend first:
  `cd frontend && npm ci && npm run build`
- If `service-sdk` adapter glue survives the migration, add a follow-up task to remove it only after the runtime-host
  architecture is stable and well covered.

## Rollout

- Land the platform crate first.
- Land runtime extraction service by service.
- Keep the controller hosting shim compiling at every step.
- Do not try to delete `ServiceHandler` on day one; demote it after the runtime architecture is proven.

## Rollback

- Because this plan ignores backward compatibility, rollback is by git revert per task commit, not by preserving dual
  architectures long term.
- If one runtime extraction stalls, keep the platform crate and revert only that service’s migration commit range.

## Risks

- The temporary standalone adapter from `ServiceRuntime` to `ServiceHandler` may become sticky. Explicitly remove it
  after all four runtimes are migrated.
- Scheduler injection boundaries can regress into controller-owned business logic if notifier and executor factories are
  not kept narrow.
- MQTT embedded hosting may expose assumptions in the current config-claim flow that are hidden by the standalone-only
  deployment model.
- Agent and agent-ssh extraction can accidentally leave behavioral logic in the controller crate if the migration only
  moves loops but not policy.

## Implementation Readiness

- The target architecture is defined in the approved spec:
  `docs/superpowers/specs/2026-04-13-service-platform-unification-design.md`
- The existing controller embedded transport and registry wiring are reusable and should be wrapped, not immediately
  deleted.
- The existing `service-sdk` standalone lifecycle is reusable as an intermediate host implementation.
- The required yield behavior is fully specified and must be treated as acceptance criteria, not a follow-up.

## Related Documents

- `docs/superpowers/specs/2026-04-13-service-platform-unification-design.md`
- `docs/internal/changes/TASK-0002/TASK.md`
- `docs/internal/changes/TASK-0002/DESIGN-0005.md`
- `docs/internal/changes/TASK-0002/DESIGN-0007.md`
