# Complete Graceful Reload — Design

Status: Draft for review
Author: Andrey Yantsen
Date: 2026-05-12
ADR: [`docs/adr/0008-graceful-reload-architecture.md`](../../adr/0008-graceful-reload-architecture.md) (new)
Edition: Rust 2024

## 1. Goal

Every setting that an Operator can change should take effect on the running Controller without a
process restart. The change in scope is _complete_: not just the DB-persisted settings reachable
through `SettingKey`, but also the values currently sourced from CLI flags and environment
variables — including DB URL, master-key location, listen addresses, NATS URL, and the
embedded-services topology.

After this work, the Operator-visible mental model is: "I changed a setting, and the change
applied. I never restarted the Controller." Reaching that goal requires (a) consolidating the
boot-time configuration surface into a single re-readable TOML file, (b) introducing a uniform
reload protocol that every long-lived subsystem implements, (c) atomic two-phase validation and
application with watchdog-protected rollback, and (d) graceful self-`exec()` for the small set
of keys whose change cannot be applied to an already-running process without compromising
correctness or safety.

## 2. Background

Today the Controller's configuration surface is fragmented across three layers:

- **CLI flags / environment variables**, parsed in `crates/core/controller-runtime/src/startup/*`,
  baked into the process at boot. Changing any of them requires a full restart.
- **Per-Tenant `settings`** and **instance-wide `global_settings`** rows, keyed by `SettingKey`
  (`crates/ui/web-api-auth/src/setting_key.rs`). Most of these are read once at startup or
  consulted on every request. A handful (TLS via `CaSnapshotReceiver`, agent-cert renewal,
  zeroconf advertiser) already use `tokio::sync::watch` to react to changes mid-flight.
- **Plugin configs** (`plugin_configs`, `plugin_type_settings`, instance-scoped slots in
  `global_settings`) — partially reactive depending on the plugin's internal architecture.

The piecemeal reactivity is the problem. Operators who change `nats.url` through the Dashboard
still have to bounce the Controller. Operators who change `network.https_addr` must edit the
DB row _and_ restart. There is no uniform model: each subsystem implements (or does not
implement) reload in its own way, and most do not. Several settings rows are intentionally
labelled "requires restart" in the Dashboard. The "Instance Configuration" surface is
implicitly read-only because the Controller cannot react to mutations.

The `ServerState`, `CaSnapshotReceiver`, and `revocation_notify` fields on `AppState` already
demonstrate a working hot-reload pattern using `tokio::sync::watch` and `tokio::sync::Notify`.
The plumbing exists for a subset of the surface; this spec generalises it across the whole
Controller.

## 3. Scope

### 3.1 In scope

- Single-file TOML configuration (`/etc/uptrakit/controller.toml` by default) covering every
  boot-time input that is currently a CLI flag or environment variable, with the exception of
  three bootstrap shims: `--config`, `--master-key-from`, `--migrate-and-exit`, `--version`.
- Per-section `tokio::sync::watch<Arc<SectionConfig>>` propagation from a central
  `RuntimeConfig` to every long-lived subsystem.
- Reload triggers: `SIGHUP`, file-system watch on the TOML file (`notify` crate, 500 ms
  debounce), and `settings_version`-bump polling for DB-rooted sections.
- A `Reloadable` trait implemented by every long-lived subsystem (listeners, DB pool, NATS
  client, plugin registries, audit dispatcher, zeroconf advertiser, embedded-service
  supervisor).
- Two-phase reload coordinator: validate-all then apply-all, with per-subsystem watchdog and
  revert-from-snapshot on health-check failure within a per-subsystem rollback window.
- Self-`exec()` (process replacement, preserving listening sockets via `LISTEN_FDS`) for the
  irreversibly-bound key set: `db.url`, `master_key.path`, `log.path`, embedded-services
  topology.
- `If-Match` / ETag optimistic locking on settings mutation endpoints (reuse
  `settings_version` as the ETag value).
- New audit event types for every reload phase, plus a read-only Dashboard surface for
  reload state, file digest, pending changes, and recent reload audit history.
- New permission `ViewInstanceConfigState` for the read-only surface.
- All-or-nothing rollout: the feature ships in a single release, every subsystem
  participating from day one.

### 3.2 Out of scope

- **Backward compatibility for the old CLI / environment variable surface.** Old flags and env
  vars are removed in this release; no shim, no automated migration tool, no deprecation
  window. Existing deployments must be reconfigured.
- **Cross-node TOML replication.** Each Controller node has its own local TOML file. Operators
  with multi-node deployments use external configuration management (Ansible, Kubernetes
  `ConfigMap`, etc.). DB-rooted settings continue to propagate cluster-wide through the
  existing `settings_version` mechanism.
- **Out-of-process Service reload.** Reload only applies to Services running in Embedded
  Mode inside the Controller binary. External Agent, Agent-SSH, MQTT Service, and Scheduler
  processes have their own restart story and are not addressed here.
- **Plugin tenant configs.** Already cross-node via DB; covered by the existing reactive plug
  registry path without new mechanism.
- **DB schema migrations.** Migrations run only at boot. Reload never triggers a migration.
- **Frontend hot-reload.** Separate dev-server concern. Dashboard reloads on its own schedule.
- **A Dashboard "Reload Now" button.** Reload is exclusively triggered by `SIGHUP`,
  file-watch, or DB-version bump. Dashboard remains observability-only for reload state.

## 4. Domain Language Additions

The following terms are added to `CONTEXT.md`. They are not implementation details — they
appear in Operator-facing UI copy, runbook prose, and audit log entries.

- **Reload Coordinator** — the single Tokio task that serialises reload requests, runs the
  two-phase validate-then-apply protocol, drives the watchdog, and commits or reverts. There
  is exactly one Reload Coordinator per Controller process. _Avoid_: "config manager", "reload
  manager" (overloaded).
- **Reloadable** — a long-lived subsystem that implements the `Reloadable` trait and
  participates in reload. Listeners, DB pool, NATS client, plugin registries, audit
  dispatcher, zeroconf advertiser, and embedded-service supervisor are all Reloadables.
- **Config Section** — a logical grouping of settings whose lifetime is bound together for
  reload purposes. Each section corresponds to one `Arc<SectionConfig>` and one
  `watch::Sender` channel. Examples: `[network]`, `[nats]`, `[tls]`, `[db]`, `[audit]`.
- **Reexec** — in-place process replacement via `exec()` with inherited listening sockets,
  used for the irreversibly-bound key set. From the Operator's point of view this is still
  "graceful reload", because the new process serves traffic on the same sockets without
  downtime. _Avoid_: "graceful restart" (ambiguous; some readers will think we mean
  `systemctl restart`).
- **Irreversibly-bound key** — a configuration key whose change cannot be applied to a
  running process without compromising correctness, safety, or operability. Currently:
  `db.url`, `master_key.path`, `log.path`, embedded-services topology. The set is
  intentionally small and is reviewed as part of every reload-related change.
- **Watchdog window** — per-subsystem time budget within which a newly-applied configuration
  must pass `Reloadable::health_check()`. Expiry triggers subsystem-internal revert from
  pre-apply snapshot.
- **`ConfigReconciler`** — the Tokio task that polls `settings_version` (and per-tenant
  `global_version`) for bumps and enqueues a reload request when DB sections change. One
  per Controller; runs at a 2-second baseline interval. Future optimisation: Postgres
  `LISTEN/NOTIFY`.

## 5. Architecture Overview

```text
                            +----------------------------+
                            |  /etc/uptrakit/controller. |
                            |       toml (on disk)       |
                            +-------------+--------------+
                                          |
       +------ SIGHUP -------+            | notify::Watcher (500 ms debounce)
       |                     |            |
       |          +----------v------------v----------+
       |          |   TomlConfigLoader                |
       |          | - parse + serde::Deserialize      |
       |          | - top-level Validate trait        |
       |          +-----------------+-----------------+
       |                            |
       |                            v
       |          +-----------------+------------------+
       +--------->|        Reload Coordinator          |
                  |  (single tokio task, mpsc queue)   |<------+
                  |                                    |       |
                  |  Phase 1: validate_all()           |       |
                  |  Phase 2: snapshot + apply_all()   |       |
                  |  Phase 3: watchdog + commit/revert |       |
                  +-+------+-------+--------+----------+       |
                    |      |       |        |                  |
            +-------v+ +---v---+ +-v---+ +--v------+   +-------+---------+
            |Network | |  NATS | | TLS | |  Audit  |   | ConfigReconciler|
            |Listener| | Client| |Snap | |Dispatch |   |  (DB poller)    |
            +--------+ +-------+ +-----+ +---------+   +--------+--------+
                ^         ^        ^         ^                  |
                |         |        |         |                  |
                +---+-----+----+---+---------+                  |
                    |          |                                |
            tokio::sync::watch  |                               |
            (one channel per    |     +-----------+             |
             Config Section)    +-----+ Plugin    |<---settings_version
                                      | Registry  |   bump polled here
                                      +-----------+
                                            |
                                      Arc<dyn Plugin> swap
                                      on config change

                     +----------------------------+
                     |   Reexec branch (if any    |
                     |  irreversibly-bound key    |
                     |  changed):                  |
                     |  std::os::unix::process::  |
                     |  CommandExt::exec() with   |
                     |  LISTEN_FDS=N, LISTEN_PID  |
                     |  inherited sockets via     |
                     |  listenfd crate            |
                     +----------------------------+
```

### 5.1 Crate layout

A new crate `uptrakit-config-reload` lives under `crates/shared/`. It contains:

- `RuntimeConfig` (root struct holding all sections)
- Section types (`NetworkConfig`, `NatsConfig`, `TlsConfig`, `DbConfig`, `AuditConfig`,
  `LogConfig`, `EmbeddedServicesConfig`, `MasterKeyConfig`, `ZeroconfConfig`)
- `Reloadable` trait
- `ReloadCoordinator`
- `TomlConfigLoader`
- `ConfigReconciler`
- Reexec helper (`reexec::trigger`, `reexec::inherit_listeners`)
- TOML serde derives and round-trip tests
- A `Validate` trait implementation for every section (matching the snapshot rule that all
  externally-sourced types implement `Validate`)

`controller-runtime` depends on this crate and wires it into `main.rs`. Subsystems live in
their existing crates and gain a `Reloadable` impl alongside their constructor.

Per the snapshot rule, all extensible public enums on `RuntimeConfig` carry
`#[non_exhaustive]`. Section structs that may be extended carry `#[non_exhaustive]` as well,
forcing external callers to use `::new()` or `Default` constructors.

## 6. Configuration Sources

### 6.1 TOML file shape

The canonical TOML file is structured as one table per Config Section. Each section maps to
exactly one `Arc<SectionConfig>` published via `watch`. All keys have explicit defaults
materialised in code; the file may omit any key and the default applies.

```toml
# /etc/uptrakit/controller.toml

[db]
url = "sqlite://var/lib/uptrakit/controller.db"
pool_size = 16
acquire_timeout_ms = 5000
# url change ⇒ reexec (irreversibly-bound)

[master_key]
path = "/etc/uptrakit/master.key"
# path change ⇒ reexec (irreversibly-bound)

[network.https]
addr = "0.0.0.0:8443"
trusted_proxies = ["127.0.0.1/32"]
real_ip_header = "x-forwarded-for"
forwarded_client_cert_info_header = "x-forwarded-client-cert"
forwarded_client_cert_pem_header  = "x-forwarded-client-cert-pem"

[network.pki]
addr = "0.0.0.0:8444"

[tls]
cert_path = "/etc/uptrakit/tls/cert.pem"
key_path  = "/etc/uptrakit/tls/key.pem"
sans      = ["controller.example.com"]

[nats]
url = "nats://localhost:4222"
# in-process reconnect; no reexec

[audit]
filter    = "all"
retention_days = 90
# in-process reload

[log]
path  = "/var/log/uptrakit/controller.log"
level = "info"
# path change ⇒ reexec

[zeroconf]
enabled = true
url      = "https://controller.local:8443"
pki_addr = "controller.local:8444"

[embedded_services]
agent     = false
agent_ssh = false
mqtt      = false
scheduler = true
# topology change ⇒ reexec
```

A `Validate` impl on `RuntimeConfig` enforces cross-section invariants (e.g.,
`network.https.addr` and `network.pki.addr` are distinct, `db.pool_size` ≥ 1,
`audit.filter` ∈ {`all`, `mutations`, `none`}). Per-section `Validate` checks individual key
shapes (URL parses, port-in-range, file paths absolute).

Every section struct derives `serde::Deserialize` with `#[serde(deny_unknown_fields)]` so
that typos in the TOML file (`poool_size = 8`) fail at parse time instead of silently
falling back to defaults. This makes `--check-config` reliable as a CI gate.

**Downgrade escape hatch.** Strict deny would brick rollbacks (a TOML written for version
N+1 with new keys would fail to parse on version N). To preserve rollback ergonomics, each
section struct also carries a `#[serde(flatten)] _extra: HashMap<String, toml::Value>`
field. Unknown keys land in `_extra`; a `RuntimeConfig::warn_about_extras()` pass emits a
`tracing::warn!` for every captured key during boot and writes a `system_alerts` row with
severity `Warning`. The Operator sees the alert, knows the key is being ignored, and can
clean up the file at their convenience instead of during a deploy. Typos still fail
loudly because they sit in `_extra` and produce warnings; structural errors (wrong type,
missing required field) still fail parse. This is the kubernetes / terraform pattern.

### 6.2 DB-rooted sections

Sections whose source of truth is the DB remain in DB, accessed through `settings_store`
and keyed by `SettingKey`. They are exposed to subsystems through the same
`watch<Arc<SectionConfig>>` channels as TOML-rooted sections. `ConfigReconciler` is the
producer for these channels.

DB-rooted (today, partial list — verified at impl time against `SettingKey`):

- `registration.mode`, `registration.token_hash`, `registration.require_token_for_oidc`
- `auth.password_enabled`
- `agent_certificate.lifetime_hours`, `agent_certificate.renewal_window_hours`
- `pki.active_ca_fingerprint`, `pki.ca_version`
- `multi_tenancy.enabled`
- Per-tenant `audit_log.filter`, `audit_log.retention_days` (override global)

These continue to live in `global_settings` / `settings`. The reload work introduces a
mapping function `db_section_from_keys(keys: &[SettingKey]) -> ConfigSection` so the
reconciler can publish only the affected section after a bump.

### 6.3 Settings split

Each `SettingKey` and each new TOML key is assigned at design-review time to one of:

- **File** — TOML only; not present in `global_settings`.
- **DB** — `global_settings` / `settings` only; not present in TOML.
- **Forbidden** — neither source can persist this; rejected at validation. Reserved for keys
  that exist only as ephemeral runtime state.

A `where` column is added to a developer-facing table in
`docs/development/coding-standards.md`. The list is the source of truth for "which file is
this setting in?"; reviewers reject mutations that violate the assignment.

Boot-critical and process-lifetime keys (DB URL, master key path, listen addresses, log file
path, embedded topology) are **File**. Anything Operator-tunable through the Dashboard
remains **DB**.

### 6.4 CLI shrink

After this work, the Controller binary's CLI surface is:

```text
uptrakit-controller --config <path>            # default: /etc/uptrakit/controller.toml
                    --master-key-from <ref>    # path:/abs/file | env:VARNAME
                    --migrate-and-exit         # run migrations, then exit 0
                    --check-config             # validate the TOML file, exit 0/1; no side effects
                    --version
                    --help
```

`--check-config` is the CI / pre-rollout **lint** hook (think `nginx -t`, not `nginx
--probe-upstreams`). It runs only the parse-plus-validate path: `toml` parse, per-section
`Validate` impls, the cross-section `RuntimeConfig::validate()`, and the unknown-keys
warning pass (warnings go to stderr but do not change the exit code). It prints the first
hard error (if any) to stderr and exits non-zero. It does **not** touch the DB, listeners,
plugins, the NATS server, or the master-key file. A green `--check-config` means "this
file parses and validates statically"; it does **not** mean "the Controller will reach the
configured database". Operators who want a network-reachable probe must run the full
binary in a staging environment. This limitation is documented in the runbook so green
`--check-config` is not confused with deployment readiness.

`UPTRAKIT_CONFIG` and `UPTRAKIT_MASTER_KEY_FROM` are honoured as overrides of `--config` and
`--master-key-from` respectively. No other environment variable is read by the Controller.

All prior CLI flags (`--db-url`, `--https-addr`, `--pki-addr`, `--nats-url`,
`--audit-log-filter`, etc.) are removed. There is no shim. Deployments that previously
relied on them must produce a TOML file before the upgrade; the operator runbook documents
the mapping but no automated tool generates the file. The hard break is deliberate — keeping
flag aliases would double the config-source surface and create "which wins on reload?"
ambiguity that we have explicitly chosen to avoid.

## 7. Reload Triggers

Three sources can request a reload. All three deliver into the same coordinator queue.

### 7.1 `SIGHUP`

A Tokio task created at startup awaits `SignalKind::hangup()` and enqueues
`ReloadRequest::Sighup` on the coordinator channel. This is the canonical Operator trigger
and the trigger used in non-systemd contexts. It is idempotent: holding `kill -HUP` does not
reliably enqueue more than one request, and that is fine — the coordinator coalesces
redundant work.

### 7.2 File-watch

A `notify_debouncer_full::Debouncer` (from the `notify-debouncer-full` crate) watches the
parent directory of the TOML file. The debouncer coalesces events with a 500 ms tick and
delivers a batch of de-duplicated events to a Tokio channel; the channel reader enqueues
`ReloadRequest::FileWatch { path }`. Watching the parent directory (not the file itself)
handles rename-in-place edits used by atomic editors. TLS cert paths are watched the same
way and trigger only the `TlsConfig` section.

We use the debouncer crate rather than hand-rolling `notify::RecommendedWatcher` +
`tokio::time::sleep` resets because the debouncer correctly handles cancellation of the
in-flight delay future, de-duplicates by path, and avoids the `clippy::large_futures = "deny"`
risk of holding a multi-arm `select!` open across reload cycles.

### 7.3 `settings_version` bump (`ConfigReconciler`)

A `tokio::time::interval(Duration::from_secs(2))` task reads the `settings_version` row(s)
and compares against last-observed counters. On any bump, the reconciler determines which
DB sections are affected and enqueues `ReloadRequest::DbBump { sections, scope }`.

The reconciler holds the latest counter per scope (global and per-tenant) in
`arc_swap::ArcSwap<HashMap<Scope, u64>>` (workspace dep, already in active use for
`InstancePluginSnapshot`). The write-rarely / read-often access pattern fits `ArcSwap`'s
lock-free read path more naturally than a `parking_lot::Mutex`; the snapshot rule on
sync locks is also satisfied because `ArcSwap` is not a `Mutex` at all.

