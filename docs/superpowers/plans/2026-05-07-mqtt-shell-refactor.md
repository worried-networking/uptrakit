# MQTT Shell Refactor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move `StandaloneMqttHandler` from `mqtt/src/main.rs` into `mqtt-runtime` (renamed
`MqttHandler`), making the `mqtt` binary a minimal thin shell that satisfies the
binary/runtime boundary invariant.

**Architecture:** `MqttHandler` is a new public struct in `mqtt-runtime/src/handler.rs`
wrapping `MqttRuntime`. It implements `ServiceHandler`. The `mqtt` binary's `main.rs` shrinks
to a 30-line process-init shell. `service_migrations()` is not overridden (uses the default
empty `vec![]`). No behaviour changes.

**Tech Stack:** Rust, uptrakit-service-sdk, uptrakit-wire, base64

---

## File Map

| Action | Path                                      |
| ------ | ----------------------------------------- |
| Modify | `crates/core/mqtt-runtime/Cargo.toml`     |
| Create | `crates/core/mqtt-runtime/src/handler.rs` |
| Modify | `crates/core/mqtt-runtime/src/lib.rs`     |
| Modify | `crates/core/mqtt/src/main.rs`            |
| Modify | `crates/core/mqtt/Cargo.toml`             |

---

### Task 1: Add base64 to mqtt-runtime regular dependencies

**Files:**

- Modify: `crates/core/mqtt-runtime/Cargo.toml`

`MqttHandler::on_connected` encodes the ECIES public key with base64. This encoding currently
lives in `mqtt/src/main.rs`; after the move it belongs to mqtt-runtime. `base64` is only in
mqtt-runtime's dev-dependencies today.

- [ ] **Step 1: Move base64 from dev-deps to regular deps**

In `crates/core/mqtt-runtime/Cargo.toml`, add to `[dependencies]`:

```toml
base64 = { workspace = true }
```

Remove from `[dev-dependencies]`:

```toml
base64 = { workspace = true }
```

- [ ] **Step 2: Verify the crate still compiles**

```bash
cargo check -p uptrakit-mqtt-runtime
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add crates/core/mqtt-runtime/Cargo.toml
git commit -m "build(mqtt-runtime): promote base64 to regular dep for MqttHandler"
```

---

### Task 2: Create mqtt-runtime/src/handler.rs

**Files:**

- Create: `crates/core/mqtt-runtime/src/handler.rs`

This is a direct lift of `StandaloneMqttHandler` and `map_runtime_outcome` from
`mqtt/src/main.rs` with the following changes:

- Rename `StandaloneMqttHandler` → `MqttHandler`
- Replace `uptrakit_mqtt_runtime::` prefixes with `crate::` (the code is now inside the crate)
- Add `pub` visibility to `MqttHandler` and its `new()` constructor

- [ ] **Step 1: Write failing test first**

```bash
cat > /tmp/test_mqtt_handler.sh << 'EOF'
grep -q "pub struct MqttHandler" crates/core/mqtt-runtime/src/handler.rs && \
grep -q "pub fn new()" crates/core/mqtt-runtime/src/handler.rs && \
echo "PASS" || echo "FAIL: MqttHandler or new() not found"
EOF
bash /tmp/test_mqtt_handler.sh
```

Expected: `FAIL: MqttHandler or new() not found` (file does not exist yet).

- [ ] **Step 2: Create handler.rs**

