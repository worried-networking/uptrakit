# Code Review: `crates/plugins` Umbrella

- Review date: 2026-03-17
- Reviewer: claude-opus-4-6
- Scope: full 14-dimension review of all plugin crates without a dedicated `CODEREVIEW.md`

## Covered Crates

- `uptrakit-plugin-generic-shell`
- `uptrakit-plugin-hook-shell`
- `uptrakit-plugin-hook-systemd`
- `uptrakit-plugin-infrastructure-proxmox`
- `uptrakit-notification-plugin-core`
- `uptrakit-notification-plugin-email`
- `uptrakit-notification-plugin-telegram`
- `uptrakit-notification-plugin-webhook`
- `uptrakit-plugin-package-manager-apk`
- `uptrakit-plugin-package-manager-cargo`
- `uptrakit-plugin-package-manager-dnf`
- `uptrakit-plugin-package-manager-mas`
- `uptrakit-plugin-package-manager-pacman`
- `uptrakit-plugin-package-manager-pkg`
- `uptrakit-plugin-package-manager-snap`
- `uptrakit-plugin-enhancement-dashboard-icons`

## Summary

The plugin subsystem is in good shape overall. Config validation, SSRF defence, timeout
discipline, and unit coverage are broadly strong. This review cycle identified one new HIGH
finding (Telegram global bot token stored in plaintext), confirmed the existing email and
telegram panic risks, and added a finding about the Dashboard Icons HTTP client bypassing the
shared security builder. The extension handler registration limitation (compile-time only) is
an accepted architectural tradeoff and is documented explicitly.

## Strengths

- The small plugins remain easy to reason about and rely on shared infrastructure instead of
  duplicating HTTP, command, or secret-handling code.
- The generic-shell plugin has focused unit tests for placeholder replacement and failure
  propagation, and applies `shell_escape` defensively to all user-supplied identifiers.
- All notification plugins inherit consistent timeout and SSRF behavior from the shared client
  builder (webhook, telegram) or from the email plugin's SMTP connect timeout.
- Plugin HMAC signing (webhook) and HTML escaping (telegram) are correctly applied.
- The webhook plugin correctly disables redirect following and explicitly rejects redirect
  responses as a secondary SSRF defence layer.
- Webhook custom headers are validated against a case-insensitive blocklist at both config
  validation and delivery time (defence-in-depth).
- Telegram callback webhook uses constant-time comparison (`subtle::ConstantTimeEq`) after
  SHA-256 hashing to prevent timing attacks on the secret token.
- The `list_channels` shared helper in `notification-plugin-core` avoids code duplication
  across all three notification plugins.
- Package manager plugins consistently use `execute_and_capture` instead of inline boilerplate.
- Snap, Pacman, Cargo, and npm all implement proper batch operations for version detection
  and release fetching.
- The systemd hook plugin correctly declares `required_sudo_commands` with `args_suffix`
  constraints (`stop *`, `start *`) for least-privilege sudoers entries.

## Active Findings

### [HIGH] Telegram global bot_token is stored in plaintext in the `global_settings` table

- **Dimension**: security
- **Scope**: `crates/plugins/notifications/telegram/src/extensions.rs:handle_save_global_telegram`
- **Description**: The `save_global_telegram` action writes the bot token directly as a JSON
  string to `global_settings` via `upsert_global_setting_raw`, without calling `encrypt_str`.
  In contrast, the email plugin encrypts SMTP passwords using `uptrakit_crypto::encrypt_str`
  before persisting them (see `handle_save_smtp` and `handle_save_global_smtp` in
  `crates/plugins/notifications/email/src/extensions.rs`).
- **Why it matters**: A Telegram bot token grants full control of the bot (send messages, read
  updates, manage webhooks). If the database is compromised, the token is immediately usable.
  The SMTP password is encrypted at rest with AAD, making the same attack significantly harder.
- **Failure scenario**: Database backup leak, SQL injection read, or shared-hosting DB access
  exposes the bot token in plaintext. An attacker can impersonate the bot, read callback data,
  or pivot to further attacks via the Telegram Bot API.

### [MEDIUM] Email and Telegram plugins panic on non-object config JSON

- **Dimension**: fault tolerance, coding standards
- **Scope**: `crates/plugins/notifications/email/src/lib.rs:97` (`merge_smtp_into_config`),
  `crates/plugins/notifications/telegram/src/lib.rs:267` (`deliver`)
- **Description**: Both call `config.as_object_mut().expect("config is always an object")`.
  A malformed row or unexpected caller can take down the dispatch path instead of returning a
  typed error.
- **Why it matters**: The project coding standard prohibits `unwrap()`/`expect()` in
  production code. Both sites are in the notification delivery hot path.
- **Failure scenario**: Settings-store corruption, manual DB edit, or a buggy caller passes a
  non-object JSON value during notification delivery, causing a panic instead of a graceful
  error return.

