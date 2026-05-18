# Embedded Service Identity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix `run_embedded_service` to call `handler.on_connected` before `handler.on_settings` so all embedded services (agent-ssh, mqtt,
scheduler) receive their identity identically to the standalone lifecycle.

**Architecture:** Add `ServiceIdentityState::for_embedded` as a `pub(crate)` in-memory constructor. Update `run_embedded_service` to accept a
`service_id: Uuid`, generate an ephemeral P-256 keypair, build a `for_embedded` identity, and call `on_connected` before `on_settings`. Thread
`service_id` from `EmbeddedServiceHost::add` into the `run_fn` closure. Remove all handler-side workarounds (`EciesKeypair`, `embedded_identity`,
`persist_tenant_id` flag) once the SDK supplies identity correctly.

**Tech Stack:** Rust, Tokio, `rcgen` (already in workspace), `parking_lot` (already in workspace)

---

## File Map

| File                                                               | Change                                                                                                       |
| ------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------ |
| `crates/shared/service-sdk/src/identity.rs`                        | Add `pub(crate) fn for_embedded(service_id: Uuid, keypair: rcgen::KeyPair) -> Self`                          |
| `crates/shared/service-sdk/src/shared_types.rs`                    | Remove stale doc comment on `on_connected` ("not called by run_embedded_service")                            |
| `crates/shared/service-sdk/src/embedded.rs`                        | Add `service_id: Uuid` param; keygen; call `on_connected`; update all test call sites                        |
| `crates/core/controller-runtime/src/embedded/mod.rs`               | `run_fn` type gains `Uuid` first param; call `run_fn(service_id, transport, tokens)`                         |
| `crates/core/controller-runtime/src/service_host/embedded_host.rs` | `run_fn` type in `BuiltinServiceHost::add` gains `Uuid` first param                                          |
| `crates/core/controller-runtime/src/service_host/builtins.rs`      | Update all `host.add()` closure signatures; remove ECIES keygen                                              |
| `crates/core/controller-runtime/src/ssh_agent/mod.rs`              | Remove `generate_ecies_keypair()`                                                                            |
| `crates/core/controller-runtime/src/mqtt/mod.rs`                   | Remove `generate_ecies_keypair()`                                                                            |
| `crates/core/agent-ssh-runtime/src/handler.rs`                     | Remove `EciesKeypair` struct + field; simplify `on_connected`; remove `persist_tenant_id` from `on_settings` |
| `crates/core/agent-ssh-runtime/src/lib.rs`                         | Remove `persist_tenant_id` field from `SshAgentSettings`                                                     |
| `crates/core/agent-ssh-runtime/src/runtime_support.rs`             | Remove `persist_tenant_id: bool` field + guard from `persist_tenant_id()`                                    |
| `crates/core/mqtt-runtime/src/handler.rs`                          | Remove `embedded_identity` field, `new_embedded()`, `on_settings` workaround                                 |

---

### Task 1: `ServiceIdentityState::for_embedded` constructor + doc cleanup

**Files:**

- Modify: `crates/shared/service-sdk/src/identity.rs`
- Modify: `crates/shared/service-sdk/src/shared_types.rs`

- [ ] **Step 1: Write the failing tests in `identity.rs`**

Add after the closing `}` of the existing `mod tests` block at the bottom of `crates/shared/service-sdk/src/identity.rs` (a separate top-level
`#[cfg(test)]` module, not nested inside the existing one):

```rust
#[cfg(test)]
mod for_embedded_tests {
    use uuid::Uuid;
    use super::ServiceIdentityState;

    #[test]
    fn for_embedded_returns_correct_service_id() {
        let id = Uuid::new_v4();
        let kp = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let identity = ServiceIdentityState::for_embedded(id, kp);
        assert_eq!(identity.service_id(), Some(id));
    }

    #[test]
    fn for_embedded_public_key_raw_is_uncompressed_p256_point() {
        let id = Uuid::new_v4();
        let kp = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let identity = ServiceIdentityState::for_embedded(id, kp);
        let raw = identity.public_key_raw().expect("public key must be present");
        // Uncompressed EC point: 0x04 prefix + 32 bytes X + 32 bytes Y = 65 bytes
        assert_eq!(raw.len(), 65, "expected 65-byte uncompressed P-256 point");
        assert_eq!(raw[0], 0x04, "expected 0x04 uncompressed prefix");
    }

    #[test]
    fn for_embedded_private_key_pkcs8_der_is_non_empty() {
        let id = Uuid::new_v4();
        let kp = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let identity = ServiceIdentityState::for_embedded(id, kp);
        let der = identity.private_key_pkcs8_der().expect("private key DER must be present");
        assert!(!der.is_empty());
    }
}
```

- [ ] **Step 2: Run the tests to confirm they fail**

```bash
cargo test -p uptrakit-service-sdk for_embedded -- --nocapture
```

Expected: compile error — `for_embedded` not found.

- [ ] **Step 3: Implement `ServiceIdentityState::for_embedded`**

In `crates/shared/service-sdk/src/identity.rs`, add the following method inside the `impl ServiceIdentityState` block (place it near
`new_single_dir`):

