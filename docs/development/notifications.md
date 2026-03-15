# Notification subsystem

Development guide for the plugin-based notification subsystem. This document covers the architecture, crate layout,
dispatcher flow, and how to extend the system with new notification plugins.

## Architecture overview

The notification subsystem follows a strict plugin-agnostic pipeline:

```text
Event producers --> NotificationEvent --> Dispatcher --> match rules --> DeliveryMessage --> NotificationTransportPlugin::deliver()
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
| `uptrakit-notification-plugin-core` | `crates/plugins/notifications/core/` | `DeliveryMessage`, `MessageAction`, `NotificationPluginError`, `escape_html()` |
| `uptrakit-notification-plugin-webhook` | `crates/plugins/notifications/webhook/` | Webhook plugin (SSRF validation, header blocklist, HMAC-SHA256 signing); implements `PluginBase` + `NotificationTransportPlugin` |
| `uptrakit-notification-plugin-telegram` | `crates/plugins/notifications/telegram/` | Telegram plugin (inline keyboard support); implements `PluginBase` + `NotificationTransportPlugin` |
| `uptrakit-notification-plugin-email` | `crates/plugins/notifications/email/` | Email plugin (SMTP via mail-send, `SmtpSettingsSnapshot`, `merge_smtp_into_config()`); implements `PluginBase` + `NotificationTransportPlugin` |
| `uptrakit-plugin-infrastructure-registry` | `crates/plugins/infrastructure/registry/` | Unified `PluginRegistry` stores notification plugins via `with_notifications(config)`; `NotificationRegistryConfig`; consumers use `PluginOps::notification_transport()` |
| `uptrakit-web-api-types` | `crates/shared/web-api-types/src/notifications.rs` | Shared request/response types, public enums (`NotificationEventType`, `NotificationDeliveryStatus`); `channel_type` is `String` (not an enum) |
| `uptrakit-web-api` | `crates/ui/web-api/src/notifications/` | Dispatcher, internal event types, `message_builder` |
| `uptrakit-web-api` | `crates/ui/web-api-queries/src/queries/notifications.rs` | DB query helpers (CRUD for channels, rules, log) |
| `uptrakit-web-api` | `crates/ui/web-api/src/routes/notifications.rs` | REST API route handlers + generic notification callback endpoint |

## Feature flags

| Feature | Crate | Default | Description |
| --- | --- | --- | --- |
| `webhook` | `plugin-infrastructure-registry` | yes | Webhook plugin (always available) |
| `telegram` | `plugin-infrastructure-registry` | no | Telegram plugin with inline keyboard |
| `email` | `plugin-infrastructure-registry` | no | Email plugin (SMTP via mail-send, async TLS) |
| `notifications-telegram` | `web-api`, `controller` | no | Propagated feature flag enabling Telegram |
| `notifications-email` | `web-api`, `controller` | no | Propagated feature flag enabling email |
| `notifications-all` | `web-api` | no | Enables all optional notification plugins |
| `notifications-all` | `controller` | **yes** | Enables all optional notification plugins (default since `notifications-all` is in `default`) |

Feature flags are additive and chain through the dependency graph:

```text
controller/Cargo.toml           web-api/Cargo.toml                 plugin-infrastructure-registry/Cargo.toml
  notifications-telegram  --->    notifications-telegram  --->       telegram
  notifications-email     --->    notifications-email     --->       email
