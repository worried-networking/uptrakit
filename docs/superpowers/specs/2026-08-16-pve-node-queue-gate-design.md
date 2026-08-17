# PVE-Node-Aware Update Queue Promotion — Design

Date: 2026-08-16
Status: Draft (pending review)

## 1. Problem

Pre-update protection for Proxmox guests runs a vzdump backup (or snapshot) on the PVE node before the
agent is contacted (`crates/plugins/infrastructure/proxmox/src/update_protection.rs`). PVE serializes
vzdump per node internally, but the controller does not: when N containers on the same PVE node are
updated together (typically via a batch), every started update issues its vzdump immediately. PVE queues
them; our task poll (`client.rs` `wait_for_task_completion`, 2 s poll against the policy timeout) burns the
900 s backup budget waiting for earlier backups, and later updates fail before the agent is ever contacted.

The queue promoters (`Queued → Pending`) only check the per-host slot
(`uix_update_history_host_active`); nothing anywhere checks PVE-node contention — including the
direct-to-`Pending` enqueue path (`trigger_update_for_host`), which is exactly what N independent
per-host triggers hit. There is also no user-visible explanation of _why_ a row waits.

## 2. Goals

- Never run two controller-initiated backups on the same PVE node concurrently, at any controller count
  (multi-controller cluster behind a shared DB is a supported deployment —
  `docs/development/cross-controller-comm.md`).
- Detect externally started vzdumps on the node and wait for them too (best effort).
- Make every wait reason visible to the end user (REST + web UI), for **all** wait causes, not only
  the node gate.
- Batch and non-batch updates queue and promote identically with respect to the gate.
- Waiting rows drain without user action once the blocker clears. Host-slot waits drain event-driven
  (existing promoters); node-gate waits drain via the periodic sweeper by design (no release
  notification — worst-case added latency is one sweep interval per serialized backup).

## 3. Non-goals / out of scope

- **Cancellation** of Queued (or any) update rows. Waits are indefinite by design decision; cancellation
  is explicitly out of scope.
- **Promoter unification** (merging the batch and non-batch promoters into one code path). Both promoters
  gain the same gate via a shared decision function, but structural unification is deferred to a separate
  spec.
- **Protection-status CAS hardening.** `write_pre_update_protection_status`
  (`crates/ui/web-api-queries/src/queries/update_dispatch.rs:537`) and `fail_before_agent_dispatch`
  (`:567`) lack status guards and can overwrite terminal rows (e.g. flip `Interrupted → Failed` after a
  reap); the replay predicate treats `NULL` as "protection never started"
  (`crates/ui/web-api/src/routes/service_ws/handler/updates/replay.rs:45`) which a plugin returning
  `success(None)` corrupts. These are pre-existing defects orthogonal to queueing; they go to a separate
  hardening spec. This spec only depends on: terminal status frees the node slot (true today — the reaper
  terminalizes wedged `InProgress` rows).
- Gating **snapshot-mode** protection. Snapshots take a per-guest lock only, complete in seconds, and have
  a 120 s budget; only backup mode contends per node.
- MQTT / Home Assistant exposure of wait reasons.

## 4. Design overview

Five cooperating pieces, all DB-authoritative so they are correct under multi-controller clusters:

1. **Typed protection status.** `PreUpdateProtectionStatus` enum replaces free-form strings at all
   read/write boundaries; the batch path starts stamping `InProgress` like the orchestrator path already
   does. Claim release keys off this phase.
2. **`pve_node_claims` table.** One row per busy PVE node, unique on `(plugin_config_id, proxmox_node)`.
   The claim is acquired **at protection start** — in the same `begin_immediate()` transaction as the
   `Pending → InProgress` CAS plus the `InProgress` protection stamp — because that is the moment the
   node resource is actually consumed. A unique violation rolls the transaction back and leaves the row
   `Pending`; the sweeper retries it. Acquiring at protection start (not at queue promotion) means the
   claim is only ever held by an `InProgress` row, so the existing update reaper's 125 min bound covers
   every orphan; a claim acquired earlier could be wedged forever by an agent-offline row stuck in
   `Pending`, which the reaper never touches.
3. **Single wait-decision function.** `evaluate_promotion` is the one choke point deciding
   Proceed vs Blocked(reason). It is **advisory** for queueing/visibility (the claim insert is the
   authoritative gate) and is called by the enqueue path, both event-driven promoters, the
   protection-start path, and the sweeper.
4. **`update_wait_reasons` sidecar table.** Blocked outcomes are upserted here; a row's reason is
   **deleted in the same transaction as the successful transition it was blocking** (queue promotion or
   protection start) and cleaned up on terminal transitions, with the sweeper as backstop. REST enriches
   waiting rows from it; an SSE admin event invalidates the UI.