```rust
pub(crate) fn for_embedded(service_id: Uuid, keypair: rcgen::KeyPair) -> Self {
    Self {
        config_dir: std::path::PathBuf::new(), // sentinel — never used for I/O
        state_dir: std::path::PathBuf::new(),  // sentinel — never used for I/O
        service_id: Some(service_id),
        tenant_id: None,
        enrollment_secret: None,
        keypair: Some(keypair),
        certificate_pem: None,
        ca_cert_pem: None,
    }
}
```

> **Note on algorithm:** The `keypair` field carries P-256 here and P-384 in the enrollment path (`ensure_keypair`). This asymmetry is intentional
> for this spec's lifetime; the ECIES migration spec will unify both to P-256. Do not call any disk I/O method (`load`, `save_*`) on a
> `for_embedded` instance — `PathBuf::new()` resolves to the process working directory.

- [ ] **Step 4: Run the tests to confirm they pass**

```bash
cargo test -p uptrakit-service-sdk for_embedded -- --nocapture
```

Expected: 3 tests pass.

- [ ] **Step 5: Remove stale doc comment from `ServiceHandler::on_connected`**

In `crates/shared/service-sdk/src/shared_types.rs`, find the `on_connected` method of the `ServiceHandler` trait and remove the two stale lines
(around line 164):

```rust
    /// Note: **not called** by `run_embedded_service`. Embedded handlers must
    /// perform any initialization that would normally happen here inside their
    /// constructor.
```

The updated comment block should read:

```rust
    /// Called after the WebSocket connection is established (standalone) or after
    /// the embedded service starts (embedded).
    ///
    /// Send initial messages (e.g. `ReportHosts`, `Register`) here.
    async fn on_connected(
        &mut self,
        conn: &mut dyn ServiceTransport,
        identity: &ServiceIdentityState,
    ) -> LoopResult<()>;
```

- [ ] **Step 6: Verify compilation**

```bash
cargo check --all-features -p uptrakit-service-sdk
```

Expected: no errors.

- [ ] **Step 7: Commit**

```bash
git add crates/shared/service-sdk/src/identity.rs \
        crates/shared/service-sdk/src/shared_types.rs
git commit -m "feat(service-sdk): add ServiceIdentityState::for_embedded constructor

pub(crate) constructor for embedded services. Sets service_id and keypair;
uses sentinel PathBuf dirs (never used for I/O). Removes stale on_connected
doc comment claiming it is not called in embedded mode.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

### Task 2: `run_embedded_service` — add `service_id`, keygen, `on_connected` call

**Files:**

- Modify: `crates/shared/service-sdk/src/embedded.rs`
- Modify: `crates/core/controller-runtime/src/embedded/mod.rs`
- Modify: `crates/core/controller-runtime/src/service_host/embedded_host.rs`
- Modify: `crates/core/controller-runtime/src/service_host/builtins.rs`

> **Note:** All four files must change in this task to keep the workspace compiling. Changes to `builtins.rs` in this task only update closure
> signatures — handler constructors still receive keypair params until Task 3.

- [ ] **Step 1: Write the failing call-order test in `embedded.rs`**

Extend `MockHandler` in the `#[cfg(test)]` module of `crates/shared/service-sdk/src/embedded.rs` to track call order. Replace the existing
`MockHandler` and `CallLog` with:

```rust
#[derive(Debug, Default)]
struct CallLog {
    call_order: Vec<&'static str>,
    on_settings_called: bool,
    on_shutdown_called: bool,
    on_yield_change_called: bool,
    on_message_called: bool,
}

struct MockHandler {
    log: std::sync::Arc<parking_lot::Mutex<CallLog>>,
    on_connected_result: LoopResult<()>,
}

impl MockHandler {
    fn new() -> (Self, std::sync::Arc<parking_lot::Mutex<CallLog>>) {
        let log = std::sync::Arc::new(parking_lot::Mutex::new(CallLog::default()));
        (
            Self {
                log: log.clone(),
                on_connected_result: Ok(()),
            },
            log,
        )
    }

    fn new_failing_connected() -> (Self, std::sync::Arc<parking_lot::Mutex<CallLog>>) {
        let log = std::sync::Arc::new(parking_lot::Mutex::new(CallLog::default()));
        (
            Self {
                log: log.clone(),
                on_connected_result: Err(rootcause::report!(
                    crate::shared_types::LoopError::Other("on_connected failed".to_string())
                )),
            },
            log,
        )
    }
}
```

Update `on_connected` and `on_settings` in the `ServiceHandler` impl to record call order. All other methods (`on_message`, `on_shutdown`,
`on_yield_change`, `poll_service_event`, `on_service_event`) remain identical to the original — keep them unchanged:

```rust
    async fn on_connected(
        &mut self,
        _conn: &mut dyn ServiceTransport,
        _identity: &crate::identity::ServiceIdentityState,
    ) -> LoopResult<()> {
        self.log.lock().call_order.push("on_connected");
        // Clone to return without holding the lock
        match &self.on_connected_result {
            Ok(()) => Ok(()),
            Err(e) => Err(rootcause::report!(
                crate::shared_types::LoopError::Other(e.to_string())
            )),
        }
    }

    async fn on_settings(
        &mut self,
        _settings: &ServiceSettingsPayload,
        _conn: &mut dyn ServiceTransport,
        _agreed: &BTreeSet<Capability>,
    ) {
        let mut log = self.log.lock();
        log.on_settings_called = true;
        log.call_order.push("on_settings");
    }
```