```

The `web-api` always depends on `plugin-infrastructure-registry` with the `webhook` feature enabled,
ensuring webhooks are always compiled in.

## `PluginBase` + `NotificationTransportPlugin` traits

Notification plugins implement two traits from `uptrakit-plugin-infrastructure-core`:

**`PluginBase`** (defined in `crates/plugins/infrastructure/core/src/plugin_base.rs`):

```rust
pub trait PluginBase: Send + Sync {
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> Vec<PluginCapability>;
    fn as_notification_transport(&self) -> Option<&dyn NotificationTransportPlugin> {
        None
    }
    // ... other optional downcasting methods
}
```

**`NotificationTransportPlugin`** (defined in `crates/plugins/infrastructure/core/src/plugin_base.rs`):

```rust
#[async_trait]
pub trait NotificationTransportPlugin: PluginBase {
    fn channel_type(&self) -> &'static str;
    async fn deliver(
        &self,
        config: &serde_json::Value,
        settings: &serde_json::Value,
        message: &DeliveryMessage,
    ) -> Result<()>;
    fn validate_config(&self, config: &serde_json::Value) -> Result<()>;
    #[must_use]
    fn mask_config_secrets(&self, config: &serde_json::Value) -> serde_json::Value;
}
```

The `settings` parameter is a generic JSON bag with the structure
`{"tenant": {"key": value, ...}, "global": {"key": value, ...}}`. Each plugin extracts
what it needs internally (e.g. the email plugin performs SMTP merge, the telegram plugin
extracts `bot_token` from tenant settings, the webhook plugin ignores it).

Each notification plugin implements both traits and overrides `as_notification_transport()` to return `Some(self)`.
The `channel_type()` method on `NotificationTransportPlugin` returns the channel type string identifier
(e.g. `"webhook"`, `"telegram"`, `"email"`).

Consumers look up notification plugins via `PluginOps::notification_transport(channel_type)`, which calls
`as_notification_transport()` on the matching plugin.

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
`uptrakit-notification-plugin-core` and `uptrakit-plugin-infrastructure-core`:

```toml
[package]
name = "uptrakit-notification-plugin-slack"

[dependencies]
uptrakit-notification-plugin-core = { workspace = true }
uptrakit-plugin-infrastructure-core = { workspace = true }
async-trait = { workspace = true }
reqwest = { workspace = true }
rootcause = { workspace = true }
serde_json = { workspace = true }
```

### 2. Implement `PluginBase` + `NotificationTransportPlugin`

Create `crates/plugins/notifications/slack/src/lib.rs` implementing both traits:

```rust
use async_trait::async_trait;
use rootcause::prelude::*;

use uptrakit_notification_plugin_core::{
    DeliveryMessage, NotificationPluginError, Result,
};
use uptrakit_plugin_infrastructure_core::{
    NotificationTransportPlugin, PluginBase, PluginCapability,
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

impl PluginBase for SlackPlugin {
    fn name(&self) -> &'static str {
        "slack"
    }

    fn capabilities(&self) -> Vec<PluginCapability> {
        vec![PluginCapability::NotificationDelivery]
    }

    fn as_notification_transport(&self) -> Option<&dyn NotificationTransportPlugin> {
        Some(self)
    }
}

