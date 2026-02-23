# CODEREVIEW — uptrakit-mqtt

> Reviewed: 2026-02-23
> Files: `src/main.rs`, `src/mqtt_client.rs`, `src/tenant_manager.rs`, `src/cli.rs`, `Cargo.toml`
> Phase 1 source: `.review/phase1_findings.md`

---

## Summary

`uptrakit-mqtt` is a lean, single-purpose service binary (~430 LoC across four source files). Its
responsibility is narrow: hold a mTLS WebSocket connection to the controller, receive per-tenant
MQTT broker assignments, and manage a pool of `rumqttc`-backed MQTT client connections. The
`ServiceHandler` trait from `uptrakit-service-sdk` drives the entire lifecycle; `main.rs` is
~190 lines.

The crate is notably clean. There are no `unwrap()` calls outside tests, no `#[allow(clippy::...)]`
suppressions, no dead code, and secret material is consistently redacted from `Debug` output. The
primary concern raised in Phase 1 is a fixed 5-second reconnect delay with no backoff, which
creates a load storm against the broker during extended outages. One dependency management issue
(`rumqttc` not in workspace) and a minor TLS configuration gap are the other notable findings.

---

## Architecture

### Strengths

- **Minimal surface area.** The binary owns exactly one concern: bridging the controller wire
  protocol to `rumqttc`. All lifecycle plumbing (enrollment, mTLS, reconnect to controller,
  signal handling) is delegated to `uptrakit-service-sdk` via `ServiceHandler`. `main.rs` is
  ~190 lines with no business logic.

- **Clean separation of concerns.** Three modules with clear, non-overlapping responsibilities:
  `mqtt_client.rs` owns the `rumqttc` interface and LWT configuration; `tenant_manager.rs` owns
  the per-client lifecycle map; `main.rs` owns the `ServiceHandler` impl and wires them together
  via an unbounded MPSC channel.

- **Push-based config model.** `TenantManager` receives config updates directly from the
  controller via `TenantAssignments` / `TenantConfigUpdated` / `TenantRevoked` wire messages.
  There is no polling loop, no database, and no timer-based reconciliation in this crate.
  Config-change detection uses a per-process hash (`compute_config_hash`) with a clear doc-comment
  explaining why `DefaultHasher` is acceptable for this in-process, non-persistent use case.

- **Concurrent shutdown.** `TenantManager::shutdown_all` uses `FuturesUnordered` to shut down all
  MQTT clients concurrently rather than serially, avoiding O(N × shutdown-timeout) blocking during
  graceful termination.

- **Instance ID design.** `generate_instance_id` produces a collision-resistant,
  human-debuggable string (`{hostname}-{uuid_v7_prefix_8}`) without pulling in a UUID-only
  dependency. The UUID v7 prefix preserves time-ordering for log correlation.

### Issues

**[SEVERITY: Low]** `Cargo.toml:24` — `rumqttc` not in workspace dependencies

`rumqttc = { version = "0.25.1" }` is declared inline rather than in the root
`[workspace.dependencies]` table. This crate is currently the sole consumer, so version drift is
not an immediate risk. However, the pattern is inconsistent with the workspace standard and would
become a real risk if a second crate (e.g., a test utility or a future provider) ever needed to
import `rumqttc` directly. Move to `[workspace.dependencies]` to keep version pinning centralised.

---

## Security & Safety

### Strengths

- **Credentials redacted in `Debug`.** `MqttConfig` provides a hand-written `Debug` impl
  (`mqtt_client.rs:34-47`) that prints `"[REDACTED]"` for `username`, `password`, and `ca_pem`
  regardless of whether the field is `Some` or `None`. A dedicated test (`credentials_redacted_in_debug`,
  `mqtt_client.rs:352-408`) verifies all four negative cases (username present, password present,
  username None, password None). This matches the `CaKeyStore` pattern in `uptrakit-web-api`.

- **`SecretString` at the config boundary.** `MqttConfig.username`, `.password`, and `.ca_pem`
  are all typed as `Option<SecretString>`, not `Option<String>`. The wire type `MqttTenantConfig`
  uses the same types, so secrets are never widened to plain `String` at the translation layer
  in `build_config_from_wire` (`tenant_manager.rs:150-169`).

- **Zero `unsafe` blocks.** Consistent with the rest of the workspace.

- **No `unwrap()` in production paths.** All error propagation uses `?` with `context_to` or
  `tracing::warn!` fallback. The single potential panic point (`hostname::get()` fallback in
  `generate_instance_id`, `main.rs:184-188`) degrades gracefully to `"unknown"` rather than
  panicking.

### Issues

**[SEVERITY: Low]** `mqtt_client.rs:202-214` — TLS uses `rumqttc::TlsConfiguration::Simple`, no hostname verification documented

