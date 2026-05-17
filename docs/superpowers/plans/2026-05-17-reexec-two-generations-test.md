# reexec Two-Generation Integration Test — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `reexec_two_generations_inherit_sockets` integration test verifying that both the
HTTPS (lib.rs:443) and PKI (lib.rs:464) sockets remain reachable after two sequential
reexecs — exercising the gen 1→2 inherited-socket `clear_cloexec` path that was previously
untested.

**Architecture:** A test-only HTTP endpoint (`POST /test/force-reexec`, behind
`cfg(feature = "test-utils")`) triggers `reexec::perform_reexec` via a
`tokio::sync::Notify` stored in `AppState`. The test polls `GET /healthz` on both HTTPS and
plain-HTTP PKI ports, reading the `X-Reexec-Generation` response header to detect generation
transitions.

**Tech Stack:** Rust/tokio, axum, testcontainers-rs async runner, reqwest (raw, for header
access), parking_lot.

---

## File Map

| File                                                               | Role                                            |
| ------------------------------------------------------------------ | ----------------------------------------------- |
| `crates/ui/web-api/src/routes/health.rs`                           | Add `X-Reexec-Generation` header to `healthz()` |
| `crates/ui/web-api/src/app_state.rs`                               | Add `test_reexec_notify` field + `build()` init |
| `crates/ui/web-api/src/routes/test_utils.rs`                       | Add `force_reexec` handler                      |
| `crates/ui/web-api/src/router.rs`                                  | Register `/test/force-reexec` route             |
| `crates/core/controller-runtime/src/lib.rs`                        | Spawn background reexec task from notify        |
| `crates/core/integration-tests/tests/helpers/containers.rs`        | Fix PKI TOML addr; expose port 8444             |
| `crates/core/integration-tests/tests/helpers/api_client.rs`        | Add PKI field + all new helpers                 |
| `crates/core/integration-tests/tests/system/controller_startup.rs` | Add the test                                    |

---

### Task 1: Add `X-Reexec-Generation` header to `healthz` + update unit test

**Files:**

- Modify: `crates/ui/web-api/src/routes/health.rs`

