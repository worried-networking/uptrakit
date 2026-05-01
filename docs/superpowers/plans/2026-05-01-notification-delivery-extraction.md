# Notification Delivery Extraction Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract stateless notification delivery out of `uptrakit-web-api` into a new
`uptrakit-notification-delivery` crate, preceded by a targeted import fix in
`build_settings_bag`.

**Architecture:** Four consecutive commits. Commit 1 removes a misplaced
`uptrakit-web-api-auth` import from `dispatcher.rs` by calling `raw_settings` directly.
Commits 2 and 3 must land back-to-back (no other commits between): Commit 2 creates the
new crate with `event.rs`, `message_builder.rs`, and `deliver.rs`; Commit 3 deletes the
originals from `web-api` and rewires the dispatcher to the new crate. Commit 4 updates
the ADR.

**Tech Stack:** Rust, `sea-orm` (existing, not added), `uptrakit-notification-plugin-core`,
`uptrakit-plugin-infrastructure-core` (no features), `rootcause`

---

## File Map

**Commit 1 — modify only:**

- `crates/ui/web-api/src/notifications/dispatcher.rs` — remove `uptrakit_web_api_auth`
  import, inline `raw_settings` calls, update `typed_smtp_settings_or_empty` signature

**Commit 2 — create:**

- `crates/plugins/notifications/delivery/Cargo.toml`
- `crates/plugins/notifications/delivery/src/lib.rs`
- `crates/plugins/notifications/delivery/src/event.rs`
- `crates/plugins/notifications/delivery/src/message_builder.rs`
- `crates/plugins/notifications/delivery/src/deliver.rs`

**Commit 2 — modify:**

- `Cargo.toml` (workspace root) — add `uptrakit-notification-delivery` to
  `[workspace.dependencies]`

**Commit 3 — delete:**

- `crates/ui/web-api/src/notifications/events.rs`
- `crates/ui/web-api/src/notifications/message_builder.rs`

**Commit 3 — modify:**

- `crates/ui/web-api/src/notifications/mod.rs` — replace `pub mod events;` with shim;
  drop `pub mod message_builder;`
- `crates/ui/web-api/src/notifications/dispatcher.rs` — update imports and deliver call
- `crates/ui/web-api/Cargo.toml` — add `uptrakit-notification-delivery` dep

**Commit 4 — modify:**

- `docs/adr/0001-web-api-decomposition-strategy.md`

---

## Task 1: Fix `build_settings_bag` settings dependency

**Files:**

- Modify: `crates/ui/web-api/src/notifications/dispatcher.rs`

- [ ] **Step 1: Update the unit test to use `RawSettingsError`**

  In `dispatcher.rs`, find the test at the bottom of the file (inside `#[cfg(test)] mod tests`).
  Replace the `AuthError::Internal` construction with `RawSettingsError::Decode`:

  ```rust
  #[test]
  fn typed_smtp_settings_or_empty_returns_default_on_load_error() {
      let tenant_id = Uuid::now_v7();
      let settings = typed_smtp_settings_or_empty(
          Err(rootcause::report!(
              uptrakit_shared_db::raw_settings::RawSettingsError::Decode("boom".into())
          )),
          "tenant",
          Some(tenant_id),
          SMTP_PASSWORD_AAD,
      );

      assert_eq!(settings, EmailSmtpSettings::default());
  }
  ```

- [ ] **Step 2: Run the test to confirm it fails with a type mismatch**

  ```bash
  cargo test -p uptrakit-web-api --no-default-features --features db-sqlite \
    typed_smtp_settings_or_empty_returns_default_on_load_error 2>&1 | tail -20
  ```

  Expected: compile error about mismatched types (`AuthError` vs `RawSettingsError`) or
  `RawSettingsError` not in scope. This confirms the test now exercises the right type.

