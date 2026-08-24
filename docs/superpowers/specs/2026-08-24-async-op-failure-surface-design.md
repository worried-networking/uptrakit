# Per-Item Failure Surface for Async Agent Operations — Design

**Date:** 2026-08-24
**Status:** Design (pending plan)
**Origin:** promotes epic bead `uptrakit-async-op-failure-surface`

All `file:line` references are locator hints against `main` @ `c39f9727a`; verify before editing.

## Problem

The controller has no failure channel for asynchronous operations. When a result arrives with
`error.is_some()`, the handler drops it before any DB write, SSE emission, or MQTT publish:

- **Agent version checks** — the result loop `continue`s on errored results
  (`crates/ui/web-api/src/routes/service_ws/handler/messages/version_check.rs:566-575`),
  incrementing only an audit `error_count`; `finalize_version_check_results` filters errored
  results out of the `last_checked_at` batch update and out of the `VersionCheckCompleted` SSE
  `completed_pairs`.
- **Controller-side release fetches** — the manual trigger route and the scheduled
  `FetchReleasesExecutor` log per-item fetch errors and move on. A scheduled fetch that fails for
  every package reports a clean run.
- **Discovery** — per-plugin errors are dropped at
  `crates/ui/web-api-queries/src/queries/autodiscovery/mod.rs:128-137` (crate
  `uptrakit-web-api-queries`). The `DiscoveryCompleted` SSE event is defined but never emitted.

Consequence: a failing check (plugin error, timeout, unreachable upstream) is visible only in
controller logs and an audit counter. The user cannot tell "never checked" from "checked and
failing since June". No DB column records failed state; only success timestamps exist
(`last_checked_at` on `software_items`; `last_discovered_at` and `latest_version_fetched_at` on
`host_software_items`).

## Goal

Persist a one-line error summary per failing item/plugin, clear it on the next success of the same
writer, push an SSE refresh signal, and render the failure in the UI (per-item badge and host-level
aggregate). Keep the no-internal-log-storage invariant: a summary line, never full command output.

## Decisions (settled with owner, 2026-08-24)

