# Agent-Side Timeout Enforcement for SSH Surface Background Tasks — Design

**Date:** 2026-07-12
**Status:** Design (pending plan)
**Scope:** `crates/core/agent-ssh-runtime/src/surface_runtime.rs` (dispatch match + 5 `spawn_*` helpers + a
`LazyLock` budget map + `resolve_action_timeout` + a `make_surface_timeout_response` builder + a generic resolve-only
`resolve_surface_task_with_timeout` seam + an `audit_and_send_surface_response` tail helper + a
`DEFAULT_SURFACE_ACTION_TIMEOUT` const). First real use of the existing `SurfaceActionErrorCode::Timeout`. No wire
change, no ADR, no dependency, no frontend change.

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

**What the agent-side timeout actually buys (honest framing).** The load-bearing benefit is **fd/session release** —
the hung task is dropped, its `Arc<SshSession>` released, its socket closed. It is **not** primarily about informing
the user: the controller runs its *own* concurrent timeout on the *same* budget (`surface-proxy/src/proxy.rs:381`),
and on elapse it sends `ControllerMessage::SurfaceActionCancel { reason: Timeout }` and removes the pending entry. So
by the time the agent's `Timeout` `SurfaceActionResponse` reaches the controller, the pending request is usually
**already gone** — `complete()` finds no pending entry and drops the response silently. The agent-side `Timeout`
response is therefore **best-effort / usually superseded**; its real job is to close the local resource leak the
controller-side timeout cannot reach. (It still fires meaningfully in the edge where the agent's budget is shorter,
or the controller's timeout has not yet elapsed.)

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
  spawned future**, not stashed in a cross-spawn registry. The `spawn_*` bodies call thin wrappers
  `run_bootstrap_connect` (`surface_runtime.rs:1632`) / `run_bootstrap_execute` (`:1694`) — which only decrypt params
  — that in turn call the **actual session-owning functions** `bootstrap::bootstrap_connect` /
  `bootstrap::bootstrap_execute` in `bootstrap.rs`. Those create the session (via
  `ssh_transport::connect_and_authenticate`, wrapped `Arc::new(session)`) and call
  `SshSession::disconnect_shared(session)` before returning (`bootstrap.rs:245,286` connect; `:658` execute).
  `sync.rs` mirrors this: `establish_session` inside `sync_connect` / `sync_execute`, `disconnect_shared` at exit
  (`sync.rs:336` connect, `:593` execute). Every `Arc::clone(&session)` is a short-lived local (executors built in the
  same scope); none is stashed longer-lived, and all SSH ops are `.await`ed in-line with **no nested `tokio::spawn`**,
  so dropping the outer future genuinely cancels mid-`.await`.
- **How drop actually closes the socket (byte-checked — abrupt, not graceful)**: `SshSession`
  (`ssh_transport.rs:250-257`) has **no `impl Drop`**. On timeout, dropping the future drops the in-flight `.await`
  and the owned `Arc<SshSession>`; the socket then closes **indirectly** — when the last russh `Handle`'s `Sender`
  clone drops, the background connection-driver task's `receiver.recv()` returns `None` and the task exits, dropping
  its owned socket. This releases the fd + task (the leak fix) but is an **abrupt socket close — no SSH `DISCONNECT`
  message is sent** to the peer, unlike the normal `disconnect_shared` path which sends
  `handle.disconnect(Disconnect::ByApplication, …)` (`ssh_transport.rs:651-654`). This is exactly the leak the
  controller-side-only timeout cannot stop; the abrupt close is acceptable for a hung task and is called out in the
  docs deliverable (a peer may log the drop as an error).
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

Wrap the **handler future** of each of the **5 spawned** surface handlers in `tokio::time::timeout(budget, fut)` via a
resolve-only seam, matching the crate's existing idiom (`ssh_transport.rs:300` exec_raw, `:866` connect,
`ssh_executor.rs:125`). On elapse, the seam returns a `SurfaceActionResponse` carrying the existing
`SurfaceActionErrorCode::Timeout`, which the spawn body then audits + sends exactly as it does the success response;
dropping the timed-out handler future drops the owned `Arc<SshSession>` and closes the connection — the root fix.