- [ ] **Step 3: Update `typed_smtp_settings_or_empty` parameter type**

  Change the function signature from `uptrakit_web_api_auth::auth::Result<EmailSmtpSettings>`
  to `uptrakit_shared_db::raw_settings::Result<EmailSmtpSettings>`:

  ```rust
  fn typed_smtp_settings_or_empty(
      result: uptrakit_shared_db::raw_settings::Result<EmailSmtpSettings>,
      scope: &'static str,
      tenant_id: Option<Uuid>,
      password_aad: &str,
  ) -> EmailSmtpSettings {
      match result {
          Ok(settings) => normalize_smtp_settings(settings, password_aad, scope, tenant_id),
          Err(error) => {
              if let Some(tenant_id) = tenant_id {
                  tracing::warn!(
                      error = ?error,
                      %tenant_id,
                      scope,
                      "failed to load typed SMTP settings for notification dispatch; using empty fallback"
                  );
              } else {
                  tracing::warn!(
                      error = ?error,
                      scope,
                      "failed to load typed SMTP settings for notification dispatch; using empty fallback"
                  );
              }
              EmailSmtpSettings::default()
          }
      }
  }
  ```

- [ ] **Step 4: Expand the tenant SMTP call in `build_settings_bag`**

  Replace the `load_typed_settings_by_prefix` call (lines ~89–99) with the two-step
  raw_settings expansion:

  ```rust
  let tenant_smtp = typed_smtp_settings_or_empty(
      {
          let raw = uptrakit_shared_db::raw_settings::load_settings_by_prefix(
              db,
              tenant_id,
              SMTP_PREFIX,
          )
          .await;
          raw.and_then(|r| {
              uptrakit_shared_db::raw_settings::decode_prefixed_settings(SMTP_PREFIX, &r)
          })
      },
      "tenant",
      Some(tenant_id),
      SMTP_PASSWORD_AAD,
  );
  ```

- [ ] **Step 5: Expand the global SMTP call in `build_settings_bag`**

  Replace the `load_typed_global_settings_by_prefix` call (lines ~101–110):

  ```rust
  let global_smtp = typed_smtp_settings_or_empty(
      {
          let raw = uptrakit_shared_db::raw_settings::load_global_settings_by_prefix(
              db,
              GLOBAL_SMTP_PREFIX,
          )
          .await;
          raw.and_then(|r| {
              uptrakit_shared_db::raw_settings::decode_prefixed_settings(GLOBAL_SMTP_PREFIX, &r)
          })
      },
      "global",
      None,
      GLOBAL_SMTP_PASSWORD_AAD,
  );
  ```

- [ ] **Step 6: Replace the global Telegram call in `build_settings_bag`**

  Replace the `uptrakit_web_api_auth::settings_store::load_global_settings_by_prefix`
  call (lines ~112–117) with the direct raw_settings call:

  ```rust
  let global_telegram =
      uptrakit_shared_db::raw_settings::load_global_settings_by_prefix(db, GLOBAL_TELEGRAM_PREFIX)
          .await
          .unwrap_or_default();
  ```

- [ ] **Step 7: Verify all `uptrakit_web_api_auth` references are gone**

  `uptrakit_web_api_auth` is used **inline only** in `dispatcher.rs` — there is no
  top-level `use` statement for it. Steps 1, 4, 5, and 6 replace all four occurrences
  (one in the test, three in `build_settings_bag`). Confirm nothing remains:

  ```bash
  grep "uptrakit_web_api_auth" crates/ui/web-api/src/notifications/dispatcher.rs
  ```

  Expected: no output.

- [ ] **Step 8: Format and lint**

  ```bash
  cargo fmt -p uptrakit-web-api
  cargo clippy -p uptrakit-web-api --no-default-features --features db-sqlite 2>&1 | tail -20
  ```

  Expected: no warnings from `dispatcher.rs`.

- [ ] **Step 9: Run tests green**

  ```bash
  cargo test -p uptrakit-web-api --no-default-features --features db-sqlite 2>&1 | tail -20
  ```

  Expected: `test result: ok`.

- [ ] **Step 10: Commit**

  ```bash
  git commit --only crates/ui/web-api/src/notifications/dispatcher.rs \
    -m "fix(notifications): replace web-api-auth settings_store with raw_settings in build_settings_bag"
  ```

---

## Task 2: Create `uptrakit-notification-delivery` crate

**Files:**

- Create: `crates/plugins/notifications/delivery/Cargo.toml`
- Create: `crates/plugins/notifications/delivery/src/lib.rs`
- Create: `crates/plugins/notifications/delivery/src/event.rs`
- Create: `crates/plugins/notifications/delivery/src/message_builder.rs`
- Create: `crates/plugins/notifications/delivery/src/deliver.rs`
- Modify: `Cargo.toml` (workspace root)

