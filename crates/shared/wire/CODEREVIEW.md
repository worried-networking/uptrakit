# Code Review: `uptrakit-internal-wire`

- Review date: 2026-03-17
- Scope: current-state review (full 14-dimension)

## Summary

The service/controller wire crate is stable and well-structured. Forward-compatibility patterns
(`#[serde(other)]` on message enums, `Other(String)` on value enums) are consistently applied.
The `is_nats_publishable` gate is comprehensive and covers all credential-bearing and
session-targeted variants. No active findings were confirmed in this review pass.

## Strengths

- `ServiceMessage` and `ControllerMessage` both carry `#[non_exhaustive]` and `#[serde(other)]
  Unknown` for forward compatibility during rolling upgrades.
- Wire-safe value enums (`UpdateFinalStatus`, `DisconnectReason`, `EnrollmentStatus`, `ErrorCode`,
  `Capability`) all follow the canonical `Other(String)` pattern with infallible deserialization,
  custom `Serialize`/`Deserialize`, `as_str()`, `Display`, and `From<String>`.
- `is_nats_publishable()` is the authoritative gate for credential-bearing messages, covering
  `ServiceCredentials`, `ExtensionRequest`, `ExtensionResponse`, `UpdateStdinData`, `ResetData`,
  all `ServiceConfig*` variants, `WorkloadClaimResult`, `TestPluginConfig`, and `Unknown`.
- Envelope-level sequence validation (`IncomingSeq`/`OutgoingSeq`) provides replay protection.
- Protocol version stamping (`CURRENT_PROTOCOL_VERSION`) enables version negotiation.
- Pagination metadata (`ReportPagination`) supports streaming large reports without buffering.
- `Box<ExecuteUpdatePayload>` and `Box<ExecuteBatchUpdatePayload>` avoid inflating the
  `ControllerMessage` enum size.
- `#[non_exhaustive]` on all payload structs with corresponding `::new()` constructors.
- Test coverage across serialization, unknown-variant handling, and message behavior is broad.

## Active Findings

No active findings were confirmed in this review pass.