### 1. Budget resolution — cached once, reads the declared descriptor (kills drift)

Mirror the crate's existing precedent for caching `build_actions()`-derived data: `REGISTERED_INTERACTION_IDS`
(`surface_runtime.rs:66`) is a `LazyLock<BTreeSet<String>>` built from `build_actions()` and consulted on every
dispatch. Add a sibling `LazyLock` map instead of rebuilding the descriptor `Vec` per call. Read the actual field —
`SurfaceActionDescriptor.action_id: String` (`surface_form_authoring.rs:251`; existing code already keys on
`action.action_id.as_str()` at `:93`/`:122`), **not** `id`. Clamp to `1..=300 s` to match the crate's own
descriptor→`InteractionDescriptor` conversion clamp at `surface_runtime.rs:607-609` (`.clamp(1, 300)`). This is
**defensive, not a symmetry claim**: a descriptor declaring `> 300` cannot reach production anyway — the controller
**rejects** it (`InteractionDescriptor::validate`, `interaction.rs:193`; `resolve_timeout`,
`surface-proxy/src/proxy.rs:831`, both error on `> 300` rather than clamping). The agent clamp simply guarantees a
sane budget for any locally-built descriptor without a second, divergent bound:

```rust
use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Duration;

/// Fallback budget for a surface action whose descriptor declares no `timeout_seconds`.
/// Matches the longest declared SSH action budget (120s) so an undeclared action is never
/// cut shorter than the wizard steps.
const DEFAULT_SURFACE_ACTION_TIMEOUT: Duration = Duration::from_secs(120);

/// Declared agent-side timeout budgets, keyed by `action_id`. Built once from `build_actions()`
/// — the same descriptor list the controller enforces and that authors `.with_timeout(N)` —
/// mirroring `REGISTERED_INTERACTION_IDS` (:66) so per-dispatch lookup never rebuilds the Vec.
static SURFACE_ACTION_TIMEOUTS: LazyLock<HashMap<String, u32>> = LazyLock::new(|| {
    build_actions()
        .into_iter()
        .filter_map(|d| d.timeout_seconds.map(|secs| (d.action_id, secs)))
        .collect()
});

/// Resolve the agent-side timeout budget for a surface action from its declared descriptor.
/// Clamped to 1..=300s to match the wire-side conversion (`surface_runtime.rs:609`).
fn resolve_action_timeout(interaction_id: &str) -> Duration {
    SURFACE_ACTION_TIMEOUTS
        .get(interaction_id)
        .map(|&secs| Duration::from_secs(u64::from(secs.clamp(1, 300))))
        .unwrap_or(DEFAULT_SURFACE_ACTION_TIMEOUT)
}
```

- Resolve **once** in the dispatch match (`handle_surface_request_internal`), before each `spawn_*` call:
  `let budget = resolve_action_timeout(request.interaction_id.as_str());` — then thread `budget: Duration` as a new
  third parameter into each of the 5 `spawn_*` helpers. One resolution site; no per-arm literal.
- **Covers the `_` infra-plugin arm uniformly**: because `build_actions()` already folds in infra-plugin actions
  (`:194-201`), the cached map contains their declared budget by `action_id` with zero extra code — an infra action
  that declares `.with_timeout(N)` is enforced at `N`; one that declares none falls back to the default.

### 2. One extracted **resolve-only** timeout seam — testable without a live SSH session