> **⚠ Commits 2 and 3 must be consecutive. Do not commit anything else between them.**

- [ ] **Step 1: Add workspace dependency**

  In the root `Cargo.toml`, find the `[workspace.dependencies]` section where other
  `uptrakit-notification-plugin-*` entries live (around line 121) and add:

  ```toml
  uptrakit-notification-delivery = { path = "crates/plugins/notifications/delivery", version = "0.0.1" }
  ```

- [ ] **Step 2: Create `Cargo.toml` for the new crate**

  Create `crates/plugins/notifications/delivery/Cargo.toml`:

  ```toml
  [package]
  name = "uptrakit-notification-delivery"
  description = "Stateless notification delivery core for Uptrakit"
  edition.workspace = true
  license.workspace = true
  authors.workspace = true
  repository.workspace = true
  version.workspace = true

  [dependencies]
  uptrakit-notification-plugin-core  = { workspace = true }
  uptrakit-plugin-infrastructure-core = { workspace = true }
  uptrakit-web-api-types             = { workspace = true }
  rootcause                          = { workspace = true }
  uuid                               = { workspace = true }
  serde                              = { workspace = true }
  serde_json                         = { workspace = true }

  [dev-dependencies]
  async-trait = { workspace = true }
  tokio       = { workspace = true, features = ["macros", "rt"] }

  [lints]
  workspace = true
  ```

  Note: `uptrakit-plugin-infrastructure-core` is listed **without features** — its
  `sea-orm` dep is behind the optional `plugin-ops` and `agent-infra` features. No DB
  graph is pulled.

- [ ] **Step 3: Create `src/event.rs`**

  Copy the content of `crates/ui/web-api/src/notifications/events.rs` verbatim into
  `crates/plugins/notifications/delivery/src/event.rs`. No logic changes needed — all
  imports (`serde`, `uuid`, `uptrakit_web_api_types`) are in the new crate's deps.

  Verify the file compiles in isolation (next steps will confirm this).

- [ ] **Step 4: Create `src/message_builder.rs`**

  Copy the content of `crates/ui/web-api/src/notifications/message_builder.rs` into
  `crates/plugins/notifications/delivery/src/message_builder.rs`, then make one import
  change: replace the first line:

  Old:

  ```rust
  use uptrakit_plugin_infrastructure_registry::{DeliveryMessage, MessageAction, escape_html};
  ```

  New:

  ```rust
  use uptrakit_notification_plugin_core::{DeliveryMessage, MessageAction, escape_html};
  ```

  Also update the internal import of `NotificationEvent` and `NotificationEventDetails`:

  Old:

  ```rust
  use super::events::{NotificationEvent, NotificationEventDetails};
  ```

  New:

  ```rust
  use crate::event::{NotificationEvent, NotificationEventDetails};
  ```

  No other changes. All test code at the bottom of the file moves as-is.

