# CLI Command Correctness (tail flush + update-freeze arg-group) — Design

**Date:** 2026-07-12
**Status:** Design (pending plan)
**Scope:** `crates/ui/cli/src/commands/{tail.rs,services.rs}` + `crates/ui/cli/src/tests.rs`. No ADR, no deps,
no wire change.

## Problem

Two verified `ui-cli` Immediate-Medium findings, both "the CLI command produces the wrong observable behavior,"
merged into one CLI-layer editing pass (shared `try_parse_from` test harness).

| # | Audit | File:line | Hazard |
| - | ----- | --------- | ------ |
| 1 | L1122 | `tail.rs:69` | The SSE `tail` loop `print!`s each chunk and **never flushes** — interactive PTY prompts (no trailing newline) stay buffered, so the user sees a silent hang instead of `Continue? [y/N]:`; piped output is block-buffered (breaks `\| tee`); `print!` also **panics on a broken pipe** (`\| head`). |
| 2 | L1138 | `services.rs:184` | `update-freeze`'s `--enable`/`--disable` are a mutually-exclusive but **not-required** clap group; the dispatch **discards `disable`** and sends `enabled = enable`, so `uptrakit services update-freeze <id>` with **no flag** silently sends `enabled: false` — **unfreezing updates from an argument-less invocation**. |

## Verified current reality (byte-checked, 2026-07-12)

- **tail** (`tail.rs`): the loop is a `tokio::select!`; the Output arm (`:68-70`) is
  `Some(Ok(UpdateOutputEvent::Output(line))) => { print!("{}", line.text); }` — no flush, no import of
  `std::io::Write`. `TailResult { status: String, error: Option<String> }` (`:17-21`); the ctrl-c arm already
  breaks with `status: "detached".to_string(), error: None` (`:62-63`).
- **update-freeze** (`services.rs`): variant `UpdateFreeze { id, enable: bool, disable: bool, reason }` with
  `#[arg(long, group = "freeze_action")]` on both `enable` (`:69-70`) and `disable` (`:72-73`). clap v4 derive
  auto-creates a group referenced this way as **optional** (there is **no** `#[command(group(...))]` making it
  required — grep found no `ArgGroup` anywhere in the crate). Dispatch (`:184-189`): `disable: _` (discarded),
  `update_freeze(&id, enable, …)`. Helper `update_freeze(id, enabled: bool, …)` (`:510-522`) builds
  `SetUpdateFreezeRequest { enabled, … }` — its signature already takes the final bool, so **only the dispatch
  computes it**.
- Test harness: `crates/ui/cli/src/tests.rs` uses `Cli::try_parse_from([...])` extensively (`:39/45/52/…`);
  `try_parse_from` returns a `Result`, so `.is_err()` assertions work. clap is `clap = { version = "4",
  features = ["derive", "env", "string"] }`.

## Approach (chosen — two minimal CLI corrections, YAGNI)

### Fix 1 — `tail`: flush per event + graceful broken-pipe

Replace the `print!` Output arm with an explicit locked write + flush:

```rust
use std::io::Write; // module-level import

// Output arm:
Some(Ok(UpdateOutputEvent::Output(line))) => {
    let mut out = std::io::stdout().lock();
    if out.write_all(line.text.as_bytes()).and_then(|_| out.flush()).is_err() {
        // stdout closed (broken pipe, e.g. `| head`) — stop gracefully, don't panic
        break TailResult { status: "detached".to_string(), error: None };
    }
}
```

- **Flush per event** removes the **client-side** buffering barrier: no-newline chunks no longer sit in the
  local stdout buffer, and piped output (`| tee`) is no longer block-buffered. Per-event flush cost is negligible
  — it is dominated by the per-event SSE transport work (server serialize + network + JSON deserialize) already
  paid for each `Output` event, so the granularity adds nothing measurable even for a chatty update.
- **Graceful broken-pipe**: `write_all`/`flush` return `io::Result`; on error (closed reader) we break instead of
  letting `print!` panic. The lock is acquired inside the arm, used synchronously, and dropped at arm end — no
  `.await` is held while locked, so no deadlock with the other `select!` arms.
- **Scope of the interactive-prompt win (do not over-claim):** the client flush is **necessary** for a live
  no-newline prompt (`Continue? [y/N]:`) but **sufficient only if the upstream path streams per chunk**. The
  standard agent executor reads command output via `BufReader` + `AsyncBufReadExt` (`command.rs:104-105`) —
  **line-buffered** — so a bare prompt would stall in the agent until a newline arrives regardless of client
  flush. Making interactive prompts truly appear live also requires the interactive PTY path to forward
  pre-newline chunks; **that is the Interactive-PTY-Lifecycle spec's concern, not this CLI fix.** This fix's
  independent, unconditional wins are: killing the `| tee`/pipe block-buffering delay and the broken-pipe panic,
  and removing client-side buffering. Plan step: trace the interactive `Output`-event producer (PTY read → wire
  `Output` → SSE) and confirm whether it streams per chunk; if it line-buffers, note it as a dependency of the
  interactive-PTY work, and keep the live-prompt claim scoped accordingly.