This task **replaces** the existing 30-second `SETTINGS_POLL_INTERVAL` / `spawn_settings_reload`
loop (in `crates/core/controller-runtime/src/tasks.rs`). That task is removed during
implementation; there is no coexistence period. All consumers move to the per-section
`watch::Receiver` channels produced by the reconciler.

Postgres `LISTEN/NOTIFY` is a future optimisation. SQLite (single-node dev) is unaffected
because the reconciler polls in-process.

### 7.4 Coordinator queue

The coordinator owns a `tokio::sync::mpsc::Receiver<ReloadRequest>`. Multiple producers send;
the single consumer processes requests one at a time. If multiple requests queue while a
reload is in flight, the coordinator drains the queue at the start of the next cycle and
unions the trigger sources into a single composite request. This guarantees that two
quick edits never cause overlapping reloads while preserving the audit record of which
sources triggered.

## 8. Reload Coordinator

```rust
pub struct ReloadRequest {
    pub source: ReloadSource,
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ReloadSource {
    Sighup,
    FileWatch { path: PathBuf },
    DbBump { scope: Scope, sections: Vec<String> },
    Boot,
    /// Catch-all for forward-compatible audit-log JSON. Mandatory per the snapshot rule:
    /// this enum is serialised into audit events that cross the REST and audit-log
    /// boundaries.
    Other(String),
}
```

`ReloadSource` is **wire-exposed** through audit-log JSON and through the
`/api/v1/instance/config-state` endpoint's `recent_events` payload. It therefore carries
`Other(String)` per the snapshot rule. Same applies to `ReloadPhase` below.

### 8.1 State machine

```text
                Idle
                 |
                 |  request enqueued
                 v
            DrainAndCoalesce
                 |
                 v
            LoadCandidate     <-- read TOML + read affected DB sections
                 |
                 v
            ValidateAll       <-- per-Reloadable validate(&new) plus
                 |                cross-section RuntimeConfig::validate()
                 |
            +----+----+
            v         v
        Failed     Passed
            |         |
            |         v
            |    Snapshot      <-- each Reloadable stashes pre-apply state
            |         |
            |         v
            |    ApplySequenced
            |         |
            |         v
            |    Watchdog
            |         |
            |    +----+----+
            |    v         v
            |  Reverted  Committed
            |    |         |
            |    v
            |  (any revert returned Err?)
            |    |
            |  +-+-+
            |  Y   N
            |  |   |
            |  v   |
            | Degraded  <-- absorbing state; refuses further reloads
            |    |       until /api/v1/instance/config-reload/clear-degraded
            |  +-+----+
            |  |      |
            +--+------+
                 |
                 v
              Audit emit
                 |
                 v
                Idle (or Degraded sink)
```

### 8.2 Apply ordering

Subsystems apply in a fixed dependency-respecting order, executed sequentially:

1. **DbConfig pool resize / timeout updates** (DB URL itself is irreversibly-bound; this
   step only applies in-place tunables: `pool_size`, `acquire_timeout_ms`)
2. **NatsConfig** (reconnect to new URL, re-establish subscriptions)
3. **TlsConfig / CA snapshot** (TLS rotation continues to flow through the existing
   `CaSnapshotReceiver`; this step publishes any path changes)
4. **NetworkConfig listeners** (HTTPS, PKI; pre-bound during validate)
5. **AuditConfig** (filter, retention, dispatcher buffers)
6. **ZeroconfConfig** (re-advertise mDNS)
7. **Plugin registry** (drop-and-recreate `Arc<dyn Plugin>` for plugins whose config
   changed)
8. **Embedded-service supervisor** (only handles in-process topology adjustments that are
   _not_ topology-add/remove — adds/removes are irreversibly-bound)

If apply on any step returns `Err`, the coordinator immediately enters Reverted state and
calls `revert()` on every subsystem that already applied, **in reverse application order**
(last applied first; matches dependency unwinding semantics). Revert is best-effort — each
subsystem holds its own pre-apply snapshot — and any revert failure is logged + emitted as
a system alert with severity `Critical`.

The coordinator's main loop is split into small async helpers (`run_validate_phase`,
`run_apply_phase`, `run_watchdog_phase`, `run_revert_phase`) rather than a single large
`async fn`, because the workspace denies `clippy::large_futures`. Watchdog concurrency uses
`futures::stream::FuturesUnordered` rather than `join_all(Vec<…>)` for the same reason.

### 8.3 Failure modes

- **Validate failure**: no subsystem is mutated. Coordinator emits `ConfigReloadFailed
{ phase: Validate, subsystem: <name or None>, error }`, writes a `system_alerts` row with
  severity `Warning`, returns to Idle. The new TOML or DB state is _not_ discarded — the
  file remains on disk and the DB rows remain. The current loaded configuration continues
  to serve traffic.
- **Apply failure** mid-sequence: revert previously-applied subsystems; emit
  `ConfigReloadReverted { subsystem, reason }` per reverted subsystem, then
  `ConfigReloadFailed { phase: Apply, subsystem, error }`. `system_alerts` severity
  `Error`.
- **Watchdog timeout / health-check failure**: **all** participating subsystems revert in
  reverse application order. We do not retain partial commits, because subsystem
  dependencies (e.g., HTTPS routes depending on NATS event routing) make "healthy
  subsystems keep new config; unhealthy reverts" produce mixed states that are very hard to
  reason about. The simpler, safer rule: any single watchdog failure is a full reload
  failure. Audit events: one `ConfigReloadReverted { subsystem }` per reverted subsystem
  plus one `ConfigReloadFailed { phase: Watchdog, subsystem: <the failing one> }`.
- **Revert itself fails (`revert()` returns `Err`)**: coordinator enters the
  **Degraded** state. From Degraded, the coordinator:
  - Refuses further reload requests: `SIGHUP` and file-watch events log a refusal and
    do nothing; DB `settings_version` bumps are observed but not actioned (the
    reconciler keeps consuming so the version stream is not lost; it just does not
    drive applies).
  - `system_alerts` severity `Critical`, persistent until cleared.
  - The `GET /api/v1/instance/config-state` endpoint returns the Degraded reason and the
    set of subsystems whose revert failed.
  - The Operator clears Degraded explicitly via
    `POST /api/v1/instance/config-reload/clear-degraded` (requires a new permission
    `ManageInstanceConfigState`). The clear endpoint re-runs `health_check()` on every
    Reloadable; if all pass, the coordinator returns to Idle and resumes accepting
    reloads. If any still fail, Degraded persists and the response details the failures.
    This converts §16's "Operator must intervene" line from advisory text into an
    enforceable runtime constraint.
- **Reexec failure** before `exec()`: parent stays alive with old config; emit
  `ConfigReloadFailed { phase: Reexec, error }`; system alert `Critical`.
- **Reexec failure** _after_ `exec()` (child fails to start or fails its health check):
  child exits non-zero; systemd (or the init system) restarts the Controller, which boots
  with the new config and either succeeds or enters a crash loop. The crash loop is the
  Operator's signal that the new config is fatally broken; they revert the TOML file from
  another node, from a backup, or via an out-of-band fix.

## 9. The `Reloadable` Trait

```rust
use std::sync::Arc;
use std::time::Duration;
use rootcause::Report;

pub trait Reloadable: Send + Sync {
    /// The configuration section this subsystem owns.
    type Config: Send + Sync + 'static;

    /// Stable identifier used in audit events, logs, and Dashboard surface.
    fn name(&self) -> &'static str;

    /// Pure validation: does this subsystem accept `new` as a usable configuration?
    /// MUST NOT mutate state. MAY perform read-only probes (e.g., parse a URL, try to
    /// pre-bind a port, attempt a connect on a probe socket — _without_ holding the
    /// resulting handle for production use).
    fn validate(&self, new: &Self::Config) -> Result<(), Report>;

    /// Apply the new configuration. The subsystem MUST internally snapshot enough of its
    /// pre-apply state to satisfy `revert()`. Returning `Ok` means "I have applied the
    /// new config; you may now run `health_check()` against me".
    async fn apply(&self, new: Arc<Self::Config>) -> Result<(), Report>;

    /// Roll back to the pre-`apply()` snapshot. Called by the coordinator on apply
    /// failure further down the sequence, or on watchdog timeout. MUST be best-effort
    /// idempotent and MUST NOT panic.
    async fn revert(&self) -> Result<(), Report>;

    /// Run a liveness probe against the just-applied configuration. Returning `Ok` within
    /// `rollback_window()` commits the apply; returning `Err` or exceeding the window
    /// triggers `revert()`.
    async fn health_check(&self) -> Result<(), Report>;

    /// Per-subsystem watchdog budget. Defaults below; subsystems may override.
    fn rollback_window(&self) -> Duration;
}
```

The typed `Reloadable<Config = T>` trait is **not** object-safe (it has an associated type
and `async fn` methods). The coordinator therefore holds a `Vec<Box<dyn ReloadableErased>>`
where `ReloadableErased` is a separate trait annotated with the workspace's
`#[async_trait]` macro (already in active use across the codebase, e.g.,
`crates/ui/web-api-queries/src/plugin_ops.rs`). `ReloadableErased` accepts a
`&RuntimeConfigDelta` enum whose variants carry the new section payloads. Each
subsystem's `ReloadableErased` impl matches on the variant it owns and forwards to its
typed `Reloadable`. There is no `Arc<dyn Any>` downcast: the enum is the dispatch.