- [ ] **Step 5: Create `src/deliver.rs`**

  Create `crates/plugins/notifications/delivery/src/deliver.rs` with the following
  complete content:

  ```rust
  use std::sync::Arc;

  use rootcause::Report;
  use uptrakit_notification_plugin_core::{DeliveryMessage, NotificationPluginError};
  use uptrakit_plugin_infrastructure_core::NotificationTransport;

  /// Error returned by [`deliver`].
  #[non_exhaustive]
  #[derive(Debug)]
  pub enum NotificationDeliveryError {
      DeliveryFailed(Report<NotificationPluginError>),
  }

  impl std::fmt::Display for NotificationDeliveryError {
      fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
          match self {
              Self::DeliveryFailed(e) => e.fmt(f),
          }
      }
  }

  impl std::error::Error for NotificationDeliveryError {}

  /// Invoke a transport for a single channel delivery.
  ///
  /// The caller is responsible for looking up the transport and handling
  /// `TransportNotFound` before calling this function.
  pub async fn deliver(
      transport: Arc<dyn NotificationTransport>,
      channel_config: &serde_json::Value,
      settings_bag: &serde_json::Value,
      message: &DeliveryMessage,
  ) -> Result<(), NotificationDeliveryError> {
      transport
          .deliver(channel_config, settings_bag, message)
          .await
          .map_err(NotificationDeliveryError::DeliveryFailed)
  }

  #[cfg(test)]
  mod tests {
      use std::sync::Arc;

      use async_trait::async_trait;
      use rootcause::report;
      use uptrakit_notification_plugin_core::{DeliveryMessage, NotificationPluginError};
      use uptrakit_plugin_infrastructure_core::{NotificationTransport, PluginMeta, PluginTypeId};

      use super::*;

      struct StubTransport {
          should_fail: bool,
      }

      impl PluginMeta for StubTransport {
          fn plugin_type_id(&self) -> PluginTypeId {
              PluginTypeId::new("stub")
          }
      }

      #[async_trait]
      impl NotificationTransport for StubTransport {
          async fn deliver(
              &self,
              _config: &serde_json::Value,
              _settings: &serde_json::Value,
              _message: &DeliveryMessage,
          ) -> uptrakit_notification_plugin_core::Result<()> {
              if self.should_fail {
                  Err(report!(NotificationPluginError::DeliveryFailed(
                      "stub error".to_string()
                  )))
              } else {
                  Ok(())
              }
          }
      }

      fn stub_message() -> DeliveryMessage {
          DeliveryMessage::new(
              "title",
              "body",
              None,
              serde_json::json!({}),
              vec![],
          )
      }

      #[tokio::test]
      async fn deliver_success_path() {
          let transport = Arc::new(StubTransport { should_fail: false });
          let result = deliver(
              transport,
              &serde_json::json!({}),
              &serde_json::json!({}),
              &stub_message(),
          )
          .await;
          assert!(result.is_ok());
      }

      #[tokio::test]
      async fn deliver_wraps_transport_error_as_delivery_failed() {
          let transport = Arc::new(StubTransport { should_fail: true });
          let result = deliver(
              transport,
              &serde_json::json!({}),
              &serde_json::json!({}),
              &stub_message(),
          )
          .await;
          assert!(
              matches!(result, Err(NotificationDeliveryError::DeliveryFailed(_))),
              "expected DeliveryFailed, got: {result:?}",
          );
      }
  }
  ```

- [ ] **Step 6: Create `src/lib.rs`**

  Create `crates/plugins/notifications/delivery/src/lib.rs`:

  ```rust
  mod deliver;
  mod event;
  mod message_builder;

  pub use deliver::{NotificationDeliveryError, deliver};
  pub use event::{ActionParams, NotificationEvent, NotificationEventDetails};
  pub use message_builder::build_delivery_message;
  ```

- [ ] **Step 7: Format, lint, and test the new crate**

  ```bash
  cargo fmt -p uptrakit-notification-delivery
  cargo clippy -p uptrakit-notification-delivery 2>&1 | tail -20
  cargo test -p uptrakit-notification-delivery 2>&1 | tail -20
  ```

  Expected test output:

  ```text
  test deliver::tests::deliver_success_path ... ok
  test deliver::tests::deliver_wraps_transport_error_as_delivery_failed ... ok
  test message_builder::tests::build_ca_rotated_message_has_no_actions ... ok
  test message_builder::tests::build_update_available_escapes_html_in_body_html ... ok
  test message_builder::tests::build_update_available_message ... ok
  test event::tests::action_params_requires_host_and_software_item ... ok
  test event::tests::event_details_serde_round_trip ... ok
  test event::tests::event_type_update_available ... ok
  test event::tests::event_type_update_completed ... ok
  test result: ok.
  ```

- [ ] **Step 8: Check all-features and deny**

  ```bash
  cargo check --all-features -p uptrakit-notification-delivery 2>&1 | tail -10
  cargo deny check 2>&1 | tail -10
  ```

  Expected: no errors.

- [ ] **Step 9: Commit (immediately proceed to Task 3 — no other commits before it)**

  ```bash
  git commit --only crates/plugins/notifications/delivery/ Cargo.toml \
    -m "feat(notifications): create uptrakit-notification-delivery crate scaffold"
  ```

---

## Task 3: Wire `web-api` to `uptrakit-notification-delivery`

**Files:**

