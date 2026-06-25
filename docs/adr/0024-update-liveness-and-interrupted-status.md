# 0024 — Update Liveness and the `Interrupted` Status

**Date:** 2026-06-25 **Status:** Accepted

## Context

Two interactive updates (n8n, mealie) dispatched over SSH from `agent-ssh` got stuck permanently in `in_progress`.
The MacBook running **both** the controller and `agent-ssh` went to clamshell sleep mid-update (09:59:50 UTC) and
woke 10:09:36. Output froze at the same instant on both sides, the remote build was killed, and no terminal result
ever reached the controller. The rows stayed `InProgress` forever — pinning the host execution slot so the user
could not even re-trigger.

Three independent defects let a dead update masquerade as alive forever:

1. **No SSH keepalive.** `agent-ssh` built the russh client with `client::Config::default()`
   (`keepalive_interval` / `keepalive_max` unset). The interactive driver blocked on `channel.wait()` forever on a
   zombie half-open TCP connection, so the completion future never resolved and no `UpdateResult` was ever sent.
2. **No reconnect → no recovery.** The agent↔controller websocket merely _paused_ across the sleep (macOS kept the
   TCP up) and resumed on the **same** connection. The only existing recovery
   (`mark_owned_in_progress_as_failed_on_reconnect`) fires only on reconnect, so it never ran.
3. **Payload `timeout` dropped on the interactive path.** The non-interactive executor wraps the future in
   `tokio::time::timeout`; `execute_interactive` did not. The 7200 s budget was silently discarded on exactly the
   path that hung.

See `docs/superpowers/specs/2026-06-25-update-liveness-recovery-design.md` for the full incident analysis and the
rejected alternatives.

## Decision

### Authority model — the agent is the sole authority on update liveness

The agent owns the SSH session and is the **only** component that can observe whether its update is alive. The
controller↔agent link is **not** evidence of update liveness. A brief loss of connectivity between agent and
controller must NOT mark an update `Interrupted` while the update is still running on the host.

This is decisive and rejects any reaper keyed on controller-side connectivity staleness (`service.last_seen_at`,
missed pings, or a per-update heartbeat): a long but transient websocket partition with a healthy SSH session must
leave the update running. The contrarian review reached the same conclusion — a per-update heartbeat is redundant
with agent-side detection and reintroduces the very false-positive path this design exists to remove.

### The controller backstop keys only on the update's absolute budget — never on connectivity

The controller backstop (the reaper) may therefore key off **only** the update's own absolute budget —
`started_at + DEFAULT_UPDATE_TIMEOUT + grace` — which an alive update cannot legitimately exceed. It never reads
agent connectivity. The budget is the single wire constant `uptrakit_wire::DEFAULT_UPDATE_TIMEOUT` (7200 s / 2 h);
the only dispatch site always sets `ExecuteUpdatePayload.timeout` to it, so there is no per-update or per-plugin
override and **no new column** is required.

### `Interrupted` = terminal "outcome unknown", distinct from `Failed`

`Interrupted` means **the outcome is unknown** — the update may have failed, partially applied, or even succeeded
with the result lost. It is **not** a cosmetically-different `Failed`. `Failed` carries real captured failure output
the agent reported; `Interrupted` carries no such evidence. This distinction is what justifies a separate status:
for non-idempotent shell-plugin updates, a false "failed" invites a damaging blind re-run, whereas
"outcome unknown — verify before re-running" steers the operator to check first.

`Interrupted` is **terminal**: it is excluded from both `UpdateStatus::unfinished()` and
`UpdateStatus::host_blocking()`, so a reaped row frees the host execution slot and the user can re-trigger.

### Recovery is user-driven

