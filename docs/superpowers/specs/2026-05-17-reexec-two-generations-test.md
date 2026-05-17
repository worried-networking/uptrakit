# `reexec_two_generations_inherit_sockets` Integration Test

**Date:** 2026-05-17
**Scope:** `controller-runtime` (test-utils feature), `web-api` (test-utils routes), integration test
harness
**Effort:** Medium (~140 lines net change, seven files — four production files behind `test-utils`
feature gate, plus ~18 test struct-literal sites that need a new feature-gated field)
**Follows up:** `docs/superpowers/specs/2026-05-16-remove-listenfd-unsafe.md` — Task 5 Step 6 (not
implemented in original plan; deferred with documented rationale)

## Problem

The gen 1→2 inherited-socket path in `controller-runtime` is unexercised by integration tests.

Every existing `ControllerContainer`-based test verifies at most **one** reexec (gen 0→1). The
`WaitFor::Log("HTTPS server reusing inherited socket on")` startup guard proves only that the
first inherited-socket path works. The second call to `clear_cloexec` — on the already-inherited
socket (gen 1→2) — is the primary correctness invariant of the remove-listenfd-unsafe refactor.
It is not tested.

The code path at risk:

```rust
// lib.rs — inherited path (after into_std())
reexec::listenfd::clear_cloexec(&https_std)  // gen N → N+1; without this call every
    .map_err(...)?;                           // reexec beyond gen 0→1 fails silently
```

## Why the Original Approach Was Rejected

The initial spec proposed: host-side TOML file mutation + `bollard::Docker::kill_container`
SIGHUP to trigger config reload, relying on the triage layer to decide "reexec needed."

GAN-style Generator + Critic evaluation found three blocking issues:

| Issue | Severity | Description                                                                                                                                                                                                                                                                                                   |
| ----- | -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| CF-2  | HIGH     | `set_len(0)` → `write()` creates a torn-read window. File-watch fires mid-rewrite; controller reads zero-byte TOML; triage sees parse error, not a config diff; no reexec; test hangs for 30s.                                                                                                                |
| CF-4  | FATAL    | `nats_name` is not stored in `ControllerContainer`. The NATS container UUID is generated independently from the controller UUID — not recoverable from `container_name`. TOML rewrite cannot reconstruct the NATS URL. Would not compile, or would write a wrong URL and cause gen 1 to fail NATS connection. |
| HA-1  | MEDIUM   | Test correctness is coupled to `log.path` remaining in the reexec triage whitelist. No compile-time enforcement. A triage refactor silently breaks the test.                                                                                                                                                  |

Additional fragilities: `flush()` ≠ `fsync()` on VirtioFS (stale reads on macOS CI ~2-5%),
bollard socket path instability across Docker Desktop versions, inotify/SIGHUP double-enqueue
noise on Linux.

## Chosen Approach: `POST /test/force-reexec` HTTP Endpoint

Add a test-only HTTP endpoint that triggers a reexec unconditionally — bypassing triage entirely.
This endpoint follows the existing `test_utils.rs` pattern (`UPTRAKIT_TEST_UTILS_ENABLED` guard,
`cfg(feature = "test-utils")`).

**Mechanism:**

1. `AppStateBuilder::build()` creates `Arc<tokio::sync::Notify>` when the `test-utils` feature
   is active and `UPTRAKIT_TEST_UTILS_ENABLED=true` (same `build()`-time initialization pattern
   as `interactive_sessions`). The notify is stored in `AppState::test_reexec_notify`.
2. After `build()`, lib.rs extracts the notify and spawns a background task. The task awaits the
   notify, then calls `reexec::perform_reexec(&plan)` — no triage, no config reload, no file I/O.
3. The endpoint handler calls `notify.notify_one()` and returns 202 immediately (before exec
   fires — the task runs on a separate tokio task).
4. exec() replaces the process image; the 202 response may or may not reach the client (either
   outcome is acceptable — the test does not check the response body).