5. **Wait sweeper.** A periodic loop (default 15 s, TOML-configurable, hot-reloadable) with three duties:
   (a) re-evaluate every host with `Queued` rows and promote where possible — the drain path for cases
   the event-driven promoters miss (e.g. a batch item reaped mid-protection never advances its batch
   today, because `execution_owner_service_id` is still NULL — `crates/ui/web-api/src/update_reaper.rs:104`);
   (b) re-attempt protection start for `Pending` rows whose protection never started and whose agent is
   connected to this controller — the retry path for node-busy and external-vzdump waits (idempotent:
   the `Pending → InProgress` CAS guarantees a single winner, so respawning is safe, mirroring the
   reconnect-replay respawn in `replay.rs:45`); (c) janitor — collect orphaned claims and stale reason
   rows.

Non-PVE hosts, PVE hosts without a `proxmox_host_mappings` row, and hosts whose effective protection mode
is `do_nothing`/`snapshot` never touch the claim table; for them the gate reduces to the existing per-host
check.

**Rejected alternative — queued-aware task deadline.** The minimal fix for the timeout symptom alone is
to make `wait_for_task_completion` not count PVE-side lock-wait time against the backup budget (vzdump
logs `trying to get global lock - waiting...` until it acquires; `wait_for_task_completion_with_logs`
already exists). One-file change, no tables, no sweeper. Rejected because it only stops the timeout: all
N updates still go `Pending` and sit inside long-running protection phases with zero user-visible
explanation (goal 3 unmet), agent-side work still starts in arbitrary order, and the fix hinges on
scraping a log line PVE does not guarantee stable. The chosen design is bigger because the requirement
is queue behavior + wait visibility, not just a longer timeout.

## 5. Components

### 5.1 `PreUpdateProtectionStatus` enum

New enum in `uptrakit-shared-types` (next to `PluginRole`/`UpdateStatus`):

```rust
pub enum PreUpdateProtectionStatus { InProgress, Protected, Failed, Skipped }
```

(Sketches here and in 5.3 omit derives; both new public enums carry `#[non_exhaustive]` per the
workspace convention for new enums.)

- `FromStr` with a typed `ParsePreUpdateProtectionStatusError` + `Display`, per the coding-standards
  string-conversion rule. Serialized DB/API strings stay exactly `"in_progress"`, `"protected"`,
  `"failed"`, `"skipped"` — **no migration of existing data, no column type change**. The DB column and
  the wire-visible REST field remain strings; the enum lives at the boundaries.
- All writers (`set_inprogress_for_orchestrator` — `update_dispatch.rs:484`,
  `write_pre_update_protection_status` — `:537`) and readers (replay predicate, REST response mapping —
  `web-api-queries/src/queries/update_history.rs:76`) go through the enum. In the proxmox plugin this
  replaces the two `STATUS_*` constants (`update_protection.rs:32-33`) **and** the raw
  `"failed"`/`"skipped"` string literals scattered through the same file — the literal sites outnumber
  the constants; plan sizing should expect ~10 call sites.
- `Option<PreUpdateProtectionStatus>` keeps `None` = "protection never started" (replay semantics
  unchanged).
- **Batch path stamping**: `dispatch_next_queued_for_host`
  (`crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs:121`) currently runs protection
  without ever writing `in_progress`. It now stamps `InProgress` before invoking protection and the
  outcome after, identically to the orchestrator path. This is required for claim acquisition/release
  (5.2) and makes the protection phase uniformly visible to the reaper/replay machinery.

### 5.2 `pve_node_claims` table

Migration `m20260816_000001_pve_node_claims` (pattern:
`m20260513_000006_oauth_controller_instances.rs`; conventions per
`docs/development/database-migrations.md` — `.timestamp()` columns, FK indexes, new-table checklist).

| Column                | Type           | Notes                                           |
| --------------------- | -------------- | ----------------------------------------------- |
| `id`                  | uuid PK        | UUIDv7                                          |
| `tenant_id`           | uuid           | FK `tenants`, indexed; entity is `TenantScoped` |
| `plugin_config_id`    | uuid           | FK `plugin_configs`                             |
| `proxmox_node`        | text           | node name from `proxmox_host_mappings`          |
| `update_history_id`   | uuid           | FK `update_history`, **unique**                 |
| `claimed_at`          | timestamp      |                                                 |
| `submit_attempted_at` | timestamp NULL | stamped immediately **before** the vzdump POST  |
| `pve_upid`            | text NULL      | vzdump task UPID, stamped once the POST returns |

Unique index on `(plugin_config_id, proxmox_node)` — the node slot. Unique index on
`update_history_id` — one claim per update.

Lifecycle:

- **Acquire — at protection start, not at queue promotion.** Both protection entry points (the
  orchestrator's `set_inprogress_for_orchestrator` CAS and the batch path's new equivalent stamping,
  5.1) are wrapped in a `begin_immediate()` transaction that performs: `Pending → InProgress` CAS +
  protection `InProgress` stamp + claim INSERT. A unique violation on the node slot rolls the whole
  transaction back: the row stays `Pending`, protection has not started, and the orchestrator exits;
  the sweeper retries (duty b). The `PveNodeBusy` reason is then recorded **in a fresh transaction
  after the rollback — never inside the failed one**: on Postgres a unique violation aborts the
  transaction (`25P02`, every later statement errors), so sharing the transaction would silently drop
  the reason on Postgres while working on SQLite. Unique-violation detection uses
  the typed helper `uptrakit_shared_db::db_error::is_unique_constraint_violation` — **not** the local
  string-matching in `update_triggers.rs:84-93`, which that helper exists to replace.
  This is new structural work: today both protection entry points run the CAS as a bare
  `update_many().exec(db)` with no surrounding transaction, and a lost claim must abort by rolling back
  a fresh transaction per attempt (retry = new transaction, never a loop inside one).
  The attempt has **three distinct outcomes**, and they must not be conflated: claim unique violation ⇒
  rollback, `PveNodeBusy` reason (fresh tx), sweeper retries; **CAS matched 0 rows** (another
  orchestrator or the sweeper's respawn already took the row — expected under the idempotent-respawn
  design) ⇒ rollback and exit **silently** — no reason row, no warn-level noise, since the row is
  legitimately `InProgress` elsewhere and a spurious "backup running on node X" reason would stick
  until the sweeper backstop; any other DB error ⇒ normal error path.
- **Submit-intent stamping.** After the acquire transaction commits, the protection caller stamps
  `submit_attempted_at` in its own DB write immediately **before** issuing the vzdump POST, and stamps
  `pve_upid` when the POST response returns the UPID. `POST /nodes/{node}/vzdump` forks the task
  server-side before the response arrives, so a lost response (client timeout, proxy reset) leaves a
  running vzdump with `submit_attempted_at` set and `pve_upid` NULL — the stamp is what lets release
  logic treat that case fail-closed instead of freeing the node under a live backup.
- **Release — claim-state-driven, decided by the protection caller only.** The protection code
  (orchestrator path: `run_protection_and_dispatch` in
  `crates/ui/controller-core/src/update/controller.rs`; batch path: after its inline protection call) is
  the sole non-janitor release site: it writes the protection outcome **and** the claim disposition
  together. The claim is DELETEd only when the caller can prove no vzdump from this claim is running:
  either the submit was never attempted (`submit_attempted_at` NULL — covers `Skipped`, mode-resolution
  exits, and pre-submit `Failed`), or the poll observed the UPID reach a stopped state (normal
  `Protected` completion, and post-submit `Failed` where the task itself ended). In every other case —
  poll timeout (`backup_timeout_seconds` elapsed with the task still running), lost submit response,
  poll error of unknown state — the claim is **retained** and the janitor owns it; releasing there
  would re-enter the exact double-vzdump failure this spec prevents. Blind terminal-status writers
  (`fail_before_agent_dispatch` and any other query-layer path that terminalizes a row without
  protection context) must **not** touch claims at all — they cannot distinguish "vzdump still running"
  from "never started"; anything they leave behind is the janitor's, whose rule below is complete.
  The agent-update phase does **not** hold the node.
- **Orphan collection (sweeper duty c)**: for claims whose `update_history` row is terminal
  (`Completed`/`Failed`/`Interrupted`) or whose protection status is no longer `InProgress`, one
  submit-state rule covers the timeout-retained, lost-response, and crashed-controller cases:
  - `submit_attempted_at` NULL ⇒ no vzdump was ever submitted — DELETE immediately.
  - `pve_upid` set ⇒ DELETE once the UPID reports stopped (own-task status probe via `PveNodeGate`,
    GET `/nodes/{node}/tasks/{upid}/status`; a failed probe keeps the claim — bounded fail-closed,
    unlike the fail-open admission probe, because premature release risks the double vzdump), or
    unconditionally once `claimed_at` exceeds the 7500 s cap.
  - `submit_attempted_at` set, `pve_upid` NULL (submit response lost — a vzdump may be running with no
    UPID to poll) ⇒ fail-closed: hold until the 7500 s cap. Where the admission probe is usable
    (`Sys.Audit` present, 5.3), the janitor may release earlier once the node task list shows no
    running vzdump for the mapped VMID started at or after `claimed_at`.

  A controller that dies mid-backup is thus bounded by the existing reaper (125 min → `Interrupted`)
  plus at most one sweep — and if its vzdump outlives the row, the UPID probe keeps the node held until
  the task actually stops. No heartbeat/lease-timeout machinery: unlike `scheduled_tasks` claims,
  liveness here is already delegated to the update reaper, so the claim table stays dumb. This
  divergence from `docs/architecture/scheduler.md` §HA Claim Mechanism is deliberate and documented in
  the ADR.

**Safety invariant — `backup_timeout_seconds` must not exceed the reaper bound.** Orphan collection is
only safe because a reaped (`Interrupted`) row's vzdump is presumed dead. `backup_timeout_seconds` is an
operator-settable protection-policy field (default 900) with no upper bound today; a value above the
reaper bound (7500 s) would let the reaper terminalize a row whose vzdump is still running, the janitor
free the node, and a second concurrent vzdump start — the exact failure this spec prevents. Policy
writes therefore validate `backup_timeout_seconds ≤ 7500` (reject, not clamp — loud per the config
philosophy), and the ADR records the invariant so a future reaper-bound change re-examines it.

