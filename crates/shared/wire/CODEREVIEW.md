# Wire Protocol Code Review

Reviewed: 2026-02-08
Scope: `uptrakit-internal-wire` crate, WebSocket handlers (`service_ws.rs`, `agent_ws.rs`, `mqtt_ws.rs`), `ServiceConnectionRegistry`, `NotificationService`, `EventPoller`, enrollment WebSocket client (`enrollment::ws`).

---

## 1. Architecture

### 1.1 Strengths

- **Clean message taxonomy.** `ServiceMessage` and `ControllerMessage` as tagged enums with `#[serde(tag = "type")]` is the right pattern. All variants are well-typed with dedicated payload structs.
- **Replay protection.** The `OutgoingSeq`/`IncomingSeq` envelope mechanism is simple, correct, and well-tested with 229 unit tests.
- **Backward compatibility.** Generous use of `#[serde(default)]` and `#[serde(skip_serializing_if)]` makes the protocol forward-compatible. Tests verify this explicitly.
- **Unified entry point.** Single `/api/v1/ws/service` endpoint with `ConnectionType` dispatch is clean and avoids path explosion.
- **Boxed large variant.** The large `ExecuteUpdatePayload` is boxed in `ControllerMessage::ExecuteUpdate(Box<...>)` to avoid inflating the enum size.

### 1.2 Issues

#### A2. Duplicate broadcast helpers on `ServiceConnectionRegistry`

**Location:** `service_connections.rs:143-158`, `notification_service.rs:51-59`

`broadcast_ca_bundle_updated()` and `broadcast_request_cert_renewal()` exist on both `ServiceConnectionRegistry` and `NotificationService`. The `ServiceConnectionRegistry` variants bypass the outbox, which could lead to missed cross-controller delivery if called directly.

- **Recommendation:** Remove the convenience methods from `ServiceConnectionRegistry` or mark them `pub(crate)` with a clear doc comment that they are local-only. All cross-controller-aware callers should use `NotificationService`.

#### A3. `target_service_type` in `controller_events` is a free-form `String`

**Location:** `notification_service.rs:83`, `event_poller.rs:144,148`

`notification_service.rs` sets `target_service_type: Option<String>` with raw strings like `"agent"` or `"mqtt"`. The `EventPoller` matches on these with string literals. A typo in either site silently drops events.

- **Recommendation:** Use a shared enum or constants for the service type discriminator.

#### A4. `UpdateOutput` messages do a read-modify-write on `update_history.output` per line — PARTIALLY FIXED

**Location:** `agent_ws.rs:333-343`

Each `UpdateOutput` message reads the full `update_history` row, appends the new output line, and writes the entire `output` field back. For updates that produce thousands of output lines, this is N full-row reads and N full-column writes with growing payload size.

- **Recommendation:** Use a SQL `CONCAT`/`||` append operation via a raw `UPDATE ... SET output = output || $1` expression, or buffer output server-side and flush periodically.
- **Resolution:** A 1 MB output cap (`MAX_UPDATE_OUTPUT_BYTES`) prevents unbounded growth. SQL-level concat was not feasible due to SeaORM cross-backend limitations (`.concat()` is Postgres-only via `PgExpr`). The read-modify-write pattern remains but is bounded.

---

## 2. Security & Safety

#### S3. No rate limiting on WebSocket message processing — PARTIALLY FIXED

**Location:** All WebSocket handler loops (`agent_ws.rs`, `mqtt_ws.rs`)

Once a WebSocket connection is established (even anonymous), there's no throttling on how many messages per second can be processed. A flood of `Ping` messages from an enrolled service would generate a DB query per ping (for approval polling in `agent_ws.rs:508-530`).

- **Recommendation:** Add per-connection message rate limiting, or at least rate-limit the approval polling (e.g., check DB at most once every 5 seconds instead of on every ping).
- **Resolution:** Approval DB polling is now decoupled from client pings via a dedicated `APPROVAL_POLL_INTERVAL` (5 seconds) `tokio::time::interval` in both agent and MQTT enrolled loops. Per-connection message rate limiting remains a future improvement.

#### S7. Enrollment token is transmitted in plaintext in the `EnrollPayload`

