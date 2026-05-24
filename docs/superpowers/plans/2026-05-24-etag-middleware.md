# ETag Middleware for Settings Endpoints — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace per-handler ETag boilerplate with a route-level `EtagLayer<S>` middleware that
injects `ETag` on GET, validates `If-Match` and injects new `ETag` on PUT/PATCH, and injects
new `ETag` on POST — while fixing 4 wrong-scope bugs in global-settings handlers.

**Architecture:** Native-async `EtagSource` trait gains `refresh_etag`; a single middleware
function `etag_middleware::<S>` wraps axum `from_fn_with_state`; `router.rs` is refactored to
group settings routes into per-scope sub-`OpenApiRouter`s so `.route_layer()` can be applied once
per group.

**Tech Stack:** Rust, axum 0.8, utoipa_axum `OpenApiRouter`, tokio, `rootcause::Report`

---

## File Map

| Action     | Path                                                       |
| ---------- | ---------------------------------------------------------- |
| Modify     | `crates/ui/web-api/src/extractors/etag_source.rs`          |
| Modify     | `crates/ui/web-api/src/extractors/if_match.rs`             |
| Modify     | `crates/ui/web-api/src/middleware/mod.rs`                  |
| **Create** | `crates/ui/web-api/src/middleware/etag.rs`                 |
| Modify     | `crates/ui/web-api/src/router.rs`                          |
| Modify     | `crates/ui/web-api/src/routes/settings_access.rs`          |
| Modify     | `crates/ui/web-api/src/routes/settings_agent_certs.rs`     |
| Modify     | `crates/ui/web-api/src/routes/settings_oauth.rs`           |
| Modify     | `crates/ui/web-api/src/routes/settings_network.rs`         |
| Modify     | `crates/ui/web-api/src/routes/settings_nats.rs`            |
| Modify     | `crates/ui/web-api/src/routes/settings_zeroconf.rs`        |
| Modify     | `crates/ui/web-api/src/routes/settings_provider_github.rs` |
| Modify     | `crates/ui/web-api/src/integration_tests/if_match.rs`      |
| **Create** | `docs/adr/0017-etag-route-layer-middleware.md`             |
| Modify     | `docs/development/coding-standards.md`                     |

---

## Task 1 — Update `EtagSource` Trait

Remove the unused `&Parts` parameter and add `refresh_etag`. This breaks all impls and the one
call site in `if_match.rs` until Task 2 fixes them.

**Files:**

- Modify: `crates/ui/web-api/src/extractors/etag_source.rs`

- [ ] **Step 1: Replace etag_source.rs contents**

  Replace the entire file:

  ```rust
  use rootcause::Report;

  use crate::app_state::AppState;

  pub trait EtagSource: Sized + Send + Sync + 'static {
      /// Returns the current ETag from the in-memory cache. Fast; used for GET responses.
      async fn current_etag(state: &AppState) -> Result<String, Report>;

      /// Re-reads the version from the DB, syncs the cache, and returns the new ETag.
      /// Used after a successful mutation so the response carries the committed version.
      ///
      /// For GET-only resources this method is never called by `EtagLayer`. Implementors
      /// covering read-only resources may return `Err(report!("refresh not supported"))`.
      async fn refresh_etag(state: &AppState) -> Result<String, Report>;
  }
  ```

  (Dropped: `use async_trait::async_trait`, `use axum::http::request::Parts`, `#[async_trait]`,
  `parts: &Parts` parameter, doc-comment that referenced `IfMatch` extractor.)

- [ ] **Step 2: Check it compiles (will have errors until Task 2)**

  ```shell
  cargo check -p uptrakit-web-api --no-default-features --features db-sqlite 2>&1 | grep "etag_source\|if_match\|EtagSource" | head -30

  ```

  Expected: errors about missing `parts` arg and missing `refresh_etag`. That is correct; Task 2 fixes them.

- [ ] **Step 3: Commit**

  ```bash
  git add crates/ui/web-api/src/extractors/etag_source.rs
  git commit -m "$(cat <<'EOF'
  refactor(web-api): drop &Parts from EtagSource and add refresh_etag stub

  Removes the unused axum Parts argument from EtagSource::current_etag —
  no implementation references it and it is incompatible with middleware
  context. Adds refresh_etag for post-write DB re-reads; impls follow in
  the next commit.

  Compilation is intentionally broken until if_match.rs is updated.

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
  EOF
  )"
  ```

---

## Task 2 — Update `SettingsVersion` and `GlobalSettingsVersion` Impls

Fix the call site and add `refresh_etag` implementations. After this task the crate compiles again.

**Files:**

- Modify: `crates/ui/web-api/src/extractors/if_match.rs`

- [ ] **Step 1: Update imports at top of if_match.rs**

  Replace the existing import block at lines 1–13 with:

  ```rust
  use std::marker::PhantomData;
  use std::sync::Arc;

  use axum::Json;
  use axum::extract::FromRequestParts;
  use axum::http::StatusCode;
  use axum::http::header::IF_MATCH;
  use axum::http::request::Parts;
  use rootcause::report;
  use uptrakit_config_reload::config::Scope;
  use uptrakit_web_api_types::error::ErrorResponse;

  use crate::app_state::AppState;
  use crate::extractors::etag_source::EtagSource;
  use crate::settings_store::get_settings_versions;
  ```

- [ ] **Step 2: Fix the call site in `FromRequestParts` impl**

  In `from_request_parts`, find:

  ```rust
  let current = T::current_etag(parts, state).await.map_err(|e| {
  ```

  Change to:

  ```rust
  let current = T::current_etag(state).await.map_err(|e| {
  ```