```rust
#[non_exhaustive]
pub enum RuntimeConfigDelta {
    Db(Arc<DbConfig>),
    Network(Arc<NetworkConfig>),
    Nats(Arc<NatsConfig>),
    Tls(Arc<TlsConfig>),
    Audit(Arc<AuditConfig>),
    Zeroconf(Arc<ZeroconfConfig>),
    EmbeddedServices(Arc<EmbeddedServicesConfig>),
    Plugin(PluginTypeId, Arc<PluginConfig>),
}
```

`RuntimeConfigDelta` is in-process only (never serialised over wire) so it does not need
`Other(String)`. Adding a new section means: add a variant here, write a new `Reloadable`

- `ReloadableErased` impl, register it in the coordinator's table.

```rust
#[async_trait]
pub trait ReloadableErased: Send + Sync {
    fn name(&self) -> &'static str;
    fn validate(&self, delta: &RuntimeConfigDelta) -> Result<(), Report>;
    async fn apply(&self, delta: &RuntimeConfigDelta) -> Result<(), Report>;
    async fn revert(&self) -> Result<(), Report>;
    async fn health_check(&self) -> Result<(), Report>;
    fn rollback_window(&self) -> Duration;
}
```

`#[async_trait]` desugars each `async fn` to `Pin<Box<dyn Future + Send>>` exactly as the
rust-idioms rule on type erasure expects; no hand-written `Pin<Box<dyn Future>>` signatures
appear in the `ReloadableErased` declaration itself. The macro is the workspace's
established idiom for `dyn`-compatible async traits.

The typed `Reloadable` is preferred wherever the call site holds a concrete subsystem
(per-subsystem unit tests, internal helpers). Only the coordinator's heterogeneous registry
holds `dyn ReloadableErased`.

### 9.1 Default watchdog windows (hardcoded constants)

```rust
// In uptrakit-config-reload::defaults — exposed as TOML keys only if Operator demand
// surfaces. TODO: revisit per real-world telemetry.
pub const WATCHDOG_DB_POOL:    Duration = Duration::from_secs(15);
pub const WATCHDOG_NATS:       Duration = Duration::from_secs(10);
pub const WATCHDOG_HTTPS:      Duration = Duration::from_secs(5);
pub const WATCHDOG_PKI:        Duration = Duration::from_secs(5);
pub const WATCHDOG_PLUGINS:    Duration = Duration::from_secs(30);
pub const WATCHDOG_AUDIT:      Duration = Duration::from_secs(5);
pub const WATCHDOG_ZEROCONF:   Duration = Duration::from_secs(5);
pub const WATCHDOG_EMBEDDED:   Duration = Duration::from_secs(30);

pub const HTTPS_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);
pub const PKI_DRAIN_TIMEOUT:   Duration = Duration::from_secs(5);

pub const FILE_WATCH_DEBOUNCE: Duration = Duration::from_millis(500);
pub const RECONCILER_POLL:     Duration = Duration::from_secs(2);
```

## 10. Per-Subsystem Designs

This section enumerates the Reloadables, their `Config` type, their validate / apply /
revert / health_check semantics, and whether they reload in-process or trigger reexec.

### 10.1 HTTPS listener (`NetworkConfig::https`)

- **Watch channel**: `watch<Arc<HttpsConfig>>`, owned by the listener supervisor task.
- **validate**: parse `addr`. Pre-binding a probe listener at validate time is **skipped**
  when the new `addr` equals the current `addr` (no change) and when a drain of the old
  listener is in flight (which would race with the probe bind). When the addr genuinely
  changes and no drain is in flight, the probe bind is attempted; failure (port in use by
  another process) fails validation.
- **apply**: bind a new production listener on the new `addr`. Hand the new listener to a
  freshly-spawned axum `serve` task with the existing router. Signal the old serve task to
  enter `with_graceful_shutdown` with `HTTPS_DRAIN_TIMEOUT`. The old listener stops
  accepting new connections; in-flight requests complete or hit the drain timeout.
- **revert**: drop the new listener, signal the new task to shut down, ensure the old task
  is still serving (no-op if the old task was not yet drained).
- **health_check**: poll the new listener by connecting to one of the addresses the socket
  is actually bound on (prefer `127.0.0.1` if loopback is in the bind set; otherwise pick
  the first concrete IP from the bind set). Pass if the round-trip completes within 1
  second.
- **In-process** — no reexec.

### 10.2 PKI listener (`NetworkConfig::pki`)

Same shape as HTTPS but with `PKI_DRAIN_TIMEOUT = 5s` because Service ↔ Controller mTLS
connections reconnect automatically and quickly.

### 10.3 DB pool (`DbConfig`, in-place tunables only)

`sqlx::Pool` (and therefore SeaORM's `DatabaseConnection`) does not expose a `resize()`
method; `max_connections` is fixed at construction time. The reload model is therefore
**reconnect**, not in-place mutation.

- **Watch channel**: `watch<Arc<DbConnHandle>>`, owned by an inner wrapper around
  `DatabaseConnection`. `DbConnHandle` is the wrapper, not the raw `DatabaseConnection`,
  so consumers fetch a handle on every query rather than caching the inner
  `DatabaseConnection`. Definition:

  ```rust
  /// Reload-aware DB connection wrapper.
  pub struct DbConnHandle {
      /// The live pool. Replaced wholesale on reload; never mutated in place.
      conn: DatabaseConnection,
  }

  impl DbConnHandle {
      pub fn conn(&self) -> &DatabaseConnection { &self.conn }
  }
  ```

  Call sites that previously held `Arc<DatabaseConnection>` now hold
  `watch::Receiver<Arc<DbConnHandle>>` and call `rx.borrow().conn()` per query. The
  `Arc` ensures that any reference held over an `.await` point survives a concurrent
  reload (the old pool stays alive until the last reference drops).

- **validate**: confirm `db.url` is unchanged versus the running pool's URL (URL change is
  irreversibly-bound → reexec path); validate `pool_size ≥ 1` and `acquire_timeout_ms > 0`.
- **apply**: construct a new `DatabaseConnection` (and underlying pool) with the new
  `pool_size` / `acquire_timeout_ms` (URL unchanged). Atomically publish the new
  `Arc<DbConnHandle>` through the watch channel. The old pool drains naturally: when the
  last `Arc<DbConnHandle>` reference (held by some in-flight query) drops, the wrapper's
  `Drop` calls `pool.close()` in a detached task. No fixed grace timeout — long-running
  queries (e.g., tenant export) keep the old pool alive for as long as they need; new
  queries arrive on the new pool. The old pool's resource footprint is bounded by the
  longest in-flight query, which is itself bounded by `acquire_timeout_ms` and the
  query-level deadlines already applied by callers.
- **revert**: publish the prior `Arc<DbConnHandle>` (held in the pre-apply snapshot); close
  the would-be-new pool.
- **health_check**: `SELECT 1` round-trip on the new handle within 2 seconds.
- **In-process** for tunables. URL change → reexec.

The reconnect cost (new pool spin-up, TLS handshakes for Postgres connections) is real but
amortised across the pool lifetime — `pool_size` and `acquire_timeout_ms` are very rarely
tuned at runtime. The simpler "always reconnect on any tunable change" model removes the
need for a (nonexistent) `pool.resize()` API while preserving the Operator-visible
invariant that the change applied with no Controller restart.

### 10.4 NATS client (`NatsConfig`)

- **Watch channel**: `watch<Arc<NatsConfig>>`, owned by the NATS transport wrapper.
- **validate**: parse URL; attempt a connect on a probe `async_nats::Client` with a 3-second
  timeout, close it.
- **apply**: open new client; re-register every subscription the wrapper tracks; swap the
  wrapper's `Arc<async_nats::Client>` atomically; drop old client (its in-flight in-bound
  messages complete on their original subscriptions before the old client is GC'd).
- **revert**: re-open old client (same URL stored on the snapshot), restore subscriptions.
- **health_check**: call `Client::flush().await` on the new client with a 5-second
  `tokio::time::timeout`. A successful flush proves the client is connected to the new URL
  and that the protocol round-trip works. This avoids inventing a controller-internal
  `$INBOX.*` echo responder, which the current codebase does not register.
- **In-process** — no reexec.

### 10.5 TLS / CA snapshot (`TlsConfig`)

- Reuses the existing `CaSnapshotReceiver` plumbing. `Reloadable` impl on the snapshot
  wrapper detects `cert_path` / `key_path` changes by stat + digest comparison.
- **validate**: read both files, parse cert + key, check key matches cert public key.
- **apply**: publish new `CaSnapshot` via existing `watch::Sender`.
- **revert**: re-publish prior `CaSnapshot`.
- **health_check**: terminate one TLS handshake against the in-process listener with the
  new cert (an internal client connects to `https://127.0.0.1:<https_addr>`).
- **In-process** — no reexec.

### 10.6 Plugin registry (`PluginRegistry`, `NotificationPluginRegistry`)

- **Watch channel**: per-plugin-config slot in DB; the registry holds a
  `watch::Receiver<Arc<HashMap<PluginTypeId, PluginConfig>>>` driven by the reconciler.
- **validate**: for each changed config, instantiate a throw-away plugin via the existing
  constructor (`from_config(new)` or equivalent) and discard it. Constructor failure is
  the validation signal; no new trait method.
- **apply**: for each changed config, call the same constructor again to build the
  production instance and swap the registry's `Arc<dyn Plugin>` atomically. We accept the
  double-construct because plugin constructors are required to be O(small) per the rule
  below.
