# Notifications API

The notification subsystem provides event-driven alerts through configurable delivery channels.
Notifications are composed of three entities: **channels** (where to send), **rules** (what to send
and when), and **log entries** (delivery audit trail).

All authenticated endpoints require JWT access tokens with the appropriate permission. See
[Authentication and Authorization](../security/auth-and-authorization.md) for the full permission
model.

## Overview

### Channels

| Method | Path | Permission | Description |
| --- | --- | --- | --- |
| POST | `/api/v1/notifications/channels` | `ManageNotifications` | Create a channel |
| GET | `/api/v1/notifications/channels` | `ViewNotifications` | List channels (paginated) |
| GET | `/api/v1/notifications/channels/{id}` | `ViewNotifications` | Get channel by ID |
| PUT | `/api/v1/notifications/channels/{id}` | `ManageNotifications` | Update channel |
| DELETE | `/api/v1/notifications/channels/{id}` | `ManageNotifications` | Delete channel |
| POST | `/api/v1/notifications/channels/{id}/test` | `ManageNotifications` | Test channel delivery |

### Rules

| Method | Path | Permission | Description |
| --- | --- | --- | --- |
| POST | `/api/v1/notifications/rules` | `ManageNotifications` | Create a rule |
| GET | `/api/v1/notifications/rules` | `ViewNotifications` | List rules (paginated, filterable) |
| GET | `/api/v1/notifications/rules/{id}` | `ViewNotifications` | Get rule by ID |
| PUT | `/api/v1/notifications/rules/{id}` | `ManageNotifications` | Update rule |
| DELETE | `/api/v1/notifications/rules/{id}` | `ManageNotifications` | Delete rule |

### Log

| Method | Path | Permission | Description |
| --- | --- | --- | --- |
| GET | `/api/v1/notifications/log` | `ViewNotifications` | List delivery log (paginated) |

### Public (not JWT-authenticated)

| Method | Path | Auth | Description |
| --- | --- | --- | --- |
| POST | `/api/v1/notifications/callback/telegram/{channel_id}` | `X-Telegram-Bot-Api-Secret-Token` header | Telegram bot callback |

## Channel Endpoints

### `POST /api/v1/notifications/channels`

Create a new notification channel. The channel config is validated against the channel type
implementation before storage. Config secrets are encrypted at rest.

**Request body** (`CreateNotificationChannelRequest`):