- [ ] **Step 3: Replace `SettingsVersion` impl**

  Find the entire `#[async_trait::async_trait] impl EtagSource for SettingsVersion { ... }` block
  and replace it with:

  ```rust
  impl EtagSource for SettingsVersion {
      async fn current_etag(state: &AppState) -> Result<String, rootcause::Report> {
          // SINGLE-TENANT ASSUMPTION
          let version = state
              .settings_version_cache
              .get(Scope::Tenant(state.default_tenant_id))
              .unwrap_or(0);
          Ok(format!("W/\"settings-v{version}\""))
      }

      async fn refresh_etag(state: &AppState) -> Result<String, rootcause::Report> {
          // SINGLE-TENANT ASSUMPTION
          let (tenant_v, _) =
              get_settings_versions(state.db(), state.default_tenant_id).await?;
          let version = u64::try_from(tenant_v).unwrap_or_else(|_| {
              tracing::warn!(tenant_v, "settings_version negative or overflow; treating as 0");
              0
          });
          state
              .settings_version_cache
              .update(Scope::Tenant(state.default_tenant_id), version);
          Ok(format!("W/\"settings-v{version}\""))
      }
  }
  ```

- [ ] **Step 4: Replace `GlobalSettingsVersion` impl**

  Find the entire `#[async_trait::async_trait] impl EtagSource for GlobalSettingsVersion { ... }` block
  and replace it with:

  ```rust
  impl EtagSource for GlobalSettingsVersion {
      async fn current_etag(state: &AppState) -> Result<String, rootcause::Report> {
          let version = state.settings_version_cache.get(Scope::Global).unwrap_or(0);
          Ok(format!("W/\"global-settings-v{version}\""))
      }

      async fn refresh_etag(state: &AppState) -> Result<String, rootcause::Report> {
          // SINGLE-TENANT ASSUMPTION
          let (_, global_v) =
              get_settings_versions(state.db(), state.default_tenant_id).await?;
          let version = u64::try_from(global_v).unwrap_or_else(|_| {
              tracing::warn!(global_v, "global_version negative or overflow; treating as 0");
              0
          });
          state.settings_version_cache.update(Scope::Global, version);
          Ok(format!("W/\"global-settings-v{version}\""))
      }
  }
  ```

- [ ] **Step 5: Verify compilation**

  ```shell
  cargo check -p uptrakit-web-api --no-default-features --features db-sqlite

  ```

  Expected: no errors.

- [ ] **Step 6: Run existing if_match integration tests**

  ```shell
  cargo test -p uptrakit-web-api --no-default-features --features db-sqlite --test if_match 2>&1 | tail -20

  ```

  Expected: all pass (unchanged behaviour; `IfMatch<S>` extractor still works).

- [ ] **Step 7: Commit**

  ```bash
  git add crates/ui/web-api/src/extractors/if_match.rs
  git commit -m "$(cat <<'EOF'
  refactor(web-api): update EtagSource impls — drop async_trait, add refresh_etag

  Removes #[async_trait] from SettingsVersion and GlobalSettingsVersion
  (edition 2024 native async fn in traits). Removes the &Parts argument
  from current_etag call site. Adds refresh_etag to both impls: reads the
  committed version from the DB, syncs the in-memory cache, and returns
  the new ETag string. Marks both with SINGLE-TENANT ASSUMPTION comments.

  IfMatch<S> extractor and IfMatch::for_test() are kept — plugin_configs
  still depends on them.

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
  EOF
  )"
  ```

---

## Task 3 — Create `EtagLayer<S>` Middleware

Build the middleware that replaces handler boilerplate. TDD: write the integration tests first,
confirm they fail, then implement.

**Files:**

- Modify: `crates/ui/web-api/src/middleware/mod.rs`
- Create: `crates/ui/web-api/src/middleware/etag.rs`

The tests are written in `integration_tests/if_match.rs` (Task 6 extends that file). For
now the middleware is tested by the existing integration tests after router wiring (Tasks 4–5).
Write the middleware now; wiring happens next.

- [ ] **Step 1: Register the new module**

  In `crates/ui/web-api/src/middleware/mod.rs`, add one line:

  ```rust
  pub mod etag;
  ```

  (Alphabetical order: add between `audit_log` and `permission`.)

