# Notification subsystem

Development guide for the channel-agnostic notification subsystem. This document covers the architecture, crate layout,
dispatcher flow, and how to extend the system with new channels.

## Architecture overview

The notification subsystem follows a strict channel-agnostic pipeline:

```text
Event producers --> NotificationEvent --> Dispatcher --> match rules --> DeliveryMessage --> Channel::deliver()
```

**Event producers** know nothing about channels. They emit a `NotificationEvent` containing contextual data
(tenant, host, software item) and a typed `NotificationEventDetails` variant. The **dispatcher** is the single
translation point: it matches rules, builds a `DeliveryMessage` via `message_builder`, and hands it to the channel
implementation. **Channels** receive only `DeliveryMessage` and render it into their native format (JSON POST for
webhooks, HTML message with inline keyboard for Telegram, etc.).

This separation means adding a new channel never requires changes to event-producing code.

## Crate structure

| Crate | Path | Purpose |
| --- | --- | --- |
| `uptrakit-notification-channels` | `crates/shared/notification-channels/` | `NotificationChannel` trait, `DeliveryMessage`, webhook + telegram impls, `ChannelRegistry` |
| `uptrakit-web-api-types` | `crates/shared/web-api-types/src/notifications.rs` | Shared request/response types, public enums (`NotificationEventType`, `NotificationChannelType`, `NotificationDeliveryStatus`) |
| `uptrakit-web-api` | `crates/ui/web-api/src/notifications/` | Dispatcher, internal event types, `message_builder` |
| `uptrakit-web-api` | `crates/ui/web-api/src/queries/notifications.rs` | DB query helpers (CRUD for channels, rules, log) |
| `uptrakit-web-api` | `crates/ui/web-api/src/routes/notifications.rs` | REST API route handlers + Telegram callback endpoint |

## Feature flags

| Feature | Crate | Default | Description |
| --- | --- | --- | --- |
| `webhook` | `notification-channels` | yes | Webhook channel (always available) |
| `telegram` | `notification-channels` | no | Telegram channel with inline keyboard |
| `email` | `notification-channels` | no | Email channel (SMTP via lettre, async TLS) |
| `notifications-telegram` | `web-api`, `controller` | no | Propagated feature flag enabling Telegram |
| `notifications-email` | `web-api`, `controller` | no | Propagated feature flag enabling email |
| `notifications-all` | `web-api`, `controller` | no | Enables all optional notification channels |

Feature flags are additive and chain through the dependency graph:

```text
controller/Cargo.toml           web-api/Cargo.toml                 notification-channels/Cargo.toml
  notifications-telegram  --->    notifications-telegram  --->       telegram
  notifications-email     --->    notifications-email     --->       email
```

The `web-api` always depends on `notification-channels` with `default-features = false, features = ["webhook"]`,
ensuring webhooks are always compiled in.

## `NotificationChannel` trait

Defined in `crates/shared/notification-channels/src/channel.rs`:

```rust
#[async_trait]
pub trait NotificationChannel: Send + Sync {
    /// Deliver a pre-built message using the given channel-specific config.
    async fn deliver(
        &self,
        config: &serde_json::Value,
        message: &DeliveryMessage,
    ) -> error::Result<()>;

    /// Validate channel-specific config JSON at create/update time.
    fn validate_config(&self, config: &serde_json::Value) -> error::Result<()>;

    /// Return a copy of the config with secrets replaced by `"***"`.
    #[must_use]
    fn mask_config_secrets(&self, config: &serde_json::Value) -> serde_json::Value;
}
```

There is no `supports_actions()` method. Each channel decides independently whether to render `DeliveryMessage.actions`.
Channels that do not support interactive elements silently ignore the `actions` field.

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

### `ChannelError`

All channel operations return `error::Result<T>` which is `Result<T, Report<ChannelError>>`. The error variants are:

- `InvalidConfig` -- channel-specific config is invalid
- `DeliveryFailed` -- delivery to the external service failed
- `HttpRequest` -- underlying HTTP request failed
- `HttpClientBuild` -- `reqwest::Client` could not be constructed
- `Serialization` -- payload serialization failed
- `HmacKey` -- HMAC key construction failed

## Adding a new channel

Follow these steps to add a channel (for example, `slack`):

### 1. Add feature flag

In `crates/shared/notification-channels/Cargo.toml`:

```toml
[features]
default = ["webhook"]
webhook = []
telegram = []
email = []
slack = []                                          # <-- new
all = ["webhook", "telegram", "email", "slack"]     # <-- update
```

### 2. Create the channel implementation

Create `crates/shared/notification-channels/src/slack.rs` implementing `NotificationChannel`:

```rust
use async_trait::async_trait;
use rootcause::prelude::*;

use crate::channel::{DeliveryMessage, NotificationChannel};
use crate::error::{self, ChannelError};

pub struct SlackChannel {
    http: reqwest::Client,
}

impl SlackChannel {
    pub fn new() -> error::Result<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| report!(ChannelError::HttpClientBuild(e.to_string())))?;
        Ok(Self { http })
    }
}

#[async_trait]
impl NotificationChannel for SlackChannel {
    async fn deliver(
        &self,
        config: &serde_json::Value,
        message: &DeliveryMessage,
    ) -> error::Result<()> {
        // Build Slack Block Kit payload from message.title, message.body, etc.
        todo!()
    }

    fn validate_config(&self, config: &serde_json::Value) -> error::Result<()> {
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

### 3. Register in the `ChannelRegistry`

In `crates/shared/notification-channels/src/registry.rs`, add inside `ChannelRegistry::new()`:

```rust
#[cfg(feature = "slack")]
{
    channels.insert(
        "slack".to_string(),
        Arc::new(crate::slack::SlackChannel::new()?),
    );
}
```

### 4. Export the module

In `crates/shared/notification-channels/src/lib.rs`:

```rust
#[cfg(feature = "slack")]
mod slack;

#[cfg(feature = "slack")]
pub use slack::SlackChannel;
```

### 5. Add the `NotificationChannelType` variant

In `crates/shared/web-api-types/src/notifications.rs`, add `Slack` to the `NotificationChannelType` enum and update
`as_str()`, `FromStr`, and `Display` implementations accordingly.

### 6. Propagate the feature flag

In `crates/ui/web-api/Cargo.toml`:

```toml
notifications-slack = ["uptrakit-notification-channels/slack"]
```

In `crates/core/controller/Cargo.toml`:

```toml
notifications-slack = ["uptrakit-web-api/notifications-slack"]
```

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
   - Look up the channel implementation from `ChannelRegistry`.
   - Parse and decrypt the channel config (`EncryptedString`).
   - Generate `action_token` (UUIDv7) if the event is actionable.
   - Build `DeliveryMessage` via `message_builder::build_delivery_message()`.
   - Insert a `notification_log` row with `status = "pending"`.
   - Spawn a `tokio::spawn` delivery task.
5. The delivery task calls `channel.deliver()` and updates the log to `"delivered"` or `"failed"`.

Delivery failures are logged at `warn` level but never propagate back to event producers.

### `message_builder`

`crates/ui/web-api/src/notifications/message_builder.rs` is the single translation point between `NotificationEvent`
and `DeliveryMessage`. Channel implementations never see `NotificationEvent`.

The builder generates:

- `title` -- one-line summary (e.g. "Update Available: nginx")
- `body` -- multi-line plain text
- `body_html` -- HTML-formatted version for rich-text channels
- `event_payload` -- serialized `NotificationEventDetails` as JSON
- `actions` -- "Install {version}" button for `UpdateAvailable` events only

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

## Webhook channel details

`crates/shared/notification-channels/src/webhook.rs`

- POSTs a JSON payload to the configured `url`.
- Config fields: `url` (required), `secret` (optional), `headers` (optional object).
- When `secret` is present, the request body is signed with HMAC-SHA256 and the signature is included as
  `X-Uptrakit-Signature: sha256=<hex>`.
- Custom headers from `config.headers` are added to the request.
- `validate_config` requires `url` to start with `http://` or `https://` and `headers` to be an object if present.
- `mask_config_secrets` replaces the `secret` field with `"***"`.

