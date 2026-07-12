# Agent-Side Timeout Enforcement for SSH Surface Background Tasks — Design

**Date:** 2026-07-12
**Status:** Design (pending plan)
**Scope:** `crates/core/agent-ssh-runtime/src/surface_runtime.rs` (dispatch match + 5 `spawn_*` helpers + one new
timeout-response builder + one budget-resolution helper + a `DEFAULT_SURFACE_ACTION_TIMEOUT` const). First real use
of the existing `SurfaceActionErrorCode::Timeout`. No wire change, no ADR, no dependency, no frontend change.

## Problem

Audit `audit-2026-07-11` L1295 (MEDIUM · stability · effort M · core-agent-ssh · verified): surface background
tasks spawned in `surface_runtime.rs` (bootstrap connect/execute, sync connect/execute, infra-plugin action) run
with **no agent-side timeout**, and the per-command SSH exec on those paths is also timeout-less. A live remote
peer whose command never returns (`getent` against a hung NSS/LDAP, a plugin helper blocking on the network, a sudo
misconfig) pins the spawned task **and its SSH session forever**. SSH keepalive (15 s × 4 = 60 s) only detects a
**dead** peer, not a live-but-hung command.

The interaction descriptors already declare `.with_timeout(N)` budgets (`build_actions()`,
`surface_runtime.rs:163-203`), but that budget is enforced **controller-side only** — the controller abandons the
request while leaving the agent task and its SSH session alive. A user retry then stacks another unbounded task and
another connection. The declared budget is a promise the agent never keeps.

## Verified current reality (byte-checked, 2026-07-12)

- **Dispatch** — `handle_surface_request_internal` (`surface_runtime.rs:1115-1190`) matches
  `request.interaction_id.as_str()` (`:1141`):
  - `"list-hosts"` / `"remove-host"` / `"bootstrap"` / `"sync-host"` → handled **inline** (awaited in-band, then
    `send_response`), not spawned.
  - `"bootstrap-connect"` → `spawn_bootstrap_connect` (:1268); `"bootstrap-execute"` → `spawn_bootstrap_execute`
    (:1301); `"sync-connect"` → `spawn_sync_connect` (:1348); `"sync-execute"` → `spawn_sync_execute` (:1394);
    `_` (catch-all) → `spawn_infra_plugin_action` (:1031), which serves **every** infra-plugin action id.
  - Each `spawn_*` has signature `(request: SurfaceActionRequest, ctx: &SurfaceRuntimeContext<'_>)` and internally
    does a bare `tokio::spawn(async move { … })` with **no** `tokio::time::timeout` wrapper.
- **SSH session lifecycle (the load-bearing hinge — RESOLVED)**: the session is **owned by and dropped with each
  spawned future**, not stashed in a cross-spawn registry. `bootstrap.rs` creates the session inside
  `run_bootstrap_connect` / `run_bootstrap_execute` (via `ssh_transport::connect_and_authenticate`, wrapped
  `Arc::new(session)`) and calls `SshSession::disconnect_shared(session)` before returning (`bootstrap.rs:245,286`
  for connect; `:658` for execute). `sync.rs` mirrors this: `establish_session` inside `sync_connect` / `sync_execute`,
  `disconnect_shared` at exit (`sync.rs:336` connect, `:593` execute). **Consequence:** dropping the future on
  timeout drops the owned `Arc<SshSession>`, closing the connection — the explicit `disconnect_shared` at the tail is
  simply skipped. This is exactly the leak the controller-side-only timeout cannot stop.
- **Response construction**: `make_surface_error_response(request_id, message)` (`surface_runtime.rs:2233-2243`)
  hardcodes `code: SurfaceActionErrorCode::InvalidRequest` (`:2239`) for **every** error. Spawned tasks send the
  response over `ctx.bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>` (`:973`) as
  `bg_tx.send(ServiceMessage::SurfaceActionResponse(response)).await` (e.g. `:1293-1296`).
- **`SurfaceActionErrorCode::Timeout`** (`crates/shared/surfaces/src/protocol.rs:885`) is a plain unit variant on a
  `#[derive(… Serialize, Deserialize)] #[serde(rename_all = "snake_case")]` enum → serializes as `"timeout"`, needs
  **no** wire change. Grep confirms it is **matched** in the web-api HTTP-status mapping
  (`routes/surfaces.rs`, `service_ws/handler/{audit_surface,surface_wire}.rs`) but **constructed nowhere** — this
  spec is its first producer.