The seam must **resolve the response only** — it wraps the handler future in `tokio::time::timeout` and *returns* a
`SurfaceActionResponse` (the handler's on completion, or a `Timeout` one on elapse). It must **not** send or audit.
This is load-bearing, not stylistic: **three** of the five spawns emit `emit_surface_mutation_audit(&bg_tx, …,
&response)` **inside** the spawn body, **between** the handler await and the `bg_tx` send — `spawn_bootstrap_execute`
(`surface_runtime.rs:1327`), `spawn_sync_execute` (`:1459`), and `spawn_infra_plugin_action` (`:1076`). (The two
connect spawns — `spawn_bootstrap_connect`, `spawn_sync_connect` — have no audit line; the `remove-host` audit at
`:1148` is the **inline** arm, awaited in-band and unaffected by this change.) If the timeout wrapped the *whole* body
(handler + audit + send), a timeout would drop the future mid-run and the audit emission would **never fire** —
leaving a hole in the audit trail for a state-changing action that timed out. Keeping the helper resolve-only means the
existing audit+send run on **both** branches: a timed-out mutation is audited with its `Timeout` response, same as a
successful one.

Resolve-only also (a) **closes the test feasibility gap** — the timeout branch is exercisable by feeding the helper a
`std::future::pending()` / `std::future::ready(...)` instead of needing a mock SSH server (none exists in the crate) —
and (b) needs no `interaction_id` clone for the four literal-id arms (they pass a `&'static str`), and no send/audit
duplication concerns (each spawn keeps its own).

Add a sibling response builder next to `make_surface_error_response` (do **not** change that fn's signature — its
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
            message: format!("surface action '{interaction_id}' timed out after {}s", budget.as_secs()),
            details: None,
        }),
    }
}

/// Await a spawned surface handler under `budget`, returning its `SurfaceActionResponse` — or a
/// `Timeout` response if the budget elapses first. RESOLVE-ONLY: the caller still owns auditing and
/// sending the returned response, so any in-body side effect (e.g. `emit_surface_mutation_audit`)
/// runs on the timeout branch too. Dropping `task` on timeout cancels its in-flight `.await`,
/// releasing the owned `Arc<SshSession>` (see "How drop closes the socket").
async fn resolve_surface_task_with_timeout<F>(
    budget: Duration,
    request_id: uuid::Uuid,
    interaction_id: &str,
    task: F,
) -> SurfaceActionResponse
where
    F: std::future::Future<Output = SurfaceActionResponse>,
{
    match tokio::time::timeout(budget, task).await {
        Ok(response) => response,
        // `Elapsed` carries no context; the interaction_id + budget in the message are the
        // actionable detail (mirrors the crate's map_err_ignore-with-reason idiom for tokio timeouts).
        Err(_elapsed) => make_surface_timeout_response(request_id, interaction_id, budget),
    }
}
```

Each `spawn_*` wraps **only its handler future** in the helper, leaving its existing audit+send lines untouched:

```rust
tokio::spawn(async move {
    let response = resolve_surface_task_with_timeout(budget, request_id, "bootstrap-execute", async move {
        // existing spawn_bootstrap_execute handler body, unchanged, ending in a `SurfaceActionResponse`
    }).await;
    // EXISTING audit + send — now run on the timeout branch too. §3 extracts this pair into
    // `audit_and_send_surface_response(&bg_tx, tenant_id, "bootstrap-execute", request_id, &params, response)`:
    emit_surface_mutation_audit(&bg_tx, tenant_id, "bootstrap-execute", request_id, &params, &response).await;
    if bg_tx.send(ServiceMessage::SurfaceActionResponse(response)).await.is_err() {
        tracing::error!("failed to send bootstrap-execute result via bg_tx");
    }
});
```

- **No `interaction_id` clone** for the four literal-id arms — they pass a `&'static str` (`"bootstrap-execute"`
  etc.); the `_` infra-plugin arm passes `&request.interaction_id` *before* `request` is consumed (or a cheap clone if
  ordering forces it). `request_id: Uuid` is `Copy`.
- **No `Send + 'static` bound** on `task` — it is `.await`ed inline inside the already-spawned block, not itself
  spawned, so the future need not be `Send`/`'static`.
- **Audit + send stay in the spawn body** (resolve-only) — this is the fix for the audit-on-timeout hole; do not move
  them into the helper. The two connect spawns (`spawn_bootstrap_connect`, `spawn_sync_connect`) have no
  `emit_surface_mutation_audit` line and simply keep their existing send.

**Per-spawn wrap boundary (not uniform — verified against source).** The wrapped `task` is **only the SSH-owning,
hangable expression that produces the terminal `SurfaceActionResponse`** — not the whole spawn body. The five spawns
differ in what precedes/follows that expression:

- `spawn_bootstrap_connect` (`:1268`) / `spawn_bootstrap_execute` (`:1301`): the whole handler body reduces to the
  connect/execute call producing a `SurfaceActionResponse`; wrap that. bootstrap-execute keeps its `:1327` audit +
  send **after** the wrap.
