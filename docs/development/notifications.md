# Notification subsystem

Development guide for the plugin-based notification subsystem. This document covers the architecture, crate layout,
dispatcher flow, and how to extend the system with new notification plugins.

## Architecture overview

The notification subsystem follows a strict plugin-agnostic pipeline:

```text
Event producers --> NotificationEvent --> Dispatcher --> match rules --> DeliveryMessage --> NotificationTransport::deliver()
```

**Event producers** know nothing about notification plugins. They emit a `NotificationEvent` containing contextual data
(tenant, host, software item) and a typed `NotificationEventDetails` variant. The **dispatcher** is the single
translation point: it matches rules, builds a `DeliveryMessage` via `message_builder`, and hands it to the plugin
implementation. **Plugins** receive only `DeliveryMessage` and render it into their native format (JSON POST for
webhooks, HTML message with inline keyboard for Telegram, etc.).

This separation means adding a new notification plugin never requires changes to event-producing code.

## Crate structure

| Crate                                     | Path                                                     | Purpose                                                                                                                                                             |
| ----------------------------------------- | -------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `uptrakit-notification-plugin-core`       | `crates/plugins/notifications/core/`                     | `DeliveryMessage`, `MessageAction`, `NotificationPluginError`, `escape_html()`                                                                                      |
| `uptrakit-notification-plugin-webhook`    | `crates/plugins/notifications/webhook/`                  | Webhook plugin (SSRF validation, header blocklist, HMAC-SHA256 signing); typed `WebhookChannelConfig` + `NotificationTransport` impl                                |
| `uptrakit-notification-plugin-telegram`   | `crates/plugins/notifications/telegram/`                 | Telegram plugin (inline keyboard support); typed `TelegramChannelConfig` + `NotificationTransport` impl                                                             |
| `uptrakit-notification-plugin-email`      | `crates/plugins/notifications/email/`                    | Email plugin (SMTP via mail-send, `SmtpSettingsSnapshot`, `merge_smtp_into_config()`); typed `EmailChannelConfig` + `NotificationTransport` impl                    |
| `uptrakit-plugin-infrastructure-core`     | `crates/plugins/infrastructure/core/`                    | `NotificationTransport` role trait, `PluginMeta`, `PluginDescriptor`, `PluginConfig` trait, `PluginFamily`, `ConfigModel`, `CatalogConfig`, `declare_plugin!` macro |
| `uptrakit-plugin-infrastructure-registry` | `crates/plugins/infrastructure/registry/`                | `PluginCatalog` registers all plugins (software, notification, enhancement) via descriptors; transport lookup via `catalog.transport(&PluginTypeId)`                |
| `uptrakit-web-api-types`                  | `crates/shared/web-api-types/src/notifications/`         | Shared request/response types, public enums (`NotificationEventType`, `NotificationDeliveryStatus`); `channel_type` is `String` (not an enum)                       |
| `uptrakit-web-api`                        | `crates/ui/web-api/src/notifications/`                   | Dispatcher, internal event types, `message_builder`                                                                                                                 |
| `uptrakit-web-api-queries`                | `crates/ui/web-api-queries/src/queries/notifications.rs` | DB query helpers (CRUD for channels, rules, log)                                                                                                                    |
| `uptrakit-web-api`                        | `crates/ui/web-api/src/routes/notifications.rs`          | REST API route handlers + generic notification callback endpoint                                                                                                    |

## Feature flags

| Feature                  | Crate                            | Default | Description                                                                                   |
| ------------------------ | -------------------------------- | ------- | --------------------------------------------------------------------------------------------- |
| `notifications-webhook`  | `plugin-infrastructure-registry` | no      | Webhook plugin                                                                                |
| `notifications-telegram` | `plugin-infrastructure-registry` | no      | Telegram plugin with inline keyboard                                                          |
| `notifications-email`    | `plugin-infrastructure-registry` | no      | Email plugin (SMTP via mail-send, async TLS)                                                  |
| `notifications-telegram` | `web-api`, `controller`          | no      | Propagated feature flag enabling Telegram                                                     |
| `notifications-email`    | `web-api`, `controller`          | no      | Propagated feature flag enabling email                                                        |
| `notifications-all`      | `web-api`                        | no      | Enables all optional notification plugins                                                     |
| `notifications-all`      | `controller`                     | **yes** | Enables all optional notification plugins (default since `notifications-all` is in `default`) |

Feature flags are additive and chain through the dependency graph:

```text
crates/core/controller/Cargo.toml   crates/ui/web-api/Cargo.toml       crates/plugins/infrastructure/registry/Cargo.toml
  notifications-telegram       --->   notifications-telegram       --->   notifications-telegram
  notifications-email          --->   notifications-email          --->   notifications-email
```

The `web-api` always depends on `plugin-infrastructure-registry` with the `notifications-webhook` feature enabled,
ensuring webhooks are always compiled in.

## Unified plugin framework

Notification plugins use the same unified plugin framework as software and enhancement plugins.
There is no separate notification-only registry. Notification transports register through
`PluginCatalog` via their `PluginDescriptor`, and notification transport lookup is exposed through
the shared `NotificationOps` trait implemented by `PluginCatalog`.

### `PluginMeta` trait

Every plugin struct implements `PluginMeta` (defined in `crates/plugins/infrastructure/core/src/roles.rs`):

```rust
pub trait PluginMeta: Send + Sync + 'static {
    fn plugin_type_id(&self) -> PluginTypeId;
}
```

The `plugin_type_id()` replaces the old `name()` and `channel_type()` methods. It returns
a typed `PluginTypeId` (e.g. `PluginTypeId::new("notifications.webhook")`).

### `NotificationTransport` role trait

Notification plugins implement the `NotificationTransport` role trait
(defined in `crates/plugins/infrastructure/core/src/roles.rs`):

```rust
#[async_trait]
pub trait NotificationTransport: PluginMeta {
    async fn deliver(
        &self,
        config: &serde_json::Value,
        settings: &serde_json::Value,
        message: &DeliveryMessage,
    ) -> Result<()>;
}
```

The `settings` parameter is a generic JSON bag with the structure
`{"tenant": {"key": value, ...}, "global": {"key": value, ...}}`. Each plugin extracts
what it needs internally (e.g. the email plugin performs SMTP merge, the telegram plugin
extracts `bot_token` from tenant settings, the webhook plugin ignores it).

