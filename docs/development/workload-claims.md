# Workload Claims

The workload claim protocol provides exclusive config-key-level ownership
across service instances. It prevents duplicate work (e.g. duplicate MQTT
publishes) when multiple instances of the same service type are deployed.

## Design Principles

- **Exclusive claims**: Each config key is owned by exactly one service instance.
- **Cross-controller coordination**: Claims are synchronized via NATS across all controllers.
- **Generic protocol**: Not MQTT-specific; any service type can use the same mechanism.
- **Controller as arbitrator**: The controller decides grant/reject; services request.
- **Graceful failover**: When a service disconnects, its claims are released and become available.
- **Config-driven**: Services derive their desired key set from their config entries.

## Key Concepts

### Config-key-level granularity

Claims operate at the config-key level (e.g. `clients.{uuid}`), not at the
tenant level. A single tenant can have thousands of config keys spread across
multiple service instances. Each config key is exclusively owned, but multiple
instances can serve the same tenant (with different configs).

### Full-replacement semantics

Each `WorkloadClaim` message sends the complete desired config key set. The
controller diffs against the service's current grants:

- **New keys**: claimed if available, granted or rejected.
- **Removed keys**: released, announcement sent via NATS.
- **Unchanged keys**: no action.

### Proactive re-grant

When claims are released (service disconnect or voluntary release), the
controller checks if any local service had a pending desire for those keys
and automatically grants them. This minimizes manual intervention and speeds
up failover.

## Architecture

### WorkloadClaimRegistry

Located in `crates/ui/web-api/src/workload_claims.rs`. Thread-safe via
`parking_lot::RwLock`. Maintains:

| Index | Type | Purpose |
| --- | --- | --- |
| `claims` | `HashMap<String, ClaimOwner>` | Config key to owner (service_id, controller_id, tenant_id, timestamp) |
| `by_service` | `HashMap<Uuid, BTreeSet<String>>` | Reverse index for fast release on disconnect |
| `tenant_services` | `HashMap<Uuid, BTreeSet<Uuid>>` | Tenant routing index for SoftwareStates delivery |
| `pending_desires` | `HashMap<Uuid, BTreeMap<String, Uuid>>` | Rejected claims for proactive re-grant |

### Tenant-scoped delivery

`NotificationService` and `deliver_event` use the claim registry's
`services_for_tenant()` to route `SoftwareStates` and
`HostConnectivityUpdated` only to services that hold at least one granted
config for that tenant. Falls back to broadcast when no claim registry is
configured.

### Cross-controller sync

- **`WorkloadClaimAnnouncement`** — published to NATS when claims change.
  Other controllers update their global registry.
- **`WorkloadClaimSyncRequest/Response`** — on startup, a controller
  requests the full claim state from peers before accepting connections.
- **Conflict resolution** — if two controllers grant the same key
  simultaneously, `(claimed_at, service_id)` comparison resolves it. The
  loser's controller revokes the claim and notifies the service.

## Wire Messages

| Message | Direction | Wire type | Purpose |
| --- | --- | --- | --- |
| `WorkloadClaim` | Service to Controller | `workload_claim` | Request exclusive ownership of config keys |
| `WorkloadClaimResult` | Controller to Service | `workload_claim_result` | Grant/reject response |
| `WorkloadRelease` | Service to Controller | `workload_release` | Voluntarily release specific config keys |
| `WorkloadClaimAnnouncement` | Controller to NATS | `workload_claim_announcement` | Sync claims across controllers |
| `WorkloadClaimSyncRequest` | Controller to NATS | `workload_claim_sync_request` | Request claim state from peers |
| `WorkloadClaimSyncResponse` | Controller to NATS | `workload_claim_sync_response` | Respond with local claim state |

See [Wire Protocol](../api/wire-protocol.md#workload-claim-protocol) for
payload schemas and examples.

## Key Files

| File | Description |
| --- | --- |
| `crates/shared/wire/src/payloads.rs` | Wire payload types (`WorkloadClaimPayload`, etc.) |
| `crates/shared/wire/src/capabilities.rs` | `Capability::WorkloadClaims` variant |
| `crates/ui/web-api/src/workload_claims.rs` | `WorkloadClaimRegistry` implementation |
| `crates/ui/web-api/src/routes/service_ws/handler/workload.rs` | WebSocket claim/release handlers |
| `crates/ui/web-api/src/event_delivery.rs` | NATS routing with tenant-scoped delivery |
| `crates/ui/web-api/src/notification_service.rs` | Tenant-scoped SoftwareStates delivery |
| `crates/core/mqtt/src/main.rs` | MQTT service claim protocol implementation |

## Scale Considerations

| Parameter | Limit | Notes |
| --- | --- | --- |
| Keys per claim message | 100,000 | `MAX_WORKLOAD_CLAIM_KEYS` |
| Service config entries | 1,000,000 | `MAX_SERVICE_CONFIG_ENTRIES` |
| Memory at 1M keys | ~250 MB | Registry + reverse indexes |

## Testing

Unit tests for the `WorkloadClaimRegistry` are in
`crates/ui/web-api/src/workload_claims.rs` (inline `#[cfg(test)]` module).
They cover:

- Claim granting and rejection
- Release and re-grant
- Remote announcement processing
- Conflict resolution (timestamp + service_id tiebreak)
- Tenant routing index maintenance
- Pending desires tracking

## Security Considerations

- Claims are enforced server-side by the controller. Services cannot bypass
  the claim protocol.
- The registry is in-memory only (no DB persistence). On controller restart,
  services reconnect and re-claim.
- NATS announcements contain config keys and tenant UUIDs but no secret
  material.
- See [Security documentation](../security/) for the broader security model.
