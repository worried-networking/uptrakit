# PVE-Node-Aware Update Queue Promotion — Design

Date: 2026-08-16
Status: Draft (pending review)

## 1. Problem

Pre-update protection for Proxmox guests runs a vzdump backup (or snapshot) on the PVE node before the
agent is contacted (`crates/plugins/infrastructure/proxmox/src/update_protection.rs`). PVE serializes
vzdump per node internally, but the controller does not: when N containers on the same PVE node are
updated together (typically via a batch), every promoted update issues its vzdump immediately. PVE queues
them; our task poll (`client.rs` `wait_for_task_completion`, 2 s poll against the policy timeout) burns the
900 s backup budget waiting for earlier backups, and later updates fail before the agent is ever contacted.

The queue promoters (`Queued → Pending`) only check the per-host slot
(`uix_update_history_host_active`); they know nothing about PVE-node contention. There is also no
user-visible explanation of _why_ a row sits in `Queued`.

## 2. Goals

- Never start a backup-mode protected update while another controller-initiated backup holds the same PVE
  node, at any controller count (multi-controller cluster behind a shared DB is a supported deployment —
  `docs/development/cross-controller-comm.md`).
- Detect externally started vzdumps on the node and wait for them too (best effort).
- Make every queue-wait reason visible to the end user (REST + web UI), for **all** queue causes, not
  only the node gate.
- Batch and non-batch updates queue and promote identically with respect to the gate.
- Queued rows drain without user action once the blocker clears — event-driven plus a periodic sweeper.

## 3. Non-goals / out of scope

- **Cancellation** of Queued (or any) update rows. Queued waits are indefinite by design decision;
  cancellation is explicitly out of scope.
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
  terminalizes wedged rows).
- Gating **snapshot-mode** protection. Snapshots take a per-guest lock only, complete in seconds, and have
  a 120 s budget; only backup mode contends per node.
- MQTT / Home Assistant exposure of queue reasons.

## 4. Design overview

Five cooperating pieces, all DB-authoritative so they are correct under multi-controller clusters:

1. **Typed protection status.** `PreUpdateProtectionStatus` enum replaces free-form strings at all
   read/write boundaries; the batch path starts stamping `InProgress` like the orchestrator path already
   does. Claim release keys off this phase.
2. **`pve_node_claims` table.** One row per busy PVE node, unique on `(plugin_config_id, proxmox_node)`.
   Claim insert happens in the same transaction as the `Queued → Pending` CAS; a unique violation means
   "node busy — stay Queued". Hard mutual exclusion at any controller count.
3. **Single promotion decision function.** `evaluate_promotion` is the one choke point deciding
   Promote vs Blocked(reason); the enqueue path, both event-driven promoters, and the sweeper all call it.
4. **`queued_update_reasons` sidecar table.** The decision function's Blocked outcome is upserted here;
   the row is **deleted in the same transaction as a successful promotion** and cleaned up on terminal
   transitions, with the sweeper as backstop. REST enriches Queued rows from it; an SSE admin event
   invalidates the UI.
5. **Queue promotion sweeper.** A periodic loop (default 15 s, TOML-configurable) re-evaluates every host
   with Queued rows. It is the retry path for external-vzdump waits, the drain path for cases the
   event-driven promoters miss (e.g. a batch item reaped mid-protection never advances its batch today,
   because `execution_owner_service_id` is still NULL — `crates/ui/web-api/src/update_reaper.rs:102`), and
   the janitor for orphaned claims and stale reason rows.

Non-PVE hosts, PVE hosts without a `proxmox_host_mappings` row, and hosts whose effective protection mode
is `do_nothing`/`snapshot` never touch the claim table; for them the gate reduces to the existing per-host
check.

## 5. Components

### 5.1 `PreUpdateProtectionStatus` enum

New enum in `uptrakit-shared-types` (next to `PluginRole`/`UpdateStatus`):

```rust
pub enum PreUpdateProtectionStatus { InProgress, Protected, Failed, Skipped }
```

