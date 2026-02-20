# Code Review: `uptrakit-mqtt`

Reviewed: `src/main.rs` (343 lines), `src/mqtt_client.rs` (439 lines),
`src/tenant_manager.rs` (311 lines), `src/cli.rs` (157 lines), `Cargo.toml`.

## Summary

The MQTT crate is clean and well-structured with proper credential redaction,
hash-based config change detection, and graceful shutdown. It serves as a
good example of a service built with the SDK. Key issues are limited TLS
configuration for MQTT broker connections, a missing handler for
`MqttClientCreated`, and no proactive certificate renewal timer.

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

### Medium

#### ~~M1: TLS configuration uses empty CA — no pinned CA support~~ (FIXED)

**Resolution:** Added optional `ca_pem: Option<SecretString>` field
end-to-end: DB entity (`ca_cert_pem` as `EncryptedString`), wire protocol
(`MqttTenantConfig.ca_pem`), `MqttConfig`, web API types
(create/update/response), store, routes, CLI, and frontend. When present,
the PEM bytes are passed to `TlsConfiguration::Simple { ca }`. The CA
certificate is encrypted at rest and redacted in API responses
(`has_ca_cert: bool`).

#### ~~M2: `MqttClientCreated` message not handled~~ (RESOLVED)

**Resolution:** `MqttClientCreated` is a controller-to-controller outbox
event handled by the event poller (`event_poller.rs`), not a message sent
to the MQTT service. The MQTT service correctly ignores it via the wildcard
arm and receives client assignments exclusively via `TenantAssignments`.
No code change needed.

#### ~~M3: No proactive certificate renewal timer~~ (FIXED)

**Resolution:** Added proactive certificate renewal timer using the shared
`create_renewal_sleep()`, `update_renewal_schedule()`, and
`handle_renewal_timer()` helpers from the service-sdk. The MQTT service
now renews certificates independently of controller-pushed
`RequestCertRenewal` messages, matching the agent's behavior.

### Low

#### ~~L1: Credentials stored as plain `String` in `MqttConfig`~~ (FIXED)

**Resolution:** `MqttConfig.username` and `MqttConfig.password` are now
`Option<SecretString>`. `build_config_from_wire()` clones the `SecretString`
instead of extracting plaintext. Credentials are only exposed at the point
of passing to `MqttOptions::set_credentials()`.

#### L2: `compute_config_hash` uses `DefaultHasher` (non-deterministic)

**File:** `src/tenant_manager.rs:177-188`

`DefaultHasher` uses `SipHash` which is randomized per process (different
seed each run). This is fine for within-process change detection (the
intended use), but worth noting that hashes are not comparable across
process restarts.

**Recommendation:** No change needed — the hash is only used for
same-process config dedup. Add a brief comment noting this.

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
