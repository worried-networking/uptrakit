# Code Review: uptrakit-mqtt

- **Review date**: 2026-02-28
- **Reviewer**: AI code review (architecture | security | quality | HA | standards | extensibility)
- **Branch**: docs/codereview-backend

## Summary

The MQTT bridge service (~2.1K LoC) is a lean, single-purpose service binary. Its responsibility is
narrow: hold a mTLS WebSocket connection to the controller, receive per-tenant MQTT broker
assignments, and manage a pool of `rumqttc`-backed MQTT client connections. The `ServiceHandler`
trait from `uptrakit-service-sdk` drives the entire lifecycle; `main.rs` is ~190 lines.

The crate is notably clean: no `unwrap()` calls outside tests, no `#[allow(clippy::...)]`
suppressions, no dead code, and secret material consistently redacted from `Debug` output. The
primary concern is an unbounded channel between `TenantManager` and `MqttHandler` that could lead
to memory growth under connection disruption.

## Architecture

### Strengths

- Minimal surface area. The binary owns exactly one concern: bridging the controller wire protocol
  to `rumqttc`. All lifecycle plumbing delegated to `uptrakit-service-sdk`. `main.rs` is ~190
  lines with no business logic.
- Clean separation of concerns: `mqtt_client.rs` owns the `rumqttc` interface and LWT config;
  `tenant_manager.rs` owns the per-client lifecycle map; `main.rs` owns the `ServiceHandler`
  impl and wires them together via an MPSC channel.
- Push-based config model. `TenantManager` receives config updates directly from the controller.
  No polling loop, no database, no timer-based reconciliation. Config-change detection uses a
  per-process hash with documented `DefaultHasher` rationale.
- `src/tenant_manager.rs:81-94` -- Concurrent shutdown via `FuturesUnordered`.
- `src/main.rs:217-223` -- `generate_instance_id()` produces collision-resistant,
  human-debuggable string (`{hostname}-{uuid_v7_prefix_8}`).

### Issues

**[LOW]** `Cargo.toml:24` -- `rumqttc` not in workspace dependencies. Declared inline
(`version = "0.25.1"`) rather than in `[workspace.dependencies]`. Currently sole consumer, but
inconsistent with workspace standard and risks version drift if a second crate ever needs it.

## Security and Safety

### Strengths

- `src/mqtt_client.rs:34-47` -- `MqttConfig` hand-written `Debug` impl prints `"[REDACTED]"` for
  `username`, `password`, and `ca_pem`. Dedicated test (`credentials_redacted_in_debug`,
  `mqtt_client.rs:352-408`) verifies all four negative cases.
- `SecretString` at the config boundary: `MqttConfig.username`, `.password`, `.ca_pem` typed as
  `Option<SecretString>`. Wire type `MqttTenantConfig` uses same types, so secrets never widened
  to plain `String` at the translation layer.
- Zero `unsafe` blocks.
- No `unwrap()` in production paths. All error propagation uses `?` with `context_to` or
  `tracing::warn!` fallback.

### Issues

**[LOW]** `src/mqtt_client.rs:202-214` -- TLS uses `rumqttc::TlsConfiguration::Simple`, no
hostname verification documented. Whether `alpn: None` and absence of `client_auth` are
intentional (MQTT brokers typically do not require mTLS from clients) is not documented. A
comment stating the deliberate choice would make the security posture self-documenting.

## Code Quality

### Strengths

- No magic numbers. `SHUTDOWN_TIMEOUT` named and typed.
- Consistent error handling: `MqttError` uses `thiserror`, `impl_report_conversion!` macro, and
  `Result<T>` crate-local alias. No `Box<dyn Error>` or `String` error types.
- Zero `#[allow(clippy::...)]` suppressions. Zero `#[allow(dead_code)]`.
- `on_message` wildcard arm benign here: uses `_ =>` with `tracing::debug!`. Appropriate since
  the MQTT handler is intentionally narrow.
- `src/mqtt_client.rs:453` -- `#[tokio::test(start_paused = true)]` used correctly for shutdown
  abort timeout test.
- `src/mqtt_client.rs` -- 11 tests covering LWT, credential handling, TLS, debug redaction.
  `src/tenant_manager.rs` -- 11 tests covering wire-to-config translation, hash stability.
  `src/cli.rs` -- 8 tests covering CLI parsing.
- Tests avoid live network. No test requires running MQTT broker.
- Deterministic fixture construction via `tcp_config()` with struct update syntax.

