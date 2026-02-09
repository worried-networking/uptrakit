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

## 4. Minor / Code Quality

## Summary Table

| ID | Category | Severity | Summary | Status |
|----|----------|----------|---------|--------|
| A4 | Architecture | Important | Read-modify-write per output line | **Partially fixed** |
| S3 | Security | Important | No WS message rate limiting | **Partially fixed** |