Whether protection start needs a claim is resolved **before** the transaction (read-only): load the
`proxmox_host_mappings` row for the host, resolve the effective protection policy via
`ProxmoxProtectionStore::load_effective_policy(tenant_id, software_item_id, plugin_config_id)`
(`crates/plugins/infrastructure/proxmox/src/protection_store.rs` — the dedicated
`proxmox_protection_default`/`proxmox_protection_item_override` tables, **not** the generic
`resolve_effective_config()` plugin-config merge), and require mode = backup. No mapping / no proxmox
detect-role assignment / mode ≠ backup ⇒ no claim. Config changes between resolution and commit are
tolerated (worst case: one conservative extra claim, or one unprotected run that PVE's own serialization
still tolerates).

**No network I/O inside the transaction.** The `begin_immediate()` transaction contains exactly the
CAS + protection stamp + claim INSERT and commits before any PVE call. Policy resolution and the
external-vzdump probe run before it; the vzdump submission and task poll run after it. An IMMEDIATE
transaction holds the SQLite writer lock — a PVE HTTP round-trip inside it would stall every writer in
the controller.

**Crate boundary**: `uptrakit-web-api-queries` depends on the plugin **registry**, never on the proxmox
plugin crate directly (`ci/check_plugin_semantic_boundary.py` gates this). Both plugin-touching reads —
the policy resolution above and the external-vzdump probe (5.3) — go through one registry-side trait
(working name `PveNodeGate`), implemented by the proxmox plugin and dispatched via the registry, so no
`web-api-queries` code names a plugin type. The trait doubles as the test seam.

Known limitation (documented, accepted): two `plugin_configs` pointing at the same physical PVE
cluster produce distinct claim keys and can double-book a node. Mappings are per-config by model
(unique index `uix_proxmox_hm_config_node_vmid` on `(plugin_config_id, proxmox_node, proxmox_vmid)` —
`crates/plugins/infrastructure/proxmox/src/controller_migration.rs`), so the gate follows the same
scoping.

### 5.3 Wait-decision function

