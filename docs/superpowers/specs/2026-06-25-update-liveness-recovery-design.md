# Update liveness & interrupted-recovery — design spec

- **Date:** 2026-06-25
- **Status:** Draft (NO_PLAN)
- **Author:** Andrey Yantsen (with Claude)
- **Snapshot:** `.superpowers/standards-snapshot.md` (v1, 2026-06-25)

## 1. Problem

Two interactive updates (n8n, mealie) dispatched over SSH from `agent-ssh` got stuck
permanently in `in_progress`. Root cause (fully diagnosed): the MacBook running **both**
controller and `agent-ssh` went to clamshell sleep mid-update (09:59:50 UTC), woke 10:09:36.
Output froze at the same instant on both; the remote build was killed; no terminal result
ever reached the controller. Three independent defects let a dead update masquerade as alive
forever:

1. **No SSH keepalive.** `agent-ssh` builds the russh client with `client::Config::default()`
   (`crates/core/agent-ssh-runtime/src/ssh_transport.rs:810`) — `keepalive_interval` /
   `keepalive_max` unset. The interactive driver (`drive_interactive_ssh_session`,
   `ssh_transport.rs:468`) blocks on `channel.wait()` forever on a zombie half-open TCP
   connection. The completion future never resolves → no `UpdateResult` ever sent.
2. **No reconnect → no recovery.** The agent↔controller websocket merely _paused_ across the
   sleep (macOS `TCPKeepAlive=active` kept the TCP up) and resumed on the same connection. The
   only existing recovery, `mark_owned_in_progress_as_failed_on_reconnect`
   (`crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs:425`), fires only on
   reconnect, so it never ran.
3. **Payload `timeout` dropped on the interactive path.** The non-interactive executor wraps
   the future in `tokio::time::timeout` (`ssh_executor.rs:120`); `execute_interactive`
   (`ssh_executor.rs:182`) does not. The 7200 s budget is silently discarded on exactly the
   path that hung.

A previous record left `output_bytes` set but the `output` column empty — that is expected
(streamed lines live in `update_output_lines`; `update_history.output` is written only at
finalization), not a defect.

## 2. Goals / non-goals

### Goals

- An update whose execution dies silently (sleep, host crash, network partition, zombie TCP)
  always reaches a terminal state in bounded time, with an honest outcome.
- The host slot frees so the user can re-trigger.
- Detection is **agent-authoritative**: only the agent holding the SSH session, or the
  update's own absolute time budget, decides liveness — never the controller's view of
  agent connectivity.

### Non-goals