- `spawn_sync_connect` (`:1359`) / `spawn_sync_execute` (`:1405`): each **opens** with
  `let Some((host_id, auth_override)) = resolve_sync_auth(…).await else { return; };`. `resolve_sync_auth` (`:2172`)
  **self-sends** its own error response over `bg_tx` and the spawn `return`s — it does **not** produce a
  `SurfaceActionResponse` value. It performs **no SSH work** (only param decrypt), so it needs no timeout and **must
  stay outside the wrap** (its early-`return` returns from the spawn, not the inner future). Wrap **only** the
  `sync::sync_connect(…)` / `sync::sync_execute(…)` result→response expression that follows. For `sync-execute`, the
  `ReportPluginConfig` streaming loop (`:1434-1454`) lives inside that Ok arm and is wrapped with it (on timeout
  `sync_execute` never returned, so no reports stream — correct); its `:1459` audit + send stay **after** the wrap.
- `spawn_infra_plugin_action` (`:1031`): wrap the `for bundle … handle_service_extension_action` loop that produces
  `resp` (`:1055-1074`); its `:1076` audit + send stay **after** the wrap. (Audit only actually emits for
  `bootstrap-proxmox-guest` — `emit_surface_mutation_audit` is interaction-gated at `:2126-2133`.)

### 3. Extract the audit+send tail — so audit-on-timeout is *tested*, not just structurally argued

The audit-on-timeout guarantee lives **outside** the resolve-only helper (in the spawn body), so the helper's own
tests do not exercise it. To lock it against regression, extract the identical `emit_surface_mutation_audit(&bg_tx, …,
&response).await` + `bg_tx.send(SurfaceActionResponse(response))` tail — byte-identical across the three mutating
spawns (`bootstrap-execute` `:1327-1345`, `sync-execute` `:1459-1471`, `infra-plugin-action` `:1076-1092`, all using
the `is_err()` + `tracing::error!` send form) — into one helper:

```rust
/// Audit a completed surface mutation, then send its response over `bg_tx`.
/// Both run regardless of whether `response` is a success or a `Timeout` — so a timed-out
/// mutation is still audited. Shared tail of the three auditing surface spawns.
async fn audit_and_send_surface_response(
    bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
    tenant_id: Option<uuid::Uuid>,
    interaction_id: &str,
    request_id: uuid::Uuid,
    params: &serde_json::Value,
    response: SurfaceActionResponse,
) {
    emit_surface_mutation_audit(bg_tx, tenant_id, interaction_id, request_id, params, &response).await;
    if bg_tx.send(ServiceMessage::SurfaceActionResponse(response)).await.is_err() {
        tracing::error!(%interaction_id, "failed to send surface action result via bg_tx");
    }
}
```

Each mutating spawn becomes `let response = resolve_surface_task_with_timeout(budget, request_id, "<id>",
async { … }).await; audit_and_send_surface_response(&bg_tx, tenant_id, "<id>", request_id, &params, response).await;`.
This makes the pass-critical behavior — *a `Timeout` response still triggers the audit emit* — directly unit-testable
via a `bg_rx` (test 4), and dedups the tail. The two connect spawns do **not** use this helper (no audit); they keep
their existing bare send.

### Key decisions (baked in)

- **Budget from the descriptor, never a per-spawn literal** — `resolve_action_timeout` reads `build_actions()`, the
  same list the controller enforces and that authors `.with_timeout(N)`. Kills the drift the finding names.
- **Timeout wraps the whole spawned handler** (connect+exec chain), not per-command — one guard bounds the entire
  task; dropping the future releases the session. Lazy and correct.
- **Existing `Timeout` code**, existing `SurfaceActionResponse`, existing `bg_tx` send — no wire/serde/protocol
  change. New surface area is two private fns + one const, all inside `surface_runtime.rs`.
- **`make_surface_error_response` untouched** for its other callers — the InvalidRequest-for-everything taxonomy fix
  belongs to L1318; this spec only adds a dedicated timeout builder.
- **Resolve-only seam, audit/send stay in the spawn body** — so a timed-out **mutation** is still audited (the audit
  emission is inside the spawn body, between handler and send). Wrapping the whole body would drop the audit on
  timeout. See §2.

