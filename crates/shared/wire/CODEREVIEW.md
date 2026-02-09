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

---

## 2. Security & Safety

---

## 3. High Availability (Multi-Controller)

## 4. Minor / Code Quality

## Summary Table

| ID | Category | Severity | Summary | Status |
|----|----------|----------|---------|--------|
