# Code Review: uptrakit-notification-channels

- **Review date**: 2026-03-02
- **Reviewer**: AI code review (architecture | security | quality | HA | standards |
  extensibility | tests | consistency | maintainability | database | crate-structure)
- **Branch**: docs/codereview-backend

## Summary

`uptrakit-notification-channels` is the shared notification delivery library at ~1,157 LoC,
providing channel implementations for email (SMTP via `lettre`), webhook (with HMAC-SHA256
signing), and Telegram (Bot API). Each channel type is feature-gated (`email`, `webhook`,
`telegram`), with `webhook` as the default feature. The crate has no dependency on database or
web-api crates -- it is a pure delivery abstraction consuming `DeliveryMessage` structs via the
`NotificationChannel` trait.

The two-layer email config design is well-executed: per-channel config stores only `to_addresses`,
while global SMTP settings (`smtp_host`, `smtp_port`, `smtp_username`, `smtp_password`,
`from_address`) are merged by the dispatcher before delivery. The webhook channel correctly
implements HMAC-SHA256 payload signing. Test coverage is strong with 38 tests across all modules.

Key concerns include a duplicated `escape_html` function across email and Telegram modules, the
`ChannelError` enum missing `#[non_exhaustive]`, and the absence of delivery integration tests
that verify actual message rendering (not just error paths).

## Architecture

### Strengths

- Feature-gated channel implementations (`email`, `webhook`, `telegram`) in `Cargo.toml:11-16`.
  Consumers can opt into only the channels they need, avoiding unnecessary dependencies (e.g.,
  `lettre` is only pulled in with the `email` feature).
- `NotificationChannel` trait (`channel.rs:44-59`) with `serde_json::Value` config parameter
  provides clean extensibility. Channels do not know about database schema or API types.
- `DeliveryMessage` (`channel.rs:12-24`) with optional `body_html` and `actions` fields.
  Channels that do not support rich text or action buttons silently ignore these fields,
  enabling graceful degradation.
- Two-layer email config: `EmailChannelConfig` (`email.rs:22-25`) stores only `to_addresses`
  in the per-channel database row. `EmailConfig` (`email.rs:32-43`) is the full merged struct
  containing both per-channel and global SMTP settings. The dispatcher is responsible for
  merging via `merge_smtp_into_config()` before calling `deliver()`.
- `ChannelRegistry` (`registry.rs:16-68`) provides compile-time-determined channel registration
  via feature flags. Lookup by type name string enables runtime dispatch without trait object
  downcasting.
- `#[must_use]` on `mask_config_secrets` (`channel.rs:57`) prevents callers from accidentally
  discarding the masked config and using the original.
- Webhook HMAC-SHA256 signing (`webhook.rs:86-92`) uses the `hmac` crate's typed API
  (`HmacSha256::new_from_slice`) rather than manual hash construction, reducing cryptographic
  implementation risk.

### Issues

No architecture issues found.

## Security and Safety

### Strengths

- Webhook channel (`webhook.rs:35-38`) and Telegram channel (`telegram.rs:28-31`) both set
  `.connect_timeout(10s)` and `.timeout(60s)` on their `reqwest::Client`, complying with the
  project HTTP client requirements.
- Webhook HMAC-SHA256 signing (`webhook.rs:86-92`) uses the `hmac` crate which provides
  constant-time verification internally. The signature is computed with
  `HmacSha256::new_from_slice` and formatted as `sha256=<hex>` in the `X-Uptrakit-Signature`
  header.
- `mask_config_secrets` is implemented per channel: webhook masks `secret` (`webhook.rs:140-148`),
  Telegram masks `bot_token` and `webhook_secret` (`telegram.rs:140-151`), email returns config
  unchanged since per-channel config contains no secrets (`email.rs:214-216`). SMTP credentials
  are stored in the global settings table, not in the per-channel config.
- Email channel validates recipient addresses at create/update time via `is_valid_email`
  (`email.rs:51-56`) with checks for `@` sign, non-empty local/domain parts, and at least one
  `.` in the domain.
- Zero `unsafe` blocks in production code.

### Issues

**[MEDIUM]** `src/webhook.rs:77-83` -- Custom headers from channel config
(`config["headers"]`) are applied to the outgoing webhook request without any header name
validation. A malicious or misconfigured channel config could inject headers like `Host`,
`Authorization`, or `Content-Length` that override the request's intended behavior. Consider
rejecting or warning on restricted header names (e.g., `Host`, `Content-Length`,
`Transfer-Encoding`).

