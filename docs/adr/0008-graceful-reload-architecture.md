# 0008 — Graceful Reload Architecture

Date: 2026-05-12

## Status

Accepted

## Context

Prior to this change, controller configuration was fragmented across three surfaces: CLI flags and
environment variables set at process start, DB `global_settings` rows mutated at runtime, and a
`CaSnapshotReceiver` watch channel that distributed TLS state reactively. There was no atomic reload
path. A DB mutation that changed a TLS or network setting could produce partially updated in-process
state, with no mechanism to revert if a downstream subsystem rejected the change. Expanding reactive
config to additional subsystems would require ad-hoc point solutions rather than a systematic
mechanism.

## Decision

A single TOML file (`/etc/uptrakit/controller.toml`, overrideable via `--config` flag or
`UPTRAKIT_CONFIG` env var) becomes the primary config source. Each top-level section is parsed into
a typed struct, wrapped in `Arc<…>`, and distributed to subscribing subsystems via a
`tokio::sync::watch` channel. This gives every subsystem a live receiver without polling.

A `Reloadable` trait defines the contract each long-lived subsystem must satisfy:

- `validate()` — checks the new config for correctness without mutating state
- `apply()` — applies the new config and stores a pre-apply snapshot for potential revert
- `health_check()` — verifies the subsystem is healthy after apply
- `revert()` — restores the pre-apply snapshot
- `rollback_window()` — returns the maximum duration the coordinator waits for `health_check()`

The `ReloadCoordinator` (in `crates/shared/config-reload/`) orchestrates a full reload cycle:
parse → validate-all → apply-all → health-check-all (within each subsystem's rollback window) →
commit or revert-all. If any health check fails, the coordinator reverts all subsystems atomically
and enters the `Degraded` state, preserving the last-known-good configuration.

Three triggers can initiate a reload: (a) `SIGHUP` signal; (b) a file-watch event on the TOML file,
debounced to 500 ms via `notify-debouncer-full`; (c) a `settings_version` increment in the DB
detected by `ConfigReconciler`, which polls every 2 s and replaces the former 30 s
`spawn_settings_reload` task.

If any changed key belongs to the irreversibly-bound set (listen addresses, DB pool URL, TLS trust
domain), the coordinator triggers `exec()` instead of an in-process reload. Listening sockets are
preserved via `listenfd`/`sd_notify`; accepted TCP connections are reset and clients reconnect via
retry loops.

Reloadable subsystems in v1: `AuditDispatcherReloadable`, `DbPoolReloadable`,
`TlsSnapshotReloadable`, `HttpsListenerReloadable`, `PkiListenerReloadable`,
`ZeroconfReloadable`, `PluginCatalogReloadable`, `EmbeddedServicesReloadable`.

## Alternatives considered

### 1. Best-effort per-subsystem reload

Each subsystem applies its config change independently, with no central coordinator. Rejected:
when two subsystems are related (e.g., DB pool and audit dispatcher both depend on the DB URL), a
failure in one leaves the system in a partial state with no recovery path. An operator cannot
determine which subsystems hold the new config and which hold the old.

### 2. In-process DB pool URL swap

Swap the live `sqlx` connection pool to a new URL without restarting the process. Rejected: `sqlx`
provides no `resize()` or pool-swap API. Replacing the pool under live queries introduces
ABA-hazard windows where in-flight queries complete against the old pool while new queries start on
the new pool. Connection-leak risk during the transition window is non-trivial.

### 3. RPC-based reload control

Accept reload instructions over an HTTP or socket RPC interface rather than signals and file-watch.
Rejected: bootstrap paradox — the controller must start before it can accept RPC connections, so
the very config that determines listen addresses cannot be changed via RPC before the listener is
up. Signal-based and file-watch triggers work before any listener is bound.

### 4. Splitting cluster config to NATS

Distribute config changes via NATS so multi-instance deployments receive updates simultaneously.
Rejected: introduces distributed consensus on config version ordering. The TOML-plus-DB model
achieves multi-instance coordination through `settings_version` without requiring a NATS cluster
for single-server deployments. NATS distribution can be added as a future extension.

## Consequences

**Positive:**

- Atomic revert-all on health-check failure eliminates partial-state scenarios. Operators always
  observe either the old config or the new config, never a mix.
- `ConfigReconciler` reduces DB-mutation-to-reload latency from 30 s to 2 s without busy-polling.
- Per-section `tokio::sync::watch` channels let subsystems react to config changes lazily (on next
  request) or eagerly (`receiver.changed().await`), without rebuilding the entire AppState.
- Reexec via `exec()` handles irreversibly-bound keys without a separate restart management tool;
  socket preservation means the HTTPS listener resumes with zero new-connection downtime.

**Negative / trade-offs:**

- **Hard CLI break:** The surviving flags are `--config`, `--master-key-from`,
  `--migrate-and-exit`, `--check-config`, `--version`, `--verbose`. All other flags previously
  accepted at the command line are now config-file keys.
- **New runtime dependencies:** `notify-debouncer-full`, `listenfd`, `sd-notify`, `toml`.
- **New permissions:** `view_instance_config_state` and `manage_instance_config_state` added to the
  `Permission` enum.
- **New endpoints:** `GET /api/v1/instance/config-state` (requires `view_instance_config_state`)
  and `POST /api/v1/instance/config-reload/clear-degraded` (requires
  `manage_instance_config_state`).
- **Ongoing ADR amendment discipline:** The irreversibly-bound key set must be kept accurate.
  Adding a key to that set or removing one is an ADR amendment, not a silent code change.
- **Removed paths:** `--reuseport`/`SIGUSR1` graceful-restart is removed; `spawn_settings_reload`
  30 s poll is replaced by `ConfigReconciler`.
- **Reexec resets accepted connections:** Clients relying on long-lived HTTP connections (SSE,
  WebSocket) must reconnect. The service-sdk reconnect loops handle this transparently for
  services; browser clients reconnect on the next SSE/WS open.
- **All settings mutation endpoints now require `If-Match`:** Missing header → 428; stale ETag
  → 409. This prevents lost-update races across multiple admin sessions.

## Cross-references

- Spec: `docs/superpowers/specs/2026-05-12-graceful-reload-design.md`
- Operator runbook: `docs/end-user/operator-runbook-reload.md`

## Amendment — 2026-05-17: Config Schema Simplification

The following config-format changes were applied to reduce section nesting and clarify field semantics.
No reload behaviour or irreversibly-bound-key semantics changed.

| Old key                                    | New key                        | Notes                                                                                                                                                 |
| ------------------------------------------ | ------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| `[master_key]` section + `path` field      | Top-level `master_key` string  | Accepts `file:<path>`, `env:<VAR>`, or inline 64-char hex. Inline form requires `chmod 0600` on the config file.                                      |
| `[network.https]` sub-section              | Fields directly in `[network]` | `addr`, `trusted_proxies`, proxy-cert headers now at `network.*`.                                                                                     |
| `[network.pki]` sub-section + `addr` field | `network.pki_addr`             | Renamed to clarify it is an advertisement address (not necessarily the bind address). Accepts bare `host:port` or `http://` URL. `https://` rejected. |

Irreversibly-bound key name updated: `master_key` (was `master_key.path`). The reexec trigger
in `triage.rs` and the related documentation in `coding-standards.md` reflect this rename.