5. The new process starts, sets `UPTRAKIT_REEXEC_GENERATION = N+1` in its env, and begins
   serving on the inherited socket.
6. The test polls `wait_for_generation(N+1)` via `GET /healthz` until the
   `X-Reexec-Generation` response header shows the expected generation.

**Why this eliminates the previous issues:**

- No TOML mutation → no torn-read window (CF-2 gone)
- No bollard in the test → no socket-path instability (SF-4 gone)
- No inotify/SIGHUP → no file-watch double-enqueue noise (CF-3, SF-2 gone)
- No triage dependency → `log.path` triage whitelist irrelevant (HA-1 gone)
- No `nats_name` reconstruction needed (CF-4 gone)

**Trade-off accepted:** Four production files require changes, all behind
`cfg(feature = "test-utils")`. Additionally ~18 existing `AppState { ... }` raw struct literal
sites (across route unit tests and middleware tests) each need one new feature-gated field
added — the established pattern from `interactive_sessions`. This is the dominant implementation
cost but each site is a mechanical one-liner.

## Changes

### 1. `crates/ui/web-api/src/routes/health.rs` — `X-Reexec-Generation` header

Add `X-Reexec-Generation` to `/healthz` so the test can poll for a specific generation.
`UPTRAKIT_REEXEC_GENERATION` is already set by `perform_reexec` in the child process env.
Reading it at response time requires no `AppState` change. Preserve the existing
`#[tracing::instrument(skip_all)]` attribute.

```rust
#[tracing::instrument(skip_all)]
pub async fn healthz() -> impl IntoResponse {
    let generation: u64 = std::env::var("UPTRAKIT_REEXEC_GENERATION")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    (
        axum::http::StatusCode::OK,
        [(
            axum::http::HeaderName::from_static("x-reexec-generation"),
            generation.to_string(),
        )],
        "ok",
    )
}
```

Update the existing unit test to assert the header is present and equals `"0"`. Because the
header value is read from an environment variable, the test must first clear it to guarantee
isolation. `UPTRAKIT_REEXEC_GENERATION` could be set in a CI environment, and Rust's test runner
executes unit tests in parallel within the same process, so concurrent tests that touch env vars
can race. Use a process-global mutex to serialize env-mutation tests:

```rust
// Env vars are process-global; serialize against any other test that touches
// UPTRAKIT_REEXEC_GENERATION in the same test binary run.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
std::env::remove_var("UPTRAKIT_REEXEC_GENERATION");
// ... then call healthz() and assert X-Reexec-Generation: 0
```

**Not a public API change.** Adding a response header is backward-compatible; the OpenAPI spec
does not document response headers for `/healthz`.

### 2. `crates/ui/web-api/src/routes/test_utils.rs` — `force_reexec` endpoint

The file already carries `#![cfg(feature = "test-utils")]` at the module level. Do NOT add a
redundant per-function `#[cfg(feature = "test-utils")]` attribute — this deviates from the
pattern of the existing handlers in the file (which carry no per-function cfg).

```rust
/// Trigger an unconditional reexec without going through config-triage.
///
/// Returns 202 immediately. The reexec fires asynchronously from a background
/// task; the HTTP connection will be closed when exec() replaces the process
/// image. The caller must poll GET /healthz (checking X-Reexec-Generation) to
/// know when the new generation is ready.
///
/// Returns 404 if UPTRAKIT_TEST_UTILS_ENABLED is not "true".
/// Returns 503 if the notify handle is not installed (test-utils env not set at startup).
pub(crate) async fn force_reexec(State(state): State<Arc<AppState>>) -> Response {
    if !test_utils_allowed() {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Some(notify) = &state.test_reexec_notify {
        notify.notify_one();
        StatusCode::ACCEPTED.into_response()
    } else {
        StatusCode::SERVICE_UNAVAILABLE.into_response()
    }
}
```

### 3. `crates/ui/web-api/src/router.rs` — register route

Behind the existing `#[cfg(feature = "test-utils")]` block where test-utils routes are
registered:

```rust
.route(
    "/test/force-reexec",
    axum::routing::post(crate::routes::test_utils::force_reexec),
)
```

### 4. `crates/ui/web-api/src/app_state.rs` — `test_reexec_notify` field

**a. Add field to `AppState` struct (follows `interactive_sessions` pattern at line 304):**

```rust
/// Notified by `POST /test/force-reexec` to trigger an unconditional reexec.
/// `None` when `UPTRAKIT_TEST_UTILS_ENABLED` is not `"true"` at startup.
#[cfg(feature = "test-utils")]
pub(crate) test_reexec_notify: Option<Arc<tokio::sync::Notify>>,
```

**b. Initialize in `AppStateBuilder::build()` (follows `interactive_sessions` at line
1080-1081):**

```rust
#[cfg(feature = "test-utils")]
test_reexec_notify: if std::env::var("UPTRAKIT_TEST_UTILS_ENABLED").as_deref() == Ok("true") {
    Some(Arc::new(tokio::sync::Notify::new()))
} else {
    None
},
```

No builder setter method is needed. The value is constructed inside `build()` — the same
pattern as `interactive_sessions`. lib.rs accesses the notify from the built `Arc<AppState>`
after construction (see Change 5).

**c. Update all raw `AppState { ... }` struct literal sites (~18 sites in test modules).**

Every existing site that constructs `AppState { ... }` directly (not via `AppStateBuilder::build()`)
must add this field, following the established pattern for `interactive_sessions` (e.g. `auth.rs`
line 1095-1096):

```rust
#[cfg(feature = "test-utils")]
test_reexec_notify: None,
```

The affected files are in `crates/ui/web-api/src/` — route unit tests and middleware tests.
Run `grep -rn "Arc::new(AppState {" crates/ui/web-api/src/` to enumerate all sites. Without
this change, `cargo check --all-features` fails with "missing field `test_reexec_notify`" at
every literal site.

### 5. `crates/core/controller-runtime/src/lib.rs` — background reexec task

After the `Arc::new(state)` construction, behind `#[cfg(feature = "test-utils")]`:

```rust
#[cfg(feature = "test-utils")]
if let Some(notify) = state.test_reexec_notify.as_ref().map(Arc::clone) {
    // Call current_exe() a second time — the original was moved into ControllerReexecHook
    // at set_reexec_hook() above. PathBuf is Clone but the binding is consumed; a fresh
    // OS call is the simplest correct approach and has negligible cost at startup.
    let current_exe = std::env::current_exe()
        .map_err(|e| report!(AppError::Config(format!("resolve current_exe (test-utils): {e}"))))?;
    let plan = reexec::ReexecPlan {
        current_exe,
        config_path: config_path_for_coord.clone(),
        master_key_file: args.master_key_from.clone(),
        listener_count,
        generation: reexec::listenfd::current_generation(),
    };
    tokio::spawn(async move {
        notify.notified().await;
        // perform_reexec returns Result<Infallible, _>; Ok branch is unreachable.
        // On Err the exec syscall itself failed (e.g. binary not at path) — rare,
        // but the process stays alive and the test times out rather than hanging forever.
        match reexec::perform_reexec(&plan) {
            Ok(infallible) => match infallible {},
            Err(e) => tracing::error!(error = %e, "test-utils force_reexec: exec failed"),
        }
    });
}
```

**Why `Arc::clone` before the `if let`:** `state.test_reexec_notify` is `Option<Arc<Notify>>`.
`.as_ref().map(Arc::clone)` extracts a fresh `Arc` without consuming the field from the shared
`Arc<AppState>`. The handler also holds `Arc<AppState>` and will read the same `Option`.

**`generation` field in the plan:** `reexec::listenfd::current_generation()` reads
`UPTRAKIT_REEXEC_GENERATION` from env. Gen 0 reads 0, gen 1 reads 1, etc. When
`perform_reexec` fires it writes `generation + 1` into the child's env. Consistent with
`ControllerReexecHook`.