| #   | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| --- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1  | **Error-source scope: all three sources.** (A) agent version checks, (B) controller-side release fetches — manual trigger and scheduled executor, (C) per-plugin discovery runs. The source bead names discovery explicitly; the scheduled fetch path runs unattended and needs visibility most. Gate `uptrakit-0wpg4`, answer: option 3.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| D2  | **Storage: two column pairs on `host_software_items` + one new discovery-failure table.** Nullable `last_check_error`/`last_check_error_at` (writer: agent version-check path, covering merged detect+fetch errors from the agent) and nullable `last_fetch_error`/`last_fetch_error_at` (writer: controller fetch path). Each pair is written on its writer's failure and cleared only by its own writer's success — no cross-writer erasure. Discovery failures go to a new `host_discovery_failures` table: `tenant_id`, `host_id`, `plugin_type`, `plugin_config_id` (nullable), `error`, `failed_at`, with partial unique indexes over the nullable config id (one row per `(host, plugin_type, config)` identity; upsert on repeat failure, delete on that identity's success). Reuses existing row scoping and REST assembly; follows the `scheduled_tasks.last_error` precedent. Gate `uptrakit-bktl8`, answer: option 1. |
| D3  | **Additive `skipped` marker on `DiscoveryPluginResult`.** Today a plugin that never ran (host requirements not met, incompatible OS) and a plugin that found nothing both report `error=None, discoveries=[]`. New additive wire field marks the skipped case (flag or skip reason). The controller ignores skipped results for the failure surface (no clear, no write) AND for reconciliation (a skipped plugin never stamps `missing_since` on its items). Root-cause fix; closes the never-ran reconciliation hole for skipped plugins. The separate false-`missing_since` defect from per-chunk reconciliation of paginated reports is pre-existing debt, deferred (`uptrakit-def-reconcile-chunked-pagination`). Gate `uptrakit-wkg2b`, answer: option 1.                                                                                                                                                                   |
| D4  | **SSE shape: reuse `VersionCheckCompleted`, start emitting `DiscoveryCompleted`.** `VersionCheckCompleted` (a per-pair event) also fires for failed pairs; the event's meaning widens to "check finished, result recorded". `DiscoveryCompleted` (already defined, never emitted) starts firing, gated on the final report page (`PageOutcome::Final`) so paginated reports emit once. Per-host event coalescing is deferred to its own bead (`uptrakit-def-sse-host-coalescing`) — the per-pair refetch burst is pre-existing debt, orthogonal to this feature. Gate `uptrakit-kxdoe`, answer: option 1.                                                                                                                                                                                                                                                                                                                         |
| D5  | **Errored results persist their valid data fields.** An errored version-check result can carry fresh data (e.g. `installed_version` when detect succeeded but fetch failed). Presence-guarded fields persist first, then the error is recorded. The error path never overwrites `update_category` with its default. The agent did the work; discarding fresh data misleads the user. Gate `uptrakit-9la40`, answer: option 1.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| D6  | **Pre-existing reconciliation config-identity defect: deferred.** Reconcile matches candidates by host and `plugin_type` only; with two configs of one plugin type, config A's result can stamp `missing_since` on config B's items. D3's skipped marker removes the never-ran trigger; the config-identity fix changes reconciliation identity semantics and gets its own cycle (`uptrakit-def-reconcile-config-identity`). Gate `uptrakit-ybq4s`, answer: option 1.                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| D7  | **Agent-side error truncation to the wire cap.** Today an error string over 4096 chars (`MAX_MEDIUM_STRING_LEN`) kills the WS connection: `check_opt_string_len` rejects the frame → `TextAction::Break`. The agent truncates error strings to the wire cap before send. The controller additionally truncates to ~1 KB with an ellipsis before DB write — a summary, never full output. Settled by fact-finding.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| D8  | **Attribution rules.** Error writes go through the existing host-ownership resolution, batched (no per-item N+1). An errored result with `host_software_item_id=None` counts as unmatched and is dropped from per-item persistence — never attributed via legacy fallback. `not_ready` is currently dead (all producers hardcode `None`); its intended semantics — no write, no clear — are documented, not implemented. Settled by fact-finding.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| D9  | **Stale discovery-failure pruning at full-host dispatch time.** When full-host discovery dispatch builds its plugin set (all compiled-in discovery plugin types gated by the effective tenant/host allowlist, plus per-config rows), `host_discovery_failures` rows whose `(plugin_type, plugin_config_id)` identity is absent from that set are deleted. There are TWO full-host dispatch sites — the service-WS route path and the external scheduler's periodic executor (`scheduler-runtime/src/executors/discover_software.rs` `build_assignments`) — so the prune is a shared helper both call; implementing only one site leaves allowlist edits unpruned until an agent reconnect. The single-config discovery route dispatches a one-plugin subset and never prunes. Settled by fact-finding.                                                                                                                            |
| D10 | **New table is tenant-scoped.** `host_discovery_failures` implements `TenantScoped`, all queries go through `TenantDb`, and the reset-data endpoint gains a step for it. Migration uses `helpers::timestamp_null` for nullable timestamps. Settled by fact-finding.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| D11 | **MQTT exposure deferred.** `SoftwareStates` and HA discovery entities do not carry failure state in this cycle (`uptrakit-def-mqtt-failure-state`). Settled by fact-finding.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |

Alternatives rejected during grilling: a shared single error-column pair (cross-writer erasure —
one writer's success hides the other's failure); a generic `(item, source)` side table for check
errors (new tenant-scoping plumbing and reset-data step for no added capability over two column
pairs); heuristic restriction of failure-row deletion instead of the skipped marker (leaves the
reconciliation hole open); designing new coalesced per-host SSE events now (migrates all frontend
subscribers inside an unrelated feature).

## Chosen approach

### Wire (uptrakit-wire)

- `DiscoveryPluginResult` gains an additive `skipped` marker (serde-defaulted so old agents remain
  valid). If it carries a reason string, the field gets a `WireValidate` length limit in
  `limits.rs`/`wire_validate_impls.rs`. Regenerate `crates/shared/wire/asyncapi.yaml`.
- No other wire shape changes. Version-check results already carry `error`.

### Agent (uptrakit-agent-core)

- Truncate error strings to `MAX_MEDIUM_STRING_LEN` before building result payloads (D7). The
  wire limit (`check_string_len`) counts **bytes**; the truncation must cut at a `char`
  boundary (`String::truncate` at a non-boundary panics, and the workspace builds with
  `panic = "abort"`). Same rule for the controller's ~1 KB DB-side truncation.
- Report gated-out discovery plugins as `skipped` instead of empty success (D3).

### Controller persistence

- Migration: add `last_check_error`, `last_check_error_at`, `last_fetch_error`,
  `last_fetch_error_at` to `host_software_items`; create `host_discovery_failures` with partial
  unique indexes over nullable `plugin_config_id` (D2, D10). The `plugin_config_id` FK is
  `ON DELETE CASCADE`: `Restrict` would block plugin-config deletion while a stale failure row
  exists, and `SetNull` would collapse a config-scoped row into the `(host, plugin_type, NULL)`
  identity that legitimately belongs to the default-assignment row, colliding on the partial
  unique index.
