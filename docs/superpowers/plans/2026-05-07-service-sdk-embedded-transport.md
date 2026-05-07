# service-sdk Embedded Transport Abstraction — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `ServiceHandler` trait methods accept `&mut dyn ServiceTransport` instead of
`&mut ControllerConnection`, and add `run_embedded_service` to `uptrakit-service-sdk` so the
same `ServiceHandler` implementation runs in both standalone (WebSocket) and embedded
(in-process) modes.

**Architecture:** Change all `ServiceHandler` method signatures (6 conn params + new
`on_yield_change` method + `on_settings` agreed-capabilities parameter) in
`shared_types.rs`; update call sites in `event_loop.rs` to coerce `&mut ControllerConnection`
to `&mut dyn ServiceTransport`; add `run_embedded_service` in a new `embedded.rs` file with
startup timeout, two-phase select, and `EmbeddedDrain` shutdown cause. The controller side
sends `ServiceSettings` over the in-process channel immediately after spawning the service.

**Tech Stack:** Rust 2024, `tokio`, `tokio_util::sync::CancellationToken`, `async_trait`,
`rootcause`, `uptrakit-wire` (`ServiceTransport`, `ServiceSettingsPayload`,
`CancellationToken`).

---

## File Map

| Action | File                                                     | Responsibility                                                                                                                  |
| ------ | -------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| Modify | `crates/shared/wire/Cargo.toml`                          | Add `"sync"` to tokio features for `Arc<Notify>`                                                                                |
| Modify | `crates/shared/wire/src/transport.rs`                    | Add `yield_change_notifier` default method to `ServiceTransport`                                                                |
| Modify | `crates/shared/service-sdk/src/shared_types.rs`          | `ServiceHandler` trait: all conn params, `on_settings`, `on_yield_change`, `ShutdownCause::EmbeddedDrain` + `#[non_exhaustive]` |
| Modify | `crates/shared/service-sdk/src/lifecycle.rs`             | Add `EmbeddedDrain` arm to `default_resolve_shutdown`                                                                           |
| Modify | `crates/shared/service-sdk/src/event_loop.rs`            | Coerce `conn as &mut dyn ServiceTransport` at all handler call sites                                                            |
| Create | `crates/shared/service-sdk/src/embedded.rs`              | `run_embedded_service` — startup timeout, two-phase select loop                                                                 |
| Modify | `crates/shared/service-sdk/src/lib.rs`                   | Export `run_embedded_service`                                                                                                   |
| Modify | `crates/core/agent/src/main.rs`                          | `AgentHandler`: update conn param types                                                                                         |
| Modify | `crates/core/mqtt/src/main.rs`                           | `StandaloneMqttHandler`: update conn params, agreed-caps, `on_yield_change`                                                     |
| Modify | `crates/core/scheduler-runtime/src/standalone.rs`        | `StandaloneSchedulerHandler`: update all params, `conn.send` → `transport_send`, `EmbeddedDrain` match arm                      |
| Modify | `crates/core/controller-runtime/src/embedded/types.rs`   | Override `yield_change_notifier` on `EmbeddedTransport`                                                                         |
| Modify | `crates/core/controller-runtime/src/embedded/mod.rs`     | Send `ServiceSettings` after forwarder spawn                                                                                    |
| Create | `docs/adr/0003-service-handler-transport-abstraction.md` | ADR: why `dyn ServiceTransport` not `ControllerConnection`                                                                      |
| Modify | `docs/development/coding-standards.md`                   | Add `ServiceHandler` transport-abstraction rule                                                                                 |

---

## Task 1: Wire — `yield_change_notifier` on `ServiceTransport`

**Files:**

- Modify: `crates/shared/wire/Cargo.toml`
- Modify: `crates/shared/wire/src/transport.rs`

- [ ] **Step 1: Add `"sync"` to tokio features in `uptrakit-wire/Cargo.toml`**

  Change line 18 from:

  ```toml
  tokio = { workspace = true, features = ["time"] }
  ```

  to:

  ```toml
  tokio = { workspace = true, features = ["sync", "time"] }
  ```

- [ ] **Step 2: Add `use std::sync::Arc;` import at the top of `transport.rs`**

  After the existing imports (line 35–36):

  ```rust
  use std::sync::Arc;

  use crate::CloseReason;
  use crate::messages::{ControllerMessage, ServiceMessage};
  ```

- [ ] **Step 3: Write the failing test first (in `transport.rs` `#[cfg(test)]` block)**

  The existing `DefaultTransport` in the tests module does not override
  `yield_change_notifier`, so calling it on it returns `None`. Add a test verifying this:

  ```rust
  #[test]
  fn default_yield_change_notifier_returns_none() {
      let t = DefaultTransport;
      assert!(t.yield_change_notifier().is_none());
  }
  ```