Add the new tests (before the closing `}` of the `tests` module):

```rust
    #[tokio::test]
    async fn on_connected_called_before_on_settings() {
        let (ctrl_tx, ctrl_rx) = mpsc::channel::<ControllerMessage>(4);
        let transport = make_transport(ctrl_rx);
        let (handler, log) = MockHandler::new();
        let drain = CancellationToken::new();
        let abort = CancellationToken::new();

        ctrl_tx
            .send(ControllerMessage::ServiceSettings(make_settings()))
            .await
            .expect("send settings");
        drain.cancel();

        run_embedded_service(uuid::Uuid::new_v4(), handler, transport, drain, abort).await;

        let log = log.lock();
        let connected_pos = log.call_order.iter().position(|&s| s == "on_connected");
        let settings_pos = log.call_order.iter().position(|&s| s == "on_settings");
        assert!(connected_pos.is_some(), "on_connected must be called");
        assert!(settings_pos.is_some(), "on_settings must be called");
        assert!(
            connected_pos.unwrap() < settings_pos.unwrap(),
            "on_connected must be called before on_settings"
        );
    }

    #[tokio::test]
    async fn abort_when_on_connected_returns_err() {
        let (ctrl_tx, ctrl_rx) = mpsc::channel::<ControllerMessage>(4);
        let transport = make_transport(ctrl_rx);
        let (handler, log) = MockHandler::new_failing_connected();
        let drain = CancellationToken::new();
        let abort = CancellationToken::new();

        ctrl_tx
            .send(ControllerMessage::ServiceSettings(make_settings()))
            .await
            .expect("send settings");

        run_embedded_service(uuid::Uuid::new_v4(), handler, transport, drain, abort).await;

        let log = log.lock();
        assert!(
            log.call_order.contains(&"on_connected"),
            "on_connected must be called"
        );
        assert!(
            !log.on_settings_called,
            "on_settings must NOT be called when on_connected fails"
        );
    }
```

- [ ] **Step 2: Run the new tests to confirm they fail**

```bash
cargo test -p uptrakit-service-sdk on_connected_called_before -- --nocapture 2>&1 | tail -5
cargo test -p uptrakit-service-sdk abort_when_on_connected -- --nocapture 2>&1 | tail -5
```

Expected: compile errors — `run_embedded_service` takes 4 args, not 5.

- [ ] **Step 3: Update `run_embedded_service` signature and body**

In `crates/shared/service-sdk/src/embedded.rs`, update the function:

```rust
// Add `uuid` import at the top of the file if not present:
use uuid::Uuid;

/// Run a [`ServiceHandler`] in embedded mode.
///
/// Startup sequence:
/// 1. Wait up to 10 s for the controller to send `ServiceSettings` (first message).
/// 2. Generate an ephemeral P-256 keypair and build an in-memory identity.
/// 3. Call `on_connected` — provides service identity to the handler.
/// 4. Compute agreed capabilities (intersection of handler's and controller's).
/// 5. Call `on_settings` — handler configuration callback.
/// 6. Call `on_yield_change` with the transport's current yield state.
/// 7. Enter the two-phase event loop.
///
/// Exits when: drain fires (graceful via `on_shutdown`), abort fires
/// (immediate), transport closes, or a handler callback requests exit.
pub async fn run_embedded_service<H: ServiceHandler>(
    service_id: Uuid,
    mut handler: H,
    mut transport: impl ServiceTransport,
    drain: CancellationToken,
    abort: CancellationToken,
) {
    // ── Startup: wait for ServiceSettings ──────────────────────────────────
    let first_msg = tokio::select! {
        biased;
        () = abort.cancelled() => return,
        result = tokio::time::timeout(
            EMBEDDED_STARTUP_TIMEOUT,
            transport.transport_recv(),
        ) => {
            match result {
                Err(_elapsed) => {
                    tracing::error!(
                        service = H::SERVICE_LABEL,
                        "embedded service did not receive ServiceSettings within 10s; aborting"
                    );
                    return;
                }
                Ok(None) => {
                    tracing::error!(
                        service = H::SERVICE_LABEL,
                        "embedded transport closed before ServiceSettings arrived"
                    );
                    return;
                }
                Ok(Some(msg)) => msg,
            }
        }
    };

    let settings = match first_msg {
        ControllerMessage::ServiceSettings(s) => s,
        other => {
            tracing::warn!(
                service = H::SERVICE_LABEL,
                ?other,
                "expected ServiceSettings as first embedded message; aborting"
            );
            return;
        }
    };

    let mut shutdown_timeout = settings
        .shutdown_timeout
        .unwrap_or(DEFAULT_SHUTDOWN_TIMEOUT);

    // ── Identity: generate ephemeral P-256 keypair ─────────────────────────
    // P-256 is intentional: sealed_box_decrypt in sensitive_params.rs is
    // hardcoded to ECDH_P256. Migration to P-384 is a separate future spec.
    let keypair = match rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256) {
        Ok(kp) => kp,
        Err(e) => {
            tracing::error!(
                service = H::SERVICE_LABEL,
                error = %e,
                "failed to generate embedded service keypair; aborting"
            );
            return;
        }
    };
    let identity = crate::identity::ServiceIdentityState::for_embedded(service_id, keypair);

    // ── on_connected ───────────────────────────────────────────────────────
    if let Err(e) = handler.on_connected(&mut transport, &identity).await {
        tracing::error!(
            service = H::SERVICE_LABEL,
            error = %e,
            "embedded on_connected failed; aborting"
        );
        return;
    }

    let agreed = compute_agreed_capabilities(&handler, &settings);

    handler
        .on_settings(&settings, &mut transport, &agreed)
        .await;

    // Initial yield notification.
    {
        let is_yielded = transport.is_yielded();
        handler.on_yield_change(is_yielded, &mut transport).await;
    }

    // ... rest of event loop unchanged ...
```