- **revert**: re-swap the prior `Arc<dyn Plugin>` (held in the snapshot).
- **health_check**: the constructor succeeded both in validate and apply; no further
  liveness probe is performed at the registry level. The plugin trait surface does not
  currently include a `Plugin::health` method, and adding one is outside the scope of this
  spec. If a plugin fails on first use after the swap, the failure surfaces through the
  normal plugin error path, not through the reload watchdog.
- **In-process** — no reexec.

`docs/development/plugin-guidelines.md` gains a rule: **plugin constructors must be O(small);
expensive resources (HTTP clients, SMTP connections, JIT-compiled regexes) live in a
module-level `OnceCell` or shared `Arc<…>` owned outside the plugin instance**. The rule
keeps drop-and-recreate cheap and predictable.

### 10.7 Audit dispatcher (`AuditConfig`)

- **Watch channel**: `watch<Arc<AuditConfig>>` owned by `AuditLogDispatcher`.
- **validate**: filter is one of the known values; retention_days ≥ 0.
- **apply**: swap dispatcher's filter; reconfigure retention sweeper interval.
- **revert**: restore prior values.
- **health_check**: verify the dispatcher's send-channel is open (i.e., the consumer task
  is still draining). The dispatcher's `dispatch()` is fire-and-forget by design; we do
  not attempt an end-to-end "audit row appeared in DB" probe because that would race the
  background flush. Channel-open-and-not-saturated is the strongest signal available
  without changing the dispatcher contract.
- **In-process** — no reexec.

### 10.8 Zeroconf advertiser (`ZeroconfConfig`)

- **validate**: parse `url`, `pki_addr`.
- **apply**: re-publish mDNS records (`_uptrakit._tcp` family).
- **revert**: re-publish prior records.
- **health_check**: query our own mDNS responder for the published record.
- **In-process** — no reexec.

### 10.9 Embedded services (`EmbeddedServicesConfig`)

This Reloadable handles only **in-place tunables** for already-enabled embedded Services:
per-service heartbeat interval and log-level override. Adding or removing an embedded
Service from the topology is in the irreversibly-bound set (§11) and goes through reexec
exclusively.

- **Watch channel**: `watch<Arc<EmbeddedServicesConfig>>` owned by the embedded-service
  supervisor.
- **validate**: heartbeat interval ≥ 1 second; log level is one of the known
  `tracing::Level` values; the set of enabled services equals the boot-time set (any
  topology delta fails validation and triggers the reexec branch in §11 instead).
- **apply**: push the new heartbeat interval and log level into each running embedded
  service via the existing per-service control channels.
- **revert**: push the prior values back.
- **health_check**: each embedded service confirms it received the new tunables within 2
  seconds (it acknowledges through a `tokio::sync::oneshot` channel established at apply
  time).
- **In-process for tunables. Topology change ⇒ reexec.**

## 11. Reexec Path

When the validate phase observes that any irreversibly-bound key has changed, the
coordinator takes the reexec branch instead of the in-process apply path.

### 11.1 Irreversibly-bound key set (initial)

- `db.url`
- `master_key.path`
- `log.path`
- `embedded_services.*` (any change to which services run embedded)

Adding to this set requires an ADR amendment. The set is intentionally small. Future
candidates (e.g., changes to a Cargo-feature-gated subsystem) are added only after explicit
review.

### 11.2 Pre-reexec preparation

1. Validate the new TOML (full validate-all pass) inside the running process. Reexec only
   if validation passed — never hand a known-bad config to the child.
2. Mark the coordinator queue as drained / refusing new requests.
3. Emit `ConfigReloadRequested { source, reexec: true }` audit event and `flush()` the
   audit dispatcher (the parent will not exist long enough to flush asynchronously).
4. Clear `FD_CLOEXEC` on every listening socket that should be inherited.
5. Set `LISTEN_FDS`, `LISTEN_PID`, and `UPTRAKIT_REEXEC_GENERATION` in the child env.

### 11.3 The `exec()` call

Using `std::os::unix::process::CommandExt::exec()`:

```rust
// Build child argv from a known-good allowlist, not by passing through env::args().
// After this work the only flags the binary accepts are --config and --master-key-from
// (plus --version / --help / --check-config / --migrate-and-exit, none of which apply at
// reexec). Anything else in env::args() is either a legacy artefact or a malicious
// injection; in either case we refuse to propagate it.
let mut cmd = Command::new(current_exe_path);
cmd.arg("--config").arg(&config_path);
if let Some(mk) = master_key_arg {
    cmd.arg("--master-key-from").arg(mk);
}
cmd.env("LISTEN_FDS", listener_count.to_string());
// LISTEN_PID must equal the PID that will eventually call sd_listen_fds(). exec()
// preserves the PID across image replacement, so the parent's std::process::id() is
// also the child's PID — this is correct, not a bug.
cmd.env("LISTEN_PID", std::process::id().to_string());
cmd.env("UPTRAKIT_REEXEC_GENERATION", (current_generation + 1).to_string());
// Sockets to inherit are already FD 3..(3+N) per LISTEN_FDS protocol
let err = cmd.exec();  // never returns on success; returns io::Error on failure
```

The `listenfd` crate (new workspace dep) normalises `LISTEN_FDS` parsing in the child via
`ListenFd::from_env()`; each inherited listener is fetched with
`take_tcp_listener(idx)`. The parent does the inverse: it sets `FD_CLOEXEC = false` on the
production HTTPS and PKI listeners (and any other inherited sockets) before `exec()`. The
listenfd protocol numbers FDs starting at 3.

The previous graceful-restart mechanism in this codebase (`--reuseport`,
`--takeover-from`, `SIGUSR1`-based handshake) is **removed** in this release. The reexec
path replaces it. Operators who relied on `--reuseport` for rolling deploys move to
either (a) the new reexec path triggered by file edit + `SIGHUP`, or (b) external load
balancing across two Controller instances. The breaking-change list in §20 calls this
out.

The child boots:

1. Calls `ListenFd::from_env()`; if `LISTEN_PID == getpid()`, claims the inherited sockets
   via `take_tcp_listener(idx)`.
2. Reads the new TOML; full boot path otherwise unchanged.
3. Wires the inherited sockets into the new HTTPS / PKI listener subsystems.
4. After every in-process Reloadable's boot-time `health_check()` passes, calls
   `sd_notify("READY=1")` (no-op when `NOTIFY_SOCKET` is unset) and prints a literal
   `READY` line to stdout for non-systemd supervisors.

The parent:

1. After `exec()` is replaced by the child image; if `exec()` returns (failure), the parent
   aborts the reload with `ConfigReloadFailed { phase: Reexec, error }` and continues
   serving on the old config.
2. There is no parent-after-child to "reclaim sockets" because `exec()` replaces the
   process. The child takes over directly. If the child fails to come up, systemd (or the
   init system) restarts the binary; the binary then boots into a permanent failure mode
   only if the new TOML is fatally broken, which the Operator must fix out-of-band.

**Note on the "parent drains" model from earlier sketches**: with `exec()` (single process
replacement), the parent does not drain — its address space is replaced atomically. The
"drain" semantics apply only to the _socket_ layer: in-flight connections on listening
sockets do not survive the `exec()`, but established connections (accepted file descriptors
already pulled off the listener) are dropped along with the parent process state. For a
true parent-survives-child-drains pattern we would need a separate `fork()` step. That is
explicitly out of scope for this spec; the Operator-visible difference is that an HTTPS
client that was mid-request _at the moment of reexec_ may see one TCP-level reset and
retry. Clients (Agents, Dashboard, MCP) handle this through their existing reconnect/retry
loops; we audit-emit `ConfigReloadApplied { reexec: true }` so the Operator can correlate.

If this trade-off proves unacceptable in production, the upgrade path is fork-then-exec
with file-descriptor passing through `SCM_RIGHTS`, which preserves the parent's accepted
connections during drain. That work is captured in the Future Work section but is not in
scope here.

### 11.4 Watchdog on the child

The parent does not exist after `exec()`. The watchdog for the irreversibly-bound path is
therefore the init system (systemd `Restart=on-failure` + `WatchdogSec=…`, or equivalent).

On systemd hosts, the child calls `sd_notify("READY=1")` via the `sd-notify` crate (pure
Rust, new workspace dep, no C dependency) after every in-process Reloadable has passed its
`health_check()` at boot. systemd treats this as successful start. The same crate is used
to send periodic `WATCHDOG=1` pings if `WatchdogSec` is configured. On non-systemd hosts
(`NOTIFY_SOCKET` env var absent), `sd_notify` calls are no-ops.

For non-systemd platforms (FreeBSD, custom supervisors), the operator's supervisor reads
the Controller's stdout/stderr for a literal `READY` line that the binary emits in parallel
with `sd_notify`. The runbook documents both protocols. There is no parent-side status FD —
the spec's earlier mention of `UPTRAKIT_REEXEC_PARENT_STATUS_FD` is removed because there
is no parent process to read it after `exec()`.