#[async_trait]
impl NotificationTransportPlugin for SlackPlugin {
    fn channel_type(&self) -> &'static str {
        "slack"
    }

    async fn deliver(
        &self,
        config: &serde_json::Value,
        settings: &serde_json::Value,
        message: &DeliveryMessage,
    ) -> Result<()> {
        // Build Slack Block Kit payload from message.title, message.body, etc.
        // `settings` contains {"tenant": {...}, "global": {...}} -- extract any
        // settings the plugin needs (e.g. API tokens from tenant settings).
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

### 3. Register in the unified `PluginRegistry`

Add a feature flag in `crates/plugins/infrastructure/registry/Cargo.toml` and register the plugin
in the `with_notifications()` builder:

```toml
[features]
default = ["webhook"]
webhook = ["uptrakit-notification-plugin-webhook"]
telegram = ["uptrakit-notification-plugin-telegram"]
email = ["uptrakit-notification-plugin-email"]
slack = ["uptrakit-notification-plugin-slack"]       # <-- new
all = ["webhook", "telegram", "email", "slack"]      # <-- update
```

In `crates/plugins/infrastructure/registry/src/registry.rs`, add inside `with_notifications()`:

```rust
#[cfg(feature = "slack")]
{
    notification_plugins.insert(
        "slack",
        Arc::new(uptrakit_notification_plugin_slack::SlackPlugin::new()?),
    );
}
```

### 4. Add the extension action handler

Each notification plugin owns its own `extensions.rs` module with a `handle_action()` function
that handles settings CRUD, channel listing, and callback handling. Create
`crates/plugins/notifications/slack/src/extensions.rs`:

```rust
pub async fn handle_action(
    ctx: &ExtensionActionContext<'_>,
    extension_id: &str,
    action_id: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    match (extension_id, action_id) {
        ("notifications.slack", "list") => {
            // Use the shared list_channels helper from notification-plugin-core
            list_channels(ctx, "slack", params).await
        }
        _ => Err(format!("unknown action '{action_id}' for extension '{extension_id}'")),
    }
}
```

The shared `list_channels` helper (in `uptrakit-notification-plugin-core`, behind the `extensions`
feature) provides pagination and config flattening that all notification plugins share. See
[Shared list_channels helper](#shared-list_channels-helper) for details.

### 5. Propagate the feature flag

In `crates/ui/web-api/Cargo.toml`:

```toml
notifications-slack = ["uptrakit-plugin-infrastructure-registry/slack"]
```

In `crates/core/controller/Cargo.toml`:

```toml
notifications-slack = ["uptrakit-web-api/notifications-slack"]
```

### 6. Global shared settings (if applicable)

If the new plugin uses global shared settings (like email uses global SMTP settings), the plugin
handles settings extraction internally from the `settings` bag passed to `deliver()`. The
dispatcher builds the settings bag generically from the database (tenant and global settings by
prefix) and passes it to all plugins -- no channel-type-specific logic in the dispatcher.

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

The dispatcher (`crates/ui/web-api/src/notifications/dispatcher.rs`) runs a fire-and-forget
background loop. It is fully generic -- there are no channel-type-specific code blocks:

1. Event received via `mpsc::UnboundedSender<NotificationEvent>`.
2. Load matching rules by `(tenant_id, event_type, enabled=true)`.
3. Filter by scope: if a rule specifies `host_id`, `software_item_id`, or `plugin_type`, the event must match.
4. For each matched rule:
   - Load the channel from DB and verify it is enabled.
   - Look up the plugin implementation via `PluginOps::notification_transport(channel_type)`.
   - Parse and decrypt the channel config (`EncryptedString`).
   - Build a generic settings bag from the database: `{"tenant": {...}, "global": {...}}`
     using `load_settings_by_prefix` and `load_global_settings_by_prefix`.
   - Generate `action_token` (UUIDv7) if the event is actionable.
   - Build `DeliveryMessage` via `message_builder::build_delivery_message()`.
   - Insert a `notification_log` row with `status = "pending"`.
   - Spawn a `tokio::spawn` delivery task.
5. The delivery task calls `plugin.deliver(config, settings, message)` and updates the log
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
| `POST` | `/api/v1/notifications/callback/{channel_type}/{channel_id}` | Public (plugin-verified) | Generic notification callback |

The callback endpoint is not authenticated via JWT. It dispatches to the plugin's
`handle_callback` extension action, which performs channel-type-specific verification
(e.g. the Telegram plugin verifies the `X-Telegram-Bot-Api-Secret-Token` header against
the channel's `webhook_secret` config field).

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

The email plugin uses a **three-layer config** model:

- **Per-channel config** (stored encrypted in `notification_channels.config`): contains only `to_addresses`.
- **Global SMTP defaults** (stored in the `global_settings` table): server-wide SMTP server host, port,
  credentials, sender identity, and TLS mode. Managed via the "SMTP Defaults" extension panel on the
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
"SMTP Defaults" extension panel on the Global Settings page. See
[Settings Runtime Architecture](../api/settings-runtime.md) for the full key reference.

| Setting key | DB key | Description |
| --- | --- | --- |
| SMTP host | `global_smtp.host` | SMTP server hostname |
| SMTP port | `global_smtp.port` | SMTP server port (default: 587) |
| SMTP username | `global_smtp.username` | Auth username (optional) |
| SMTP password | `global_smtp.password` | Auth password (stored encrypted, optional) |
| From address | `global_smtp.from_address` | Sender email address |
| From name | `global_smtp.from_name` | Sender display name (optional) |
| TLS mode | `global_smtp.tls_mode` | `"starttls"` (default), `"tls"`, or `"none"` |
| EHLO hostname | `global_smtp.helo_host` | Hostname sent in the SMTP EHLO command (optional; defaults to the domain of `from_address`) |

### Per-tenant SMTP overrides

Per-tenant SMTP settings override the global defaults on a field-by-field basis. Empty fields
inherit from global defaults. Configured via the email channel extension's "Configure SMTP" action.

| Setting key | DB key | Description |
| --- | --- | --- |
| SMTP host | `smtp.host` | SMTP server hostname (overrides global) |
| SMTP port | `smtp.port` | SMTP server port (overrides global) |
| SMTP username | `smtp.username` | Auth username (overrides global) |
| SMTP password | `smtp.password` | Auth password (stored encrypted, overrides global) |
| From address | `smtp.from_address` | Sender email address (overrides global) |
| From name | `smtp.from_name` | Sender display name (overrides global) |
| TLS mode | `smtp.tls_mode` | TLS mode (overrides global) |

### TLS modes

| Mode | Description | Default Port |
| --- | --- | --- |
| `starttls` | Opportunistic STARTTLS (upgrades plaintext connection to TLS) | 587 |
| `tls` | Implicit TLS/SMTPS (TLS from the first byte) | 465 |
| `none` | No TLS (plaintext -- development only) | 25 |

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

The "Send Test Email" action (`test_global_smtp_email`) sends a test message directly to the
**calling user's profile email address** (looked up from the database). No recipient address
input is required.

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

The same merge logic is applied when the email plugin handles the `test_channel` extension
action.

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
  (`webhook`, `telegram`, `email`), `notifications.rs` (web-api-types).
- **Plugin tests** use standard `#[test]` for sync methods (`validate_config`, `mask_config_secrets`).
  Use `httpmock` for delivery assertions in async tests. Email delivery tests verify error conversion
  against non-routable SMTP hosts (the test waits up to 60 s for connection timeout).
- **Serde round-trip tests** cover all enum variants (`NotificationEventType`,
  `NotificationDeliveryStatus`) and request/response types.
- **Dispatcher testing**: the dispatcher uses fire-and-forget semantics. Test by verifying `notification_log`
  entries in the database after dispatching events.
- **`start_paused = true`** is only needed for tests that call tokio time APIs. Most notification tests do not
  need it.

## Key files

| File | Purpose |
| --- | --- |
| `crates/plugins/infrastructure/core/src/plugin_base.rs` | `PluginBase` trait, `NotificationTransportPlugin` trait (with `channel_type()`, `deliver(config, settings, message)`) |
| `crates/plugins/notifications/core/src/lib.rs` | `DeliveryMessage`, `MessageAction`, `NotificationPluginError`, `escape_html()` |
| `crates/plugins/notifications/core/src/list_channels.rs` | Shared `list_channels` helper (behind `extensions` feature) |
| `crates/plugins/infrastructure/registry/src/registry.rs` | Unified `PluginRegistry` with `with_notifications()` builder; `notification_transport()` lookup |
| `crates/plugins/notifications/webhook/src/lib.rs` | Webhook plugin (HMAC-SHA256 signing) |
| `crates/plugins/notifications/webhook/src/extensions.rs` | Webhook extension action handler |
| `crates/plugins/notifications/telegram/src/lib.rs` | Telegram plugin (inline keyboard) |
| `crates/plugins/notifications/telegram/src/extensions.rs` | Telegram extension action handler (including callback handling) |
| `crates/plugins/notifications/email/src/lib.rs` | Email plugin (SMTP via mail-send, multipart/alternative) |
| `crates/plugins/notifications/email/src/extensions.rs` | Email extension action handler (including SMTP settings CRUD) |
| `crates/shared/web-api-types/src/notifications.rs` | Shared request/response types, `Validate` impls |
| `crates/ui/web-api/src/notifications/dispatcher.rs` | Fire-and-forget generic background dispatcher loop |
| `crates/ui/web-api/src/notifications/events.rs` | `NotificationEvent`, `NotificationEventDetails`, `ActionParams` |
| `crates/ui/web-api/src/notifications/message_builder.rs` | Event-to-`DeliveryMessage` translation |
| `crates/ui/web-api-queries/src/queries/notifications.rs` | DB query helpers, `ChannelQueryError`, `RuleQueryError` |
| `crates/ui/web-api/src/routes/notifications.rs` | REST route handlers, generic notification callback |
| `crates/ui/web-api-auth/src/settings_store.rs` | Raw-key settings store functions (`upsert_setting_raw`, `load_settings_by_prefix`, etc.) |
| `crates/ui/web-api/src/extension_registry.rs` | Extension registry with `Notification` owner variant |

## Extension framework integration

The notification settings UI uses the extension framework to render per-transport channel management
tabs without any transport-specific knowledge in the frontend or web API route handlers.

### Architecture

Each notification plugin owns its extension definitions **and** action handlers in an
`extensions.rs` module within the plugin crate. This keeps all transport-specific knowledge
co-located with the plugin implementation. The unified `PluginRegistry` delegates to each
registered notification plugin and aggregates the results through
`PluginOps::extension_manifests()` and `PluginOps::handle_extension_action()`.

Each plugin's `extensions.rs` module exports:

- `extension_manifests() -> Vec<ExtensionManifest>` -- UI manifests for channel management
- `extension_actions() -> Vec<(String, Vec<ActionDef>)>` -- action catalogue
- `handle_action(ctx, extension_id, action_id, params) -> Result<Value, String>` -- action
  dispatch including settings CRUD, channel listing, and callback handling

### Shared `list_channels` helper

The `uptrakit-notification-plugin-core` crate provides a shared `list_channels` module (behind
the `extensions` feature) that all notification plugins use for paginated channel listing with
config flattening. It queries channels by type, decrypts config, masks secrets via
`mask_config_secrets()`, and flattens all top-level config keys into the row object. The
extension manifest's `DataTable` column definitions reference these flattened keys.

### Extension IDs

Extension IDs follow the convention `notifications.<channel_type>`:

| Extension ID | Label | Sort order | Placement |
| --- | --- | --- | --- |
| `notifications.webhook` | Webhook Channels | 500 | Tab (group: "Notification Channels") |
| `notifications.telegram` | Telegram Channels | 501 | Tab (group: "Notification Channels") |
| `notifications.email` | Email Channels | 502 | Tab (group: "Notification Channels") |
| `notifications.email.global_smtp` | SMTP Defaults | 600 | Below (target: "global-settings") |

Channel extensions share the `tab_group` value `"Notification Channels"`, so they render as
sections within a single "Notification Channels" tab on the Settings page rather than as separate
tabs. The global SMTP extension renders below the existing Global Settings content.

### Plugin extension action handlers

Each notification plugin handles its own extension actions. Common patterns:

**Channel listing** (all plugins): delegates to the shared `list_channels` helper.

**Settings management** (email plugin): the email plugin handles SMTP settings CRUD
via extension actions rather than dedicated REST endpoints:

- `get_smtp` -- returns current per-tenant SMTP settings plus `effective_*` fields showing the
  resolved value after global/tenant merge, and `has_global_defaults: bool`
- `save_smtp` -- receives flat params via extension invoke, performs patch-semantic updates on
  per-tenant settings using raw-key settings store functions (`upsert_setting_raw`)

**Global SMTP defaults** (via the `notifications.email.global_smtp` extension):

- `get_global_smtp` -- returns the server-wide SMTP default settings
- `save_global_smtp` -- saves global SMTP defaults to the `global_settings` table using
  `upsert_global_setting_raw`

**Callback handling** (telegram plugin): the `handle_callback` action verifies the
`X-Telegram-Bot-Api-Secret-Token` header, parses `callback_query.data` as a UUID action
token, and updates the notification log entry.

### Raw-key settings store functions

Plugins use raw-key settings store functions instead of `SettingKey` enum variants:

- `upsert_setting_raw(db, tenant_id, key, value)` -- write a tenant setting by string key
- `upsert_global_setting_raw(db, key, value)` -- write a global setting by string key
- `load_settings_by_prefix(db, tenant_id, prefix)` -- load all tenant settings with a prefix
- `load_global_settings_by_prefix(db, prefix)` -- load all global settings with a prefix

This decouples notification plugins from `SettingKey` and allows plugins to define their own
settings key namespaces (e.g. `smtp.*`, `global_smtp.*`, `telegram.*`).

### `FormDef.pre_load_action`

When set on a form, the frontend invokes this action when the form modal opens and uses the
response to populate field values. This avoids separate REST endpoints for read-before-edit
flows. The `configure_smtp` action uses `FormDef.pre_load_action = "get_smtp"` so the frontend
pre-populates the form with current SMTP values on open.

### `SchemaForm` pre-population

The `SchemaForm` component pre-populates all field types from row data (not just hidden
fields), enabling the edit-channel flow where masked secrets and current values appear in the
form. The `preLoadAction` prop triggers an extension action invoke on form open.

### Built-in components

Notification rules and delivery log are **not** extension-powered -- they are built-in Svelte
components (`NotificationRulesSettings.svelte`, `NotificationLogView.svelte`) with direct REST
API calls, following the same pattern as MQTT and OIDC settings.

## Cross-references

- [Notifications API](../api/notifications.md)
- [Notifications Security](../security/notifications-security.md)
- [Notifications End-User Guide](../end-user/notifications.md)
- [Coding Standards](coding-standards.md)
- [Error Handling](error-handling.md)
- [Testing](testing.md)