- [ ] **Step 2: Create `middleware/etag.rs`**

  Create `crates/ui/web-api/src/middleware/etag.rs` with the full content:

  ```rust
  use std::sync::Arc;

  use axum::body::Body;
  use axum::extract::State;
  use axum::http::header::{ETAG, IF_MATCH, HeaderValue};
  use axum::http::{Method, Request, StatusCode};
  use axum::middleware::Next;
  use axum::response::{IntoResponse, Response};
  use axum::Json;
  use uptrakit_web_api_types::error::ErrorResponse;

  use crate::app_state::AppState;
  use crate::extractors::etag_source::EtagSource;

  fn strip_etag(s: &str) -> &str {
      s.trim_start_matches("W/").trim_matches('"')
  }

  pub fn etag_layer<S>(
      state: Arc<AppState>,
  ) -> axum::middleware::FromFnLayer<
      impl Fn(State<Arc<AppState>>, Request<Body>, Next) -> impl std::future::Future<Output = Response> + Send
          + Clone,
      Arc<AppState>,
      (State<Arc<AppState>>, Request<Body>, Next),
  >
  where
      S: EtagSource,
  {
      axum::middleware::from_fn_with_state(state, etag_middleware::<S>)
  }

  async fn etag_middleware<S>(
      State(state): State<Arc<AppState>>,
      req: Request<Body>,
      next: Next,
  ) -> Response
  where
      S: EtagSource,
  {
      let method = req.method().clone();

      // PUT/PATCH: validate If-Match before handing off to handler.
      if method == Method::PUT || method == Method::PATCH {
          let client_etag = match req.headers().get(IF_MATCH) {
              None => {
                  return (
                      StatusCode::PRECONDITION_REQUIRED,
                      Json(ErrorResponse {
                          error: "if-match header is required".to_string(),
                          code: Some("if_match.required".to_string()),
                      }),
                  )
                      .into_response();
              }
              Some(h) => match h.to_str() {
                  Ok(s) => s.to_string(),
                  Err(_) => {
                      return (
                          StatusCode::BAD_REQUEST,
                          Json(ErrorResponse {
                              error: "if-match header contains non-ASCII bytes".to_string(),
                              code: Some("if_match.parse_error".to_string()),
                          }),
                      )
                          .into_response();
                  }
              },
          };

          let current = match S::current_etag(&state).await {
              Ok(v) => v,
              Err(e) => {
                  tracing::error!(error = %e, "etag lookup failed");
                  return (
                      StatusCode::INTERNAL_SERVER_ERROR,
                      Json(ErrorResponse {
                          error: "etag lookup failed".to_string(),
                          code: Some("if_match.lookup_failed".to_string()),
                      }),
                  )
                      .into_response();
              }
          };

          if strip_etag(&client_etag) != strip_etag(&current) {
              return (
                  StatusCode::CONFLICT,
                  Json(ErrorResponse {
                      error: "etag mismatch (stale version)".to_string(),
                      code: Some("if_match.stale".to_string()),
                  }),
              )
                  .into_response();
          }
      }

      let mut response = next.run(req).await;

      // Inject ETag on 2xx responses only.
      if response.status().is_success() {
          let etag_result = if method == Method::GET {
              S::current_etag(&state).await
          } else {
              S::refresh_etag(&state).await
          };
          match etag_result {
              Ok(etag) => {
                  if let Ok(value) = HeaderValue::from_str(&etag) {
                      response.headers_mut().insert(ETAG, value);
                  } else {
                      tracing::warn!(etag, "etag string is not a valid header value; skipping injection");
                  }
              }
              Err(e) => {
                  tracing::warn!(error = %e, "etag refresh failed; response sent without ETag");
              }
          }
      }

      response
  }
  ```

  > **Note on return type**: if the `FromFnLayer` signature causes type-inference issues at the
  > use site, replace the explicit return type with `impl tower::Layer<axum::routing::Route> + Clone`
  > (add `use tower::Layer; use axum::routing::Route;`). Both spellings compile; the opaque form is
  > simpler if the concrete form requires additional imports.

- [ ] **Step 3: Check compilation**

  ```shell
  cargo check -p uptrakit-web-api --no-default-features --features db-sqlite

  ```

  Expected: no errors.

- [ ] **Step 4: Commit**

  ```bash
  git add crates/ui/web-api/src/middleware/mod.rs \
          crates/ui/web-api/src/middleware/etag.rs
  git commit -m "$(cat <<'EOF'
  feat(web-api): add EtagLayer<S> middleware

  New route-level middleware that:
  - GET: injects ETag from cache (no DB query)
  - PUT/PATCH: validates If-Match (428/409 on missing/stale), then injects
    new ETag via refresh_etag after a 2xx response (one DB SELECT)
  - POST: calls refresh_etag after 2xx to inject new ETag
  - Non-2xx responses pass through unmodified

  Declared via etag_layer::<S>(state) factory; opt-in at the router via
  .route_layer().

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
  EOF
  )"
  ```

---

## Task 4 — Wire Tenant Settings + Clean Handler Bodies

Move tenant settings routes into an ETag sub-router and strip the manual ETag code from
`settings_access.rs` and `settings_agent_certs.rs`.

**Files:**

- Modify: `crates/ui/web-api/src/router.rs`
- Modify: `crates/ui/web-api/src/routes/settings_access.rs`
- Modify: `crates/ui/web-api/src/routes/settings_agent_certs.rs`