**Location:** `wire/src/lib.rs:182-193`

While the connection is TLS-encrypted, the enrollment token appears as a plaintext field in the wire protocol JSON. If any logging or debugging captures the raw message text, the token is exposed.

- **Recommendation:** Document that message-level logging must never log `EnrollPayload` fields (or mask the token in log output).

---

## 3. High Availability (Multi-Controller)

#### H1. Possible message loss during service reconnection

**Location:** `notification_service.rs:38-41`

`NotificationService::send()` delivers locally AND writes to the outbox unconditionally. If the target service disconnects from Controller A and reconnects to Controller B between the local send and the outbox poll, the message is lost — the local send succeeded on A (but the connection dropped), and Controller B's poller picks it up but may not find the service if it reconnected before the event was polled.

- **Impact:** Low probability but possible message loss during service migration between controllers.
- **Recommendation:** For critical messages, services should reconcile state on reconnect. The existing `ca_bundle_hash` in `ServiceSettings` is a good example of this pattern.

#### H3. EventPoller startup cursor initialization

**Location:** `event_poller.rs:38`

`fetch_max_id()` initializes the cursor to the current max event ID. Events written between this call and the first `poll_events()` tick (up to 1 second later) by another controller will be caught correctly (polled with `id > last_seen_id`). This is correct behavior but worth noting as a design assumption.

#### H4. 1-hour event cleanup may be too aggressive

**Location:** `event_poller.rs:202-221`

Events older than 1 hour are deleted. If a controller is down for more than 1 hour (e.g., during a long deployment or disaster recovery), it will miss all events created and cleaned up during its downtime.

- **Impact:** After a prolonged outage, a controller could miss CA bundle updates, approval notifications, or MQTT config changes.
- **Recommendation:** Make the cleanup TTL configurable and consider a longer default (e.g., 24 hours), or use a startup reconciliation mechanism.

#### H5. MQTT lease coordination has a TOCTOU gap

**Location:** `mqtt_ws.rs:148-171`

The controller checks available leases and assigns them, but between the check and the assignment, another controller could assign the same lease. The DB lease table provides the authoritative state, but the in-memory `ServiceConnectionRegistry` state may diverge from the DB.

- **Recommendation:** Use database-level locking (SELECT FOR UPDATE or similar) in the lease coordinator to prevent double-assignment.

#### H7. Broadcast during service migration

**Location:** `notification_service.rs:45-48`

When Controller A broadcasts a `CaBundleUpdated` message, it sends to all locally connected services AND writes one outbox event. Controller B's poller picks up the event and broadcasts to its locally connected services. If a service disconnects from A and reconnects to B during the broadcast window, it might miss the message. The existing `ca_bundle_hash` in `ServiceSettings` mitigates this for CA updates specifically.

- **Recommendation:** Extend the reconcile-on-reconnect pattern to other critical state changes.

---

## 4. Minor / Code Quality

#### M1. `mpsc` channel buffer size is hardcoded to 16

**Location:** `service_connections.rs:56,80`

The push message channel capacity is fixed at 16. If the controller is slow to write to the WebSocket (e.g., network backpressure) and multiple push events arrive rapidly, the channel fills up. Consider making this configurable or documenting the rationale.

#### M2. `get_instance_for_mqtt_client` is O(n)

**Location:** `service_connections.rs:259-267`

Iterates all connections to find which service holds an MQTT client. Could use a reverse index `HashMap<mqtt_client_id, service_id>` for O(1) lookup.

#### M3. `wire_hook_shell` conversion function is mechanical boilerplate

**Location:** `agent_ws.rs:823-837`

Two identical `HookShell` enums exist in `web-api-types` and `wire`. Consider a `From` impl or a single shared definition.

#### M4. `notification_service.rs` tests don't verify outbox writes

**Location:** `notification_service.rs:104-143`

The tests use an in-memory SQLite DB without running migrations, so the outbox INSERT silently fails. The tests only verify the code path doesn't panic, not that it works correctly.

- **Recommendation:** Either run migrations in the test DB or add integration tests that verify outbox contents.

#### M5. No explicit lint configuration in the wire crate