- Automatic re-dispatch of updates (forbidden by AGENTS.md invariant "Uptrakit updates never
  automatic"). Recovery is user-driven.
- Auto-triggering a version check as part of reaping (user decision: user-driven only). The
  normal scheduled read-only version check reconciles displayed installed-version over time.
- Fixing the disk-pressure side-effect observed in the incident (orphaned npm cache on n8n).
  Tracked separately; see §10.
- Preventing host sleep. Out of scope; environmental.

## 3. Load-bearing constraint (user)

> A brief loss of connectivity **between agent and controller** must NOT mark an update
> `Interrupted` while the update is still running on the host.

The agent owns the SSH session and is the sole authority on whether its update is alive. The
controller↔agent link is **not** evidence of update liveness. This constraint is decisive:

- It **rejects** any reaper keyed on controller-side connectivity staleness
  (`service.last_seen_at`, missed pings, a per-update heartbeat). A long but transient
  websocket partition with a healthy SSH session must leave the update running.
- The controller backstop may therefore key off only the update's **own absolute budget**
  (`started_at + payload_timeout`), which an alive update cannot legitimately exceed.

This both honors the constraint and matches the contrarian review's conclusion that a
per-update heartbeat is redundant with agent-side detection.

## 4. Design — defense in depth (recommended)

Four changes. B and C fix the causes at the agent; A is the correct-by-construction backstop;
D makes the new terminal state safe and consistent. The existing instance-aware reconnect
recovery is refined (E) but not replaced.

### A. Controller-side absolute-deadline reaper (backstop)

A periodic background sweep, modeled on the in-repo heartbeat/stale-prune pattern in
`crates/ui/web-api/src/oauth/boot.rs` (`spawn_heartbeat`, `STALE_TTL_HOURS`, cutoff query).

- **Predicate (wall-clock only):** `now_utc() - started_at > UPDATE_TIMEOUT + grace`. The
  update budget is the wire constant `uptrakit_wire::DEFAULT_UPDATE_TIMEOUT` (7200 s / 2 h,
  `crates/shared/wire/src/shared_types.rs:77`) — the single dispatch site
  (`update_dispatch.rs:1091`) always sets `ExecuteUpdatePayload.timeout` to it; there is no
  per-update or per-plugin override. `grace` is a small fixed margin (proposed 300 s) to let
  agent-side detection (B/C) report first. **No new column** — the reaper uses the constant
  directly against the existing `started_at`. (See resolved Q1, §11.)
- **Scope the sweep:** only rows with `status = in_progress` and a non-null `started_at`. Do
  not reap `Queued`/`Pending` (not yet executing) or `AwaitingRestart` (a real post-update
  state with its own `awaiting_restart_timeout`).
- **Action:** transition matching rows to **`Interrupted`** (see D) with a `recovery_hint` of
  "execution outcome unknown — connection lost or deadline exceeded; verify installed version
  before re-running". Set `completed_at`. Emit the same notification + admin/SSE events that
  real completions emit.
- **Authority:** uses only the update's own budget. Never reads agent connectivity → honors §3.
- **Cadence:** fixed interval, mirroring the `oauth/boot.rs` heartbeat loop
  (`tokio::time::sleep` in a `loop`); proposed 60 s. No need to derive from the smallest
  outstanding deadline — a fixed tick is simplest and matches precedent. (Resolved Q2, §11.)
- **Sleep-robustness:** the tokio loop's `sleep` freezes during host sleep and resumes after
  wake; because the predicate compares two **wall-clock** instants (`now_utc()` vs
  `started_at`), the first post-wake tick correctly observes the elapsed real time. **Rule:**
  never cache a monotonic `tokio::time::Instant` deadline — wall-clock only.
- **Clock injection (testability):** the repo rule is deterministic time tests via
  `#[tokio::test(start_paused = true)]` + `tokio::time::advance` (testing.md). But `advance`
  moves the paused **monotonic** clock, not `OffsetDateTime::now_utc()`. So the reaper must
  take an injected wall-clock source (`Arc<dyn Fn() -> OffsetDateTime>` or a small `Clock`
  trait) that tests drive independently of the tokio clock. Decide this at the interface
  boundary; it is painful to retrofit.

### B. Agent SSH keepalive (russh)

Set on the client `Config` at `ssh_transport.rs:810` (russh 0.61.2 — fields already present,
no new dependency):

- `keepalive_interval = Some(Duration::from_secs(15))`
- `keepalive_max = 4`

russh sends SSH-level keepalive probes and tears the connection down once
`alive_timeouts > keepalive_max` (`russh-0.61.2/src/client/mod.rs:1218`). On a zombie peer,
`channel.wait()` then returns → the completion future resolves `Err` → the agent reports a
clean terminal `UpdateResult`. After wake from sleep, the keepalive timer resumes and detects
the dead peer within ~`interval × max` (≈60 s) — this is the primary fix for the incident.

- **Do NOT set `inactivity_timeout`.** It would kill a legitimately quiet-but-alive session
  (e.g. the incident's 13-min backup that emitted no lines). Keepalive probes liveness without
  requiring data flow; that is the correct tool.

### C. Wire payload `timeout` into the interactive PTY path

Make `execute_interactive` (`ssh_executor.rs:182`) symmetric with the non-interactive path:
wrap the interactive completion future in `tokio::time::timeout(payload_timeout, …)`. On
elapse, abort the channel and report a terminal failure ("update exceeded N s timeout"). This
bounds a connected-but-hung command (e.g. one blocked forever on stdin) independent of B.

### D. New terminal status `Interrupted` (= "outcome unknown")

`Interrupted` means **the outcome is unknown** — the update may have failed, partially applied,
or even succeeded with the result lost. It is **not** a cosmetically-different `Failed`
(`Failed` carries real captured failure output from the agent). This semantic is what
justifies a distinct status and what the reaper (A) and orphan recovery (E) emit.

- Add `Interrupted` to `UpdateStatus` (`crates/shared/types/src/update_status.rs`). Enum is
  `#[non_exhaustive]`, SeaORM `DeriveActiveEnum` (`string_value = "interrupted"`), and a wire
  type with `Other(String)` catch-all — follow the existing wire-safe pattern. **Terminal**:
  exclude from `unfinished()` and `host_blocking()`.
- **Migration:** add the enum value; existing rows untouched.
- **Cross-stack:** update web-api-types, OpenAPI/generated client, and the Svelte frontend
  (status badge + i18n label, distinct from "Failed"). Review all ~24 concrete-variant match
  sites flagged by the snapshot's `#[non_exhaustive]` rule; add wildcard arms with
  `tracing::warn!` where required (coding-standards.md).
- **Active-index safety (mandatory).** Two partial unique indexes hand-maintain SQL literal
  status sets. They differ **by design** (not a bug — Resolved Q3, §11):
  - `uix_update_history_host_active` (`m20260430_000003:15`) →
    `('pending','in_progress','awaiting_restart')` = **`host_blocking()`** (excludes `Queued`:
    a queued item does not block the host, so a batch can queue siblings).
  - `uix_update_history_host_software_item_active` (`m20260515_000002:23`) →
    `('queued','pending','in_progress','awaiting_restart')` = **`unfinished()`** (`Queued`
    counts as active per (host, item) to prevent duplicate triggers).

  `Interrupted` is terminal and must be **absent from both**. Add a unit test mapping **each
  index to its own set** — host index ↔ `host_blocking()`, item index ↔ `unfinished()` (as-str
  projections) — failing CI if a future variant is added without reconciling the matching
  index. Do **not** assert one shared set across both (that would wrongly flag the intentional
  `Queued` difference). This prevents the highest-severity, compiler-invisible failure mode: a
  terminal `Interrupted` (or any mis-placed status) pinning the host/item slot and blocking the
  user's re-trigger with a 409. **Do this regardless of D.**

### E. Refine the existing reconnect-orphan recovery

`mark_owned_in_progress_as_failed_on_reconnect` (`dispatch.rs:425`) is already instance-aware:
it fails only rows whose `ExecutionOwnerInstanceId` is null or `!= current` (lines 446–453), so
a **same-instance** websocket reconnect does not kill a live update — it is preserved for the
existing claim/replay path (`claim_or_replay_update_start_db`, `dispatch.rs:607`). Keep that.

- Change the orphan (different/old instance) outcome from `Failed` to **`Interrupted`** — a
  restarted agent's old in-flight updates are outcome-unknown, not known-failed. Aligns the
  semantics: agent-reported failures = `Failed`; outcome-unknown reaps (A, E) = `Interrupted`.

## 5. Detection authority model (summary)

| Failure mode                                          | Detected by               | Outcome                | Latency                        |
| ----------------------------------------------------- | ------------------------- | ---------------------- | ------------------------------ |
| Remote peer dead, agent alive                         | B (keepalive)             | agent reports `Failed` | ~`interval×max` (≈60 s)        |
| Command hung but TCP alive                            | C (interactive timeout)   | agent reports `Failed` | payload budget                 |
| Agent process restarted                               | E (instance-aware orphan) | `Interrupted`          | on reconnect                   |
| Agent fully dark past budget (sleep/crash, no return) | A (deadline backstop)     | `Interrupted`          | `started_at + timeout + grace` |
| Transient agent↔controller WS loss, update alive      | — (intentionally none)    | update continues       | n/a                            |

Nothing reaps on controller↔agent connectivity → §3 honored.

## 6. Rejected alternatives

- **Per-update execution heartbeat.** Redundant with B+C when the agent is alive, and in the
  agent-dark case its staleness needs the same wall-clock reasoning as the deadline backstop —
  while violating §3 (a WS partition stops heartbeats though the update is alive). Adds wire
  surface and a new false-positive path. Rejected.
- **Reaping on `service.last_seen_at` / missed pings.** Directly violates §3. Rejected.
- **`Failed` + recovery_hint instead of a new status.** Cheaper, but conflates "ran and
  failed" with "outcome unknown" — the latter matters for non-idempotent shell-plugin updates
  where a false "failed" invites a damaging re-run. User chose the distinct status; the
  mandatory index test (D) contains the main risk. Documented for the record.
- **Controller-backstop-only / agent-side-only.** Each leaves a real gap (slow-only, or
  blind to agent sleep). Rejected in favor of defense-in-depth.

## 7. Residual risk (accepted)

**"Succeeded but report lost."** An update that actually completed, then lost its connection
before delivering `UpdateResult`, is marked `Interrupted`. Because recovery is user-driven with
no auto version-check (user decision), the UI shows a false negative until the next scheduled
read-only version check reconciles the installed version, or the user verifies. `Interrupted`'s
"outcome unknown — verify before re-running" hint is chosen precisely to discourage a blind
re-run of a non-idempotent update. Accepted; surfaced in the UI copy and runbook.

## 8. Snapshot conformance

- Errors: `rootcause::Report` + `.context_to()?`; no `unwrap`/`expect`/`panic` in prod code.
- `#[non_exhaustive]` + wire `Other(String)` for the new `UpdateStatus` variant.
- Time tests: `start_paused` + `advance`; the reaper's injected wall-clock source satisfies the
  "never sleep on real wall-clock" rule while keeping the deadline in wall-clock.
- `parking_lot` locks if any shared state added; drop guards before `.await`.
- `#[expect(reason = …)]` for any unavoidable lint; no bare `#[allow]`.
- Read-then-write reaper transaction uses `begin_with_options(… Immediate …)` per the SQLite
  rule.
- No new external dependency (russh 0.61.2 already provides keepalive). **Deviations:** none.

## 9. Documentation deliverables

- `CONTEXT.md` — update the `UpdateStatus` glossary entry to list `Interrupted` and its
  terminal/outcome-unknown semantics.
- New ADR under `docs/adr/` — "Agent-authoritative update liveness; controller deadline
  backstop; `Interrupted` status" (architectural decision: detection authority model + §3).
- `docs/development/coding-standards.md` — note the index↔enum consistency test convention
  (active partial-index predicates derive from the active status set).
- Public docstrings — `UpdateStatus::Interrupted`, the reaper module, the injected `Clock`
  seam, and the russh keepalive constants.
- Frontend i18n/status-badge docs if any catalog exists; otherwise inline.
- Runbook note (or `docs/` ops section) — what `Interrupted` means operationally and the
  "verify installed version before re-running" guidance.
- `README` — no change expected (state machine not documented there); confirm in plan.

## 10. Deferred / out of scope

- Disk-pressure cleanup from interrupted PHS updates (orphaned `/root/.npm` cache on n8n,
  pre-update backup artifacts). Separate concern.
- Host sleep prevention / `caffeinate` guidance for self-hosted controller+agent on a laptop.
- Auto version-check reconcile after `Interrupted` (explicitly declined; user-driven only).
- Auto re-dispatch / retry (forbidden invariant).
- Tuning keepalive/grace constants per-host or making them configurable — start with fixed
  defaults (15 s × 4; 300 s grace), revisit if needed.

## 11. Resolved questions

1. **No new column.** The update budget is a single wire constant
   `uptrakit_wire::DEFAULT_UPDATE_TIMEOUT` (7200 s, `shared_types.rs:77`); the only dispatch
   site (`update_dispatch.rs:1091`) always sets `ExecuteUpdatePayload.timeout` to it — no
   per-update/per-plugin override. The reaper uses the constant against `started_at`.
   _Forward note:_ if per-plugin/per-update timeouts are ever introduced (the wire field is
   technically variable via `default_update_timeout()`), persist a `timeout_seconds` column at
   that time and have the reaper read it instead of the constant.
2. **Fixed cadence.** Reaper runs on a fixed interval (proposed 60 s) via a
   `loop { … tokio::time::sleep }`, mirroring `oauth/boot.rs:196` (30 s heartbeat). Not derived
   from outstanding deadlines.
3. **Index difference is intentional — not corrected.** The `Queued` discrepancy between the
   host index (`host_blocking()`) and item index (`unfinished()`) is by design and documented
   in `m20260515_000002`'s doc comment. No predicate change. The mandatory consistency test
   (§4.D) maps each index to its own set, codifying the difference so it can't silently drift.

## 12. Open questions

None outstanding. Ready for `/write-plan`.