- [ ] **Step 1: Replace the `healthz` function body**

  Current file: `crates/ui/web-api/src/routes/health.rs:10-13`

  ```rust
  #[tracing::instrument(skip_all)]
  pub async fn healthz() -> impl IntoResponse {
      "ok"
  }
  ```

  Replace with:

  ```rust
  /// Health check. Returns `200 OK` with body `"ok"` and the `X-Reexec-Generation`
  /// response header. The header reflects how many times the controller has re-exec'd
  /// in-process since original launch (0 = initial, 1 = first reexec, …). Internal
  /// diagnostics only — not part of the public OpenAPI spec.
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

- [ ] **Step 2: Update the unit test**

  Replace the existing `#[cfg(test)] mod tests` block (lines 56-80) with:

  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;
      use axum::Router;
      use axum::body::Body;
      use axum::routing::get;
      use http::{Request as HttpRequest, header::HeaderName};
      use tower::ServiceExt;

      #[tokio::test]
      async fn healthz_returns_ok_with_generation_header() {
          // No env var set → handler's unwrap_or(0) yields generation 0.
          // No env mutation needed; no lock required.
          let app = Router::new().route("/healthz", get(healthz));
          let response = app
              .oneshot(
                  HttpRequest::builder()
                      .uri("/healthz")
                      .body(Body::empty())
                      .unwrap(),
              )
              .await
              .unwrap();

          assert_eq!(response.status(), axum::http::StatusCode::OK);
          let gen_header = response
              .headers()
              .get(HeaderName::from_static("x-reexec-generation"))
              .expect("X-Reexec-Generation header must be present");
          assert_eq!(gen_header, "0", "generation is 0 when UPTRAKIT_REEXEC_GENERATION is unset");
      }
  }
  ```

  **Why no `ENV_LOCK`:** `std::env::remove_var` is `unsafe` since Rust 1.81 (process-env mutation
  is not thread-safe). More importantly, the unit test doesn't need to mutate the env at all —
  the handler calls `std::env::var("UPTRAKIT_REEXEC_GENERATION").ok().and_then(...).unwrap_or(0)`,
  which returns `0` when the variable is absent. In the integration test (Task 7) the actual
  generation values are validated against real reexec transitions.

- [ ] **Step 3: Run the unit test**

  ```bash
  cargo test -p uptrakit-web-api --all-features health -- --nocapture
  ```

  Expected: `healthz_returns_ok_with_generation_header ... ok`

- [ ] **Step 4: Run quality gates**

  ```bash
  cargo fmt --all
  cargo clippy -p uptrakit-web-api --all-features
  ```

  Expected: no warnings, no errors.

- [ ] **Step 5: Commit**

  ```bash
  git commit --only crates/ui/web-api/src/routes/health.rs \
    -m "feat(web-api): add X-Reexec-Generation header to healthz"
  ```

---

### Task 2: Add `test_reexec_notify` to `AppState` and fix struct literal sites

**Files:**

- Modify: `crates/ui/web-api/src/app_state.rs`

- [ ] **Step 1: Add the field to `AppState` struct**

  In `app_state.rs`, after the `interactive_sessions` field (line ~305):

  ```rust
  /// Registry of active interactive update sessions (single-writer enforcement).
  #[cfg(feature = "interactive")]
  pub interactive_sessions: crate::interactive_sessions::InteractiveSessionRegistry,
  ```

  Add immediately after:

  ```rust
  /// Notified by `POST /test/force-reexec` to trigger an unconditional reexec.
  /// `None` when `UPTRAKIT_TEST_UTILS_ENABLED` is not `"true"` at startup.
  #[cfg(feature = "test-utils")]
  pub(crate) test_reexec_notify: Option<Arc<tokio::sync::Notify>>,
  ```

- [ ] **Step 2: Initialize in `AppStateBuilder::build()`**

  In `app_state.rs`, after the `interactive_sessions` initialization (line ~1081):

  ```rust
  #[cfg(feature = "interactive")]
  interactive_sessions: crate::interactive_sessions::InteractiveSessionRegistry::new(),
  ```

  Add immediately after:

  ```rust
  #[cfg(feature = "test-utils")]
  test_reexec_notify: if std::env::var("UPTRAKIT_TEST_UTILS_ENABLED").as_deref()
      == Ok("true")
  {
      Some(Arc::new(tokio::sync::Notify::new()))
  } else {
      None
  },
  ```

- [ ] **Step 3: Discover all struct literal sites that need the new field**

  ```bash
  # Save the file list — needed for the commit in Step 6.
  cargo check --all-features 2>&1 \
    | grep "missing field \`test_reexec_notify\`" -A1 \
    | grep " --> " \
    | sed 's/.*--> \(.*\):[0-9]*:[0-9]*/\1/' \
    | sort -u \
    > /tmp/reexec_sites.txt
  cat /tmp/reexec_sites.txt
  ```

  Each line is a raw `AppState { ... }` struct literal site that does NOT use
  struct-update syntax (`..`). Sites using `..(*state).clone()` or similar auto-inherit
  the new field and will NOT appear in this list.

- [ ] **Step 4: Add the field to each failing site**

  For each file reported by the compiler, add the following line inside the `AppState { ... }`
  literal, adjacent to the `interactive_sessions` initialization:

  ```rust
  #[cfg(feature = "test-utils")]
  test_reexec_notify: None,
  ```

  Typically affects `~11–17` sites across `crates/ui/web-api/src/routes/`,
  `crates/ui/web-api/src/middleware/`, and `crates/ui/web-api/src/test_harness/`.
  The exact count is determined by the compiler in Step 3; trust the list over the estimate.

- [ ] **Step 5: Verify compilation**

  ```bash
  cargo check --no-default-features --features db-sqlite
  cargo check --all-features
  ```

  Expected: zero errors, zero warnings.

- [ ] **Step 6: Commit**

  Commit `app_state.rs` plus exactly the files discovered in Step 3 (no more, no less):

  ```bash
  git commit --only crates/ui/web-api/src/app_state.rs \
    $(cat /tmp/reexec_sites.txt) \
    -m "feat(web-api): add test_reexec_notify field to AppState (test-utils gate)"
  ```

---

### Task 3: Add `force_reexec` handler and register route

**Files:**

- Modify: `crates/ui/web-api/src/routes/test_utils.rs`
- Modify: `crates/ui/web-api/src/router.rs`

- [ ] **Step 1: Add `force_reexec` handler to `test_utils.rs`**

  Append to the end of `crates/ui/web-api/src/routes/test_utils.rs` (before the closing brace
  of the `#![cfg(feature = "test-utils")]` module). Do NOT add a per-function
  `#[cfg(feature = "test-utils")]` — the module-level `#![cfg(...)]` on line 6 already gates
  the entire file.

  ```rust
  /// Trigger an unconditional reexec without going through config-triage.
  ///
  /// Returns 202 immediately. The reexec fires asynchronously from a background
  /// task; the HTTP connection will be closed when exec() replaces the process
  /// image. The caller must poll GET /healthz (checking X-Reexec-Generation) to
  /// know when the new generation is ready.
  ///
  /// Returns 404 if UPTRAKIT_TEST_UTILS_ENABLED is not "true".
  /// Returns 503 if the notify handle is not installed (env var not set at startup).
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

- [ ] **Step 2: Register the route in `router.rs`**

  In `crates/ui/web-api/src/router.rs`, inside the existing `#[cfg(feature = "test-utils")]`
  block (lines 973-983), add a third route after the two existing ones:

  ```rust
  router = router.route(
      "/test/force-reexec",
      axum::routing::post(crate::routes::test_utils::force_reexec),
  );
  ```

  The final block should look like:

  ```rust
  #[cfg(feature = "test-utils")]
  {
      router = router.route(
          "/api/v1/test/services/{id}/request-renewal",
          axum::routing::post(crate::routes::test_utils::request_service_renewal),
      );
      router = router.route(
          "/api/v1/test/services/{id}/disconnect",
          axum::routing::post(crate::routes::test_utils::disconnect_service),
      );
      router = router.route(
          "/test/force-reexec",
          axum::routing::post(crate::routes::test_utils::force_reexec),
      );
  }
  ```

  **Note on route prefix:** The two existing test-utils routes use `/api/v1/test/` prefix because
  they are consumed via the typed OpenAPI client. `/test/force-reexec` deliberately omits the
  prefix because it is called via raw reqwest (the OpenAPI client does not expose it), matching
  the same convention as `GET /healthz`. This is intentional — do not add `/api/v1/`.

