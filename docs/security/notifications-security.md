---
title: Notification Subsystem Security
weight: 120
description: Security model for notification channels covering secret storage, webhook HMAC signing, Telegram callback verification, and tenant isolation.
---

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

Config secrets are masked in API responses via the `mask_config_secrets()` method on the
`NotificationTransportPlugin` trait (`crates/plugins/infrastructure/core/src/plugin_base.rs`).
The method carries `#[must_use]`, following the same pattern as `PluginRegistry::mask_config_secrets`
-- callers must use the masked output.

Masked fields per channel type:

| Channel type | Masked fields |
| --- | --- |
| `webhook` | `secret` |
| `telegram` | `bot_token`, `webhook_secret` |
| `email` | none (per-channel config stores only `to_addresses`) |

All other config fields (e.g. `url`, `chat_id`) are returned unmasked.

## Email Channel Security

The email channel uses a three-layer config model that keeps SMTP credentials separate from per-channel
recipient lists. This section covers the security properties of that design.

### SMTP credentials storage

SMTP settings are stored at two levels:

- **Global defaults** in the `global_settings` table (`global_smtp.*` keys) — server-wide SMTP
  configuration accessible to all tenants.
- **Per-tenant overrides** in the `settings` table (`smtp.*` keys) — per-tenant settings that
  override global defaults on a field-by-field basis.

Password fields (`global_smtp.password` and `smtp.password`) are encrypted at rest using
`uptrakit_crypto::encrypt_str` (AES-256-GCM with the master key) before being written to the database.
Decryption occurs at settings-load time and the plaintext is held in memory only within
`SmtpSettingsSnapshot` structs.

SMTP passwords are **never returned in API responses**. Extension action responses expose only
`has_password: bool` to indicate whether a password is configured at each level.

See [Secrets and Encryption](secrets-and-encryption.md) for encryption semantics and master key
management.

### TLS mode recommendations

| Mode | Security level | Recommended use |
| --- | --- | --- |
| `tls` | Highest — full SMTPS, no downgrade possible | Production (port 465) |
| `starttls` | Good — opportunistic TLS upgrade | Production (port 587), widely supported |
| `none` | Plaintext — credentials and email transmitted in the clear | Development/testing only, never production |

The default TLS mode is `"starttls"`. Administrators are advised to use `"tls"` whenever possible.

### Per-channel config contains no credentials

Each email notification channel stores only `to_addresses` in the encrypted `notification_channels.config`
column. SMTP credentials are never written to per-channel config. This means:

- Compromising a per-channel config row reveals only the recipient list, not SMTP credentials.
- Multiple email channels share the same SMTP server without duplicating the password.
- The dispatcher merges global and per-tenant SMTP settings into the config at delivery time,
  in memory only.

### `has_password` masking

SMTP extension actions return `has_password: bool` rather than the actual password. This follows the
same convention used by MQTT (`has_password`) and OIDC (`has_client_secret`). Both global and per-tenant
SMTP responses use this masking pattern.

### Permission requirements

SMTP settings are managed via extension actions within the email notification plugin.
The extension manifests gate access through `ManageSettings` and `ViewSettings` permissions
on the relevant action definitions.

### Key files

| File | Purpose |
| --- | --- |
| `crates/plugins/notifications/email/src/lib.rs` | `EmailPlugin` -- SMTP delivery, config validation, internal SMTP merge |
| `crates/plugins/notifications/email/src/surfaces.rs` | SMTP settings handlers (global and per-tenant) with password encryption |
| `crates/ui/web-api/src/settings.rs` | `SmtpSettingsSnapshot` with masked `Debug` impl |
| `crates/shared/db/src/raw_settings.rs` | Raw-key settings store functions used by notification plugins |

## Webhook URL Validation and Header Blocklist

Webhook notification channels validate URLs and custom headers at config creation/update time
to mitigate SSRF and header injection attacks.

### Private host validation

When `allow_private_urls` is `false` (the default), `validate_config()` rejects webhook URLs
pointing to private, loopback, link-local, CGNAT, and reserved addresses. The validation uses
the shared `is_private_host()` function from `uptrakit_shared_types::network`, which blocks:

- IPv4: `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `127.0.0.0/8`, `169.254.0.0/16`,
  `100.64.0.0/10` (CGNAT), `0.0.0.0`
- IPv6: `::1` (loopback), `::` (unspecified), `fc00::/7` (ULA), `fe80::/10` (link-local)
- Hostnames: `localhost`, `*.local`, `*.internal`, `*.localhost`

Self-hosted / single-tenant deployments can opt out via the `--allow-private-notification-urls`
controller CLI flag. When set, the private-host check is skipped, allowing webhooks to reach
internal services (e.g. internal Mattermost, local Docker registries).

### Redirect following disabled

The webhook HTTP client uses `redirect(Policy::none())` to disable automatic redirect following.
Any 3xx response is explicitly rejected with a descriptive error message. This prevents
redirect-based SSRF where an attacker-controlled server returns a 301 to an internal address
after passing initial URL validation.

### Header blocklist

Custom webhook headers (`config["headers"]`) are validated against a case-insensitive blocklist.
The following header names are always rejected, regardless of the `allow_private_urls` setting:

- `authorization`, `cookie`, `host`, `proxy-authorization`
- `x-forwarded-for`, `x-forwarded-host`, `x-real-ip`

This prevents an attacker from injecting authentication or proxy-related headers into outbound
webhook requests.

### Key files

| File | Purpose |
| --- | --- |
| `crates/shared/types/src/network.rs` | Shared `is_private_host()` function |
| `crates/plugins/notifications/webhook/src/lib.rs` | `validate_config()` with URL and header validation |
| `crates/core/controller/src/cli.rs` | `--allow-private-notification-urls` CLI flag |

The threat model: a user with `manage_notifications` permission can configure a webhook URL
targeting internal services (cloud metadata endpoints, private IPs) to probe the internal network
or exfiltrate operational data. Mitigations include `is_private_host()` URL validation,
`SsrfSafeResolver` DNS rebinding protection, a security-header blocklist, and redirect blocking.

## Webhook HMAC Signing

When a webhook channel has a `secret` field configured, outbound HTTP requests include an
`X-Uptrakit-Signature` header containing an HMAC-SHA256 signature of the request body in the format
`sha256=<hex>`.

Implementation: `crates/plugins/notifications/webhook/src/lib.rs`.

Recipients should verify this signature to authenticate webhook payloads:

1. Read the raw request body bytes.
2. Compute `HMAC-SHA256(secret, body)`.
3. Compare the hex-encoded result against the value after the `sha256=` prefix in the
   `X-Uptrakit-Signature` header using a constant-time comparison function.

If no `secret` is configured on the channel, the `X-Uptrakit-Signature` header is omitted entirely.

## Notification Callback Verification

The generic callback endpoint (`POST /api/v1/notifications/callback/{channel_type}/{channel_id}`)
is public -- it is not behind JWT authentication. It is registered outside the authenticated API
router so that external services (e.g. Telegram's Bot API servers) can reach it directly.

The callback endpoint dispatches to the plugin's `handle_callback` extension action, which
performs channel-type-specific verification. The controller-side route handler provides
common pre-checks:

1. **Channel ID binding**: The `channel_id` in the URL path must resolve to an existing
   notification channel. If the channel does not exist, the request is rejected with HTTP 404.
2. **Channel type matching**: The `channel_type` in the URL path must match a registered
   notification plugin. Unsupported types are rejected with HTTP 404.
3. **Plugin-specific verification**: The plugin's `handle_callback` action performs its own
   authentication (e.g. the Telegram plugin verifies the `X-Telegram-Bot-Api-Secret-Token`
   header against the channel's `webhook_secret` config field).
4. **Action token validation**: Each actionable notification generates a unique UUIDv7 action
   token. The plugin's callback handler validates that the token exists in the
   `notification_log` table and has not already been actioned (`action_taken IS NULL`).

If any check fails, the request is rejected. Invalid action tokens or already-actioned tokens
return HTTP 200 with an empty JSON body to prevent the external service from retrying.

Implementation: `notification_callback` in `crates/ui/web-api/src/routes/notifications.rs`
dispatches to the plugin's `handle_callback` action via
`plugin_ops.handle_extension_action()`.

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

### Notification callback and tenant scoping

The generic notification callback endpoint bypasses `TenantDb` -- it loads the channel by primary
key directly from the database (`notification_channel::Entity::find_by_id`). This is necessary
because the endpoint is not JWT-authenticated and therefore has no tenant context.

The callback's scope is intentionally minimal:

- It reads the channel's encrypted config to pass to the plugin's `handle_callback` action.
- The plugin verifies the request and may read/update a single `notification_log` entry by
  action token.
- It does **not** return any tenant data, channel metadata, or log content in the response body.

The channel itself is tenant-bound (its `tenant_id` foreign key references the `tenants` table), so a valid
callback can only affect log entries belonging to that channel's tenant.

## Rate Limiting Considerations

The notification callback endpoint is currently **not** rate-limited. Because it is publicly
reachable, it is susceptible to brute-force attempts against action tokens or denial-of-service
via high request volume.

Mitigating factors:

- Action tokens are UUIDv7 (122 bits of entropy in the random portion), making brute-force infeasible.
- Plugin-specific secret verification rejects unauthorized requests before any database write occurs.
- Invalid or already-actioned tokens return immediately without side effects.

**Future work**: Add per-IP rate limiting to the callback endpoint, similar to the WebSocket rate limiter, to
provide defense-in-depth against abuse.

## Key Files

| File | Purpose |
| --- | --- |
| `crates/plugins/infrastructure/core/src/plugin_base.rs` | `NotificationTransportPlugin` trait with `#[must_use]` on `mask_config_secrets` |
| `crates/plugins/notifications/webhook/src/lib.rs` | Webhook plugin: HMAC-SHA256 signing, secret masking |
| `crates/plugins/notifications/webhook/src/surfaces.rs` | Webhook surface action handler |
| `crates/plugins/notifications/telegram/src/lib.rs` | Telegram plugin: bot token masking, webhook secret masking |
| `crates/plugins/notifications/telegram/src/surfaces.rs` | Telegram surface action handler (including callback verification) |
| `crates/plugins/notifications/email/src/lib.rs` | Email plugin: SMTP delivery, no per-channel secrets |
| `crates/plugins/notifications/email/src/surfaces.rs` | Email surface action handler (SMTP settings with password encryption) |
| `crates/ui/web-api/src/settings.rs` | `SmtpSettingsSnapshot`: masked `Debug`, decrypted password in memory only |
| `crates/plugins/infrastructure/registry/src/registry.rs` | Unified `PluginRegistry` with `notification_transport()` for channel type dispatch |
| `crates/ui/web-api/src/routes/notifications.rs` | API route handlers including generic `notification_callback` |
| `crates/ui/web-api/src/notifications/dispatcher.rs` | Background dispatcher: rule matching, action token generation, generic delivery |
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