- Version-check result handler: for errored results, persist presence-guarded data fields (D5),
  then batch-write `last_check_error/_at` (truncated, D7) via `update_many` + `is_in`; on success,
  clear only `last_check_error/_at`. Unmatched results stay excluded (D8). On the error path
  `update_category` is not written at all (it is not presence-guarded on the wire); the plan
  restructures `apply_version_update_to_db` accordingly — its `debug_assert!` currently forbids
  error-bearing results and its `UpdateCategory` write is unconditional
  (`version_check.rs:114-155`). Two D5 side effects the plan must handle: (a) `update_available`
  is derived at read time as plain `installed != latest` inequality
  (`queries/software_states.rs:175-181`), so persisting a fresh `installed_version` from an
  error-bearing result against a stale `latest_version` can flip it — accepted, the co-presented
  failure badge carries the context; (b) writes from error-bearing results must not trigger
  update-available notifications — a failed run must never notify. Batching detail: distinct
  per-result error strings cannot share one `update_many`; group rows by identical error text
  (one `update_many` per distinct message — the run-level timeout case is naturally one group);
  the clear path is a single batch. D8's no-N+1 rule means no per-item queries, not one
  statement.
- Writer/role alignment: agent `CheckVersions` dispatch omits the `fetch_releases` assignment
  when `is_controller_fetch_site` resolves controller-side
  (`routes/software_items/version_check_dispatch.rs:262-287`), so for controller-fetch items the
  agent leg is detect-only and the two column pairs align with the role split;
  `last_check_error` covers merged detect+fetch only when the fetch role is agent-sited.
  Dispatch-time hygiene (mirror of D9): when dispatch finds no agent-side work for an item (no
  `detect_version` assignment and fetch controller-sited), a stale `last_check_error/_at` on
  that item is cleared — an execution-site flip never orphans an error.
- Concurrency: error/clear writes are last-write-wins with no run-identity tracking. A stale
  value from an out-of-order report self-heals on the next run (scheduled defaults: 6 h
  `fetch_releases`, 24 h `detect_version`); the agent-side background-op guard (network-timeout
  plan 3, M1.7) removes overlapping runs at the source. Accepted for a visibility-only surface.
- Controller fetch paths (manual route + scheduled executor): write/clear `last_fetch_error/_at`
  per this exit taxonomy — the current code has several non-error `continue` exits
  (`controller_fetch.rs:97-151`), and "symmetric" alone would leave them undefined:
  - fetch returned releases and versions were written → success, clear the pair;
  - fetch succeeded but returned zero releases → success for the failure surface, clear the pair
    (an upstream with legitimately no releases must not keep a stale red badge);
  - upstream call failed → failure, write the pair;
  - unknown plugin type / missing `release_fetcher` role / fetcher-creation failure → failure,
    write the pair (a misconfigured assignment is a real, user-fixable fault);
  - item skipped before any fetch attempt (job-construction exits) → neutral, no write, no clear.
    A shared-cause upstream outage (one API call serving N target items) writes the same summary on
    N rows — accepted redundancy under the owner-settled column model (D2).
- Discovery result handling: per-plugin error upserts its `host_discovery_failures` row; success
  for the same `(host, plugin_type, config)` identity deletes it; skipped results write nothing,
  delete nothing, and are excluded from reconciliation (D3). A run-level discovery timeout
  stamps the same error on every dispatched plugin identity — N rows from one cause, accepted.
  Under report pagination, chunks of one plugin's result share identity and error; the repeated
  upsert/delete per chunk is idempotent on the same row. Upsert implementation: sea-query 1.0.2
  supports a conflict-target predicate for the partial unique indexes
  (`OnConflict::target_and_where`/`target_cond_where`); the insert-then-
  `is_unique_constraint_violation`-fallback idiom (`queries/autodiscovery/discovery_items.rs`)
  is the alternative.
- Pruning (D9): the pruning set is the assignment vec the full-host discovery dispatch just
  built (all compiled-in discovery plugin types gated by the effective tenant/host allowlist,
  plus per-config rows). Rows whose `(plugin_type, plugin_config_id)` identity is absent from
  that set are deleted at full-host dispatch only. The prune is one shared query helper called
  from BOTH full-host dispatch sites: the service-WS route path
  (`routes/service_ws/handler/discovery.rs`) and the external scheduler's periodic executor
  (`scheduler-runtime/src/executors/discover_software.rs` — it builds assignments and sends
  `DiscoverSoftware` itself, without going through the web-api route). The helper lives where
  both crates can reach it (`uptrakit-web-api-queries` if the scheduler runtime can depend on
  it, else the shared DB layer). The single-config discovery route
  (`plugin_configs/{id}/discover`) dispatches a one-plugin subset and never prunes.
