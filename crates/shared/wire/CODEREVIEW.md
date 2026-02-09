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

#### H3. EventPoller startup cursor initialization

**Location:** `event_poller.rs:38`

`fetch_max_id()` initializes the cursor to the current max event ID. Events written between this call and the first `poll_events()` tick (up to 1 second later) by another controller will be caught correctly (polled with `id > last_seen_id`). This is correct behavior but worth noting as a design assumption.

---

## 4. Minor / Code Quality

#### M5. No explicit lint configuration in the wire crate

**Location:** `wire/Cargo.toml`

The wire crate's `Cargo.toml` doesn't configure lints. Relying on workspace-level clippy is fine, but making it explicit prevents accidental regressions.

---

## Summary Table

| ID | Category | Severity | Summary | Status |
|----|----------|----------|---------|--------|
| A4 | Architecture | Important | Read-modify-write per output line | **Partially fixed** |
| S3 | Security | Important | No WS message rate limiting | **Partially fixed** |
| H3 | HA | Minor | EventPoller startup cursor assumption | Open |
| M5 | Quality | Minor | No explicit lint config | Open |

---

## Fix Plans

| Plan | Addresses | Summary | Status |
|------|-----------|---------|--------|
| FP-14 | M5 | Add explicit lint configuration to the wire crate | |
| FP-15 | H3 | Add wire protocol version negotiation | |

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