Keep everything from the `yield_notifier` line onward unchanged (lines 97–170 in the original).

Also add `use rcgen;` import at top if not already present (check — `rcgen` is already used in `identity.rs` in the same crate, so `rcgen` is
already a workspace dependency):

```rust
// Add at top of embedded.rs, with existing use statements:
use uuid::Uuid;
```

- [ ] **Step 4: Update `EmbeddedServiceHost::add` in `embedded/mod.rs`**

In `crates/core/controller-runtime/src/embedded/mod.rs`, change the `run_fn` parameter type (lines 167–172):

Before:

```rust
        run_fn: impl FnOnce(
            EmbeddedTransport,
            EmbeddedShutdownTokens,
        ) -> Pin<Box<dyn Future<Output = ()> + Send>>
        + Send
        + 'static,
```

After:

```rust
        run_fn: impl FnOnce(
            Uuid,
            EmbeddedTransport,
            EmbeddedShutdownTokens,
        ) -> Pin<Box<dyn Future<Output = ()> + Send>>
        + Send
        + 'static,
```

And change the call site (line 290):

Before:

```rust
        let service_handle = tokio::spawn(run_fn(transport, tokens));
```

After:

```rust
        let service_handle = tokio::spawn(run_fn(service_id, transport, tokens));
```

Verify `Uuid` is imported at the top of `mod.rs` (already imported since `service_id` is used above).

- [ ] **Step 5: Update `BuiltinServiceHost::add` in `embedded_host.rs`**

In `crates/core/controller-runtime/src/service_host/embedded_host.rs`, change the `run_fn` parameter type (around lines 101–106):

Before:

```rust
        run_fn: impl FnOnce(
            crate::embedded::types::EmbeddedTransport,
            crate::embedded::EmbeddedShutdownTokens,
        ) -> Pin<Box<dyn Future<Output = ()> + Send>>
        + Send
        + 'static,
```

After:

```rust
        run_fn: impl FnOnce(
            uuid::Uuid,
            crate::embedded::types::EmbeddedTransport,
            crate::embedded::EmbeddedShutdownTokens,
        ) -> Pin<Box<dyn Future<Output = ()> + Send>>
        + Send
        + 'static,
```

The body of `BuiltinServiceHost::add` just forwards `run_fn` unchanged to `self.embedded.add(...)` — no other change needed there.

- [ ] **Step 6: Update `builtins.rs` closure signatures**

In `crates/core/controller-runtime/src/service_host/builtins.rs`:

**`register_agent_ssh`** — update the closure signature only (do NOT remove keypair generation yet — that happens in Task 3):

Before:

```rust
            move |transport, tokens| {
                Box::pin(uptrakit_service_sdk::run_embedded_service(
                    handler,
                    transport,
                    tokens.drain,
                    tokens.abort,
                ))
            },
```

After:

```rust
            move |service_id, transport, tokens| {
                Box::pin(uptrakit_service_sdk::run_embedded_service(
                    service_id,
                    handler,
                    transport,
                    tokens.drain,
                    tokens.abort,
                ))
            },
```

**`register_mqtt`** — same pattern:

Before:

```rust
            move |transport, tokens| {
                Box::pin(uptrakit_service_sdk::run_embedded_service(
                    handler,
                    transport,
                    tokens.drain,
                    tokens.abort,
                ))
            },
```

After:

```rust
            move |service_id, transport, tokens| {
                Box::pin(uptrakit_service_sdk::run_embedded_service(
                    service_id,
                    handler,
                    transport,
                    tokens.drain,
                    tokens.abort,
                ))
            },
```

**Scheduler closure** — accepts `service_id` but ignores it (scheduler calls `run_embedded_scheduler` directly, which handles its own identity via
the standard lifecycle):

Before:

```rust
            move |transport, tokens| {
                Box::pin(async move {
```

After:

```rust
            move |service_id, transport, tokens| {
                let _ = service_id;
                Box::pin(async move {
```

- [ ] **Step 7: Update existing test call sites in `embedded.rs`**

In the `#[cfg(test)]` module of `crates/shared/service-sdk/src/embedded.rs`, all calls to `run_embedded_service` must pass a `Uuid` as the first
argument. Find all occurrences of `run_embedded_service(handler,` and replace with `run_embedded_service(uuid::Uuid::nil(), handler,`:

```rust
// In exits_when_transport_closed_before_settings:
run_embedded_service(uuid::Uuid::nil(), handler, transport, drain, abort).await;

// In abort_before_settings_exits_immediately:
run_embedded_service(uuid::Uuid::nil(), handler, transport, drain, abort).await;

// In normal_startup_then_drain_calls_on_shutdown:
run_embedded_service(uuid::Uuid::nil(), handler, transport, drain, abort).await;

// In startup_timeout_exits_without_callback:
let task = tokio::spawn(run_embedded_service(uuid::Uuid::nil(), handler, transport, drain, abort));

// In yielded_transport_drops_messages_silently:
run_embedded_service(uuid::Uuid::nil(), handler, transport, drain, abort).await;
```

Add `use uuid::Uuid;` in the test module if not already present.

- [ ] **Step 8: Verify compilation and run all embedded tests**

```bash
cargo check --all-features 2>&1 | grep "^error" | head -20
cargo test -p uptrakit-service-sdk -- --nocapture 2>&1 | tail -20
```

Expected: clean compile, all tests pass including the two new ones.

- [ ] **Step 9: Commit**

```bash
git add crates/shared/service-sdk/src/embedded.rs \
        crates/core/controller-runtime/src/embedded/mod.rs \
        crates/core/controller-runtime/src/service_host/embedded_host.rs \
        crates/core/controller-runtime/src/service_host/builtins.rs
git commit -m "feat(service-sdk): call on_connected in run_embedded_service with ephemeral identity

Adds service_id: Uuid param to run_embedded_service. Generates an ephemeral
P-256 keypair, builds ServiceIdentityState::for_embedded, and calls
handler.on_connected before on_settings — matching the standalone lifecycle.
Aborts if keygen or on_connected fails.

Threads service_id through EmbeddedServiceHost::add run_fn type and all
builtins.rs closures. Scheduler closure receives but ignores service_id
(run_embedded_scheduler handles its own identity via the standard lifecycle).

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

### Task 3: SSH agent handler cleanup + `persist_tenant_id` unification

**Files:**

- Modify: `crates/core/agent-ssh-runtime/src/handler.rs`
- Modify: `crates/core/agent-ssh-runtime/src/lib.rs`
- Modify: `crates/core/agent-ssh-runtime/src/runtime_support.rs`
- Modify: `crates/core/controller-runtime/src/ssh_agent/mod.rs`
- Modify: `crates/core/controller-runtime/src/service_host/builtins.rs`

- [ ] **Step 1: Remove `AgentSshMode` enum, `EciesKeypair` struct, and related items from `handler.rs`**

In `crates/core/agent-ssh-runtime/src/handler.rs`:

Remove the `AgentSshMode` enum definition (lines 22–27):

```rust
// DELETE these lines:
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSshMode {
    Binary,
    Embedded,
}
```

Remove the entire `EciesKeypair` struct (lines 29–32):

```rust
// DELETE these lines:
pub struct EciesKeypair {
    pub private_key_der: Option<Vec<u8>>,
    pub encryption_public_key: String,
}
```

Remove the `ecies_keypair` field from `AgentSshHandler` (line 36):

```rust
// Before:
pub struct AgentSshHandler {
    runtime: SshAgentRuntime<AgentSshRuntimeSupport>,
    ecies_keypair: Option<EciesKeypair>,
}

// After:
pub struct AgentSshHandler {
    runtime: SshAgentRuntime<AgentSshRuntimeSupport>,
}
```

Remove the `ecies_keypair` parameter from `AgentSshHandler::new` (line 44) and update the body:

```rust
// Before:
    pub fn new(
        db: DatabaseConnection,
        state_dir: PathBuf,
        mode: AgentSshMode,
        ecies_keypair: Option<EciesKeypair>,
    ) -> Self {
        // ...
        Self {
            runtime,
            ecies_keypair,
        }
    }

// After:
    pub fn new(
        db: DatabaseConnection,
        state_dir: PathBuf,
        mode: AgentSshMode,
    ) -> Self {
        // ...
        Self {
            runtime,
        }
    }
```

Also update the `pub use` re-export in `crates/core/agent-ssh-runtime/src/lib.rs` to remove `AgentSshMode` and `EciesKeypair` (both now deleted):

```rust
// Before (around line 1253):
pub use handler::{AgentSshHandler, AgentSshMode, EciesKeypair};

// After:
pub use handler::AgentSshHandler;
```

- [ ] **Step 2: Simplify `AgentSshHandler::on_connected`**

Replace the entire `on_connected` implementation (lines 88–116 in the original):

```rust
    async fn on_connected(
        &mut self,
        conn: &mut dyn ServiceTransport,
        identity: &ServiceIdentityState,
    ) -> LoopResult<()> {
        let enc_pub = identity
            .public_key_raw()
            .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes));
        self.runtime
            .on_connected(
                conn,
                SshAgentIdentity {
                    service_id: identity.service_id(),
                    private_key_der: identity.private_key_pkcs8_der(),
                    encryption_public_key: enc_pub,
                },
            )
            .await
            .map_err(|error| report!(LoopError::Other(error.to_string())))
    }
```

- [ ] **Step 3: Remove `persist_tenant_id` from `AgentSshHandler::on_settings`**

Replace the `on_settings` implementation (lines 127–147 in the original):

```rust
    async fn on_settings(
        &mut self,
        settings: &uptrakit_wire::ServiceSettingsPayload,
        conn: &mut dyn ServiceTransport,
        agreed_capabilities: &BTreeSet<Capability>,
    ) {
        if let Err(error) = self
            .runtime
            .apply_settings(
                SshAgentSettings {
                    tenant_id: settings.tenant_id,
                    ui_surfaces_enabled: agreed_capabilities.contains(&Capability::UiSurfaces),
                },
                conn,
            )
            .await
        {
            tracing::warn!(error = %error, "failed to apply SSH agent service settings");
        }
    }