- **`status` on broken-pipe:** `TailResult::exit_code()` maps `completed→0`, `failed→1`, **everything else → 2**
  (`tail.rs:30-35`), so reusing `"detached"` yields exit 2 — identical to any non-completion, and identical to a
  distinct `"disconnected"` would. The status string is cosmetic here (on a broken pipe stdout is closed, so the
  final status can't even be printed), so `"detached"` is acceptable; a distinct `"disconnected"` is optional if
  a JSON consumer ever distinguishes the cause. No exit-code change either way.

### Fix 2 — `update-freeze`: required arg-group + explicit `enabled`

1. Make the `freeze_action` group **required** so clap rejects the no-flag invocation. Declare the group
   explicitly on the `UpdateFreeze` variant (clap v4 derive auto-creates it only as optional otherwise):

   ```rust
   #[command(group(clap::ArgGroup::new("freeze_action").required(true).multiple(false)))]
   UpdateFreeze { id, /* #[arg(long, group="freeze_action")] enable/disable */, reason },
   ```

   (Plan verifies the exact clap-v4-derive incantation for a required group on a subcommand enum variant; the
   `#[arg(long, group="freeze_action")]` attributes on `enable`/`disable` stay.)
2. Stop discarding `disable` — compute `enabled` explicitly in the dispatch and pass it through:

   ```rust
   ServicesCommands::UpdateFreeze { id, enable, disable, reason } => {
       let enabled = enable && !disable;   // belt-and-suspenders: correct even if the group ever regresses
       let resp = update_freeze(&id, enabled, reason.as_deref(), …).await?;
       …
   }
   ```

   With the required + exclusive group, exactly one flag is set, so `enable` alone is already correct; `enable &&
   !disable` stays correct if the group constraint is ever weakened. `update_freeze`'s signature is unchanged.
   (Not `match … unreachable!()` — `unreachable!` is a `panic!` banned by the workspace clippy deny-lints.)

Plan step: grep for any construction of `ServicesCommands::UpdateFreeze` / `SetUpdateFreezeRequest` **outside** the
CLI dispatch + tests (a helper or non-CLI caller would not gain the required-flag protection — acceptable, those
paths set `enabled` directly, but confirm none relies on the old default), and confirm there is no committed
shell-completion golden/snapshot file the required group would invalidate.

## Tests

- **update-freeze (load-bearing, feasible — reuse `Cli::try_parse_from` in `tests.rs`):**
  - `["uptrakit","services","update-freeze",<UUID>]` with **no** flag → `.is_err()` (rejected).
  - `--enable` → parses; the parsed variant yields `enable=true` (enabled path).
  - `--disable` → parses; yields `disable=true` (disabled path).
  - both `--enable --disable` → `.is_err()` (mutual exclusion preserved).
  This asserts the arg-group contract — the actual new logic.
- **tail flush:** no unit test — the flush + broken-pipe branch is stdout **I/O-mode behavior** coupled to the
  real `stdout()`, not new branching logic worth a stdout-injection refactor (rejected per the repo testing
  decision-table; matches how prior I/O-mode swaps were handled). No `start_paused` (no tokio-time API added).

## Deliverables

- `crates/ui/cli/src/commands/tail.rs` — `use std::io::Write` + the locked-write/flush/broken-pipe Output arm.
- `crates/ui/cli/src/commands/services.rs` — required `freeze_action` group on `UpdateFreeze` + `enabled = enable
  && !disable` in the dispatch (stop discarding `disable`).
- `crates/ui/cli/src/tests.rs` — the `update-freeze` parse-contract tests.

**Commit granularity:** land as **two separate commits** (one per fix — the two mechanisms are unrelated and
independently revertable), per `docs/development/commit-messages.md` ("small, granular commits, focused on a
single thing"): `fix(cli): flush tail output per event and handle broken pipe` and `fix(cli): require an explicit
--enable/--disable on update-freeze`. One spec, two commits.

**Compatibility note (for the changelog/release notes):** the update-freeze fix is a deliberate **behavior
break** — `uptrakit services update-freeze <id>` with no flag was previously *accepted* (silently sending
`enabled: false`) and is now *rejected* with a parse error. This is the intended fix (fail-loud instead of a
silent unfreeze); any automation relying on the old no-flag-means-disable behavior must switch to explicit
`--disable`. Call this out in the release notes.

### Documentation deliverables

- `docs/end-user/cli-usage.md` — documents both `update-freeze` (~L108/111) and `tail` (~L634-637); its examples
  already use explicit `--enable`/`--disable`, so documented *usage* is unchanged, but add a line: `update-freeze`
  now **requires** an explicit `--enable`/`--disable` (no-flag is rejected, no longer silently unfreezes), and
  `tail` flushes output per event (no client-side buffering; `| tee` no longer delayed). Do **not** claim
  interactive prompts appear live here (that depends on the upstream interactive path — see Fix 1 scope note).
- **No ADR, wire/OpenAPI/frontend/dependency change** — `SetUpdateFreezeRequest` and the SSE stream protocol are
  unchanged; both fixes are CLI-presentation/parsing only.

## Alternatives considered

- **Model update-freeze as a required `--state <enable|disable>` ValueEnum** — rejected: a **breaking CLI-surface
  change** (existing `--enable`/`--disable` invocations break) for no benefit over making the existing group
  required.
- **A line-buffered writer wrapper / global stdout reconfiguration for tail** — rejected: a per-event flush is
  sufficient at human-scale tail throughput; a wrapper is unneeded machinery.
- **Refactor the tail loop to inject a fake stdout for testability** — rejected: over-engineering to unit-test
  I/O-mode behavior.
- **Keep discarding `disable`, rely only on the required group** — rejected in favor of the explicit `enable &&
  !disable`: cheap defense-in-depth if the group constraint is ever weakened, and it documents intent at the
  dispatch site.

## Out of scope

Other unspecced immediate-Medium findings (core-mqtt-scheduler L911, plugins-infra L1042/L1052,
ui-cli-surface-proxy L1105, web-api-routes L1226) — separate specs. No change to `SetUpdateFreezeRequest` / the
wire contract, the SSE stream protocol, or the tail detach/ctrl-c logic beyond the broken-pipe handling. No
CLI-surface redesign.