- Version-skew posture: an old agent that does not know a newly compiled-in discovery plugin
  type returns `error: Some("unknown plugin type: …")` per dispatched type
  (`agent-core/src/client.rs:843-848`); after a controller upgrade this creates persistent
  failure rows per host until the agent is upgraded. Accepted: the rows state a real incapacity
  ("agent cannot run this plugin"), and clear on the first post-upgrade success or skip.
  Likewise, an old agent without the `skipped` field still reports gated-out plugins as empty
  success — the D3 fix (no delete, no `missing_since`) applies only to upgraded agents; the
  pre-upgrade behavior is today's behavior, no regression.

### API + SSE

- REST: the error pairs are per-host facts on `host_software_items`, so they land where per-host
  fields already land — the host-filtered software-item responses and the per-host summaries in
  the item detail (`SoftwareItemHostSummary`-style), NOT as new cross-host aggregate fields on
  the unfiltered list response. A host-level discovery failure listing (aggregate) goes on the
  host detail response or a narrow sub-resource. Params via `IntoParams` structs; regenerate
  `openapi.json` + frontend generated client; keep `uptrakit-openapi-client` in sync.
- SSE: `VersionCheckCompleted` is a per-pair event (`{ host_id, software_item_id }` — there is
  no `completed_pairs` wire field; that is a handler-local vec). "Include failed pairs" means:
  emit the event for failed results too, which requires running the errored results through the
  same host-ownership resolution as successes (D8 attribution) to obtain the pair. Emit
  `DiscoveryCompleted` on `PageOutcome::Final` (D4). Both remain fire-and-forget refresh
  triggers.
- The controller fetch path emits `VersionCheckCompleted` for its failed pairs even when zero
  items succeeded — the current emission is gated on a non-empty updated-item set
  (`controller_fetch.rs:206-236`), which would hide an all-failed run from the UI.
- New SSE load, accepted: `DiscoveryCompleted` has never fired, so the 6-hour tenant-wide
  rediscovery (every host dispatched) now produces one event per host in a burst; the hosts page
  refetches per event. Bounded by host count at current scale; the per-host coalescing bead
  (`uptrakit-def-sse-host-coalescing`) is the fix path if it becomes a problem.
- Known gap, accepted: `PageOutcome::Final` tracking is per-connection (`ReportTracker`), so an
  agent reconnect mid-report can drop the `DiscoveryCompleted` emission for that run. The
  5-minute fallback poll covers it; the persisted failure rows are unaffected.

### Frontend

- Software page: per-item failure badge (existing `StatusBadge` primitive, danger tone) with the
  stored error summary and timestamp, for both column pairs. The detail text uses an existing
  disclosure pattern: the `Tooltip` primitive is an icon-trigger component (its own `(i)` button),
  not a badge-hover wrapper — the plan composes badge + existing primitives; no new
  badge-as-trigger primitive.
- Host view: aggregate indicator fed by `host_discovery_failures` + per-item error counts. This
  aggregate is distinct from the existing host-row danger badge (`error_count` from failed
  `update_history` rows, `queries/hosts.rs` `HostSoftwareStatusSummary`) and is never merged into
  it — update-execution failures stay out of scope. The plan places it so the two indicators
  cannot be confused (host detail view or a separately labeled element).
- Existing SSE-triggered list refresh plus the 5-minute fallback poll pick up the new fields; no
  new client-side state machine.

### Audit

- Error counting in the audit path already exists and stays. New state-changing write sites
  (error column writes, discovery-failure upserts/deletes) get `audit-catalog.toml` entries
  (`action` or justified `skip`) so `cargo xtask audit-coverage-check` stays green. These sites
  are **Event**-class (or justified `skip`), never Stateful: these result-handling write paths
  run against a plain `DatabaseConnection` with no wrapping transaction, so the in-tx
  `emit_stateful` write is unavailable — the rationale documented on `emit_reactivation_event`
  (`queries/autodiscovery/discovery_items.rs:24-29`). The batched `update_many` writes also carry
  no per-row before/after snapshots. New sites keep this posture (plain connection, no
  `begin_immediate()` wrapper) — the version-check handler's existing aggregate Event audit is
  the model.

## Non-goals / out of scope

- No retry/backoff or scheduling changes — visibility only.
- No in-flight "checking" state; the UI keeps request-scoped spinners.
- No MQTT/Home Assistant failure exposure (D11, deferred).
- No per-host SSE coalescing (D4, deferred).
- No reconciliation config-identity fix (D6, deferred).
- No storage of full command output — one-line summaries only.
- No changes to `update_history` (update execution already has its own failure surface).