When `MqttTransport::Tls` is selected, the code passes `ca: Vec<u8>` to
`TlsConfiguration::Simple`. The `rumqttc 0.25` `Simple` variant performs certificate chain
validation against the provided CA bundle but delegates hostname verification behaviour to the
underlying TLS stack. Whether `alpn: None` and the absence of `client_auth` are intentional (MQTT
brokers typically do not require mTLS from clients) is not documented. A comment stating the
deliberate choice (no client cert, server-only auth, standard hostname verification) would make
the security posture self-documenting and prevent a future contributor from assuming this needs to
be extended.

---

## Code Quality

### Strengths

- **No magic numbers.** The single shared constant `SHUTDOWN_TIMEOUT: Duration =
  Duration::from_secs(5)` (`mqtt_client.rs:265`) is named and typed. The reconnect delay
  (`Duration::from_secs(5)`, `mqtt_client.rs:254`) is the one exception — see Issues.

- **Consistent error handling.** `MqttError` uses `thiserror`, the `impl_report_conversion!`
  macro ties it into the workspace-standard `rootcause` chain, and `Result<T>` is a
  crate-local alias over `Report<MqttError>`. No `Box<dyn Error>` or `String` error types.

- **No `#[allow(clippy::...)]` suppressions** anywhere in the crate.

- **No `#[allow(dead_code)]`** annotations anywhere in the crate.

- **`on_message` wildcard arm is benign here.** `main.rs:76-79` uses `_ =>` with a
  `tracing::debug!` log. Unlike the High-severity wildcards in `service_ws.rs` and `agent_ws.rs`
  flagged in Phase 1, this pattern is appropriate: the MQTT handler is intentionally narrow and
  new `ControllerMessage` variants (e.g., PKI rotation messages) should be silently forwarded
  to the SDK loop rather than causing a compile error in this crate.

### Issues

**[SEVERITY: Low]** `mqtt_client.rs:254` — Magic number `5` for reconnect delay

```rust
_ = tokio::time::sleep(Duration::from_secs(5)) => {
```

This literal is the root cause of the HA issue described below. Extracting it to a named constant
(e.g., `RECONNECT_DELAY: Duration = Duration::from_secs(5)`) would make it co-located with
`SHUTDOWN_TIMEOUT` and easier to find when implementing backoff. A constant name also signals
intent and makes the backoff issue visible in code review.

---

## Tests

### Strengths

- **`#[tokio::test(start_paused = true)]` used correctly.** `mqtt_client.rs:453` tests the
  shutdown abort timeout path by sleeping the spawned task for 60 seconds inside a paused-clock
  test. This correctly drives `shutdown_task` into the `SHUTDOWN_TIMEOUT` branch without burning
  real wall-clock time. This is one of five sites in the workspace using this pattern correctly,
  as noted in the Phase 1 tests review.

- **Broad unit coverage for a pure-function surface.** `mqtt_client.rs` has 11 tests covering
  LWT configuration, credential handling, TLS option building, debug redaction (four negative
  cases), and the shutdown state machine. `tenant_manager.rs` has 11 tests covering wire-to-config
  translation, default port fallback, hash stability, hash sensitivity, and the lifecycle
  no-ops. `cli.rs` has 8 tests covering CLI parsing defaults, overrides, and directory resolution.

- **Tests avoid live network.** No test requires a running MQTT broker. The `start()` function is
  not called in any test; all tests operate on `build_mqtt_options`, `compute_config_hash`,
  `build_config_from_wire`, and the `shutdown_task` helper directly.

- **Deterministic fixture construction.** Tests use `tcp_config()` as a base fixture with struct
  update syntax (`..tcp_config()`) to isolate the single field under test. No shared mutable
  state.

### Issues

**[SEVERITY: Low]** `mqtt_client.rs:421-430` — `tls_transport_sets_tls` and
`tls_with_custom_ca_pem_does_not_panic` only assert no panic

```rust
// Just verify it doesn't panic
let _opts = build_mqtt_options(&config);
```

These tests confirm the code path executes but verify nothing about the produced `MqttOptions`.
A minimal assertion — e.g., confirming `opts.transport()` is the TLS variant — would convert
a no-op smoke test into a regression guard. The CA PEM test in particular should verify that the
custom CA bytes reach the `TlsConfiguration::Simple { ca }` field.

**[SEVERITY: Low]** No integration test for `TenantManager::start_or_update_client`

The config-change detection path (`tenant_manager.rs:96-100`) — skip update when hash matches,
reload when hash differs — is only covered for the hash computation itself, not for the
manager-level lifecycle behavior. This path manages MQTT connection churn; a test using a mock or
in-process broker would catch regressions in the stop-then-restart sequencing.

---

## High Availability

### Strengths

- **MQTT Last Will and Testament (LWT).** `build_mqtt_options` (`mqtt_client.rs:185-191`)
  registers a retained `offline` LWT on the `{prefix}/status` topic at `QoS::AtLeastOnce`. If
  the service crashes or is forcibly disconnected, the broker publishes `offline` automatically.
  This is the correct MQTT pattern for presence detection.