There is no `channel_type()` method on the trait. The channel type (`"webhook"`/`"telegram"`/`"email"`,
a separate, runtime-validated concept used by the notification-dispatch subsystem) is namespaced into the
plugin's `type_id` from its `PluginDescriptor` (`"notifications.webhook"`, etc.), which is also the value
returned by `plugin_type_id()`. Code deriving one from the other uses
`uptrakit_shared_types::notification_plugin_type(channel_type)`
(`crates/shared/types/src/plugin_type_id.rs`) rather than assuming equality.

There is no `supports_actions()` method. Each plugin decides independently whether to render `DeliveryMessage.actions`.
Plugins that do not support interactive elements silently ignore the `actions` field.

### `PluginConfig` trait

Each notification plugin has a typed config struct implementing `PluginConfig`
(defined in `crates/plugins/infrastructure/core/src/plugin_config.rs`):

```rust
pub trait PluginConfig:
    Serialize + DeserializeOwned + Default + Clone + Send + Sync + 'static
{
    fn validate(&self) -> Result<(), String> { Ok(()) }
    fn with_secrets_masked(self) -> Self { self }
    fn restore_secrets_from(&mut self, _existing: &Self) {}
    fn form_schema() -> Vec<FormField> { vec![] }
}
```

Concrete config structs:

- `WebhookChannelConfig` -- fields: `url`, `secret`, `headers`
- `TelegramChannelConfig` -- fields: `bot_token`, `chat_id`, `webhook_secret`
- `EmailChannelConfig` -- fields: `to_addresses`

The `declare_plugin!` macro generates JSON-level wrapper functions (`ConfigOps`) that delegate
to the typed `PluginConfig` methods, handling serialization/deserialization automatically.

### `declare_plugin!` macro

Every notification plugin uses `declare_plugin!` to generate its `PluginDescriptor` and
`PluginMeta` implementation:

```rust
declare_plugin!(WebhookPlugin, WebhookChannelConfig, "notifications.webhook", {
    display_name: "Webhook",
    family: PluginFamily::Notification,
    config_model: ConfigModel::NotificationChannel,
    roles: [NotificationTransport],
    notification_transport: create_webhook_transport,
    raw_settings_keys: &[],
    surfaces: {
        registrations: webhook_plugin_surfaces,
    },
});
```

`surfaces:` is the single source for both the surface/interaction registrations served to
`SurfaceRegistry` and the exact-id dispatch map `PluginCatalog` derives for `PluginHandled`
interactions (ADR-0028); there is no separate `surface_actions:`/`owned_surface_ids:` arm to keep in
sync.

The macro generates:

- A `pub static DESCRIPTOR: PluginDescriptor` with all metadata, config ops, role creators,
  and surface registrations.
- An `impl PluginMeta for WebhookPlugin` that returns `PluginTypeId::from_static("notifications.webhook")`.
- Compile-time assertions that the plugin struct implements all declared role traits.

### Transport creation and lookup

Transport creation uses `CreateTransportFn`:

```rust
pub type CreateTransportFn =
    fn(&CatalogConfig) -> Result<Arc<dyn NotificationTransport>>;
```

`CatalogConfig` carries shared configuration (notably `allow_private_urls: bool`) that was
previously on the removed `NotificationRegistryConfig`.

At startup, the `PluginCatalog` calls each notification plugin's `CreateTransportFn` to
construct a singleton `Arc<dyn NotificationTransport>`. The dispatcher looks up transports via:

```rust
catalog.transport(&uptrakit_shared_types::notification_plugin_type(channel_type))
```

This replaces the old `notification_ops.transport(channel_type)` pattern.

### `DeliveryMessage`

```rust
pub struct DeliveryMessage {
    pub title: String,            // One-line human-readable title
    pub body: String,             // Multi-line plain-text body
    pub body_html: Option<String>, // Optional HTML body for rich-text channels
    pub event_payload: serde_json::Value, // Machine-readable payload (webhook JSON bodies)
    pub actions: Vec<MessageAction>,      // Optional action buttons
}
```

### `MessageAction`

```rust
pub struct MessageAction {
    pub label: String,        // Button label (e.g. "Install 2.0")
    pub callback_url: String, // URL the channel calls when the button is pressed
    pub token: String,        // Opaque action token (UUIDv7)
}
```

### `NotificationPluginError`

All plugin operations return `Result<T>` which is `Result<T, Report<NotificationPluginError>>`. The error variants are:

- `InvalidConfig` -- plugin-specific config is invalid
- `DeliveryFailed` -- delivery to the external service failed
- `RecipientsFailed` -- delivery failed for one or more recipients (email attempt-all; carries the per-recipient list)
- `HttpRequest` -- underlying HTTP request failed
- `HttpClientBuild` -- `reqwest::Client` could not be constructed
- `Serialization` -- payload serialization failed
- `HmacKey` -- HMAC key construction failed

## Adding a new notification plugin

Follow these steps to add a plugin (for example, `slack`):

### 1. Create the plugin crate

Create a new crate at `crates/plugins/notifications/slack/` with a `Cargo.toml` depending on
`uptrakit-notification-plugin-core` and `uptrakit-plugin-infrastructure-core`:

```toml
[package]
name = "uptrakit-notification-plugin-slack"

[dependencies]
uptrakit-notification-plugin-core = { workspace = true, features = ["channel_admin"] }
uptrakit-plugin-infrastructure-core = { workspace = true, features = ["plugin-ops"] }
uptrakit-shared-db = { workspace = true }
async-trait = { workspace = true }
reqwest = { workspace = true }
rootcause = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
```

### 2. Define the typed config struct

Create `crates/plugins/notifications/slack/src/config.rs` implementing `PluginConfig`:

```rust
use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::PluginConfig;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SlackChannelConfig {
    #[serde(default)]
    pub webhook_url: String,
}

impl PluginConfig for SlackChannelConfig {
    fn validate(&self) -> Result<(), String> {
        if self.webhook_url.is_empty() {
            return Err("'webhook_url' is required".to_string());
        }
        if !self.webhook_url.starts_with("https://") {
            return Err("'webhook_url' must start with https://".to_string());
        }
        Ok(())
    }

    fn with_secrets_masked(mut self) -> Self {
        if !self.webhook_url.is_empty() {
            self.webhook_url = "***".to_string();
        }
        self
    }

    fn restore_secrets_from(&mut self, existing: &Self) {
        if self.webhook_url == "***" {
            self.webhook_url = existing.webhook_url.clone();
        }
    }
}
```

