# Notification Subsystem Security

## Overview

The notification subsystem handles sensitive data (channel credentials, webhook secrets, bot tokens) and
exposes a public callback endpoint for Telegram interactive actions. This document covers the security model,
secret management, authentication of external callbacks, and tenant isolation guarantees.

## Secret Storage

Channel configuration -- bot tokens, webhook URLs, HMAC secrets -- is stored encrypted in the database using
`uptrakit_crypto::EncryptedString`. The `notification_channels.config` column holds AES-256-GCM encrypted JSON
(ciphertext format: `ENC:v1:<hex(nonce || ciphertext || tag)>`), decrypted at runtime using the master key.

The master key must be initialized before any channel can be created or read. A missing master key is a hard
failure -- there is no plaintext fallback. See [Secrets and Encryption](secrets-and-encryption.md) for the
master key lifecycle and `EncryptedString` semantics.

### Secret masking in API responses

Config secrets are masked in API responses via the `mask_config_secrets()` method on the `NotificationChannel`
trait (`crates/shared/notification-channels/src/channel.rs`). The method carries `#[must_use]`, following the
same pattern as `PluginRegistry::mask_config_secrets` -- callers must use the masked output.

Masked fields per channel type:

| Channel type | Masked fields |
| --- | --- |
| `webhook` | `secret` |
| `telegram` | `bot_token`, `webhook_secret` |

All other config fields (e.g. `url`, `chat_id`) are returned unmasked.

## Webhook HMAC Signing

When a webhook channel has a `secret` field configured, outbound HTTP requests include an
`X-Uptrakit-Signature` header containing an HMAC-SHA256 signature of the request body in the format
`sha256=<hex>`.

Implementation: `crates/shared/notification-channels/src/webhook.rs`.

Recipients should verify this signature to authenticate webhook payloads:

1. Read the raw request body bytes.
2. Compute `HMAC-SHA256(secret, body)`.
3. Compare the hex-encoded result against the value after the `sha256=` prefix in the
   `X-Uptrakit-Signature` header using a constant-time comparison function.

If no `secret` is configured on the channel, the `X-Uptrakit-Signature` header is omitted entirely.

## Telegram Callback Verification

The Telegram callback endpoint (`POST /api/v1/notifications/callback/telegram/{channel_id}`) is public -- it
is not behind JWT authentication. It is registered outside the authenticated API router so that Telegram's Bot
API servers can reach it directly.

Security is provided by three layered checks:

1. **`X-Telegram-Bot-Api-Secret-Token` header**: Telegram sends this header with every webhook request. The
   value must match the `webhook_secret` field in the channel's encrypted config. If the secret is empty or
   does not match, the request is rejected with HTTP 401.
2. **Action token validation**: Each actionable notification generates a unique UUIDv7 action token. The
   callback handler validates:
   - The token exists in the `notification_log` table.
   - The token has not already been actioned (`action_taken IS NULL`).
3. **Channel ID binding**: The `channel_id` in the URL path must resolve to an existing notification channel.
   If the channel does not exist, the request is rejected with HTTP 404.

If any check fails, the request is rejected. Invalid action tokens or already-actioned tokens return
HTTP 200 with an empty JSON body to prevent Telegram from retrying.

Implementation: `telegram_callback` in `crates/ui/web-api/src/routes/notifications.rs`.

## Action Token Lifecycle

Action tokens enable one-time interactive actions (e.g. "Install Update") from notification messages.

1. When the dispatcher processes an actionable event (e.g. `UpdateAvailable`), it generates a UUIDv7
   `action_token` via `Uuid::now_v7()`.
2. The token is stored in `notification_log.action_token` (nullable, with a UNIQUE index).
3. The token is embedded in the Telegram inline keyboard button's `callback_data` field.
4. When the user presses the button, Telegram sends the token back via the callback endpoint.
5. The handler sets `action_taken = "triggered"` on the log entry.
6. The token cannot be reused -- subsequent presses of the same button are silently ignored
   (the handler checks `action_taken.is_some()` and returns HTTP 200 with an empty body).