If the child fails to start or fails its boot-time health checks, it exits non-zero. The
init system restarts it; the process continues to boot until either the new TOML is fixed
or the init system gives up (in which case the Operator's monitoring fires). The runbook
documents the recovery path.

## 12. Watchdog and Revert

For the in-process path (every reload that does not touch an irreversibly-bound key):

1. After every subsystem's `apply()` returns `Ok`, the coordinator spawns watchdog
   futures `tokio::time::timeout(subsystem.rollback_window(), subsystem.health_check())`.
2. All watchdogs run **concurrently** via `FuturesUnordered` after apply-all completes
   (not interleaved with apply, because some subsystems' health depends on others being
   already applied — e.g., the HTTPS listener health check requires that TLS has applied
   first). `FuturesUnordered` is used rather than `join_all(Vec<…>)` because the
   workspace denies `clippy::large_futures` (`workspace.lints.clippy`); the unordered
   stream keeps each future's memory cost flat.
3. **Atomic outcome** (consistent with §8.3): if **any** watchdog returns `Err` or its
   window elapses, the coordinator drains the remaining watchdogs (cancelling them) and
   reverts **all** participating subsystems in reverse application order. Per-subsystem
   partial-commit is not permitted; the simpler atomic rule wins over the dependency
   reasoning required to argue "subsystem X can stay on new config while Y reverts".
4. If `revert()` itself returns `Err`, emit a `system_alerts` row with severity
   `Critical` (the subsystem is now in an undefined state and an Operator must
   intervene).
5. After watchdogs resolve, the coordinator emits exactly one of:
   - `ConfigReloadApplied { sections, duration_ms, per_subsystem_ms, reexec: false }`
     (all healthy)
   - `ConfigReloadFailed { phase: Watchdog, subsystem, error }` plus one
     `ConfigReloadReverted` per reverted subsystem (all of them, by the atomic rule).

`per_subsystem_ms: BTreeMap<String, u64>` captures each Reloadable's measured
apply-plus-health duration so the hardcoded `rollback_window()` constants can be tuned
against real production telemetry without changing the constants speculatively.

Snapshots are dropped after the maximum `rollback_window()` across participating
subsystems has elapsed without an in-flight reload — i.e., the next reload always
starts from a clean snapshot slate.

## 13. Multi-Controller Propagation

This section covers the cluster picture; a single-node deployment can ignore it without
loss.

### 13.1 File-rooted sections

File-rooted sections are **local-only**. Each node has its own TOML file. Edits are not
propagated by uptrakit. Operators with multi-node deployments use external configuration
management (Ansible, Kubernetes ConfigMaps mounted into the container, Puppet, manual
SSH-and-edit, etc.). Each node reloads independently on file-watch / SIGHUP.

This is the same model as Postgres `postgresql.conf` or HAProxy: the per-node config file
is part of the node's deployment, not part of the cluster's shared state.

### 13.2 DB-rooted sections

DB-rooted sections continue to propagate cluster-wide through `settings_version` and (per
tenant) `global_version` counters. The `ConfigReconciler` task on every Controller polls
these counters at 2-second intervals; on bump, it determines the affected
`SettingKey`s, re-reads the affected DB section, and publishes the new section to the
appropriate `watch::Sender`, which fans out to the subsystem.

Existing infrastructure (`bump_revocation_version`, the per-tenant
`bump_settings_version(db, tenant_id)`, `bump_global_settings_version(db)`, the
`settings_version` table, the per-tenant `version` and `global_version` columns) is reused
unchanged. The reconciler reads from the same row(s) those functions write to. No new
helper is introduced; settings mutations call the existing per-scope helpers as they
already do today, and the reconciler observes the bump on its next poll tick.

Postgres `LISTEN/NOTIFY` is a forward-looking optimisation for latency-sensitive sections;
it is not part of this spec.

## 14. Concurrency: `If-Match` Optimistic Locking

Settings mutation endpoints (the routes under `crates/ui/web-api/src/routes/settings*.rs`
and `plugin_configs.rs`) currently do not require an ETag. Two Operators editing the same
Tenant's settings can therefore overwrite each other.

### 14.1 New requirement

Every settings-mutation route:

1. Computes an ETag from the relevant `settings_version` counter (or the per-row
   `updated_at` for per-row routes). The ETag is returned on `GET` and on every mutation
   response.
2. On mutation requests, requires the `If-Match` header. Missing header → 428 Precondition
   Required; stale value → 409 Conflict.

### 14.2 ETag format

`W/"settings-v{version}"` for whole-section ETags. `W/"setting-{key}-{updated_at_iso}"` for
per-row ETags. Weak ETag because we compare semantic equality, not byte equality.

### 14.3 Implementation location

The `If-Match` check lives in an axum extractor (`IfMatch<T>` typed wrapper) defined once
in `crates/ui/web-api/src/extractors/if_match.rs`. `T` is a marker type that implements
the trait:

```rust
#[async_trait]
pub trait EtagSource: Sized + Send + Sync + 'static {
    async fn current_etag(
        parts: &mut http::request::Parts,
        state: &AppState,
    ) -> Result<String, ApiError>;
}
```

`IfMatch<T>` implements `FromRequestParts<AppState>` by:

1. Reading the `If-Match` header (returning 428 if absent).
2. Calling `T::current_etag(parts, state).await` to load the current ETag — this method
   reuses `Parts` and the shared `AppState` instead of trying to invoke other extractors
   ad-hoc. Tenant resolution happens inside the `EtagSource` impl. **The current ETag is
   read from the `ConfigReconciler`'s `ArcSwap<HashMap<Scope, u64>>` cache**, not from
   a fresh DB query, so the extractor adds zero DB round-trips on the mutation path. The
   reconciler refreshes the cache on its 2-second poll cycle; an Operator who mutates
   then immediately re-reads on the same connection will see at most 2 seconds of
   staleness, which is well within the optimistic-locking window. Settings mutations
   themselves still bump `settings_version` synchronously; the reconciler picks up the
   new value on its next tick and the cache converges.
3. Comparing weak ETags semantically; returning 409 if stale.
4. Yielding the unwrapped client ETag value to the handler when valid.

`SettingsVersion`, `GlobalSettingsVersion`, and any per-row marker type implement
`EtagSource` once. Route handlers add a single `IfMatch<SettingsVersion>` argument; no
per-route boilerplate. `tower-http` 0.6 does not ship a ready-made conditional-request
middleware, so this is hand-rolled — but it is hand-rolled exactly once.

### 14.4 Backward compatibility

There is no client-side backward compatibility concern for this work because the only
clients are (a) the in-tree Dashboard (which we update in lockstep) and (b) the CLI (same).
Any third-party API client that wrote settings without `If-Match` will start receiving 428
on those calls. That is the intended behaviour.

## 15. Governance: Audit, Permissions, Dashboard Surface

### 15.1 New audit event variants

Per the existing `AuditEvent` enum (semantic-audit-logs-v2):

```rust
#[non_exhaustive]
pub enum AuditEvent {
    // ... existing variants ...
    ConfigReloadRequested {
        source: ReloadSource,
        file_path: Option<PathBuf>,
        changed_sections: Vec<String>,
        reexec: bool,
    },
    ConfigReloadApplied {
        sections: Vec<String>,
        duration_ms: u64,
        /// Per-subsystem apply+health timing, used to tune rollback windows.
        /// Keys are constrained to `Reloadable::name()` returns (`&'static str` values)
        /// so the key space is closed by construction; no per-instance / per-plugin
        /// cardinality. New subsystems are added by editing the per-section list in
        /// §10 of the graceful-reload spec.
        per_subsystem_ms: BTreeMap<String, u64>,
        reexec: bool,
    },
    ConfigReloadFailed {
        phase: ReloadPhase,
        subsystem: Option<String>,
        error: String,
    },
    ConfigReloadReverted {
        subsystem: String,
        reason: String,
    },
}

#[non_exhaustive]
pub enum ReloadPhase {
    Validate,
    Apply,
    Watchdog,
    Reexec,
    /// Catch-all for forward-compatible audit-log JSON. Wire-exposed via audit events
    /// and the `/api/v1/instance/config-state` response.
    Other(String),
}
```

For `SIGHUP` and file-watch triggers, the audit event records `actor =
"system:config_reload"`; for DB-version-bump triggers, the actor is the original
`AuditActor` from the settings mutation request that caused the bump (propagated through
the reconciler).

### 15.2 `system_alerts` rows

Validation failures, apply failures, watchdog reverts, and revert-of-revert failures all
write a corresponding row into `system_alerts` (already wired into the existing alert
banner). Severity mapping:

- Validate failure → `Warning`
- Apply failure → `Error`
- Watchdog revert → `Error`
- Revert-of-revert failure → `Critical`
- Reexec failure (pre-`exec()`) → `Critical`
- Crash loop after reexec → handled by the init system; the Operator sees the binary
  failing to start, which is the strongest possible signal.

### 15.3 New permissions

Two new variants added to the existing `Permission` enum (already `#[non_exhaustive]`
per snapshot):

- `ViewInstanceConfigState` — read-only. Grants access to `GET
/api/v1/instance/config-state` and the new Dashboard tab.
- `ManageInstanceConfigState` — privileged. Grants access to `POST
/api/v1/instance/config-reload/clear-degraded` (clears the coordinator's Degraded
  state, see §8.3). Should be tightly scoped — operationally equivalent to "ack the
  on-call page".

Both are additive to a `#[non_exhaustive]` enum, so no breaking change. Tests in
`crates/shared/types/src/permissions.rs` update their hardcoded variant-count assertions
to match the new size.

### 15.4 New endpoints

`GET /api/v1/instance/config-state` returns:

```json
{
  "file": {
    "path": "/etc/uptrakit/controller.toml",
    "digest": "sha256:…",
    "loaded_at": "2026-05-12T13:42:00Z",
    "pending_digest": "sha256:…", // null if file matches loaded
    "pending_detected_at": "2026-05-12T13:43:10Z"
  },
  "last_reload": {
    "started_at": "2026-05-12T13:30:00Z",
    "finished_at": "2026-05-12T13:30:01Z",
    "outcome": "applied", // applied | failed | reverted
    "duration_ms": 1023,
    "reexec": false,
    "sections": ["nats", "audit"]
  },
  "sections": {
    "network": {
      /* current rendered values, secrets redacted */
    },
    "nats": {
      /* … */
    },
    "audit": {
      /* … */
    },
    "db": { "url": "<redacted>", "pool_size": 16 }
  },
  "recent_events": [
    /* last 20 reload audit events */
  ],
  "coordinator_state": "idle",
  "degraded": null
}
```