- [ ] **Step 3: Verify compilation**

  ```bash
  cargo check --all-features -p uptrakit-web-api
  cargo clippy --all-features -p uptrakit-web-api
  ```

  Expected: no errors, no warnings.

- [ ] **Step 4: Commit**

  ```bash
  git commit --only \
    crates/ui/web-api/src/routes/test_utils.rs \
    crates/ui/web-api/src/router.rs \
    -m "feat(web-api): add POST /test/force-reexec endpoint (test-utils gate)"
  ```

---

### Task 4: Spawn background reexec task in `lib.rs`

**Files:**

- Modify: `crates/core/controller-runtime/src/lib.rs`

- [ ] **Step 1: Insert the background task block**

  In `lib.rs`, after line 937 (the `let app_state = Arc::new(builder.build()...)` block), add:

  ```rust
  #[cfg(feature = "test-utils")]
  if let Some(notify) = app_state.test_reexec_notify.as_ref().map(Arc::clone) {
      // current_exe was moved into ControllerReexecHook at set_reexec_hook() above.
      // Call current_exe() a second time — the OS call is cheap at startup.
      let current_exe = std::env::current_exe().map_err(|e| {
          report!(AppError::Config(format!(
              "resolve current_exe (test-utils): {e}"
          )))
      })?;
      let plan = reexec::ReexecPlan {
          current_exe,
          config_path: config_path_for_coord.clone(),
          master_key_file: args.master_key_from.clone(),
          listener_count,
          generation: reexec::listenfd::current_generation(),
      };
      tokio::spawn(async move {
          notify.notified().await;
          tracing::warn!(
              "test-utils force_reexec: triggering unconditional reexec at generation {}; \
               a concurrent coordinator-driven reexec at this moment would produce an \
               unexpected generation number in the integration test",
              plan.generation
          );
          // Brief pause to allow the 202 ACCEPTED response to be flushed by the HTTP
          // layer before exec() replaces the process image. Without this, the response
          // can be dropped by the kernel mid-send on multi-threaded runtimes.
          tokio::time::sleep(std::time::Duration::from_millis(50)).await;
          // perform_reexec returns Result<Infallible, _>; the Ok branch is unreachable.
          // On Err, exec() itself failed (binary not at path) — process stays alive
          // and the integration test times out rather than hanging forever.
          match reexec::perform_reexec(&plan) {
              Ok(infallible) => match infallible {},
              Err(e) => tracing::error!(error = %e, "test-utils force_reexec: exec failed"),
          }
      });
  }
  ```

  **Insertion context:** place this block immediately after the closing `)?;` of
  `Arc::new(builder.build()...)`. The variables `config_path_for_coord`, `args`,
  `listener_count`, and `reexec` module are all in scope at this point.