```json
{
  "name": "Ops Webhook",
  "channel_type": "webhook",
  "config": {
    "url": "https://example.com/hooks/uptrakit",
    "secret": "my-hmac-secret",
    "headers": {
      "X-Custom": "value"
    }
  },
  "enabled": true
}
```

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `name` | string | Yes | -- | Human-readable label. Must not be empty or whitespace-only. |
| `channel_type` | string | Yes | -- | `"webhook"` or `"telegram"`. |
| `config` | object | Yes | -- | Channel-specific configuration (see [Channel Config](#channel-config)). Must be a JSON object. |
| `enabled` | bool | No | `true` | Whether the channel is active. |

**Response** (`201`): `NotificationChannelResponse`

```json
{
  "id": "019...",
  "name": "Ops Webhook",
  "channel_type": "webhook",
  "config": {
    "url": "https://example.com/hooks/uptrakit",
    "secret": "***",
    "headers": {
      "X-Custom": "value"
    }
  },
  "enabled": true,
  "created_at": "2026-03-01T00:00:00Z",
  "updated_at": "2026-03-01T00:00:00Z"
}
```

Config secrets are masked in the response (see [Secret Masking](#secret-masking)).

**Error responses**:

- `400` -- validation failed (empty name, non-object config).
- `400` -- unsupported channel type.
- `400` -- invalid config (missing required fields for the channel type).

### `GET /api/v1/notifications/channels`

List all notification channels for the tenant, ordered by creation date (newest first).

**Query parameters**: `page` (default 1), `per_page` (default 20, max 1000).

**Response** (`200`): `PaginatedResponse<NotificationChannelResponse>`

```json
{
  "items": [
    {
      "id": "019...",
      "name": "Ops Webhook",
      "channel_type": "webhook",
      "config": {
        "url": "https://example.com/hooks/uptrakit",
        "secret": "***"
      },
      "enabled": true,
      "created_at": "2026-03-01T00:00:00Z",
      "updated_at": "2026-03-01T00:00:00Z"
    }
  ],
  "total": 1,
  "page": 1,
  "per_page": 20,
  "total_pages": 1
}
```

### `GET /api/v1/notifications/channels/{id}`

Get a single notification channel by UUID.

**Path parameters**: `id` -- channel UUID.

**Response** (`200`): `NotificationChannelResponse`

**Error responses**:

- `404` -- channel not found.

### `PUT /api/v1/notifications/channels/{id}`

Update an existing notification channel. All fields are optional; only provided fields are changed.
When `config` is provided, it replaces the entire config object and is re-validated against the
channel type implementation.

**Path parameters**: `id` -- channel UUID.

**Request body** (`UpdateNotificationChannelRequest`):

```json
{
  "name": "Ops Webhook (updated)",
  "config": {
    "url": "https://example.com/hooks/uptrakit-v2",
    "secret": "new-hmac-secret"
  },
  "enabled": false
}
```

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `name` | string | No | Updated label. Must not be empty or whitespace-only when provided. |
| `config` | object | No | Replacement config. Must be a JSON object when provided. |
| `enabled` | bool | No | Toggle channel on/off. |

**Response** (`200`): `NotificationChannelResponse`

**Error responses**:

- `400` -- validation failed (empty name, non-object config, invalid config for the channel type).
- `404` -- channel not found.

### `DELETE /api/v1/notifications/channels/{id}`

Delete a notification channel. Associated rules are **not** automatically deleted.

**Path parameters**: `id` -- channel UUID.

**Response** (`204`): no body.

**Error responses**:

- `404` -- channel not found.

### `POST /api/v1/notifications/channels/{id}/test`

Send a test notification through the channel. The test message is delivered using the channel's
stored (decrypted) config. The response always returns HTTP 200 -- check the `success` field
to determine whether delivery succeeded.

**Path parameters**: `id` -- channel UUID.

**Response** (`200`): `TestNotificationResponse`

```json
{
  "success": true,
  "message": "Test notification delivered successfully"
}
```

On delivery failure:

```json
{
  "success": false,
  "message": "webhook returned 502: Bad Gateway"
}
```

**Error responses**:

- `400` -- unsupported channel type.
- `404` -- channel not found.

## Rule Endpoints

### `POST /api/v1/notifications/rules`

Create a notification rule that links an event type to a channel. Optional scope filters
(`host_id`, `software_item_id`, `plugin_type`) narrow which events trigger the rule.

**Request body** (`CreateNotificationRuleRequest`):

```json
{
  "channel_id": "019...",
  "event_type": "update_available",
  "host_id": null,
  "software_item_id": "019...",
  "plugin_type": null,
  "enabled": true
}
```

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `channel_id` | UUID | Yes | -- | Target channel. Must exist in the same tenant. |
| `event_type` | string | Yes | -- | One of the [event types](#event-types). |
| `host_id` | UUID | No | `null` | Scope to a specific host. `null` matches all hosts. |
| `software_item_id` | UUID | No | `null` | Scope to a specific software item. `null` matches all items. |
| `plugin_type` | string | No | `null` | Scope to a specific plugin type (e.g. `"releases_github"`). `null` matches all plugin types. |
| `enabled` | bool | No | `true` | Whether the rule is active. |

**Response** (`201`): `NotificationRuleResponse`

```json
{
  "id": "019...",
  "channel_id": "019...",
  "event_type": "update_available",
  "host_id": null,
  "software_item_id": "019...",
  "plugin_type": null,
  "enabled": true,
  "created_at": "2026-03-01T00:00:00Z"
}
```

**Error responses**:

- `400` -- validation failed.
- `404` -- channel not found (the referenced `channel_id` does not exist in the tenant).

### `GET /api/v1/notifications/rules`

List notification rules for the tenant, ordered by creation date (newest first). Supports
filtering by channel and event type.

**Query parameters**:

| Parameter | Type | Default | Description |
| --- | --- | --- | --- |
| `channel_id` | UUID | -- | Filter rules by channel ID |
| `event_type` | string | -- | Filter rules by event type (e.g. `update_available`) |
| `page` | u64 | 1 | Page number (1-indexed) |
| `per_page` | u64 | 20 | Items per page (clamped to 1--1000) |

**Response** (`200`): `PaginatedResponse<NotificationRuleResponse>`

```json
{
  "items": [
    {
      "id": "019...",
      "channel_id": "019...",
      "event_type": "update_available",
      "host_id": null,
      "software_item_id": null,
      "plugin_type": null,
      "enabled": true,
      "created_at": "2026-03-01T00:00:00Z"
    }
  ],
  "total": 1,
  "page": 1,
  "per_page": 20,
  "total_pages": 1
}
```

### `GET /api/v1/notifications/rules/{id}`

Get a single notification rule by UUID.

**Path parameters**: `id` -- rule UUID.

**Response** (`200`): `NotificationRuleResponse`

**Error responses**:

- `404` -- rule not found.

### `PUT /api/v1/notifications/rules/{id}`

Update a notification rule. All fields are optional; only provided fields are changed.

**Path parameters**: `id` -- rule UUID.

**Request body** (`UpdateNotificationRuleRequest`):

```json
{
  "event_type": "update_failed",
  "host_id": "019...",
  "enabled": false
}
```

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `event_type` | string | No | Updated event type. |
| `host_id` | UUID | No | Updated host scope. |
| `software_item_id` | UUID | No | Updated software item scope. |
| `plugin_type` | string | No | Updated plugin type scope. |
| `enabled` | bool | No | Toggle rule on/off. |

**Response** (`200`): `NotificationRuleResponse`

**Error responses**:

- `400` -- validation failed.
- `404` -- rule not found.

### `DELETE /api/v1/notifications/rules/{id}`

Delete a notification rule.

**Path parameters**: `id` -- rule UUID.

**Response** (`204`): no body.

**Error responses**:

- `404` -- rule not found.

## Log Endpoint

### `GET /api/v1/notifications/log`

List notification delivery log entries for the tenant, ordered by creation date (newest first).

**Query parameters**: `page` (default 1), `per_page` (default 20, max 1000).

**Response** (`200`): `PaginatedResponse<NotificationLogResponse>`

```json
{
  "items": [
    {
      "id": "019...",
      "channel_id": "019...",
      "rule_id": "019...",
      "event_type": "update_available",
      "event_payload": {
        "software_item": "nginx",
        "host": "web-01",
        "latest_version": "1.25.4"
      },
      "status": "delivered",
      "error_message": null,
      "action_token": "019...",
      "action_taken": null,
      "created_at": "2026-03-01T12:00:00Z",
      "delivered_at": "2026-03-01T12:00:01Z"
    }
  ],
  "total": 1,
  "page": 1,
  "per_page": 20,
  "total_pages": 1
}
```

## Telegram Callback Endpoint

### `POST /api/v1/notifications/callback/telegram/{channel_id}`

Public endpoint called by Telegram's Bot API when a user presses an inline keyboard button
on a notification message. This endpoint is **not** authenticated via JWT. Instead, it
verifies the `X-Telegram-Bot-Api-Secret-Token` header against the channel's `webhook_secret`
config field.

**Path parameters**: `channel_id` -- the notification channel UUID.

**Headers**:

| Header | Required | Description |
| --- | --- | --- |
| `X-Telegram-Bot-Api-Secret-Token` | Yes | Must match the channel's `webhook_secret` config value |

**Request body**: raw Telegram `Update` JSON (sent by the Telegram Bot API). The handler
extracts `callback_query.data` which must contain a valid UUID action token.

**Behavior**:

1. Loads the channel from the database and decrypts the config.
2. Verifies the `X-Telegram-Bot-Api-Secret-Token` header against `webhook_secret`.
3. Parses the `callback_query.data` field as a UUID action token.
4. Looks up the notification log entry by action token.
5. If the action has not already been taken, sets `action_taken = "triggered"`.
6. Returns `200` with an empty JSON body (`{}`).

**Error responses**:

- `401` -- invalid or missing secret token.
- `400` -- invalid request body.
- `404` -- channel not found.

## Enums

### Event Types

The `NotificationEventType` enum (`#[non_exhaustive]`) defines which system events can trigger
notifications:

| Value | Description |
| --- | --- |
| `update_available` | A new version is available for a software item |
| `update_completed` | An update finished successfully |
| `update_failed` | An update failed |
| `new_software_discovered` | A new software item was discovered by autodiscovery |
| `new_service_enrolled` | A new service (agent, MQTT bridge, SSH agent) enrolled |
| `ca_rotated` | The CA certificate was rotated |

### Channel Types

The `NotificationChannelType` enum (`#[non_exhaustive]`) defines supported delivery
mechanisms:

| Value | Description |
| --- | --- |
| `webhook` | HTTP POST with JSON payload and optional HMAC-SHA256 signature |
| `telegram` | Telegram Bot API `sendMessage` with HTML formatting and inline keyboards |

### Delivery Statuses

The `NotificationDeliveryStatus` enum (`#[non_exhaustive]`) tracks the outcome of each
delivery attempt:

| Value | Description |
| --- | --- |
| `pending` | Queued for delivery, not yet attempted |
| `delivered` | Successfully delivered to the channel |
| `failed` | Delivery failed (see `error_message` for details) |

## Channel Config

### Webhook

The webhook channel POSTs a JSON payload to the configured URL. When a `secret` is configured,
the request body is signed with HMAC-SHA256 and the signature is included in the
`X-Uptrakit-Signature` header as `sha256=<hex>`.

**Config fields**:

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `url` | string | Yes | Destination URL. Must start with `http://` or `https://`. |
| `secret` | string | No | HMAC-SHA256 signing secret for payload verification. |
| `headers` | object | No | Additional HTTP headers to include in the request. Keys are header names, values are header values. |

**Example config**:

```json
{
  "url": "https://example.com/hooks/uptrakit",
  "secret": "my-hmac-secret",
  "headers": {
    "X-Custom": "value"
  }
}
```

**Webhook payload format** (sent as JSON POST body):

```json
{
  "title": "Update Available: nginx",
  "body": "A new version 1.25.4 is available for nginx on web-01.",
  "event": {
    "software_item": "nginx",
    "host": "web-01",
    "latest_version": "1.25.4"
  },
  "actions": [
    {
      "label": "Install Update",
      "callback_url": "https://controller.example.com/api/v1/...",
      "token": "019..."
    }
  ]
}
```

### Telegram

The Telegram channel sends messages via the Bot API `sendMessage` endpoint with HTML parse mode.
Action buttons are rendered as Telegram inline keyboard buttons.

**Config fields**:

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `bot_token` | string | Yes | Telegram Bot API token (e.g. `123456:ABC-DEF`). Must not be empty. |
| `chat_id` | string | Yes | Telegram chat ID (e.g. `"-100123456789"` for a group). Must not be empty. |
| `webhook_secret` | string | No | Secret for verifying Telegram callback requests. Required if using interactive buttons. |

**Example config**:

```json
{
  "bot_token": "123456:ABC-DEF",
  "chat_id": "-100123456789",
  "webhook_secret": "random-secret"
}
```

## Secret Masking

Config secrets are masked in all API responses. The following fields are replaced with `"***"`:

| Channel type | Masked fields |
| --- | --- |
| `webhook` | `secret` |
| `telegram` | `bot_token`, `webhook_secret` |

Non-secret fields (e.g. `url`, `chat_id`, `headers`) are returned as-is.

Config values are stored encrypted at rest using `EncryptedString`. The plaintext config is only
decrypted server-side for delivery and test operations. See
[Notifications Security](../security/notifications-security.md) for details on secret storage
and callback verification.

## Response Types

Types are defined in `crates/shared/web-api-types/src/notifications.rs`:

| Type | Fields |
| --- | --- |
| `CreateNotificationChannelRequest` | `name` (String), `channel_type` (NotificationChannelType), `config` (JSON object), `enabled` (bool, default `true`) |
| `UpdateNotificationChannelRequest` | `name?` (String), `config?` (JSON object), `enabled?` (bool) |
| `NotificationChannelResponse` | `id` (Uuid), `name` (String), `channel_type` (NotificationChannelType), `config` (JSON, masked), `enabled` (bool), `created_at` (OffsetDateTime), `updated_at` (OffsetDateTime) |
| `CreateNotificationRuleRequest` | `channel_id` (Uuid), `event_type` (NotificationEventType), `host_id?` (Uuid), `software_item_id?` (Uuid), `plugin_type?` (String), `enabled` (bool, default `true`) |
| `UpdateNotificationRuleRequest` | `event_type?` (NotificationEventType), `host_id?` (Uuid), `software_item_id?` (Uuid), `plugin_type?` (String), `enabled?` (bool) |
| `NotificationRuleResponse` | `id` (Uuid), `channel_id` (Uuid), `event_type` (NotificationEventType), `host_id?` (Uuid), `software_item_id?` (Uuid), `plugin_type?` (String), `enabled` (bool), `created_at` (OffsetDateTime) |
| `NotificationLogResponse` | `id` (Uuid), `channel_id` (Uuid), `rule_id` (Uuid), `event_type` (NotificationEventType), `event_payload` (JSON), `status` (NotificationDeliveryStatus), `error_message?` (String), `action_token?` (Uuid), `action_taken?` (String), `created_at` (OffsetDateTime), `delivered_at?` (OffsetDateTime) |
| `TestNotificationResponse` | `success` (bool), `message` (String) |

## Key Files

| File | Purpose |
| --- | --- |
| `crates/ui/web-api/src/routes/notifications.rs` | Route handlers (channels, rules, log, Telegram callback) |
| `crates/ui/web-api-queries/src/queries/notifications.rs` | Database query helpers and error types |
| `crates/shared/web-api-types/src/notifications.rs` | Request/response types and enum definitions |
| `crates/plugins/notifications/core/src/traits.rs` | `NotificationPlugin` trait and `DeliveryMessage` type |
| `crates/plugins/notifications/webhook/src/lib.rs` | Webhook plugin implementation |
| `crates/plugins/notifications/telegram/src/lib.rs` | Telegram plugin implementation |
| `crates/shared/db/src/entity/notification_channel.rs` | SeaORM entity for `notification_channels` table |

## Related Documentation

- [HTTP Web API](http-web-api.md) -- API overview, error responses, pagination
- [Auth and Authorization](../security/auth-and-authorization.md) -- permissions model (`ManageNotifications`, `ViewNotifications`)
- [Notifications Security](../security/notifications-security.md) -- secret storage, config encryption, and callback verification
- [Notifications Development](../development/notifications.md) -- architecture, dispatcher design, and adding new channel types