There is no auto-retry and no auto version-check on reaping (AGENTS.md invariant: "Uptrakit updates never
automatic"). The normal scheduled read-only version check reconciles the displayed installed-version over time; the
operator decides whether and when to re-trigger.

### Three layers of defense in depth, plus orphan re-pointing

| Failure mode                                          | Detected by                         | Outcome                | Latency                        |
| ----------------------------------------------------- | ----------------------------------- | ---------------------- | ------------------------------ |
| Remote peer dead, agent alive                         | keepalive (Plan A)                  | agent reports `Failed` | ~`interval × max` (≈60 s)      |
| Command hung but TCP alive                            | interactive timeout (Plan A)        | agent reports `Failed` | payload budget                 |
| Agent process restarted                               | instance-aware orphan re-pointing   | `Interrupted`          | on reconnect                   |
| Agent fully dark past budget (sleep/crash, no return) | deadline backstop / reaper (Plan B) | `Interrupted`          | `started_at + timeout + grace` |
| Transient agent↔controller WS loss, update alive      | — (intentionally none)              | update continues       | n/a                            |

- **Layer 1 — agent SSH keepalive (Plan A).** russh `keepalive_interval` (15 s) + `keepalive_max` (4) tears down a
  zombie peer within ≈60 s; `channel.wait()` returns and the agent reports a clean terminal `Failed`. (`inactivity_timeout`
  is deliberately NOT set — it would kill a legitimately quiet-but-alive session such as a long backup that emits no
  lines.)
- **Layer 2 — interactive PTY timeout (Plan A).** `execute_interactive` is made symmetric with the non-interactive
  path: the completion future is wrapped in `tokio::time::timeout(payload_timeout, …)`, bounding a connected-but-hung
  command independent of keepalive.
- **Layer 3 — controller absolute-deadline reaper (Plan B).** A fixed-interval background sweep transitions
  `InProgress` rows with `now_utc() - started_at > DEFAULT_UPDATE_TIMEOUT + grace` to terminal `Interrupted`, keyed on
  wall-clock only.
- **Orphan re-pointing (refinement).** `mark_owned_in_progress_as_failed_on_reconnect` is instance-aware: a
  same-instance reconnect preserves a live update (claim/replay path); only rows owned by a different/old instance are
  re-pointed — and their outcome is changed from `Failed` to **`Interrupted`** (a restarted agent's old in-flight
  updates are outcome-unknown, not known-failed).

### Budget is absolute from the first execution start

`started_at` is stamped when a row first claims `InProgress`. A same-instance resume **preserves** it — it does not
grant a fresh 2 h budget. The grace exists only so the agent's own timeout (keepalive / interactive timeout) reports
first: a healthy update is killed agent-side at ≈budget, before the reaper's `budget + grace` fires.

### Late agent results win

A late authoritative agent result **upgrades** a reaped `Interrupted` row (Task 7b): a known outcome always beats
outcome-unknown. The grace window is therefore non-critical — if the agent eventually reports, the truth replaces the
backstop's guess.

### Accepted residual risk

**"Succeeded but report lost."** An update that actually completed, then lost its connection before delivering
`UpdateResult`, is marked `Interrupted`. Because recovery is user-driven with no auto version-check, the UI shows a
false negative until the next scheduled read-only version check reconciles the installed version — or until a late
agent result upgrades the row. The "outcome unknown — verify before re-running" hint is chosen precisely to discourage
a blind re-run of a non-idempotent update. Accepted; surfaced in the UI copy and these operating notes.

## Rollout ordering

Land **Plan B (reaper) before Plan A (agent timing)** so production has reaper-only finalization — a single writer,
no two-writer race — before agent timing behavior changes. Task 7b's late-result upgrade path then covers the combined
state once both ship.

**Caveat:** until Plan A's agent-side keepalive/timeout lands, there is no early agent signal before `budget + grace`,
so the premature-reap window for legitimately-long updates is **widest during the B→A gap**. Keep that gap short, or
temporarily raise `REAPER_GRACE` and tighten it once Plan A ships (a single constant).

**Accepted limitation — post-settlement batch rollup is not retro-corrected.** If a reaped `Interrupted` item is later
upgraded to `Failed` by a late agent result (Task 7b) _after_ its batch already settled (e.g. the batch reached
`Completed`, having counted the `Interrupted` item as terminal-non-failed), the batch rollup is **not** retro-corrected
— `maybe_complete_batch`'s double-write guard early-returns on an already-terminal batch. The individual row is correct
(`Failed`); only the batch summary may understate failures in this rare ordering. Accepted and documented here rather
than adding retro-correction machinery.

## Operating notes

- **What `Interrupted` means.** The controller could not confirm the update's outcome — the agent went dark past its
  time budget, or its connection was lost. The update may have partially applied, failed, or even succeeded. **Verify
  before re-running:** re-run a version check or inspect the host before re-triggering, especially for non-idempotent
  (shell-plugin) updates where a blind re-run can cause damage.
- **Backstop timing.** The backstop fires ≈`DEFAULT_UPDATE_TIMEOUT + 300s` (≈2 h 5 m) after `started_at`. Agent-side
  keepalive / interactive timeout normally reports far sooner (≈60 s for a dead peer; the payload budget for a hung
  command), so a healthy deployment rarely reaches the backstop.
- **No auto-recovery.** Reaping never re-dispatches an update and never triggers a version check. The displayed
  installed version reconciles on the next scheduled read-only check; re-triggering is always an explicit user action.

## Consequences

- An update whose execution dies silently (sleep, host crash, network partition, zombie TCP) always reaches a terminal
  state in bounded time with an honest outcome, and the host slot frees so the user can re-trigger.
- The controller never reaps on connectivity, so a transient agent↔controller partition with a live SSH session leaves
  the update running.
- A new terminal status (`Interrupted`) must stay absent from the two active partial-index status sets; this is
  compiler-invisible and is guarded by the index↔enum consistency test (see `coding-standards.md`).
- The reaper loop reads wall-clock time per tick and must never cache a monotonic `tokio::time::Instant` deadline, so a
  host sleep that freezes the loop is observed as real elapsed time on the first post-wake tick.

## Cross-references

- Spec: `docs/superpowers/specs/2026-06-25-update-liveness-recovery-design.md`
- `CONTEXT.md` — `UpdateStatus` glossary (terminal vs non-terminal)
- `docs/development/coding-standards.md` — "Active partial-index predicates must track the active status set"
- Reaper modules: `crates/ui/web-api/src/update_reaper.rs`,
  `crates/ui/web-api-queries/src/queries/update_reaper.rs`
- AGENTS.md invariant: "Uptrakit updates never automatic"