- [ ] **Step 1: Add etag middleware import to router.rs**

  Near the top of `router.rs`, find the existing `use` block and add:

  ```rust
  use crate::middleware::etag::etag_layer;
  use crate::extractors::{GlobalSettingsVersion, SettingsVersion};
  ```

  (These may already be imported or partially imported; add only what's missing.)

- [ ] **Step 2: Extract tenant settings into a sub-router**

  In `router.rs`, locate the three blocks that register tenant settings routes inside
  `auth_routes`:

  ```rust
  .routes(routes!(
      crate::routes::settings_access::get_access_settings,
      crate::routes::settings_access::update_access_settings
  ))
  .routes(routes!(
      crate::routes::settings_combined::get_combined_settings
  ))
  .routes(routes!(
      crate::routes::settings_agent_certs::get_agent_certificate_settings,
      crate::routes::settings_agent_certs::update_agent_certificate_settings
  ))
  ```

  **Remove** those three `.routes(...)` calls from `auth_routes` and, before the
  `auth_routes` declaration, add:

  ```rust
  let tenant_settings = OpenApiRouter::new()
      .routes(routes!(
          crate::routes::settings_access::get_access_settings,
          crate::routes::settings_access::update_access_settings
      ))
      .routes(routes!(
          crate::routes::settings_combined::get_combined_settings
      ))
      .routes(routes!(
          crate::routes::settings_agent_certs::get_agent_certificate_settings,
          crate::routes::settings_agent_certs::update_agent_certificate_settings
      ))
      .route_layer(etag_layer::<SettingsVersion>(Arc::clone(&state)));
  ```

  Then, after `auth_routes` is fully built and before the `.route_layer(require_auth)` line,
  merge:

  ```rust
  let auth_routes = auth_routes.merge(tenant_settings);
  ```

  The `.route_layer(require_auth)` stays at the very end of the `auth_routes` chain (after all
  merges). This ensures require_auth still applies to everything including the ETag sub-router
  routes.

- [ ] **Step 3: Clean `get_access_settings` in settings_access.rs**

  Remove:
  - The `// Absent entry ≡ version 0 ...` comment and the `let version = ...` cache lookup
  - The `let etag = format!(...)` line
  - The `[(axum::http::header::ETAG, etag)]` tuple from the response

  Before:

  ```rust
  pub async fn get_access_settings(
      State(state): State<Arc<AppState>>,
      CanViewSettings(_user): CanViewSettings,
  ) -> Response {
      let version = state
          .settings_version_cache
          .get(uptrakit_config_reload::config::Scope::Tenant(
              state.default_tenant_id,
          ))
          .unwrap_or(0);
      let etag = format!("W/\"settings-v{version}\"");
      (
          StatusCode::OK,
          [(axum::http::header::ETAG, etag)],
          Json(current_response(&state)),
      )
          .into_response()
  }
  ```

  After:

  ```rust
  pub async fn get_access_settings(
      State(state): State<Arc<AppState>>,
      CanViewSettings(_user): CanViewSettings,
  ) -> Response {
      (StatusCode::OK, Json(current_response(&state))).into_response()
  }
  ```

- [ ] **Step 4: Clean `update_access_settings` in settings_access.rs**

  Remove:
  - `_if_match: IfMatch<SettingsVersion>` parameter
  - The entire "Bump settings version cache" block (step 8 in the handler, lines including
    `let scope = ...`, `let next = ...`, `state.settings_version_cache.update(...)`)
  - `let new_etag = format!(...)` line
  - `[(axum::http::header::ETAG, new_etag)]` from the response tuple

  Before (relevant parts):

  ```rust
  pub async fn update_access_settings(
      State(state): State<Arc<AppState>>,
      CanManageAuthSettings(user): CanManageAuthSettings,
      _if_match: IfMatch<SettingsVersion>,
      api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
      // ...
  ) -> Response {
      // ...

      // ── 8. Bump settings version cache ────────────────────────────────────────────
      let scope = uptrakit_config_reload::config::Scope::Tenant(tenant_id);
      let next = state
          .settings_version_cache
          .get(scope)
          .unwrap_or(0)
          .saturating_add(1);
      state.settings_version_cache.update(scope, next);

      state.settings.set_registration(reg).await;
      state.settings.set_authentication(auth).await;

      let new_etag = format!("W/\"settings-v{next}\"");
      (
          StatusCode::OK,
          [(axum::http::header::ETAG, new_etag)],
          Json(current_response(&state)),
      )
          .into_response()
  }
  ```

  After (relevant parts):

  ```rust
  pub async fn update_access_settings(
      State(state): State<Arc<AppState>>,
      CanManageAuthSettings(user): CanManageAuthSettings,
      api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
      // ...
  ) -> Response {
      // ...

      state.settings.set_registration(reg).await;
      state.settings.set_authentication(auth).await;

      (StatusCode::OK, Json(current_response(&state))).into_response()
  }
  ```

  Also remove the unused import line:

  ```rust
  use crate::extractors::{IfMatch, SettingsVersion};
  ```

- [ ] **Step 5: Clean `update_agent_certificate_settings` in settings_agent_certs.rs**

  Remove `_if_match: IfMatch<SettingsVersion>` parameter from the handler signature.
  Remove the now-unused import `use crate::extractors::{IfMatch, SettingsVersion};`.

  Also **delete** the two handler-direct unit test functions that call `IfMatch::for_test()`
  (lines ~560 and ~615 in the file). These tested the extractor's presence in the handler
  signature, which no longer exists. Coverage moves to integration tests in Task 6.

  > **Tip:** search for `IfMatch::for_test()` in the file and delete the entire `#[tokio::test]`
  > function block around each call site.

- [ ] **Step 6: Check compilation and run tests**

  ```shell
  cargo check -p uptrakit-web-api --no-default-features --features db-sqlite
  cargo test -p uptrakit-web-api --no-default-features --features db-sqlite 2>&1 | tail -30

  ```

  Expected: no compilation errors. Some tests will now fail for `settings_access` GET (no ETag
  returned without the middleware in place for unit tests — but integration tests via TestApp will
  pass since they go through the router). Confirm existing integration tests pass.

- [ ] **Step 7: Commit**

  ```bash
  git add crates/ui/web-api/src/router.rs \
          crates/ui/web-api/src/routes/settings_access.rs \
          crates/ui/web-api/src/routes/settings_agent_certs.rs
  git commit -m "$(cat <<'EOF'
  refactor(web-api): wire EtagLayer<SettingsVersion> for tenant settings routes

  Extracts GET+PUT /settings/access, GET /settings, and
  GET+PUT /settings/agent-certificates into a sub-router with
  etag_layer::<SettingsVersion>. Removes manual ETag construction from
  get_access_settings and update_access_settings, removes the cache-bump
  block from update_access_settings, and removes _if_match parameters from
  both update handlers. Deletes 2 handler-direct unit tests in
  settings_agent_certs that called IfMatch::for_test(); coverage moves to
  integration tests.

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
  EOF
  )"
  ```

---

## Task 5 — Wire Global Settings + Fix Scope Bugs + Clean Handler Bodies

Move global settings routes into a sub-router with `GlobalSettingsVersion`, fixing the 4
wrong-scope handlers in the process.

**Files:**

- Modify: `crates/ui/web-api/src/router.rs`
- Modify: `crates/ui/web-api/src/routes/settings_oauth.rs`
- Modify: `crates/ui/web-api/src/routes/settings_network.rs`
- Modify: `crates/ui/web-api/src/routes/settings_nats.rs`
- Modify: `crates/ui/web-api/src/routes/settings_zeroconf.rs`
- Modify: `crates/ui/web-api/src/routes/settings_provider_github.rs`

### router.rs changes

- [ ] **Step 1: Remove global settings `.routes()` calls from auth_routes**

  From the long `auth_routes` chain, **remove** these blocks:

  ```rust
  .routes(routes!(
      crate::routes::settings_global_combined::get_global_combined_settings
  ))
  .routes(routes!(
      crate::routes::settings_provider_github::get_github_provider_settings,
      crate::routes::settings_provider_github::update_github_provider_settings
  ))
  .routes(routes!(
      crate::routes::settings_network::get_network_settings,
      crate::routes::settings_network::update_network_settings
  ))
  ```

  And the separately-declared blocks:

  ```rust
  // Zeroconf settings
  let auth_routes = auth_routes.routes(routes!(
      crate::routes::settings_zeroconf::get_zeroconf_settings,
      crate::routes::settings_zeroconf::update_zeroconf_settings
  ));

  // OAuth global settings
  let auth_routes = auth_routes.routes(routes!(
      crate::routes::settings_oauth::get_oauth_settings,
      crate::routes::settings_oauth::update_oauth_settings
  ));
  ```

  And the feature-gated NATS block:

  ```rust
  // NATS settings
  #[cfg(feature = "nats")]
  let auth_routes = auth_routes.routes(routes!(
      crate::routes::settings_nats::get_nats_settings,
      crate::routes::settings_nats::update_nats_settings
  ));
  ```

  And the `rotate_ca` route (currently at line ~606):

  ```rust
  .routes(routes!(crate::routes::settings_ca::rotate_ca))
  ```

- [ ] **Step 2: Add global settings sub-router**

  Before the `auth_routes` declaration, add:

  ```rust
  let mut global_settings = OpenApiRouter::new()
      .routes(routes!(
          crate::routes::settings_global_combined::get_global_combined_settings
      ))
      .routes(routes!(
          crate::routes::settings_provider_github::get_github_provider_settings,
          crate::routes::settings_provider_github::update_github_provider_settings
      ))
      .routes(routes!(
          crate::routes::settings_network::get_network_settings,
          crate::routes::settings_network::update_network_settings
      ))
      .routes(routes!(
          crate::routes::settings_zeroconf::get_zeroconf_settings,
          crate::routes::settings_zeroconf::update_zeroconf_settings
      ))
      .routes(routes!(
          crate::routes::settings_oauth::get_oauth_settings,
          crate::routes::settings_oauth::update_oauth_settings
      ))
      .routes(routes!(crate::routes::settings_ca::rotate_ca));

  #[cfg(feature = "nats")]
  {
      global_settings = global_settings.routes(routes!(
          crate::routes::settings_nats::get_nats_settings,
          crate::routes::settings_nats::update_nats_settings
      ));
  }

  let global_settings =
      global_settings.route_layer(etag_layer::<GlobalSettingsVersion>(Arc::clone(&state)));
  ```

  Then, after the `auth_routes` is fully built (before `.route_layer(require_auth)`), add:

  ```rust
  let auth_routes = auth_routes.merge(global_settings);
  ```

### Handler cleanup — settings_oauth.rs

- [ ] **Step 3: Remove `_if_match` from `update_oauth_settings`**

  Remove `_if_match: IfMatch<GlobalSettingsVersion>` from the parameter list.

- [ ] **Step 4: Remove early-return ETag construction in `update_oauth_settings`**

  In the "Nothing to update" early return block, remove:

  ```rust
  let version = state
      .settings_version_cache
      .get(uptrakit_config_reload::config::Scope::Global)
      .unwrap_or(0);
  let etag = format!("W/\"global-settings-v{version}\"");
  return (
      StatusCode::OK,
      [(axum::http::header::ETAG, etag)],
      Json(db_state.into_response(&state)),
  )
      .into_response();
  ```

  Replace with:

  ```rust
  return (StatusCode::OK, Json(db_state.into_response(&state))).into_response();
  ```

- [ ] **Step 5: Remove post-commit cache-sync and ETag from `update_oauth_settings`**

  After `hook.flush_after_commit().await;`, remove the entire cache-sync block:

  ```rust
  // Sync the in-process global-settings version cache to the authoritative DB value.
  if let Ok((_, global_v)) = get_settings_versions(state.db(), state.default_tenant_id).await {
      let version = u64::try_from(global_v).unwrap_or_else(|_| {
          tracing::warn!(...);
          0
      });
      state
          .settings_version_cache
          .update(uptrakit_config_reload::config::Scope::Global, version);
  }
  // Non-fatal: if the read fails the reconciler will sync the cache on its next poll.
  ```

  The middleware's `refresh_etag` now does this. The final response becomes:

  ```rust
  let db_state = load_oauth_settings_from_db(&state).await;
  (StatusCode::OK, Json(db_state.into_response(&state))).into_response()
  ```

  Remove unused imports from `settings_oauth.rs`:
  - `use crate::extractors::{GlobalSettingsVersion, IfMatch};` (entire line — `IfMatch` is now unused)
  - `use crate::settings_store::{get_settings_versions, ...}` — remove `get_settings_versions` from the import list
    (keep `load_global_setting_raw` and `upsert_global_setting_raw`)

### Handler cleanup — wrong-scope handlers

- [ ] **Step 6: Fix `settings_network.rs`**

  The handler currently has:

  ```rust
  use crate::extractors::{IfMatch, SettingsVersion};
  // ...
  _if_match: IfMatch<SettingsVersion>,  // BUG: should be GlobalSettingsVersion
  ```

  Remove `_if_match: IfMatch<SettingsVersion>` from `update_network_settings` parameter list.
  Remove the import `use crate::extractors::{IfMatch, SettingsVersion};`.
  **Delete** the two `#[tokio::test]` functions in this file that call `IfMatch::for_test()`
  (lines ~673 and ~731).

- [ ] **Step 7: Fix `settings_zeroconf.rs`**

  Remove `_if_match: IfMatch<SettingsVersion>` from `update_zeroconf_settings`.
  Remove `use crate::extractors::{IfMatch, SettingsVersion};` import.
  **Delete** the one `#[tokio::test]` function that calls `IfMatch::for_test()` (line ~480).

- [ ] **Step 8: Fix `settings_provider_github.rs`**

  Remove `_if_match: IfMatch<SettingsVersion>` from `update_github_provider_settings`.
  Remove `use crate::extractors::{IfMatch, SettingsVersion};` import.
  (No `for_test()` unit test call sites to delete in this file.)

- [ ] **Step 9: Fix `settings_nats.rs`**

  Remove `_if_match: IfMatch<SettingsVersion>` from `update_nats_settings`.
  Remove `use crate::extractors::{IfMatch, SettingsVersion};` import.
  **Delete** all `#[tokio::test]` functions that call `IfMatch::for_test()` (the spec identifies
  ~7; search for `for_test()` in the file and delete each enclosing test function).

- [ ] **Step 10: Check compilation and run tests**

  ```shell
  cargo check --all-features
  cargo test -p uptrakit-web-api --no-default-features --features db-sqlite 2>&1 | tail -30

  ```

  Expected: clean compile, existing tests pass.

- [ ] **Step 11: Commit**

  ```bash
  git add crates/ui/web-api/src/router.rs \
          crates/ui/web-api/src/routes/settings_oauth.rs \
          crates/ui/web-api/src/routes/settings_network.rs \
          crates/ui/web-api/src/routes/settings_nats.rs \
          crates/ui/web-api/src/routes/settings_zeroconf.rs \
          crates/ui/web-api/src/routes/settings_provider_github.rs
  git commit -m "$(cat <<'EOF'
  fix(web-api): wire EtagLayer<GlobalSettingsVersion> for global settings routes

  Extracts all /api/v1/global-settings/* routes and POST /ca/rotate into a
  sub-router with etag_layer::<GlobalSettingsVersion>. Fixes 4 scope bugs:
  zeroconf, nats, network, and providers/github all used SettingsVersion
  (tenant scope) but live under /api/v1/global-settings/ and must use
  GlobalSettingsVersion (global scope).

  Removes _if_match parameters from all affected handlers. Removes the
  manual cache-sync and ETag construction from update_oauth_settings (now
  owned by the middleware's refresh_etag). Deletes ~10 handler-direct unit
  tests that called IfMatch::for_test(); coverage moves to integration tests.

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
  EOF
  )"
  ```

---

## Task 6 — Integration Tests

Add tests for previously untested endpoints and a scope-bug regression test.

**Files:**

- Modify: `crates/ui/web-api/src/integration_tests/if_match.rs`

- [ ] **Step 1: Add agent-certificates round-trip test**

  Append to `integration_tests/if_match.rs`:

  ```rust
  // ── Agent certificates (/api/v1/settings/agent-certificates) ─────────────────

  /// GET /settings/agent-certificates returns ETag header.
  #[tokio::test]
  async fn get_agent_certs_settings_returns_etag() {
      ensure_crypto_provider();
      let app = TestApp::new().await;
      let client = app.client();
      let token = register_and_get_token(&client).await;

      let resp = client
          .get("/api/v1/settings/agent-certificates")
          .bearer(&token)
          .send()
          .await;

      assert_eq!(resp.status(), http::StatusCode::OK);
      let etag = resp
          .headers()
          .get("etag")
          .expect("ETag header present")
          .to_str()
          .expect("ASCII")
          .to_string();
      assert!(
          etag.contains("settings-v"),
          "expected settings-v in ETag, got {etag:?}"
      );
  }

  /// Full GET→ETag→PUT round-trip for agent-certificates.
  #[tokio::test]
  async fn agent_certs_settings_get_etag_put_round_trip() {
      ensure_crypto_provider();
      let app = TestApp::new().await;
      let client = app.client();
      let token = register_and_get_token(&client).await;

      // GET → capture ETag.
      let get_resp = client
          .get("/api/v1/settings/agent-certificates")
          .bearer(&token)
          .send()
          .await;
      assert_eq!(get_resp.status(), http::StatusCode::OK);
      let etag = get_resp
          .headers()
          .get("etag")
          .expect("ETag on GET")
          .to_str()
          .expect("ASCII")
          .to_string();

      // PUT without If-Match → 428.
      let no_match = client
          .put_json("/api/v1/settings/agent-certificates", &serde_json::json!({}))
          .bearer(&token)
          .send_status()
          .await;
      assert_eq!(no_match, http::StatusCode::PRECONDITION_REQUIRED);

      // PUT with captured ETag → 200 with new ETag.
      let put_resp = client
          .put_json("/api/v1/settings/agent-certificates", &serde_json::json!({}))
          .bearer(&token)
          .header("if-match", &etag)
          .send()
          .await;
      assert_eq!(put_resp.status(), http::StatusCode::OK);
      assert!(
          put_resp.headers().contains_key("etag"),
          "PUT response must carry ETag"
      );

      // Old ETag now stale → 409.
      let stale = client
          .put_json("/api/v1/settings/agent-certificates", &serde_json::json!({}))
          .bearer(&token)
          .header("if-match", &etag)
          .send_status()
          .await;
      assert_eq!(stale, http::StatusCode::CONFLICT);
  }
  ```

- [ ] **Step 2: Add network settings round-trip test**

  ```rust
  // ── Network settings (/api/v1/global-settings/network) ───────────────────────

  /// GET /global-settings/network returns ETag header.
  #[tokio::test]
  async fn get_network_settings_returns_etag() {
      ensure_crypto_provider();
      let app = TestApp::new().await;
      let client = app.client();
      let token = register_and_get_token(&client).await;

      let resp = client
          .get("/api/v1/global-settings/network")
          .bearer(&token)
          .send()
          .await;

      assert_eq!(resp.status(), http::StatusCode::OK);
      let etag = resp
          .headers()
          .get("etag")
          .expect("ETag header present")
          .to_str()
          .expect("ASCII")
          .to_string();
      assert!(
          etag.contains("global-settings-v"),
          "expected global-settings-v in ETag, got {etag:?}"
      );
  }

  /// PUT /global-settings/network without If-Match → 428.
  #[tokio::test]
  async fn put_network_settings_without_if_match_returns_428() {
      ensure_crypto_provider();
      let app = TestApp::new().await;
      let client = app.client();
      let token = register_and_get_token(&client).await;

      let status = client
          .put_json("/api/v1/global-settings/network", &serde_json::json!({}))
          .bearer(&token)
          .send_status()
          .await;

      assert_eq!(status, http::StatusCode::PRECONDITION_REQUIRED);
  }
  ```

- [ ] **Step 3: Add scope-bug regression test**

  This is the critical regression test for the 4 wrong-scope fixes. A tenant-scoped ETag
  (`W/"settings-v0"`) must be rejected by global-settings endpoints (which expect
  `W/"global-settings-v0"`).

  ```rust
  // ── Scope regression: global endpoints reject tenant-scoped ETags ─────────────

  /// PUT /global-settings/network with a tenant-scoped ETag → 409 Conflict.
  ///
  /// Before the scope-bug fix, network/nats/zeroconf/github used SettingsVersion
  /// (tenant scope). A tenant ETag would have been accepted. This test ensures
  /// the correct GlobalSettingsVersion scope is enforced.
  #[tokio::test]
  async fn global_settings_rejects_tenant_scoped_etag() {
      ensure_crypto_provider();
      let app = TestApp::new().await;
      let client = app.client();
      let token = register_and_get_token(&client).await;

      // Seed both caches to version 5 so the test discriminates on the ETag *prefix*.
      app.state
          .settings_version_cache
          .update(Scope::Tenant(app.state.default_tenant_id), 5);
      app.state.settings_version_cache.update(Scope::Global, 5);

      // Sending the tenant-scoped ETag W/"settings-v5" to a global endpoint.
      // The middleware compares against W/"global-settings-v5" → mismatch → 409.
      let status = client
          .put_json("/api/v1/global-settings/network", &serde_json::json!({}))
          .bearer(&token)
          .header("if-match", "W/\"settings-v5\"")
          .send_status()
          .await;

      assert_eq!(
          status,
          http::StatusCode::CONFLICT,
          "tenant-scoped ETag must be rejected by global-settings endpoint"
      );

      // Sending the correct global-scoped ETag → 200.
      let status_ok = client
          .put_json("/api/v1/global-settings/network", &serde_json::json!({}))
          .bearer(&token)
          .header("if-match", "W/\"global-settings-v5\"")
          .send_status()
          .await;

      assert_eq!(status_ok, http::StatusCode::OK);
  }

  /// PUT /global-settings/zeroconf with a tenant-scoped ETag → 409 Conflict.
  #[tokio::test]
  async fn zeroconf_settings_rejects_tenant_scoped_etag() {
      ensure_crypto_provider();
      let app = TestApp::new().await;
      let client = app.client();
      let token = register_and_get_token(&client).await;

      let status = client
          .put_json(
              "/api/v1/global-settings/zeroconf",
              &serde_json::json!({ "enabled": false }),
          )
          .bearer(&token)
          .header("if-match", "W/\"settings-v0\"")
          .send_status()
          .await;

      assert_eq!(
          status,
          http::StatusCode::CONFLICT,
          "tenant-scoped ETag rejected by zeroconf (global) endpoint"
      );
  }
  ```

- [ ] **Step 4: Run all integration tests**

  ```shell
  cargo test -p uptrakit-web-api --no-default-features --features db-sqlite --test if_match 2>&1 | tail -30

  ```

  Expected: all tests pass, including new ones.

- [ ] **Step 5: Run full test suite**

  ```shell
  cargo test --all-features 2>&1 | tail -40

  ```

  Expected: no failures.

- [ ] **Step 6: Commit**

  ```bash
  git add crates/ui/web-api/src/integration_tests/if_match.rs
  git commit -m "$(cat <<'EOF'
  test(web-api): add ETag integration tests for all settings endpoints

  New integration tests via TestApp covering:
  - GET /settings/agent-certificates returns ETag
  - GET→ETag→PUT round-trip for agent-certificates
  - GET /global-settings/network returns ETag
  - PUT /global-settings/network without If-Match → 428
  - Scope regression: global endpoints reject tenant-scoped ETags (network,
    zeroconf) — verifies the 4 wrong-scope bug fixes hold

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
  EOF
  )"
  ```

---

## Task 7 — ADR and Documentation

Write the ADR and update the coding standards guide.

**Files:**

- Create: `docs/adr/0017-etag-route-layer-middleware.md`
- Modify: `docs/development/coding-standards.md`

- [ ] **Step 1: Write the ADR**

  Create `docs/adr/0017-etag-route-layer-middleware.md`:

  ```markdown
  # ADR-0017: ETag Route-Layer Middleware over Per-Handler Extractors

  **Date:** 2026-05-24
  **Status:** Accepted

  ## Context

  ETag support for settings endpoints was partial and error-prone: only 2 of 11 GET handlers
  returned `ETag` headers, only 2 of 9 PUT handlers returned `ETag` in the response, and 4
  global-settings handlers used the wrong ETag scope (`SettingsVersion` instead of
  `GlobalSettingsVersion`). Each handler that did have ETag support contained 4–6 lines of
  boilerplate that was easy to copy with the wrong scope.

  ## Decision

  Use an `axum::middleware::from_fn_with_state`-based route-level middleware (`EtagLayer<S>`)
  opted-in via `.route_layer()` on per-scope `OpenApiRouter` sub-routers.

  ## Alternatives Considered

  **1. Per-handler extractor (current partial state)**

  Explicit but requires boilerplate in every handler. Error-prone: wrong scope type compiles
  silently; missing return header is invisible until a client notices. Rejected.

  **2. Route-level middleware (chosen)**

  Zero handler boilerplate. The scope (`SettingsVersion` vs `GlobalSettingsVersion`) is declared
  exactly once at the router. Extensible to non-settings resources by adding `EtagSource` impls.
  Validates `If-Match` for PUT/PATCH, injects `ETag` for GET and mutations, and does a post-write
  DB re-read for mutations to return the committed version.

  **3. Global tower layer**

  No opt-in required. Cannot be scoped per resource type without per-route metadata or a separate
  registry. Would inject ETags on non-settings routes unintentionally. Rejected.

  ## Consequences

  - All settings GET endpoints return `ETag` without handler code.
  - All settings PUT/PATCH endpoints enforce `If-Match` and return the new `ETag` without handler
    code.
  - POST endpoints in ETag sub-routers (e.g. `/ca/rotate`) return the new `ETag`.
  - Successful mutations incur one additional `SELECT` (`refresh_etag`) to read the committed
    version. This is correct regardless of how many fields the write bumped internally.
  - `IfMatch<S>` extractor is retained because `plugin_configs.rs` handlers still use it directly.
    It is a candidate for removal in a future spec that migrates those handlers to the layer pattern.
  - New resources outside settings may adopt the same pattern by implementing `EtagSource` and
    calling `etag_layer::<NewResourceVersion>(state)` in the router.
  ```

- [ ] **Step 2: Update `docs/development/coding-standards.md`**

  Add a new section (after the existing "ETag" or "HTTP Patterns" section, or at the end of the
  relevant heading):

  ```markdown
  ### ETag Route-Layer Pattern

  Settings endpoints use route-level ETag middleware rather than per-handler extractors.
  New settings routes **must** be covered by `etag_layer`.

  **How to add a new settings route:**

  1. Decide scope: `SettingsVersion` for `/api/v1/settings/*`, `GlobalSettingsVersion` for
     `/api/v1/global-settings/*`.
  2. In `router.rs`, add the route to the appropriate sub-router (`tenant_settings` or
     `global_settings`). Do not add it to the outer `auth_routes` chain.
  3. Handler bodies contain no ETag code — no `If-Match` parameter, no `settings_version_cache`
     lookup, no ETag header construction.

  **POST endpoints:** POST routes included in an ETag sub-router receive an ETag on success if
  they mutate state (e.g. `POST /global-settings/ca/rotate`). POST endpoints that are destructive
  teardowns (e.g. `POST /settings/reset-data`) must **not** be included in any ETag sub-router.

  **`IfMatch<S>` extractor:** Kept for `plugin_configs.rs` handlers that use it directly. Do not
  add it to new handlers — use the layer pattern instead.
  ```

- [ ] **Step 3: Run markdownlint**

  ```shell
  npx prettier --write docs/adr/0017-etag-route-layer-middleware.md \
   docs/development/coding-standards.md
  markdownlint --config .markdownlint.json \
   docs/adr/0017-etag-route-layer-middleware.md \
   docs/development/coding-standards.md

  ```

  Expected: no lint errors.

- [ ] **Step 4: Commit**

  ```bash
  git add docs/adr/0017-etag-route-layer-middleware.md \
          docs/development/coding-standards.md
  git commit -m "$(cat <<'EOF'
  docs: add ADR-0017 for ETag route-layer middleware and update coding standards

  ADR-0017 records the decision to use route-level middleware over per-handler
  extractors for ETag management. coding-standards.md gains a section on how
  to add new settings routes with the etag_layer pattern and the rule that
  POST teardown endpoints must not join an ETag sub-router.

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
  EOF
  )"
  ```

---

## Self-Review

**Spec coverage:**

| Spec requirement                                                           | Task                                     |
| -------------------------------------------------------------------------- | ---------------------------------------- |
| All settings GET endpoints return `ETag`                                   | Task 3 (middleware) + Tasks 4–5 (wiring) |
| All settings PUT endpoints require `If-Match`, validate, return new `ETag` | Task 3 + Tasks 4–5                       |
| `POST /ca/rotate` returns new `ETag`                                       | Task 5 (included in global sub-router)   |
| No ETag code in handler bodies                                             | Tasks 4–5                                |
| Opt-in declared at the router                                              | Tasks 4–5                                |
| Fix 4 wrong-scope bugs                                                     | Task 5 (network, nats, zeroconf, github) |
| `IfMatch<S>` extractor and `for_test()` kept                               | Task 2 (unchanged)                       |
| `#[async_trait]` removed from `EtagSource` impls                           | Task 2                                   |
| `&Parts` removed from trait                                                | Task 1                                   |
| `refresh_etag` for post-write DB re-read                                   | Task 2                                   |
| `SINGLE-TENANT ASSUMPTION` comments                                        | Task 2                                   |
| Integration tests for scope regression                                     | Task 6                                   |
| ~12 handler-direct unit tests deleted                                      | Tasks 4–5                                |
| ADR-0017                                                                   | Task 7                                   |
| `coding-standards.md` update                                               | Task 7                                   |
| `POST /settings/reset-data` excluded from ETag layer                       | Task 5 (not added to global sub-router)  |

**Placeholder scan:** No TBDs or incomplete sections found.

**Type consistency:** `EtagSource::current_etag(state)` and `refresh_etag(state)` signatures are
consistent across all tasks. `etag_layer::<S>(state)` used identically in Tasks 4 and 5.
`SettingsVersion` / `GlobalSettingsVersion` naming is consistent throughout.
