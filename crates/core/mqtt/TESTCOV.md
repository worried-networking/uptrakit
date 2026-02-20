# Test Coverage: uptrakit-mqtt

> Generated: 2026-02-20 | Tool: cargo-llvm-cov 0.8.4 | Rust: 1.93.1

## Summary

| Metric | Value |
| -------- | ------- |
| Line coverage | 56.9% (506 / 890) |
| Function coverage | 62.6% (57 / 91) |
| Test count | 29 |

## Coverage by Module

| File | Line % | Lines | Function % | Functions |
| ------ | -------- | ------- | ------------ | ----------- |
| cli.rs | 100.0% | 99/99 | 100.0% | 8/8 |
| tenant_manager.rs | 81.5% | 243/298 | 82.9% | 29/35 |
| mqtt_client.rs | 61.0% | 164/269 | 66.7% | 20/30 |
| main.rs | 0.0% | 0/224 | 0.0% | 0/18 |

## Uncovered Critical Paths

### Tier 2 — Business-Logic

- **Service main loop** (`main.rs`, 0% coverage, 224 lines): MQTT service startup, controller connection, tenant assignment
  receipt, MQTT broker connection, and graceful shutdown. Risk: main loop bugs could prevent the MQTT service from functioning.
- **MQTT client remaining gaps** (`mqtt_client.rs`, 61% coverage, 269 lines): 105 uncovered lines include MQTT message
  publishing, reconnection with credential refresh, and subscription management. Risk: client bugs could cause stale or missing
  MQTT state.
- **Tenant manager remaining gaps** (`tenant_manager.rs`, 81.5% coverage): `start_or_update_client` requires a real MQTT broker
  connection. Config parsing, lifecycle management, and assignment routing are now tested.

## Test Recommendations

1. **MQTT client reconnection tests** — Test reconnection after broker disconnect, credential refresh on reconnect, and message
   retry. Covers `mqtt_client.rs` gaps (Tier 2). Use mock MQTT broker or `rumqttc` test helpers.
2. **MQTT publish/subscribe tests** — Test Home Assistant discovery message publishing, state updates, and topic management.
   Covers `mqtt_client.rs` gaps (Tier 2). Mock MQTT event loop.
3. **Service lifecycle integration test** — Test startup, tenant receipt, broker connection, and shutdown sequence. Covers
   `main.rs` (Tier 2). Requires mock controller and MQTT broker.