## Alternatives considered

- **Handle the existing `ControllerMessage::SurfaceActionCancel` instead of / in addition to a self-timeout.** The
  controller *already* sends `SurfaceActionCancel { reason: Timeout }` on its own budget elapse
  (`surface-proxy/src/proxy.rs:462`, `dispatch.rs:171`), but the agent **drops it** — `lib.rs`'s `ControllerMessage`
  match has no `SurfaceActionCancel` arm (`_ => {}`). Wiring that arm to abort the matching in-flight surface task
  would also close the leak, and *reactively* (as soon as the controller decides), without the agent re-deriving a
  budget. **Rejected as the primary fix** (kept as a complementary follow-up) because it needs a **task registry**
  keyed by `request_id` — the agent currently tracks no handle for a spawned surface task (only `in_flight_updates` is
  registered), so `SurfaceActionCancel` has nothing to abort. That registry is the *same* machinery the deferred
  surface-spawn shutdown-drain needs; both should land together. The `tokio::time::timeout` wrap is chosen here because
  it is **self-sufficient** (bounds the leak with zero new shared state), **independent of controller liveness** (fires
  even if the controller connection is gone), and matches the crate's existing timeout idiom. Wiring
  `SurfaceActionCancel` + the task registry is a valid, strictly-additive follow-up — noted, not built (YAGNI).

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

## Tests (drive the extracted helper — no live SSH needed)

The `resolve_surface_task_with_timeout` seam is what makes these constructible: there is **no mock SSH server anywhere
in the crate** (`SshSession` needs a real handshake; `bootstrap_*`/`sync_*` are concrete, non-injectable) — verified
by Review A. So the timeout tests feed the helper a synthetic `task` future, not a real handler, exercising exactly
the timeout-resolution logic this spec adds. Timeout tests call `tokio::time::timeout` ⇒
**`#[tokio::test(start_paused = true)]` + `tokio::time::advance()`** (repo rule — never real sleeps). Do **not** test
`tokio::time::timeout` itself. Because the seam is resolve-only, the tests assert on its **return value** directly —
no mpsc pair needed:

1. **Timeout fires and returns `Timeout`**: `let r = resolve_surface_task_with_timeout(budget, id,
   "bootstrap-execute", std::future::pending::<SurfaceActionResponse>()).await;` after `tokio::time::advance()` past
   `budget`; assert `r.success == false` and `r.error.code == SurfaceActionErrorCode::Timeout`. (Advance to just before
   `budget` first → the future is still pending, nothing resolves.)
2. **Happy path — no false trip**: call the helper with a ready future
   (`std::future::ready(make_surface_success_response(...))` or a ready error response); advance time past `budget`;
   assert the returned response is that exact one and `error.code != Timeout`.
3. **Budget sourced from the descriptor, clamped**: unit-test `resolve_action_timeout` directly (pure, **no Tokio
   time ⇒ no `start_paused`**): a known `action_id` (e.g. `"bootstrap-execute"`) → `Duration::from_secs(120)`; an
   unknown id → `DEFAULT_SURFACE_ACTION_TIMEOUT`; verify the `1..=300` clamp (a hypothetical `0` → `1 s`, a
   hypothetical `>300` → `300 s`) so the agent budget stays sanely bounded.
4. **A `Timeout` response is still audited** (the pass-critical guarantee, which lives *outside* the resolve-only
   helper): on a `(tx, mut rx)` mpsc pair, call `audit_and_send_surface_response(&tx, Some(tenant), "bootstrap-execute",
   id, &params, make_surface_timeout_response(id, "bootstrap-execute", budget)).await`; assert `rx` yields **first** a
   `ServiceMessage::AuditEvent` (with the timeout/failure outcome) **then** the `SurfaceActionResponse{ Timeout }`.
   Pure send/await, **no Tokio time ⇒ no `start_paused`**. This is the regression guard for the audit-on-timeout hole —
   without it, a future refactor moving the audit back inside the wrapped future would go undetected.

This covers the timeout branch, the no-false-trip branch, budget sourcing/clamping, **and audit-on-timeout** — the
entire behavior this spec introduces — without a fake SSH harness the crate does not have.

## Deliverables