- **Budgets (single source)**: declared in `build_actions()` — list-hosts 15 (`:170`), remove-host 30 (`:175`),
  bootstrap-connect 60 (`:183`), bootstrap-execute 120 (`:186`), sync-connect 60 (`:189`), sync-execute 120 (`:192`),
  plus `sync_host_action()` 120 (`:872`) and `bootstrap_action()` 120 (`:954`), plus every infra-plugin action's own
  `.with_timeout(N)`. `build_actions()` is a **pure function** returning `Vec<SurfaceActionDescriptor>` that already
  folds in infra-plugin actions via `all_descriptors()` (`:194-201`). `SurfaceActionDescriptor.timeout_seconds:
  Option<u32>` + builder `with_timeout(u32)` live at
  `crates/plugins/infrastructure/core/src/surface_form_authoring.rs`.
- **Shutdown**: `lib.rs` shutdown (`:723-820`) drains only `in_flight_updates`; surface spawns are not registered
  there (noted as out-of-scope follow-up below — the timeout already bounds the leak).

## Approach (chosen — wrap each spawn in `tokio::time::timeout`, budget from the descriptor, YAGNI)

Wrap the inner future of each of the **5 spawned** surface handlers in `tokio::time::timeout(budget, fut)`, matching
the crate's existing idiom (`ssh_transport.rs:300` exec_raw, `:866` connect, `ssh_executor.rs:125`). On elapse, send
a `SurfaceActionResponse` carrying the existing `SurfaceActionErrorCode::Timeout`; dropping the timed-out future
drops the owned `Arc<SshSession>` and closes the connection — the root fix.

### 1. Budget resolution — one helper, reads the declared descriptor (kills drift)

```rust
/// Fallback budget for a surface action whose descriptor declares no `timeout_seconds`.
/// Named so the one place it applies is greppable; chosen to match the longest declared
/// SSH action budget (120s) so an undeclared action is never cut shorter than the wizard steps.
const DEFAULT_SURFACE_ACTION_TIMEOUT: Duration = Duration::from_secs(120);

/// Resolve the agent-side timeout budget for a surface action from its declared descriptor.
/// Single source of truth: reads `build_actions()` (the same list that authors `.with_timeout(N)`
/// and that the controller enforces), so the agent budget can never drift from the declared one.
fn resolve_action_timeout(interaction_id: &str) -> Duration {
    build_actions()
        .iter()
        .find(|d| d.id == interaction_id)
        .and_then(|d| d.timeout_seconds)
        .map(|secs| Duration::from_secs(u64::from(secs)))
        .unwrap_or(DEFAULT_SURFACE_ACTION_TIMEOUT)
}
```

- Resolve **once** in the dispatch match (`handle_surface_request_internal`), before each `spawn_*` call:
  `let budget = resolve_action_timeout(request.interaction_id.as_str());` — then thread `budget: Duration` as a new
  third parameter into each of the 5 `spawn_*` helpers. One resolution site; no per-arm literal.
- **Covers the `_` infra-plugin arm uniformly**: because `build_actions()` already includes infra-plugin actions,
  `resolve_action_timeout` finds their declared budget by id with zero extra code — an infra action that declares
  `.with_timeout(N)` is enforced at `N`; one that declares none falls back to the default.
- **`build_actions()` re-runs per dispatch** — it is a cheap static assembly of descriptor structs. This is on the
  human-triggered surface-action path (not a hot loop), so the allocation is immaterial.
  `// ponytail: rebuilds the descriptor Vec per dispatch; cache in SurfaceRuntimeContext only if profiling shows it matters.`

### 2. The timeout wrap + response — one new builder, existing `Timeout` code