## Documentation deliverables

- New ADR (via `adrs new`, never hand-allocated): per-writer error-column ownership, the
  `host_discovery_failures` identity model, and skipped-result semantics for the failure surface
  and reconciliation (extends the ADR-0027 family).
- `docs/api/wire-protocol.md` + regenerated `crates/shared/wire/asyncapi.yaml`: `skipped` marker,
  error truncation note.
- `docs/development/autodiscovery-internals.md`: skipped semantics, failure table, dispatch-time
  pruning.
- `docs/architecture/software-item-entity.md` (and `unified-software-tracking.md` if it lists
  columns): new error column pairs and their single-writer rule.
- `docs/development/sse-events.md`: widened `VersionCheckCompleted` meaning, `DiscoveryCompleted`
  emission.
- Regenerated `crates/ui/web-api/openapi.json` + `frontend/src/lib/api/generated/`;
  `uptrakit-openapi-client` updated with the changed endpoints.
- `audit-catalog.toml` entries for new state-changing sites.
- `db_access_policy.toml` updates for touched handlers (same commit as signature changes).

## Dependencies

- Hard predecessors (same-files only — serialize implementation; each wired `blocks` onto the
  spec epic):
  - `uptrakit-spec-2026-08-19-phs-base-os-compatibility` — adds columns to `host_software_items`,
    touches the version-check enrichment write path, REST software-item summaries, and the same
    frontend badge surface.
  - `uptrakit-plan-2026-08-19-tag-series-3-discovery-fill-notify` — edits the same autodiscovery
    result-handling path (discovery-time persistence + notification emission).
- Soft relations (`bd dep relate`, informational only):
  - `uptrakit-plan-2026-08-17-network-timeout-safety-3-m1-controller-ssh-sse` — origin of the
    source bead; the 2026-08-22 amendment classified this epic independent, so no blocking edge.
  - `uptrakit-plan-2026-08-17-network-timeout-safety-4-m2-kill-mechanics` — new agent-core error
    variants flow through the truncation path; additive on both sides.
  - `uptrakit-plan-2026-08-19-tag-series-2-phs-inference` — PHS plugin adjacency only.
  - `uptrakit-plan-2026-08-22-m2-2-visibility-aware-queries` — trait impls on the same entities;
    query-layer only, no schema conflict.
- New external dependencies: none. All work uses crates already in the workspace.

## Deferred

Each item gets a `discovered-from` bead at registration:

- `uptrakit-def-sse-host-coalescing` — coalesce per-pair SSE events into per-host events; migrate
  frontend subscribers (D4).
- `uptrakit-def-reconcile-config-identity` — include plugin config identity in reconciliation
  candidate matching (D6).
- `uptrakit-def-mqtt-failure-state` — expose failure state via MQTT/HA entities (D11).
- `uptrakit-def-reconcile-chunked-pagination` — reconciliation runs per report chunk
  (`normalize()` splits one plugin's result across pages; `reconcile.rs` treats each chunk as the
  full result set), so plugins over `MAX_DISCOVERIES_PER_PLUGIN` can false-stamp `missing_since`.
  Pre-existing defect discovered in review, D6-adjacent (D3).

## Success criteria

- A failing agent version check writes `last_check_error/_at` on the item row; the next successful
  check clears it; a successful controller fetch does not clear it (and vice versa for
  `last_fetch_error/_at`).
- An errored result carrying fresh data fields persists them (presence-guarded); `update_category`
  is never reset to default on the error path.
- A failing discovery plugin run upserts one `host_discovery_failures` row per
  `(host, plugin_type, config)` identity; that identity's success deletes it; a skipped plugin
  writes nothing, deletes nothing, and stamps no `missing_since`.
- Removing a plugin from the effective discovery allowlist (or deleting its plugin config)
  removes its stale failure rows at the next full-host dispatch; a single-config discovery run
  deletes no other plugin's rows.
- `VersionCheckCompleted` includes failed pairs, and the controller fetch path emits it even when
  every pair failed; `DiscoveryCompleted` fires once per discovery report (final page).
- REST responses expose the error fields; the UI shows a per-item failure badge with summary +
  timestamp and a host-level aggregate.
- An error string > 4096 chars no longer breaks the WS connection; stored summaries are ≤ ~1 KB.
- Success and failure paths covered by tests; all quality gates pass, including
  `audit-coverage-check`, `openapi-client-check`, asyncapi golden test, and staleness gates.