- `crates/core/agent-ssh-runtime/src/surface_runtime.rs`:
  - Add imports `use std::collections::HashMap; use std::sync::LazyLock; use std::time::Duration;` (none currently
    imported in this file).
  - Add `const DEFAULT_SURFACE_ACTION_TIMEOUT`, `static SURFACE_ACTION_TIMEOUTS: LazyLock<HashMap<String, u32>>` (from
    `build_actions()`), and `fn resolve_action_timeout(&str) -> Duration` (doc-commented, clamps `1..=300`).
  - Add `fn make_surface_timeout_response(...)`, the generic resolve-only `async fn
    resolve_surface_task_with_timeout<F>(...)` seam, and the `async fn audit_and_send_surface_response(...)` tail
    helper (all doc-commented); leave `make_surface_error_response` unchanged.
  - Resolve `budget` once per spawned arm in `handle_surface_request_internal`; add `budget: Duration` param to
    `spawn_bootstrap_connect` / `spawn_bootstrap_execute` / `spawn_sync_connect` / `spawn_sync_execute` /
    `spawn_infra_plugin_action`; in each, wrap **only the SSH-owning response-producing expression** (per the §2
    per-spawn boundary — sync spawns keep `resolve_sync_auth`'s early-`return` outside the wrap) in
    `resolve_surface_task_with_timeout(budget, request_id, "<action-id>", async { … })`. The three mutating spawns
    then call `audit_and_send_surface_response(...)`; the two connect spawns keep their bare send.
  - Add the two helper-driven timeout tests, the `resolve_action_timeout` unit test, and the
    `audit_and_send_surface_response` audit-on-timeout test.

### Documentation deliverables

- **`docs/development/surfaces.md`** (and/or **`docs/security/surfaces.md`**, action-level-timeouts section): note
  that surface action descriptor `timeout_seconds` budgets are now **enforced agent-side** (previously
  controller-only), clamped to `1..=300 s`. State the primary effect plainly: on expiry the agent **drops the hung
  task and closes its SSH session** (the fd/resource-leak fix the controller-side timeout could not reach). The
  emitted `SurfaceActionErrorCode::Timeout` response is **best-effort** — because the controller runs its own timeout
  on the same budget and typically abandons the pending request first, the agent's `Timeout` response is usually
  superseded (not a guaranteed user-facing signal). State that the session is closed by an **abrupt socket drop** (no
  graceful SSH `DISCONNECT` — a peer may log it as an error), which is acceptable for a hung task. Document the
  `DEFAULT_SURFACE_ACTION_TIMEOUT` fallback for descriptors that declare no budget.
- **No ADR** (bug fix / internal mechanics, not an architectural decision). **No `asyncapi.yaml` / wire change** —
  `SurfaceActionResponse` and the `Timeout` code already exist and are already serialized. **No dependency change**
  (`tokio` is a workspace dep; `tokio::time::timeout` already used in this crate). **No frontend change** — the UI
  already renders `SurfaceActionResponse` error codes; `Timeout` simply now actually arrives.

## Plan-gate (verify before/while implementing)

- **Socket-close mechanism is indirect (verified, byte-checked).** `SshSession` has no `impl Drop`; drop-on-timeout
  releases the fd via the russh `Handle` `Sender`-drop cascade (last sender drops → driver task exits → socket
  dropped) — an abrupt close, not a graceful `DISCONNECT`. All session `Arc` clones are same-scope locals (verified in
  `bootstrap.rs`/`sync.rs`), so none outlives the future; the leak is bounded to `budget`. The plan just re-confirms
  no new long-lived `Arc<SshSession>` clone is introduced.
- **Field is `action_id`, not `id`** (`surface_form_authoring.rs:251`) — the resolver keys on `d.action_id`
  (existing code precedent at `:93`/`:122`). Already reflected in §1.
- **Per-spawn wrap boundary is non-uniform (verified — see §2).** Two sync spawns open with a `resolve_sync_auth`
  early-`return` that must stay outside the wrap; `sync-execute` streams `ReportPluginConfig` inside its Ok arm;
  infra-plugin-action wraps its bundle loop. The plan wraps only the SSH-owning response-producing expression per §2,
  not the whole spawn body.

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
