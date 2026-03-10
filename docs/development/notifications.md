# Notification subsystem

Development guide for the plugin-based notification subsystem. This document covers the architecture, crate layout,
dispatcher flow, and how to extend the system with new notification plugins.

## Architecture overview

The notification subsystem follows a strict plugin-agnostic pipeline:

```text
Event producers --> NotificationEvent --> Dispatcher --> match rules --> DeliveryMessage --> NotificationPlugin::deliver()
```

**Event producers** know nothing about notification plugins. They emit a `NotificationEvent` containing contextual data
(tenant, host, software item) and a typed `NotificationEventDetails` variant. The **dispatcher** is the single
translation point: it matches rules, builds a `DeliveryMessage` via `message_builder`, and hands it to the plugin
implementation. **Plugins** receive only `DeliveryMessage` and render it into their native format (JSON POST for
webhooks, HTML message with inline keyboard for Telegram, etc.).

This separation means adding a new notification plugin never requires changes to event-producing code.

## Crate structure

| Crate | Path | Purpose |
| --- | --- | --- |
| `uptrakit-notification-plugin-core` | `crates/plugins/notifications/core/` | `NotificationPlugin` trait, `DeliveryMessage`, `MessageAction`, `NotificationPluginError`, `escape_html()` |
| `uptrakit-notification-plugin-webhook` | `crates/plugins/notifications/webhook/` | Webhook plugin (SSRF validation, header blocklist, HMAC-SHA256 signing) |
| `uptrakit-notification-plugin-telegram` | `crates/plugins/notifications/telegram/` | Telegram plugin (inline keyboard support) |
| `uptrakit-notification-plugin-email` | `crates/plugins/notifications/email/` | Email plugin (SMTP via mail-send, `SmtpSettingsSnapshot`, `merge_smtp_into_config()`) |
| `uptrakit-notification-plugin-registry` | `crates/plugins/notifications/registry/` | `NotificationPluginRegistry`, `NotificationOps` trait, re-exports core types |
| `uptrakit-web-api-types` | `crates/shared/web-api-types/src/notifications.rs` | Shared request/response types, public enums (`NotificationEventType`, `NotificationChannelType`, `NotificationDeliveryStatus`) |
| `uptrakit-web-api` | `crates/ui/web-api/src/notifications/` | Dispatcher, internal event types, `message_builder` |
| `uptrakit-web-api` | `crates/ui/web-api-queries/src/queries/notifications.rs` | DB query helpers (CRUD for channels, rules, log) |
| `uptrakit-web-api` | `crates/ui/web-api/src/routes/notifications.rs` | REST API route handlers + Telegram callback endpoint |

## Feature flags

| Feature | Crate | Default | Description |
| --- | --- | --- | --- |
| `webhook` | `notification-plugin-registry` | yes | Webhook plugin (always available) |
| `telegram` | `notification-plugin-registry` | no | Telegram plugin with inline keyboard |
| `email` | `notification-plugin-registry` | no | Email plugin (SMTP via mail-send, async TLS) |
| `notifications-telegram` | `web-api`, `controller` | no | Propagated feature flag enabling Telegram |
| `notifications-email` | `web-api`, `controller` | no | Propagated feature flag enabling email |
| `notifications-all` | `web-api` | no | Enables all optional notification plugins |
| `notifications-all` | `controller` | **yes** | Enables all optional notification plugins (default since `notifications-all` is in `default`) |

Feature flags are additive and chain through the dependency graph:

```text
controller/Cargo.toml           web-api/Cargo.toml                 notification-plugin-registry/Cargo.toml
  notifications-telegram  --->    notifications-telegram  --->       telegram
  notifications-email     --->    notifications-email     --->       email
```

The `web-api` always depends on `notification-plugin-registry` with `default-features = false, features = ["webhook"]`,
ensuring webhooks are always compiled in.

## `NotificationPlugin` trait

Defined in `crates/plugins/notifications/core/src/traits.rs`:

