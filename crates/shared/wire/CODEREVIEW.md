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

#### M4. `notification_service.rs` tests don't verify outbox writes

**Location:** `notification_service.rs:104-143`

The tests use an in-memory SQLite DB without running migrations, so the outbox INSERT silently fails. The tests only verify the code path doesn't panic, not that it works correctly.

- **Recommendation:** Either run migrations in the test DB or add integration tests that verify outbox contents.

#### M5. No explicit lint configuration in the wire crate

**Location:** `wire/Cargo.toml`

The wire crate's `Cargo.toml` doesn't configure lints. Relying on workspace-level clippy is fine, but making it explicit prevents accidental regressions.

---

## Summary Table

| ID | Category | Severity | Summary | Status |
|----|----------|----------|---------|--------|
| A4 | Architecture | Important | Read-modify-write per output line | **Partially fixed** |
| S3 | Security | Important | No WS message rate limiting | **Partially fixed** |
| H1 | HA | Important | Message loss during service migration | Open |
| H3 | HA | Minor | EventPoller startup cursor assumption | Open |
| H4 | HA | Important | 1-hour cleanup TTL too aggressive | Open |
| H5 | HA | Important | MQTT lease TOCTOU gap | Open |
| H7 | HA | Minor | Broadcast gap during service migration | Open |
| M4 | Quality | Minor | Tests don't verify outbox writes | Open |
| M5 | Quality | Minor | No explicit lint config | Open |

---

## Fix Plans

| Plan | Addresses | Summary | Status |
|------|-----------|---------|--------|
| FP-7 | H4, H1 | Configurable event cleanup TTL, startup reconciliation | |
| FP-8 | H5 | Atomic lease acquisition with DB-level conflict handling | |
| FP-12 | H3, H7 | EventPoller startup cursor safety and reconnect state reconciliation | |
| FP-14 | M5 | Add explicit lint configuration to the wire crate | |
| FP-15 | H3 | Add wire protocol version negotiation | |

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