### 3. Implement `NotificationTransport`

Create `crates/plugins/notifications/slack/src/plugin.rs`:

```rust
use std::sync::Arc;
use std::future::Future;
use std::pin::Pin;

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_notification_plugin_core::{DeliveryMessage, NotificationPluginError, Result};
use uptrakit_plugin_infrastructure_core::{
    CatalogConfig, ConfigModel, NotificationTransport, PluginFamily, declare_plugin,
};

use crate::config::SlackChannelConfig;

pub struct SlackPlugin {
    http: reqwest::Client,
}

impl SlackPlugin {
    pub fn new() -> Result<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| report!(NotificationPluginError::HttpClientBuild(e.to_string())))?;
        Ok(Self { http })
    }
}

#[async_trait]
impl NotificationTransport for SlackPlugin {
    async fn deliver(
        &self,
        config: &serde_json::Value,
        _settings: &serde_json::Value,
        message: &DeliveryMessage,
    ) -> Result<()> {
        // Build Slack Block Kit payload from message.title, message.body, etc.
        todo!()
    }
}

fn create_slack_transport(
    _config: &CatalogConfig,
) -> uptrakit_plugin_infrastructure_core::Result<
    Arc<dyn NotificationTransport>,
> {
    Ok(Arc::new(
        SlackPlugin::new().map_err(|e| {
            rootcause::report!(
                uptrakit_plugin_infrastructure_core::PluginError::Configuration(e.to_string())
            )
        })?,
    ))
}

// Interaction registration functions omitted for brevity -- see Step 5.

declare_plugin!(SlackPlugin, SlackChannelConfig, "slack", {
    display_name: "Slack",
    family: PluginFamily::Notification,
    config_model: ConfigModel::NotificationChannel,
    roles: [NotificationTransport],
    notification_transport: create_slack_transport,
    raw_settings_keys: &[],
    surfaces: {
        registrations: slack_plugin_surfaces,
    },
});
```

Key requirements:

- HTTP client **must** set `.connect_timeout(10s)` and `.timeout(60s)` (see [coding-standards.md](coding-standards.md)).
- `with_secrets_masked()` must replace all secret fields with `"***"`.
- `restore_secrets_from()` must restore secrets when the incoming value is `"***"`.
- Use `report!()` / `bail!()` macros for error creation, never `Report::new()` directly.
- No `unwrap()` in production code.

### 4. Register in the `PluginCatalog`

Add a feature flag in `crates/plugins/infrastructure/registry/Cargo.toml` and add the
plugin's `DESCRIPTOR` to the catalog registration list:

```toml
[features]
default = ["webhook"]
webhook = ["uptrakit-notification-plugin-webhook"]
telegram = ["uptrakit-notification-plugin-telegram"]
email = ["uptrakit-notification-plugin-email"]
slack = ["uptrakit-notification-plugin-slack"]       # <-- new
all = ["webhook", "telegram", "email", "slack"]      # <-- update
```

In the `all_descriptors()` function (`crates/plugins/infrastructure/registry/src/registry.rs`), add the
descriptor behind its feature gate:

```rust
#[cfg(feature = "slack")]
descriptors.push(&uptrakit_notification_plugin_slack::DESCRIPTOR);
```

The `PluginCatalog` reads each descriptor's `family`, `config_model`, and `roles` to
automatically register the transport singleton, config ops, and surface action handlers.

### 5. Add interaction handlers and register them

Each notification plugin owns its own `surfaces.rs` module with one `InteractionHandler` fn per
`PluginHandled` interaction (ADR-0028), plus a `*_plugin_surfaces()` fn in `plugin.rs` that builds the
`Vec<PluginSurfaceRegistration>` pairing each `surfaces::InteractionDescriptor` with its handler via
`RegisteredInteraction::new(descriptor, InteractionDelivery::PluginHandled(handler))`. Create
`crates/plugins/notifications/slack/src/surfaces.rs`:

```rust
pub(crate) fn slack_list_handler<'a>(
    ctx: &'a SurfaceActionContext<'a>,
    params: serde_json::Value,
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<serde_json::Value, SurfaceActionError>> + Send + 'a>,
> {
    // Use the shared list_channels helper from notification-plugin-core
    Box::pin(async move { list_channels(ctx, "slack", &params).await })
}
```

Then, in `plugin.rs`, wire it into the registration:

```rust
RegisteredInteraction::new(
    surfaces::InteractionDescriptor::new(
        surfaces::InteractionId::new("list").expect("literal interaction id is valid"),
        surfaces::InteractionKind::DataLoad,
        "List",
        surfaces::InteractionTransport::ControllerLocal,
    ),
    InteractionDelivery::PluginHandled(crate::surfaces::slack_list_handler),
)
```