```

- [ ] **Step 4: Remove `persist_tenant_id` from `SshAgentSettings` in `lib.rs`**

In `crates/core/agent-ssh-runtime/src/lib.rs`, update the `SshAgentSettings` struct:

```rust
// Before:
#[derive(Debug, Clone, Copy, Default)]
pub struct SshAgentSettings {
    pub tenant_id: Option<uuid::Uuid>,
    pub ui_surfaces_enabled: bool,
    pub persist_tenant_id: bool,
}

// After:
#[derive(Debug, Clone, Copy, Default)]
pub struct SshAgentSettings {
    pub tenant_id: Option<uuid::Uuid>,
    pub ui_surfaces_enabled: bool,
}
```

Search for all construction sites of `SshAgentSettings` in the codebase and remove the `persist_tenant_id` field:

```bash
grep -rn "persist_tenant_id" crates/core/agent-ssh-runtime/
```

Update every `SshAgentSettings { ... persist_tenant_id: ... }` construction to omit the field.

- [ ] **Step 5: Remove `persist_tenant_id` field from `AgentSshRuntimeSupport`**

In `crates/core/agent-ssh-runtime/src/runtime_support.rs`:

Remove the field from the struct:

```rust
// Before:
pub struct AgentSshRuntimeSupport {
    db: DatabaseConnection,
    state_dir: PathBuf,
    pool: ssh_pool::SshConnectionPool,
    surface_proxy: Arc<ServiceSurfaceProxy>,
    infra_bundles: Arc<Vec<InfraBundle>>,
    persist_tenant_id: bool,
}

// After:
pub struct AgentSshRuntimeSupport {
    db: DatabaseConnection,
    state_dir: PathBuf,
    pool: ssh_pool::SshConnectionPool,
    surface_proxy: Arc<ServiceSurfaceProxy>,
    infra_bundles: Arc<Vec<InfraBundle>>,
}
```

Remove the parameter from `new()` and update the body:

```rust
// Before:
    pub fn new(
        db: DatabaseConnection,
        state_dir: PathBuf,
        pool: ssh_pool::SshConnectionPool,
        surface_proxy: Arc<ServiceSurfaceProxy>,
        infra_bundles: Arc<Vec<InfraBundle>>,
        persist_tenant_id: bool,
    ) -> Self {
        Self {
            db,
            state_dir,
            pool,
            surface_proxy,
            infra_bundles,
            persist_tenant_id,
        }
    }

// After:
    pub fn new(
        db: DatabaseConnection,
        state_dir: PathBuf,
        pool: ssh_pool::SshConnectionPool,
        surface_proxy: Arc<ServiceSurfaceProxy>,
        infra_bundles: Arc<Vec<InfraBundle>>,
    ) -> Self {
        Self {
            db,
            state_dir,
            pool,
            surface_proxy,
            infra_bundles,
        }
    }
```

Make `persist_tenant_id()` unconditional — remove the early-return guard:

```rust
// Before:
    async fn persist_tenant_id(&self, tenant_id: uuid::Uuid) {
        if !self.persist_tenant_id {
            return;
        }
        let mut identity =
            uptrakit_service_sdk::ServiceIdentityState::new_single_dir(&self.state_dir);
        if let Err(error) = identity.load().await {
            tracing::warn!(error = %error, "failed to load identity for tenant_id persistence");
            return;
        }
        if let Err(error) = identity.save_tenant_id(tenant_id).await {
            tracing::warn!(error = %error, "failed to persist tenant_id to service.json");
        }
    }

// After:
    async fn persist_tenant_id(&self, tenant_id: uuid::Uuid) {
        let mut identity =
            uptrakit_service_sdk::ServiceIdentityState::new_single_dir(&self.state_dir);
        if let Err(error) = identity.load().await {
            tracing::warn!(error = %error, "failed to load identity for tenant_id persistence");
            return;
        }
        if let Err(error) = identity.save_tenant_id(tenant_id).await {
            tracing::warn!(error = %error, "failed to persist tenant_id to service.json");
        }
    }
```

> **Why this is safe for embedded:** `state_dir` for embedded services contains no `service.json` (never enrolled). `load()` succeeds with all
> fields `None`. `save_tenant_id()` sees `self.service_id = None` and returns `Ok(())` immediately — no disk write. Same code path, naturally no-op
> behaviour.

- [ ] **Step 6: Remove `mode` parameter from `AgentSshHandler::new()` and update call sites**

`mode` was only used to compute `is_standalone`. Remove both. Update `handler.rs` lines 59–67:

```rust
// Before:
        let is_standalone = matches!(mode, AgentSshMode::Binary);
        let support = AgentSshRuntimeSupport::new(
            db,
            state_dir.clone(),
            SshConnectionPool::new(),
            surface_proxy,
            infra_bundles,
            is_standalone,
        );

// After:
        let support = AgentSshRuntimeSupport::new(
            db,
            state_dir.clone(),
            SshConnectionPool::new(),
            surface_proxy,
            infra_bundles,
        );