```rust
use std::collections::BTreeSet;
use std::time::Duration;

use base64::Engine as _;
use rootcause::prelude::*;

use crate::{
    MqttRuntime, MqttRuntimeIdentity, MqttRuntimeLoopOutcome, MqttRuntimeSettings,
    mqtt_capabilities,
};
use uptrakit_service_sdk::{
    LoopError, LoopOutcome, LoopResult, ServiceHandler, ServiceIdentityState, ShutdownCause,
    default_resolve_shutdown,
};
use uptrakit_wire::{Capability, ControllerMessage, ServiceTransport, payloads::ServiceConfigAckPayload};

pub struct MqttHandler {
    runtime: MqttRuntime,
}

impl MqttHandler {
    pub fn new() -> Self {
        Self {
            runtime: MqttRuntime::new(),
        }
    }
}

impl Default for MqttHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl ServiceHandler for MqttHandler {
    const DIR_NAME: &'static str = crate::MQTT_DIR_NAME;
    const SERVICE_LABEL: &'static str = crate::MQTT_SERVICE_LABEL;
    const SERVICE_APP_NAME: &'static str = crate::MQTT_SERVICE_APP_NAME;

    type ServiceEvent = Option<crate::MqttServiceEvent>;

    #[expect(
        clippy::map_err_ignore,
        reason = "internal runtime errors are mapped to LoopError::Other(String) with a descriptive message; the original error type is not part of the public interface"
    )]
    async fn on_connected(
        &mut self,
        conn: &mut dyn ServiceTransport,
        identity: &ServiceIdentityState,
    ) -> LoopResult<()> {
        self.runtime
            .on_connected(
                conn,
                MqttRuntimeIdentity {
                    service_id: identity.service_id(),
                    private_key_der: identity.private_key_pkcs8_der(),
                    encryption_public_key: identity
                        .public_key_raw()
                        .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes)),
                },
            )
            .await
            .map_err(|_| report!(LoopError::Other("failed to send MQTT register".to_string())))
    }

    #[expect(
        clippy::map_err_ignore,
        reason = "internal runtime errors are mapped to LoopError::Other(String) with a descriptive message; the original error type is not part of the public interface"
    )]
    async fn on_message(
        &mut self,
        msg: ControllerMessage,
        conn: &mut dyn ServiceTransport,
    ) -> LoopResult<Option<LoopOutcome>> {
        self.runtime
            .handle_controller_message(msg, conn)
            .await
            .map(|outcome| outcome.map(map_runtime_outcome))
            .map_err(|_| {
                report!(LoopError::Other(
                    "failed to handle MQTT controller message".to_string()
                ))
            })
    }

    async fn on_settings(
        &mut self,
        settings: &uptrakit_wire::ServiceSettingsPayload,
        conn: &mut dyn ServiceTransport,
        agreed_capabilities: &BTreeSet<Capability>,
    ) {
        self.runtime
            .apply_settings(
                MqttRuntimeSettings {
                    ui_surfaces_enabled: agreed_capabilities.contains(&Capability::UiSurfaces),
                    tenant_id: settings.tenant_id,
                },
                conn,
            )
            .await;
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        mqtt_capabilities()
    }

    async fn poll_service_event(&mut self) -> Self::ServiceEvent {
        self.runtime.poll_event().await
    }

    async fn on_service_event(
        &mut self,
        event: Self::ServiceEvent,
        conn: &mut dyn ServiceTransport,
    ) -> LoopResult<Option<LoopOutcome>> {
        Ok(self
            .runtime
            .handle_event(event, conn)
            .await
            .map(map_runtime_outcome))
    }

    fn on_surface_action_response(
        &mut self,
        response: uptrakit_wire::surfaces::SurfaceActionResponse,
    ) {
        self.runtime.on_surface_action_response(response);
    }

    fn on_service_config_ack(&self, ack: ServiceConfigAckPayload) {
        self.runtime.on_service_config_ack(ack);
    }

    async fn on_yield_change(&mut self, is_yielded: bool, conn: &mut dyn ServiceTransport) {
        self.runtime.handle_yield_change(is_yielded, conn).await;
    }

    #[expect(
        clippy::map_err_ignore,
        reason = "internal runtime errors are mapped to LoopError::Other(String) with a descriptive message; the original error type is not part of the public interface"
    )]
    async fn on_surface_action_request(
        &mut self,
        request: uptrakit_wire::surfaces::SurfaceActionRequest,
        conn: &mut dyn ServiceTransport,
    ) -> LoopResult<()> {
        self.runtime
            .handle_controller_message(ControllerMessage::SurfaceActionRequest(request), conn)
            .await
            .map(|_| ())
            .map_err(|_| {
                report!(LoopError::Other(
                    "failed to handle MQTT surface action request".to_string()
                ))
            })
    }

    async fn on_shutdown(
        &mut self,
        conn: &mut dyn ServiceTransport,
        cause: ShutdownCause,
        _shutdown_timeout: Duration,
    ) -> LoopOutcome {
        let (reason, outcome) = default_resolve_shutdown(cause);
        self.runtime.shutdown(conn, reason).await;
        outcome
    }
}

fn map_runtime_outcome(outcome: MqttRuntimeLoopOutcome) -> LoopOutcome {
    match outcome {
        MqttRuntimeLoopOutcome::Disconnected => LoopOutcome::Disconnected,
    }
}
```

- [ ] **Step 3: Run the test**

```bash
bash /tmp/test_mqtt_handler.sh
```

Expected: `PASS`.

- [ ] **Step 4: Verify handler.rs compiles in isolation**

```bash
cargo check -p uptrakit-mqtt-runtime
```

Expected: error `module 'handler' not declared` — that's expected because we haven't wired it
into lib.rs yet.

- [ ] **Step 5: Commit**

```bash
git add crates/core/mqtt-runtime/src/handler.rs
git commit -m "feat(mqtt-runtime): add MqttHandler ServiceHandler impl"
```

---

### Task 3: Export MqttHandler from mqtt-runtime/src/lib.rs

**Files:**

- Modify: `crates/core/mqtt-runtime/src/lib.rs`

- [ ] **Step 1: Add module declaration and re-export**