- `FromStr` with a typed `ParsePreUpdateProtectionStatusError` + `Display`, per the coding-standards
  string-conversion rule. Serialized DB/API strings stay exactly `"in_progress"`, `"protected"`,
  `"failed"`, `"skipped"` — **no migration of existing data, no column type change**. The DB column and
  the wire-visible REST field remain strings; the enum lives at the boundaries.
- All writers (`set_inprogress_for_orchestrator` — `update_dispatch.rs:484`,
  `write_pre_update_protection_status` — `:537`, the proxmox plugin's status constants —
  `update_protection.rs`) and readers (replay predicate, REST response mapping —
  `web-api-queries/src/queries/update_history.rs:76`) go through the enum. The Proxmox plugin's
  `STATUS_*` constants are replaced by the shared enum.
- `Option<PreUpdateProtectionStatus>` keeps `None` = "protection never started" (replay semantics
  unchanged).
- **Batch path stamping**: `dispatch_next_queued_for_host`
  (`crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs:121`) currently runs protection
  without ever writing `in_progress`. It now stamps `InProgress` before invoking protection and the
  outcome after, identically to the orchestrator path. This is required for observability and for claim
  release timing; it also makes the protection phase visible to the reaper/replay machinery uniformly.

### 5.2 `pve_node_claims` table

Migration `m20260816_000001_pve_node_claims` (pattern:
`m20260513_000006_oauth_controller_instances.rs`; conventions per
`docs/development/database-migrations.md` — `.timestamp()` columns, FK indexes, new-table checklist).

| Column              | Type      | Notes                                           |
| ------------------- | --------- | ----------------------------------------------- |
| `id`                | uuid PK   | UUIDv7                                          |
| `tenant_id`         | uuid      | FK `tenants`, indexed; entity is `TenantScoped` |
| `plugin_config_id`  | uuid      | FK `plugin_configs`                             |
| `proxmox_node`      | text      | node name from `proxmox_host_mappings`          |
| `update_history_id` | uuid      | FK `update_history`, **unique**                 |
| `claimed_at`        | timestamp |                                                 |

Unique index on `(plugin_config_id, proxmox_node)` — the node slot. Unique index on
`update_history_id` — one claim per update.

Lifecycle:

- **Acquire**: inside the promotion transaction (`begin_immediate()`), after the `Queued → Pending` CAS
  succeeds, INSERT the claim. Unique violation ⇒ roll back the transaction, leave the row Queued, record
  `PveNodeBusy`. This mirrors the existing unique-violation-as-signal idiom in `trigger_update_for_host`
  (`update_triggers.rs:186-211`) and the scheduler claim CAS (`crates/core/scheduler-runtime/src/claim.rs:37`).
- **Release**: DELETE by `update_history_id` as soon as the protection phase ends — i.e. when the
  protection status is written as `Protected`/`Failed`/`Skipped` (orchestrator path:
  `run_protection_and_dispatch` in `crates/ui/controller-core/src/update/controller.rs`; batch path: after
  its inline protection call), and on every terminal-status write that can precede that
  (`fail_before_agent_dispatch`). The agent-update phase does **not** hold the node.
- **Orphan collection (sweeper)**: DELETE claims whose `update_history` row is terminal
  (`Completed`/`Failed`/`Interrupted`) or whose protection status is no longer
  `NULL`/`InProgress`. A controller that dies mid-backup leaves an orphan; the existing reaper
  terminalizes the row (125 min) and the next sweep frees the node. No heartbeat/lease-timeout machinery:
  unlike `scheduled_tasks` claims, liveness here is already delegated to the update reaper, so the claim
  table stays dumb. This divergence from `docs/architecture/scheduler.md` §HA Claim Mechanism is
  deliberate and documented in the ADR.

Whether a promotion needs a claim is resolved **before** the transaction (read-only): load the
`proxmox_host_mappings` row for the host, resolve the effective protection config
(`resolve_effective_config()` three-layer merge), and require mode = backup. No mapping / no proxmox
detect-role assignment / mode ≠ backup ⇒ no claim. Config changes between resolution and commit are
tolerated (worst case: one conservative extra claim, or one unprotected promotion that PVE's own
serialization still tolerates).