**Location:** `wire/Cargo.toml`

The wire crate's `Cargo.toml` doesn't configure lints. Relying on workspace-level clippy is fine, but making it explicit prevents accidental regressions.

---

## 5. Additional Findings (Deep Dive)

#### D1. No connection deduplication — stale WebSocket loops on reconnect [IMPORTANT]

**Location:** `service_connections.rs:55-67,74-91`, `agent_ws.rs:64`, `mqtt_ws.rs:122-125`

When `register_agent()` or `register_mqtt()` is called, the new `ServiceConnection` is inserted into the `HashMap` via `.insert()`, which silently replaces the previous entry. However, the old WebSocket handler loop is still running in its `tokio::select!` with a now-orphaned `push_rx` receiver. The old `mpsc::Sender` was dropped when the HashMap entry was replaced, so `push_rx.recv()` will return `None` and the old loop will terminate — but only when it polls the push channel. If the old loop is blocked waiting on `stream.next()` (no WebSocket message arriving), it remains alive until the old TCP connection times out or the client sends a ping.

- **Impact:** During the overlap window, both the old and new handler loops can process incoming messages from their respective WebSocket connections. The old handler's DB writes succeed but its push channel is dead. Slight data inconsistency and wasted resources.
- **Recommendation:** Before inserting a new entry, close the old push channel explicitly (which terminates the old loop), or track a connection generation to detect stale handlers.

---

## Summary Table

| ID | Category | Severity | Summary | Status |
|----|----------|----------|---------|--------|
| A2 | Architecture | Minor | Duplicate broadcast helpers | Open |
| A3 | Architecture | Minor | Free-form service type string | Open |
| A4 | Architecture | Important | Read-modify-write per output line | **Partially fixed** |
| S3 | Security | Important | No WS message rate limiting | **Partially fixed** |
| S7 | Security | Minor | Enrollment token in plaintext in payload | Open |
| H1 | HA | Important | Message loss during service migration | Open |
| H3 | HA | Minor | EventPoller startup cursor assumption | Open |
| H4 | HA | Important | 1-hour cleanup TTL too aggressive | Open |
| H5 | HA | Important | MQTT lease TOCTOU gap | Open |
| H7 | HA | Minor | Broadcast gap during service migration | Open |
| M1 | Quality | Minor | Hardcoded mpsc buffer size | Open |
| M2 | Quality | Minor | O(n) MQTT client lookup | Open |
| M3 | Quality | Minor | Boilerplate HookShell conversion | Open |
| M4 | Quality | Minor | Tests don't verify outbox writes | Open |
| M5 | Quality | Minor | No explicit lint config | Open |
| D1 | Deep Dive | Important | No connection deduplication on reconnect | Open |

---

## Fix Plans

| Plan | Addresses | Summary | Status |
|------|-----------|---------|--------|
| FP-6 | S6, S5 | Replace `expect()` with `LazyLock`, fix silent timestamp truncation | **DONE** |
| FP-7 | H4, H1 | Configurable event cleanup TTL, startup reconciliation | |
| FP-8 | H5 | Atomic lease acquisition with DB-level conflict handling | |
| FP-9 | A2, A3 | Remove duplicate broadcast helpers, type-safe service type discriminator | |
| FP-10 | M2, M1, M4 | Reverse index for MQTT lookup, named buffer constant, proper tests | |
| FP-11 | S7 | Prevent enrollment token and sensitive payloads from leaking into logs | |
| FP-12 | H3, H7 | EventPoller startup cursor safety and reconnect state reconciliation | |
| FP-13 | M3 | Unify `HookShell` enum across wire and web-api-types crates | |
| FP-14 | M5 | Add explicit lint configuration to the wire crate | |
| FP-15 | H3 | Add wire protocol version negotiation | |
| FP-16 | D1 | Connection deduplication with generation tracking | |
| FP-17 | D2 | Validate UpdateHistory ownership against agent | **DONE** |
| FP-18 | D3 | Reorder register_agent before deliver_pending_updates | **DONE** |
| FP-19 | D4 | Non-blocking broadcast with sender snapshot | **DONE** |
| FP-20 | D5 | Add configurable timeout to enrollment client wait_for_approval | **DONE** |

