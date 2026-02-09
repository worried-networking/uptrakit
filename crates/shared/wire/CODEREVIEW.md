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

#### A1. MQTT password travels through the notification outbox in plaintext [CRITICAL] — FIXED (FP-2)

**Location:** `notification_service.rs:71`, `MqttTenantConfig` in `wire/src/lib.rs:549-576`

`MqttTenantConfig` includes `password: Option<String>`. When `TenantAssignments` or `TenantConfigUpdated` messages are broadcast via `NotificationService`, the full JSON (including the password) is written to the `controller_events.message_json` TEXT column.

- **Impact:** MQTT broker credentials are persisted in plaintext in the DB outbox, readable by any DB admin or backup process, and retained for up to 1 hour.
- **Recommendation:** Strip or encrypt sensitive fields before outbox serialization. See [fix plan FP-2](#fp-2-strip-mqtt-credentials-from-outbox-events).
- **Resolution:** MQTT credential-bearing messages (`TenantAssignments`, `TenantConfigUpdated`, `TenantRevoked`) are now filtered by `is_mqtt_tenant_message()` in `NotificationService` and never written to the outbox. `MqttLeaseCoordinator` delivers these directly via `ServiceConnectionRegistry`. MQTT services reconcile from DB on reconnect.

#### A2. Duplicate broadcast helpers on `ServiceConnectionRegistry`

**Location:** `service_connections.rs:143-158`, `notification_service.rs:51-59`

`broadcast_ca_bundle_updated()` and `broadcast_request_cert_renewal()` exist on both `ServiceConnectionRegistry` and `NotificationService`. The `ServiceConnectionRegistry` variants bypass the outbox, which could lead to missed cross-controller delivery if called directly.

- **Recommendation:** Remove the convenience methods from `ServiceConnectionRegistry` or mark them `pub(crate)` with a clear doc comment that they are local-only. All cross-controller-aware callers should use `NotificationService`.

#### A3. `target_service_type` in `controller_events` is a free-form `String`

**Location:** `notification_service.rs:83`, `event_poller.rs:144,148`

`notification_service.rs` sets `target_service_type: Option<String>` with raw strings like `"agent"` or `"mqtt"`. The `EventPoller` matches on these with string literals. A typo in either site silently drops events.

- **Recommendation:** Use a shared enum or constants for the service type discriminator.

#### A4. `UpdateOutput` messages do a read-modify-write on `update_history.output` per line — PARTIALLY FIXED (FP-5)

**Location:** `agent_ws.rs:333-343`

Each `UpdateOutput` message reads the full `update_history` row, appends the new output line, and writes the entire `output` field back. For updates that produce thousands of output lines, this is N full-row reads and N full-column writes with growing payload size.

- **Recommendation:** Use a SQL `CONCAT`/`||` append operation via a raw `UPDATE ... SET output = output || $1` expression, or buffer output server-side and flush periodically.
- **Resolution:** A 1 MB output cap (`MAX_UPDATE_OUTPUT_BYTES`) prevents unbounded growth. SQL-level concat was not feasible due to SeaORM cross-backend limitations (`.concat()` is Postgres-only via `PgExpr`). The read-modify-write pattern remains but is bounded.

#### A5. No maximum message size enforcement on the wire [IMPORTANT] — FIXED (FP-5)

**Location:** `service_ws.rs:187` (WebSocket upgrade), all message receive loops

Neither the wire crate nor the WebSocket handlers enforce a maximum incoming message size. A buggy or malicious service could send a multi-megabyte JSON payload, causing excessive memory allocation on the controller.

- **Impact:** Memory exhaustion DoS.
- **Recommendation:** Configure axum's `WebSocketUpgrade` with `max_frame_size` / `max_message_size` limits. See [fix plan FP-5](#fp-5-enforce-websocket-message-size-limits-and-cap-update-output).
- **Resolution:** `MAX_WS_MESSAGE_SIZE` (1 MB) is now enforced via `ws.max_message_size()` on the WebSocket upgrade in `service_ws.rs`.

---

## 2. Security & Safety

#### S1. `lookup_by_secret` brute-forces argon2 against all MQTT services [CRITICAL] — FIXED (FP-1)

**Location:** `service_ws.rs:200-232`

If the SHA-256 agent-style lookup fails, the code loads *every* non-deactivated MQTT service from the DB and runs `verify_password()` (argon2) against each one. This is an unbounded CPU-bound operation triggered by any unauthenticated WebSocket connection with a `Bearer` header.

- **Impact:** An attacker can cause sustained CPU load by sending random bearer tokens to the WebSocket endpoint. With 100 MQTT services, each attempt runs ~100 argon2 verifications.
- **Recommendation:** Use deterministic hashing (SHA-256, like agents) for MQTT enrollment secrets too, or add rate limiting to the WebSocket upgrade path. See [fix plan FP-1](#fp-1-eliminate-argon2-brute-force-in-lookup_by_secret).
- **Resolution:** `lookup_by_secret()` now uses SHA-256 hash comparison only (single DB query). The argon2 brute-force fallback has been removed.

#### S2. No connection timeout for anonymous WebSocket connections [CRITICAL] — FIXED (FP-1)

**Location:** `service_ws.rs:545-624`

`handle_anonymous()` waits indefinitely in a loop for the first `Enroll` message. A client can open the WebSocket and never send anything, holding a connection slot forever.

- **Impact:** Resource exhaustion — an attacker can open many anonymous WebSocket connections without sending any data.
- **Recommendation:** Add a `tokio::time::timeout` (e.g. 30 seconds) around the initial message receive loop. See [fix plan FP-1](#fp-1-eliminate-argon2-brute-force-in-lookup_by_secret).
- **Resolution:** `ANONYMOUS_TIMEOUT` (30 seconds) is now enforced via `tokio::time::timeout()` wrapping the anonymous handler loop.

#### S3. No rate limiting on WebSocket message processing — PARTIALLY FIXED (FP-4)

**Location:** All WebSocket handler loops (`agent_ws.rs`, `mqtt_ws.rs`)

Once a WebSocket connection is established (even anonymous), there's no throttling on how many messages per second can be processed. A flood of `Ping` messages from an enrolled service would generate a DB query per ping (for approval polling in `agent_ws.rs:508-530`).

- **Recommendation:** Add per-connection message rate limiting, or at least rate-limit the approval polling (e.g., check DB at most once every 5 seconds instead of on every ping).
- **Resolution:** Approval DB polling is now decoupled from client pings via a dedicated `APPROVAL_POLL_INTERVAL` (5 seconds) `tokio::time::interval` in both agent and MQTT enrolled loops. Per-connection message rate limiting remains a future improvement.

#### S4. Unbounded `update_history.output` growth [IMPORTANT] — FIXED (FP-5)

**Location:** `agent_ws.rs:327-343`, `agent_ws.rs:345-389`

An agent can send unlimited `UpdateOutput` messages, each appending to the `output` column. There's no size cap.

- **Impact:** Database bloat, potential OOM when loading large output fields.
- **Recommendation:** Enforce a maximum output size (e.g. 1 MB) and truncate or drop further output lines once the limit is reached. See [fix plan FP-5](#fp-5-enforce-websocket-message-size-limits-and-cap-update-output).
- **Resolution:** `MAX_UPDATE_OUTPUT_BYTES` (1 MB) cap is enforced in both `UpdateOutput` and `UpdateResult` handlers. Output exceeding the cap is dropped with a debug log.

#### S5. `utc_datetime_millis::serialize` uses `as i64` for `i128` to `i64` truncation

**Location:** `wire/src/lib.rs:253-254`

`let millis = dt.unix_timestamp_nanos() / 1_000_000; serializer.serialize_i64(millis as i64)`. The `as i64` cast silently truncates if the value falls outside the `i64` range. For any practical timestamp this is safe, but `i64::try_from(millis).map_err(...)` would be more robust and aligned with the project's safety-first philosophy.

#### S6. `MIN_AGENT_VERSION` is parsed at runtime with `.expect()`

**Location:** `agent_ws.rs:117-118`

`semver::Version::parse(MIN_AGENT_VERSION).expect(...)` will panic if the constant is ever changed to an invalid value. This violates the project rule against `unwrap`/`panic!` outside of lock guards.

- **Recommendation:** Use a `LazyLock<semver::Version>` or validate at compile time.

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

#### H2. EventPoller cursor advancement is non-transactional [CRITICAL] — FIXED (FP-3)

**Location:** `event_poller.rs:105-128`

The cursor (`new_cursor`) advances past each event after attempting delivery. If `deliver_event()` fails (e.g., the local send channel is full because the `mpsc(16)` buffer is saturated), the event is silently lost — the cursor moves forward but the message was never delivered.

- **Impact:** Message loss under backpressure.
- **Recommendation:** Only advance the cursor past successfully delivered events, or use at-least-once semantics. See [fix plan FP-3](#fp-3-make-eventpoller-cursor-advancement-delivery-aware).
- **Resolution:** `deliver_event()` and `deliver_mqtt_event()` now return `bool`. The cursor only advances past successfully delivered events. Failed deliveries are retried up to `MAX_DELIVERY_RETRIES` (3) before being skipped. Batch processing stops on first failure to preserve ordering.

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

#### H6. Enrolled loop DB polling on every ping is expensive at scale — FIXED (FP-4)

**Location:** `agent_ws.rs:508-530`, `mqtt_ws.rs:390-414`

Both agent and MQTT enrolled loops poll the `services` table on every ping for pending services. With 100 pending agents pinging every 15 seconds, that's ~400 DB queries/minute per controller.

- **Recommendation:** Only poll on every Nth ping, or use a `tokio::time::interval` alongside the ping handler to poll at a fixed, lower rate.
- **Resolution:** Both enrolled loops now use a dedicated `APPROVAL_POLL_INTERVAL` (5s) `tokio::time::interval` with a conditional `if !approved` guard, decoupled from client-controlled pings.

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

#### D2. `UpdateHistory` messages are not validated against the owning agent [IMPORTANT]

**Location:** `agent_ws.rs:306-389`

When processing `UpdateStarted`, `UpdateOutput`, and `UpdateResult`, the handler looks up the `update_history` record by `payload.update_history_id` (a UUID) but never verifies that the record belongs to a host linked to the current `agent_id`. A compromised or misbehaving agent could manipulate the status and output of any `update_history` record in the database by sending messages with an arbitrary `update_history_id`.

- **Impact:** Privilege escalation — an agent can tamper with update records belonging to other agents/hosts.
- **Recommendation:** Validate that the `update_history.host_id` corresponds to a host linked to the current `agent_id` before processing the message.

#### D3. `deliver_pending_updates` runs before `register_agent` — race window for lost updates [IMPORTANT]

**Location:** `agent_ws.rs:60-64`

In `handle_agent_authenticated()`, `deliver_pending_updates()` is called at line 60, followed by `register_agent()` at line 64. Between these two calls, if another controller writes an `ExecuteUpdate` outbox event targeting this agent, the `EventPoller` will try `registry.send()` — but the agent isn't registered yet, so the send fails silently. Meanwhile, `deliver_pending_updates` already queried the DB before the new pending update was written. Result: the update message is lost.

- **Impact:** On rare timing, a pending update may not be delivered to a reconnecting agent.
- **Recommendation:** Register the agent in the connection registry *before* delivering pending updates, so any concurrent outbox events are captured by the push channel.

#### D4. `broadcast()` holds `RwLock` read guard across async channel sends [IMPORTANT]

**Location:** `service_connections.rs:125-140`

`broadcast()` and `broadcast_by_type()` acquire the `RwLock` read guard and then iterate all connections, calling `conn.sender.send(msg.clone()).await` for each. Since `mpsc::Sender::send()` is async and waits if the channel (capacity 16) is full, a single slow consumer can hold the read lock for an extended time, blocking all write operations (register, unregister, assign_mqtt_client). In the worst case, if the slow consumer itself needs to call a write method on the registry (e.g., during unregister), this creates a deadlock.

- **Impact:** Under backpressure, all connection management operations are blocked. Potential deadlock if a consumer chain involves registry writes.
- **Recommendation:** Collect senders under the lock, drop the lock, then send. Or use `try_send()` with overflow logging instead of `send().await`.

#### D5. Enrollment client `wait_for_approval` blocks indefinitely [IMPORTANT]

**Location:** `enrollment/src/ws.rs:136-191`

`wait_for_approval()` loops on `ws.next().await` with no timeout. If the controller goes down silently (without sending a WebSocket close frame or TCP RST), or if the administrator never approves the service, the enrollment client blocks forever. Unlike the server-side anonymous handler (which has a similar issue — S2), the client has no recovery mechanism.

- **Impact:** Agent or MQTT service enrollment process hangs indefinitely.
- **Recommendation:** Wrap the approval wait loop in `tokio::time::timeout()`, or use a `tokio::select!` with a timeout branch that returns a descriptive error. The caller already handles retries.

---

## Summary Table

| ID | Category | Severity | Summary | Status |
|----|----------|----------|---------|--------|
| A1 | Architecture | Critical | MQTT password in outbox plaintext | **FIXED** (FP-2) |
| A2 | Architecture | Minor | Duplicate broadcast helpers | Open |
| A3 | Architecture | Minor | Free-form service type string | Open |
| A4 | Architecture | Important | Read-modify-write per output line | **Partially fixed** (FP-5) |
| A5 | Architecture | Important | No max message size on WebSocket | **FIXED** (FP-5) |
| S1 | Security | Critical | Argon2 brute-force DoS via bearer lookup | **FIXED** (FP-1) |
| S2 | Security | Critical | No anonymous connection timeout | **FIXED** (FP-1) |
| S3 | Security | Important | No WS message rate limiting | **Partially fixed** (FP-4) |
| S4 | Security | Important | Unbounded update output growth | **FIXED** (FP-5) |
| S5 | Security | Minor | Silent i128-to-i64 truncation in timestamp | Open |
| S6 | Security | Important | `expect()` on MIN_AGENT_VERSION parse | Open |
| S7 | Security | Minor | Enrollment token in plaintext in payload | Open |
| H1 | HA | Important | Message loss during service migration | Open |
| H2 | HA | Critical | Non-transactional cursor advancement | **FIXED** (FP-3) |
| H3 | HA | Minor | EventPoller startup cursor assumption | Open |
| H4 | HA | Important | 1-hour cleanup TTL too aggressive | Open |
| H5 | HA | Important | MQTT lease TOCTOU gap | Open |
| H6 | HA | Minor | DB polling on every ping | **FIXED** (FP-4) |
| H7 | HA | Minor | Broadcast gap during service migration | Open |
| M1 | Quality | Minor | Hardcoded mpsc buffer size | Open |
| M2 | Quality | Minor | O(n) MQTT client lookup | Open |
| M3 | Quality | Minor | Boilerplate HookShell conversion | Open |
| M4 | Quality | Minor | Tests don't verify outbox writes | Open |
| M5 | Quality | Minor | No explicit lint config | Open |
| D1 | Deep Dive | Important | No connection deduplication on reconnect | Open |
| D2 | Deep Dive | Important | UpdateHistory not validated against agent |
| D3 | Deep Dive | Important | deliver_pending_updates race with register |
| D4 | Deep Dive | Important | broadcast holds RwLock across async sends |
| D5 | Deep Dive | Important | Enrollment client wait_for_approval no timeout |

---

## Fix Plans

| Plan | Addresses | Summary | Status |
|------|-----------|---------|--------|
| FP-1 | S1, S2 | Switch MQTT secrets to SHA-256, add 30s anonymous connection timeout | **DONE** |
| FP-2 | A1 | Strip MQTT credentials from outbox events | **DONE** |
| FP-3 | H2 | Make EventPoller cursor advancement conditional on delivery | **DONE** |
| FP-4 | S3, H6 | Rate-limit approval polling with dedicated interval | **DONE** |
| FP-5 | A5, S4, A4 | Set 1 MB WS message limit, cap update output, optimize appending | **DONE** |
| FP-6 | S6, S5 | Replace `expect()` with `LazyLock`, fix silent timestamp truncation |
| FP-7 | H4, H1 | Configurable event cleanup TTL, startup reconciliation |
| FP-8 | H5 | Atomic lease acquisition with DB-level conflict handling |
| FP-9 | A2, A3 | Remove duplicate broadcast helpers, type-safe service type discriminator |
| FP-10 | M2, M1, M4 | Reverse index for MQTT lookup, named buffer constant, proper tests |
| FP-11 | S7 | Prevent enrollment token and sensitive payloads from leaking into logs |
| FP-12 | H3, H7 | EventPoller startup cursor safety and reconnect state reconciliation |
| FP-13 | M3 | Unify `HookShell` enum across wire and web-api-types crates |
| FP-14 | M5 | Add explicit lint configuration to the wire crate |
| FP-15 | H3 | Add wire protocol version negotiation |
| FP-16 | D1 | Connection deduplication with generation tracking |
| FP-17 | D2 | Validate UpdateHistory ownership against agent |
| FP-18 | D3 | Reorder register_agent before deliver_pending_updates |
| FP-19 | D4 | Non-blocking broadcast with sender snapshot |
| FP-20 | D5 | Add configurable timeout to enrollment client wait_for_approval |

### FP-1. Eliminate argon2 brute-force in `lookup_by_secret` and add anonymous connection timeout — DONE

**Addresses:** S1, S2

**Problem:** `lookup_by_secret()` falls back to iterating all MQTT services with argon2 verification, creating a CPU-exhaustion DoS vector. Additionally, anonymous WebSocket connections can idle indefinitely.

**Implementation:**

1. **Simplified `lookup_by_secret()` to SHA-256 only.** Removed the argon2 brute-force fallback. The function now does a single DB query with SHA-256 hash comparison, same as agents.

2. **Added `ANONYMOUS_TIMEOUT` (30 seconds) to `handle_anonymous()`.** The anonymous message receive loop is wrapped in `tokio::time::timeout()`. On timeout, the connection is closed with a warning log.

**Files modified:**
- `crates/ui/web-api/src/routes/service_ws.rs` — simplified `lookup_by_secret()`, added `ANONYMOUS_TIMEOUT` to `handle_anonymous()`

---

### FP-2. Strip MQTT credentials from outbox events — DONE

**Addresses:** A1

**Problem:** `MqttTenantConfig` includes plaintext `password` (and `username`) fields. When MQTT-related `ControllerMessage` variants are written to the `controller_events` outbox via `NotificationService`, the full serialized JSON — including credentials — is stored in the database.

**Plan:**

1. **Add a `strip_secrets()` method on `ControllerMessage`.**
   - Define a method in the `web-api` crate (not in the wire crate, which should remain a pure data definition crate) that returns a sanitized clone of the message:
     ```rust
     fn strip_outbox_secrets(msg: &ControllerMessage) -> ControllerMessage
     ```
   - For `TenantAssignments`: clone with `password: None` in each `MqttTenantConfig`.
   - For `TenantConfigUpdated`: clone with `password: None` in the nested config.
   - All other variants: return as-is (no secrets).

2. **Call `strip_outbox_secrets()` in `NotificationService::write_outbox_event()`.**
   - Before serializing to JSON, apply the stripping function.
   - The local delivery path (`self.registry.send()`) still sends the original message with the password intact.

3. **Handle the stripped password on the receiving controller.**
   - When `EventPoller` delivers a `TenantAssignments` or `TenantConfigUpdated` message to a locally connected MQTT service, the password will be `None`.
   - The receiving controller must look up the password from the DB before forwarding.
   - Add a `hydrate_secrets()` function in the event poller that enriches the message with the actual password from the `mqtt_clients` table before local delivery.

4. **Alternative (simpler): don't write MQTT tenant messages to the outbox at all.**
   - MQTT tenant assignment/config/revocation messages are only relevant to the specific MQTT service instance that holds the lease.
   - The lease is managed in the DB. When a new MQTT service connects to a different controller, it gets its assignments from the DB directly.
   - Only write non-MQTT messages (approvals, CA updates, cert renewals) to the outbox.
   - This is simpler and avoids the hydration complexity.

**Recommended approach:** Alternative (step 4) — don't write MQTT-specific messages to the outbox. The MQTT service gets its configuration from the controller it's connected to, and lease coordination happens via the DB.

**Files to modify:**
- `crates/ui/web-api/src/notification_service.rs` — skip outbox writes for MQTT-specific messages, or add `strip_outbox_secrets()`
- `crates/ui/web-api/src/event_poller.rs` — if using hydration approach, add DB lookups for passwords
- `crates/ui/web-api/src/routes/settings_mqtt.rs` — if MQTT config changes need cross-controller delivery, use targeted `send()` to the specific service instead of broadcast

**Testing:**
- Unit test: verify outbox JSON does not contain password strings
- Integration test: MQTT config update on Controller A is delivered to MQTT service on Controller B (if using hydration approach)

**Implementation notes:** Used the recommended simpler approach (Alternative step 4). Added `is_mqtt_tenant_message()` helper that matches `TenantAssignments`, `TenantConfigUpdated`, `TenantRevoked`. `NotificationService::send()` and `broadcast()` skip outbox writes for these. `MqttLeaseCoordinator` no longer depends on `NotificationService` — delivers directly via `ServiceConnectionRegistry`. Added comprehensive test `is_mqtt_tenant_message_matches_credential_bearing_variants`.

**Files modified:** `notification_service.rs`, `mqtt_lease_coordinator.rs`, `mqtt_ws.rs`, `settings_mqtt.rs`

---

### FP-3. Make EventPoller cursor advancement delivery-aware — DONE

**Addresses:** H2

**Problem:** The `EventPoller::poll_events()` method advances the cursor past each event regardless of whether `deliver_event()` succeeded. If the local `mpsc` channel is full, the message is silently dropped and the cursor moves past it.

**Plan:**

1. **Make `deliver_event()` return a delivery status.**
   - Change signature: `async fn deliver_event(...) -> bool`
   - Return `true` if the message was delivered to at least one local service (or if no local service was targeted — the event is not for this controller).
   - Return `false` if delivery was attempted but failed (channel full, send error).

2. **Only advance cursor past successfully delivered events.**
   - In `poll_events()`, track two cursors: `new_cursor` (events seen) and `delivered_cursor` (events successfully processed).
   - On delivery failure, stop processing the batch and return `delivered_cursor`.
   - The next poll will retry from `delivered_cursor + 1`.

3. **Handle the "not for this controller" case.**
   - When `target_service_id` is set but the service isn't locally connected, the event should still advance the cursor (it's not for us).
   - When broadcast delivery partially fails (some sends succeed, some fail), consider the event delivered — at-least-once to the successful recipients is acceptable.

4. **Add backoff on repeated delivery failures.**
   - If the same event fails delivery 3+ times, log a warning and advance past it to avoid blocking the entire pipeline.
   - Track retry count in a local `HashMap<i64, u8>`.

**Files to modify:**
- `crates/ui/web-api/src/event_poller.rs` — modify `poll_events()` and `deliver_event()`, add retry tracking
- `crates/ui/web-api/src/service_connections.rs` — ensure `send()` return value is reliable (it already returns `bool`)

**Testing:**
- Unit test: cursor does not advance past failed deliveries
- Unit test: cursor advances past events not targeted at this controller
- Unit test: retry limit prevents permanent blocking

**Implementation notes:** `deliver_event()` and `deliver_mqtt_event()` now return `bool`. Added `retry_counts: HashMap<i64, u8>` field and `MAX_DELIVERY_RETRIES` constant (3). Cursor only advances past successfully delivered events. Deserialization failures permanently advance the cursor. Delivery failures stop batch processing; after 3 retries an event is skipped. Retry entries are cleaned up after cursor advances past them. `run()` takes `mut self`.

**Files modified:** `event_poller.rs`

---

### FP-4. Rate-limit approval polling in enrolled loops — DONE

**Addresses:** S3, H6

**Problem:** Both agent and MQTT enrolled loops poll the `services` table on *every* ping to detect cross-controller approval. With a 15-second ping interval and many pending services, this generates excessive DB queries. A malicious client could also flood pings to amplify the DB load.

**Plan:**

1. **Track last poll time per enrolled connection.**
   - Add a `last_approval_poll: Instant` variable in both `run_agent_enrolled_loop()` and `handle_mqtt_enrolled()`.
   - Only poll the DB if at least 5 seconds have elapsed since the last poll.

2. **Add per-connection message rate limiting.**
   - Track message count and time window per connection.
   - If a service sends more than 20 messages per second, send an `Error` with `BadRequest` code and close the connection.
   - Implement as a simple token-bucket or sliding-window counter (per connection, not shared).

3. **Consider replacing ping-based polling with a dedicated interval.**
   - In the enrolled loops, add a `tokio::time::interval(Duration::from_secs(5))` branch to the `tokio::select!` that polls the DB for approval status.
   - This decouples the polling frequency from client-controlled ping frequency.

**Recommended approach:** Step 1 (throttled poll) + Step 3 (dedicated interval). Step 2 (rate limiting) can be added separately.

**Files to modify:**
- `crates/ui/web-api/src/routes/agent_ws.rs` — add throttled polling in `run_agent_enrolled_loop()`
- `crates/ui/web-api/src/routes/mqtt_ws.rs` — add throttled polling in `handle_mqtt_enrolled()`

**Testing:**
- Unit test: approval poll is skipped if called within 5 seconds of last poll
- Manual test: verify cross-controller approval still works within acceptable latency

**Implementation notes:** Used the recommended approach (steps 1 + 3). Added `APPROVAL_POLL_INTERVAL` (5 seconds) constant and a dedicated `tokio::time::interval` branch in both `run_agent_enrolled_loop()` and `handle_mqtt_enrolled()`. The interval branch uses a conditional guard (`if !approved`) so it stops polling once approved. Ping handlers no longer trigger DB queries — they only respond with pong. Per-connection message rate limiting (step 2) was not implemented and remains a future improvement.

**Files modified:** `agent_ws.rs`, `mqtt_ws.rs`

---

### FP-5. Enforce WebSocket message size limits and cap update output — DONE

**Addresses:** A5, S4, A4

**Problem:** No maximum incoming WebSocket message size is enforced, and the `update_history.output` column grows unboundedly via `UpdateOutput` messages.

**Plan:**

1. **Set WebSocket message size limit.**
   - In the `service_ws()` handler, configure the WebSocket upgrade with a max message size.
   - Axum's `WebSocketUpgrade` supports `.max_frame_size()` and `.max_message_size()`.
   - Set a 1 MB limit for incoming messages. This is generous for any legitimate wire protocol message.
   - The largest legitimate message is `ExecuteUpdate` (with release assets and provider config), which should be well under 100 KB.

2. **Cap `update_history.output` at 1 MB.**
   - In the `UpdateOutput` handler (`agent_ws.rs:327-343`), check the current output length before appending.
   - If `record.output.len() + payload.output.len() > 1_048_576`, skip the append and log a warning.
   - In the `UpdateResult` handler (`agent_ws.rs:345-389`), apply the same cap to the final output.

3. **Optimize output appending (from A4).**
   - Instead of read-modify-write, use SeaORM's `Expr` to do a SQL-level concat:
     ```rust
     UpdateHistory::update_many()
         .filter(update_history::Column::Id.eq(id))
         .col_expr(
             update_history::Column::Output,
             Expr::col(update_history::Column::Output).concat(new_output),
         )
         .exec(db)
     ```
   - This avoids loading the (potentially large) output column into memory on each append.
   - The size check can be done with a `CASE WHEN LENGTH(output) < 1048576 THEN output || $1 ELSE output END` expression.

4. **Document the limits in the asyncapi.yaml.**
   - Add a note about the 1 MB message size limit and the 1 MB output cap.

**Files to modify:**
- `crates/ui/web-api/src/routes/service_ws.rs` — add `max_message_size` to WebSocket upgrade
- `crates/ui/web-api/src/routes/agent_ws.rs` — add output size cap, optimize appending
- `crates/shared/wire/asyncapi.yaml` — document limits

**Testing:**
- Unit test: messages exceeding 1 MB are rejected by WebSocket layer
- Unit test: output exceeding 1 MB is not appended
- Unit test: SQL-level concat produces correct output

**Implementation notes:** Steps 1, 2, and 4 implemented. Step 3 (SQL-level concat) was not feasible: SeaORM's `.concat()` is Postgres-only via `PgExpr` trait, and raw SQL approaches had cross-backend compatibility issues. The read-modify-write pattern remains but is bounded by `MAX_UPDATE_OUTPUT_BYTES` (1 MB). `MAX_WS_MESSAGE_SIZE` (1 MB) set via `ws.max_message_size()` on WebSocket upgrade. `UpdateOutput` handler skips append when output exceeds cap. `UpdateResult` handler caps final output at remaining capacity. Limits documented in `asyncapi.yaml`.

**Files modified:** `service_ws.rs`, `agent_ws.rs`, `asyncapi.yaml`

---

### FP-6. Replace runtime `expect()` and fix silent timestamp truncation

**Addresses:** S6, S5

**Problem:** `MIN_AGENT_VERSION` is parsed with `.expect()` on every `ReportHostInfo` message, which violates the project rule against `panic!`/`unwrap` outside lock guards. The `utc_datetime_millis` serializer silently truncates `i128` to `i64` with `as`, which could mask bugs for extreme timestamp values.

**Plan:**

1. **Parse `MIN_AGENT_VERSION` once at process start using `LazyLock`.**
   - In `agent_ws.rs`, replace the per-message `.expect()` with a module-level static:
     ```rust
     static MIN_AGENT_VER: LazyLock<semver::Version> = LazyLock::new(|| {
         semver::Version::parse(MIN_AGENT_VERSION)
             .expect("MIN_AGENT_VERSION must be valid semver")
     });
     ```
   - `LazyLock` initialization panics are acceptable because they happen exactly once at first access (effectively process startup), not on each request. This is analogous to the approved `Mutex::lock().unwrap()` pattern — startup validation, not runtime fallibility.
   - Reference `&*MIN_AGENT_VER` in the handler instead of parsing each time.

2. **Replace `as i64` with `i64::try_from()` in `utc_datetime_millis::serialize`.**
   - In `wire/src/lib.rs:253-254`, change:
     ```rust
     // Before
     serializer.serialize_i64(millis as i64)
     // After
     let millis_i64 = i64::try_from(millis).map_err(serde::ser::Error::custom)?;
     serializer.serialize_i64(millis_i64)
     ```
   - This turns a silent data corruption into an explicit serialization error.

3. **Add a test for the timestamp boundary.**
   - Add a test that verifies serialization round-trips for timestamps near the `i64` boundary (both positive and negative extremes within practical range).

**Files to modify:**
- `crates/ui/web-api/src/routes/agent_ws.rs` — `LazyLock` for `MIN_AGENT_VERSION`
- `crates/shared/wire/src/lib.rs` — `i64::try_from()` in `utc_datetime_millis::serialize`

**Testing:**
- Existing tests continue to pass (no behavioral change for valid timestamps)
- New test: extreme timestamp values produce an error instead of silent truncation
- Verify `MIN_AGENT_VER` is accessed at least once in test suite (lazy init)

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
   - In `CODEREVIEW.md` or `ARCHITECTURE.md`, document which message types are reconciled on reconnect (safe to miss) vs. which are fire-and-forget (may be lost if missed).

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
   - Add a second `HashMap<Uuid, Uuid>` to `ServiceConnectionRegistry` mapping `mqtt_client_id -> service_id`:
     ```rust
     struct Inner {
         connections: HashMap<Uuid, ServiceConnection>,
         mqtt_client_index: HashMap<Uuid, Uuid>,
     }
     ```
   - Update `assign_mqtt_client()` to insert into the index.
   - Update `release_mqtt_client()` to remove from the index.
   - Update `unregister()` to remove all entries for the disconnecting service.
   - Change `get_instance_for_mqtt_client()` to a single `HashMap::get()` — O(1).

2. **Make the `mpsc` channel buffer size a named constant with documentation.**
   - Define `const PUSH_CHANNEL_CAPACITY: usize = 32;` at the top of `service_connections.rs`.
   - Add a doc comment explaining the rationale: the buffer must be large enough to absorb bursts of push messages (e.g., multiple tenant assignments sent in quick succession) without blocking the sender, but small enough to detect a stalled consumer.
   - Increase from 16 to 32 to provide more headroom for cross-controller event delivery bursts.

3. **Add proper integration tests for `NotificationService`.**
   - Create a test helper that sets up an in-memory SQLite DB with migrations applied.
   - Test `send()`: verify an event row appears in `controller_events` with correct `source_controller_id`, `target_service_id`, and parseable `message_json`.
   - Test `broadcast()`: verify an event row appears with `target_service_id = NULL`.
   - Test that `ServerRestarting` is NOT sent through `NotificationService` (document the design intent).

4. **Add tests for `EventPoller` delivery routing.**
   - Test targeted delivery: event with `target_service_id` is sent only to that service.
   - Test type-filtered delivery: event with `target_service_type = "mqtt"` is sent only to MQTT services.
   - Test broadcast: event with both fields NULL is sent to all services.
   - Use a mock `ServiceConnectionRegistry` or in-memory channels to verify delivery.

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

**Problem:** `EnrollPayload.enrollment_token` contains the raw pre-shared secret. `EnrolledPayload.enrollment_secret` contains the newly generated enrollment secret. Both are transmitted as plaintext JSON fields over the (TLS-encrypted) WebSocket. If any middleware, debug logging, or error handler captures the raw message text, these secrets are exposed. The `Debug` derive on all payload structs means `tracing::debug!("{:?}", payload)` would print them too.

**Plan:**

1. **Implement a custom `Debug` for sensitive payload structs.**
   - For `EnrollPayload`, override `Debug` to redact `enrollment_token`:
     ```rust
     impl fmt::Debug for EnrollPayload {
         fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
             f.debug_struct("EnrollPayload")
                 .field("hostname", &self.hostname)
                 .field("friendly_name", &self.friendly_name)
                 .field("enrollment_token", &self.enrollment_token.as_ref().map(|_| "[REDACTED]"))
                 .field("service_type", &self.service_type)
                 .field("host_info", &self.host_info)
                 .finish()
         }
     }
     ```
   - For `EnrolledPayload`, redact `enrollment_secret`:
     ```rust
     impl fmt::Debug for EnrolledPayload {
         fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
             f.debug_struct("EnrolledPayload")
                 .field("service_id", &self.service_id)
                 .field("enrollment_secret", &"[REDACTED]")
                 .field("status", &self.status)
                 .finish()
         }
     }
     ```
   - For `MqttTenantConfig`, redact `password`:
     ```rust
     impl fmt::Debug for MqttTenantConfig {
         fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
             f.debug_struct("MqttTenantConfig")
                 // ... all other fields ...
                 .field("password", &self.password.as_ref().map(|_| "[REDACTED]"))
                 .finish()
         }
     }
     ```

2. **Remove the derived `Debug` from affected structs.**
   - Replace `#[derive(Debug, ...)]` with a manual `Debug` impl for `EnrollPayload`, `EnrolledPayload`, and `MqttTenantConfig`.
   - Keep the derived `Debug` for all other (non-sensitive) payload structs.

3. **Audit all `tracing::*!` calls in WebSocket handlers for raw message logging.**
   - Search for any `tracing::debug!` or `tracing::trace!` calls that log raw WebSocket text (the `text` variable in the match arms).
   - Ensure none of the handler code logs the raw JSON of enrollment or MQTT tenant messages.
   - The `deserialize_service_msg` error path logs the error string (from serde) which may include partial payload content — verify this doesn't include secrets.

4. **Add a lint comment in the wire crate.**
   - Add a module-level doc comment warning that message text must never be logged verbatim because it may contain secrets.

**Files to modify:**
- `crates/shared/wire/src/lib.rs` — custom `Debug` impls for `EnrollPayload`, `EnrolledPayload`, `MqttTenantConfig`
- `crates/ui/web-api/src/routes/service_ws.rs` — audit log statements
- `crates/ui/web-api/src/routes/agent_ws.rs` — audit log statements
- `crates/ui/web-api/src/routes/mqtt_ws.rs` — audit log statements
- `crates/shared/enrollment/src/ws.rs` — audit log statements

**Testing:**
- Unit test: `format!("{:?}", enroll_payload)` does not contain the token string
- Unit test: `format!("{:?}", enrolled_payload)` does not contain the secret string
- Unit test: `format!("{:?}", mqtt_tenant_config)` does not contain the password string
- Verify `PartialEq` and `Serialize`/`Deserialize` still work (they don't use `Debug`)

---

### FP-12. EventPoller startup cursor safety and reconnect state reconciliation

**Addresses:** H3, H7

**Problem:** `EventPoller::fetch_max_id()` initializes the cursor to the current max event ID at startup. While events written *after* this point are correctly caught, the initialization creates a race window: if the poller starts during a burst of cross-controller writes, the max ID it reads may not reflect all committed events due to transaction isolation (a concurrent INSERT with a higher auto-increment ID may commit before a lower one). Additionally, when services migrate between controllers during a broadcast window, they may miss messages because the broadcast was delivered to the old controller.

**Plan:**

1. **Use a safety margin on the startup cursor.**
   - Instead of `fetch_max_id()`, initialize the cursor to `max(0, max_id - STARTUP_CURSOR_MARGIN)` where `STARTUP_CURSOR_MARGIN = 100`.
   - This causes the poller to re-process the last 100 events on startup. Since delivery is already idempotent at the WebSocket level (services handle duplicate messages gracefully or the message targets a service not connected to this controller), the only cost is a few extra no-op deliveries.
   - Add a log message: `"event poller starting with safety margin, cursor={cursor}, max_id={max_id}"`.

2. **Filter out self-sourced events in `poll_events`.**
   - The existing `source_controller_id != self.controller_id` filter already prevents self-delivery. With the safety margin, the poller may see old events from other controllers that were already processed before restart. This is harmless (they target services that aren't locally connected).

3. **Push authoritative state on service reconnect (reconnect reconciliation).**
   - When a service connects (authenticated path in `service_ws.rs`), the controller already sends `ServiceSettings` (with `ca_bundle_hash`). Extend this to cover the gap identified in H7:
     - **Agents:** Already reconciled — `ServiceSettings` on connect, plus pending updates are delivered.
     - **MQTT services:** Already reconciled — `TenantAssignments` sent after `Register`.
   - Add a comment in `handle_authenticated()` documenting which state is pushed on reconnect and why this makes the system resilient to missed broadcast events.

4. **Add a `last_connected_at` timestamp to service connections.**
   - When a service registers in `ServiceConnectionRegistry`, record `Instant::now()`.
   - When delivering an outbox event via `EventPoller`, skip events whose `created_at` is older than the service's `last_connected_at` — the service already received the authoritative state on connect.
   - This prevents stale outbox events from being delivered to recently-reconnected services.

**Files to modify:**
- `crates/ui/web-api/src/event_poller.rs` — startup safety margin, age-based skip
- `crates/ui/web-api/src/service_connections.rs` — `last_connected_at` field
- `crates/ui/web-api/src/routes/service_ws.rs` — document reconnect reconciliation

**Testing:**
- Unit test: startup cursor is `max_id - 100` (not `max_id`)
- Unit test: events older than a service's connection time are skipped
- Unit test: empty table produces cursor 0 (margin doesn't go negative)

---

### FP-13. Unify `HookShell` enum across wire and web-api-types crates

**Addresses:** M3

**Problem:** Two identical `HookShell` enums exist — one in `uptrakit-internal-wire` (`wire/src/lib.rs:67-78`) and one in `uptrakit-web-api-types` (`web-api-types/src/update_hooks.rs`). The `wire_hook_shell()` conversion function in `agent_ws.rs:823-837` is pure boilerplate mapping between them. Adding a new shell variant requires updating both enums and the conversion function — a maintenance burden and divergence risk.

**Plan:**

1. **Move the canonical `HookShell` to the wire crate (it's already there).**
   - The wire crate's `HookShell` is the protocol-level definition. It should be the single source of truth.
   - The wire crate already has the correct serde attributes (`rename_all = "snake_case"`, `Default`).

2. **Re-export from `web-api-types` instead of defining a duplicate.**
   - Add `uptrakit-internal-wire` as a dependency of `uptrakit-web-api-types` (if not already).
   - In `web-api-types/src/update_hooks.rs`, replace the local `HookShell` definition with:
     ```rust
     pub use uptrakit_internal_wire::HookShell;
     ```
   - If `web-api-types` already depends on the wire crate: straightforward re-export.
   - If not: check whether adding this dependency is acceptable. The wire crate is a lightweight data-definition crate with minimal dependencies (`serde`, `serde_json`, `time`, `uuid`, `uptrakit-provider-core`), so the coupling is acceptable.

3. **Remove the `wire_hook_shell()` conversion function.**
   - Delete `wire_hook_shell()` from `agent_ws.rs:823-837`.
   - All call sites now use the same `HookShell` type directly — no conversion needed.

4. **Check for other duplicate enums between wire and web-api-types.**
   - Audit whether `MqttTransport` has a similar duplication. If so, apply the same re-export pattern.
   - Document the policy: types used in both the wire protocol and the HTTP API should be defined in the wire crate and re-exported from web-api-types.

**Files to modify:**
- `crates/shared/web-api-types/Cargo.toml` — add wire crate dependency (if missing)
- `crates/shared/web-api-types/src/update_hooks.rs` — replace local `HookShell` with re-export
- `crates/ui/web-api/src/routes/agent_ws.rs` — delete `wire_hook_shell()`, use `HookShell` directly
- All call sites of `wire_hook_shell()` — use the type directly

**Testing:**
- Compile-time: if the re-export works, all existing code compiles without the conversion function
- Existing tests continue to pass (the type is identical, just sourced from one place)
- Verify serde serialization is unchanged (same `rename_all` attributes)

---

### FP-14. Add explicit lint configuration to the wire crate

**Addresses:** M5

**Problem:** The wire crate's `Cargo.toml` has no `[lints]` section. While workspace-level clippy catches issues during CI, the crate itself doesn't declare its expectations. A contributor working on the wire crate in isolation (e.g., `cargo clippy -p uptrakit-internal-wire`) may miss lints that the workspace enforces. Additionally, the wire crate is a protocol-definition crate where correctness is paramount — it should have stricter-than-default lints.

**Plan:**

1. **Add a `[lints]` section to the wire crate's `Cargo.toml`.**
   - If the workspace already has a `[workspace.lints]` section, reference it:
     ```toml
     [lints]
     workspace = true
     ```
   - If not, add crate-local lints:
     ```toml
     [lints.rust]
     unsafe_code = "forbid"
     missing_docs = "warn"

     [lints.clippy]
     all = { level = "deny", priority = -1 }
     pedantic = { level = "warn", priority = -1 }
     unwrap_used = "deny"
     expect_used = "deny"
     panic = "deny"
     ```

2. **Check if the workspace already configures lints.**
   - Read the root `Cargo.toml` for `[workspace.lints]`.
   - If present, just add `[lints] workspace = true` to the wire crate.
   - If not, consider adding workspace-level lints and having all crates inherit them (broader scope, but out of this plan's scope — just add to wire crate for now).

3. **Fix any new warnings surfaced by the stricter lints.**
   - `missing_docs`: add doc comments to any undocumented public items.
   - `clippy::pedantic`: address any new pedantic warnings (likely few, the code is already clean).

4. **Add `#![doc = include_str!("../README.md")]` or a module-level doc comment.**
   - Ensures `cargo doc` produces useful documentation for the wire crate.
   - Add a brief crate-level doc: `//! Uptrakit service-controller wire protocol message definitions.`

**Files to modify:**
- `crates/shared/wire/Cargo.toml` — add `[lints]` section
- `crates/shared/wire/src/lib.rs` — add crate-level doc comment, fix any new warnings
- Root `Cargo.toml` — check for existing `[workspace.lints]`

**Testing:**
- `cargo clippy -p uptrakit-internal-wire -- -D warnings` passes with no warnings
- `cargo doc -p uptrakit-internal-wire --no-deps` builds cleanly
- CI continues to pass (no regressions)

---

### FP-15. Add wire protocol version negotiation

**Addresses:** H3 (related), forward-looking architectural improvement

**Problem:** The wire protocol has no version negotiation mechanism. Currently, the controller and services must run exactly compatible protocol versions. If a controller is upgraded before its agents (rolling deployment), the controller may send message types that old agents don't understand, or vice versa. The `asyncapi.yaml` tracks a version number (currently `0.0.8`) but it's documentation-only — it's not exchanged on the wire. The backward-compatibility serde defaults mitigate this partially, but unknown message `type` values cause deserialization failures (hard errors, not graceful degradation).

**Plan:**

1. **Add a `protocol_version` field to `ReportHostInfoPayload` and `MqttRegisterPayload`.**
   - These are the first messages sent by authenticated services after connecting:
     ```rust
     pub struct ReportHostInfoPayload {
         pub host_info: HostInfo,
         pub agent_version: String,
         /// Wire protocol version supported by this agent (e.g., "0.0.8").
         #[serde(default)]
         pub protocol_version: Option<String>,
     }

     pub struct MqttRegisterPayload {
         pub instance_id: String,
         #[serde(default)]
         pub max_tenants: u32,
         #[serde(default, skip_serializing_if = "Vec::is_empty")]
         pub active_mqtt_clients: Vec<Uuid>,
         /// Wire protocol version supported by this MQTT service.
         #[serde(default)]
         pub protocol_version: Option<String>,
     }
     ```
   - Old agents that don't send this field will deserialize with `None` (backward-compatible via `#[serde(default)]`).

2. **Add `protocol_version` to `ServiceSettingsPayload`.**
   - The controller's first response to authenticated services:
     ```rust
     pub struct ServiceSettingsPayload {
         pub renewal_window_hours: u16,
         #[serde(default)]
         pub ca_bundle_hash: String,
         #[serde(default, skip_serializing_if = "Option::is_none")]
         pub shutdown_timeout_seconds: Option<u32>,
         /// Wire protocol version used by this controller.
         #[serde(default)]
         pub protocol_version: Option<String>,
     }
     ```

3. **Define a `PROTOCOL_VERSION` constant in the wire crate.**
   ```rust
   /// Current wire protocol version, matching asyncapi.yaml.
   pub const PROTOCOL_VERSION: &str = "0.0.8";
   ```
   - Both controller and service code reference this constant when populating the field.

4. **Log version mismatches on the controller side.**
   - When the controller receives `ReportHostInfo` or `Register` with a `protocol_version`:
     - If it matches `PROTOCOL_VERSION`: no action.
     - If it's older: log an info message suggesting the service should be upgraded.
     - If it's newer: log a warning (the controller may not understand messages from a newer service).
   - Do NOT reject connections based on protocol version (graceful degradation, not hard enforcement).

5. **Add `#[serde(other)]` fallback for unknown message types (future).**
   - This is a larger change and can be done in a follow-up. Currently, unknown `type` values cause a deserialization error. A graceful approach would be:
     ```rust
     #[serde(other)]
     Unknown,
     ```
   - Note: `serde(other)` only works with *untagged* or *internally-tagged* enums on unit variants. For `#[serde(tag = "type")]` with payloads, this requires a custom deserializer. Document this as a follow-up item.

6. **Update `asyncapi.yaml`.**
   - Add the `protocol_version` field to the relevant message schemas.
   - Add a section documenting the version negotiation mechanism.

**Files to modify:**
- `crates/shared/wire/src/lib.rs` — `PROTOCOL_VERSION` constant, add `protocol_version` field to 3 payloads
- `crates/shared/wire/asyncapi.yaml` — document new fields and versioning
- `crates/ui/web-api/src/routes/agent_ws.rs` — log protocol version on `ReportHostInfo`
- `crates/ui/web-api/src/routes/mqtt_ws.rs` — log protocol version on `Register`
- `crates/ui/web-api/src/routes/service_ws.rs` — include `PROTOCOL_VERSION` in `ServiceSettings`
- `crates/core/agent/src/main.rs` (or host_info module) — set `protocol_version` in `ReportHostInfo`
- `crates/core/mqtt/src/controller_client.rs` — set `protocol_version` in `Register`

**Testing:**
- Unit test: serde round-trip with `protocol_version` set
- Unit test: serde backward compat — missing `protocol_version` deserializes as `None`
- Unit test: `PROTOCOL_VERSION` constant matches the version in `asyncapi.yaml` (parse the YAML in a test)

---

### FP-16. Connection deduplication with generation tracking

**Addresses:** D1

**Problem:** When a service reconnects (e.g., after a network disruption), `register_agent()` / `register_mqtt()` silently replaces the `HashMap` entry, dropping the old sender. The old WebSocket handler loop continues running until it discovers the broken push channel, which may not happen immediately if it's blocked in `stream.next().await`. During the overlap, both old and new handler loops can process incoming messages and write to the DB.

**Plan:**

1. **Add a connection generation counter to `ServiceConnectionRegistry`.**
   - Add a monotonically increasing `u64` generation to each `ServiceConnection`:
     ```rust
     struct ServiceConnection {
         sender: mpsc::Sender<ControllerMessage>,
         generation: u64,
         // ...existing fields...
     }
     ```
   - Maintain a global generation counter: `next_generation: u64` in the inner state.
   - On each `register_agent()` / `register_mqtt()`, increment the counter and assign it to the new connection.
   - Return the `generation` alongside the `mpsc::Receiver` from the register methods.

2. **Return the previous sender on replacement.**
   - When `HashMap::insert()` returns `Some(old_conn)`, explicitly drop the old sender. This immediately unblocks `push_rx.recv()` in the old handler loop (returns `None`), causing it to exit the `tokio::select!` and terminate.
   - This is already the default behavior of `HashMap::insert`, but we should log when a replacement occurs:
     ```rust
     if let Some(old) = self.connections.insert(service_id, conn) {
         tracing::warn!(%service_id, old_gen = old.generation, new_gen = generation, "replacing existing connection");
         // old.sender is dropped here → old push_rx returns None
     }
     ```

3. **Add a `CancellationToken` for immediate teardown.**
   - Instead of relying on the push channel drop (which only triggers when the old loop polls push_rx), associate a `CancellationToken` with each connection.
   - When replacing a connection, cancel the old token.
   - The handler loops add `token.cancelled()` as a branch in their `tokio::select!`:
     ```rust
     tokio::select! {
         msg = stream.next() => { ... }
         push = push_rx.recv() => { ... }
         _ = cancel_token.cancelled() => {
             tracing::debug!("connection superseded by newer connection");
             break;
         }
     }
     ```
   - This ensures immediate termination even if the old loop is blocked on `stream.next()`.

4. **Update register methods to return `(Receiver, CancellationToken)`.**
   - `register_agent()` and `register_mqtt()` return `(mpsc::Receiver<ControllerMessage>, CancellationToken)`.
   - All handler call sites pass the token into the `tokio::select!` loop.

**Files to modify:**
- `crates/ui/web-api/src/service_connections.rs` — add generation, `CancellationToken`, log on replacement
- `crates/ui/web-api/src/routes/agent_ws.rs` — accept and use `CancellationToken`
- `crates/ui/web-api/src/routes/mqtt_ws.rs` — accept and use `CancellationToken`
- `crates/ui/web-api/src/routes/service_ws.rs` — pass token through

**Testing:**
- Unit test: registering the same service_id twice drops the first sender (first receiver returns `None`)
- Unit test: cancellation token from old connection is cancelled on re-register
- Unit test: generation counter increments correctly

---

### FP-17. Validate UpdateHistory ownership against the requesting agent

**Addresses:** D2

**Problem:** When processing `UpdateStarted`, `UpdateOutput`, and `UpdateResult` messages from an authenticated agent, the handler looks up the `update_history` record by `payload.update_history_id` but never verifies that the record belongs to a host linked to the current `agent_id`. A compromised or misbehaving agent could manipulate any update record in the database.

**Plan:**

1. **Add an ownership validation query.**
   - Before processing any update message, verify the ownership chain:
     ```
     update_history.host_id → service_host.host_id WHERE service_host.service_id = agent_id
     ```
   - Create a helper function:
     ```rust
     async fn validate_update_ownership(
         db: &DatabaseConnection,
         agent_id: Uuid,
         update_history_id: Uuid,
     ) -> Result<update_history::Model, AgentWsError>
     ```
   - This function:
     1. Fetches the `update_history` record.
     2. Checks that the `host_id` exists in `service_host` for this `agent_id`.
     3. Returns the record if valid, or an error if not.

2. **Apply validation to all three update message handlers.**
   - `UpdateStarted`: validate before setting status to `InProgress`.
   - `UpdateOutput`: validate before appending output.
   - `UpdateResult`: validate before setting final status.

3. **Log and reject unauthorized attempts.**
   - If validation fails, log at `warn!` level with the agent_id, update_history_id, and expected host_id.
   - Send an `Error(Forbidden)` message and `continue` (don't break the connection — it might be a legitimate race condition with host unlinking).

4. **Cache the host_id set per connection.**
   - To avoid repeated DB queries, cache the set of `host_id`s linked to this agent at connection start (already done for `deliver_pending_updates`).
   - Refresh the cache on `ReportHostInfo` (which may link new hosts).
   - Use the cached set for ownership validation.

**Files to modify:**
- `crates/ui/web-api/src/routes/agent_ws.rs` — add `validate_update_ownership()`, apply to 3 handlers, cache host IDs

**Testing:**
- Unit test: agent can update records for its own hosts
- Unit test: agent is rejected when trying to update records for other hosts
- Unit test: cached host_id set is refreshed after `ReportHostInfo`

---

### FP-18. Reorder register_agent before deliver_pending_updates

**Addresses:** D3

**Problem:** In `handle_agent_authenticated()`, `deliver_pending_updates()` is called before `register_agent()`. During the gap between these two calls, outbox events from other controllers targeting this agent will fail delivery (agent not registered), and the updates may have been written to the DB after `deliver_pending_updates` queried for pending records. The update is lost.

**Plan:**

1. **Swap the order: register first, deliver second.**
   - In `handle_agent_authenticated()`, move `register_agent()` before `deliver_pending_updates()`:
     ```rust
     // Register first so outbox events can reach us
     let mut push_rx = state.service_connections.register_agent(agent_id).await;

     // Deliver pending updates (any concurrent outbox events go to push_rx)
     if let Err(e) = deliver_pending_updates(state, agent_id, sink, out_seq).await {
         tracing::error!(...);
     }
     ```

2. **Handle potential duplicate deliveries.**
   - With this ordering, it's possible that an `ExecuteUpdate` arrives via both `deliver_pending_updates` (from DB query) AND `push_rx` (from an outbox event that fired between registration and the DB query).
   - The agent already handles duplicate `ExecuteUpdate` messages gracefully — if it receives an `ExecuteUpdate` for an update that's already `InProgress`, it ignores it.
   - Document this idempotency guarantee in the handler.

3. **Ensure `deliver_pending_updates` sends via the `sink` (not via `push_rx`).**
   - Currently `deliver_pending_updates` sends directly to the WebSocket `sink`, not through the `mpsc` channel. This means there's no ordering conflict with push messages.
   - However, if an `ExecuteUpdate` arrives via `push_rx` while `deliver_pending_updates` is sending to `sink`, the push message will be buffered in the channel (not lost).
   - The main loop's `tokio::select!` will drain both sources.

4. **Ensure unregister still runs on any exit path.**
   - The `unregister` call at the end of the function must still execute. Since `push_rx` is now created earlier, ensure all early-return paths also unregister.
   - Alternatively, use a `Drop` guard or `defer!` pattern.

**Files to modify:**
- `crates/ui/web-api/src/routes/agent_ws.rs` — swap registration order in `handle_agent_authenticated()`

**Testing:**
- Integration test: agent connects, pending update is delivered + outbox event arrives simultaneously → agent receives the update at least once
- Verify no functional regression in agent reconnect flow

---

### FP-19. Non-blocking broadcast with sender snapshot

**Addresses:** D4

**Problem:** `broadcast()` and `broadcast_by_type()` hold the `RwLock` read guard while iterating connections and calling `sender.send(msg).await`. If any connection's `mpsc` channel is full (consumer is slow), the entire broadcast blocks, holding the read lock. This prevents all write operations (register, unregister, assign_mqtt_client) on the registry during the stall. In pathological cases, if the slow consumer's handler needs to call a write method on the registry, a deadlock occurs.

**Plan:**

1. **Snapshot senders, then release lock before sending.**
   - Change `broadcast()` to:
     ```rust
     pub async fn broadcast(&self, msg: ControllerMessage) {
         let senders: Vec<mpsc::Sender<ControllerMessage>> = {
             let guard = self.inner.read().await;
             guard.values().map(|c| c.sender.clone()).collect()
         };
         // Lock is released here
         for sender in senders {
             let _ = sender.send(msg.clone()).await;
         }
     }
     ```
   - Cloning `mpsc::Sender` is cheap (it's an `Arc` increment).
   - The lock is held only for the duration of the snapshot, not during sends.

2. **Apply the same pattern to `broadcast_by_type()`.**
   - Same approach: snapshot filtered senders, drop lock, then send.

3. **Consider `try_send()` for fire-and-forget broadcasts.**
   - For broadcasts where message loss is acceptable (like `CaBundleUpdated` which is reconciled on reconnect), use `try_send()` instead of `send().await`:
     ```rust
     for sender in senders {
         if sender.try_send(msg.clone()).is_err() {
             tracing::warn!("broadcast channel full, message dropped");
         }
     }
     ```
   - This makes broadcast non-blocking even if channels are full.
   - For targeted `send()` calls (to a specific service), keep the blocking `send().await` since those messages are critical.

4. **Apply the snapshot pattern to the existing `send()` method too.**
   - Currently `send()` holds the read lock while sending:
     ```rust
     let guard = self.inner.read().await;
     if let Some(conn) = guard.get(service_id) {
         conn.sender.send(msg).await.is_ok()
     }
     ```
   - Change to:
     ```rust
     let sender = {
         let guard = self.inner.read().await;
         guard.get(service_id).map(|c| c.sender.clone())
     };
     if let Some(sender) = sender {
         sender.send(msg).await.is_ok()
     } else {
         false
     }
     ```

5. **Document the lock-ordering guarantees.**
   - Add a doc comment to `ServiceConnectionRegistry` explaining that methods never hold the lock across async suspension points, preventing deadlock.

**Files to modify:**
- `crates/ui/web-api/src/service_connections.rs` — refactor `broadcast()`, `broadcast_by_type()`, and `send()` to use sender snapshots

**Testing:**
- Unit test: broadcast completes even when one channel is full (with `try_send` variant)
- Unit test: write operations (register/unregister) are not blocked during broadcast
- Unit test: broadcast to a service that disconnects mid-broadcast doesn't panic

---

### FP-20. Add configurable timeout to enrollment client `wait_for_approval`

**Addresses:** D5

**Problem:** `wait_for_approval()` in `enrollment/src/ws.rs` loops on `ws.next().await` with no timeout. If the controller goes down silently or the administrator never approves, the enrollment client blocks forever. The server-side `handle_anonymous()` has the same issue (S2), but the client side is arguably worse — it blocks the entire agent/MQTT service startup.

**Plan:**

1. **Add a `timeout` parameter to `wait_for_approval()`.**
   - Change the signature to accept an optional timeout:
     ```rust
     pub async fn wait_for_approval(
         ws: &mut WsStream,
         in_seq: &mut IncomingSeq,
         timeout: Duration,
     ) -> Result<()>
     ```
   - Wrap the approval loop in `tokio::time::timeout()`:
     ```rust
     tokio::time::timeout(timeout, async {
         loop {
             let msg = ws.next().await...;
             // existing logic
         }
     })
     .await
     .map_err(|_| report!(EnrollmentError::ApprovalTimeout))?
     ```

2. **Add `ApprovalTimeout` to `EnrollmentError`.**
   - ```rust
     #[error("timed out waiting for approval after {0:?}")]
     ApprovalTimeout(Duration),
     ```
   - Or keep it simple:
     ```rust
     #[error("timed out waiting for approval")]
     ApprovalTimeout,
     ```

3. **Set reasonable defaults.**
   - For `run_enrollment()`: use a configurable timeout, default 30 minutes.
   - For `resume_enrollment()`: use the same timeout.
   - The caller (agent main loop) already has retry logic — on timeout, it will reconnect and resume enrollment.

4. **Apply the same pattern to `send_enroll()` and `request_certificate_ws()`.**
   - These also loop on `ws.next().await` without timeout.
   - Add a shorter timeout (e.g., 60 seconds) since these expect immediate responses.
   - `send_enroll` timeout: 60s (controller should respond quickly).
   - `request_certificate_ws` timeout: 60s (CSR signing should be fast).

5. **Add timeout to `connect_ws()` TCP connection.**
   - `tokio::net::TcpStream::connect()` uses the OS default timeout (can be minutes).
   - Wrap in `tokio::time::timeout(Duration::from_secs(30), TcpStream::connect(...))`.

**Files to modify:**
- `crates/shared/enrollment/src/ws.rs` — add timeout to `wait_for_approval()`, `send_enroll()`, `request_certificate_ws()`, `connect_ws()`
- `crates/shared/enrollment/src/error.rs` — add `ApprovalTimeout` variant
- `crates/core/agent/src/main.rs` — pass timeout when calling enrollment functions
- `crates/core/mqtt/src/controller_client.rs` — pass timeout when calling enrollment functions

**Testing:**
- Unit test: `wait_for_approval` returns `ApprovalTimeout` after the specified duration
- Unit test: `send_enroll` returns error on timeout
- Unit test: `connect_ws` returns error on connection timeout
- Verify existing enrollment flow works with generous timeout values