### Issues

**[MEDIUM]** `src/mqtt_client.rs:445` and `src/tenant_manager.rs:344,353,361,383` -- Five mqtt
crate tests use bare `#[tokio::test]`. One sibling test correctly uses `start_paused = true`
(`shutdown_task` at line 453), demonstrating inconsistency.

**[LOW]** `src/main.rs:221` -- `&uuid::Uuid::now_v7().to_string()[..8]` uses byte-offset slicing
on a UTF-8 string. UUID v7 string representation is always ASCII and this is safe, but the
pattern is fragile. Consider `.chars().take(8).collect::<String>()`.

**[LOW]** `src/mqtt_client.rs:421-430` -- `tls_transport_sets_tls` and
`tls_with_custom_ca_pem_does_not_panic` only assert no panic. No verification of produced
`MqttOptions`. A minimal assertion confirming `opts.transport()` is the TLS variant would
convert a no-op smoke test into a regression guard.

**[LOW]** No integration test for `TenantManager::start_or_update_client`. The config-change
detection path (skip when hash matches, reload when differs) is only covered for hash
computation, not manager-level lifecycle behavior.

## High Availability

### Strengths

- `src/mqtt_client.rs:281-353` -- MQTT client has proper reconnection with exponential backoff.
- `src/mqtt_client.rs:246-252` -- Last Will and Testament (LWT) ensures broker publishes
  `offline` status on unexpected disconnect.
- `src/mqtt_client.rs:63-79` -- Clean shutdown publishes `offline` before disconnecting.
  Ordered sequence: publish offline, disconnect, wait.
- `src/mqtt_client.rs:267-289` -- Shutdown abort path bounded by `SHUTDOWN_TIMEOUT` (5 seconds).
- `src/tenant_manager.rs:81-94` -- `shutdown_all` uses `FuturesUnordered` for parallel client
  shutdown.
- `src/tenant_manager.rs:165-177` -- Config change detection uses hash comparison.
- Controller reconnect handled by SDK (exponential backoff, base 2s, cap 60s, ~25% jitter).
- `src/main.rs:112-136` -- Graceful shutdown notifies controller with active MQTT client list,
  allowing immediate client reassignment.

### Issues

**[CRITICAL]** `src/main.rs:198` -- `tokio::sync::mpsc::unbounded_channel()` between
`TenantManager` and `MqttHandler` has no backpressure. If the controller WebSocket is slow or
temporarily blocked, MQTT events accumulate unboundedly in memory. Use a bounded channel
(512-1024 capacity) with backpressure handling.

**[HIGH]** `src/tenant_manager.rs:81-93` -- In `shutdown_all`, `self.clients` is consumed via
`std::mem::take` at line 82, then `report_status` at line 90 uses `self.event_tx`. If the
receiver has already been dropped, status reports are silently lost.

## Coding Standards

### Strengths

- `edition = "2024"`, `publish = false` set correctly.
- All workspace-available dependencies use `workspace = true` except `rumqttc` (see Architecture
  Issues).
- `bail!` / `report!` / `context_to` pattern used consistently; no `Report::new()` anti-pattern.
- No `Result<T, String>`.
- `SecretString` at API boundaries.
- `src/mqtt_client.rs:218,288,332` -- MQTT reconnect loop uses `Backoff` with `tokio::select!`
  on shutdown token.
- `src/ha_discovery.rs:185-186` -- Correctly uses `Uuid::parse_str(...).ok()?` for MQTT topic
  segment parsing.

### Issues

No coding standards issues found.

## Extensibility

### Strengths

- `ServiceHandler` trait isolates MQTT-specific logic. Adding a new message type requires only a
  new match arm in `MqttHandler::on_message`.
- `TenantManager` is transport-agnostic. Holds `MqttHandle` values and calls `start()` and
  `shutdown()`. Changing the underlying MQTT client library would be confined to
  `mqtt_client.rs`.
- `status_sender: Option<...>` allows status reporting channel to be omitted in tests.
- Lease-based tenant distribution allows horizontal scaling.

### Issues

**[LOW]** `src/cli.rs:17` -- `max_tenants = 0` means "unlimited" via implicit convention. The
`--max-tenants` argument uses `0` as sentinel for "unlimited" with no type-system enforcement.
If controller interprets `0` literally as "zero allowed tenants" it would silently starve the
instance. `Option<NonZeroU32>` would make the sentinel explicit.