### FP-6. Replace runtime `expect()` and fix silent timestamp truncation — DONE

**Addresses:** S6, S5

**Problem:** `MIN_AGENT_VERSION` is parsed with `.expect()` on every `ReportHostInfo` message, which violates the project rule against `panic!`/`unwrap` outside lock guards. The `utc_datetime_millis` serializer silently truncates `i128` to `i64` with `as`, which could mask bugs for extreme timestamp values.

**Implementation:**

1. **Parsed `MIN_AGENT_VERSION` once using `LazyLock`.**
   - Added module-level static in `agent_ws.rs`:
     ```rust
     static MIN_AGENT_VER: LazyLock<semver::Version> = LazyLock::new(|| {
         semver::Version::parse(MIN_AGENT_VERSION)
             .expect("MIN_AGENT_VERSION must be valid semver")
     });
     ```
   - References `*MIN_AGENT_VER` in the handler instead of parsing each time.

2. **Replaced `as i64` with `i64::try_from()` in `utc_datetime_millis::serialize`.**
   - In `wire/src/lib.rs`, changed to:
     ```rust
     let millis_i64 = i64::try_from(millis).map_err(serde::ser::Error::custom)?;
     serializer.serialize_i64(millis_i64)
     ```

3. **Added 4 timestamp roundtrip tests** covering practical range, epoch, far future (year 9999), and negative timestamps.

**Files modified:** `agent_ws.rs`, `wire/src/lib.rs`

---

### FP-7. Harden event cleanup TTL and add startup reconciliation

**Addresses:** H4, H1

**Problem:** Events older than 1 hour are unconditionally deleted. If a controller is down for more than 1 hour (deployment, disaster recovery), it misses all events created and cleaned up during its downtime. Additionally, services that migrate between controllers during a broadcast window may miss messages.

**Plan:**

1. **Make cleanup TTL configurable.**
   - Add a constant `EVENT_CLEANUP_TTL_HOURS: u64 = 24` in `event_poller.rs`.
   - Change `cleanup_old_events()` to use this constant instead of the hardcoded `time::Duration::hours(1)`.
   - Optionally expose this as a `SettingKey` so it can be tuned per deployment without recompilation.

2. **Extend cleanup interval proportionally.**
   - With a 24-hour TTL, the 5-minute cleanup interval is fine (low overhead, eventual cleanup).

3. **Add startup reconciliation for critical state.**
   - On controller startup (or when a service connects), push the current authoritative state rather than relying solely on incremental events:
     - `ServiceSettings` is already sent on connect (includes `ca_bundle_hash`) — good.
     - `TenantAssignments` are already sent on MQTT registration — good.
   - Add a comment documenting which state is reconciled on connect vs. which relies on events.

4. **Add a "full sync" mechanism for prolonged outages.**
   - If the `EventPoller` detects that `fetch_max_id()` returns an ID much larger than its `last_seen_id` (gap > configurable threshold, e.g., 10000 events), log a warning that a full state reconciliation may be needed.
   - This is informational only — the controller continues normally, but the operator is alerted.

5. **Document reconnect-resilience guarantees.**
   - In `ARCHITECTURE.md`, document which message types are reconciled on reconnect (safe to miss) vs. which are fire-and-forget (may be lost if missed).

**Files to modify:**
- `crates/ui/web-api/src/event_poller.rs` — configurable TTL, gap detection
- `ARCHITECTURE.md` — document reconciliation guarantees

**Testing:**
- Unit test: cleanup respects the configured TTL
- Unit test: gap detection logs a warning when `max_id - last_seen_id` exceeds threshold

---

### FP-8. Fix MQTT lease coordination TOCTOU with DB-level locking

**Addresses:** H5

**Problem:** In `mqtt_ws.rs:148-171`, the controller reads available leases and assigns them in separate operations. Between the read and the write, another controller could assign the same lease, leading to double-assignment. The in-memory `ServiceConnectionRegistry` may diverge from the DB `mqtt_leases` table.

**Plan:**