The UNIQUE index on `action_token` prevents duplicate tokens at the database level.

## Permissions

Two dedicated permissions govern access to the notification subsystem:

| Permission | Serialized name | Grants |
| --- | --- | --- |
| `ViewNotifications` | `view_notifications` | Read channels, rules, and delivery log |
| `ManageNotifications` | `manage_notifications` | Create, update, delete channels and rules; test channel delivery |

These permissions use the standard typed-extractor pattern (`CanViewNotifications`, `CanManageNotifications`)
and carry `x-required-permission` OpenAPI extensions. See
[Auth and Authorization -- Permissions Model](auth-and-authorization.md#permissions-model---detailed) for the
full RBAC architecture.

## Tenant Isolation

All notification data is tenant-scoped:

| Table | Tenant column |
| --- | --- |
| `notification_channels` | `tenant_id` |
| `notification_rules` | `tenant_id` |
| `notification_log` | `tenant_id` |

All authenticated API queries use `TenantDb`, which automatically filters by the authenticated user's tenant.
Foreign keys from `notification_rules` and `notification_log` reference `notification_channels`, which is
itself tenant-scoped, so cross-tenant references are structurally impossible for authenticated endpoints.

### Telegram callback and tenant scoping

The Telegram callback endpoint bypasses `TenantDb` -- it loads the channel by primary key directly from the
database (`notification_channel::Entity::find_by_id`). This is necessary because the endpoint is not
JWT-authenticated and therefore has no tenant context.

The callback's scope is intentionally minimal:

- It reads the channel's encrypted config to verify the webhook secret.
- It reads and updates a single `notification_log` entry by action token.
- It does **not** return any tenant data, channel metadata, or log content in the response body.

The channel itself is tenant-bound (its `tenant_id` foreign key references the `tenants` table), so a valid
callback can only affect log entries belonging to that channel's tenant.

## Rate Limiting Considerations

The Telegram callback endpoint is currently **not** rate-limited. Because it is publicly reachable, it is
susceptible to brute-force attempts against action tokens or denial-of-service via high request volume.

Mitigating factors:

- Action tokens are UUIDv7 (122 bits of entropy in the random portion), making brute-force infeasible.
- The webhook secret check rejects unauthorized requests before any database write occurs.
- Invalid or already-actioned tokens return immediately without side effects.

**Future work**: Add per-IP rate limiting to the callback endpoint, similar to the WebSocket rate limiter, to
provide defense-in-depth against abuse.

## Key Files

| File | Purpose |
| --- | --- |
| `crates/shared/notification-channels/src/channel.rs` | `NotificationChannel` trait with `#[must_use]` on `mask_config_secrets` |
| `crates/shared/notification-channels/src/webhook.rs` | Webhook channel: HMAC-SHA256 signing, secret masking |
| `crates/shared/notification-channels/src/telegram.rs` | Telegram channel: bot token masking, webhook secret masking |
| `crates/shared/notification-channels/src/registry.rs` | `ChannelRegistry` for channel type dispatch |
| `crates/ui/web-api/src/routes/notifications.rs` | API route handlers including `telegram_callback` |
| `crates/ui/web-api/src/notifications/dispatcher.rs` | Background dispatcher: rule matching, action token generation, delivery |
| `crates/shared/db/src/entity/notification_channel.rs` | `notification_channels` entity with `EncryptedString` config |
| `crates/shared/db/src/entity/notification_log.rs` | `notification_log` entity with `action_token` and `action_taken` |
| `crates/shared/db/src/entity/notification_rule.rs` | `notification_rules` entity with scope filters |
| `crates/shared/db/src/migration/m20260301_000001_notifications.rs` | Database migration: tables, indexes, foreign keys |

## See Also

- [Secrets and Encryption](secrets-and-encryption.md) -- encryption-at-rest, master key handling,
  `EncryptedString` semantics
- [Auth and Authorization](auth-and-authorization.md) -- JWT authentication, RBAC permission model, typed
  permission extractors
- [Secure Development](secure-development.md) -- secure coding expectations for contributors