```rust
#[async_trait]
pub trait NotificationPlugin: Send + Sync {
    fn channel_type(&self) -> &'static str;
    async fn deliver(&self, config: &serde_json::Value, message: &DeliveryMessage) -> Result<()>;
    fn validate_config(&self, config: &serde_json::Value) -> Result<()>;
    #[must_use]
    fn mask_config_secrets(&self, config: &serde_json::Value) -> serde_json::Value;
}
```

The `channel_type()` method returns the string identifier for the plugin (e.g. `"webhook"`, `"telegram"`, `"email"`).

There is no `supports_actions()` method. Each plugin decides independently whether to render `DeliveryMessage.actions`.
Plugins that do not support interactive elements silently ignore the `actions` field.

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
- `HttpRequest` -- underlying HTTP request failed
- `HttpClientBuild` -- `reqwest::Client` could not be constructed
- `Serialization` -- payload serialization failed
- `HmacKey` -- HMAC key construction failed

## Adding a new notification plugin

Follow these steps to add a plugin (for example, `slack`):

### 1. Create the plugin crate

Create a new crate at `crates/plugins/notifications/slack/` with a `Cargo.toml` depending on
`uptrakit-notification-plugin-core`:

```toml
[package]
name = "uptrakit-notification-plugin-slack"

[dependencies]
uptrakit-notification-plugin-core = { path = "../core" }
async-trait = { workspace = true }
reqwest = { workspace = true }
rootcause = { workspace = true }
serde_json = { workspace = true }
```

### 2. Implement `NotificationPlugin`

Create `crates/plugins/notifications/slack/src/lib.rs` implementing `NotificationPlugin`:

```rust
use async_trait::async_trait;
use rootcause::prelude::*;

use uptrakit_notification_plugin_core::{
    DeliveryMessage, NotificationPlugin, NotificationPluginError, Result,
};

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
impl NotificationPlugin for SlackPlugin {
    fn channel_type(&self) -> &'static str {
        "slack"
    }

    async fn deliver(
        &self,
        config: &serde_json::Value,
        message: &DeliveryMessage,
    ) -> Result<()> {
        // Build Slack Block Kit payload from message.title, message.body, etc.
        todo!()
    }

    fn validate_config(&self, config: &serde_json::Value) -> Result<()> {
        // Require "webhook_url" field
        todo!()
    }

    fn mask_config_secrets(&self, config: &serde_json::Value) -> serde_json::Value {
        // Mask "webhook_url" value
        todo!()
    }
}
```

Key requirements:

- HTTP client **must** set `.connect_timeout(10s)` and `.timeout(60s)` (see [coding-standards.md](coding-standards.md)).
- `mask_config_secrets` has `#[must_use]` on the trait and must replace all secret fields with `"***"`.
- Use `report!()` / `bail!()` macros for error creation, never `Report::new()` directly.
- No `unwrap()` in production code.

### 3. Register in the `NotificationPluginRegistry`

Add a feature flag in `crates/plugins/notifications/registry/Cargo.toml` and register the plugin
in `NotificationPluginRegistry::new()`:

```toml
[features]
default = ["webhook"]
webhook = ["uptrakit-notification-plugin-webhook"]
telegram = ["uptrakit-notification-plugin-telegram"]
email = ["uptrakit-notification-plugin-email"]
slack = ["uptrakit-notification-plugin-slack"]       # <-- new
all = ["webhook", "telegram", "email", "slack"]      # <-- update
```

In `crates/plugins/notifications/registry/src/lib.rs`, add inside `NotificationPluginRegistry::new()`:

```rust
#[cfg(feature = "slack")]
{
    plugins.insert(
        "slack".to_string(),
        Arc::new(uptrakit_notification_plugin_slack::SlackPlugin::new()?),
    );
}
```

### 4. Add the `NotificationChannelType` variant