- [ ] **Step 2: Add call-ordering invariant comments to the two `clear_cloexec` call sites**

  The spec (Safety Invariants) requires inline comments at both `clear_cloexec` call sites
  explaining the ordering constraint. Locate the two existing calls in `lib.rs`:
  - **lib.rs:443** — HTTPS socket:

    ```rust
    // ORDERING: call clear_cloexec AFTER take_inherited_listeners(); listenfd
    // re-arms FD_CLOEXEC on every claimed socket, so clearing it again here is
    // required to ensure the fd survives exec() in subsequent reexec generations.
    clear_cloexec(&https_std);
    ```

  - **lib.rs:464** — PKI socket:

    ```rust
    // ORDERING: same invariant as the HTTPS clear_cloexec above (lib.rs:443).
    // Both sockets must be cleared after take_inherited_listeners().
    clear_cloexec(&pki_std);
    ```

  The exact line numbers may shift as you insert the background task block in Step 1. Search
  for the two `clear_cloexec(` call sites in `lib.rs` and add the comments immediately above
  each one.

- [ ] **Step 3: Verify compilation**

  ```bash
  cargo check --no-default-features --features db-sqlite -p uptrakit-controller-runtime
  cargo check --all-features -p uptrakit-controller-runtime
  cargo clippy --all-features -p uptrakit-controller-runtime
  ```

  Expected: no errors. If `config_path_for_coord` or `listener_count` is out of scope, check
  that the insertion point is before any `move` closures that might have captured them.

- [ ] **Step 4: Commit**

  ```bash
  git commit --only crates/core/controller-runtime/src/lib.rs \
    -m "feat(controller-runtime): spawn force_reexec background task; document clear_cloexec call-ordering invariant (test-utils gate)"
  ```

---

### Task 5: Fix PKI TOML addr and expose port 8444 in `ControllerContainer`

**Files:**

- Modify: `crates/core/integration-tests/tests/helpers/containers.rs`

- [ ] **Step 1: Fix the PKI TOML address**

  In `containers.rs`, change line 106:

  ```toml
  addr = "[::]:8444"
  ```

  to:

  ```toml
  addr = "http://[::]:8444"
  ```

  **Why:** The startup validation at `startup/validation.rs:57-58` gates the PKI HTTP listener
  on `url.starts_with("http://")`. Without the `http://` prefix, `pki_http_port = None`,
  the listener is never started, and port 8444 is never bound. This change also sets
  `listener_count = 2` so both HTTPS and PKI sockets are passed via `LISTEN_FDS` and inherited
  on reexec.

