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

Four self-contained commits, each leaving the codebase green.

### Commit 1 — Fix `build_settings_bag` settings dependency

**Goal:** Remove `uptrakit_web_api_auth` import from `dispatcher.rs`.

`build_settings_bag` calls three functions from `uptrakit_web_api_auth::settings_store`:
`load_typed_settings_by_prefix`, `load_typed_global_settings_by_prefix`, and
`load_global_settings_by_prefix`. These are already one-line wrappers over
`uptrakit_shared_db::raw_settings`, which `dispatcher.rs` can reach directly since
`uptrakit-shared-db` is already a transitive dependency.

Replace all three call sites in `dispatcher.rs` with direct `uptrakit_shared_db::raw_settings`
calls. Remove the `uptrakit_web_api_auth::settings_store` import. Audit whether
`uptrakit-web-api-auth` remains used elsewhere in `dispatcher.rs`; if not, drop it
from the imports entirely.

The second call site of `build_settings_bag` lives in
`surface_proxy/controller_local/notifications.rs` — that file uses the function via
`crate::notifications::dispatcher::build_settings_bag`, not via `web-api-auth`
directly, so no change needed there.

No behaviour change. All existing tests pass.

### Commit 2 — Create `uptrakit-notification-delivery` crate scaffold

**Goal:** New crate with `events.rs`, `message_builder.rs` moved in; `deliver()` added.

Create `crates/plugins/notifications/delivery/Cargo.toml`. Register the crate in two
places in the root `Cargo.toml`: add `crates/plugins/notifications/delivery` to
`[workspace.members]` and add `uptrakit-notification-delivery = { path = "crates/plugins/notifications/delivery", version = "..." }` to `[workspace.dependencies]`.

Move `notifications/events.rs` → `src/event.rs` and
`notifications/message_builder.rs` → `src/message_builder.rs` into the new crate with
import path updates only (no logic changes).

Add `src/deliver.rs` with a new public function:

```rust
/// Invoke a notification transport for a single channel.
///
/// Looks up the transport by `plugin_type_id` via `ops`. Returns
/// `TransportNotFound` if no plugin is registered for that type.
pub async fn deliver(
    ops: &dyn PluginOps,
    plugin_type_id: &PluginTypeId,
    channel_config: &serde_json::Value,
    settings_bag: &serde_json::Value,
    message: &DeliveryMessage,
) -> Result<(), NotificationDeliveryError>

#[non_exhaustive]
pub enum NotificationDeliveryError {
    TransportNotFound,
    DeliveryFailed(rootcause::Report<NotificationPluginError>),
}
```

`lib.rs` public surface:

```rust
pub use event::{ActionParams, NotificationEvent, NotificationEventDetails};
pub use message_builder::build_delivery_message;
pub use deliver::{deliver, NotificationDeliveryError};
```

**Crate dependencies:**

```toml
[dependencies]
uptrakit-plugin-infrastructure-registry = { workspace = true }
uptrakit-web-api-types                  = { workspace = true }
rootcause                               = { workspace = true }
uuid                                    = { workspace = true }
serde                                   = { workspace = true }
serde_json                              = { workspace = true }
```

No `sea-orm`, no `uptrakit-shared-db`, no Axum. This crate does not depend on
`uptrakit-web-api` or any UI-layer crate.

At this point the new crate compiles and its tests pass. `web-api` is not yet updated.

### Commit 3 — Update `web-api` to use `uptrakit-notification-delivery`

**Goal:** Remove `events.rs` and `message_builder.rs` from `web-api`; update
`dispatcher.rs` to use the new crate.

Delete `notifications/events.rs` and `notifications/message_builder.rs` from `web-api`.

Add re-exports in `notifications/mod.rs` so existing callers (`actions/`, `routes/`)
are unaffected:

```rust
pub use uptrakit_notification_delivery::{
    ActionParams, NotificationEvent, NotificationEventDetails,
};
```

Update `dispatcher.rs`:

- Replace `super::events::*` import with `uptrakit_notification_delivery::*`
- Replace `super::message_builder::build_delivery_message` with
  `uptrakit_notification_delivery::build_delivery_message`
- Replace inline `transport.deliver(...)` call with
  `uptrakit_notification_delivery::deliver(&*notification_ops, ...)` — propagate
  `NotificationDeliveryError` into the existing error logging path

Add `uptrakit-notification-delivery = { workspace = true }` to
`web-api/Cargo.toml`.

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
    mod.rs          ← re-exports NotificationEvent etc. from delivery crate
```

`dispatch_loop` data flow:

```text
receive NotificationEvent
  → load matching rules (DB)
  → for each rule:
      load channel (DB)
      build_settings_bag(&db, tenant_id)        ← web-api, raw_settings
      build_delivery_message(&event, ...)       ← notification-delivery crate
      deliver(&ops, type_id, config, settings, &message)  ← notification-delivery crate
      write notification_log (DB)               ← web-api
```

---

## Testing

### `uptrakit-notification-delivery` (no DB required)

- Existing `events.rs` tests move as-is: event type derivation, action params,
  serialization round-trips.
- Existing `message_builder.rs` tests move as-is: message content, HTML escaping.
- New `deliver()` tests using a stub `NotificationTransport` impl:
  - Success path: stub returns `Ok(())`, function returns `Ok(())`
  - `TransportNotFound`: `NotificationOps::transport()` returns `None`
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