In `crates/shared/web-api-types/src/notifications.rs`, add `Slack` to the `NotificationChannelType` enum and update
`as_str()`, `FromStr`, and `Display` implementations accordingly.

### 5. Propagate the feature flag

In `crates/ui/web-api/Cargo.toml`:

```toml
notifications-slack = ["uptrakit-notification-plugin-registry/slack"]
```

In `crates/core/controller/Cargo.toml`:

```toml
notifications-slack = ["uptrakit-web-api/notifications-slack"]
```

### 6. Global shared settings (if applicable)

If the new plugin uses global shared settings (like email uses global SMTP settings), add a merge
step in the dispatcher and `test_channel` handler following the email channel pattern.

### 7. Add tests

- Unit tests for `validate_config`, `mask_config_secrets` (sync tests, no `start_paused`).
- Delivery tests using `httpmock` for HTTP assertions.
- Serde round-trip tests for the new `NotificationChannelType` variant.

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

### Dispatcher flow

The dispatcher (`crates/ui/web-api/src/notifications/dispatcher.rs`) runs a fire-and-forget background loop:

1. Event received via `mpsc::UnboundedSender<NotificationEvent>`.
2. Load matching rules by `(tenant_id, event_type, enabled=true)`.
3. Filter by scope: if a rule specifies `host_id`, `software_item_id`, or `plugin_type`, the event must match.
4. For each matched rule:
   - Load the channel from DB and verify it is enabled.
   - Look up the plugin implementation from `NotificationPluginRegistry`.
   - Parse and decrypt the channel config (`EncryptedString`).
   - Generate `action_token` (UUIDv7) if the event is actionable.
   - Build `DeliveryMessage` via `message_builder::build_delivery_message()`.
   - Insert a `notification_log` row with `status = "pending"`.
   - Spawn a `tokio::spawn` delivery task.
5. The delivery task calls `plugin.deliver()` and updates the log to `"delivered"` or `"failed"`.

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

| Event | File | Handler |
| --- | --- | --- |
| `UpdateAvailable` | `routes/service_ws/handler/messages.rs` | `handle_version_check_results()` |
| `NewSoftwareDiscovered` | `routes/service_ws/handler/messages.rs` | `handle_discovery_results()` |
| `UpdateCompleted` / `UpdateFailed` | `routes/service_ws/handler/updates.rs` | `handle_update_result()` |
| `NewServiceEnrolled` | `routes/services.rs` | `approve_service()` |
| `CaRotated` | `routes/settings_ca.rs` | `rotate_ca()` |

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

| Table | Purpose |
| --- | --- |
| `notification_channels` | Channel configs (encrypted via `EncryptedString`), one per tenant+channel |
| `notification_rules` | Event-to-channel bindings with optional scope filters (`host_id`, `software_item_id`, `plugin_type`) |
| `notification_log` | Delivery audit trail: status (`pending`/`delivered`/`failed`), `action_token`, `action_taken`, timestamps |

All three tables implement `TenantScoped`. IDs use UUIDv7 for time-ordered indexing.

### Channel config encryption

Channel configs are stored as `EncryptedString` in the database. The config is serialized to JSON, encrypted, and
stored. When reading, the config is decrypted via `config.expose_secret()` and then parsed. API responses always
return masked configs (secrets replaced with `"***"`) via `channel.mask_config_secrets()`.

## REST API endpoints

### Channels

| Method | Path | Permission | Description |
| --- | --- | --- | --- |
| `POST` | `/api/v1/notifications/channels` | `manage_notifications` | Create channel |
| `GET` | `/api/v1/notifications/channels` | `view_notifications` | List channels (paginated) |
| `GET` | `/api/v1/notifications/channels/{id}` | `view_notifications` | Get channel by ID |
| `PUT` | `/api/v1/notifications/channels/{id}` | `manage_notifications` | Update channel |
| `DELETE` | `/api/v1/notifications/channels/{id}` | `manage_notifications` | Delete channel |
| `POST` | `/api/v1/notifications/channels/{id}/test` | `manage_notifications` | Send test notification |