`RegisteredInteraction::new` derives `descriptor.transport` from the delivery -- never author the
transport field directly. The shared `list_channels` helper (in `uptrakit-notification-plugin-core`,
behind the `extensions` feature) provides pagination and config flattening that all notification
plugins share. See [Shared list_channels helper](#shared-list_channels-helper) for details.

The `declare_plugin!` macro's `surfaces: { registrations }` arm wires the `*_plugin_surfaces`
fn into the descriptor; `PluginCatalog` derives the exact-id `(surface_id, interaction_id)` dispatch map
from it at build time. Each handler receives a `descriptor::SurfaceActionContext` exposing the typed
controller boundary via its `controller` field -- use its accessors (`ctx.tenant_id()`,
`ctx.caller_user_id()`, `ctx.tenant_db()`); there is no `dyn Any` database escape hatch to downcast.

### 6. Propagate the feature flag

In `crates/ui/web-api/Cargo.toml`:

```toml
notifications-slack = ["uptrakit-plugin-infrastructure-registry/slack"]
```

In `crates/core/controller/Cargo.toml`:

```toml
notifications-slack = ["uptrakit-web-api/notifications-slack"]
```

### 7. Global shared settings (if applicable)

If the new plugin uses global shared settings (like email uses global SMTP settings), the plugin
handles settings extraction internally from the `settings` bag passed to `deliver()`. The
dispatcher builds the settings bag generically from the database (tenant and global settings by
prefix) and passes it to all plugins -- no channel-type-specific logic in the dispatcher.

### 8. Add tests

- Unit tests for `PluginConfig` methods: `validate()`, `with_secrets_masked()`, `restore_secrets_from()` (sync tests, no `start_paused`).
- Descriptor tests verifying `DESCRIPTOR.type_id`, `DESCRIPTOR.family`, `DESCRIPTOR.config_model`, and `DESCRIPTOR.config.*` function pointers.
- Delivery tests using `httpmock` for HTTP assertions.
- Serde round-trip tests for config structs.

## `NotificationEvent` and dispatcher

### `NotificationEvent` (internal type)

Defined in `crates/ui/web-api/src/notifications/events.rs`:

```rust
pub struct NotificationEvent {
    pub tenant_id: Uuid,
    pub host_id: Option<Uuid>,
    pub host_name: Option<String>,
    pub software_item_id: Option<Uuid>,
    pub software_item_name: Option<String>,
    pub plugin_type: Option<String>,
    pub details: NotificationEventDetails,
}
```

The `details` enum is the single source of truth for both event type and event-specific data. There is no redundant
`event_type` field -- the type is derived via `event.event_type()`, which matches on `details` and returns the
corresponding `NotificationEventType` from `web-api-types`.

### `NotificationEventDetails`

```rust
#[non_exhaustive]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NotificationEventDetails {
    UpdateAvailable { installed_version: Option<String>, latest_version: String },
    UpdateCompleted { from_version: Option<String>, to_version: String, update_history_id: Uuid },
    UpdateFailed { from_version: Option<String>, to_version: String, error: Option<String>, update_history_id: Uuid },
    NewSoftwareDiscovered { discovered_count: u32 },
    NewServiceEnrolled { service_id: Uuid, service_label: String },
    CaRotated { reason: String },
}
```

Only `UpdateAvailable` is actionable (produces `MessageAction` buttons via `event.action_params()`). Action parameters
require both `host_id` and `software_item_id` to be present.

`NotificationEventDetails` also carries `BatchUpdateCompleted`, `BatchUpdatePartiallyCompleted`, and
`StdinAttention` variants (added after the table above was last updated -- see
`crates/plugins/notifications/delivery/src/event.rs` for the current field list). The `stdin_attention`
event fires when an interactive update appears to be waiting for stdin input. The full set of wire-level
event-type strings lives in the `NotificationEventType` enum
(`crates/shared/web-api-types/src/notifications/event_types.rs`); do not hardcode the variant count here --
check that file for the authoritative list.

### Dispatcher flow

The dispatcher (`crates/ui/web-api/src/notifications/dispatcher.rs`) runs a fire-and-forget
background loop. It is fully generic -- there are no channel-type-specific code blocks:

1. Event received via `mpsc::UnboundedSender<NotificationEvent>`.
2. Load matching rules by `(tenant_id, event_type, enabled=true)`.
3. Filter by scope: if a rule specifies `host_id`, `software_item_id`, or `plugin_type`, the event must match.
4. For each matched rule:
   - Load the channel from DB and verify it is enabled.
   - Look up the transport via `catalog.transport(&PluginTypeId::new(channel_type))`.
   - Parse and decrypt the channel config (`EncryptedString`).
   - Build a generic settings bag from the database: `{"tenant": {...}, "global": {...}}`
     using `load_settings_by_prefix` and `load_global_settings_by_prefix`.
   - Generate `action_token` (UUIDv7) if the event is actionable.
   - Build `DeliveryMessage` via `message_builder::build_delivery_message()`.
   - Insert a `notification_log` row with `status = "pending"`.
   - Spawn a `tokio::spawn` delivery task.
5. The delivery task calls `transport.deliver(config, settings, message)` and updates the log
   to `"delivered"` or `"failed"`. Each plugin extracts what it needs from the settings bag
   internally (e.g. the email plugin performs SMTP merge from global/tenant settings).

Delivery failures are logged at `warn` level but never propagate back to event producers.

### `message_builder`

`crates/ui/web-api/src/notifications/message_builder.rs` is the single translation point between `NotificationEvent`
and `DeliveryMessage`. Plugin implementations never see `NotificationEvent`.

The builder generates:

- `title` -- one-line summary (e.g. "Update Available: nginx")
- `body` -- multi-line plain text
- `body_html` -- HTML-formatted version for rich-text channels (user-controlled values are
  HTML-escaped via `uptrakit_notification_plugin_core::escape_html()`)
- `event_payload` -- serialized `NotificationEventDetails` as JSON
- `actions` -- "Install {version}" button for `UpdateAvailable` events only

**HTML escaping requirement**: all user-controlled values (software names, host names, version
strings, error messages, etc.) **must** be escaped with `escape_html()` before interpolation into
`body_html`. The `body` (plain text) and `title` do not need escaping. The shared `escape_html()`
function (`crates/plugins/notifications/core/src/lib.rs`) escapes `& < > " '` and is also used by
the Telegram and email plugin implementations for consistency.

## Emitting events

Event hooks are wired into existing handlers. Each call site constructs a `NotificationEvent` and calls
`state.notification_dispatcher.dispatch(...)`:

| Event                                                    | File                                         | Handler                                                                     |
| -------------------------------------------------------- | -------------------------------------------- | --------------------------------------------------------------------------- |
| `UpdateAvailable`                                        | `routes/service_ws/handler/messages.rs`      | `handle_version_check_results()`                                            |
| `NewSoftwareDiscovered`                                  | `routes/service_ws/handler/messages.rs`      | `handle_discovery_results()`                                                |
| `UpdateCompleted` / `UpdateFailed`                       | `routes/service_ws/handler/updates.rs`       | `handle_update_result()`                                                    |
| `NewServiceEnrolled`                                     | `routes/services.rs`                         | `approve_service()`                                                         |
| `CaRotated`                                              | `routes/settings_ca.rs`                      | `rotate_ca()`                                                               |
| `BatchUpdateCompleted` / `BatchUpdatePartiallyCompleted` | `routes/service_ws/handler/updates/batch.rs` | batch completion handling                                                   |
| `StdinAttention`                                         | `routes/service_ws/handler/updates/stdin.rs` | dispatched when an interactive update appears to be waiting for stdin input |

Example pattern:

```rust
state.notification_dispatcher.dispatch(NotificationEvent {
    tenant_id,
    host_id: Some(host_id),
    host_name: Some(hostname),
    software_item_id: Some(item_id),
    software_item_name: Some(item_name),
    plugin_type: None,
    details: NotificationEventDetails::UpdateAvailable {
        installed_version: Some("1.0".to_string()),
        latest_version: "2.0".to_string(),
    },
});
```

The `dispatch()` call never blocks and never returns an error. If the dispatcher channel is closed, the event is
silently dropped with a `tracing::warn!`.

## Database tables

| Table                   | Purpose                                                                                                   |
| ----------------------- | --------------------------------------------------------------------------------------------------------- |
| `notification_channels` | Channel configs (encrypted via `EncryptedString`), one per tenant+channel                                 |
| `notification_rules`    | Event-to-channel bindings with optional scope filters (`host_id`, `software_item_id`, `plugin_type`)      |
| `notification_log`      | Delivery audit trail: status (`pending`/`delivered`/`failed`), `action_token`, `action_taken`, timestamps |

All three tables implement `TenantScoped`. IDs use UUIDv7 for time-ordered indexing.

### Channel config encryption

Channel configs are stored as `EncryptedString` in the database. The config is serialized to JSON, encrypted, and
stored. When reading, the config is decrypted via `config.expose_secret()` and then parsed. API responses always
return masked configs (secrets replaced with `"***"`) via the descriptor's `config.mask_secrets` function pointer,
which delegates to the typed `PluginConfig::with_secrets_masked()`.

## REST API endpoints

Route handlers enforce the `view_notifications` / `manage_notifications` permission strings below via typed
extractors. The corresponding `Permission` enum variants (`ViewNotifications`, `ManageNotifications`) are defined
in `crates/shared/types/src/permissions.rs` alongside the full set of platform permissions -- see that file for
the authoritative variant list rather than hardcoding a count here. For the roles/bundles that grant these
permissions, see [Authentication and Authorization](../security/auth-and-authorization.md).

### Channels

| Method   | Path                                       | Permission             | Description               |
| -------- | ------------------------------------------ | ---------------------- | ------------------------- |
| `POST`   | `/api/v1/notifications/channels`           | `manage_notifications` | Create channel            |
| `GET`    | `/api/v1/notifications/channels`           | `view_notifications`   | List channels (paginated) |
| `GET`    | `/api/v1/notifications/channels/{id}`      | `view_notifications`   | Get channel by ID         |
| `PUT`    | `/api/v1/notifications/channels/{id}`      | `manage_notifications` | Update channel            |
| `DELETE` | `/api/v1/notifications/channels/{id}`      | `manage_notifications` | Delete channel            |
| `POST`   | `/api/v1/notifications/channels/{id}/test` | `manage_notifications` | Send test notification    |

Channel create/update/test validate the channel type against a live transport
(`plugin_ops.transport(...)`) because they must interpret the submitted config. **Delete intentionally
skips that check** (ADR-0033 D5): deletion is cleanup and must keep working for channels whose plugin
type is no longer compiled into the running binary — otherwise such rows would orphan permanently. The
surface-dispatch route to notification interactions still 404s while the owning plugin is not
effectively enabled; only the direct, permission-gated DELETE endpoint stays reachable, pinned by
`delete_channel_succeeds_for_unknown_channel_type`.

### Rules

| Method   | Path                               | Permission             | Description                        |
| -------- | ---------------------------------- | ---------------------- | ---------------------------------- |
| `POST`   | `/api/v1/notifications/rules`      | `manage_notifications` | Create rule                        |
| `GET`    | `/api/v1/notifications/rules`      | `view_notifications`   | List rules (paginated, filterable) |
| `GET`    | `/api/v1/notifications/rules/{id}` | `view_notifications`   | Get rule by ID                     |
| `PUT`    | `/api/v1/notifications/rules/{id}` | `manage_notifications` | Update rule                        |
| `DELETE` | `/api/v1/notifications/rules/{id}` | `manage_notifications` | Delete rule                        |

### Log and callbacks

| Method | Path                                                         | Permission               | Description                   |
| ------ | ------------------------------------------------------------ | ------------------------ | ----------------------------- |
| `GET`  | `/api/v1/notifications/log`                                  | `view_notifications`     | List delivery log (paginated) |
| `POST` | `/api/v1/notifications/callback/{channel_type}/{channel_id}` | Public (plugin-verified) | Generic notification callback |

The callback endpoint is not authenticated via JWT. It is not a surface interaction either: it resolves
the transport for `channel_type` via `plugin_ops.transport(&channel_type_id)` and calls
`NotificationTransport::handle_callback` directly (ADR-0028 / spec D2a), off the surface dispatch path.
The default trait implementation returns a "callback not supported for channel type '...'" error;
channel types that need a callback override it and perform channel-type-specific verification (e.g. the
Telegram plugin verifies the `X-Telegram-Bot-Api-Secret-Token` header against the channel's
`webhook_secret` config field).

## Webhook plugin details

`crates/plugins/notifications/webhook/src/plugin.rs`

- POSTs a JSON payload to the configured `url`.
- Config struct: `WebhookChannelConfig` with fields `url` (required), `secret` (optional), `headers` (optional map).
- When `secret` is present, the request body is signed with HMAC-SHA256 and the signature is included as
  `X-Uptrakit-Signature: sha256=<hex>`.
- Custom headers from `config.headers` are added to the request.
- `PluginConfig::validate()` requires `url` to start with `http://` or `https://`, validates custom headers
  against a blocklist of security-sensitive names (`authorization`, `cookie`, `host`, etc.), and requires
  `headers` to be an object if present. SSRF host validation is enforced at the HTTP client level via
  `SsrfSafeResolver` (unless `allow_private_urls` is set in `CatalogConfig`).
- `PluginConfig::with_secrets_masked()` replaces the `secret` field with `"***"`.
- `PluginConfig::restore_secrets_from()` restores `secret` when the incoming value is `"***"`.

## Email plugin details

`crates/plugins/notifications/email/src/plugin.rs`

The email plugin sends notifications via SMTP using the [mail-send](https://crates.io/crates/mail-send) 0.5 library with
async Tokio support and [mail-builder](https://crates.io/crates/mail-builder) for message construction.
It is gated on the `email` feature flag.

### Config split

The email plugin uses a **three-layer config** model:

- **Per-channel config** (stored encrypted in `notification_channels.config`): contains only `to_addresses`.
  Typed as `EmailChannelConfig` implementing `PluginConfig`.
- **Global SMTP defaults** (stored in the `global_settings` table): server-wide SMTP server host, port,
  credentials, sender identity, and TLS mode. Managed via the "SMTP Defaults" shared surface on the
  Global Settings page.
- **Per-tenant SMTP overrides** (stored in the `settings` table, keyed by `tenant_id`): per-tenant SMTP
  settings that override global defaults on a field-by-field basis.

The dispatcher merges all three layers before calling `deliver()`:

1. Start with global SMTP defaults (`settings.global_smtp()`).
2. Overlay per-tenant SMTP settings (`settings.smtp()`): non-empty tenant fields replace global defaults.
3. Merge the resulting SMTP config into the per-channel config object.

Per-channel config contains no SMTP credentials, which means multiple email channels can share the same
SMTP server without duplicating secrets.

### Per-channel config fields

```json
{
  "to_addresses": ["alice@example.com", "bob@example.com"]
}
```

- `to_addresses`: non-empty list of recipient email addresses (validated at channel create/update time).

### Global SMTP defaults

Server-wide SMTP defaults are stored in the `global_settings` table and managed via the
"SMTP Defaults" shared surface on the Global Settings page. See
[Settings Runtime Architecture](../api/settings-runtime.md) for the full key reference.

| Setting key   | DB key                     | Description                                                                                 |
| ------------- | -------------------------- | ------------------------------------------------------------------------------------------- |
| SMTP host     | `global_smtp.host`         | SMTP server hostname                                                                        |
| SMTP port     | `global_smtp.port`         | SMTP server port (default: 587)                                                             |
| SMTP username | `global_smtp.username`     | Auth username (optional)                                                                    |
| SMTP password | `global_smtp.password`     | Auth password (stored encrypted, optional)                                                  |
| From address  | `global_smtp.from_address` | Sender email address                                                                        |
| From name     | `global_smtp.from_name`    | Sender display name (optional)                                                              |
| TLS mode      | `global_smtp.tls_mode`     | `"starttls"` (default), `"tls"`, or `"none"`                                                |
| EHLO hostname | `global_smtp.helo_host`    | Hostname sent in the SMTP EHLO command (optional; defaults to the domain of `from_address`) |

### Per-tenant SMTP overrides

Per-tenant SMTP settings override the global defaults on a field-by-field basis. Empty fields
inherit from global defaults. Configured via the `notifications.email` surface's `smtp` interaction (PUT).

| Setting key   | DB key              | Description                                        |
| ------------- | ------------------- | -------------------------------------------------- |
| SMTP host     | `smtp.host`         | SMTP server hostname (overrides global)            |
| SMTP port     | `smtp.port`         | SMTP server port (overrides global)                |
| SMTP username | `smtp.username`     | Auth username (overrides global)                   |
| SMTP password | `smtp.password`     | Auth password (stored encrypted, overrides global) |
| From address  | `smtp.from_address` | Sender email address (overrides global)            |
| From name     | `smtp.from_name`    | Sender display name (overrides global)             |
| TLS mode      | `smtp.tls_mode`     | TLS mode (overrides global)                        |

### TLS modes

| Mode       | Description                                                   | Default Port |
| ---------- | ------------------------------------------------------------- | ------------ |
| `starttls` | Opportunistic STARTTLS (upgrades plaintext connection to TLS) | 587          |
| `tls`      | Implicit TLS/SMTPS (TLS from the first byte)                  | 465          |
| `none`     | No TLS (plaintext -- development only)                        | 25           |

`"starttls"` is the default and is appropriate for most modern SMTP providers.

### EHLO hostname derivation

The SMTP `EHLO` command requires a valid fully-qualified domain name (FQDN) per RFC 5321. The
email plugin never falls back to `gethostname()` — Docker container hostnames (e.g. `abc123`)
and short hostnames without a `.` fail strict SMTP servers such as Gmail (error: `555 5.5.2
Syntax error`).

The EHLO hostname is resolved in priority order:

1. Global `helo_host` setting (`global_smtp.helo_host`), if non-empty. Tenants cannot override
   this setting.
2. Domain part of the resolved `from_address` (the substring after `@`).
3. Fallback: `localhost` (should not be reached in practice — `from_address` is required).

`is_configured()` requires `from_address` to be set, so path 2 is always available when email
delivery is enabled. Set `helo_host` via Global Settings when your SMTP relay requires a hostname
different from the `from_address` domain.

### Test email

The "Send Test Email" action (the `test` interaction on the `notifications.email.global-smtp` surface)
sends a test message directly to the **calling user's profile email address** (looked up from the
database). No recipient address input is required.

### Message format

The plugin sends **multipart/alternative** emails:

- **text/plain** part: `message.body`
- **text/html** part: `message.body_html` when provided, otherwise the plain body wrapped in a minimal
  HTML5 document with basic styling.

The email subject is set to `message.title`.

### Settings merge (plugin-internal)

The dispatcher passes a generic settings bag to all plugins -- it has no email-specific
code. The email plugin performs the SMTP merge internally inside its `deliver()` method:

1. Extract global SMTP settings from `settings["global"]` and tenant overrides from
   `settings["tenant"]`.
2. Merge field-by-field: tenant non-empty fields override global defaults.
3. If no SMTP host is configured after merge, return an error.
4. Merge the resulting SMTP config into the per-channel config and proceed with delivery.

The same merge logic is applied when the email plugin handles the `test_channel` surface
action.

### `PluginConfig` for email

- `EmailChannelConfig::validate()`: parses `to_addresses`, rejects empty lists and invalid email
  formats (must contain `@`).
- `EmailChannelConfig::with_secrets_masked()`: no-op; per-channel config contains no secrets.

## Telegram plugin details

`crates/plugins/notifications/telegram/src/plugin.rs`

- Sends messages via the Telegram Bot API `sendMessage` endpoint.
- Config struct: `TelegramChannelConfig` with fields `bot_token` (required), `chat_id` (required), `webhook_secret` (optional, for callback verification).
- Uses HTML parse mode. The title is wrapped in `<b>` tags. HTML special characters in the title are escaped.
- When `DeliveryMessage.actions` is non-empty, buttons are rendered as Telegram inline keyboard buttons
  with `callback_data` set to the action token.
- `TelegramChannelConfig::validate()` requires non-empty `bot_token` and `chat_id`.
- `TelegramChannelConfig::with_secrets_masked()` replaces `bot_token` and `webhook_secret` with `"***"`.

## Testing

- **Unit tests** exist in every module: `events.rs`, `message_builder.rs`, and each plugin crate
  (`webhook`, `telegram`, `email`), `notifications.rs` (web-api-types).
- **Config tests** verify `PluginConfig` methods and descriptor-level `ConfigOps` function pointers
  (`DESCRIPTOR.config.validate`, `DESCRIPTOR.config.mask_secrets`, `DESCRIPTOR.config.restore_secrets`).
- **Descriptor tests** verify `DESCRIPTOR.type_id`, `DESCRIPTOR.family`, `DESCRIPTOR.config_model`,
  role availability (`DESCRIPTOR.roles.notification_transport.is_some()`), and surface ownership.
- **Plugin tests** use standard `#[test]` for sync methods (`validate()`, `with_secrets_masked()`).
  Use `httpmock` for delivery assertions in async tests. Email delivery tests verify error conversion
  against non-routable SMTP hosts (the test waits up to 60 s for connection timeout).
- **Serde round-trip tests** cover all enum variants (`NotificationEventType`,
  `NotificationDeliveryStatus`) and request/response types.
- **Dispatcher testing**: the dispatcher uses fire-and-forget semantics. Test by verifying `notification_log`
  entries in the database after dispatching events.
- **`start_paused = true`** is only needed for tests that call tokio time APIs. Most notification tests do not
  need it.

## Key files

| File                                                      | Purpose                                                                                                                                                                                  |
| --------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/plugins/infrastructure/core/src/roles.rs`         | `PluginMeta` trait, `NotificationTransport` role trait                                                                                                                                   |
| `crates/plugins/infrastructure/core/src/plugin_config.rs` | `PluginConfig` trait (validate, mask, restore, form schema)                                                                                                                              |
| `crates/plugins/infrastructure/core/src/descriptor.rs`    | `PluginDescriptor`, `PluginFamily`, `ConfigModel`, `CatalogConfig`, `CreateTransportFn`, `ConfigOps`                                                                                     |
| `crates/plugins/infrastructure/core/src/macros.rs`        | `declare_plugin!` macro                                                                                                                                                                  |
| `crates/plugins/notifications/core/src/lib.rs`            | `DeliveryMessage`, `MessageAction`, `NotificationPluginError`, `escape_html()`                                                                                                           |
| `crates/plugins/notifications/core/src/list_channels.rs`  | Shared `list_channels` helper (behind `extensions` feature)                                                                                                                              |
| `crates/plugins/infrastructure/registry/src/registry.rs`  | `PluginCatalog` with descriptor-driven registration; `transport()` lookup                                                                                                                |
| `crates/plugins/notifications/webhook/src/plugin.rs`      | Webhook plugin (`declare_plugin!`, `NotificationTransport` impl, HMAC-SHA256 signing)                                                                                                    |
| `crates/plugins/notifications/webhook/src/config.rs`      | `WebhookChannelConfig` implementing `PluginConfig`                                                                                                                                       |
| `crates/plugins/notifications/webhook/src/surfaces.rs`    | Webhook surface action handler                                                                                                                                                           |
| `crates/plugins/notifications/telegram/src/plugin.rs`     | Telegram plugin (`declare_plugin!`, inline keyboard)                                                                                                                                     |
| `crates/plugins/notifications/telegram/src/config.rs`     | `TelegramChannelConfig` implementing `PluginConfig`                                                                                                                                      |
| `crates/plugins/notifications/telegram/src/surfaces.rs`   | Telegram surface action handler (including callback handling)                                                                                                                            |
| `crates/plugins/notifications/email/src/plugin.rs`        | Email plugin (`declare_plugin!`, SMTP via mail-send, multipart/alternative)                                                                                                              |
| `crates/plugins/notifications/email/src/config.rs`        | `EmailChannelConfig` implementing `PluginConfig`                                                                                                                                         |
| `crates/plugins/notifications/email/src/surfaces.rs`      | Email surface action handler (including SMTP settings CRUD)                                                                                                                              |
| `crates/shared/web-api-types/src/notifications/mod.rs`    | Shared request/response types, `Validate` impls                                                                                                                                          |
| `crates/ui/web-api/src/notifications/dispatcher.rs`       | Fire-and-forget generic background dispatcher loop                                                                                                                                       |
| `crates/ui/web-api/src/notifications/events.rs`           | `NotificationEvent`, `NotificationEventDetails`, `ActionParams`                                                                                                                          |
| `crates/ui/web-api/src/notifications/message_builder.rs`  | Event-to-`DeliveryMessage` translation                                                                                                                                                   |
| `crates/ui/web-api-queries/src/queries/notifications.rs`  | DB query helpers, `ChannelQueryError`, `RuleQueryError`                                                                                                                                  |
| `crates/ui/web-api/src/routes/notifications.rs`           | REST route handlers, generic notification callback                                                                                                                                       |
| `crates/shared/db/src/raw_settings.rs`                    | Raw-key settings store functions (`upsert_setting_raw`, `upsert_global_setting_raw`, `load_settings_by_prefix`, `load_global_settings_by_prefix`); used directly by notification plugins |
| `crates/ui/web-api-auth/src/settings_store.rs`            | Typed settings store using the `SettingKey` enum; delegates raw-key functions to `uptrakit_shared_db::raw_settings` for non-notification settings                                        |
| `crates/shared/openapi-client/src/notifications.rs`       | Typed HTTP client methods for the notifications REST API                                                                                                                                 |
| `crates/ui/cli/src/commands/notifications.rs`             | CLI `notifications` command group                                                                                                                                                        |
| `crates/ui/surface-proxy/src/proxy.rs`                    | Shared-surface interaction dispatch to plugin `handle_surface_action()`                                                                                                                  |

## Shared surface integration

The notification settings UI uses shared surfaces to render per-transport channel management tabs
without transport-specific branching in frontend route code.

### Architecture

Each notification plugin owns its interaction registrations and handlers in a `surfaces.rs` module
within the plugin crate. This keeps all transport-specific knowledge co-located with the plugin
implementation. `declare_plugin!`'s single `surfaces: { registrations }` arm
(ADR-0028) is the source for both: `registrations` is a `fn() -> Vec<PluginSurfaceRegistration>`
where each interaction is a `RegisteredInteraction::new(descriptor, delivery)` pairing a
`surfaces::InteractionDescriptor` with an `InteractionDelivery` -- `PluginHandled(shim)` for
interactions the plugin executes (settings CRUD, channel listing), or `ControllerExecutor` for
interactions executed entirely by controller-side code (channel `create`/`edit`/`test`/`delete`,
allowlisted via `CONTROLLER_LOCAL_EXECUTOR_TABLE`). `PluginCatalog` derives an exact-id
`(surface_id, interaction_id)` dispatch map from every plugin's `registrations()` call at build
time -- not the longest-prefix routing an earlier revision of this doc described.

Each plugin's `surfaces.rs` module exports:

- `handle_surface_action(ctx, surface_id, action_id, params) -> Result<Value, String>` -- exact-id
  action dispatch for `PluginHandled` interactions (settings CRUD, channel listing)

Callback handling (e.g. telegram's Bot API webhook) is not part of this dispatch path -- it is a
`NotificationTransport::handle_callback` trait method invoked directly by the public notification
callback route (see "Plugin surface action handlers" below).

The registrations returned by `surfaces.registrations` are also what the shared runtime loads into
controller `SurfaceRegistry` at startup and renders through `frontend/src/lib/components/surfaces/`
-- registration and dispatch are now the same source, so they cannot drift apart (the historical
`list-all-unmatched` incident ADR-0028 documents).

### Shared `list_channels` helper

The `uptrakit-notification-plugin-core` crate provides a shared `list_channels` module (behind
the `extensions` feature) that all notification plugins use for paginated channel listing with
config flattening. It queries channels by type, decrypts config, masks secrets via the
descriptor's `config.mask_secrets` function pointer, and flattens all top-level config keys
into the row object. The shared surface table definitions reference these flattened keys.

### Surface IDs

Surface IDs follow the convention `notifications.<channel_type>`:

| Surface ID                        | Label             | Sort order | Placement                            |
| --------------------------------- | ----------------- | ---------- | ------------------------------------ |
| `notifications.webhook`           | Webhook Channels  | 500        | Tab (group: "Notification Channels") |
| `notifications.telegram`          | Telegram Channels | 501        | Tab (group: "Notification Channels") |
| `notifications.email`             | Email Channels    | 502        | Tab (group: "Notification Channels") |
| `notifications.email.global-smtp` | SMTP Defaults     | 600        | Below (target: "global-settings")    |

Channel surfaces share the `tab_group` value `"Notification Channels"`, so they render as
sections within a single "Notification Channels" tab on the Settings page rather than as separate
tabs. The global SMTP surface renders below the existing Global Settings content.

### Plugin surface action handlers

Each notification plugin handles its own surface actions. Common patterns:

**Channel listing** (all plugins): delegates to the shared `list_channels` helper.

**Settings management** (email plugin): the email plugin handles SMTP settings CRUD
via the `smtp` surface interaction rather than dedicated REST endpoints:

- `smtp` GET (on the `notifications.email` surface) -- returns current per-tenant SMTP settings
  plus `effective_*` fields showing the resolved value after global/tenant merge, and
  `has_global_defaults: bool`
- `smtp` PUT (on the `notifications.email` surface) -- receives flat params via surface invoke,
  performs patch-semantic updates on per-tenant settings using raw-key settings store functions
  (`upsert_setting_raw`)

**Global SMTP defaults** (via the `notifications.email.global-smtp` surface):

- `smtp` GET -- returns the server-wide SMTP default settings
- `smtp` PUT -- saves global SMTP defaults to the `global_settings` table using
  `upsert_global_setting_raw`

**Callback handling** (telegram plugin): `handle_callback` is a `NotificationTransport` trait method
(ADR-0028 / spec D2a), not a surface interaction — it is invoked off the surface dispatch path by the
public, unauthenticated `/api/v1/notifications/callback/{channel_type}/{channel_id}` route, which resolves
the transport by `channel_type` and calls the trait method directly. Telegram's override verifies the
`X-Telegram-Bot-Api-Secret-Token` header, parses `callback_query.data` as a UUID action token, and updates
the notification log entry.

### Raw-key settings store functions

Plugins use raw-key settings store functions (defined in `crates/shared/db/src/raw_settings.rs`) instead of
`SettingKey` enum variants:

- `upsert_setting_raw(db, tenant_id, key, value)` -- write a tenant setting by string key
- `upsert_global_setting_raw(db, key, value)` -- write a global setting by string key
- `load_settings_by_prefix(db, tenant_id, prefix)` -- load all tenant settings with a prefix
- `load_global_settings_by_prefix(db, prefix)` -- load all global settings with a prefix

This decouples notification plugins from `SettingKey` and allows plugins to define their own
settings key namespaces (e.g. `smtp.*`, `global_smtp.*`, `telegram.*`).

### `FormDef.pre_load_action`

When set on a form, the frontend invokes this action when the form modal opens and uses the
response to populate field values. This avoids separate REST endpoints for read-before-edit
flows. The `smtp` PUT interaction's form sets `FormDef.pre_load_action = "smtp"` (its own GET
variant) so the frontend pre-populates the form with current SMTP values on open.

### `SchemaForm` pre-population

The `SchemaForm` component pre-populates all field types from row data (not just hidden
fields), enabling the edit-channel flow where masked secrets and current values appear in the
form. The `preLoadAction` prop triggers an interaction invoke on form open.

### Built-in components

Notification rules and delivery log are **not** surface-powered -- they are built-in Svelte
components (`NotificationRulesSettings.svelte`, `NotificationLogView.svelte`) with direct REST
API calls, following the same pattern as MQTT and OIDC settings.

## Cross-references

- [Notifications API](../api/notifications.md)
- [Notifications Security](../security/notifications-security.md)
- [Notifications End-User Guide](../end-user/notifications.md)
- [Authentication and Authorization](../security/auth-and-authorization.md) -- permission model, roles, and role
  bundles that gate `view_notifications` / `manage_notifications`
- [User Management API](../api/user-management.md) -- endpoint reference for managing which users hold the
  notification permissions
- [Coding Standards](coding-standards.md)
- [Error Handling](error-handling.md)
- [Testing](testing.md)