- Delete: `crates/ui/web-api/src/notifications/events.rs`
- Delete: `crates/ui/web-api/src/notifications/message_builder.rs`
- Modify: `crates/ui/web-api/src/notifications/mod.rs`
- Modify: `crates/ui/web-api/src/notifications/dispatcher.rs`
- Modify: `crates/ui/web-api/Cargo.toml`

> **⚠ This is the second half of the atomic Commits 2+3 pair. Complete and commit
> this task before committing anything else.**

- [ ] **Step 1: Add the dependency to `web-api/Cargo.toml`**

  In `crates/ui/web-api/Cargo.toml`, find the `[dependencies]` block and add alongside
  the other `uptrakit-*` deps:

  ```toml
  uptrakit-notification-delivery = { workspace = true }
  ```

- [ ] **Step 2: Delete the two source files from `web-api`**

  ```bash
  rm crates/ui/web-api/src/notifications/events.rs
  rm crates/ui/web-api/src/notifications/message_builder.rs
  ```

- [ ] **Step 3: Replace `notifications/mod.rs`**

  The current content of `crates/ui/web-api/src/notifications/mod.rs` is:

  ```rust
  pub mod dispatcher;
  pub mod events;
  pub mod message_builder;
  ```

  Replace it entirely with:

  ```rust
  pub mod dispatcher;

  pub mod events {
      pub use uptrakit_notification_delivery::{
          ActionParams, NotificationEvent, NotificationEventDetails,
      };
  }
  ```

  Note: `pub mod message_builder;` is dropped with no shim — `message_builder` has no
  callers outside `dispatcher.rs` itself. The shim for `events` preserves the existing
  `crate::notifications::events::NotificationEvent` import path used by 14+ callers
  across `actions/`, `routes/`, and test files.

  Note: `routes/mod.rs` also has a `pub mod events` (at `crate::routes::events`) — that
  is unrelated. Do not touch it.

- [ ] **Step 4: Update imports in `dispatcher.rs`**

  At the top of `crates/ui/web-api/src/notifications/dispatcher.rs`, replace:

  ```rust
  use super::events::NotificationEvent;
  ```

  with:

  ```rust
  use uptrakit_notification_delivery::NotificationEvent;
  ```

  Then search for all remaining `super::events::` and `super::message_builder::` usages:

  ```bash
  grep -n "super::events::\|super::message_builder::" \
    crates/ui/web-api/src/notifications/dispatcher.rs
  ```

  There should be one `message_builder` reference (the `build_delivery_message` call,
  around line 370). Replace:

  ```rust
  let message = super::message_builder::build_delivery_message(
      &event,
      action_token,
      &callback_base_url,
      &channel_model.channel_type,
      channel_model.id,
  );
  ```

  with:

  ```rust
  let message = uptrakit_notification_delivery::build_delivery_message(
      &event,
      action_token,
      &callback_base_url,
      &channel_model.channel_type,
      channel_model.id,
  );
  ```

- [ ] **Step 5: Replace the `deliver` call in the `tokio::spawn` block**

  Find the spawn block in `dispatcher.rs` (around line 411). Replace:

  ```rust
  let channel_transport = channel_transport.clone();
  tokio::spawn(async move {
      match channel_transport
          .deliver(&config_json, &settings_bag, &message)
          .await
      {
          Ok(()) => {
  ```

  with:

  ```rust
  let channel_transport = channel_transport.clone();
  tokio::spawn(async move {
      match uptrakit_notification_delivery::deliver(
          channel_transport,
          &config_json,
          &settings_bag,
          &message,
      )
      .await
      {
          Ok(()) => {
  ```

  The `Err(e)` arm stays unchanged — `NotificationDeliveryError` implements `Display`
  (forwards to the inner `Report`), so `error = %e` and `e.to_string()` both work
  correctly.

- [ ] **Step 6: Verify no stale references remain**

  ```bash
  grep -n "super::events::\|super::message_builder::\|uptrakit_web_api_auth" \
    crates/ui/web-api/src/notifications/dispatcher.rs
  ```

  Expected: no output.

- [ ] **Step 7: Run the web-api test suite**

  ```bash
  cargo test -p uptrakit-web-api --no-default-features --features db-sqlite 2>&1 | tail -20
  ```

  Expected: `test result: ok`.

- [ ] **Step 8: Run the delivery crate tests**

  ```bash
  cargo test -p uptrakit-notification-delivery 2>&1 | tail -10
  ```

  Expected: `test result: ok`.