`coordinator_state` is one of `idle`, `reloading`, `degraded`. When `degraded` is
non-null it carries `{ "failed_subsystems": ["nats", "audit"], "since":
"2026-05-12T13:31:42Z", "reason": "revert returned Err on nats: …" }`.

All secret-bearing fields are redacted (`<redacted>` strings). The endpoint requires
`ViewInstanceConfigState`.

`POST /api/v1/instance/config-reload/clear-degraded` is a privileged action that
re-runs `health_check()` across every Reloadable and, if all pass, returns the
coordinator to Idle. Body: `{}` (no parameters). Response: the updated
`config-state` payload. Requires `ManageInstanceConfigState`.

### 15.5 Dashboard tab

A new tab under **Settings → Instance Configuration** consumes the endpoint and renders:

- File status: digest, load time, "pending changes detected" badge if `pending_digest`
  differs from loaded.
- Last reload status with timestamp, duration, outcome, reexec flag.
- Per-section read-only render. Each section is a collapsible card. Secrets show
  `<redacted>`.
- Recent reload events table (last 20), filterable by phase / outcome.

There is no "Reload Now" button. The Dashboard cannot trigger a reload. This is a hard
constraint of the spec.

## 16. Failure Modes and Recovery

The runbook walks the Operator through the following matrix; this section is the canonical
source of the matrix and the runbook references it.

| Failure                                    | Operator-visible signal                                                                                                                                                                                                                                                                 | Recovery                                                                                                                                |
| ------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| TOML parse error                           | `ConfigReloadFailed { Validate }` audit; `system_alerts` Warning; log line; Dashboard "validation failed" badge on Instance Configuration tab                                                                                                                                           | Edit the TOML; reload (`SIGHUP` or save file).                                                                                          |
| Cross-section invariant fails              | Same as parse error                                                                                                                                                                                                                                                                     | Same.                                                                                                                                   |
| Subsystem `validate()` fails               | Same as parse error, with `subsystem` set                                                                                                                                                                                                                                               | Same.                                                                                                                                   |
| Subsystem `apply()` fails                  | `ConfigReloadFailed { Apply }`, `system_alerts` Error, partial revert audit chain                                                                                                                                                                                                       | Investigate logs; revert TOML or DB if needed.                                                                                          |
| Watchdog timeout / `health_check` fails    | `ConfigReloadFailed { Watchdog }`, revert audit, `system_alerts` Error                                                                                                                                                                                                                  | Investigate; the subsystem is back on old config.                                                                                       |
| `revert()` itself fails                    | Coordinator enters **Degraded** state (§8.3). Further reloads refused. `system_alerts` Critical. `GET /api/v1/instance/config-state` reports `coordinator_state: degraded` with failing subsystems.                                                                                     | Operator investigates; calls `POST /api/v1/instance/config-reload/clear-degraded` once subsystem health restored, or restarts manually. |
| Reexec validation passes but child crashes | systemd restart loop; binary keeps booting until it succeeds or stays down. **No in-product audit row for the post-exec crash** — the parent that would emit it no longer exists. Operator relies on system logs (`journalctl`) and on the absence of new `ConfigReloadApplied` events. | Fix TOML out-of-band (revert from VCS, edit on another node, etc.); init system picks up working config.                                |
| Concurrent edits (two Operators)           | 409 Conflict on the second writer                                                                                                                                                                                                                                                       | Re-fetch, re-apply, retry.                                                                                                              |

For the irreversibly-bound / reexec path, the failure mode of "child crashes" is the most
operationally significant. The runbook documents:

- How to recover when the only running node is in a crash loop: edit the TOML file on disk,
  run `uptrakit-controller --check-config /etc/uptrakit/controller.toml` from a shell, then
  let systemd restart the binary.
- How to roll forward on a multi-node deployment: file-driven config means each node is
  independent; a known-good node continues to serve traffic while the broken node is
  fixed.

## 17. Test Strategy

All tests follow `docs/development/testing.md` rules: `parking_lot` only in async paths,
no `unwrap()` in production but allowed in tests, `start_paused = true` for time-dependent
tests _without_ DB / pool timers, FK constraints on in tests, `EncryptedString::plaintext_for_test`
with the `testing` feature where applicable.

### 17.1 Per-subsystem unit tests

Each Reloadable gets its own unit-test module that:

- Constructs the subsystem against a synthetic `watch::Receiver<Arc<SectionConfig>>`.
- Pushes a new config and asserts the subsystem reflects the change.
- Pushes a deliberately-bad config and asserts `validate()` rejects it.
- Calls `revert()` after `apply()` and asserts pre-apply state restored.
- Times out a fake `health_check` and asserts the watchdog branch.

These tests run under `#[tokio::test(start_paused = true)]` for the timeout pieces. DB-pool
unit tests run without `start_paused` (SQLx pool timers fire prematurely when paused, per
snapshot rule).

### 17.2 Coordinator unit tests

`ReloadCoordinator` is exercised against a list of mock Reloadables that record their
phase transitions. Tests assert:

- Validate-all is called before any apply.
- Apply order matches the documented sequence.
- A single Reloadable returning `Err` from `apply` causes revert on every prior
  Reloadable in order.
- Multiple queued requests coalesce into a single cycle.

### 17.3 File-reload integration tests

- Write a TOML file to a `tempfile::TempDir`.
- Boot a Controller with `--config <tempdir>/controller.toml` and a SQLite in-memory DB.
- Mutate the file via atomic-rename.
- Assert that `ConfigReloadApplied` lands in the audit log within 2 seconds.

### 17.4 File-watch debounce tests

Atomic-rename the file three times in 200 ms; assert the coordinator runs exactly one
reload cycle.

### 17.5 DB-reload integration tests

- Boot with seeded `global_settings`.
- Mutate a `global_settings` row.
- Bump `settings_version`.
- Assert the affected subsystem's `apply()` runs within 3 seconds (2-second poll +
  budget).

### 17.6 Multi-controller propagation

Two Controllers share a SQLite DB file (or a Postgres test container in the integration
suite). Mutate `global_settings` via one Controller's HTTP API; assert the second
Controller's `ConfigReloadApplied` audit event lands within 5 seconds.

### 17.7 Reexec integration tests

Marked `#[ignore]`; run via the existing Docker-based system-integration test path
(`docker build -f docker/Dockerfile.test -t uptrakit-test:latest .` and
`cargo test -p uptrakit-integration-tests -- --ignored`):

- Boot a Controller with a TOML that has `db.url = "sqlite://…"`.
- Open an HTTP client TCP-keepalive connection.
- Mutate `db.url` to a new SQLite file path containing the same schema.
- `SIGHUP` the Controller.
- Assert the controller's PID changes (reexec actually happened) but the listening port
  has zero downtime measurable via the keepalive client.

### 17.8 Watchdog-revert integration

Inject a failing `health_check` on the NATS Reloadable; assert the coordinator reverts
**every** participating subsystem (atomic revert rule from §8.3 / §12), each in reverse
application order. Audit assertions: exactly one `ConfigReloadFailed { phase: Watchdog,
subsystem: "nats" }` plus one `ConfigReloadReverted { subsystem }` row per reverted
subsystem.

### 17.9 Concurrency tests

Two HTTP clients race to mutate the same `SettingKey` with stale `If-Match`. Assert
exactly one succeeds (200) and one fails (409).

## 18. Documentation Deliverables

Implementation is **not** complete until every item below has been authored. The PR(s)
must touch each file or explicitly note "no change required, because …" in the PR
description. The standards-snapshot file (`.superpowers/standards-snapshot.md`) is
**not** edited manually — it is regenerated by the next `/spec` run after the rules are
in their source files.

- `docs/adr/0008-graceful-reload-architecture.md` (NEW) — combined decision record for
  single TOML config, per-section watch propagation, reexec for irreversibly-bound keys.
- `CONTEXT.md` — add **Reload Coordinator**, **Reloadable**, **Config Section**,
  **Reexec**, **Irreversibly-bound key**, **Watchdog window**, **`ConfigReconciler`**
  glossary terms. _Avoid_: "graceful restart".
- `ARCHITECTURE.md` — new "Configuration & Graceful Reload" section describing the
  trigger sources, coordinator, per-section watch model, and reexec branch.
- `docs/development/coding-standards.md` — new subsection:
  - `Reloadable` trait is the canonical contract for hot-reloadable subsystems.
  - Subsystems publish state through `tokio::sync::watch<Arc<…>>`; consumers hold
    receivers in their constructor.
  - No `lazy_static!`/`OnceCell` of configuration in production code.
  - Plugin constructors must be O(small); expensive resources in shared `Arc`/`OnceCell`
    outside the plugin struct.
  - Table of every setting and which source it lives in (File vs DB).
- `docs/development/plugin-guidelines.md` — cheap-constructor rule with example.
- `docs/development/testing.md` — watchdog-revert + file-watch tempdir test patterns.
- `docs/development/quality-gates.md` — note that the new Docker-backed reexec integration
  test runs under the existing `cargo test -p uptrakit-integration-tests -- --ignored`
  gate.