- [ ] **Step 4: Run the test — expect FAIL (method doesn't exist yet)**

  ```bash
  cargo test -p uptrakit-wire -- default_yield_change_notifier
  ```

  Expected: compile error `no method named yield_change_notifier`.

- [ ] **Step 5: Add `yield_change_notifier` default method to `ServiceTransport`**

  Insert after the `is_yielded` method (currently ends at line 106) and before the closing
  `}` of the trait:

  ```rust
  /// Optional notifier fired when the yield state changes.
  ///
  /// Returns `Some` only for transports that support embedded yield signalling
  /// (i.e. `EmbeddedTransport`). `ControllerConnection` uses the default `None`.
  ///
  /// Called once before the `run_embedded_service` loop to obtain a stable
  /// notifier handle. The `Arc<Notify>` is valid for the transport lifetime.
  fn yield_change_notifier(&self) -> Option<Arc<tokio::sync::Notify>> {
      None
  }
  ```

- [ ] **Step 6: Run the test — expect PASS**

  ```bash
  cargo test -p uptrakit-wire -- default_yield_change_notifier
  ```

  Expected: `test transport::tests::default_yield_change_notifier_returns_none ... ok`

- [ ] **Step 7: Verify full wire crate test suite still passes**

  ```bash
  cargo test -p uptrakit-wire --all-features
  ```

  Expected: all tests pass, no new failures.

- [ ] **Step 8: Commit**

  ```bash
  git add crates/shared/wire/Cargo.toml crates/shared/wire/src/transport.rs
  git commit -m "feat(wire): add yield_change_notifier default method to ServiceTransport"
  ```

---

## Task 2: `ShutdownCause::EmbeddedDrain` + `#[non_exhaustive]` + lifecycle

**Files:**

- Modify: `crates/shared/service-sdk/src/shared_types.rs`
- Modify: `crates/shared/service-sdk/src/lifecycle.rs`

- [ ] **Step 1: Write the failing test for `EmbeddedDrain` resolution**

  In `crates/shared/service-sdk/src/lifecycle.rs`, in the `tests` module (around line 547),
  add:

  ```rust
  #[test]
  fn default_resolve_shutdown_embedded_drain() {
      let (reason, outcome) = default_resolve_shutdown(ShutdownCause::EmbeddedDrain);
      assert_eq!(reason, DisconnectReason::Shutdown);
      assert_eq!(outcome, LoopOutcome::Shutdown);
  }
  ```

- [ ] **Step 2: Run the test — expect FAIL**

  ```bash
  cargo test -p uptrakit-service-sdk -- default_resolve_shutdown_embedded_drain
  ```

  Expected: compile error `no variant EmbeddedDrain`.

- [ ] **Step 3: Add `#[non_exhaustive]` and `EmbeddedDrain` to `ShutdownCause`**

  In `crates/shared/service-sdk/src/shared_types.rs`, change the `ShutdownCause` definition
  (currently at line 92–99) from:

  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum ShutdownCause {
      /// An OS signal was received (`SIGINT`, `SIGTERM`, `SIGHUP`).
      Signal(Signal),
      /// The controller sent `ServerRestarting`; the service should disconnect
      /// and reconnect once the controller is available again.
      ServerRestarting,
  }
  ```

  to:

  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  #[non_exhaustive]
  pub enum ShutdownCause {
      /// An OS signal was received (`SIGINT`, `SIGTERM`, `SIGHUP`).
      Signal(Signal),
      /// The controller sent `ServerRestarting`; the service should disconnect
      /// and reconnect once the controller is available again.
      ServerRestarting,
      /// The embedded drain token was cancelled; graceful shutdown of an
      /// in-process embedded service.
      EmbeddedDrain,
  }
  ```

  Also update the doc-comment table on `ShutdownCause` to add the new row:

  ```text
  /// | `EmbeddedDrain` | `Shutdown` | `Shutdown` |
  ```

- [ ] **Step 4: Add `EmbeddedDrain` arm to `default_resolve_shutdown` in `lifecycle.rs`**

  The function currently (line 29–38) reads:

  ```rust
  match cause {
      ShutdownCause::Signal(Signal::Hangup) => (DisconnectReason::Restart, LoopOutcome::Restart),
      ShutdownCause::Signal(_) => (DisconnectReason::Shutdown, LoopOutcome::Shutdown),
      ShutdownCause::ServerRestarting => (DisconnectReason::Restart, LoopOutcome::Disconnected),
  }
  ```

  Change to (note: within the same crate, `#[non_exhaustive]` does not require a wildcard):

  ```rust
  match cause {
      ShutdownCause::Signal(Signal::Hangup) => (DisconnectReason::Restart, LoopOutcome::Restart),
      ShutdownCause::Signal(_) => (DisconnectReason::Shutdown, LoopOutcome::Shutdown),
      ShutdownCause::ServerRestarting => (DisconnectReason::Restart, LoopOutcome::Disconnected),
      ShutdownCause::EmbeddedDrain => (DisconnectReason::Shutdown, LoopOutcome::Shutdown),
  }
  ```

- [ ] **Step 5: Run the test — expect PASS**

  ```bash
  cargo test -p uptrakit-service-sdk -- default_resolve_shutdown
  ```

  Expected: all four `default_resolve_shutdown_*` tests pass.

- [ ] **Step 6: Verify full service-sdk test suite**

  ```bash
  cargo test -p uptrakit-service-sdk --all-features
  ```

  Expected: all tests pass.

- [ ] **Step 7: Commit**

  ```bash
  git add crates/shared/service-sdk/src/shared_types.rs \
          crates/shared/service-sdk/src/lifecycle.rs
  git commit -m "feat(service-sdk): add ShutdownCause::EmbeddedDrain with non_exhaustive"
  ```

---

## Task 3: `ServiceHandler` Trait — Signature Changes

**Files:**

- Modify: `crates/shared/service-sdk/src/shared_types.rs`

**Note:** This task changes the `ServiceHandler` trait in a way that breaks all implementors.
Do NOT run `cargo check --all-features` until Task 7 is complete. Only run
`cargo check -p uptrakit-service-sdk` after this task.

- [ ] **Step 1: Update imports in `shared_types.rs`**

  Remove `use crate::connection::ControllerConnection;` (line 21).
  Add `ServiceTransport` to the `crate::wire_api` import:

  ```rust
  use crate::wire_api::{
      Capability, ControllerMessage, ServiceMessage, ServiceSettingsPayload, ServiceTransport,
      surfaces::{
          SurfaceActionError, SurfaceActionErrorCode, SurfaceActionRequest, SurfaceActionResponse,
      },
  };
  ```

- [ ] **Step 2: Update `on_connected` signature and add embedded-mode note**

  Change (around line 158–162):

  ```rust
  async fn on_connected(
      &mut self,
      conn: &mut ControllerConnection,
      identity: &ServiceIdentityState,
  ) -> LoopResult<()>;
  ```

  to (also add the `/// Note` line to the existing doc-comment block):

  ```rust
  /// Called when the service establishes a WebSocket connection to the controller.
  ///
  /// Note: **not called** by `run_embedded_service`. Embedded handlers must perform
  /// any initialization that would normally happen here inside their constructor.
  async fn on_connected(
      &mut self,
      conn: &mut dyn ServiceTransport,
      identity: &ServiceIdentityState,
  ) -> LoopResult<()>;
  ```

- [ ] **Step 3: Update `on_message` signature**

  Change (around line 171–175):

  ```rust
  async fn on_message(
      &mut self,
      msg: ControllerMessage,
      conn: &mut ControllerConnection,
  ) -> LoopResult<Option<LoopOutcome>>;
  ```

  to:

  ```rust
  async fn on_message(
      &mut self,
      msg: ControllerMessage,
      conn: &mut dyn ServiceTransport,
  ) -> LoopResult<Option<LoopOutcome>>;
  ```

- [ ] **Step 4: Update `on_settings` signature and add `agreed_capabilities` parameter**

  Change (around line 184–189):

  ```rust
  async fn on_settings(
      &mut self,
      _settings: &ServiceSettingsPayload,
      _conn: &mut ControllerConnection,
  ) {
  }
  ```

  to:

  ```rust
  async fn on_settings(
      &mut self,
      _settings: &ServiceSettingsPayload,
      _conn: &mut dyn ServiceTransport,
      _agreed_capabilities: &BTreeSet<Capability>,
  ) {
  }
  ```

- [ ] **Step 5: Update `on_service_event` signature**

  Change (around line 213–217):

  ```rust
  async fn on_service_event(
      &mut self,
      event: Self::ServiceEvent,
      conn: &mut ControllerConnection,
  ) -> LoopResult<Option<LoopOutcome>>;
  ```

  to:

  ```rust
  async fn on_service_event(
      &mut self,
      event: Self::ServiceEvent,
      conn: &mut dyn ServiceTransport,
  ) -> LoopResult<Option<LoopOutcome>>;
  ```

- [ ] **Step 6: Update `on_surface_action_request` signature and default body**

  Change (around line 241–264):

  ```rust
  async fn on_surface_action_request(
      &mut self,
      request: SurfaceActionRequest,
      conn: &mut ControllerConnection,
  ) -> LoopResult<()> {
      let response = SurfaceActionResponse {
          request_id: request.request_id,
          success: false,
          result: None,
          error: Some(SurfaceActionError {
              code: SurfaceActionErrorCode::UnsupportedCapability,
              message: "Surface actions not supported by this service".to_owned(),
              details: None,
          }),
      };
      conn.send(ServiceMessage::SurfaceActionResponse(response))
          .await
          .map_err(|e| {
              report!(LoopError::Other(format!(
                  "failed to send surface action response: {e}"
              )))
          })?;
      Ok(())
  }
  ```

  to:

  ```rust
  async fn on_surface_action_request(
      &mut self,
      request: SurfaceActionRequest,
      conn: &mut dyn ServiceTransport,
  ) -> LoopResult<()> {
      let response = SurfaceActionResponse {
          request_id: request.request_id,
          success: false,
          result: None,
          error: Some(SurfaceActionError {
              code: SurfaceActionErrorCode::UnsupportedCapability,
              message: "Surface actions not supported by this service".to_owned(),
              details: None,
          }),
      };
      conn.transport_send(ServiceMessage::SurfaceActionResponse(response))
          .await
          .map_err(|e| {
              report!(LoopError::Other(format!(
                  "failed to send surface action response: {e}"
              )))
          })?;
      Ok(())
  }
  ```

- [ ] **Step 7: Update `on_shutdown` signature**

  Change (around line 282–287):

  ```rust
  async fn on_shutdown(
      &mut self,
      conn: &mut ControllerConnection,
      cause: ShutdownCause,
      shutdown_timeout: Duration,
  ) -> LoopOutcome;
  ```

  to:

  ```rust
  async fn on_shutdown(
      &mut self,
      conn: &mut dyn ServiceTransport,
      cause: ShutdownCause,
      shutdown_timeout: Duration,
  ) -> LoopOutcome;
  ```

- [ ] **Step 8: Add `on_yield_change` default method**

  Insert after `on_service_config_ack` (currently around line 226) and before
  `on_surface_action_request`:

  ```rust
  /// Called when the embedded yield state changes.
  ///
  /// Only invoked by `run_embedded_service`. The default no-op is appropriate
  /// for services that drop messages silently when yielded. MQTT overrides
  /// this to call `runtime.handle_yield_change()` for reconnect-storm logic.
  async fn on_yield_change(&mut self, _is_yielded: bool, _conn: &mut dyn ServiceTransport) {}
  ```

- [ ] **Step 9: Verify `uptrakit-service-sdk` itself still compiles (other crates will fail)**

  ```bash
  cargo check -p uptrakit-service-sdk --all-features
  ```

  Expected: PASS. (External crates fail because their `impl ServiceHandler` now have wrong
  signatures — that is expected and will be fixed in Tasks 4–7.)

- [ ] **Step 10: Commit**

  ```bash
  git add crates/shared/service-sdk/src/shared_types.rs
  git commit -m "feat(service-sdk): change ServiceHandler conn params to dyn ServiceTransport"
  ```

---

## Task 4: Standalone Event Loop — Call Site Coercions

**Files:**

- Modify: `crates/shared/service-sdk/src/event_loop.rs`

**Note:** Do NOT run `cargo check --all-features` until Task 7 is complete. Only run
`cargo check -p uptrakit-service-sdk` after this task.

- [ ] **Step 1: Add `ServiceTransport` to imports**

  In `event_loop.rs`, update the `crate::wire_api` import (around line 19–21) to add
  `ServiceTransport`:

  ```rust
  use crate::wire_api::{
      Capability, CloseReason, ControllerMessage, PingPayload, ServiceMessage,
      ServiceSettingsPayload, ServiceTransport, now_millis,
  };
  ```

- [ ] **Step 2: Update `on_connected` call site (line 119)**

  Change:

  ```rust
  handler.on_connected(conn, identity).await?;
  ```

  to:

  ```rust
  handler.on_connected(conn as &mut dyn ServiceTransport, identity).await?;
  ```

- [ ] **Step 3: Update `on_service_event` call site (line 146)**

  Change:

  ```rust
  match handler.on_service_event(event, conn).await? {
  ```

  to:

  ```rust
  match handler.on_service_event(event, conn as &mut dyn ServiceTransport).await? {
  ```

- [ ] **Step 4: Update `on_shutdown` (signal arm, around line 202–206)**

  Change:

  ```rust
  break handler
      .on_shutdown(
          conn,
          ShutdownCause::Signal(signal),
          shutdown_timeout,
      )
      .await;
  ```

  to:

  ```rust
  break handler
      .on_shutdown(
          conn as &mut dyn ServiceTransport,
          ShutdownCause::Signal(signal),
          shutdown_timeout,
      )
      .await;
  ```

- [ ] **Step 5: Update `on_shutdown` inside `handle_controller_message` (ServerRestarting arm, around line 430–434)**

  Change:

  ```rust
  let outcome = handler
      .on_shutdown(
          conn,
          ShutdownCause::ServerRestarting,
          *loop_state.shutdown_timeout,
      )
      .await;
  ```

  to:

  ```rust
  let outcome = handler
      .on_shutdown(
          conn as &mut dyn ServiceTransport,
          ShutdownCause::ServerRestarting,
          *loop_state.shutdown_timeout,
      )
      .await;
  ```

- [ ] **Step 6: Update `on_surface_action_request` call site (around line 439)**

  Change:

  ```rust
  handler.on_surface_action_request(payload, conn).await?;
  ```

  to:

  ```rust
  handler.on_surface_action_request(payload, conn as &mut dyn ServiceTransport).await?;
  ```

- [ ] **Step 7: Update `on_message` call site (around line 457)**

  Change:

  ```rust
  Some(msg) => handler.on_message(msg, conn).await,
  ```

  to:

  ```rust
  Some(msg) => handler.on_message(msg, conn as &mut dyn ServiceTransport).await,
  ```

- [ ] **Step 8: Update `process_service_settings` to pass agreed capabilities**

  The function currently (around line 465–490) computes `agreed` and calls
  `handler.on_settings(settings, conn).await`. Change that call to pass `&agreed`:

  ```rust
  handler.on_settings(settings, conn as &mut dyn ServiceTransport, &agreed).await;
  ```

- [ ] **Step 9: Compile-check `uptrakit-service-sdk` (still broken externally)**

  ```bash
  cargo check -p uptrakit-service-sdk --all-features
  ```

  Expected: PASS. (External handler impls still break — fixed in Tasks 5–7.)

- [ ] **Step 10: Commit**

  ```bash
  git add crates/shared/service-sdk/src/event_loop.rs
  git commit -m "refactor(service-sdk): coerce ControllerConnection to dyn ServiceTransport at handler call sites"
  ```

---

## Task 5: `AgentHandler` Migration

**Files:**

- Modify: `crates/core/agent/src/main.rs`

**Note:** All `impl ServiceHandler for AgentHandler` steps below update method signatures only.
Preserve the existing `#[async_trait::async_trait]` attribute on the `impl` block — do not remove it.
Do NOT run `cargo check --all-features` until Task 7 is complete.

- [ ] **Step 1: Update imports**

  In `crates/core/agent/src/main.rs`, remove `ControllerConnection` from the
  `uptrakit_service_sdk` import and add `ServiceTransport` from `uptrakit_wire`:

  ```rust
  use uptrakit_agent_runtime::{
      AgentRuntime, AgentRuntimeConfig, AgentRuntimeEvent, agent_capabilities, make_local_executor,
  };
  use uptrakit_audit_log::RuntimeAuditEmitter;
  use uptrakit_service_sdk::{
      LoopError, LoopOutcome, LoopResult, ServiceHandler, ServiceIdentityState,
      ShutdownCause, default_resolve_shutdown,
  };
  use uptrakit_wire::{Capability, ServiceTransport};
  ```

- [ ] **Step 2: Update `on_connected`**

  `AgentRuntime::on_connected` already takes `&mut dyn ServiceTransport`. Remove the
  coercion that the old handler had to do:

  ```rust
  async fn on_connected(
      &mut self,
      conn: &mut dyn ServiceTransport,
      _identity: &ServiceIdentityState,
  ) -> LoopResult<()> {
      self.runtime
          .on_connected(conn)
          .await
          .map_err(|error| rootcause::Report::new(LoopError::Other(error.to_string())))
  }
  ```

- [ ] **Step 3: Update `on_settings`**

  `AgentRuntime::send_pending_initial_report` already takes `&mut dyn ServiceTransport`:

  ```rust
  async fn on_settings(
      &mut self,
      _settings: &uptrakit_wire::ServiceSettingsPayload,
      conn: &mut dyn ServiceTransport,
      _agreed_capabilities: &std::collections::BTreeSet<Capability>,
  ) {
      if let Err(error) = self.runtime.send_pending_initial_report(conn).await {
          tracing::warn!(error = %error, "failed to send initial ReportHosts message");
      }
  }
  ```

- [ ] **Step 4: Update `on_message`**

  `AgentRuntime::handle_controller_message` already takes `&mut dyn ServiceTransport`:

  ```rust
  async fn on_message(
      &mut self,
      msg: uptrakit_wire::ControllerMessage,
      conn: &mut dyn ServiceTransport,
  ) -> LoopResult<Option<LoopOutcome>> {
      self.runtime.handle_controller_message(msg, conn).await;
      Ok(None)
  }
  ```

- [ ] **Step 5: Update `on_service_event`**

  `AgentRuntime::handle_event` already takes `&mut dyn ServiceTransport`:

  ```rust
  async fn on_service_event(
      &mut self,
      event: Self::ServiceEvent,
      conn: &mut dyn ServiceTransport,
  ) -> LoopResult<Option<LoopOutcome>> {
      Ok(self.runtime.handle_event(event, conn).await)
  }
  ```

- [ ] **Step 6: Update `on_shutdown`**

  `AgentRuntime::shutdown` already takes `&mut dyn ServiceTransport`:

  ```rust
  async fn on_shutdown(
      &mut self,
      conn: &mut dyn ServiceTransport,
      cause: ShutdownCause,
      shutdown_timeout: std::time::Duration,
  ) -> LoopOutcome {
      let (disconnect_reason, outcome) = default_resolve_shutdown(cause);
      self.runtime
          .shutdown(conn, shutdown_timeout, disconnect_reason, outcome)
          .await
  }
  ```

- [ ] **Step 7: Compile-check agent crate**

  ```bash
  cargo check -p uptrakit-agent --all-features
  ```

  Expected: PASS.

- [ ] **Step 8: Commit**

  ```bash
  git add crates/core/agent/src/main.rs
  git commit -m "refactor(agent): migrate AgentHandler to dyn ServiceTransport"
  ```

---

## Task 6: `StandaloneMqttHandler` Migration

**Files:**

- Modify: `crates/core/mqtt/src/main.rs`

**Note:** All `impl ServiceHandler for StandaloneMqttHandler` steps below update method signatures
only. Preserve the existing `#[async_trait::async_trait]` attribute on the `impl` block — do not remove it.
Do NOT run `cargo check --all-features` until Task 7 is complete.

- [ ] **Step 1: Update imports**

  Remove `ControllerConnection` from the `uptrakit_service_sdk` import; add
  `ServiceTransport` from `uptrakit_wire`:

  ```rust
  use uptrakit_service_sdk::{
      LoopError, LoopOutcome, LoopResult, ServiceHandler, ServiceIdentityState,
      ShutdownCause, default_resolve_shutdown,
  };
  use uptrakit_wire::{Capability, ControllerMessage, ServiceTransport};
  ```

- [ ] **Step 2: Update `on_connected`**

  `MqttRuntime::on_connected` already takes `&mut dyn ServiceTransport`. The identity
  fields (service_id, private_key_der, encryption_public_key) come from the `identity`
  parameter — no change to logic:

  ```rust
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
  ```

- [ ] **Step 3: Update `on_message`**

  `MqttRuntime::handle_controller_message` already takes `&mut dyn ServiceTransport`:

  ```rust
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
  ```

- [ ] **Step 4: Update `on_settings` — switch from `conn.agreed_capabilities()` to parameter**

  The key change: `conn.agreed_capabilities().contains(...)` becomes
  `agreed_capabilities.contains(...)`:

  ```rust
  async fn on_settings(
      &mut self,
      settings: &uptrakit_wire::ServiceSettingsPayload,
      conn: &mut dyn ServiceTransport,
      agreed_capabilities: &std::collections::BTreeSet<Capability>,
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

- [ ] **Step 5: Update `on_service_event`**

  `MqttRuntime::handle_event` already takes `&mut dyn ServiceTransport`:

  ```rust
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
  ```

- [ ] **Step 6: Update `on_surface_action_request`**

  `MqttRuntime::handle_controller_message` already takes `&mut dyn ServiceTransport`:

  ```rust
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
  ```

- [ ] **Step 7: Update `on_shutdown`**

  `MqttRuntime::shutdown` already takes `&mut dyn ServiceTransport`:

  ```rust
  async fn on_shutdown(
      &mut self,
      conn: &mut dyn ServiceTransport,
      cause: ShutdownCause,
      _shutdown_timeout: std::time::Duration,
  ) -> LoopOutcome {
      let (reason, outcome) = default_resolve_shutdown(cause);
      self.runtime.shutdown(conn, reason).await;
      outcome
  }
  ```

- [ ] **Step 8: Add `on_yield_change` override**

  Insert after `on_service_config_ack` and before `on_surface_action_request`:

  ```rust
  async fn on_yield_change(&mut self, is_yielded: bool, conn: &mut dyn ServiceTransport) {
      self.runtime.handle_yield_change(is_yielded, conn).await;
  }
  ```

- [ ] **Step 9: Compile-check MQTT crate**

  ```bash
  cargo check -p uptrakit-mqtt --all-features
  ```

  Expected: PASS.

- [ ] **Step 10: Commit**

  ```bash
  git add crates/core/mqtt/src/main.rs
  git commit -m "refactor(mqtt): migrate StandaloneMqttHandler to dyn ServiceTransport"
  ```

---

## Task 7: `StandaloneSchedulerHandler` Migration

**Files:**

- Modify: `crates/core/scheduler-runtime/src/standalone.rs`

**Note:** All `impl ServiceHandler for StandaloneSchedulerHandler` steps below update method
signatures only. Preserve the existing `#[async_trait::async_trait]` attribute on the `impl`
block — do not remove it.

This handler directly calls `conn.send()` (returns `Result<(), Report<EnrollmentError>>`).
After migration, use `conn.transport_send()` (returns `Result<(), TransportError>`) and map
the error with `map_err(|e| report!(LoopError::Other(...)))`.

- [ ] **Step 1: Update imports**

  Change the `uptrakit_service_sdk` import to remove `ControllerConnection`:

  ```rust
  use uptrakit_service_sdk::{
      LoopError, LoopOutcome, LoopResult, ServiceHandler, ShutdownCause,
      Signal, default_resolve_shutdown,
  };
  ```

  Add `ServiceTransport` to the `uptrakit_wire` import:

  ```rust
  use uptrakit_wire::{
      Capability, ControllerMessage, DisconnectingPayload, RegisterPayload, ServiceMessage,
      ServiceTransport, payloads::AuditEventPayload,
  };
  ```

  Remove the `ServiceIdentityState` import from `uptrakit_service_sdk` (it was never used by
  the scheduler — confirm with `cargo check`, remove if unused).

- [ ] **Step 2: Update `on_connected` — replace `conn.send()` with `transport_send()`**

  `conn.send()` returns `Result<(), Report<EnrollmentError>>`; the old `.context_to::<LoopError>()`
  conversion no longer works on `TransportError`. Use `map_err` with `report!`:

  ```rust
  async fn on_connected(
      &mut self,
      conn: &mut dyn ServiceTransport,
      identity: &uptrakit_service_sdk::ServiceIdentityState,
  ) -> LoopResult<()> {
      conn.transport_send(ServiceMessage::Register(RegisterPayload::new(
          standalone_scheduler_capabilities(),
      )))
      .await
      .map_err(|e| report!(LoopError::Other(format!("failed to send Register: {e}"))))?;

      self.service_id = identity.service_id();
      tracing::info!("connected to controller, waiting for ServiceCredentials");
      Ok(())
  }
  ```

- [ ] **Step 3: Update `on_message`**

  No `conn` usage in the body — only signature change:

  ```rust
  async fn on_message(
      &mut self,
      msg: ControllerMessage,
      _conn: &mut dyn ServiceTransport,
  ) -> LoopResult<Option<LoopOutcome>> {
  ```

  (body unchanged)

- [ ] **Step 4: Update `on_service_event` — replace `conn.send()` with `transport_send()`**

  ```rust
  async fn on_service_event(
      &mut self,
      event: Self::ServiceEvent,
      conn: &mut dyn ServiceTransport,
  ) -> LoopResult<Option<LoopOutcome>> {
      match event {
          StandaloneSchedulerServiceEvent::Forward(message) => {
              conn.transport_send(message)
                  .await
                  .map_err(|e| report!(LoopError::Other(format!("failed to forward audit event: {e}"))))?;
              Ok(None)
          }
      }
  }
  ```

- [ ] **Step 5: Update `on_shutdown` — signature, `conn.send()` → `transport_send()`, and `EmbeddedDrain` match**

  The `matches!` macro pattern for selecting `SchedulerStopMode` must be updated to handle
  `EmbeddedDrain` — treat it as `Drain` (services persist pending audit events):

  ```rust
  #[expect(
      clippy::let_underscore_must_use,
      reason = "fire-and-forget Disconnecting message; send result ignored during shutdown where the connection may already be closing"
  )]
  async fn on_shutdown(
      &mut self,
      conn: &mut dyn ServiceTransport,
      cause: ShutdownCause,
      _shutdown_timeout: Duration,
  ) -> LoopOutcome {
      let (disconnect_reason, outcome) = default_resolve_shutdown(cause);
      let stop_mode = if matches!(
          cause,
          ShutdownCause::ServerRestarting
              | ShutdownCause::Signal(Signal::Hangup)
              | ShutdownCause::EmbeddedDrain
      ) {
          SchedulerStopMode::Drain
      } else {
          SchedulerStopMode::Abort
      };
      self.runtime.stop(stop_mode).await;
      self.drain_service_events(conn).await;

      let _ = conn
          .transport_send(ServiceMessage::Disconnecting(DisconnectingPayload::new(
              disconnect_reason,
          )))
          .await;

      outcome
  }
  ```

- [ ] **Step 6: Update `drain_service_events` — replace `conn.send()` with `transport_send()`**

  In the `impl StandaloneSchedulerHandler` block:

  ```rust
  async fn drain_service_events(&mut self, conn: &mut dyn ServiceTransport) {
      while let Ok(StandaloneSchedulerServiceEvent::Forward(message)) =
          self.service_event_rx.try_recv()
      {
          if let Err(error) = conn.transport_send(message).await {
              tracing::warn!(
                  error = %error,
                  "failed to drain scheduler audit event during shutdown"
              );
              break;
          }
      }
  }
  ```

- [ ] **Step 7: Compile-check full workspace**

  ```bash
  cargo check --all-features
  ```

  Expected: PASS. This is the first workspace-wide compile-clean checkpoint.

- [ ] **Step 8: Run full test suite**

  ```bash
  cargo test --all-features 2>&1 | tail -20
  ```

  Expected: all tests pass.

- [ ] **Step 9: Commit**

  ```bash
  git add crates/core/scheduler-runtime/src/standalone.rs
  git commit -m "refactor(scheduler): migrate StandaloneSchedulerHandler to dyn ServiceTransport"
  ```

---

## Task 8: `EmbeddedTransport::yield_change_notifier` Override

**Files:**

- Modify: `crates/core/controller-runtime/src/embedded/types.rs`

- [ ] **Step 1: Override `yield_change_notifier` on the `ServiceTransport` impl**

  In `crates/core/controller-runtime/src/embedded/types.rs`, the `impl ServiceTransport for
EmbeddedTransport` block (starting around line 78) currently ends with `is_yielded`. Add the
  override after it, inside the same `impl` block:

  ```rust
  fn yield_change_notifier(&self) -> Option<std::sync::Arc<tokio::sync::Notify>> {
      Some(Arc::clone(&self.yield_state_changed))
  }
  ```

  `Arc` is already imported at the top of the file (`use std::sync::Arc;`).

  **Name collision note:** `EmbeddedTransport` already has a `pub(crate)` inherent method named
  `yield_change_notifier` at line ~68 that returns `Arc<Notify>` (not `Option<Arc<Notify>>`).
  In Rust, an inherent method and a trait method with the same name coexist — the trait impl
  does not remove the inherent one. Code inside `controller-runtime` calling
  `transport.yield_change_notifier()` unqualified will call the inherent method (returning
  `Arc<Notify>`). Code in `service-sdk` calling through the trait gets `Option<Arc<Notify>>`.
  This is intentional. Do not rename or remove the inherent method.

- [ ] **Step 2: Compile-check controller-runtime**

  ```bash
  cargo check -p uptrakit-controller-runtime --all-features
  ```

  Expected: PASS.

- [ ] **Step 3: Commit**

  ```bash
  git add crates/core/controller-runtime/src/embedded/types.rs
  git commit -m "feat(controller): EmbeddedTransport overrides yield_change_notifier"
  ```

---

## Task 9: Controller Sends `ServiceSettings` at Startup

**Files:**

- Modify: `crates/core/controller-runtime/src/embedded/mod.rs`

The controller must send `ServiceSettings` to the embedded service immediately after the
response forwarder is spawned. The embedded service's `run_embedded_service` waits for it
as its first message.

- [ ] **Step 1: Add import for `ServiceSettingsPayload` and `ControllerMessage` variants**

  In `crates/core/controller-runtime/src/embedded/mod.rs`, the existing imports include
  `uptrakit_wire::Capability`. Expand to include the required types:

  ```rust
  use uptrakit_wire::{
      Capability, ControllerMessage,
      payloads::ServiceSettingsPayload,
  };
  ```

  Also import `ReportPageLimits`:

  ```rust
  use uptrakit_wire::{
      Capability, ControllerMessage, ReportPageLimits,
      payloads::ServiceSettingsPayload,
  };
  ```

- [ ] **Step 2: Add helper to build the embedded `ServiceSettings` payload**

  Insert a private helper function in `embedded/mod.rs` (before the `impl EmbeddedServiceHost`
  block):

  **Before writing this function:** verify `ServiceSettingsPayload` is not `#[non_exhaustive]`
  by running `grep -n "non_exhaustive" crates/shared/wire/src/`. If it is `#[non_exhaustive]`,
  a struct literal will fail to compile outside the defining crate — use a constructor or
  builder pattern provided by `uptrakit-wire` instead.

  ```rust
  fn embedded_service_settings(
      capabilities: &BTreeSet<Capability>,
      tenant_id: Option<Uuid>,
  ) -> ServiceSettingsPayload {
      ServiceSettingsPayload {
          // Controller advertises its full capability set; the embedded service
          // intersects this with its own capabilities to compute agreed caps.
          capabilities: controller_capabilities(),
          tenant_id,
          // Non-zero ping_interval is required — Duration::ZERO panics in
          // tokio::time::interval. The embedded loop ignores the ping timer;
          // this sentinel value is never used.
          ping_interval: std::time::Duration::from_secs(60),
          // Embedded services share the controller's CA; cert renewal and
          // page limits are not used.
          renewal_window_hours: 0,
          ca_bundle_hash: String::new(),
          report_page_limits: ReportPageLimits::default(),
          shutdown_timeout: None,
      }
  }
  ```

  `controller_capabilities()` lives in `uptrakit_web_api::routes::service_ws::protocol`. Check
  whether it is already accessible from `controller-runtime` by running `cargo check`; if not,
  either inline the capability set here or re-export it from `web-api`. If the dependency
  is already present (it is — `controller-runtime` depends on `web-api`), use the existing
  function. The import path is:

  ```rust
  use uptrakit_web_api::routes::service_ws::protocol::controller_capabilities;
  ```

  **Note:** If `controller_capabilities` is `pub(crate)` inside `web-api` (it is), it cannot be
  imported externally. In that case, inline the capability set as a private constant in
  `embedded/mod.rs`:

  ```rust
  fn controller_capabilities_for_embedded() -> BTreeSet<Capability> {
      [
          Capability::SoftwareDiscovery,
          Capability::UpdateHooks,
          Capability::GracefulShutdown,
          Capability::UpdateTracking,
          Capability::SshRemote,
          Capability::Scheduler,
          Capability::DatabaseAccess,
          Capability::NatsAccess,
          Capability::MasterKeyAccess,
          Capability::CaManagement,
          Capability::UiSurfaces,
      ]
      .into_iter()
      .collect()
  }
  ```

  And use it in `embedded_service_settings`:

  ```rust
  capabilities: controller_capabilities_for_embedded(),
  ```

- [ ] **Step 3: Send `ServiceSettings` BEFORE forwarder spawn in `EmbeddedServiceHost::add()`**

  In the `add()` body, BEFORE the response forwarder is spawned (around line 227 in the
  current file), add:

  ```rust
  // Send ServiceSettings to the embedded service. `run_embedded_service` waits
  // for this as its first message before entering the event loop.
  let settings = embedded_service_settings(&capabilities, tenant_id);
  ctrl_tx
      .send(ControllerMessage::ServiceSettings(settings))
      .await
      .map_err(|_| rootcause::report!(rootcause::fmt("failed to send initial ServiceSettings to embedded service")))?;
  ```

  **Important:** `ctrl_tx` is moved by value into `bridge::run_response_forwarder` — it is
  consumed by the `tokio::spawn(bridge::run_response_forwarder(..., ctrl_tx, ...))` call and
  is unavailable afterward (a use-after-move is a compile error). The `ctrl_tx.send(settings)`
  call must be the last use of `ctrl_tx` before it is passed to the spawn. There is no
  "reorder" needed — just ensure the send precedes the spawn line.

  The embedded service's transport receives messages via a channel created at line ~209 with
  capacity 256. This buffer is load-bearing: `ServiceSettings` sits in it until
  `run_embedded_service` starts reading. Do not reduce the channel capacity.

  After reordering, the sequence in `add()` is:
  1. Provision service
  2. Register in registry
  3. Create channels
  4. **Send `ServiceSettings` on `ctrl_tx`** ← new
  5. Spawn response forwarder (passes `ctrl_tx`)
  6. Create transport and spawn service closure

- [ ] **Step 4: Compile-check controller-runtime**

  ```bash
  cargo check -p uptrakit-controller-runtime --all-features
  ```

  Expected: PASS.

- [ ] **Step 5: Commit**

  ```bash
  git add crates/core/controller-runtime/src/embedded/mod.rs
  git commit -m "feat(controller): send ServiceSettings to embedded service at startup"
  ```

---

## Task 10: `run_embedded_service` — New Entry Point

**Files:**

- Create: `crates/shared/service-sdk/src/embedded.rs`
- Modify: `crates/shared/service-sdk/src/lib.rs`

- [ ] **Step 1: Create `crates/shared/service-sdk/src/embedded.rs`**

  ```rust
  //! Entry point for running a [`ServiceHandler`] in embedded mode.
  //!
  //! Embedded services receive messages over an in-process channel
  //! (`EmbeddedTransport`) instead of a WebSocket. They skip enrollment,
  //! certificate management, and OS signal handling. Shutdown is driven by
  //! two `CancellationToken`s: `drain` (graceful) and `abort` (immediate).

  use std::collections::BTreeSet;
  use std::sync::Arc;
  use std::time::Duration;

  use tokio_util::sync::CancellationToken;

  use crate::wire_api::{
      Capability, ControllerMessage, ServiceSettingsPayload, ServiceTransport,
  };
  use crate::shared_types::{LoopOutcome, ServiceHandler, ShutdownCause};

  /// Startup timeout for the initial `ServiceSettings` message.
  ///
  /// If the controller does not send `ServiceSettings` within this window,
  /// the service exits rather than hanging indefinitely.
  const EMBEDDED_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);

  /// Default shutdown timeout, used until `ServiceSettings` provides one.
  const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(120);

  /// Run a [`ServiceHandler`] in embedded mode.
  ///
  /// Startup sequence:
  /// 1. Wait up to 10 s for the controller to send `ServiceSettings` (first message).
  /// 2. Compute agreed capabilities (intersection of handler's and controller's).
  /// 3. Call `on_settings` — first handler callback (no `on_connected` for embedded).
  /// 4. Call `on_yield_change` with the transport's current yield state.
  /// 5. Enter the two-phase event loop.
  ///
  /// Exits when: drain fires (graceful via `on_shutdown`), abort fires
  /// (immediate), transport closes, or a handler callback requests exit.
  #[expect(
      clippy::large_futures,
      reason = "embedded service state machine; per-service allocation is acceptable"
  )]
  pub async fn run_embedded_service<H: ServiceHandler>(
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

      let agreed = compute_agreed_capabilities(&handler, &settings);

      handler
          .on_settings(&settings, &mut transport, &agreed)
          .await;

      // Initial yield notification.
      {
          let is_yielded = transport.is_yielded();
          handler.on_yield_change(is_yielded, &mut transport).await;
      }

      // ── Obtain yield notifier handle ───────────────────────────────────────
      let yield_notifier: Option<Arc<tokio::sync::Notify>> = transport.yield_change_notifier();

      // ── Event loop ─────────────────────────────────────────────────────────
      loop {
          // Phase 1: resolve the next event.
          //
          // The yield_arm is a conditional future: if yield_notifier is Some it
          // awaits Notify::notified(); otherwise it pends forever. This avoids
          // unwrap/expect on the Option (both denied by workspace lints).
          let maybe_event = tokio::select! {
              biased;
              () = abort.cancelled() => break,
              () = drain.cancelled() => {
                  handler
                      .on_shutdown(&mut transport, ShutdownCause::EmbeddedDrain, shutdown_timeout)
                      .await;
                  break;
              }
              () = async {
                  if let Some(n) = &yield_notifier { n.notified().await }
                  // `pending()` makes this arm never-ready when yield signalling is
                  // unsupported. Do NOT replace with `unreachable!()` or `unwrap()`.
                  else { std::future::pending().await }
              } => {
                  // is_yielded() is &self; read before the &mut borrow.
                  let is_yielded = transport.is_yielded();
                  handler.on_yield_change(is_yielded, &mut transport).await;
                  None
              }
              event = handler.poll_service_event() => Some(event),
              msg = transport.transport_recv() => {
                  match msg {
                      None => break, // transport closed
                      Some(msg) => {
                          if !transport.is_yielded() {
                              if let Some(outcome) =
                                  dispatch_message(msg, &mut handler, &mut transport, &mut shutdown_timeout).await
                              {
                                  let _ = outcome;
                                  break;
                              }
                          }
                          // drop silently when yielded
                          None
                      }
                  }
              }
          };

          // Phase 2: run on_service_event with drain/abort guards.
          // event is Some(_) only when the poll_service_event arm fired.
          if let Some(event) = maybe_event {
              let should_break = tokio::select! {
                  biased;
                  () = abort.cancelled() => true,
                  () = drain.cancelled() => {
                      handler
                          .on_shutdown(&mut transport, ShutdownCause::EmbeddedDrain, shutdown_timeout)
                          .await;
                      true
                  }
                  outcome = handler.on_service_event(event, &mut transport) => {
                      match outcome {
                          Ok(Some(_)) => true,
                          Ok(None) => false,
                          Err(e) => {
                              tracing::error!(
                                  service = H::SERVICE_LABEL,
                                  error = %e,
                                  "embedded service event handler error; exiting"
                              );
                              true
                          }
                      }
                  }
              };
              if should_break {
                  break;
              }
          }
      }
  }

  /// Compute the agreed capability set (intersection of controller's and handler's).
  fn compute_agreed_capabilities<H: ServiceHandler>(
      handler: &H,
      settings: &ServiceSettingsPayload,
  ) -> BTreeSet<Capability> {
      settings
          .capabilities
          .intersection(&handler.capabilities())
          .filter(|c| c.is_known())
          .cloned()
          .collect()
  }

  /// Dispatch a single controller message to the appropriate `ServiceHandler` callback.
  ///
  /// Returns `Some(outcome)` to break the loop, `None` to continue.
  ///
  /// Mirrors the routing in `handle_controller_message` in `event_loop.rs`:
  /// - `ServiceSettings` → re-negotiate caps, `on_settings`, `on_yield_change`
  /// - `SurfaceActionRequest` → `on_surface_action_request`
  /// - `SurfaceActionResponse` → `on_surface_action_response`
  /// - `ServiceConfigAck` → `on_service_config_ack`
  /// - `ServerRestarting` → `on_shutdown(ServerRestarting)`
  /// - `Unknown` → warn, continue
  /// - Cert/CA/Pong/RequestCertRenewal → debug log, no-op (embedded has no certs)
  /// - Everything else → `on_message`
  async fn dispatch_message<H: ServiceHandler>(
      msg: ControllerMessage,
      handler: &mut H,
      transport: &mut dyn ServiceTransport,
      shutdown_timeout: &mut Duration,
  ) -> Option<LoopOutcome> {
      match msg {
          ControllerMessage::ServiceSettings(settings) => {
              *shutdown_timeout = settings
                  .shutdown_timeout
                  .unwrap_or(DEFAULT_SHUTDOWN_TIMEOUT);
              let agreed = compute_agreed_capabilities(handler, &settings);
              handler.on_settings(&settings, transport, &agreed).await;
              let is_yielded = transport.is_yielded();
              handler.on_yield_change(is_yielded, transport).await;
              None
          }
          ControllerMessage::SurfaceActionRequest(payload) => {
              if let Err(e) = handler.on_surface_action_request(payload, transport).await {
                  tracing::error!(error = %e, "surface action request handler error");
              }
              None
          }
          ControllerMessage::SurfaceActionResponse(payload) => {
              handler.on_surface_action_response(payload);
              None
          }
          ControllerMessage::ServiceConfigAck(ack) => {
              handler.on_service_config_ack(ack);
              None
          }
          ControllerMessage::ServerRestarting(_payload) => {
              tracing::info!(service = H::SERVICE_LABEL, "controller restarting; embedded service shutting down");
              let outcome = handler
                  .on_shutdown(transport, ShutdownCause::ServerRestarting, *shutdown_timeout)
                  .await;
              Some(outcome)
          }
          ControllerMessage::Unknown => {
              tracing::warn!(
                  service = H::SERVICE_LABEL,
                  "received unknown controller message type in embedded mode; ignoring"
              );
              None
          }
          // Cert/CA/Pong: no-op for embedded services (no certs, no ping timer).
          ControllerMessage::Certificate(_)
          | ControllerMessage::CaBundleUpdated(_)
          | ControllerMessage::Pong(_)
          | ControllerMessage::RequestCertRenewal(_) => {
              tracing::debug!(
                  service = H::SERVICE_LABEL,
                  "ignoring cert/CA/pong message in embedded mode"
              );
              None
          }
          msg => {
              match handler.on_message(msg, transport).await {
                  Ok(Some(outcome)) => Some(outcome),
                  Ok(None) => None,
                  Err(e) => {
                      tracing::error!(service = H::SERVICE_LABEL, error = %e, "on_message error");
                      None
                  }
              }
          }
      }
  }
  ```

- [ ] **Step 2: Export `run_embedded_service` from `lib.rs`**

  In `crates/shared/service-sdk/src/lib.rs`, add the module declaration and re-export:

  ```rust
  mod embedded;
  pub use embedded::run_embedded_service;
  ```

  Place the `mod embedded;` line with the other `mod` declarations (non-`pub` ones). Place
  the `pub use` with the other `pub use` lines for logical grouping.

- [ ] **Step 3: Compile-check service-sdk**

  ```bash
  cargo check -p uptrakit-service-sdk --all-features
  ```

  Expected: PASS.

- [ ] **Step 4: Run clippy to catch lint violations**

  ```bash
  cargo clippy -p uptrakit-service-sdk --all-features -- -D warnings
  ```

  Expected: PASS (no `unwrap`/`expect` calls, etc.).

  **Note on `#[expect(clippy::large_futures)]`:** If clippy passes without emitting a
  `large_futures` warning (because the concrete instantiation is below the threshold),
  `unfulfilled_lint_expectations` will produce a compile error — the `#[expect]` was
  never satisfied. If that happens, remove the `#[expect]` line entirely rather than
  fighting it. The annotation is only needed if the lint actually fires.

- [ ] **Step 5: Commit**

  ```bash
  git add crates/shared/service-sdk/src/embedded.rs \
          crates/shared/service-sdk/src/lib.rs
  git commit -m "feat(service-sdk): add run_embedded_service entry point"
  ```

---

## Task 11: `run_embedded_service` Unit Tests

**Files:**

- Modify: `crates/shared/service-sdk/src/embedded.rs` (add `#[cfg(test)]` module)

The tests use a `MockTransport` that wraps `mpsc` channels so `transport_recv` and
`transport_send` work without network I/O. A `MockHandler` records which callbacks were
called.

- [ ] **Step 1: Add test infrastructure at the bottom of `embedded.rs`**

  ```rust
  #[cfg(test)]
  mod tests {
      use std::collections::BTreeSet;
      use std::time::Duration;

      use async_trait::async_trait;
      use tokio::sync::mpsc;
      use tokio_util::sync::CancellationToken;

      use crate::wire_api::{
          Capability, ControllerMessage, ServiceMessage, ServiceSettingsPayload,
          ServiceTransport, TransportError, payloads::DisconnectingPayload,
      };
      use crate::shared_types::{LoopOutcome, LoopResult, ServiceHandler, ShutdownCause};

      use super::run_embedded_service;

      // ── MockTransport ──────────────────────────────────────────────────────

      struct MockTransport {
          svc_rx: mpsc::Receiver<ControllerMessage>,
          yielded: bool,
      }

      fn make_transport(ctrl_in: mpsc::Receiver<ControllerMessage>) -> MockTransport {
          MockTransport { svc_rx: ctrl_in, yielded: false }
      }

      fn make_yielded_transport(ctrl_in: mpsc::Receiver<ControllerMessage>) -> MockTransport {
          MockTransport { svc_rx: ctrl_in, yielded: true }
      }

      #[async_trait]
      impl ServiceTransport for MockTransport {
          async fn transport_send(&mut self, _msg: ServiceMessage) -> Result<(), TransportError> {
              Ok(())
          }
          async fn transport_send_best_effort(&mut self, _msg: ServiceMessage) {}
          async fn transport_send_auto_paginate(&mut self, msg: ServiceMessage) -> Result<(), TransportError> {
              self.transport_send(msg).await
          }
          async fn transport_recv(&mut self) -> Option<ControllerMessage> {
              self.svc_rx.recv().await
          }
          fn close_policy(&self) -> crate::wire_api::TransportClosePolicy {
              crate::wire_api::TransportClosePolicy::Shutdown
          }
          fn is_yielded(&self) -> bool {
              self.yielded
          }
      }

      // ── MockHandler ────────────────────────────────────────────────────────

      #[derive(Debug, Default)]
      struct CallLog {
          on_settings_called: bool,
          on_shutdown_called: bool,
          on_yield_change_called: bool,
          on_message_called: bool,
      }

      struct MockHandler {
          log: std::sync::Arc<parking_lot::Mutex<CallLog>>,
      }

      impl MockHandler {
          fn new() -> (Self, std::sync::Arc<parking_lot::Mutex<CallLog>>) {
              let log = std::sync::Arc::new(parking_lot::Mutex::new(CallLog::default()));
              (Self { log: log.clone() }, log)
          }
      }

      #[async_trait]
      impl ServiceHandler for MockHandler {
          const DIR_NAME: &'static str = "mock";
          const SERVICE_LABEL: &'static str = "mock service";
          const SERVICE_APP_NAME: &'static str = "mock";

          type ServiceEvent = std::convert::Infallible;

          async fn on_connected(
              &mut self,
              _conn: &mut dyn ServiceTransport,
              _identity: &crate::identity::ServiceIdentityState,
          ) -> LoopResult<()> {
              Ok(())
          }

          async fn on_message(
              &mut self,
              _msg: ControllerMessage,
              _conn: &mut dyn ServiceTransport,
          ) -> LoopResult<Option<LoopOutcome>> {
              self.log.lock().on_message_called = true;
              Ok(None)
          }

          async fn on_settings(
              &mut self,
              _settings: &ServiceSettingsPayload,
              _conn: &mut dyn ServiceTransport,
              _agreed: &BTreeSet<Capability>,
          ) {
              self.log.lock().on_settings_called = true;
          }

          async fn poll_service_event(&mut self) -> Self::ServiceEvent {
              std::future::pending().await
          }

          async fn on_service_event(
              &mut self,
              event: Self::ServiceEvent,
              _conn: &mut dyn ServiceTransport,
          ) -> LoopResult<Option<LoopOutcome>> {
              match event {}
          }

          async fn on_shutdown(
              &mut self,
              _conn: &mut dyn ServiceTransport,
              _cause: ShutdownCause,
              _timeout: Duration,
          ) -> LoopOutcome {
              self.log.lock().on_shutdown_called = true;
              LoopOutcome::Shutdown
          }

          async fn on_yield_change(&mut self, _is_yielded: bool, _conn: &mut dyn ServiceTransport) {
              self.log.lock().on_yield_change_called = true;
          }
      }

      fn make_settings() -> ServiceSettingsPayload {
          ServiceSettingsPayload {
              capabilities: BTreeSet::new(),
              tenant_id: None,
              ping_interval: Duration::from_secs(60),
              renewal_window_hours: 0,
              ca_bundle_hash: String::new(),
              report_page_limits: Default::default(),
              shutdown_timeout: None,
          }
      }

      // ── Tests ──────────────────────────────────────────────────────────────

      /// Transport closes before ServiceSettings arrives → exits without calling any callback.
      #[tokio::test]
      async fn exits_when_transport_closed_before_settings() {
          let (ctrl_tx, ctrl_rx) = mpsc::channel::<ControllerMessage>(1);
          drop(ctrl_tx); // close immediately
          let transport = make_transport(ctrl_rx);
          let (handler, log) = MockHandler::new();
          let drain = CancellationToken::new();
          let abort = CancellationToken::new();

          run_embedded_service(handler, transport, drain, abort).await;

          let log = log.lock();
          assert!(!log.on_settings_called, "on_settings must not be called");
          assert!(!log.on_shutdown_called, "on_shutdown must not be called");
      }

      /// Abort fires before ServiceSettings → exits immediately.
      #[tokio::test]
      async fn abort_before_settings_exits_immediately() {
          let (_ctrl_tx, ctrl_rx) = mpsc::channel::<ControllerMessage>(1);
          let transport = make_transport(ctrl_rx);
          let (handler, log) = MockHandler::new();
          let drain = CancellationToken::new();
          let abort = CancellationToken::new();
          abort.cancel();

          run_embedded_service(handler, transport, drain, abort).await;

          assert!(!log.lock().on_settings_called);
      }

      /// Normal startup: ServiceSettings arrives → on_settings called → drain → on_shutdown called.
      ///
      /// Drain is cancelled synchronously before calling `run_embedded_service`. The startup
      /// phase reads settings from the buffered channel, then the event loop sees drain already
      /// cancelled and calls `on_shutdown` immediately. No `sleep` needed.
      #[tokio::test]
      async fn normal_startup_then_drain_calls_on_shutdown() {
          let (ctrl_tx, ctrl_rx) = mpsc::channel::<ControllerMessage>(4);
          let transport = make_transport(ctrl_rx);
          let (handler, log) = MockHandler::new();
          let drain = CancellationToken::new();
          let abort = CancellationToken::new();

          ctrl_tx
              .send(ControllerMessage::ServiceSettings(make_settings()))
              .await
              .expect("send settings");
          // Cancel synchronously — startup phase reads from buffered channel, then the
          // biased event loop sees drain cancelled and calls on_shutdown immediately.
          drain.cancel();

          run_embedded_service(handler, transport, drain, abort).await;

          let log = log.lock();
          assert!(log.on_settings_called, "on_settings must be called");
          assert!(log.on_shutdown_called, "on_shutdown must be called on drain");
      }

      /// Startup timeout: no message within 10s → exits without calling on_settings.
      /// Uses `start_paused = true` + `tokio::time::advance` for deterministic timing.
      #[tokio::test(start_paused = true)]
      async fn startup_timeout_exits_without_callback() {
          let (_ctrl_tx, ctrl_rx) = mpsc::channel::<ControllerMessage>(1);
          let transport = make_transport(ctrl_rx);
          let (handler, log) = MockHandler::new();
          let drain = CancellationToken::new();
          let abort = CancellationToken::new();

          let task = tokio::spawn(run_embedded_service(handler, transport, drain, abort));

          // Advance time past EMBEDDED_STARTUP_TIMEOUT (10 s).
          tokio::time::advance(Duration::from_secs(11)).await;

          task.await.expect("task panicked");

          assert!(!log.lock().on_settings_called);
          assert!(!log.lock().on_shutdown_called);
      }

      /// When transport is yielded, incoming messages are dropped silently — on_message is
      /// never called. Service exits cleanly when the channel closes.
      #[tokio::test]
      async fn yielded_transport_drops_messages_silently() {
          let (ctrl_tx, ctrl_rx) = mpsc::channel::<ControllerMessage>(4);
          // Transport starts in yielded state.
          let transport = make_yielded_transport(ctrl_rx);
          let (handler, log) = MockHandler::new();
          let drain = CancellationToken::new();
          let abort = CancellationToken::new();

          // Send settings (processed during startup — not dropped by yield check).
          ctrl_tx
              .send(ControllerMessage::ServiceSettings(make_settings()))
              .await
              .expect("send settings");
          // Send a regular message that should be dropped while yielded.
          ctrl_tx
              .send(ControllerMessage::Unknown)
              .await
              .expect("send unknown");
          // Close channel — transport_recv returns None → service exits.
          drop(ctrl_tx);

          run_embedded_service(handler, transport, drain, abort).await;

          let log = log.lock();
          assert!(log.on_settings_called, "on_settings must be called");
          assert!(!log.on_message_called, "on_message must NOT be called when yielded");
          assert!(!log.on_shutdown_called, "on_shutdown not called on transport close");
      }
  }
  ```

- [ ] **Step 2: Run the tests**

  ```bash
  cargo test -p uptrakit-service-sdk -- embedded::tests
  ```

  Expected: all 5 tests pass.

- [ ] **Step 3: Commit**

  ```bash
  git add crates/shared/service-sdk/src/embedded.rs
  git commit -m "test(service-sdk): unit tests for run_embedded_service"
  ```

---

## Task 12: Documentation Deliverables

**Files:**

- Create: `docs/adr/0003-service-handler-transport-abstraction.md`
- Modify: `docs/development/coding-standards.md`

- [ ] **Step 1: Write ADR 0003**

  Create `docs/adr/0003-service-handler-transport-abstraction.md`:

  ```markdown
  # `ServiceHandler` Transport Abstraction

  **Date:** 2026-05-07  
  **Status:** Accepted

  ## Context

  `ServiceHandler` trait methods originally accepted `&mut ControllerConnection` — a concrete
  WebSocket-specific type from `uptrakit-service-sdk`. This made it impossible to use the same
  `ServiceHandler` implementation in embedded mode (in-process `EmbeddedTransport` channels).
  Controller-runtime worked around this by maintaining bespoke event loops
  (`run_embedded_ssh_agent`, `run_embedded_mqtt`) that bypassed `ServiceHandler` entirely,
  duplicating service lifecycle logic.

  ## Decision

  All `ServiceHandler` method signatures accept `&mut dyn ServiceTransport` instead of
  `&mut ControllerConnection`. `ControllerConnection` continues to be used internally by
  `run_event_loop_connected` (the standalone path), which passes it as `conn as &mut dyn
  ServiceTransport` at each handler call site. A new `run_embedded_service` entry point in
  `uptrakit-service-sdk` accepts any `impl ServiceTransport`, enabling both WebSocket and
  in-process transports to share the same handler.

  `agreed_capabilities` (previously read from `conn.agreed_capabilities()`, a method not on
  `ServiceTransport`) is passed directly as a `&BTreeSet<Capability>` parameter to `on_settings`.

  ## Established Pattern

  `agent-runtime` and `mqtt-runtime` already used `&mut dyn ServiceTransport` throughout their
  public methods. This ADR formalises the same constraint at the `ServiceHandler` trait level,
  making transport-agnosticism enforceable by the compiler: a handler that compiles does not
  import `ControllerConnection`.

  ## Consequences

  - Handler implementations gain no dependency on `ControllerConnection` or WebSocket internals.
  - `run_embedded_service` enables the service binary/runtime boundary refactor: a single
    `AgentSshHandler`, `MqttHandler`, or `SchedulerHandler` type runs in both standalone and
    embedded modes without bespoke wrappers.
  - `StandaloneSchedulerHandler::on_connected` and related methods must map `TransportError` to
    `LoopError::Other` (not via the old `context_to::<LoopError>()` chain that consumed
    `Report<EnrollmentError>`).
  - Embedded handlers must inject identity and credentials via their constructors — `on_connected`
    is not called by `run_embedded_service`.
  ```

- [ ] **Step 2: Add `ServiceHandler` transport rule to `coding-standards.md`**

  Find the "Service binary/runtime boundary" section in
  `docs/development/coding-standards.md` (or add a new section if absent). Add:

  ```markdown
  ### `ServiceHandler` transport contract

  `ServiceHandler` implementations must not import or depend on `ControllerConnection`.
  All handler method signatures use `&mut dyn ServiceTransport` (from `uptrakit-wire`).
  A handler impl that compiles against `uptrakit-wire` types only is transport-agnostic
  by construction and can run in both standalone (WebSocket) and embedded (in-process)
  modes.

  `agreed_capabilities` for capability-dependent initialization must be read from the
  `on_settings` parameter, not from a connection method.
  ```

- [ ] **Step 3: Lint and format the new docs**

  ```bash
  npx prettier --write docs/adr/0003-service-handler-transport-abstraction.md \
                       docs/development/coding-standards.md
  npx markdownlint --config .markdownlint.json \
    docs/adr/0003-service-handler-transport-abstraction.md \
    docs/development/coding-standards.md
  ```

  Expected: no linting errors.

- [ ] **Step 4: Commit**

  ```bash
  git add docs/adr/0003-service-handler-transport-abstraction.md \
          docs/development/coding-standards.md
  git commit -m "docs: add ADR 0003 and coding-standards rule for ServiceHandler transport abstraction"
  ```

---

## Final Quality Gate

- [ ] **Full workspace compile**

  ```bash
  cargo check --all-features
  ```

- [ ] **Full clippy pass**

  ```bash
  cargo clippy --all-targets --all-features -- -D warnings
  ```

- [ ] **Full test suite**

  ```bash
  cargo test --all-features 2>&1 | tail -30
  ```

- [ ] **Dependency audit**

  ```bash
  cargo deny check
  ```

- [ ] **Markdown lint**

  ```bash
  npx markdownlint --config .markdownlint.json 'docs/**/*.md'
  ```

---

## Self-Review Checklist

- **WS1 (yield_change_notifier):** Task 1 ✓
- **WS2 (ShutdownCause::EmbeddedDrain + lifecycle):** Task 2 ✓
- **WS3 (ServiceHandler trait changes):** Tasks 3–4 ✓
- **WS3 (run_embedded_service):** Tasks 10–11 ✓
- **WS4 (controller sends ServiceSettings):** Task 9 ✓
- **Implementor migrations:** Tasks 5 (agent), 6 (mqtt), 7 (scheduler) ✓
- **EmbeddedTransport override:** Task 8 ✓
- **Startup timeout:** Included in Task 10 code (`EMBEDDED_STARTUP_TIMEOUT = 10s`) ✓
- **EmbeddedDrain on_shutdown clarified:** Included in Task 10 dispatch code ✓
- **StandaloneSchedulerHandler conn.send migration detail:** Task 7 covers both `on_connected` and `drain_service_events` with exact error mapping ✓
- **Embedded identity invariant:** ADR 0003 (Task 12) documents it ✓
- **large_futures suppression:** `#[expect(clippy::large_futures, reason = "...")]` on `run_embedded_service` ✓
- **Non-zero ping_interval in ServiceSettings:** Task 9 `embedded_service_settings` uses `Duration::from_secs(60)` ✓
- **on_yield_change borrow order:** `let is_yielded = transport.is_yielded();` then `handler.on_yield_change(is_yielded, &mut transport).await;`
  — `is_yielded` (`&self`) read before `&mut transport` borrow ✓
- **Documentation deliverables:** ADR 0003 + coding-standards.md (Task 12) ✓