Known limitation (documented, accepted): two `plugin_configs` pointing at the same physical PVE
cluster produce distinct claim keys and can double-book a node. Mappings are per-config by model
(`proxmox_host_mapping.rs` upsert key `(plugin_config_id, proxmox_vmid)`), so the gate follows the same
scoping.

### 5.3 Promotion decision function

`evaluate_promotion(host_id, …)` in `uptrakit-web-api-queries` (next to the promoters' query code).
Outcome:

```rust
enum PromotionDecision { Promote, Blocked(QueueBlockReason) }

enum QueueBlockReason {
    HostBusy,                                  // existing per-host slot taken
    PveNodeBusy { node: String },              // claim table says node taken
    ExternalBackupRunning { node: String },    // PVE task list shows a foreign vzdump
}
```

`QueueBlockReason` lives in `uptrakit-shared-types` with `FromStr`/`Display` (stable snake_case strings;
node carried separately — see 5.4). Callers:

- `trigger_update_for_host` (`update_triggers.rs:113`) — enqueue path. Today: host free ⇒ insert Pending.
  Now: host free **but node blocked** ⇒ insert Queued + reason, identical to the host-busy branch.
- `dispatch_next_queued_update_with_notifier` (non-batch promoter,
  `crates/ui/web-api/src/routes/service_ws/handler/updates/dispatch.rs:268`).
- `dispatch_next_queued_for_host` (batch promoter, `update_batches/dispatch.rs:121`).
- The sweeper (5.5).

The check-then-act window between evaluation and the promotion transaction is closed by the claim insert
inside the transaction (5.2): `evaluate_promotion` is advisory for reasons; the claim is authoritative for
mutual exclusion. `HostBusy` remains authoritative via the existing `uix_update_history_host_active`
unique-violation fallback.

**External vzdump probe.** When the candidate needs a backup-mode claim and the claim slot is free, the
decision function consults the PVE API (new `client.rs` call: GET `/nodes/{node}/tasks?source=active`,
filtered to running `vzdump` tasks) before promoting. A running foreign vzdump ⇒
`Blocked(ExternalBackupRunning)`. Probe results are cached in-memory per `(plugin_config_id, node)` for 5 s
to avoid hammering PVE during bursts; the cache is per-controller and advisory only. **Probe
failure fails open** (log warn, proceed): an unreachable PVE API must not wedge the queue — the protection
step itself will surface the real error, and the per-node claim still bounds our own concurrency. The
probe uses the plugin's existing authenticated client (SSRF-safe resolver already in place).

### 5.4 `queued_update_reasons` sidecar table

Migration `m20260816_000002_queued_update_reasons`:

| Column              | Type      | Notes                                      |
| ------------------- | --------- | ------------------------------------------ |
| `update_history_id` | uuid PK   | FK `update_history` (CASCADE on delete)    |
| `tenant_id`         | uuid      | FK `tenants`, indexed; `TenantScoped`      |
| `reason`            | text      | `QueueBlockReason` discriminant snake_case |
| `detail`            | text NULL | node name for the PVE reasons              |
| `evaluated_at`      | timestamp | last evaluation time                       |

Rules:

- Written (upsert) only by real promotion attempts that return Blocked — never derived at read time. The
  displayed reason therefore cannot drift from promoter behavior.
- **Deleted in the same transaction as the successful `Queued → Pending` promotion** (explicit user
  requirement), and by the terminal-status paths that can bypass promotion. Sweeper backstop: delete rows
  whose update is no longer `Queued`.
- `update_history` rows stay immutable (creation + status CAS excepted) — the mutable state lives entirely
  in the sidecar, satisfying the entity's immutability rule.

### 5.5 Queue promotion sweeper

New module `crates/ui/web-api/src/queue_sweeper.rs`, shaped like `update_reaper.rs` (loop + thin glue;
logic in a `web-api-queries` module), spawned from `controller-runtime` boot next to the reaper
(`boot/serve.rs:193`).

Each tick, across all tenants:

1. Find hosts having ≥1 `Queued` row (single grouped query — no per-host N+1).
2. For each such host with a free host slot, run the promotion attempt (same code path as the
   event-driven promoters: `evaluate_promotion` → claim → CAS → dispatch). Blocked ⇒ upsert reason.
3. Janitor: collect orphaned `pve_node_claims`, delete stale `queued_update_reasons`.

Multiple controllers may sweep concurrently; the CAS + claim unique index make that safe, and duplicate
reason upserts are idempotent.

**Interval configuration**: new TOML section in `uptrakit-shared-config-reload`
(`crates/shared/config-reload/src/config/`, template: `zeroconf.rs`):

```toml
[updates]
queue_sweep_interval_seconds = 15   # default 15, must be ≥ 1
```

Wire-up per the crate's idiom: new section module + field on `RuntimeConfig`, `validate()` rejecting 0,
`warn_about_extras`, `RuntimeConfigDelta` tag. Classified **restart-required** in
`docs/end-user/operator-runbook-reload.md` (hot-reloading a sleep interval isn't worth the machinery).
This deliberately diverges from the reaper's hardcoded `REAPER_INTERVAL` const because the sweep cadence
directly trades PVE API load against queue latency — an operator-visible knob; the ADR records this.

### 5.6 REST, SSE, UI

- `UpdateHistoryResponse` (`crates/shared/web-api-types/src/update_history.rs:52`) gains

  ```rust
  pub queue_block_reason: Option<QueueBlockReasonResponse> // { reason, detail, evaluated_at }
  ```

  populated for `Queued` rows by joining the sidecar in `list_update_history`/`get_update_history`
  (`web-api-queries/src/queries/update_history.rs:172/:306`) — one batched join, no per-row lookups.
  `pre_update_protection_status` stays `Option<String>` on the wire (values now guaranteed by the enum).
  After the change: `./scripts/regen-api.sh`, commit `openapi.json` + `frontend/src/lib/api/generated/`;
  extend `uptrakit-openapi-client` accordingly.

- New `AdminEvent::UpdateQueueStateChanged { update_history_id, host_id }`
  (`crates/shared/wire/src/admin_events.rs:32` + `event_name()` arm), emitted when a reason row is
  created, changes value, or is removed. Frontend: add the member to the hand-maintained
  `AdminEventType` (`frontend/src/lib/sse.ts:230`). Run `./scripts/regen-asyncapi.sh` and commit if the
  golden file changes.
- Web UI: update-history list/detail render a wait-reason badge/tooltip on Queued rows
  ("Waiting: another update active on this host" / "Waiting: backup running on PVE node X" /
  "Waiting: external backup running on PVE node X"), live-updating via the SSE event. Empty reason (row
  enqueued, not yet evaluated — ≤ one sweep) renders as "Queued — evaluating…".

## 6. Concurrency and failure analysis

| Scenario                                                         | Outcome                                                                                                          |
| ---------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| Two controllers promote different hosts, same node, same instant | One claim INSERT wins; loser rolls back, row stays Queued with `PveNodeBusy`.                                    |
| Controller dies mid-backup                                       | Claim orphaned; reaper terminalizes the row (125 min) → next sweep frees node. Documented as the recovery bound. |
| External vzdump running                                          | Probe blocks promotion; sweeper retries every 15 s until clear. Probe failure fails open.                        |
| Batch item reaped mid-protection                                 | Today the batch stalls (owner never set). Sweeper re-evaluates the host and drains the batch's next Queued row.  |
| Row enqueued between sweeps, event promoter fires                | Event-driven path promotes immediately; sweeper is backstop only — no added latency in the common case.          |
| Restart                                                          | All state in DB; first sweep (≤ 15 s) restores reasons and collects orphans.                                     |
| SQLite vs Postgres                                               | Same code path: `begin_immediate()` (sole opener) + unique indexes; no isolation-level assumptions.              |

## 7. Testing

- Enum round-trips: `PreUpdateProtectionStatus`/`QueueBlockReason` `FromStr`/`Display` including error
  cases (own logic, not serde-derive testing).
- Claim lifecycle (SQLite in-memory, real migrations): acquire, unique-violation on second acquire,
  release on each protection outcome, orphan collection for terminal rows. Non-vacuous: assert the
  violating insert _fails_ and the first claim survives.
- `evaluate_promotion`: each Blocked variant + Promote path; backup-mode resolution (no mapping ⇒ no
  claim; snapshot mode ⇒ no claim). Probe injected behind a trait seam so tests use a stub instead of a
  live PVE client (no upstream-HTTP behavior testing).
- Promotion transaction: reason row deleted atomically with the CAS; failed claim leaves row Queued and
  reason upserted.
- Enqueue path: host free + node busy ⇒ Queued with reason (new branch in `trigger_update_for_host`).
- Batch path stamping: protection status visible as `in_progress` during batch protection.
- Sweeper query functions unit-tested directly (grouped host query, janitor deletes); the loop stays thin
  glue like the reaper (constants-only assertions there). No `start_paused` on DB-touching tests.
- REST: TestApp harness — list/detail expose `queue_block_reason` for Queued rows, absent otherwise.
- Config: `[updates]` section parse, default 15, `0` rejected by `validate()`.
- Frontend: events store test for the new `AdminEventType` member; component test for the badge.

Audit: new write sites (`pve_node_claims`, `queued_update_reasons` upserts/deletes) are internal
bookkeeping — register as `skip` in `audit-catalog.toml` (the status CAS sites are already catalogued);
`cargo xtask audit-coverage-check` must stay green. `python3 ci/verify_db_access_policy.py` must pass
after the query modules are added.

## 8. Documentation deliverables

New ADR via `adrs new "PVE node concurrency gate for update promotion"` — records the claim-table gate,
its deliberate divergence from the scheduler HA-claim pattern (no heartbeat; liveness delegated to the
update reaper), the fail-open probe, and the configurable sweep interval vs. ADR-0024's constant.

Updates (from a docs-tree sweep, not memory):

- `docs/architecture/update-history-entity.md` — status/queue sections, new tables, promotion flow.
- `CONTEXT.md` — glossary: node claim, queue block reason, promotion sweeper.
- `docs/api/http-web-api.md` — `queue_block_reason` field, typed protection-status values.
- `docs/end-user/update-history.md`, `docs/end-user/update-workflow.md` — queue-wait reasons narrative.
- `docs/end-user/proxmox.md`, `docs/development/proxmox-plugin.md` — node gate + external-vzdump wait;
  plugin table additions.
- `docs/api/sse-events.md`, `docs/development/sse-events.md` — new admin event.
- `docs/api/batch-actions.md`, `docs/end-user/batch-actions.md` — second concurrency dimension note.
- `docs/architecture/scheduler.md` or the new ADR — cross-reference for the claim-pattern divergence.
- `docs/api/settings-runtime.md` — sweep interval is TOML-only.
- `ARCHITECTURE.md` — `[updates]` TOML section in the section list.
- `docs/end-user/operator-runbook-reload.md` — classify `updates.queue_sweep_interval_seconds` as
  restart-required.
- `docs/adr/0024-update-liveness-and-interrupted-status.md` — no edit; superseding notes live in the new
  ADR.

No wire-protocol (service↔controller) payload changes ⇒ no `asyncapi.yaml` change expected beyond the
admin-event regen check above. No new external dependencies ⇒ no version pins.

## 9. Deferred

- Promoter unification (batch/non-batch structural merge) — separate spec.
- Protection-status CAS hardening (`write_pre_update_protection_status` / `fail_before_agent_dispatch`
  status guards; replay-NULL semantics vs `success(None)`) — separate hardening spec.
- Update cancellation — out of scope by decision; revisit only on explicit request.
- Cross-config node identity (two plugin configs → same physical cluster double-booking a node).
- TOML key reference page (documentation gap noted during design; not created here).