### Rules

| Method | Path | Permission | Description |
| --- | --- | --- | --- |
| `POST` | `/api/v1/notifications/rules` | `manage_notifications` | Create rule |
| `GET` | `/api/v1/notifications/rules` | `view_notifications` | List rules (paginated, filterable) |
| `GET` | `/api/v1/notifications/rules/{id}` | `view_notifications` | Get rule by ID |
| `PUT` | `/api/v1/notifications/rules/{id}` | `manage_notifications` | Update rule |
| `DELETE` | `/api/v1/notifications/rules/{id}` | `manage_notifications` | Delete rule |

### Log and callbacks

| Method | Path | Permission | Description |
| --- | --- | --- | --- |
| `GET` | `/api/v1/notifications/log` | `view_notifications` | List delivery log (paginated) |
| `POST` | `/api/v1/notifications/callback/telegram/{channel_id}` | Public (secret-verified) | Telegram bot callback |

The Telegram callback endpoint is not authenticated via JWT. It verifies the `X-Telegram-Bot-Api-Secret-Token`
header against the channel's `webhook_secret` config field.

## Webhook plugin details

`crates/plugins/notifications/webhook/src/lib.rs`

- POSTs a JSON payload to the configured `url`.
- Config fields: `url` (required), `secret` (optional), `headers` (optional object).
- When `secret` is present, the request body is signed with HMAC-SHA256 and the signature is included as
  `X-Uptrakit-Signature: sha256=<hex>`.
- Custom headers from `config.headers` are added to the request.
- `validate_config` requires `url` to start with `http://` or `https://`, validates the URL host against
  `is_private_host()` (unless `allow_private_urls` is set), validates custom headers against a blocklist of
  security-sensitive names (`authorization`, `cookie`, `host`, etc.), and requires `headers` to be an object
  if present.
- `mask_config_secrets` replaces the `secret` field with `"***"`.

## Email plugin details

`crates/plugins/notifications/email/src/lib.rs`

