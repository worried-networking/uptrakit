# Notification Delivery Extraction Design

Extract the stateless notification delivery core out of `uptrakit-web-api` into a new
`uptrakit-notification-delivery` crate, and fix a misplaced dependency in
`build_settings_bag` as a prerequisite. The result is a delivery crate with zero DB
and zero HTTP dependencies, unit-testable in isolation, and a `dispatch_loop` in
`web-api` that retains only stateful orchestration (rule matching, channel loading,
log writing).

---

## Motivation

`notifications/dispatcher.rs` currently imports `uptrakit_web_api_auth::settings_store`
for three functions that are pure DB reads with no auth logic — wrong crate. This
coupling is the blocker for extracting the notification delivery subsystem per
ADR-0001.

Beyond the settings fix, `events.rs` and `message_builder.rs` are already stateless
(zero DB, zero Axum) but are buried inside `web-api`. Extracting them into their own
crate gives notification delivery a clear home, makes message-building and transport
invocation unit-testable without a DB, and lets new notification channel types be
added without touching the orchestration loop.

---

## Commit Sequence

Four self-contained commits, each leaving the codebase green. Commits 2 and 3 must
be consecutive with no other commits between them — between them `events.rs` and
`message_builder.rs` exist in both locations simultaneously, which is an intermediate
state that must never land on `main` in isolation.

### Commit 1 — Fix `build_settings_bag` settings dependency

**Goal:** Remove `uptrakit_web_api_auth` import from `dispatcher.rs`.

`build_settings_bag` calls three functions from `uptrakit_web_api_auth::settings_store`:
`load_typed_settings_by_prefix`, `load_typed_global_settings_by_prefix`, and
`load_global_settings_by_prefix`. Replace them with direct
`uptrakit_shared_db::raw_settings` calls — `uptrakit-shared-db` is already a direct
dep of `web-api`.

Expansion detail:

- `load_typed_settings_by_prefix` → one async `load_settings_by_prefix` call + one
  synchronous `decode_prefixed_settings` call
- `load_typed_global_settings_by_prefix` → one async `load_global_settings_by_prefix` call + one
  synchronous `decode_prefixed_settings` call
- `load_global_settings_by_prefix` → single async raw call, no decode step

The `typed_smtp_settings_or_empty` helper currently takes
`uptrakit_web_api_auth::auth::Result<EmailSmtpSettings>`. Update its parameter type
to `uptrakit_shared_db::raw_settings::Result<EmailSmtpSettings>` (wraps
`RawSettingsError`). Update the existing unit test — it constructs `AuthError::Internal`
as the error value; replace with `RawSettingsError::Decode("boom".into())`, which is
the path-of-least-resistance `RawSettingsError` variant constructible in a unit test
without a live DB.

Remove the `uptrakit_web_api_auth::settings_store` import. Audit whether
`uptrakit-web-api-auth` remains used elsewhere in `dispatcher.rs`; if not, drop it
from the imports entirely.

The second call site of `build_settings_bag` lives in
`surface_proxy/controller_local/notifications.rs` — it calls via
`crate::notifications::dispatcher::build_settings_bag`, so no change needed there.

No behaviour change. All existing tests pass.

### Commit 2 — Create `uptrakit-notification-delivery` crate scaffold

**Goal:** New crate with `events.rs`, `message_builder.rs` moved in; `deliver()` added.

Create `crates/plugins/notifications/delivery/Cargo.toml`. The root `Cargo.toml`
workspace glob already covers `crates/plugins/notifications/*`, so no manual
`[workspace.members]` edit is needed. Add one entry to `[workspace.dependencies]`:

```toml
uptrakit-notification-delivery = { path = "crates/plugins/notifications/delivery", version = "0.0.1" }
```

Move `notifications/events.rs` → `src/event.rs` (rename from plural to singular) and
`notifications/message_builder.rs` → `src/message_builder.rs` into the new crate.
Update `message_builder.rs` imports: replace
`uptrakit_plugin_infrastructure_registry::{DeliveryMessage, MessageAction, escape_html}`
with `uptrakit_notification_plugin_core::{DeliveryMessage, MessageAction, escape_html}`.
No logic changes.

Add `src/deliver.rs`:

```rust
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
    transport.deliver(channel_config, settings_bag, message).await
        .map_err(|e| NotificationDeliveryError::DeliveryFailed(e))
}

#[non_exhaustive]
pub enum NotificationDeliveryError {
    DeliveryFailed(rootcause::Report<NotificationPluginError>),
}
```

`NotificationTransport` comes from `uptrakit-plugin-infrastructure-core`; `DeliveryMessage`
and `NotificationPluginError` come from `uptrakit-notification-plugin-core`.
(`uptrakit-plugin-infrastructure-core` has `sea-orm` only under optional feature flags —
adding it without features pulls no DB deps.) The dispatcher keeps transport lookup
(`notification_ops.transport(...)`) — `NotificationOps` and `PluginOps` never appear
in this crate.

`lib.rs` public surface:

```rust
mod event;
mod message_builder;
mod deliver;

pub use event::{ActionParams, NotificationEvent, NotificationEventDetails};
pub use message_builder::build_delivery_message;
pub use deliver::{deliver, NotificationDeliveryError};
```

**Crate dependencies:**

```toml
[dependencies]
uptrakit-notification-plugin-core  = { workspace = true }
uptrakit-plugin-infrastructure-core = { workspace = true }
uptrakit-web-api-types             = { workspace = true }
rootcause                          = { workspace = true }
uuid                               = { workspace = true }
serde                              = { workspace = true }
serde_json                         = { workspace = true }
```