In `crates/core/mqtt-runtime/src/lib.rs`, add the following immediately after the existing
`use` block and macro definitions (before `mod client_manager;` or at top of module
declarations):

```rust
mod handler;
pub use handler::MqttHandler;
```

The exact insertion point — find the line that declares the first `mod`:

```rust
mod client_manager;
```

Insert before it:

```rust
mod handler;
pub use handler::MqttHandler;
```

- [ ] **Step 2: Verify mqtt-runtime compiles**

```bash
cargo check -p uptrakit-mqtt-runtime
```

Expected: no errors.

- [ ] **Step 3: Verify MqttHandler is accessible from outside**

```bash
cargo rustc -p uptrakit-mqtt-runtime -- --edition=2024 2>&1 | grep -c "MqttHandler" || true
```

Expected: compiles cleanly; no errors about `MqttHandler`.

- [ ] **Step 4: Commit**

```bash
git add crates/core/mqtt-runtime/src/lib.rs
git commit -m "feat(mqtt-runtime): re-export MqttHandler from crate root"
```

---

### Task 4: Thin-shell mqtt/src/main.rs and clean up mqtt/Cargo.toml

**Files:**

- Modify: `crates/core/mqtt/src/main.rs`
- Modify: `crates/core/mqtt/Cargo.toml`

The `StandaloneMqttHandler` struct, its `impl ServiceHandler`, and `map_runtime_outcome`
all move to mqtt-runtime. The binary retains only process-level init.

- [ ] **Step 1: Write thin-shell main.rs**

Replace the entire contents of `crates/core/mqtt/src/main.rs` with:

```rust
mod cli;

use clap::Parser;
use uptrakit_mqtt_runtime::{MQTT_SERVICE_APP_NAME, MqttHandler};

#[tokio::main]
async fn main() {
    let args = cli::Args::parse();
    let _ = args.max_tenants;

    if args.common.version {
        uptrakit_service_sdk::print_build_info(
            MQTT_SERVICE_APP_NAME,
            env!("CARGO_PKG_VERSION"),
            option_env!("UPTRAKIT_BUILD_ENABLED_FEATURES"),
        );
        return;
    }

    uptrakit_service_sdk::TracingBuilder::new()
        .verbosity(args.common.verbose)
        .init();
    uptrakit_service_sdk::init_crypto();

    tracing::info!("starting uptrakit-mqtt service");

    let mut handler = MqttHandler::new();

    uptrakit_service_sdk::run_lifecycle_and_handle_errors(
        MQTT_SERVICE_APP_NAME,
        &args.common,
        &mut handler,
    )
    .await;
}
```

- [ ] **Step 2: Update mqtt/Cargo.toml — remove deps no longer used by thin shell**

The thin shell uses only: `clap`, `tokio`, `tracing`, `uptrakit-mqtt-runtime`,
`uptrakit-service-sdk`. Remove the following from `[dependencies]`:

```toml
async-trait = { workspace = true }
base64 = { workspace = true }
rootcause = { workspace = true }
uptrakit-wire = { workspace = true }
```

Keep: `clap`, `tokio`, `tracing`, `tracing-subscriber`, `uptrakit-mqtt-runtime`,
`uptrakit-service-sdk`.

The final `[dependencies]` section should be:

```toml
[dependencies]
clap = { workspace = true }
tokio = { workspace = true, features = ["macros", "rt-multi-thread", "signal"] }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
uptrakit-mqtt-runtime = { workspace = true }
uptrakit-service-sdk = { workspace = true }
```

- [ ] **Step 3: Verify mqtt binary compiles**

```bash
cargo check -p uptrakit-mqtt
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add crates/core/mqtt/src/main.rs crates/core/mqtt/Cargo.toml
git commit -m "refactor(mqtt): thin-shell main.rs; move StandaloneMqttHandler to mqtt-runtime as MqttHandler"
```

---

### Task 5: Quality gates

- [ ] **Step 1: Format**

```bash
cargo fmt --all
```

- [ ] **Step 2: Clippy — no-default-features**

```bash
cargo clippy --all-targets --no-default-features --features db-sqlite 2>&1 | grep -E "^error"
```

Expected: no errors.

- [ ] **Step 3: Clippy — all features**

```bash
cargo clippy --all-targets --all-features 2>&1 | grep -E "^error"
```

Expected: no errors.

- [ ] **Step 4: Full test suite**

```bash
cargo test -p uptrakit-mqtt-runtime
```

Expected: all tests pass (mqtt-runtime has integration tests).

- [ ] **Step 5: Full workspace test**

```bash
cargo test --all-features
```

Expected: all tests pass.

- [ ] **Step 6: Commit formatting if needed**

```bash
git add -u && git diff --cached --quiet || git commit -m "style: cargo fmt after mqtt handler move"
```

---

**Plan complete and saved to `docs/superpowers/plans/2026-05-07-mqtt-shell-refactor.md`.**