### [MEDIUM] Dashboard Icons cache uses a bare `reqwest::Client` without SSRF-safe resolver or enforced timeouts

- **Dimension**: security, consistency
- **Scope**: `crates/plugins/enhancements/dashboard-icons/src/cache.rs:DashboardIconCache::new`
  and `crates/plugins/infrastructure/registry/src/registry.rs:with_dashboard_icons`
- **Description**: The `DashboardIconCache` accepts an externally-constructed `reqwest::Client`
  and uses it to make requests to the GitHub API. The caller in the registry
  (`with_dashboard_icons`) passes a `reqwest::Client` that is constructed outside the plugin
  system without the shared `build_plugin_http_client` builder, which means SSRF-safe DNS
  resolution, WebPKI TLS, connect timeout (10s), and request timeout (60s) are not guaranteed.
- **Why it matters**: The GitHub API URL (`api.github.com`) is hardcoded, so the immediate SSRF
  risk is low. However, bypassing the shared HTTP client builder violates the project convention
  that all plugin HTTP clients must use `build_plugin_http_client` or equivalent, and the cache
  has no connect timeout, making it vulnerable to slow-loris or hung-connection scenarios during
  the periodic refresh.
- **Failure scenario**: A DNS rebinding attack against `api.github.com` (low probability) or a
  hung connection during the 6-hour refresh loop blocks the tokio task indefinitely because no
  timeout is set on the client.

### [MEDIUM] `SoftwareItemPatch` builder contract is fragile for external implementors

- **Dimension**: extensibility, API stability
- **Scope**: `crates/plugins/infrastructure/core/src/plugin_base.rs`, `SoftwareItemPatch` struct
- **Description**: `SoftwareItemPatch` uses a builder pattern with `#[non_exhaustive]`. All
  current fields are `Option<T>`, which makes the builder safe for now. If a non-optional field
  is ever added, every external `SoftwareItemLifecyclePlugin` implementation that constructs a
  patch via the builder will fail to compile.
- **Why it matters**: The Dashboard Icons plugin and any future lifecycle plugins depend on this
  API.
- **Failure scenario**: Adding a required field breaks all external implementors at compile time.

### [LOW] Per-tenant SMTP settings saved non-atomically

- **Dimension**: fault tolerance, DB design
- **Scope**: `crates/plugins/notifications/email/src/extensions.rs:handle_save_smtp` and
  `handle_save_global_smtp`
- **Description**: Each SMTP field is saved via a separate `upsert_setting_raw` call without
  wrapping them in a database transaction. If the connection drops mid-save, the tenant ends
  up with a mix of old and new values (e.g., new host but old port and old TLS mode).
- **Why it matters**: The project's coding standard requires multi-statement DB mutations to
  use `db.begin()`/`txn.commit()` for atomicity.
- **Failure scenario**: Network blip during SMTP settings save leaves the tenant with
  inconsistent SMTP configuration, causing delivery failures until the admin re-saves.

### [LOW] `handle_service_extension_action()` fails silently when not overridden by infrastructure plugins

- **Dimension**: extensibility, developer experience
- **Scope**: `crates/plugins/infrastructure/core/src/plugin_base.rs:PluginBase::handle_service_extension_action`
- **Description**: The method has a default `None` implementation. An infrastructure plugin that
  exposes `extension_manifests()` UI actions but forgets to override
  `handle_service_extension_action()` will silently ignore all agent-side extension action
  requests.
- **Why it matters**: The error message from the registry is generic and does not indicate which
  method is missing.

### [LOW] DNF plugin concentrates all logic in a 1242-line `plugin.rs`

- **Dimension**: maintainability
- **Scope**: `crates/plugins/package-managers/dnf/src/plugin.rs`
- **Description**: Detection, release fetching, batch operations, update execution, discovery,
  and parsing are all in one file. Other package managers (apt, homebrew, npm, cargo, snap,
  pacman) split these concerns into separate modules.
- **Why it matters**: Future changes to any single concern (e.g., release parsing) require
  navigating a large file with higher regression risk.

## Resolved Findings

- "Several umbrella-managed plugins are now monolithic" (previous review) -- the Proxmox
  infrastructure plugin has been split into well-scoped modules (client.rs, discovery.rs,
  matching.rs, config.rs, extensions.rs, agent/). The remaining large-file concern is narrowed
  to the DNF plugin and the Proxmox Helper Scripts discovery plugin (1655-line discovery.rs),
  both of which have proportionate test coverage.

## Split/Merge Notes

- No merge is recommended for the small hook and generic plugins.
- `infrastructure/proxmox` is well-modularised across client, discovery, matching, config,
  extensions, and agent submodules.
- Extension handler registration is compile-time via the `register_plugins!` macro -- this is
  an accepted tradeoff for the current first-party-only plugin model.
- The `dashboard-icons` enhancement plugin would benefit from accepting a
  `PluginHttpClientConfig` instead of a bare `reqwest::Client`.