The email plugin sends notifications via SMTP using the [mail-send](https://crates.io/crates/mail-send) 0.5 library with
async Tokio support and [mail-builder](https://crates.io/crates/mail-builder) for message construction.
It is gated on the `email` feature flag.

### Config split

The email plugin uses a **two-layer config** model:

- **Per-channel config** (stored encrypted in `notification_channels.config`): contains only `to_addresses`.
- **Global SMTP settings** (stored in the `settings` key-value table, per-tenant): SMTP server host, port,
  credentials, sender identity, and TLS mode.

The dispatcher merges these two sources before calling `deliver()`. Per-channel config contains no SMTP
credentials, which means multiple email channels can share the same SMTP server without duplicating secrets.

### Per-channel config fields

```json
{
  "to_addresses": ["alice@example.com", "bob@example.com"]
}
```

- `to_addresses`: non-empty list of recipient email addresses (validated at channel create/update time).

### Global SMTP settings

Configured via `PUT /api/v1/settings/smtp` (see [Settings Runtime Architecture](../api/settings-runtime.md)):

| Setting key | DB key | Description |
| --- | --- | --- |
| SMTP host | `smtp.host` | SMTP server hostname (required for email delivery) |
| SMTP port | `smtp.port` | SMTP server port (default: 587) |
| SMTP username | `smtp.username` | Auth username (optional) |
| SMTP password | `smtp.password` | Auth password (stored encrypted, optional) |
| From address | `smtp.from_address` | Sender email address (required for email delivery) |
| From name | `smtp.from_name` | Sender display name (optional) |
| TLS mode | `smtp.tls_mode` | `"starttls"` (default), `"tls"`, or `"none"` |

### TLS modes

| Mode | Description | Default Port |
| --- | --- | --- |
| `starttls` | Opportunistic STARTTLS (upgrades plaintext connection to TLS) | 587 |
| `tls` | Implicit TLS/SMTPS (TLS from the first byte) | 465 |
| `none` | No TLS (plaintext -- development only) | 25 |

`"starttls"` is the default and is appropriate for most modern SMTP providers.

### Message format

The plugin sends **multipart/alternative** emails:

- **text/plain** part: `message.body`
- **text/html** part: `message.body_html` when provided, otherwise the plain body wrapped in a minimal
  HTML5 document with basic styling.

The email subject is set to `message.title`.

### Dispatcher merge step

The dispatcher (`crates/ui/web-api/src/notifications/dispatcher.rs`) performs the merge before each delivery:

1. Load the per-channel config (decrypted `to_addresses`).
2. Read the live `SmtpSettingsSnapshot` from `settings.smtp()`.
3. If `smtp.is_configured()` returns false (host or from_address missing), the notification is skipped
   with a `tracing::warn!` log and the log entry is marked `"failed"`.
4. Otherwise, call `merge_smtp_into_config()` to add SMTP fields to the config object, then call
   `email_plugin.deliver(&merged_config, &message)`.

The same merge logic is applied in the `test_channel` route handler
(`crates/ui/web-api/src/routes/notifications.rs`) and returns HTTP 400 when SMTP is not configured.

### `validate_config` and `mask_config_secrets`

- `validate_config`: parses as `EmailChannelConfig`, rejects empty `to_addresses` and invalid email
  formats (must contain `@`).
- `mask_config_secrets`: no-op; per-channel config contains no secrets.

## Telegram plugin details

`crates/plugins/notifications/telegram/src/lib.rs`

- Sends messages via the Telegram Bot API `sendMessage` endpoint.
- Config fields: `bot_token` (required), `chat_id` (required), `webhook_secret` (optional, for callback verification).
- Uses HTML parse mode. The title is wrapped in `<b>` tags. HTML special characters in the title are escaped.
- When `DeliveryMessage.actions` is non-empty, buttons are rendered as Telegram inline keyboard buttons
  with `callback_data` set to the action token.
- `validate_config` requires non-empty `bot_token` and `chat_id`.
- `mask_config_secrets` replaces `bot_token` and `webhook_secret` with `"***"`.

## Testing

- **Unit tests** exist in every module: `events.rs`, `message_builder.rs`, and each plugin crate
  (`webhook`, `telegram`, `email`), `registry`, `notifications.rs` (web-api-types).
- **Plugin tests** use standard `#[test]` for sync methods (`validate_config`, `mask_config_secrets`).
  Use `httpmock` for delivery assertions in async tests. Email delivery tests verify error conversion
  against non-routable SMTP hosts (the test waits up to 60 s for connection timeout).
- **Serde round-trip tests** cover all enum variants and request/response types.
- **Dispatcher testing**: the dispatcher uses fire-and-forget semantics. Test by verifying `notification_log`
  entries in the database after dispatching events.
- **`start_paused = true`** is only needed for tests that call tokio time APIs. Most notification tests do not
  need it.

## Key files

| File | Purpose |
| --- | --- |
| `crates/plugins/notifications/core/src/traits.rs` | `NotificationPlugin` trait, `DeliveryMessage`, `MessageAction` |
| `crates/plugins/notifications/core/src/error.rs` | `NotificationPluginError` enum, `Result` type alias |
| `crates/plugins/notifications/registry/src/lib.rs` | `NotificationPluginRegistry` -- compiled-in plugin lookup, `NotificationOps` trait |
| `crates/plugins/notifications/webhook/src/lib.rs` | Webhook plugin (HMAC-SHA256 signing) |
| `crates/plugins/notifications/telegram/src/lib.rs` | Telegram plugin (inline keyboard) |
| `crates/plugins/notifications/email/src/lib.rs` | Email plugin (SMTP via mail-send, multipart/alternative) |
| `crates/shared/web-api-types/src/notifications.rs` | Shared enums, request/response types, `Validate` impls |
| `crates/ui/web-api/src/notifications/dispatcher.rs` | Fire-and-forget background dispatcher loop |
| `crates/ui/web-api/src/notifications/events.rs` | `NotificationEvent`, `NotificationEventDetails`, `ActionParams` |
| `crates/ui/web-api/src/notifications/message_builder.rs` | Event-to-`DeliveryMessage` translation |
| `crates/ui/web-api-queries/src/queries/notifications.rs` | DB query helpers, `ChannelQueryError`, `RuleQueryError` |
| `crates/ui/web-api/src/routes/notifications.rs` | REST route handlers, Telegram callback |
| `crates/ui/web-api/src/routes/notification_extensions.rs` | Generic extension data action handler + SMTP settings |
| `crates/ui/web-api/src/extension_registry.rs` | Extension registry with `Notification` owner variant |
| `crates/plugins/notifications/registry/src/extensions/` | Per-transport extension manifests and action definitions |

## Extension framework integration

The notification settings UI uses the extension framework to render per-transport channel management
tabs without any transport-specific knowledge in the frontend or web API route handlers.

### Architecture

Only `notification-plugin-registry` knows about specific transports. It defines `ExtensionManifest`
and `ActionDef` entries for each enabled plugin through the `extensions/` module:

```text
notification-plugin-registry/src/extensions/
├── mod.rs        # feature-gated sub-modules
├── webhook.rs    # ExtensionManifest + ActionDefs for webhook channels
├── telegram.rs   # ExtensionManifest + ActionDefs for Telegram channels
└── email.rs      # ExtensionManifest + ActionDefs for email channels + SMTP
```

### Extension IDs

Extension IDs follow the convention `notifications.<channel_type>`:

| Extension ID | Label | Sort order |
| --- | --- | --- |
| `notifications.webhook` | Webhook Channels | 500 |
| `notifications.telegram` | Telegram Channels | 501 |
| `notifications.email` | Email Channels | 502 |

### `ExtensionOwner::Notification`

The `ExtensionRegistry` supports a `Notification` owner variant alongside `Plugin` and `Service`.
Notification-owned extensions are stored separately and dispatched to
`notification_extensions::handle()` in `routes/notification_extensions.rs`.

### Generic config flattening

The `list` data action in `notification_extensions.rs` queries channels by type, decrypts config,
masks secrets via `mask_config_secrets()`, and flattens all top-level config keys into the row
object. The extension manifest's `DataTable` column definitions reference these flattened keys.
The handler has zero transport-specific knowledge.

### SMTP settings via extensions

The email extension defines two extra actions for SMTP management:

- `get_smtp` — data-only action returning current SMTP settings as flat JSON
- `save_smtp` — receives flat params via extension invoke, performs patch-semantic updates

The `configure_smtp` action uses `FormDef.pre_load_action = "get_smtp"` so the frontend
pre-populates the form with current SMTP values on open.

### `FormDef.pre_load_action`

A new extension framework field. When set on a form, the frontend invokes this action when
the form modal opens and uses the response to populate field values. This avoids separate
REST endpoints for read-before-edit flows.

### `SchemaForm` pre-population

The `SchemaForm` component now pre-populates all field types from row data (not just hidden
fields), enabling the edit-channel flow where masked secrets and current values appear in the
form. The `preLoadAction` prop triggers an extension action invoke on form open.

### Built-in components

Notification rules and delivery log are **not** extension-powered — they are built-in Svelte
components (`NotificationRulesSettings.svelte`, `NotificationLogView.svelte`) with direct REST
API calls, following the same pattern as MQTT and OIDC settings.

## Cross-references

- [Notifications API](../api/notifications.md)
- [Notifications Security](../security/notifications-security.md)
- [Notifications End-User Guide](../end-user/notifications.md)
- [Coding Standards](coding-standards.md)
- [Error Handling](error-handling.md)
- [Testing](testing.md)