**[LOW]** `src/telegram.rs:89` -- The Telegram Bot API token is embedded directly in the URL
path (`format!("https://api.telegram.org/bot{bot_token}/sendMessage")`). If request logging
is enabled at the HTTP transport level (e.g., via a `tracing` middleware on `reqwest`), the
bot token could appear in log output. Consider using a `SecretString` wrapper or ensuring
transport-level logging is disabled for this client.

## Code Quality

### Strengths

- Clean trait-based design with separate channel implementations in individual files.
  Each channel file follows the same structure: config structs, `NotificationChannel` impl,
  helper functions, tests.
- Consistent error handling via `ChannelError` with `rootcause` / `report!` / `bail!` macros.
  Error messages include context about what failed (e.g., `"SMTP delivery failed to {to_addr}:
  {e}"` at `email.rs:169-171`).
- `build_mailer` (`email.rs:225-274`) cleanly handles all three TLS modes (`tls`, `starttls`,
  `none`) with the default being `starttls`. Each mode is constructed via the appropriate
  `lettre` transport builder method.
- Email sends one message per recipient (`email.rs:138-175`) with individual error reporting
  per address, rather than using BCC which would hide recipients from each other but would
  also prevent per-recipient error tracking.
- `wrap_html` (`email.rs:59-63`) produces a minimal HTML5 document shell with `charset="utf-8"`
  meta tag, ensuring correct rendering in email clients.

### Issues

**[LOW]** `src/email.rs:66-71` and `src/telegram.rs:155-159` -- `escape_html` is duplicated
across email and Telegram modules. The email version escapes four characters (`&`, `<`, `>`,
`"`), while the Telegram version escapes three (`&`, `<`, `>`). Both should be consolidated
into a shared helper in `channel.rs` or a `utils.rs` module. The difference in escaped
characters (Telegram omits `"`) should be documented.

**[LOW]** `src/email.rs:51-56` -- `is_valid_email` is a minimal format check (non-empty
local, non-empty domain with at least one dot). It does not reject addresses with spaces,
control characters, or multiple `@` signs (the `splitn(2, '@')` handles multiple `@` by
treating everything after the first as the domain). While full RFC 5321 validation is
overkill, rejecting whitespace and control characters would catch common typos.

## High Availability

### Strengths

- Delivery is fire-and-forget from the channel perspective. Failures are returned as
  `Report<ChannelError>` to the dispatcher for logging and potential retry, not retried within
  the channel itself. This keeps the channel implementation simple and predictable.
- Email delivery iterates over recipients sequentially (`email.rs:138-175`). If one recipient
  fails, the error is returned immediately. The dispatcher can decide whether to retry the
  remaining recipients.

### Issues

**[LOW]** `src/email.rs:138-175` -- Email delivery to multiple recipients fails on the first
error and does not attempt the remaining addresses. If the first address is invalid but the
second is valid, the second recipient never receives the notification. Consider collecting
errors and returning a partial success/failure result, or documenting that the dispatcher
should create one channel per recipient for independent delivery.

## Coding Standards

### Strengths

- `edition = "2024"` with workspace field inheritance (`Cargo.toml:1-9`).
- `publish = false` correctly set (`Cargo.toml:8`).
- `[lints] workspace = true` (`Cargo.toml:38-39`).
- HTTP clients in webhook and Telegram channels comply with the project's timeout requirements
  (10s connect, 60s total).
- `rootcause` / `report!` / `bail!` error patterns used consistently across all channel
  implementations.
- `#[must_use]` on `mask_config_secrets` in the trait definition (`channel.rs:57`).

### Issues

**[MEDIUM]** `src/error.rs:7-32` -- `ChannelError` enum does not carry `#[non_exhaustive]`.
Per the project's coding standards (`coding-standards.md`), all extensible public enums should
carry `#[non_exhaustive]`. Adding new error variants (e.g., for rate limiting, authentication
failure) would be a breaking change for any external consumers matching on `ChannelError`.

## Extensibility

### Strengths

- Adding a new channel type requires three localized changes: (1) a feature flag in
  `Cargo.toml`, (2) a new module implementing `NotificationChannel`, (3) one
  `channels.insert()` call in `ChannelRegistry::new()` (`registry.rs:30-55`).
- `DeliveryMessage` uses optional fields (`body_html`, `actions`) for future enrichments.
  Channels that do not support these features silently ignore them.
- `MessageAction` (`channel.rs:28-36`) provides a generic callback mechanism. Channels render
  actions in their native format (inline keyboard for Telegram, JSON array for webhooks).

### Issues

**[LOW]** `src/registry.rs:30-55` -- No runtime `register()` method on `ChannelRegistry`. Only
`new()` populates the registry based on compile-time feature flags. This is acceptable for the
current first-party-only model but would need extension for plugin-provided or dynamically
loaded channels.

