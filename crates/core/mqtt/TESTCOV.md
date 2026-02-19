# Test Coverage: uptrakit-mqtt

> Generated: 2026-02-19 | Tool: cargo-llvm-cov 0.8.4 | Rust: 1.93.1

## Summary

| Metric | Value |
| -------- | ------- |
| Line coverage | 48.8% (394 / 808) |
| Function coverage | 45.6% (36 / 79) |
| Test count | 21 |

## Coverage by Module

| File | Line % | Lines | Function % | Functions |
| ------ | -------- | ------- | ------------ | ----------- |
| cli.rs | 100.0% | 99/99 | 100.0% | 8/8 |
| mqtt_client.rs | 61.4% | 167/272 | 66.7% | 20/30 |
| tenant_manager.rs | 60.1% | 128/213 | 34.8% | 8/23 |
| main.rs | 0.0% | 0/224 | 0.0% | 0/18 |

## Uncovered Critical Paths

### Tier 2 — Business-Logic

- **Service main loop** (`main.rs`, 0% coverage, 224 lines): MQTT service startup, controller connection, tenant assignment
  receipt, MQTT broker connection, and graceful shutdown. Risk: main loop bugs could prevent the MQTT service from functioning.
- **Tenant manager gaps** (`tenant_manager.rs`): Remaining uncovered lines are in `start_or_update_client` which requires
  a real MQTT broker connection. Config hashing, assignment routing, and lifecycle management are now tested.
- **MQTT client** (`mqtt_client.rs`, 61.4% coverage, 272 lines): 105 uncovered lines include MQTT message publishing,
  subscription management, connection error recovery, and reconnection with credential refresh. Risk: client bugs could cause
  stale or missing MQTT state.

## Test Recommendations

1. **MQTT client reconnection tests** — Test reconnection after broker disconnect, credential refresh on reconnect, and message
   retry. Covers `mqtt_client.rs` gaps (Tier 2). Use mock MQTT broker or `rumqttc` test helpers.
2. **MQTT publish/subscribe tests** — Test Home Assistant discovery message publishing, state updates, and topic management.
   Covers `mqtt_client.rs` gaps (Tier 2). Mock MQTT event loop.
3. **Service lifecycle integration test** — Test startup, tenant receipt, broker connection, and shutdown sequence. Covers
   `main.rs` (Tier 2). Requires mock controller and MQTT broker.