- [ ] **Step 2: Add the `PKI_PORT` constant**

  After line 37 (`const NATS_PORT: u16 = 4222;`), add:

  ```rust
  /// PKI plain-HTTP port inside the container.
  const PKI_PORT: u16 = 8444;
  ```

- [ ] **Step 3: Expose PKI port in `GenericImage` builder**

  In `start_internal`, the `GenericImage` builder chain starts at line 125. Add
  `.with_exposed_port(PKI_PORT.tcp())` immediately after the existing
  `.with_exposed_port(CONTROLLER_PORT.tcp())` on line 126:

  ```rust
  let container = GenericImage::new(TEST_IMAGE, TEST_IMAGE_TAG)
      .with_exposed_port(CONTROLLER_PORT.tcp())
      .with_exposed_port(PKI_PORT.tcp())   // ← add this line
      .with_wait_for(WaitFor::Log(
          ...
  ```

  `GenericImage` methods (`.with_exposed_port`, `.with_wait_for`) must precede `ImageExt`
  methods (`.with_cmd`, `.with_mount`, etc.) because `ImageExt` consumes `GenericImage`
  into `ContainerRequest`. Do not place this line after `.with_cmd`.

- [ ] **Step 4: Add `pki_host_port` field to `ControllerContainer`**

  In the `ControllerContainer` struct (lines 42-55), add after `host_port`:

  ```rust
  /// Host port mapped to the controller's PKI plain-HTTP port.
  pki_host_port: u16,
  ```

- [ ] **Step 5: Query the mapped PKI port after container starts**

  After the existing `get_host_port_ipv4(CONTROLLER_PORT...)` call (lines 161-164), add:

  ```rust
  let pki_host_port = container
      .get_host_port_ipv4(PKI_PORT.tcp())
      .await
      .expect("get PKI mapped port");
  ```

- [ ] **Step 6: Add `pki_host_port` to the struct literal and add accessor**

  In the `Self { ... }` construction (lines 171-178), add `pki_host_port,` after `host_port,`.

  After the `host_port()` accessor method (lines 182-184), add:

  ```rust
  /// The host port mapped to the controller's PKI plain-HTTP port.
  pub(crate) fn pki_host_port(&self) -> u16 {
      self.pki_host_port
  }
  ```

- [ ] **Step 7: Verify compilation**

  ```bash
  cargo check --all-features -p uptrakit-integration-tests
  ```

  Expected: no errors.

- [ ] **Step 8: Commit**

  ```bash
  git commit --only crates/core/integration-tests/tests/helpers/containers.rs \
    -m "test(integration-tests): enable PKI HTTP listener and expose port 8444 in ControllerContainer"
  ```

---

### Task 6: Add PKI helpers and reexec helpers to `ApiClient`

**Files:**

- Modify: `crates/core/integration-tests/tests/helpers/api_client.rs`

The file already has `#![expect(clippy::expect_used, clippy::panic, reason = "...")]` at the
module level — all `.expect()` and `panic!()` calls below are covered by that suppression.

- [ ] **Step 1: Add `pki_base_url` field and update `new()`**

  Add `pki_base_url: Option<String>` to the `ApiClient` struct (after `client: Option<UptrakitClient>`):

  ```rust
  pub(crate) struct ApiClient {
      base_url: String,
      client: Option<UptrakitClient>,
      pki_base_url: Option<String>,
  }
  ```

  Update `new()` to initialize it:

  ```rust
  pub(crate) fn new(controller_port: u16) -> Self {
      let base_url = format!("https://127.0.0.1:{controller_port}");
      Self {
          base_url,
          client: None,
          pki_base_url: None,
      }
  }
  ```

- [ ] **Step 2: Add `with_pki_port` builder method**

  After `new()` in the `impl ApiClient` block:

  ```rust
  /// Set the PKI plain-HTTP port. Required before calling `wait_for_pki_generation`.
  pub(crate) fn with_pki_port(mut self, pki_port: u16) -> Self {
      self.pki_base_url = Some(format!("http://127.0.0.1:{pki_port}"));
      self
  }
  ```