**`tokio::sync::Notify` semantics:** `notify_one()` stores one permit; `notified().await`
consumes it. Two `notify_one()` calls before resolution coalesce into one reexec — correct,
since the test serializes `force_reexec()` → `wait_for_generation(N+1)` → next `force_reexec()`
— the second call goes to the NEW process's freshly-initialized `Arc<Notify>`.

### 6. `crates/core/integration-tests/tests/helpers/api_client.rs` — two new methods

Both methods are added to the existing file, which already carries `#![expect(clippy::expect_used,
clippy::panic, reason = "integration test infrastructure: ...")]` at the module level. The
`.expect()` and `panic!()` calls below are covered by that existing suppression.

Both helpers use `reqwest::Client` directly rather than `UptrakitClient`. Justification:
`UptrakitClient` wraps the HTTP response and does not expose raw response headers;
`X-Reexec-Generation` is not modelled in the OpenAPI client. `reqwest` is already a
dev-dependency.

**`force_reexec` (fire-and-forget):**

```rust
/// POST /test/force-reexec — triggers an unconditional reexec.
///
/// The connection is dropped when exec() replaces the process image.
/// The result is intentionally discarded — a connection error is expected.
/// Follow with `wait_for_generation` to confirm the new generation is up.
pub(crate) async fn force_reexec(&self) {
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(5))
        .build()
        .expect("build reqwest client");
    let _ = client
        .post(format!("{}/test/force-reexec", self.base_url))
        .send()
        .await;
}
```

**`wait_for_generation`:**

```rust
/// Poll GET /healthz every 500ms until X-Reexec-Generation equals `expected`.
///
/// Connection errors are swallowed silently (the controller is briefly unreachable
/// during the reexec gap); a trace-level log is emitted so CI logs remain debuggable.
/// Panics if `timeout` elapses without seeing the expected generation.
pub(crate) async fn wait_for_generation(&self, expected: u64, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(5))
        .build()
        .expect("build reqwest client");
    let url = format!("{}/healthz", self.base_url);

    loop {
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "controller did not reach generation {} within {}s",
                expected,
                timeout.as_secs()
            );
        }
        match client.get(&url).send().await {
            Ok(resp) => {
                let gen: u64 = resp
                    .headers()
                    .get("x-reexec-generation")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                if gen == expected {
                    return;
                }
            }
            Err(e) => {
                tracing::trace!(error = %e, "wait_for_generation: connection error (expected during reexec gap)");
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
```

### 7. `crates/core/integration-tests/tests/system/controller_startup.rs` — test

Do NOT add `start_paused = true` to `#[tokio::test]`. This is a Docker-backed system test; the
tokio time driver must run in real-wall-clock mode. The `start_paused` attribute is only for tests
that advance fake time; inserting it here would freeze the `sleep` calls in `wait_for_generation`
and deadlock the test.

```rust
/// Verify that the HTTPS port remains reachable after two sequential reexecs.
///
/// Exercises the gen 1→2 inherited-socket path (clear_cloexec on an
/// already-inherited fd) — the code path added by the remove-listenfd-unsafe
/// refactor that was previously untested.
///
/// The gen 0→1 path is covered implicitly by ControllerContainer::start's
/// WaitFor guard ("HTTPS server reusing inherited socket on").
#[tokio::test]
#[ignore = "System integration test (requires uptrakit-test:latest Docker image). Run: cargo test -p uptrakit-integration-tests -- --ignored"]
async fn reexec_two_generations_inherit_sockets() {
    let network = test_network_name();
    let controller = ControllerContainer::start(&network).await;
    let client = ApiClient::new(controller.host_port());

    // Generation 0 is healthy when ControllerContainer::start returns.
    client.wait_for_generation(0, Duration::from_secs(30)).await;

    // Gen 0 → 1: force reexec via test-utils endpoint (no triage, no TOML mutation).
    client.force_reexec().await;
    client.wait_for_generation(1, Duration::from_secs(60)).await;

    // Gen 1 → 2: second reexec — exercises the inherited-socket clear_cloexec path.
    client.force_reexec().await;
    client.wait_for_generation(2, Duration::from_secs(60)).await;

    // Final sanity: HTTPS still accepts requests at generation 2.
    client.wait_for_ready(Duration::from_secs(5)).await;
}
```