## Tests

### Strengths

- `src/email.rs:276-445` -- 16 tests covering: `validate_config` (rejects empty `to_addresses`,
  missing `to_addresses`, invalid email format, domain without dot; accepts valid single and
  multiple addresses), `mask_config_secrets` (returns config unchanged), `deliver` (returns
  error on missing required SMTP fields, returns error on unreachable SMTP host),
  `is_valid_email` (accepts standard addresses, rejects no-at-sign, rejects empty local/domain,
  rejects domain without dot), `escape_html` (escapes special chars, preserves plain text),
  `wrap_html` (produces valid HTML5 structure).
- `src/webhook.rs:151-229` -- 8 tests covering: `validate_config` (requires URL, rejects
  non-HTTP URLs, accepts HTTP and HTTPS, rejects non-object headers, accepts object headers),
  `mask_config_secrets` (replaces `secret`, preserves config without secret).
- `src/telegram.rs:161-243` -- 9 tests covering: `validate_config` (requires `bot_token`,
  requires `chat_id`, rejects empty `bot_token`, rejects empty `chat_id`, accepts valid
  config), `mask_config_secrets` (replaces `bot_token`, replaces `webhook_secret`),
  `escape_html` (escapes special chars, preserves plain text).
- `src/registry.rs:70-118` -- 5 tests covering: registry creates successfully, `get` returns
  `None` for unknown type, feature-gated tests verify webhook/telegram/email channels are
  registered when their feature is enabled.
- All synchronous tests correctly use `#[test]` rather than `#[tokio::test]`. The two
  `deliver` tests in `email.rs` that require async correctly use `#[tokio::test]`.

### Issues

**[LOW]** No test verifies the actual rendered webhook payload structure. The webhook `deliver`
method constructs a JSON payload with `title`, `body`, `event`, and `actions` fields
(`webhook.rs:55-66`), but no test asserts this structure. A test with a local HTTP server (or
by extracting the payload construction into a testable function) would guard against payload
schema regressions.

**[LOW]** No test verifies the HMAC-SHA256 signature computation. The webhook signing logic
(`webhook.rs:86-92`) is untested. A test providing a known secret and payload, then verifying
the `X-Uptrakit-Signature` header value matches the expected HMAC, would guard against
cryptographic regressions.

**[LOW]** No test verifies the Telegram message body rendering. The `deliver` method formats
the title in bold HTML (`<b>{}</b>`) and includes inline keyboard buttons for actions
(`telegram.rs:62-87`), but no test exercises this rendering path. A test with a mock HTTP
server would cover both the message formatting and the API call structure.

**[LOW]** `src/email.rs:352-398` -- The two `deliver` tests only verify error paths (missing
fields, unreachable host). No test verifies successful email construction (message headers,
multipart MIME structure, HTML wrapping). Extracting the `Message` building logic into a
testable function would enable assertions on the constructed email without requiring an SMTP
server.

## Consistency

### Strengths

- All three channel implementations follow the same trait method order: `deliver`,
  `validate_config`, `mask_config_secrets`. Each method has consistent doc comments.
- Error reporting follows the same pattern across channels: `report!(ChannelError::InvalidConfig(...))`
  for config problems, `report!(ChannelError::DeliveryFailed(...))` for delivery failures,
  with context messages that include the failing value.
- Non-success HTTP responses are handled identically in webhook (`webhook.rs:100-111`) and
  Telegram (`telegram.rs:99-110`): check `resp.status().is_success()`, read body text, log
  with `tracing::warn!`, return `ChannelError::DeliveryFailed` with status and body.
- `mask_config_secrets` implementations share the same pattern: clone, get mutable object ref,
  insert `"***"` for known secret keys.

### Issues

**[LOW]** `src/email.rs:66-71` vs `src/telegram.rs:155-159` -- Duplicated `escape_html`
functions with different escape sets. The email version escapes `"` (double quote) while the
Telegram version does not. This inconsistency should be documented or the functions should be
consolidated with an explicit API noting which characters are escaped.

## Maintainability

### Strengths

- Clean module structure: one file per channel (`email.rs`, `webhook.rs`, `telegram.rs`),
  plus `channel.rs` for the trait, `error.rs` for error types, and `registry.rs` for the
  channel lookup table. The `lib.rs` is 27 lines of re-exports.
- Each channel file is self-contained with its config structs, implementation, and tests
  co-located. A developer adding a new channel can use any existing channel file as a template.
- Feature flags in `Cargo.toml` include an `all` feature (`Cargo.toml:16`) for convenience in
  testing and development.

### Issues

No maintainability issues found.