- [ ] **Step 3: Add `force_reexec` helper**

  After `with_pki_port`:

  ```rust
  /// POST /test/force-reexec — triggers an unconditional reexec.
  ///
  /// The endpoint responds with 202 ACCEPTED before the background task calls exec().
  /// If the response is received, assert it is 2xx to catch mis-wired routes early.
  /// Connection errors after a 2xx response (exec drop) are ignored.
  /// Follow with `wait_for_generation` to confirm the new generation is up.
  pub(crate) async fn force_reexec(&self) {
      let client = reqwest::Client::builder()
          .danger_accept_invalid_certs(true)
          .connect_timeout(std::time::Duration::from_secs(5))
          .timeout(std::time::Duration::from_secs(5))
          .build()
          .expect("build reqwest client");
      match client
          .post(format!("{}/test/force-reexec", self.base_url))
          .send()
          .await
      {
          Ok(resp) => assert!(
              resp.status().is_success(),
              "force_reexec: expected 2xx, got {} — \
               check UPTRAKIT_TEST_UTILS_ENABLED and route registration",
              resp.status()
          ),
          // Connection reset/EOF after exec() replaces the process image is expected.
          Err(e) => tracing::trace!(error = %e, "force_reexec: connection dropped (expected)"),
      }
  }
  ```

- [ ] **Step 4: Add `wait_for_generation` helper**

  ```rust
  /// Poll GET /healthz every 500ms until X-Reexec-Generation equals `expected`.
  ///
  /// Connection errors are logged at trace level (expected during the reexec gap).
  /// Panics if `timeout` elapses without seeing the expected generation.
  pub(crate) async fn wait_for_generation(&self, expected: u64, timeout: Duration) {
      let deadline = tokio::time::Instant::now() + timeout;
      let client = reqwest::Client::builder()
          .danger_accept_invalid_certs(true)
          .connect_timeout(Duration::from_secs(5))
          .timeout(Duration::from_secs(5))
          .build()
          .expect("build reqwest client");
      let url = format!("{}/healthz", self.base_url);

      loop {
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
                  tracing::trace!(
                      error = %e,
                      "wait_for_generation: connection error (expected during reexec gap)"
                  );
              }
          }
          if tokio::time::Instant::now() >= deadline {
              panic!(
                  "controller did not reach generation {} within {}s",
                  expected,
                  timeout.as_secs()
              );
          }
          tokio::time::sleep(Duration::from_millis(500)).await;
      }
  }
  ```

- [ ] **Step 5: Add `wait_for_pki_generation` helper**

  ```rust
  /// Poll GET /healthz on the PKI plain-HTTP port every 500ms until
  /// X-Reexec-Generation equals `expected`.
  ///
  /// PKI server is plain HTTP — no TLS required.
  /// Panics if `with_pki_port` was not called, or if `timeout` elapses.
  pub(crate) async fn wait_for_pki_generation(&self, expected: u64, timeout: Duration) {
      let pki_base = self
          .pki_base_url
          .as_deref()
          .expect("pki_base_url not set — call with_pki_port first");
      let url = format!("{pki_base}/healthz");
      let deadline = tokio::time::Instant::now() + timeout;
      let client = reqwest::Client::builder()
          .connect_timeout(Duration::from_secs(5))
          .timeout(Duration::from_secs(5))
          .build()
          .expect("build reqwest client");

      loop {
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
                  tracing::trace!(
                      error = %e,
                      "wait_for_pki_generation: connection error (expected during reexec gap)"
                  );
              }
          }
          if tokio::time::Instant::now() >= deadline {
              panic!(
                  "PKI server did not reach generation {} within {}s",
                  expected,
                  timeout.as_secs()
              );
          }
          tokio::time::sleep(Duration::from_millis(500)).await;
      }
  }
  ```

- [ ] **Step 6: Verify compilation**

  ```bash
  cargo check --all-features -p uptrakit-integration-tests
  cargo clippy --all-features -p uptrakit-integration-tests
  ```

  Expected: no errors, no warnings.