1. **Use atomic INSERT with conflict handling for lease acquisition.**
   - Replace the check-then-insert pattern in the lease coordinator with a single atomic operation:
     ```sql
     INSERT INTO mqtt_leases (id, tenant_id, mqtt_client_id, instance_id, acquired_at, last_heartbeat_at)
     SELECT $1, mc.tenant_id, mc.id, $2, NOW(), NOW()
     FROM mqtt_clients mc
     LEFT JOIN mqtt_leases ml ON ml.mqtt_client_id = mc.id
     WHERE mc.enabled = true
       AND ml.id IS NULL
     LIMIT $3
     ON CONFLICT (mqtt_client_id) DO NOTHING
     ```
   - The `ON CONFLICT DO NOTHING` ensures that if another controller inserted a lease for the same `mqtt_client_id` between our SELECT and INSERT, we silently skip it.
   - SeaORM supports `on_conflict` via `Insert::on_conflict()`.

2. **Verify assignments after INSERT.**
   - After the atomic insert, SELECT back the leases that were actually acquired (where `instance_id` matches ours).
   - Only register these in `ServiceConnectionRegistry`.
   - This closes the TOCTOU gap: the DB is the single source of truth.

3. **Add periodic reconciliation between in-memory and DB state.**
   - Every 60 seconds (or on each heartbeat), compare `ServiceConnectionRegistry.assigned_mqtt_clients` with the `mqtt_leases` table.
   - Release any in-memory assignments that no longer exist in the DB (stolen by another controller after a stale heartbeat).
   - Acquire any DB leases assigned to this controller's instance that aren't in memory (shouldn't happen normally, but handles edge cases).

4. **Add a stale lease reaper.**
   - A background task that runs every 60 seconds:
     ```sql
     DELETE FROM mqtt_leases WHERE last_heartbeat_at < NOW() - INTERVAL '90 seconds'
     ```
   - This reclaims leases from crashed MQTT service instances.
   - The `MqttLeaseCoordinator` likely already does this; verify and harden.

**Files to modify:**
- `crates/ui/web-api/src/mqtt_lease_coordinator.rs` — atomic insert, reconciliation loop
- `crates/ui/web-api/src/routes/mqtt_ws.rs` — verify assignments after lease acquisition
- `crates/ui/web-api/src/service_connections.rs` — reconciliation helper

**Testing:**
- Integration test: two controllers attempt to lease the same MQTT client simultaneously — only one succeeds
- Unit test: reconciliation releases orphaned in-memory assignments
- Unit test: stale lease reaper cleans up old leases

---

### FP-9. Consolidate broadcast helpers and type-safe service type discriminator

**Addresses:** A2, A3

**Problem:** `broadcast_ca_bundle_updated()` and `broadcast_request_cert_renewal()` exist on both `ServiceConnectionRegistry` (local-only) and `NotificationService` (local + outbox). Callers can accidentally use the local-only version and miss cross-controller delivery. Additionally, `target_service_type` in the outbox is a free-form `String` matched with string literals — a typo silently drops events.

**Plan:**

1. **Remove convenience broadcast methods from `ServiceConnectionRegistry`.**
   - Delete `broadcast_ca_bundle_updated()` and `broadcast_request_cert_renewal()` from `service_connections.rs`.
   - All callers should use `NotificationService` for cross-controller-safe broadcasting.
   - Keep only the generic `broadcast()`, `broadcast_by_type()`, and `send()` on `ServiceConnectionRegistry` — these are the building blocks used by `NotificationService` internally.

2. **Audit all call sites.**
   - Search the codebase for all calls to `ServiceConnectionRegistry::broadcast_ca_bundle_updated` and `broadcast_request_cert_renewal`.
   - Replace them with the corresponding `NotificationService` methods.
   - The only exception is `broadcast_server_restarting_scattered()`, which is intentionally local-only (documented in `AGENTS.md`).

3. **Introduce a `TargetServiceType` enum for outbox events.**
   - Define in `notification_service.rs` (or a shared location):
     ```rust
     #[derive(Debug, Clone, Copy)]
     enum TargetServiceType {
         Agent,
         Mqtt,
     }

     impl TargetServiceType {
         fn as_str(self) -> &'static str {
             match self {
                 Self::Agent => "agent",
                 Self::Mqtt => "mqtt",
             }
         }
     }
     ```
   - Change `write_outbox_event()` to accept `Option<TargetServiceType>` instead of `Option<&str>`.
   - Change `EventPoller::deliver_event()` to match on the same enum's string representations, or better yet, parse the string back into the enum with a fallback.

4. **Add a compile-time guarantee.**
   - Use the enum in all call sites. This means a new service type (e.g., a future "monitor" service) would require updating the enum, making it impossible to introduce a typo.

**Files to modify:**
- `crates/ui/web-api/src/service_connections.rs` — remove `broadcast_ca_bundle_updated`, `broadcast_request_cert_renewal`
- `crates/ui/web-api/src/notification_service.rs` — add `TargetServiceType` enum, update `write_outbox_event` signature
- `crates/ui/web-api/src/event_poller.rs` — parse `target_service_type` string into enum
- All call sites that use the removed convenience methods (search for `broadcast_ca_bundle_updated`, `broadcast_request_cert_renewal`)

**Testing:**
- Compile-time: removing the old methods causes build errors at any incorrect call site (the compiler does the work)
- Unit test: `TargetServiceType::as_str()` round-trips through the event poller's parsing
- Unit test: unknown `target_service_type` strings in the DB are logged and skipped, not silently dropped

---

### FP-10. Improve `ServiceConnectionRegistry` efficiency and test coverage

**Addresses:** M2, M1, M4

**Problem:** `get_instance_for_mqtt_client()` scans all connections (O(n)) to find which service holds an MQTT client. The `mpsc` channel buffer size is hardcoded to 16 with no documentation. `NotificationService` tests don't verify outbox writes because the test DB has no schema.

**Plan:**

1. **Add a reverse index for MQTT client lookups.**
   - Add a second `HashMap<Uuid, Uuid>` to `ServiceConnectionRegistry` mapping `mqtt_client_id -> service_id`.
   - Update `assign_mqtt_client()` to insert into the index.
   - Update `release_mqtt_client()` to remove from the index.
   - Update `unregister()` to remove all entries for the disconnecting service.
   - Change `get_instance_for_mqtt_client()` to a single `HashMap::get()` — O(1).

2. **Make the `mpsc` channel buffer size a named constant with documentation.**
   - Define `const PUSH_CHANNEL_CAPACITY: usize = 32;` at the top of `service_connections.rs`.
   - Add a doc comment explaining the rationale.

3. **Add proper integration tests for `NotificationService`.**
   - Create a test helper that sets up an in-memory SQLite DB with migrations applied.
   - Test `send()`, `broadcast()`, and verify outbox contents.

4. **Add tests for `EventPoller` delivery routing.**
   - Test targeted delivery, type-filtered delivery, and broadcast.

**Files to modify:**
- `crates/ui/web-api/src/service_connections.rs` — reverse index, named constant
- `crates/ui/web-api/src/notification_service.rs` — integration tests with migrated DB
- `crates/ui/web-api/src/event_poller.rs` — delivery routing tests

**Testing:**
- Unit test: `get_instance_for_mqtt_client` returns correct result after assign/release/unregister
- Unit test: reverse index stays consistent after interleaved assign/release operations
- Integration test: full outbox write + poll + deliver cycle with real DB schema

---

### FP-11. Prevent enrollment token and sensitive payloads from leaking into logs

**Addresses:** S7

**Problem:** `EnrollPayload.enrollment_token` contains the raw pre-shared secret. Both are transmitted as plaintext JSON fields over the (TLS-encrypted) WebSocket. If any middleware, debug logging, or error handler captures the raw message text, these secrets are exposed. The `Debug` derive on all payload structs means `tracing::debug!("{:?}", payload)` would print them too.

**Plan:**

1. **Implement a custom `Debug` for sensitive payload structs.**
   - For `EnrollPayload`, `EnrolledPayload`, and `MqttTenantConfig`, override `Debug` to redact sensitive fields.

2. **Remove the derived `Debug` from affected structs.**
   - Replace `#[derive(Debug, ...)]` with a manual `Debug` impl for sensitive structs.

3. **Audit all `tracing::*!` calls in WebSocket handlers for raw message logging.**

4. **Add a lint comment in the wire crate** warning that message text must never be logged verbatim.

**Files to modify:**
- `crates/shared/wire/src/lib.rs` — custom `Debug` impls
- WebSocket handler files — audit log statements

**Testing:**
- Unit test: `format!("{:?}", ...)` does not contain secrets for sensitive payloads

---

### FP-12. EventPoller startup cursor safety and reconnect state reconciliation

**Addresses:** H3, H7

**Problem:** `EventPoller::fetch_max_id()` initializes the cursor to the current max event ID at startup. The initialization creates a race window, and services migrating between controllers during a broadcast window may miss messages.

**Plan:**

1. **Use a safety margin on the startup cursor** (`max_id - 100`).
2. **Push authoritative state on service reconnect** (document reconciliation guarantees).
3. **Add a `last_connected_at` timestamp** to skip stale outbox events for recently-reconnected services.

**Files to modify:**
- `crates/ui/web-api/src/event_poller.rs` — startup safety margin, age-based skip
- `crates/ui/web-api/src/service_connections.rs` — `last_connected_at` field
- `crates/ui/web-api/src/routes/service_ws.rs` — document reconnect reconciliation

**Testing:**
- Unit test: startup cursor is `max_id - 100` (not `max_id`)
- Unit test: events older than a service's connection time are skipped

---

### FP-13. Unify `HookShell` enum across wire and web-api-types crates

**Addresses:** M3

**Problem:** Two identical `HookShell` enums exist — one in `uptrakit-internal-wire` and one in `uptrakit-web-api-types`. The `wire_hook_shell()` conversion function is pure boilerplate.

**Plan:**

1. Re-export the wire crate's `HookShell` from `web-api-types` instead of defining a duplicate.
2. Remove the `wire_hook_shell()` conversion function.

**Files to modify:**
- `crates/shared/web-api-types/Cargo.toml` — add wire crate dependency (if missing)
- `crates/shared/web-api-types/src/update_hooks.rs` — replace local `HookShell` with re-export
- `crates/ui/web-api/src/routes/agent_ws.rs` — delete `wire_hook_shell()`, use `HookShell` directly

---

### FP-14. Add explicit lint configuration to the wire crate

**Addresses:** M5

**Problem:** The wire crate's `Cargo.toml` has no `[lints]` section. Making it explicit prevents accidental regressions.

**Plan:**

1. Add `[lints] workspace = true` or crate-local lints to the wire crate's `Cargo.toml`.
2. Fix any new warnings surfaced by stricter lints.

**Files to modify:**
- `crates/shared/wire/Cargo.toml` — add `[lints]` section
- `crates/shared/wire/src/lib.rs` — fix any new warnings

---

### FP-15. Add wire protocol version negotiation

**Addresses:** H3 (related), forward-looking architectural improvement

**Problem:** The wire protocol has no version negotiation mechanism. Unknown message `type` values cause deserialization failures.

**Plan:**

1. Add `protocol_version` field to `ReportHostInfoPayload`, `MqttRegisterPayload`, and `ServiceSettingsPayload`.
2. Define a `PROTOCOL_VERSION` constant in the wire crate.
3. Log version mismatches on the controller side (informational, not enforcement).
4. Update `asyncapi.yaml`.

**Files to modify:**
- `crates/shared/wire/src/lib.rs` — `PROTOCOL_VERSION` constant, new fields
- `crates/shared/wire/asyncapi.yaml` — document new fields
- WebSocket handler files — log protocol version

---

### FP-16. Connection deduplication with generation tracking

**Addresses:** D1

**Problem:** When a service reconnects, the old WebSocket handler loop continues running until it discovers the broken push channel. During the overlap, both handler loops can process incoming messages.

**Plan:**

1. Add a connection generation counter and `CancellationToken` to `ServiceConnectionRegistry`.
2. On replacement, cancel the old token for immediate teardown.
3. Handler loops add `cancel_token.cancelled()` as a `tokio::select!` branch.

**Files to modify:**
- `crates/ui/web-api/src/service_connections.rs` — generation, `CancellationToken`
- WebSocket handler files — accept and use `CancellationToken`

---

### FP-17. Validate UpdateHistory ownership against the requesting agent — DONE

**Addresses:** D2

**Problem:** When processing `UpdateStarted`, `UpdateOutput`, and `UpdateResult` messages from an authenticated agent, the handler looks up the `update_history` record by `payload.update_history_id` but never verifies that the record belongs to a host linked to the current `agent_id`. A compromised or misbehaving agent could manipulate any update record.

**Implementation:**

1. **Added `validate_update_ownership()` helper function.**
   - Fetches the `update_history` record and verifies `host_id` is in the agent's linked host set.
   - Returns `Forbidden` error for unauthorized access attempts.

2. **Added `load_linked_host_ids()` helper function.**
   - Queries `service_host` table for all host IDs linked to the agent.

3. **Applied validation to all three update message handlers.**
   - `UpdateStarted`, `UpdateOutput`, `UpdateResult` all call `validate_update_ownership()` before processing.

4. **Cached `linked_host_ids: HashSet<Uuid>` per connection.**
   - Initialized at connection start, refreshed after `ReportHostInfo` (which may link new hosts).

**Files modified:** `agent_ws.rs`

---

### FP-18. Reorder register_agent before deliver_pending_updates — DONE

**Addresses:** D3

**Problem:** In `handle_agent_authenticated()`, `deliver_pending_updates()` was called before `register_agent()`. During the gap, outbox events targeting this agent would fail delivery silently.

**Implementation:**

1. **Swapped the order: `register_agent()` now runs before `deliver_pending_updates()`.**
   - The agent is registered in the `ServiceConnectionRegistry` first, so any concurrent outbox events are captured by the push channel.
   - Added comment documenting the ordering rationale.

2. **Duplicate deliveries are handled gracefully.**
   - The agent already handles duplicate `ExecuteUpdate` messages idempotently.

**Files modified:** `agent_ws.rs`

---

### FP-19. Non-blocking broadcast with sender snapshot — DONE

**Addresses:** D4

**Problem:** `broadcast()` and `broadcast_by_type()` held the `RwLock` read guard while iterating connections and calling `sender.send(msg).await`. A slow consumer could hold the lock for an extended time, blocking all write operations and potentially causing deadlock.

**Implementation:**

1. **Refactored `send()` to snapshot sender under lock, release lock, then send.**
   ```rust
   let sender = {
       let guard = self.inner.read().await;
       guard.get(service_id).map(|c| c.sender.clone())
   };
   if let Some(sender) = sender {
       sender.send(msg).await.is_ok()
   } else { false }
   ```

2. **Applied the same pattern to `broadcast()` and `broadcast_by_type()`.**
   - Snapshot all senders (or filtered senders) under lock, release lock, then iterate and send.

**Files modified:** `service_connections.rs`

---

### FP-20. Add configurable timeout to enrollment client wait_for_approval — DONE

**Addresses:** D5

**Problem:** `wait_for_approval()`, `send_enroll()`, `request_certificate_ws()`, and `connect_ws()` all had no timeout. If the controller went down silently or approval never came, the enrollment client would block forever.

**Implementation:**

1. **Added three timeout constants:**
   - `CONNECT_TIMEOUT` (30s) — TCP connection establishment
   - `RESPONSE_TIMEOUT` (60s) — immediate request-response exchanges
   - `APPROVAL_TIMEOUT` (30min) — waiting for human approval

2. **Added three new error variants to `EnrollmentError`:**
   - `ApprovalTimeout`, `ResponseTimeout`, `ConnectionTimeout`

3. **Wrapped all blocking operations in `tokio::time::timeout()`:**
   - `connect_ws()`: TCP connect wrapped in `CONNECT_TIMEOUT`
   - `send_enroll()`: inner loop wrapped in `RESPONSE_TIMEOUT`
   - `wait_for_approval()`: inner loop wrapped in `APPROVAL_TIMEOUT`
   - `request_certificate_ws()`: inner loop wrapped in `RESPONSE_TIMEOUT`

**Files modified:** `enrollment/src/ws.rs`, `enrollment/src/error.rs`