Timeout is 60s per generation (not 30s): two full controller startup chains (DB init, PKI,
NATS, plugin catalog) can exceed 30s on constrained CI runners. The overall test budget is
~155s, well within the default Rust test timeout.

## File Map

| File                                                               | Change                                                                                     | Production code?           |
| ------------------------------------------------------------------ | ------------------------------------------------------------------------------------------ | -------------------------- |
| `crates/ui/web-api/src/routes/health.rs`                           | Add `X-Reexec-Generation` header; update unit test (env var isolation)                     | Yes (always)               |
| `crates/ui/web-api/src/routes/test_utils.rs`                       | Add `force_reexec` handler (no per-fn cfg)                                                 | `test-utils` feature only  |
| `crates/ui/web-api/src/router.rs`                                  | Register `/test/force-reexec` route                                                        | `test-utils` feature only  |
| `crates/ui/web-api/src/app_state.rs`                               | Add `test_reexec_notify` field + `build()` initialization; update ~18 struct literal sites | `test-utils` feature only  |
| `crates/core/controller-runtime/src/lib.rs`                        | Spawn background reexec task from `state.test_reexec_notify` after `Arc::new(state)`       | `test-utils` feature only  |
| `crates/core/integration-tests/tests/helpers/api_client.rs`        | Add `force_reexec`, `wait_for_generation`                                                  | Test-only (dev-dependency) |
| `crates/core/integration-tests/tests/system/controller_startup.rs` | Add `reexec_two_generations_inherit_sockets`                                               | Test-only (dev-dependency) |

## Safety Invariants Verified by This Test

| Invariant                                               | How verified                                            |
| ------------------------------------------------------- | ------------------------------------------------------- |
| `clear_cloexec(&https_std)` on fresh-bind path          | Every existing test — ControllerContainer WaitFor guard |
| `clear_cloexec(&https_std)` on inherited path (gen 0→1) | ControllerContainer WaitFor guard                       |
| `clear_cloexec(&https_std)` on inherited path (gen 1→2) | **`wait_for_generation(2)` in this test**               |
| HTTPS port survives two sequential `exec()` calls       | **`wait_for_ready` at generation 2 in this test**       |

**Call-ordering invariant:** `take_inherited_listeners()` (via `listenfd`) actively re-arms
`FD_CLOEXEC` on every socket it claims (`mark_cloexec` at `listenfd/src/unix.rs`). The test's
correctness therefore depends on `clear_cloexec(&https_std)` being called _after_
`take_inherited_listeners()` returns in `lib.rs`. If a refactor swaps this order, the socket
arrives at `exec()` with `FD_CLOEXEC` set again — gen 0→1 still works (fresh-bind path has no
`mark_cloexec`), but gen 1→2 fails silently. The implementation MUST preserve this ordering and
add an inline comment explaining it.

## Quality Gates

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo deny check
markdownlint --config .markdownlint.json '**/*.md'
# System integration test (requires Docker image):
docker build -f docker/Dockerfile.test -t uptrakit-test:latest . && \
  cargo test -p uptrakit-integration-tests -- --ignored reexec_two_generations_inherit_sockets
```

## Documentation Impact

`X-Reexec-Generation` is an internal diagnostics header, not part of the public API surface in
the OpenAPI spec. No ADR updates required. No external documentation changes.

## Out of Scope

- Three-or-more-generation coverage (gen 0→1→2→3) — diminishing returns after 0→1→2 covers
  both distinct code paths.
- PKI listener socket inheritance verification — same `clear_cloexec` call pattern; not a
  separate code path.
- Making `X-Reexec-Generation` part of the OpenAPI spec or visible to production clients.
- Adding `force_reexec` to any non-test code path (the `test-utils` feature gate ensures it
  cannot appear in production builds).