Do not add `sea-orm`, `uptrakit-shared-db`, `uptrakit-plugin-infrastructure-registry`,
or any DB-related dep — the registry has `sea-orm` as a direct dep and must not be
used here. `uptrakit-plugin-infrastructure-core` is explicitly allowed: its `sea-orm`
dep is behind optional features (`plugin-ops`, `agent-infra`); adding it with no
features pulls no DB graph. This crate does not depend on `uptrakit-web-api` or any
UI-layer crate.

At this point the new crate compiles and its tests pass. `web-api` is not yet updated.

### Commit 3 — Update `web-api` to use `uptrakit-notification-delivery`

**Goal:** Remove `events.rs` and `message_builder.rs` from `web-api`; update
`dispatcher.rs` to use the new crate.

Delete `notifications/events.rs` and `notifications/message_builder.rs` from `web-api`.

Replace the `pub mod events;` declaration in `notifications/mod.rs` with an inline shim
— existing callers that import `crate::notifications::events::NotificationEvent` continue
to compile unchanged. `pub mod message_builder;` is dropped entirely: `message_builder`
has no callers outside `dispatcher.rs` itself, so no shim is needed.

Note: `routes/mod.rs` also has a `pub mod events` at a different path
(`crate::routes::events`) — this is an unrelated module and does not conflict with the
notification shim. Grepping for `pub mod events` will surface both; only the one in
`notifications/mod.rs` is being replaced.

The shim:

```rust
pub mod events {
    pub use uptrakit_notification_delivery::{
        ActionParams, NotificationEvent, NotificationEventDetails,
    };
}
```

Update `dispatcher.rs`:

- Grep for all `super::events::` and `super::message_builder::` references and
  replace with `uptrakit_notification_delivery::` equivalents.
- The `dispatch_loop` already looks up `channel_transport: Arc<dyn NotificationTransport>`
  before the `tokio::spawn` block. Keep that lookup in place. Inside the spawn,
  replace `channel_transport.deliver(config, settings, message).await` with
  `uptrakit_notification_delivery::deliver(Arc::clone(&channel_transport), config, settings, message).await`.
  Map `NotificationDeliveryError::DeliveryFailed(e)` into the existing error-logging
  path. `TransportNotFound` handling (already in the pre-spawn lookup) is unchanged.

Add `uptrakit-notification-delivery = { workspace = true }` to `web-api/Cargo.toml`.

Full test suite green: `cargo test -p uptrakit-web-api --all-features` and
`cargo test -p uptrakit-notification-delivery`.

### Commit 4 — Update ADR-0001

**Goal:** Correct the extraction target from "notification dispatcher" to
"notification delivery core."

In `docs/adr/0001-web-api-decomposition-strategy.md`:

- Update the candidates table: change "Notification dispatcher" row to
  "Notification delivery core (`uptrakit-notification-delivery`)" with status
  "Completed".
- Update the notification pre-condition bullet to note that the `build_settings_bag`
  fix (commit 1) and the crate extraction (commits 2–3) fulfilled the pre-condition.
- Clarify that the `dispatch_loop` (queue, rule matching, channel loading, log
  writing) remains in `web-api` — it is stateful orchestration, not delivery.

---

## Architecture After

```text
uptrakit-notification-delivery          (new crate, no DB, no Axum)
  src/event.rs            ← NotificationEvent, NotificationEventDetails
  src/message_builder.rs  ← build_delivery_message
  src/deliver.rs          ← deliver(), NotificationDeliveryError

uptrakit-web-api  (depends on uptrakit-notification-delivery)
  notifications/
    dispatcher.rs   ← queue + dispatch_loop + build_settings_bag (raw_settings)
    mod.rs          ← events shim + top-level re-exports
```

`dispatch_loop` data flow after:

```text
receive NotificationEvent
  → load matching rules (DB)
  → for each rule:
      load channel (DB)
      build_settings_bag(&db, tenant_id)                       ← web-api, raw_settings
      look up transport via notification_ops.transport(...)    ← web-api, PluginOps
      build_delivery_message(&event, ...)                      ← notification-delivery crate
      deliver(Arc::clone(&transport), config, settings, &msg)  ← notification-delivery crate
      write notification_log (DB)                              ← web-api
```

---

## Testing

### `uptrakit-notification-delivery` (no DB required)

- Existing `events.rs` tests move as-is: event type derivation, action params,
  serialization round-trips.
- Existing `message_builder.rs` tests move as-is: message content, HTML escaping.
- New `deliver()` tests using a stub struct implementing `NotificationTransport`:
  - Success path: stub returns `Ok(())`, `deliver()` returns `Ok(())`
  - `DeliveryFailed`: stub returns `Err(...)`, error is wrapped and returned

### `uptrakit-web-api` (unchanged pattern)

- `NotificationDispatcher::test_channel()` tests remain: verify actions enqueue
  correct `NotificationEvent` variants.
- `dispatch_loop` integration tests (under `db-sqlite` feature) cover rule matching,
  channel loading, delivery dispatch, and log writing end-to-end. These require a
  migrated in-process DB — appropriate for orchestration-level tests.

---

## Out of Scope

- Extracting `NotificationDispatcher` itself (the queue + loop) — it is inherently
  DB-coupled and stays in `web-api` per ADR-0001.
- Changing the `NotificationEvent` schema or adding new event types.
- Moving `build_settings_bag` to a separate module; it stays in `dispatcher.rs` after
  the import fix.