Add a sibling builder next to `make_surface_error_response` (do **not** change that fn's signature — its
`InvalidRequest`-hardcoding and the broader error-code taxonomy are **L1318's** job):

```rust
/// Build the timeout response for a surface action that exceeded its declared budget.
/// First producer of the (previously dormant) `SurfaceActionErrorCode::Timeout`.
fn make_surface_timeout_response(request_id: uuid::Uuid, interaction_id: &str, budget: Duration) -> SurfaceActionResponse {
    SurfaceActionResponse {
        request_id,
        success: false,
        result: None,
        error: Some(SurfaceActionError {
            code: SurfaceActionErrorCode::Timeout,
            message: format!(
                "surface action '{interaction_id}' timed out after {}s",
                budget.as_secs()
            ),
            details: None,
        }),
    }
}
```

Each `spawn_*` wraps its existing handler future:

```rust
tokio::spawn(async move {
    let response = match tokio::time::timeout(budget, run_bootstrap_execute(/* … */)).await {
        Ok(inner) => inner,                                   // handler produced its own response (Ok/Err)
        Err(_elapsed) => make_surface_timeout_response(request_id, &interaction_id, budget),
    };
    if bg_tx.send(ServiceMessage::SurfaceActionResponse(response)).await.is_err() {
        tracing::error!(%interaction_id, "failed to send surface action result via bg_tx");
    }
});
```

On the `Err(_elapsed)` branch the `run_*` future is dropped, dropping its `Arc<SshSession>` → connection closed.
The message is the same terse form each `spawn_*` already logs; keep each helper's existing label.

### Key decisions (baked in)

- **Budget from the descriptor, never a per-spawn literal** — `resolve_action_timeout` reads `build_actions()`, the
  same list the controller enforces and that authors `.with_timeout(N)`. Kills the drift the finding names.
- **Timeout wraps the whole spawned handler** (connect+exec chain), not per-command — one guard bounds the entire
  task; dropping the future releases the session. Lazy and correct.
- **Existing `Timeout` code**, existing `SurfaceActionResponse`, existing `bg_tx` send — no wire/serde/protocol
  change. New surface area is two private fns + one const, all inside `surface_runtime.rs`.
- **`make_surface_error_response` untouched** for its other callers — the InvalidRequest-for-everything taxonomy fix
  belongs to L1318; this spec only adds a dedicated timeout builder.

## Explicitly rejected (YAGNI / correctness)

- **Blanket default deadline on `exec_command_streaming`** — rejected: `exec_command_streaming(cmd, Some(tx))`
  (`ssh_transport.rs:319→347`, unbounded `while let Some(msg) = channel.wait().await`) is **also** the
  interactive/long update-output path; a default deadline there would wrongly kill legitimately-long updates. The
  spawn-level wrap already bounds the whole surface task, so a per-command deadline is redundant defense-in-depth.
  If ever needed, it must be a scoped opt-in `Option<Duration>` param on the surface exec path only (mirroring
  `exec_raw`), never a blanket default — left as a follow-up.
- **Per-op `sftp_put` / `sftp_remove` timeouts** (`ssh_transport.rs:583,619`) — deferred; the spawn-level wrap
  already bounds any handler that calls them. A dedicated per-op timeout ships with the per-command deadline if a
  single hung op ever proves to need sub-task granularity.
- **Registering surface spawns for graceful shutdown-drain** (like `in_flight_updates`) — out of scope; the timeout
  already bounds the leak to `max = budget`. Full shutdown-drain is a separate, larger change. Stated, not built.
- **Hardcoding per-spawn timeout literals** — reintroduces the drift the finding names. Rejected.
- **Refactoring `make_surface_error_response`'s error-code taxonomy** — L1318 owns it. Rejected here.
- **A new error-code variant** — `Timeout` already exists; use it. **Changing descriptor budgets or the wire
  protocol** — no.

## Tests (reuse existing `surface_runtime.rs` bg-channel scaffolding at `:2702`)

All timeout tests exercise `tokio::time::timeout` for correctness ⇒ **`#[tokio::test(start_paused = true)]` +
`tokio::time::advance()`** (repo rule — never real sleeps). Do **not** test `tokio::time::timeout` itself.

1. **Timeout fires per spawn path** (at least `bootstrap-execute` and `sync-execute`, the two 120 s actions): drive
   a spawn whose handler future never completes (a hung/mock SSH-op future), `tokio::time::advance()` past the
   budget, assert the task sends a `SurfaceActionResponse` over `bg_rx` with `success == false` and
   `error.code == SurfaceActionErrorCode::Timeout` — and that it arrives (i.e. the wrap sends a response rather than
   hanging).
2. **Budget is sourced from the descriptor, not a literal**: for an action with a known short declared budget,
   assert the hung handler times out at that budget (advance to just before → no response; advance past → Timeout).
   For an action whose descriptor declares no `timeout_seconds`, assert it falls back to
   `DEFAULT_SURFACE_ACTION_TIMEOUT`. Proves `resolve_action_timeout` reads `build_actions()`.
3. **Happy path — no false trip**: a fast-completing handler returns its normal `SurfaceActionResponse` (its own
   success/error), and **no** `Timeout` is emitted, even after advancing time past the budget.

`resolve_action_timeout` itself gets a direct unit test (known id → declared budget; unknown id → default) — pure,
no Tokio time, so **no `start_paused`** on that one (testing rule: `start_paused` only when the logic under test
calls a `tokio::time::*` API).

## Deliverables

- `crates/core/agent-ssh-runtime/src/surface_runtime.rs`:
  - Add `const DEFAULT_SURFACE_ACTION_TIMEOUT` + `fn resolve_action_timeout(&str) -> Duration` (doc-commented).
  - Add `fn make_surface_timeout_response(...)` (doc-commented); leave `make_surface_error_response` unchanged.
  - Resolve `budget` once per spawned arm in `handle_surface_request_internal`; add `budget: Duration` param to
    `spawn_bootstrap_connect` / `spawn_bootstrap_execute` / `spawn_sync_connect` / `spawn_sync_execute` /
    `spawn_infra_plugin_action`; wrap each inner handler future in `tokio::time::timeout(budget, fut)` and send the
    timeout response on `Err(_elapsed)`.
  - Add the three timeout tests + the `resolve_action_timeout` unit test.

### Documentation deliverables

- **`docs/development/surfaces.md`** (and/or **`docs/security/surfaces.md`**, action-level-timeouts section): note
  that surface action descriptor `timeout_seconds` budgets are now **enforced agent-side** (previously
  controller-only), with `SurfaceActionErrorCode::Timeout` returned to the caller on expiry and the SSH session
  closed on drop. Document the `DEFAULT_SURFACE_ACTION_TIMEOUT` fallback for descriptors that declare no budget.
- **No ADR** (bug fix / internal mechanics, not an architectural decision). **No `asyncapi.yaml` / wire change** —
  `SurfaceActionResponse` and the `Timeout` code already exist and are already serialized. **No dependency change**
  (`tokio` is a workspace dep; `tokio::time::timeout` already used in this crate). **No frontend change** — the UI
  already renders `SurfaceActionResponse` error codes; `Timeout` simply now actually arrives.

## Plan-gate (verify before/while implementing)

- **Confirm `Arc<SshSession>` closes the transport on `Drop`.** The fix relies on drop-on-timeout closing the
  connection. The session is owned inside the future (verified), so it drops on timeout — confirm the underlying
  russh handle / `SshSession` closes the TCP on its `Drop` (or that no other live `Arc` clone outlives the future).
  If a clone is held elsewhere, the timeout still returns the response promptly but the socket lingers until that
  clone drops — acceptable but note it.
- **Confirm `SurfaceActionDescriptor` exposes `id`** (or the field the resolver matches `interaction_id` against)
  publicly to `surface_runtime.rs`; adjust the `find` predicate to the actual field name.

## Sequencing / interaction (same file as two other unspecced core-agent-ssh findings — keep specs separate)

- **Land this stability fix FIRST**, before the two maintainability refactors on the same file.
- **L1318** (stringly-typed error taxonomy → typed enum + `SurfaceActionErrorCode` + audit-outcome mapping): this
  spec seeds the **first real `Timeout` producer**; when L1318 lands it should **fold** this timeout branch's
  error-code mapping into its taxonomy (and may then generalize `make_surface_error_response` to take a code, at
  which point `make_surface_timeout_response` collapses into it). Disjoint edits; rebase cleanly.
- **L1339** (split `surface_runtime.rs` 3083 lines into ~3 units): moves the dispatch/spawn code this spec edits.
  Disjoint from this change; rebase cleanly. Do it after this fix.

## Out of scope

Other unspecced Medium+ findings (L1276 release-matrix dedup, L1318 error taxonomy, L1339 file split) — separate
specs. Per-command / per-sftp-op deadlines, surface-spawn shutdown-drain registration, and any change to
`make_surface_error_response`'s other callers or to descriptor budgets / the wire protocol are all out of scope.