- New: `docs/end-user/operator-runbook-reload.md` — `SIGHUP`, file-watch semantics, how to
  inspect reload state in the Dashboard, the failure-mode matrix from §16.
- `README.md` — short "Configuration" section pointing at the TOML file, the four CLI
  flags, the runbook.
- CLI help text in `crates/core/controller/src/main.rs` (and `controller-standalone`).
- OpenAPI specs under `crates/shared/openapi-client/src/` — add `ViewInstanceConfigState`
  permission, the new `/api/v1/instance/config-state` endpoint, the new audit event
  shapes.
- Frontend:
  - `frontend/src/lib/openapi/*` regenerated.
  - New Settings → Instance Configuration tab consuming the new endpoint.
  - Audit log view renders the four new audit event variants.
- **No migration guide.** This release is a hard break on the CLI surface (decision §3.1
  / §6.4); operators upgrade by producing a TOML file before the deploy. The runbook calls
  this out at the top.
- **No manual edit of `.superpowers/standards-snapshot.md`.** The new rules above land in
  their source files; the snapshot regenerates on the next `/spec` invocation.

## 19. ADR Cross-Reference

`docs/adr/0008-graceful-reload-architecture.md` (new, authored alongside this spec).
Captures:

- **Decision**: single TOML file as the boot-time config source; per-section
  `tokio::sync::watch<Arc<…>>` propagation; `Reloadable` trait; two-phase validate-all /
  apply-all with watchdog; `exec()`-based reexec for an explicit, small irreversibly-bound
  key set.
- **Status**: Accepted, 2026-05-12.
- **Context**: prior state of fragmented CLI / env / DB config sources; partial reactivity
  via `CaSnapshotReceiver`; Operator pain.
- **Alternatives considered and rejected**:
  - Best-effort per-subsystem reload (partial-state outcome rejected).
  - In-process pool swap for DB URL change (ABA hazards, multi-quarter refactor).
  - RPC-based reload control (bootstrap paradox; replaces nothing).
  - Splitting cluster config to NATS (consensus problem without consensus protocol).
- **Consequences**: hard CLI break; new direct workspace dependencies on
  `notify-debouncer-full` (file-watch with built-in debouncing; pulls `notify` itself as a
  transitive — no separate `notify` workspace entry), `listenfd`
  (`LISTEN_FDS`-protocol socket inheritance), `sd-notify` (systemd READY signalling, no-op
  on non-systemd), `toml` (read-only TOML parsing via serde; `toml_edit` rejected because
  the read-only use case does not need its mutation API); new permission; new endpoint;
  ongoing discipline required on the irreversibly-bound key set (set membership decisions
  are themselves ADR amendments). Removal of the existing `--reuseport` / `SIGUSR1`
  graceful-restart path and the 30-second `spawn_settings_reload` task; both are explicit,
  documented retirements in the CHANGELOG and runbook. Each new dep is gated through
  `cargo deny check`; licenses must match the existing allowlist
  (MIT/Apache-2.0/ISC/BSD-3/Unicode-3.0/Zlib/CDLA-Permissive-2.0/MPL-2.0).

## 20. Migration / Breaking Changes

This release is a **hard break** for any deployment that relies on CLI flags or environment
variables other than `--config` / `--master-key-from` / `--migrate-and-exit` /
`--version` / `UPTRAKIT_CONFIG` / `UPTRAKIT_MASTER_KEY_FROM`. The Operator must produce a
TOML file before upgrading. The runbook ships with a fully-commented example TOML covering
every key.

No automated migration tool. No deprecation window. No flag aliases.

Internal-API breaking changes:

- `AppState`'s direct fields for things like `audit_log_filter`,
  `audit_log_dispatcher`, etc. shift from "value owned by `AppState`" to "value behind a
  `watch::Receiver<Arc<…>>`". Call sites that previously read `state.audit_log_filter`
  now read `state.audit_log_filter.borrow().clone()` (or equivalent). This is a
  many-call-site mechanical refactor.
- Settings mutation routes gain mandatory `If-Match` headers.
- The four CLI flags above are the only ones the binary accepts.

Operational mechanisms removed in this release:

- The existing `--reuseport` / `--takeover-from` / `SIGUSR1` graceful-restart path is
  **removed**. The new reexec path (§11) does not fully replace it on one axis: the old
  mechanism used `SO_REUSEPORT` + parent-drains-while-child-serves to preserve **accepted
  TCP connections** across binary upgrades. The new `exec()`-based reexec preserves only
  the **listening** sockets; in-flight accepted connections (long-poll HTTP, MQTT, mTLS
  Agent links) are reset at reexec and reconnected by clients. This is a regression on
  the "accepted connection preservation" axis. We accept it because (a) every uptrakit
  client (Agent, Dashboard, MCP) already implements reconnect/retry, (b) reexec only
  fires on irreversibly-bound key changes — which are rare — and the in-process reload
  path (§10) handles the common case with zero connection loss, and (c) fork-then-exec
  with `SCM_RIGHTS` FD-passing is captured in §21 as the upgrade path if production
  experience shows the regression matters. Operators who used `--reuseport` for routine
  rolling deploys move to (a) the new SIGHUP-triggered in-process reload for non
  irreversibly-bound changes, or (b) external load-balancing across two Controller
  instances for hard-cutover deploys that need accepted-connection preservation.
- The existing 30-second `spawn_settings_reload` polling task and its
  `SETTINGS_POLL_INTERVAL` constant are **removed**. `ConfigReconciler` (§7.3) supersedes
  it at a 2-second cadence. The single-writer guarantee is preserved because the old
  task is deleted, not left running alongside.
- `SettingKey` variants for keys that move to TOML (`HttpsAddr`, `PkiAddr`,
  `TrustedProxies`, `RealIpHeader`, `Sans`, `ForwardedClientCertInfoHeader`,
  `ForwardedClientCertPemHeader`, `NatsUrl`, `ZeroconfEnabled`, `ZeroconfUrl`,
  `ZeroconfPkiAddr`, `AuditLogFilter` _at the global level_, `AuditLogRetentionDays` _at
  the global level_) are removed from the `SettingKey` enum. The corresponding rows in
  `global_settings` are deleted by a one-shot migration (`m20260512_000001_drop_file_keys`)
  that runs at boot. Per-tenant `audit_log.*` overrides remain in the per-tenant
  `settings` table and remain `SettingKey` entries; the migration does not touch
  per-tenant rows.
- Migration policy when a deleted DB row's value differs from the new TOML value: the
  TOML wins. The migration logs each dropped row for Operator visibility but does not
  attempt to merge.

## 21. Future Work (explicitly out of scope here)

- **Fork-then-exec with SCM_RIGHTS handoff**: preserves accepted connections across
  reexec. Considered for a future spec if the simple-exec drop of accepted connections
  proves operationally painful.
- **Postgres `LISTEN/NOTIFY`** for `settings_version` propagation: lower latency than
  2-second polling, no change to SQLite path.
- **TOML keys for the watchdog windows**: hardcoded constants today; expose if Operators
  ask.
- **Dashboard "Reload Now" button** with `ViewInstanceConfigState + ReloadInstance`
  permission: out of scope; current attack-surface trade-off is "no".
- **Cluster-wide TOML replication** via NATS: out of scope; users have external config
  management.
- **Out-of-process Service reload** (Agent, Agent-SSH, MQTT, Scheduler as separate
  binaries): each has its own restart story.

## 22. Open Questions

- **DB-pool tunables under a no-op URL change**: `pool_size` and `acquire_timeout_ms`
  reload in-place. If we later expose `min_connections` (or similar) we add it to the
  Reloadable's section. Default: keep the surface to `pool_size` + `acquire_timeout_ms`
  in this release.
- **Health probe identities**: the HTTPS listener's health probe connects to
  `127.0.0.1:<https_addr>`. On hosts where `https_addr = 0.0.0.0:8443`, that loopback
  probe is fine. On hosts where it is `192.0.2.5:8443` (specific NIC), the loopback probe
  fails. The probe should fall back to using the actual `addr` if `127.0.0.1` is not in
  the bind set. — To resolve at impl time.
- **`fcntl` advisory lock on the master key file**: confirm semantics across Linux,
  macOS, FreeBSD before reexec landing.

These are deliberately deferred to implementation; none of them change the spec's shape.

## 23. Glossary (final)

Quoted verbatim into `CONTEXT.md` on implementation:

- **Reload Coordinator** — single Tokio task that serialises reload requests, runs
  two-phase validate-then-apply, drives the watchdog, commits or reverts. Exactly one per
  Controller process. _Avoid_: "config manager", "reload manager".
- **Reloadable** — long-lived subsystem implementing the `Reloadable` trait, participating
  in reload.
- **Config Section** — logical grouping of settings whose lifetime is bound together for
  reload; one `Arc<SectionConfig>` and one `watch::Sender` per section.
- **Reexec** — in-place process replacement via `exec()` with inherited listening sockets;
  used for irreversibly-bound key changes. _Avoid_: "graceful restart".
- **Irreversibly-bound key** — configuration key whose change cannot be applied to a
  running process without compromising correctness, safety, or operability. Current set:
  `db.url`, `master_key.path`, `log.path`, embedded-services topology. Set membership
  changes are ADR amendments.
- **Watchdog window** — per-subsystem time budget within which a newly-applied
  configuration must pass `health_check()`. Default values are constants in
  `uptrakit-config-reload::defaults`.
- **`ConfigReconciler`** — Tokio task that polls `settings_version` for bumps and enqueues
  reload requests for affected DB sections.