`evaluate_promotion(host_id, …)` in `uptrakit-web-api-queries` (next to the promoters' query code).
Outcome:

```rust
enum PromotionDecision { Proceed, Blocked(WaitReason) }

enum WaitReason {
    HostBusy,                                  // existing per-host slot taken
    PveNodeBusy { node: String },              // claim table shows node taken
    ExternalBackupRunning { node: String },    // PVE task list shows a foreign vzdump
}
```

`WaitReason` lives in `uptrakit-shared-types` with `FromStr`/`Display` (stable snake_case strings; node
carried separately — see 5.4). The function is **advisory**: it exists to produce accurate wait reasons
and to avoid pointless transitions; mutual exclusion is enforced only by the claim INSERT (5.2) and the
existing `uix_update_history_host_active` unique index. Callers:

- `trigger_update_for_host` (`update_triggers.rs:113`) — enqueue path. Today: host free ⇒ insert
  `Pending` directly. Now additionally: host free **but node blocked** ⇒ insert `Queued` + reason, so
  N independent per-host triggers against one node stack up in the queue instead of all going `Pending`
  at once. (Even if one slips through — advisory check — protection start still serializes.)
- `dispatch_next_queued_update_with_notifier` (non-batch promoter,
  `crates/ui/web-api/src/routes/service_ws/handler/updates/dispatch.rs:268`).
- `dispatch_next_queued_for_host` (batch promoter, `update_batches/dispatch.rs:121`).
- The protection-start transaction (5.2) — evaluated immediately before attempting the claim, so the
  recorded reason matches the authoritative outcome.
- The sweeper (5.5).

**External vzdump probe.** When the candidate needs a backup-mode claim and the claim slot is free, the
decision function consults the PVE API (new `client.rs` call: GET `/nodes/{node}/tasks?source=active`,
filtered to running `vzdump` tasks) before proceeding. A running foreign vzdump ⇒
`Blocked(ExternalBackupRunning)`. Probe results are cached per `(plugin_config_id, node)` for 5 s,
following the `AccessEngine` cache pattern exactly: a bounded-capacity `moka` cache (`moka` is already a
workspace dependency) storing `(fetched_at, result)`, with staleness checked first-party at read time —
**not** moka's native `time_to_live`, whose quanta-based expiry clock is unreachable by
`tokio::time::advance` and therefore untestable (the `AccessEngine` doc comment records this trap). At
a 15 s sweep cadence this is **burst dedup only** — it collapses the N same-node lookups within one
tick or one batch-enqueue burst into one API call; it never spans ticks, and plan sizing should not
expect more from it. The cache is per-controller and advisory only. **Probe failure fails open** (log
warn, proceed): an unreachable PVE API must not wedge the queue — the protection step itself will
surface the real error, and the per-node claim still bounds our own concurrency. The probe uses the
plugin's existing authenticated client (SSRF-safe resolver already in place).
**Privilege — the two probes differ.** The **admission** probe (task list,
GET `/nodes/{node}/tasks`) requires `Sys.Audit` on the node, which a minimal-privilege backup token
(`VM.Backup` + `Datastore.AllocateSpace`) does not imply — without it every probe 403s and, being
fail-open, goal 2 silently never works. The **release** probe (5.2's janitor,
GET `/nodes/{node}/tasks/{upid}/status`) queries the claim's **own** task: PVE demands `Sys.Audit`
only when the requester differs from the task owner, so a minimal backup token still bounds
timeout-retained claims by UPID-stopped rather than pinning every one to the 7500 s cap. A `403` on
either probe is **not transient**: it is logged at warn naming the missing privilege (rate-limited to
first occurrence per config); the admission requirement is documented in `docs/end-user/proxmox.md`
and checked by the plugin's connection test. A release-probe `403` leaves the claim bounded by the
7500 s cap only — the cap is deliberately **not** shortened in that case, because the vzdump has
already outlived `backup_timeout_seconds` and any shorter bound re-risks the double vzdump.

### 5.4 `update_wait_reasons` sidecar table

Migration `m20260816_000002_update_wait_reasons`:

| Column              | Type      | Notes                                   |
| ------------------- | --------- | --------------------------------------- |
| `update_history_id` | uuid PK   | FK `update_history` (CASCADE on delete) |
| `tenant_id`         | uuid      | FK `tenants`, indexed; `TenantScoped`   |
| `reason`            | text      | `WaitReason` discriminant snake_case    |
| `detail`            | text NULL | node name for the PVE reasons           |
| `evaluated_at`      | timestamp | last evaluation time                    |

Reasons apply to two waiting states: `Queued` rows (waiting for promotion) and `Pending` rows whose
protection has not started (waiting for the node). Rules:

- Written (upsert) only by real transition attempts that return Blocked — never derived at read time.
  The displayed reason therefore cannot drift from actual gating behavior.
- **Deleted in the same transaction as the successful transition it was blocking** — the
  `Queued → Pending` promotion (explicit user requirement) and the protection-start transaction — and
  by terminal-status paths. Sweeper backstop: delete rows whose update is neither `Queued` nor
  `Pending`-with-`NULL`-protection.
- Rationale for a sidecar table rather than nullable columns on `update_history` (the
  `recovery_hint`/`pre_update_protection_status` precedent): the sweeper re-stamps `evaluated_at` on
  every tick for every blocked row — putting that 15 s-cadence churn on the hot `update_history` table
  would mean constant writes to rows the rest of the system treats as effectively immutable outside
  status transitions; the sidecar keeps high-churn advisory state off the primary table and FK CASCADE
  gives free cleanup. The columns alternative was considered and rejected on churn, not on immutability
  dogma.

### 5.5 Wait sweeper

New module `crates/ui/web-api/src/queue_sweeper.rs`, shaped like `update_reaper.rs` (loop + thin glue;
logic in a `web-api-queries` module), spawned from `controller-runtime` boot next to the reaper
(`boot/serve.rs:196`, the `spawn_update_reaper` call).

Each tick:

1. **Promote**: find hosts having ≥1 `Queued` row (single grouped query — no per-host N+1), and for
   each with a free host slot run the same promotion path as the event-driven promoters. Blocked ⇒
   upsert reason.
2. **Retry protection**: find `Pending` rows with `NULL` protection status whose agent is connected to
   **this** controller (local `ServiceConnectionRegistry` check; each controller handles its own
   connections) and respawn orchestration for them. Within that candidate set, resolve which rows need
   a backup-mode claim — batched, not per-row: load the `proxmox_host_mappings` rows and effective
   policies for all candidate hosts with one `.is_in()` query each, or this duty becomes an N+1 against
   the mapping table every 15 s. **The per-node throttle applies only to claim-needing rows**: attempt
   the **oldest** claim-needing row per `(plugin_config_id, proxmox_node)` per tick (FIFO by UUIDv7
   `id`, the promoters' existing ordering). Rows that need no claim (non-PVE hosts, unmapped hosts,
   `snapshot`/`do_nothing` mode) contend for nothing and are respawned **unthrottled** — this duty is
   also the heal path for the pre-existing "agent connected but orchestrator died" gap and for reaped
   batch items, and throttling those to one per tick would turn a 200-row backlog into a ~50-minute
   drain over a node contention none of them are waiting on; their concurrency is already bounded by
   the per-host active slot. The filter-then-throttle order matters: selecting the globally-oldest row
   and then filtering would let a node whose oldest contender has an offline agent be attempted by
   nobody, indefinitely. Without any ordering, a row can lose every round forever (and with
   cancellation out of scope there is no escape hatch), and every tick pays N−1 pointless rollbacks per
   node. Fairness is therefore FIFO **per controller**: rows behind other controllers' connections
   progress on those controllers' ticks; cross-controller global FIFO is not guaranteed. Idempotent
   under concurrency: the `Pending → InProgress` CAS admits a single winner.
3. **Janitor**: collect orphaned `pve_node_claims`, delete stale `update_wait_reasons`.

The three duties are three separately-testable query/action functions sharing one loop and one interval;
duty 2 is the only one touching controller-local state. The ADR notes duty 2 is an independent
liveness concern riding the same cadence, so a future split is cheap.

The grouped queries are intentionally cross-tenant: they take `&DatabaseConnection` directly, **not**
`TenantDb`, following the two existing cross-tenant sweep precedents
(`scheduler-runtime::claim::find_due_tasks` — whose doc comment states the "across all tenants" intent —
and `update_reaper::reap_overdue_updates`). The new query functions carry a doc comment stating the
tenant-agnostic intent, matching `find_due_tasks`.

Multiple controllers may sweep concurrently; the CAS + claim unique index make that safe, and duplicate
reason upserts are idempotent.

**Interval configuration**: new TOML section in `uptrakit-config-reload`
(`crates/shared/config-reload/src/config/`, template: `zeroconf.rs`):

```toml
[updates]
queue_sweep_interval_seconds = 15   # default 15, must be ≥ 1
```

Wire-up per the crate's idiom: new section module + field on `RuntimeConfig`, `validate()` rejecting 0,
`warn_about_extras`, and a `RuntimeConfigDelta::Updates` variant that is **actually applied** through
the ADR-0008 reload coordinator. Concretely, mirroring the live exemplar `AuditDispatcherReloadable`
(`crates/core/controller-runtime/src/reload/audit.rs`), which owns its own private
`tokio::sync::watch` channel — **not** the `RuntimeConfigChannels`/`RuntimeConfigReceivers` pair in
`config-reload/src/channels.rs`, which is currently `#[expect(dead_code)]` with no wired senders: a
`QueueSweeperReloadable` registered with the coordinator implementing `validate` (interval ≥ 1 —
re-check at apply boundary), `apply` (send on the watch channel), `revert` (send the prior value), and
`health_check` (trivially healthy — the interval has no external binding). The sweeper reads the
interval from the `tokio::sync::watch` receiver each tick, so a live reload takes effect on the next
tick. (The reload taxonomy has no "silently ignored until restart" category — every delta variant is
actively applied — so the key is hot-reloadable rather than half-wired.) Classified
hot-reloadable in `docs/end-user/operator-runbook-reload.md`. This deliberately diverges from the
reaper's hardcoded `REAPER_INTERVAL` const because the sweep cadence directly trades PVE API load
against queue latency — an operator-visible knob; the ADR records this.

### 5.6 REST, SSE, UI

- `UpdateHistoryResponse` (`crates/shared/web-api-types/src/update_history.rs:52`) gains

  ```rust
  pub wait_reason: Option<WaitReasonResponse> // { reason, detail, evaluated_at }
  ```

  populated for waiting rows by joining the sidecar in `list_update_history`/`get_update_history`
  (`web-api-queries/src/queries/update_history.rs:172/:306`) — one batched join, no per-row lookups.
  `pre_update_protection_status` stays `Option<String>` on the wire (values now guaranteed by the enum).
  After the change: `./scripts/regen-api.sh`, commit `openapi.json` + `frontend/src/lib/api/generated/`;
  extend `uptrakit-openapi-client` accordingly.

- New `AdminEvent::UpdateWaitStateChanged { update_history_id, host_id }`
  (`crates/shared/wire/src/admin_events.rs:32` + `event_name()` arm). Emit predicate: row created, row
  removed, or the `(reason, detail)` tuple changed — **never** an `evaluated_at`-only re-stamp. The
  sweeper re-stamps every blocked row every tick, so a naive emit-on-upsert would produce a per-row SSE
  storm during exactly the N-container batch this feature targets. Frontend: add the member to the
  hand-maintained
  `AdminEventType` (`frontend/src/lib/sse.ts:230`). Run `./scripts/regen-asyncapi.sh` and commit if the
  golden file changes.
- Web UI: update-history list/detail render a wait-reason badge/tooltip on waiting rows
  ("Waiting: another update active on this host" / "Waiting: backup running on PVE node X" /
  "Waiting: external backup running on PVE node X"), live-updating via the SSE event. Reason-less
  waiting rows render **state-aware** copy: `Queued` without a reason (just enqueued, ≤ one sweep) shows
  "Queued — evaluating…"; `Pending` without a reason (just promoted, protection attempt imminent — the
  promotion transaction deletes the queue-wait reason, and a node-busy reason is only re-written if the
  protection-start attempt loses) shows "Preparing update…".

## 6. Concurrency and failure analysis

| Scenario                                                                      | Outcome                                                                                                                                                                                                                                                |
| ----------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Two controllers start protection for different hosts, same node, same instant | One claim INSERT wins; loser's transaction rolls back, row stays `Pending` with `PveNodeBusy`, sweeper retries.                                                                                                                                        |
| N per-host triggers, same node, near-simultaneous                             | Advisory check queues most; any that slip to `Pending` serialize at protection start. Never two concurrent vzdumps.                                                                                                                                    |
| Controller dies mid-backup                                                    | Claim belongs to an `InProgress` row; reaper terminalizes it (125 min) → janitor releases on UPID-stopped (or immediately if `submit_attempted_at` NULL), 7500 s hard cap. Documented recovery bound.                                                  |
| Backup poll timeout, vzdump still running on PVE                              | Claim retained (`pve_upid` set); janitor releases only on UPID-stopped or 7500 s cap — no second vzdump through the timeout path. Release-side probe failure fail-closed (bounded).                                                                    |
| vzdump submit response lost (network), task running                           | `submit_attempted_at` set, `pve_upid` NULL ⇒ protection caller retains the claim; janitor holds fail-closed to the 7500 s cap (earlier via task-list reconcile when `Sys.Audit` available). Blind terminal writers never release.                      |
| Agent offline, row stuck `Pending`                                            | No claim exists (claims are acquired at protection start) — node never wedged by unreachable agents.                                                                                                                                                   |
| External vzdump running                                                       | Probe blocks the transition; sweeper retries every 15 s until clear. Probe failure fails open.                                                                                                                                                         |
| Batch item reaped mid-protection                                              | Today the batch stalls (owner never set). Sweeper duty a re-evaluates the host and drains the batch's next `Queued` row.                                                                                                                               |
| Row enqueued between sweeps, event promoter fires                             | Event-driven path promotes immediately; sweeper is backstop only — no added latency in the common case.                                                                                                                                                |
| Restart                                                                       | All gating state in DB; first sweep (≤ 15 s) restores reasons, retries protection for locally-connected agents, collects orphans.                                                                                                                      |
| SQLite vs Postgres                                                            | Same code path: `begin_immediate()` (sole opener) + unique indexes; no isolation-level assumptions. One PG-specific rule: reason upsert after a lost claim runs in a fresh transaction (5.2) — PG aborts the violated one.                             |
| `backup_timeout_seconds` set above reaper bound                               | Rejected at policy write time (≤ 7500 s invariant, 5.2) — reaper can never terminalize a row whose vzdump may still run.                                                                                                                               |
| Starvation on a contended node                                                | Sweeper duty 2 filters to locally-connected agents, then attempts the oldest **claim-needing** contender per node per tick (FIFO by UUIDv7); non-claim rows respawn unthrottled. FIFO is per controller — global cross-controller FIFO not guaranteed. |

## 7. Testing

- Enum round-trips: `PreUpdateProtectionStatus`/`WaitReason` `FromStr`/`Display` including error cases
  (own logic, not serde-derive testing).
- Claim lifecycle (SQLite in-memory, real migrations): acquire inside the protection-start transaction,
  unique-violation on second acquire (via `is_unique_constraint_violation`), rollback leaves row
  `Pending` with protection `NULL`, release on each protection outcome, orphan collection for terminal
  rows. Non-vacuous: assert the violating insert _fails_ and the first claim survives.
- Three-outcome attempt handling: CAS-matched-0-rows exits silently — no reason row written, no claim
  left behind; distinguishable from the unique-violation path in assertions.
- Claim-disposition rule: `Failed`-on-poll-timeout with `pve_upid` set does **not** release; janitor
  releases on UPID-stopped, retains on probe failure, and hard-releases past the 7500 s cap;
  `submit_attempted_at` NULL deletes immediately; `submit_attempted_at` set + `pve_upid` NULL (lost
  submit response) is retained to the cap. Blind terminal writers (`fail_before_agent_dispatch`) leave
  claims untouched.
- `evaluate_promotion`: each Blocked variant + Proceed path; backup-mode resolution (no mapping ⇒ no
  claim; snapshot mode ⇒ no claim). Probe injected behind a trait seam so tests use a stub instead of a
  live PVE client (no upstream-HTTP behavior testing). Probe-cache staleness: because expiry is a
  first-party read-time check (5.3), the 5 s boundary is testable — cover fresh-hit (no second probe
  call) and stale-miss (re-probe) paths.
- Promotion transaction: reason row deleted atomically with the `Queued → Pending` CAS; protection-start
  transaction: reason row deleted atomically on success; on lost claim the reason upsert lands in a
  fresh transaction after rollback. The lost-claim path also runs in the Docker Postgres integration
  suite (`cargo test -p uptrakit-integration-tests --test database -- --ignored`) — the abort-on-violation
  divergence (§6) would ship green on SQLite-only tests.
- Policy validation: `backup_timeout_seconds > 7500` rejected at write time; boundary value accepted.
- Sweeper fairness: with two claim-needing `Pending` rows on one node, duty 2 attempts only the older
  one per tick; with the older row's agent disconnected, the younger locally-connected row is
  attempted (local filter precedes oldest-per-node selection); non-claim rows in the same tick are all
  respawned, unthrottled by the per-node rule.
- SSE emit predicate: `evaluated_at`-only re-stamp emits no `UpdateWaitStateChanged`; `(reason, detail)`
  change and create/remove do.
- Enqueue path: host free + node busy ⇒ `Queued` with reason (new branch in `trigger_update_for_host`).
- Batch path stamping: protection status visible as `in_progress` during batch protection.
- Sweeper query functions unit-tested directly (grouped host query, Pending-retry selection, janitor
  deletes); the loop stays thin glue like the reaper (constants-only assertions there). No
  `start_paused` on DB-touching tests.
- REST: TestApp harness — list/detail expose `wait_reason` for waiting rows, absent otherwise.
- Config: `[updates]` section parse, default 15, `0` rejected by `validate()`; delta applied via watch
  on live reload.
- Frontend: events store test for the new `AdminEventType` member; component test for the badge.

Audit: new write sites (`pve_node_claims`, `update_wait_reasons` upserts/deletes) are internal
bookkeeping — register as `skip` in `audit-catalog.toml` (the status CAS sites are already catalogued);
`cargo xtask audit-coverage-check` must stay green. `python3 ci/verify_db_access_policy.py` must pass
after the query modules are added.

## 8. Documentation deliverables

New ADR via `adrs new "PVE node concurrency gate for update protection"` — records the claim-table gate
at protection start, its deliberate divergence from the scheduler HA-claim pattern (no heartbeat;
liveness delegated to the update reaper), the `backup_timeout_seconds ≤ reaper bound` safety invariant,
the claim-state-driven release (submit-intent stamp; delete only on proof of no running vzdump;
admission probe fail-open, release probe fail-closed with the 7500 s cap; blind terminal writers never
touch claims), the sweeper's retry duty being an independent liveness concern sharing the cadence, and
the hot-reloadable sweep interval vs. ADR-0024's constant.

Updates (from a docs-tree sweep, not memory):

- `docs/architecture/update-history-entity.md` — status/queue sections, new tables, promotion +
  protection-start flow.
- `CONTEXT.md` — glossary: node claim, wait reason, wait sweeper.
- `docs/api/http-web-api.md` — `wait_reason` field, typed protection-status values.
- `docs/end-user/update-history.md`, `docs/end-user/update-workflow.md` — wait reasons narrative.
- `docs/end-user/proxmox.md`, `docs/development/proxmox-plugin.md` — node gate + external-vzdump wait;
  the `Sys.Audit` token privilege required by the probe; the `backup_timeout_seconds ≤ 7500` policy
  bound; plugin table additions.
- `docs/api/sse-events.md`, `docs/development/sse-events.md` — new admin event.
- `docs/api/batch-actions.md`, `docs/end-user/batch-actions.md` — second concurrency dimension note.
- `docs/architecture/scheduler.md` or the new ADR — cross-reference for the claim-pattern divergence.
- `docs/api/settings-runtime.md` — sweep interval is TOML-only.
- `ARCHITECTURE.md` — `[updates]` TOML section in the section list.
- `docs/end-user/operator-runbook-reload.md` — classify `updates.queue_sweep_interval_seconds` as
  hot-reloadable.
- `docs/adr/0024-update-liveness-and-interrupted-status.md` — no edit; superseding notes live in the new
  ADR.

No wire-protocol (service↔controller) payload changes ⇒ no `asyncapi.yaml` change expected beyond the
admin-event regen check above. No new external dependencies ⇒ no version pins (`moka` is already in
`[workspace.dependencies]`).

## 9. Deferred

- Promoter unification (batch/non-batch structural merge) — separate spec.
- Protection-status CAS hardening (`write_pre_update_protection_status` / `fail_before_agent_dispatch`
  status guards; replay-NULL semantics vs `success(None)`) — separate hardening spec.
- Update cancellation — out of scope by decision; revisit only on explicit request.
- Cross-config node identity (two plugin configs → same physical cluster double-booking a node).
- TOML key reference page (documentation gap noted during design; not created here).

## 10. Dependencies

- Predecessor: `uptrakit-spec-2026-08-12-network-timeout-safety` (Network & Timeout Safety).
  Reason: needs landed implementation, design-contingent — this spec skips a heartbeat/lease on node
  claims and delegates claim liveness to the update reaper (`crates/ui/web-api/src/update_reaper.rs`,
  `crates/ui/web-api-queries/src/queries/update_reaper.rs`), whose `Pending`-row handling that spec's
  M0.4/M1.8 rewrite; the 7500 s reaper bound (ADR-0024) is a load-bearing safety invariant here.
  Premature stage: plan writing onward. Wired: predecessor spec epic blocks
  `uptrakit-write-plan-2026-08-16-pve-node-queue-gate` and
  `uptrakit-spec-2026-08-16-pve-node-queue-gate`.