- [ ] **Step 7: Commit**

  ```bash
  git commit --only crates/core/integration-tests/tests/helpers/api_client.rs \
    -m "test(integration-tests): add force_reexec, wait_for_generation, wait_for_pki_generation to ApiClient"
  ```

---

### Task 7: Add the integration test

**Files:**

- Modify: `crates/core/integration-tests/tests/system/controller_startup.rs`

- [ ] **Step 1: Add the test function**

  Append to `controller_startup.rs`:

  ```rust
  /// Verify that the HTTPS and PKI ports remain reachable after two sequential reexecs.
  ///
  /// ControllerContainer::start() returns only after the WaitFor guard fires on
  /// "HTTPS server reusing inherited socket on" — that message is emitted at gen 1
  /// (the first reexec the controller does as part of its normal startup). So when
  /// this test begins, the controller is already at generation 1. The gen 0→1 path is
  /// implicitly covered by the WaitFor; this test explicitly covers gen 1→2 and gen 2→3.
  ///
  /// Call sites exercised:
  ///   - lib.rs clear_cloexec(&https_std) — verified by wait_for_generation(2) and (3)
  ///   - lib.rs clear_cloexec(&pki_std)   — verified by wait_for_pki_generation(2) and (3)
  ///
  /// Note: do NOT add start_paused = true — this is a Docker-backed system test
  /// that must run on real wall-clock time. start_paused would freeze tokio::time::sleep
  /// calls in the helpers and deadlock the test.
  #[tokio::test]
  #[ignore = "System integration test (requires uptrakit-test:latest Docker image). Run: cargo test -p uptrakit-integration-tests -- --ignored reexec_two_generations_inherit_sockets"]
  async fn reexec_two_generations_inherit_sockets() {
      let network = test_network_name();
      let controller = ControllerContainer::start(&network).await;
      let client = ApiClient::new(controller.host_port())
          .with_pki_port(controller.pki_host_port());

      // Baseline: ControllerContainer::start() returns at gen 1 (WaitFor fires on the
      // "HTTPS server reusing inherited socket on" log line, which only appears after the
      // first reexec). Confirm both ports are up before issuing force_reexec.
      client.wait_for_generation(1, Duration::from_secs(30)).await;
      client.wait_for_pki_generation(1, Duration::from_secs(30)).await;

      // Gen 1 → 2: exercises the inherited-socket clear_cloexec path for the first time
      // (socket was inherited at gen 1; must survive another exec at gen 2).
      client.force_reexec().await;
      client.wait_for_generation(2, Duration::from_secs(60)).await;
      client.wait_for_pki_generation(2, Duration::from_secs(60)).await;

      // Gen 2 → 3: second explicitly-triggered reexec — double-checks that the
      // clear_cloexec path is idempotent across multiple generations.
      client.force_reexec().await;
      client.wait_for_generation(3, Duration::from_secs(60)).await;
      client.wait_for_pki_generation(3, Duration::from_secs(60)).await;

      // Final sanity: HTTPS still accepts requests at generation 3.
      client.wait_for_ready(Duration::from_secs(5)).await;
  }
  ```

- [ ] **Step 2: Verify compilation (non-Docker)**

  ```bash
  cargo check --all-features -p uptrakit-integration-tests
  cargo clippy --all-features -p uptrakit-integration-tests
  ```

  Expected: no errors, no warnings.

- [ ] **Step 3: Run quality gates (full suite)**

  ```bash
  cargo fmt --all
  cargo check --no-default-features --features db-sqlite
  cargo check --all-features
  cargo clippy --all-targets --no-default-features --features db-sqlite
  cargo clippy --all-targets --all-features
  cargo test --all-features
  cargo deny check
  markdownlint --config .markdownlint.json '**/*.md'
  ```

  Expected: all pass. The new integration test is `#[ignore]` so it does not run in `cargo test`.