```

Remove the `mode` parameter from `AgentSshHandler::new()` entirely:

```rust
    pub fn new(
        db: DatabaseConnection,
        state_dir: PathBuf,
    ) -> Self {
```

Find all callers to update:

```bash
grep -rn "AgentSshHandler::new" crates/
```

The standalone binary at `crates/core/agent-ssh/src/main.rs` needs two changes:

Line 8 — remove `AgentSshMode` from the import:

```rust
// Before:
use uptrakit_agent_ssh_runtime::{AgentSshHandler, AgentSshMode, db, init_ssh_data_key_ring, reencrypt_ssh_to_v3, ...};

// After (remove AgentSshMode):
use uptrakit_agent_ssh_runtime::{AgentSshHandler, db, init_ssh_data_key_ring, reencrypt_ssh_to_v3, ...};
```

Line 120 — remove `mode` and `ecies_keypair` arguments:

```rust
// Before:
let mut handler = AgentSshHandler::new(local_db, state_dir, AgentSshMode::Binary, None);

// After:
let mut handler = AgentSshHandler::new(local_db, state_dir);
```

Update every other call site to remove the `mode` and `ecies_keypair` arguments.

- [ ] **Step 7: Remove `generate_ecies_keypair()` from `ssh_agent/mod.rs`**

In `crates/core/controller-runtime/src/ssh_agent/mod.rs`, delete the entire `generate_ecies_keypair` function. The file currently contains only the
`ssh_agent_capabilities()` function and `generate_ecies_keypair`. After deletion, the file should contain only `ssh_agent_capabilities()`.

- [ ] **Step 8: Update `register_agent_ssh` in `builtins.rs`**

Remove the ECIES keypair generation and simplify the handler construction:

```rust
// Before:
    let (private_key_der, encryption_public_key) = crate::ssh_agent::generate_ecies_keypair()?;
    let keypair = uptrakit_agent_ssh_runtime::EciesKeypair {
        private_key_der,
        encryption_public_key,
    };
    let handler = uptrakit_agent_ssh_runtime::AgentSshHandler::new(
        db_for_ssh,
        state_dir,
        uptrakit_agent_ssh_runtime::AgentSshMode::Embedded,
        Some(keypair),
    );

// After:
    let handler = uptrakit_agent_ssh_runtime::AgentSshHandler::new(
        db_for_ssh,
        state_dir,
    );
```

Also remove any `use uptrakit_agent_ssh_runtime::EciesKeypair;` or `use uptrakit_agent_ssh_runtime::AgentSshMode;` imports from `builtins.rs` if
they become unused.

- [ ] **Step 9: Verify compilation**

```bash
cargo check --all-features 2>&1 | grep "^error" | head -20
```

Expected: no errors.

- [ ] **Step 10: Run agent-ssh-runtime tests**

```bash
cargo test -p uptrakit-agent-ssh-runtime --all-features -- --nocapture 2>&1 | tail -20
```

Expected: all pass.

- [ ] **Step 11: Commit**

```bash
git add crates/core/agent-ssh-runtime/src/handler.rs \
        crates/core/agent-ssh-runtime/src/lib.rs \
        crates/core/agent-ssh-runtime/src/runtime_support.rs \
        crates/core/controller-runtime/src/ssh_agent/mod.rs \
        crates/core/controller-runtime/src/service_host/builtins.rs
git commit -m "refactor(agent-ssh): remove EciesKeypair workaround; unify persist_tenant_id

Remove EciesKeypair struct, field, and on_connected branch — identity now
always derived from the ServiceIdentityState passed by the SDK. Remove
persist_tenant_id: bool flag from SshAgentSettings and AgentSshRuntimeSupport;
persist_tenant_id() is now unconditional (safe no-op for embedded via natural
None guard in save_tenant_id). Remove generate_ecies_keypair() from controller.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

### Task 4: MQTT handler cleanup + integration test rewrite

**Files:**

- Modify: `crates/core/mqtt-runtime/src/handler.rs`
- Modify: `crates/core/controller-runtime/src/mqtt/mod.rs`
- Modify: `crates/core/controller-runtime/src/service_host/builtins.rs`
- Test: `crates/core/mqtt-runtime/src/handler.rs` (test module)

- [ ] **Step 1: Remove `embedded_identity` field and `new_embedded()` from `MqttHandler`**

In `crates/core/mqtt-runtime/src/handler.rs`:

Remove the `embedded_identity` field from the struct:

```rust
// Before:
pub struct MqttHandler {
    runtime: MqttRuntime,
    embedded_identity: Option<MqttRuntimeIdentity>,
}

// After:
pub struct MqttHandler {
    runtime: MqttRuntime,
}
```

Remove the `new_embedded()` constructor (lines 32–37):

```rust
// DELETE these lines:
    pub fn new_embedded(identity: MqttRuntimeIdentity) -> Self {
        Self {
            runtime: MqttRuntime::new(),
            embedded_identity: Some(identity),
        }
    }
```

Update `new()` to remove the now-orphaned field initialization:

```rust
    pub fn new() -> Self {
        Self {
            runtime: MqttRuntime::new(),
        }
    }
```

`Default` impl is unchanged (calls `Self::new()`).

- [ ] **Step 2: Remove `on_settings` workaround in `MqttHandler`**

Replace the `on_settings` implementation (lines 98–119 in the original):

```rust
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
```

- [ ] **Step 3: Remove `generate_ecies_keypair()` from `mqtt/mod.rs`**

In `crates/core/controller-runtime/src/mqtt/mod.rs`, delete the `generate_ecies_keypair` function (lines 57–68 in the original):

```rust
// DELETE:
pub(crate) fn generate_ecies_keypair() -> rootcause::Result<MqttRuntimeIdentity> {
    use rootcause::prelude::*;
    let (private_der, public_b64) = uptrakit_service_sdk::generate_p256_keypair_for_ecies()
        .map_err(|e| {
            report!(std::io::Error::other(format!(
                "embedded MQTT: ECIES keygen failed: {e}"
            )))
        })?;
    Ok(MqttRuntimeIdentity {
        service_id: None,
        private_key_der: Some(private_der),
        encryption_public_key: Some(public_b64),
    })
}
```

Also remove the `use uptrakit_mqtt_runtime::{MqttRuntimeIdentity, ...}` import if `MqttRuntimeIdentity` is no longer used anywhere in the file.

- [ ] **Step 4: Update `register_mqtt` in `builtins.rs`**

Remove the identity generation and simplify the handler construction:

```rust
// Before:
    let identity = crate::mqtt::generate_ecies_keypair()?;
    let handler = uptrakit_mqtt_runtime::MqttHandler::new_embedded(identity);

// After:
    let handler = uptrakit_mqtt_runtime::MqttHandler::new();
```

`host.add(...).await?` is still present in `register_mqtt`, so the function signature `-> rootcause::Result<()>` is unchanged.

- [ ] **Step 5: Fix `drain_shutdown_sends_disconnecting` test**

In `crates/core/mqtt-runtime/src/handler.rs`, the test at line ~314 also constructs `MqttHandler::new_embedded(identity)`. Update it to use
`MqttHandler::new()`:

```rust
// Before:
let identity = make_identity();
let mut handler = MqttHandler::new_embedded(identity);

// After:
let mut handler = MqttHandler::new();
```

Remove any call to `make_identity()` inside this test. If `make_identity()` is only used in `drain_shutdown_sends_disconnecting` and
`embedded_mqtt_registers_surface_with_default_tenant_binding`, delete the helper entirely after updating both tests in this task.

Run the test to confirm it still passes:

```bash
cargo test -p uptrakit-mqtt-runtime drain_shutdown_sends_disconnecting -- --nocapture
```

Expected: PASS.

- [ ] **Step 6: Rewrite `embedded_mqtt_registers_surface_with_default_tenant_binding` test**

Find the test in `crates/core/mqtt-runtime/src/handler.rs`. The test currently calls `MqttHandler::new_embedded(identity)`. Rewrite it to use
`MqttHandler::new()` and remove the `make_identity()` setup (also delete the helper if it was already removed in Step 5):

```rust
// Find and remove the make_identity helper if it's only used in this test:
// fn make_identity() -> MqttRuntimeIdentity { ... }  ← DELETE

// Update the test:
#[tokio::test]
async fn embedded_mqtt_registers_surface_with_default_tenant_binding() {
    // ... (keep existing test setup for transport/state) ...
    let handler = MqttHandler::new(); // was: MqttHandler::new_embedded(make_identity())
    // ... rest of test unchanged ...
}
```

The test must pass end-to-end. Read the test's existing assertions before modifying — they may already verify surface registration. The critical
invariant to preserve: `on_connected` is called, which sends `ServiceMessage::Register` through the bridge, which marks the service active in
`ServiceConnectionRegistry` before `on_settings` runs. If the test's existing assertions don't cover this ordering, add a log assertion:

```rust
// Verify on_connected was reached (confirms Register was sent through bridge):
assert!(
    log.lock().call_order.contains(&"on_connected"),
    "on_connected must have been called before on_settings"
);
```

Adapt the variable names to match the test's existing mock infrastructure.

- [ ] **Step 7: Verify full workspace compilation and tests**

```bash
cargo check --all-features 2>&1 | grep "^error" | head -20
cargo test -p uptrakit-mqtt-runtime --all-features -- --nocapture 2>&1 | tail -30
cargo test -p uptrakit-controller-runtime --all-features -- --nocapture 2>&1 | tail -20
```

Expected: clean compile, all tests pass.

- [ ] **Step 8: Run full quality gate**

```bash
cargo fmt --all
cargo clippy --all-targets --all-features 2>&1 | grep "^error" | head -20
cargo test --all-features 2>&1 | grep -E "^(test .* FAILED|FAILED|error)" | head -20
```

Expected: no clippy errors, no test failures.

- [ ] **Step 9: Commit**

```bash
git add crates/core/mqtt-runtime/src/handler.rs \
        crates/core/controller-runtime/src/mqtt/mod.rs \
        crates/core/controller-runtime/src/service_host/builtins.rs
git commit -m "refactor(mqtt): remove embedded_identity workaround; SDK supplies identity

Remove embedded_identity field, new_embedded() constructor, and on_settings
workaround from MqttHandler. SDK now calls on_connected with identity before
on_settings — identical to standalone path. Remove generate_ecies_keypair()
from controller. Rewrite embedded_mqtt_registers_surface_with_default_tenant_binding
test to use MqttHandler::new().

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```