- [ ] **Step 9: Format, lint, all-features check, deny**

  ```bash
  cargo fmt -p uptrakit-web-api -p uptrakit-notification-delivery
  cargo clippy -p uptrakit-web-api -p uptrakit-notification-delivery \
    --no-default-features --features db-sqlite 2>&1 | tail -20
  cargo check --all-features 2>&1 | tail -10
  cargo deny check 2>&1 | tail -10
  ```

  Expected: no errors or warnings.

- [ ] **Step 10: Commit**

  ```bash
  git commit --only crates/ui/web-api/src/notifications/ crates/ui/web-api/Cargo.toml \
    -m "refactor(web-api): wire dispatcher to uptrakit-notification-delivery"
  ```

---

## Task 4: Update ADR-0001

**Files:**

- Modify: `docs/adr/0001-web-api-decomposition-strategy.md`

- [ ] **Step 1: Update the candidates table**

  Find the table row:

  ```markdown
  | Notification dispatcher | Approved — next | No Axum dependency; requires `build_settings_bag` refactor first (see Consequences) |
  ```

  Replace with:

  ```markdown
  | Notification delivery core (`uptrakit-notification-delivery`) | Completed | Spec: `docs/superpowers/specs/2026-05-01-notification-delivery-extraction-design.md`. `dispatch_loop` (queue, rule matching, channel loading, log writing) remains in `web-api` — it is stateful orchestration, not delivery. |
  ```

- [ ] **Step 2: Update the Consequences section**

  Find the notification pre-condition bullet:

  ```markdown
  - **Notification dispatcher pre-condition:** `dispatcher.rs` currently imports
    `uptrakit_web_api_auth::settings_store` to build SMTP/Telegram settings in
    `build_settings_bag`. A second call site exists in
    `surface_proxy/controller_local/notifications.rs`. Before extraction, refactor
    `build_settings_bag` to accept pre-loaded settings values as parameters rather than
    loading them internally, decoupling the dispatcher from `web-api-auth` and
    `uptrakit-shared-db`. Until this refactor is done, extracting the dispatcher
    carries the full `sea-orm`/`sqlx` compile graph and achieves no compile-time
    benefit.
  ```

  Replace with:

  ```markdown
  - **Notification delivery pre-condition (fulfilled):** `dispatcher.rs` imported
    `uptrakit_web_api_auth::settings_store` for three functions that were pure DB reads
    with no auth logic. These were replaced with direct `uptrakit_shared_db::raw_settings`
    calls (commit 1 of the extraction). The stateless delivery core (`events.rs`,
    `message_builder.rs`, `deliver()`) was then extracted into `uptrakit-notification-delivery`
    (commits 2–3). The `dispatch_loop` (queue, rule matching, channel loading, log writing)
    remains in `web-api` — it is inherently DB-coupled stateful orchestration.
  ```

- [ ] **Step 3: Update the `surface_proxy` sequencing note**

  Find:

  ```markdown
  - **surface_proxy sequencing note:** `surface_proxy` calls `build_settings_bag`
    from the notification dispatcher. Extracting the dispatcher first means that when
    `surface_proxy` is subsequently extracted, it will depend on
    `uptrakit-notification-dispatch` in addition to `uptrakit-web-api-queries`. Plan
    for this rather than discovering it mid-extraction.
  ```

  Replace with:

  ```markdown
  - **surface_proxy sequencing note:** `surface_proxy` calls `build_settings_bag`
    from the notification dispatcher. When `surface_proxy` is subsequently extracted,
    it will depend on `uptrakit-notification-delivery` in addition to
    `uptrakit-web-api-queries`. Plan for this rather than discovering it mid-extraction.
  ```

- [ ] **Step 4: Format and lint**

  ```bash
  npx prettier --write docs/adr/0001-web-api-decomposition-strategy.md
  npx markdownlint --config .markdownlint.json docs/adr/0001-web-api-decomposition-strategy.md
  ```

  Expected: no errors.

- [ ] **Step 5: Commit**

  ```bash
  git commit --only docs/adr/0001-web-api-decomposition-strategy.md \
    -m "docs(adr): mark notification delivery core extraction as completed"
  ```