- [ ] **Step 4: Run the integration test with Docker**

  Build the Docker test image first:

  ```bash
  docker build -f docker/Dockerfile.test -t uptrakit-test:latest .
  ```

  Then run the specific test:

  ```bash
  cargo test -p uptrakit-integration-tests -- --ignored reexec_two_generations_inherit_sockets 2>&1
  ```

  Expected output contains:
  - `reexec_two_generations_inherit_sockets ... ok`

  If the test hangs at `wait_for_generation(1)` for >30s: the controller did not reach gen 1
  before this check. This should be impossible because `ControllerContainer::start()` already
  waits for the "HTTPS server reusing inherited socket on" log line (gen 1). Investigate the
  container startup sequence.

  If the test hangs at `wait_for_generation(2)` for >60s: the `force_reexec` endpoint is not
  triggering a reexec. Check that `UPTRAKIT_TEST_UTILS_ENABLED=true` is set (it is, at
  `containers.rs:153`) and that the controller binary was built with `--features test-utils`.
  The `force_reexec()` helper now asserts 2xx — if it panicked before the hang, the route is
  mis-wired.

  If `wait_for_pki_generation(1)` (the baseline check) is slow (> 10s): this is the PKI
  listener racing to bind after the HTTPS gen-1 "reusing inherited socket" message. The WaitFor
  gate only waits for HTTPS; PKI may lag slightly. This is not a correctness failure — the 30s
  window is intentionally generous to absorb this race.

  If `wait_for_pki_generation(2)` times out but `wait_for_generation(2)` passes: the PKI
  listener is not starting after the forced reexec. Verify the TOML change
  (`addr = "http://[::]:8444"`) and that `PKI_PORT` is exposed and mapped.

- [ ] **Step 5: Commit**

  ```bash
  git commit --only crates/core/integration-tests/tests/system/controller_startup.rs \
    -m "test(integration-tests): add reexec_two_generations_inherit_sockets"
  ```

---

## Documentation Impact

The spec declares: `X-Reexec-Generation` is an internal diagnostics header, not part of the
public API surface in the OpenAPI spec. No ADR updates required. No external documentation
changes. This justification is carried forward as-is.

## Post-Draft Idiom Audit

- `parking_lot::Mutex` used (not `std::sync::Mutex`) ✓
- No `#[allow(...)]` — module-level `#![expect(...)]` in api_client.rs and containers.rs
  covers all `.expect()`/`panic!()` in test helpers ✓
- `tokio::sync::Notify` is the correct async primitive for handler→background-task signaling;
  no `Arc<Mutex<bool>>` polling needed ✓
- No new external dependencies introduced; `reqwest` and `tokio` are already workspace deps ✓
- Unit test does not mutate process environment (`std::env::set/remove_var` is `unsafe` since
  Rust 1.81); handler's `unwrap_or(0)` makes the zero-generation case testable without env
  mutation ✓
- No `parking_lot::Mutex` guard held across `.await`; ENV_LOCK removed from unit test ✓
- All raw `reqwest::Client` builders set both `.connect_timeout()` and `.timeout()` ✓
- All commit steps use `git commit --only <paths>` (not `git add` + `git commit`) ✓
- Bare `Arc::clone` / `Arc::new` used (no unnecessary `std::sync::` qualification) ✓
- Inline call-ordering invariant comments added at both `clear_cloexec` call sites (spec
  Safety Invariants requirement) ✓
- Test generation baseline corrected: `ControllerContainer::start()` returns at gen 1
  (WaitFor fires on "HTTPS server reusing inherited socket on"); baseline assertions start
  at `wait_for_generation(1)`, not `(0)` ✓
- `force_reexec()` asserts 2xx on received responses (routes mis-wired detected in < 5s,
  not after 60s timeout) ✓
- 50ms sleep in background task before `perform_reexec()` prevents exec() from racing with
  HTTP 202 response flush on multi-threaded runtimes ✓
- Polling deadline check moved to after the failed poll and before the sleep — avoids
  panicking on exactly one sleep interval past deadline when the next poll would have
  succeeded ✓

## Dependency Version Audit

No new external dependencies. All crates used (`reqwest`, `tokio`, `testcontainers`,
`parking_lot`) are existing workspace dependencies — no version changes required.
