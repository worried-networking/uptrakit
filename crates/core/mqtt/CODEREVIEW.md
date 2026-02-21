# Code Review: `uptrakit-mqtt`

Reviewed: `src/main.rs` (343 lines), `src/mqtt_client.rs` (439 lines),
`src/tenant_manager.rs` (311 lines), `src/cli.rs` (157 lines), `Cargo.toml`.

## Summary

The MQTT crate is clean and well-structured with proper credential redaction,
hash-based config change detection, and graceful shutdown. It serves as a
good example of a service built with the SDK.

## Role in the Architecture

The MQTT service is a standalone binary that bridges Uptrakit with MQTT
brokers and Home Assistant. It uses the service SDK for lifecycle management
and the wire protocol for controller communication.

**Clean dependency chain** -- depends on `uptrakit-service-sdk`,
`uptrakit-internal-wire`, `uptrakit-build-info`, and
`uptrakit-shared-macros`. No provider, database, or web-api dependencies.
Validates the service SDK's extensibility: if a new service type follows the
same pattern, it would have a similarly clean dependency chain.

## Findings

### Info

#### I1: Credential redaction in `Debug` is thorough

**File:** `src/mqtt_client.rs:31-43`

The custom `Debug` impl always shows `[REDACTED]` for both `username` and
`password`, regardless of whether they are `Some` or `None`. Tests
(lines 339-395) verify this behavior comprehensively, including the case
where fields are `None`.

#### I2: Platform-portable signal handling

**File:** `src/main.rs:180-312`

SIGTERM and SIGHUP use `#[cfg(unix)]` / `#[cfg(not(unix))]` guards with
`futures_util::future::pending()` fallbacks. This is the correct pattern
and should be replicated in the agent crate.

#### I3: Hash-based config change detection

**File:** `src/tenant_manager.rs:91-101`

`start_or_update_client` computes a hash of relevant config fields and
skips restart if unchanged. The hash excludes `enabled`, `mqtt_client_id`,
`tenant_id`, and `updated_at` (line 305 test confirms `updated_at` is
excluded). This avoids unnecessary MQTT reconnections.

#### I4: `FuturesUnordered` for parallel shutdown

**File:** `src/tenant_manager.rs:75-88`

`shutdown_all` uses `FuturesUnordered` to shut down all MQTT clients
concurrently rather than sequentially. Good practice for minimizing
shutdown time.

#### I5: Lease-based multi-instance tenant distribution

Multiple MQTT service instances can run concurrently with coordinated
tenant assignment. This validates the SDK's extensibility for multi-instance
service types.

#### I6: Test coverage is solid

Tests cover MQTT options configuration, credential handling, TLS transport,
shutdown timeout behavior, status topic formatting, wire config conversion,
default port fallback, and hash stability.