- **Clean shutdown publishes `offline` before disconnecting.** `MqttHandle::shutdown`
  (`mqtt_client.rs:63-79`) explicitly publishes a retained `offline` message and calls
  `client.disconnect()` before waiting for the event-loop task. The ordered sequence —
  publish offline, disconnect, wait — ensures the retained status is correct even when the
  disconnect handshake completes before the LWT would have fired.

- **Shutdown abort path is bounded.** `shutdown_task` (`mqtt_client.rs:267-289`) wraps the
  task join in a `SHUTDOWN_TIMEOUT` (5 seconds). On timeout it calls `task.abort()` and logs a
  warning. The event loop is never left running as a ghost task.

- **Concurrent client shutdown.** `TenantManager::shutdown_all` uses `FuturesUnordered` to
  overlap per-client shutdown, so the total graceful-shutdown time is bounded by the slowest
  single client rather than the sum of all clients.

- **Controller reconnect handled by SDK.** Reconnect backoff for the controller WebSocket is
  handled by `uptrakit-service-sdk` (exponential, base 2s, cap 60s, ~25% jitter). This crate
  does not need its own controller reconnect logic.

### Issues

**[SEVERITY: Medium]** `mqtt_client.rs:253-254` — Fixed 5-second MQTT reconnect delay, no exponential backoff

```rust
_ = tokio::time::sleep(Duration::from_secs(5)) => {
    report_status(&reporter, MqttClientConnectionStatus::Connecting);
}
```

After any MQTT error (connection refused, network partition, broker crash), the event loop waits
exactly 5 seconds and retries unconditionally. There is no backoff and no circuit breaker.

The contrast with the WebSocket reconnect path in `uptrakit-service-sdk` is direct: that path
uses `backoff.rs` (exponential, base 2s, cap 60s, ~25% jitter). During an extended broker
outage with N MQTT service instances each managing M tenants, every client hammers the broker
once every 5 seconds. With the default `max_tenants = 0` (unlimited), a single instance could
maintain dozens of connections, all retrying in a tight fixed-interval loop with no coordinated
relief period.

The `CancellationToken` is correctly checked in the inner `tokio::select!` inside the error
branch, so shutdown is not blocked. The availability concern is the broker-side load during
outage, not service shutdown correctness.

Recommended fix: use the existing `uptrakit-service-sdk` `Backoff` struct (or a per-client
equivalent) with exponential delay capped at 60 seconds and jitter. Reset the backoff counter on
a successful `ConnAck`.

---

## Database

This crate has no direct database dependency and performs no DB operations. All persistence
concerns (MQTT lease assignment, client status, heartbeat tracking) live in `uptrakit-web-api`'s
`mqtt_lease_coordinator.rs` and `mqtt_client_store.rs`. Issues in those components (N+1 status
updates, silent lease takeover) are documented in the `crates/ui/web-api/CODEREVIEW.md`.

---

## Coding Standards

### Strengths

- **`edition = "2024"`, `publish = false`** set correctly.
- **All workspace-available dependencies use `workspace = true`** keys except for `rumqttc` (see
  Architecture Issues).
- **`bail!` / `report!` / `context_to` pattern** used consistently; no `Report::new()` anti-pattern.
- **No `Result<T, String>`** anywhere in the crate.
- **`SecretString` at API boundaries** — all credential fields typed correctly throughout.
- **No `StatusCode` usage** — not applicable to this binary (no HTTP server), correct absence.

### Issues

None beyond the `rumqttc` workspace-dependency note in Architecture and the magic-number note in
Code Quality.

---

## Extensibility

### Strengths

- **`ServiceHandler` trait isolates MQTT-specific logic.** Adding a new message type from the
  controller (e.g., `ControllerMessage::BrokerHealthCheck`) requires only a new match arm in
  `MqttHandler::on_message` in `main.rs`. The `_ =>` wildcard ensures new variants are silently
  ignored until explicitly handled, which is the correct forward-compatibility posture for a
  consumer of `#[non_exhaustive]` wire enums.

- **`TenantManager` is transport-agnostic.** The manager holds `MqttHandle` values and calls
  `start()` and `shutdown()`. Changing the underlying MQTT client library would be confined to
  `mqtt_client.rs`; `tenant_manager.rs` and `main.rs` would be unaffected.

- **`status_sender: Option<...>`** in `start()` and `TenantManager::new()` allows the status
  reporting channel to be omitted in tests and alternative embeddings without a separate
  no-op implementation.

### Issues

**[SEVERITY: Low]** `cli.rs:17` — `max_tenants = 0` means "unlimited" via implicit convention

The `--max-tenants` argument uses `0` as a sentinel for "unlimited" with no type-system
enforcement. The semantics are clear from the doc-comment, but the `MqttHandler` struct stores it
as a `u32` and passes it verbatim to `MqttRegisterPayload`. If the controller or a future
operator tool interprets `0` literally as "zero allowed tenants" it would silently starve the
instance of assignments. A `NonZeroU32` field for the actual cap combined with an explicit `None`
for unlimited (using `Option<NonZeroU32>`) would make the sentinel explicit at the type level and
eliminate the ambiguity from the wire payload.