## Email channel details

`crates/shared/notification-channels/src/email.rs`

The email channel sends notifications via SMTP using the [lettre](https://lettre.rs/) 0.11 library with
async Tokio support. It is gated on the `email` feature flag.

### Config split

The email channel uses a **two-layer config** model:

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

The channel sends **multipart/alternative** emails:

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
   `email_channel.deliver(&merged_config, &message)`.

The same merge logic is applied in the `test_channel` route handler
(`crates/ui/web-api/src/routes/notifications.rs`) and returns HTTP 400 when SMTP is not configured.

### `validate_config` and `mask_config_secrets`

- `validate_config`: parses as `EmailChannelConfig`, rejects empty `to_addresses` and invalid email
  formats (must contain `@`).
- `mask_config_secrets`: no-op; per-channel config contains no secrets.

## Telegram channel details

`crates/shared/notification-channels/src/telegram.rs`

- Sends messages via the Telegram Bot API `sendMessage` endpoint.
- Config fields: `bot_token` (required), `chat_id` (required), `webhook_secret` (optional, for callback verification).
- Uses HTML parse mode. The title is wrapped in `<b>` tags. HTML special characters in the title are escaped.
- When `DeliveryMessage.actions` is non-empty, buttons are rendered as Telegram inline keyboard buttons with
  `callback_data` set to the action token.
- `validate_config` requires non-empty `bot_token` and `chat_id`.
- `mask_config_secrets` replaces `bot_token` and `webhook_secret` with `"***"`.

## Testing

- **Unit tests** exist in every module: `events.rs`, `message_builder.rs`, `webhook.rs`, `telegram.rs`,
  `email.rs`, `registry.rs`, `notifications.rs` (web-api-types).
- **Channel tests** use standard `#[test]` for sync methods (`validate_config`, `mask_config_secrets`).
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
| `crates/shared/notification-channels/src/channel.rs` | `NotificationChannel` trait, `DeliveryMessage`, `MessageAction` |
| `crates/shared/notification-channels/src/error.rs` | `ChannelError` enum, `Result` type alias |
| `crates/shared/notification-channels/src/registry.rs` | `ChannelRegistry` -- compiled-in channel lookup |
| `crates/shared/notification-channels/src/webhook.rs` | Webhook channel (HMAC-SHA256 signing) |
| `crates/shared/notification-channels/src/telegram.rs` | Telegram channel (inline keyboard) |
| `crates/shared/notification-channels/src/email.rs` | Email channel (SMTP via lettre, multipart/alternative) |
| `crates/shared/web-api-types/src/notifications.rs` | Shared enums, request/response types, `Validate` impls |
| `crates/ui/web-api/src/notifications/dispatcher.rs` | Fire-and-forget background dispatcher loop |
| `crates/ui/web-api/src/notifications/events.rs` | `NotificationEvent`, `NotificationEventDetails`, `ActionParams` |
| `crates/ui/web-api/src/notifications/message_builder.rs` | Event-to-`DeliveryMessage` translation |
| `crates/ui/web-api/src/queries/notifications.rs` | DB query helpers, `ChannelQueryError`, `RuleQueryError` |
| `crates/ui/web-api/src/routes/notifications.rs` | REST route handlers, Telegram callback |

## Cross-references

- [Notifications API](../api/notifications.md)
- [Notifications Security](../security/notifications-security.md)
- [Notifications End-User Guide](../end-user/notifications.md)
- [Coding Standards](coding-standards.md)
- [Error Handling](error-handling.md)
- [Testing](testing.md)
