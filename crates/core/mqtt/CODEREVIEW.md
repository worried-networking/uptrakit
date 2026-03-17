# Code Review: `uptrakit-mqtt`

- Review date: 2026-03-17
- Scope: current-state review

## Summary

`uptrakit-mqtt` has solid recovery behavior for broker reconnects: cached state is republished, HA discovery is rebuilt, and the service keeps explicit per-tenant caches instead of relying on broker state alone. The main remaining issues are lossy event delivery under backpressure and unbounded partial-state buffer lifetime.

## Strengths

- Reconnect handling republishes cached software, host, and connectivity state instead of assuming retained topics are authoritative.
- Multi-page software-state delivery is accumulated explicitly before cache replacement, which avoids partial state publication.
- The service uses timeouts and reconnect logic instead of letting broker stalls block the whole runtime.
- Exponential backoff (2s-60s) with reset on ConnAck prevents reconnect storms.
- `try_publish` / `try_subscribe` pattern avoids event loop deadlock within the rumqttc client.
- ECIES decryption of sensitive extension parameters keeps secrets out of plaintext config.
- WorkloadClaim-based config distribution enables multi-instance MQTT service deployments.

## Active Findings

### [MEDIUM] MQTT service events are dropped on a full channel

- Dimension: high availability, observability
- Scope: `crates/core/mqtt/src/mqtt_client.rs`, `crates/core/mqtt/src/tenant_manager.rs`
- Why it matters: status, reconnect, HA-online, and command events use `try_send()` and log on drop instead of retrying or reconciling later.
- Failure scenario: broker reconnect storm, slow controller link, or bursty host updates fill the event channel (capacity: 512). The service keeps running, but the controller can miss state transitions.

### [MEDIUM] Incomplete multi-page `SoftwareStates` buffers are never garbage-collected

- Dimension: fault tolerance, memory
- Scope: `crates/core/mqtt/src/tenant_manager.rs`, `partial_states` buffer
- Why it matters: multi-page `SoftwareStates` payloads are buffered per MQTT client. If page 0
  arrives but the client disconnects before page 1, the incomplete entry lives in memory
  indefinitely. No TTL or periodic cleanup pass removes it.
- Failure scenario: repeated broker churn generates many orphaned partial-state entries that
  accumulate and waste memory with no operator-visible signal.
- Fix: add a TTL (e.g., 5 minutes) per `PartialSoftwareStates` entry and clean up expired entries
  on each recv cycle.

### [LOW] One `too_many_arguments` suppression is still undocumented

- Dimension: coding standards
- Scope: `crates/core/mqtt/src/ha_discovery/device.rs:88`
- Why it matters: the code is understandable today, but the lack of a local rationale makes future cleanup harder to prioritize.
