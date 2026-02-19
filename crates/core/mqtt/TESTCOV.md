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
- **Tenant manager** (`tenant_manager.rs`, 60.1% coverage, 213 lines): 85 uncovered lines include tenant assignment updates,
  client removal, and concurrent tenant rebalancing. Risk: tenant management bugs could cause duplicate or missing Home Assistant
  device updates.
- **MQTT client** (`mqtt_client.rs`, 61.4% coverage, 272 lines): 105 uncovered lines include MQTT message publishing,
  subscription management, connection error recovery, and reconnection with credential refresh. Risk: client bugs could cause
  stale or missing MQTT state.

## Test Recommendations

1. **Tenant assignment update tests** — Test tenant add/remove/rebalance scenarios with multiple MQTT service instances. Covers
   `tenant_manager.rs` gaps (Tier 2). Mock controller lease notifications.
2. **MQTT client reconnection tests** — Test reconnection after broker disconnect, credential refresh on reconnect, and message
   retry. Covers `mqtt_client.rs` gaps (Tier 2). Use mock MQTT broker or `rumqttc` test helpers.
3. **MQTT publish/subscribe tests** — Test Home Assistant discovery message publishing, state updates, and topic management.
   Covers `mqtt_client.rs` gaps (Tier 2). Mock MQTT event loop.
4. **Service lifecycle integration test** — Test startup, tenant receipt, broker connection, and shutdown sequence. Covers
   `main.rs` (Tier 2). Requires mock controller and MQTT broker.
