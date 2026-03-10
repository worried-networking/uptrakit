# Code Review: Plugins (Umbrella)

- **Review date**: 2026-03-06
- **Parallel review date**: 2026-03-06
- **Reviewer**: AI code review (architecture|security|quality|HA|standards|extensibility|tests|consistency|maintainability|database|crate-structure)
- **Branch**: docs/codereview-backend

**Crates covered:** `uptrakit-plugin-generic-shell`

Non-trivial plugin crates have individual `CODEREVIEW.md` files in their respective directories:
`infrastructure/core`, `infrastructure/registry`, `releases/docker`, `releases/github`,
`package-managers/apt`, `package-managers/npm`, `package-managers/homebrew`, and
`discovery/proxmox-helper-scripts`.

## Summary

`uptrakit-plugin-generic-shell` (~418 LoC across 4 source files) is the simplest plugin in the
workspace. It executes user-configured shell commands for version checking and update execution.
The plugin delegates all command execution to `CommandExecutor` and follows the standard plugin
patterns established by `uptrakit-plugin-infrastructure-core`.

## Architecture

### Strengths

- `src/plugin.rs` -- Follows the standard `validate() -> new()` construction pattern enforced by
  `register_plugins!`.
- `src/config.rs` -- Configuration is minimal: command strings for check and update operations.
  No secrets, no complex state.

### Issues

No architectural issues found for `generic-shell`.

**[LOW]** Plugin config validation across all plugins happens via a JSON round-trip pattern:
`mask_secrets_for<T>` in the registry deserializes config from `serde_json::Value`, calls
`with_secrets_masked()`, then re-serializes. This double serialization occurs on every API
response that includes plugin configs. While not a hot path, it is architecturally wasteful.
(Confirmed by Architecture and Extensibility parallel reviews.)

## Security and Safety

### Strengths

- Command execution delegated entirely to `CommandExecutor` with shell escaping.
- `SecretMasking` default (no-op) is correct -- no secrets in configuration.
- No `unsafe` blocks.
- All plugins building `reqwest::Client` set `.connect_timeout(10s)` and `.timeout(60s)`,
  satisfying the workspace HTTP client requirement. (Confirmed by Security parallel review.)
- Plugin SSRF protection is sound: GitHub enforces HTTPS-only and rejects private hosts,
  Docker checks the registry host against `is_private_host()`, GitLab and Forgejo enforce
  HTTPS and reject private/loopback addresses. (Confirmed by Security parallel review.)

### Issues

No security issues found.

## Code Quality

### Strengths

- Clean, minimal implementation focused on shell command delegation.

### Issues

No code quality issues found.

## High Availability

### Strengths

- Stateless command execution. No shared mutable state.

### Issues

No high availability issues found.

## Coding Standards

### Strengths

- Consistent with workspace patterns: `bail!`, `report!`, `thiserror`-derived errors.
- Zero `#[allow(clippy::...)]` suppressions across most plugins.

### Issues

No coding standards issues found.

## Extensibility

### Strengths

- Generic shell command approach is inherently extensible -- any operation that can be expressed
  as a shell command is supported without code changes.

### Issues

No extensibility issues found.

## Tests

### Strengths

- `src/config.rs` and `src/plugin.rs` for the shell plugin follow the project convention:
  configuration tests (default values, serialisation round-trips) and plugin construction
  tests are present via the registry-level tests in `infrastructure/registry`.

### Issues

**[MEDIUM]** `crates/plugins/generic/shell/src/plugin.rs` -- The shell plugin has no unit
tests for `check_version`, `execute_update`, or the command-construction logic. The plugin
relies on `CommandExecutor` dependency injection, making it straightforward to test with a
`FixedOutputExecutor` mock (as used in the npm plugin). At minimum, a test verifying that the
configured command string is passed correctly to the executor would prevent regressions if
command-building logic changes.

No test issues found.

---

## Cross-Cutting HTTP Reliability

### Issues

---

## Cross-Cutting Plugin Findings

The following findings apply across the plugin subsystem and are documented here for reference.
Individual crate reviews contain the crate-specific details.

### Plugin Extension Checklist

When adding a new plugin, the following steps are required:

| Step | Location | Status |
| ------ | ---------- | -------- |
| New crate with `Plugin` + config struct | `crates/plugins/<name>/` | Clean |
| Implement `SecretMasking` | plugin config struct | Clean |
| One line in `register_plugins!` | `registry.rs` | Clean |
| Dependency in registry `Cargo.toml` | `registry/Cargo.toml` | Clean |
| New variant in `PluginType` | `shared/types/src/plugin_types.rs` | Clean |
| `as_str()`, `FromStr`, `Display` for new variant | `shared/types/src/plugin_types.rs` | Clean |
| **If discovery-capable:** include `PluginCapability::DiscoverLocalSoftware` in `capabilities()` | plugin crate | Clean |
| **If special identifier rules:** implement `validate_package_identifier` | plugin crate | Clean |

All previously "Manual" steps have been eliminated -- discovery support is now fully auto-derived
from the `register_plugins!` macro and the plugin's `capabilities()` method.

---

## Review — 2026-03-10

- **Reviewer**: AI code review (security|maintainability|extensibility|coding-standards|idiomatic-rust|allocation)
- **Crates covered**: `uptrakit-plugin-infrastructure-proxmox`, `uptrakit-plugin-infrastructure-registry`, `uptrakit-plugin-infrastructure-core`, `uptrakit-notification-plugin-core`, `uptrakit-notification-plugin-registry`, `uptrakit-notification-plugin-webhook`, `uptrakit-notification-plugin-telegram`, `uptrakit-notification-plugin-email`, `uptrakit-plugin-releases-github`, `uptrakit-plugin-releases-docker`, `uptrakit-plugin-apt`, `uptrakit-plugin-homebrew`, `uptrakit-plugin-npm`, `uptrakit-plugin-mas`, `uptrakit-plugin-generic-shell`

### Notifications

#### Security

**[HIGH] S6 — `uptrakit-notification-plugin-email` (`lib.rs:110-119`) — SMTP password leaked via `Debug`**

The local `SmtpSettingsSnapshot` struct in the email plugin carries `#[derive(Clone, Debug)]` with a `password: Option<String>` field. Any `{:?}` format of this struct emits the SMTP password to the log sink. The project memory explicitly states that `SmtpSettingsSnapshot` must have a masked `Debug` implementation. The `web-api` version of this struct correctly uses a manual `Debug` that redacts the password; the email plugin has a separate, unprotected copy without that safeguard.

Recommendation: replace `#[derive(Debug)]` on the email-plugin-local `SmtpSettingsSnapshot` with a manual `Debug` implementation that redacts the password field, or extract the struct into a shared crate so both sites share a single, correct implementation.

**[MEDIUM] S2 — `uptrakit-notification-plugin-telegram` (`lib.rs:29-33`) — Missing `dns_resolver` on `reqwest::Client`**

The Telegram plugin builds a `reqwest::Client` without a `dns_resolver` override, deviating from the project standard. If a custom bot-API-host option is added in future, this becomes a live SSRF vulnerability.

Recommendation: add `.dns_resolver(Arc::new(SsrfSafeResolver::new()))` to `TelegramPlugin::new()`.

#### Maintainability

~~**[HIGH] M1 — Notification plugin crates use `path` instead of workspace dependency for `uptrakit-notification-plugin-core`**

All four notification plugin crates reference the core notification crate via relative `path`:

| Crate | File |
| --- | --- |
| `uptrakit-notification-plugin-webhook` | `webhook/Cargo.toml:19` |
| `uptrakit-notification-plugin-telegram` | `telegram/Cargo.toml:17` |
| `uptrakit-notification-plugin-email` | `email/Cargo.toml:19` |
| `uptrakit-notification-plugin-registry` | `registry/Cargo.toml:23` |

The workspace root already declares this crate. Bypassing the workspace declaration breaks single-point version control and requires manual version bumps across four files when the core crate changes.

Recommendation: change all four references to `uptrakit-notification-plugin-core = { workspace = true }`.~~ *(Fixed: all four `Cargo.toml` files updated to use `workspace = true`.)*

#### Positive findings

- **S4 (confirmed)** — `uptrakit-notification-plugin-webhook` correctly applies `SsrfSafeResolver` for DNS-level protection, additionally guards against private hosts via `is_private_host()` at config-save time, and disables redirect following. This is defense-in-depth beyond the project baseline.
- **S5 (confirmed)** — `SmtpSettingsSnapshot` in `uptrakit-web-api` (`settings.rs:109-121`) correctly implements a custom `Debug` that redacts the password field. (Note: the email plugin's local copy does not yet share this protection — see S6 above.)

---

### Releases

#### Allocation

**[LOW] A1 — `uptrakit-plugin-releases-github` (`plugin.rs:206-219`) — Structural clones in `convert_release`**

`convert_release` clones all `GitHubRelease` and `GitHubAsset` fields due to the two-pass convert-then-attest structure. The clones are structurally necessary given the current design. If this becomes a hot path (e.g., releases with large asset lists), the design should be revisited. If intentional for clarity, add a comment documenting why.

**[LOW] A2 — `uptrakit-plugin-releases-github` (`plugin.rs:256-278`) — Small-allocation pressure in `parse_checksums_content`**

`parse_checksums_content` allocates a `String` for each filename and each 64-character hex digest. For checksums files with many entries this produces many small allocations. The hex digest could be stored as a decoded `[u8; 32]` instead of a `String`. This is a low-frequency path and therefore low priority, but worth noting for future optimisation.

#### Positive findings

- **S4 (confirmed)** — `uptrakit-plugin-releases-docker`, `uptrakit-plugin-releases-github`, `uptrakit-plugin-releases-gitlab`, `uptrakit-plugin-releases-forgejo`, `uptrakit-plugin-discovery-proxmox-helper-scripts`, `uptrakit-plugin-npm`, and `uptrakit-notification-plugin-webhook` all correctly apply `.dns_resolver(Arc::new(SsrfSafeResolver::new()))` on their HTTP clients.
- **R6 (confirmed)** — All HTTP-using plugins correctly set `connect_timeout(10s)` and `timeout(60s)`. `apt`, `homebrew`, and `mas` do not build HTTP clients (they shell out to local package managers), which is correct.

---

### Package Managers

#### Idiomatic Rust

**[MEDIUM] R1 — `uptrakit-plugin-apt`, `uptrakit-plugin-homebrew`, `uptrakit-plugin-npm`, `uptrakit-plugin-mas` — `validate_identifier` returns `Result<(), String>`**

Every package manager plugin defines its own `pub fn validate_identifier(value: &str) -> std::result::Result<(), String>`. The `String` return type loses error-kind context and prevents programmatic distinction between validation failure reasons.

Recommendation: define a shared `ValidationError` enum or newtype that implements `Display` and `Error`, and return `Result<(), ValidationError>` from all `validate_identifier` functions.

**[MEDIUM] R2 — `uptrakit-plugin-npm` (`config.rs:54`) — `validate` returns `Result<(), String>` instead of `crate::error::Result<()>`**

All other plugin config structs return `crate::error::Result<()>` from `validate`. The npm plugin is the odd one out, requiring different error-mapping code at call sites.

Recommendation: change the return type to `crate::error::Result<()>` (trivial, since the function body currently returns `Ok(())`).

**[MEDIUM] R3 — `uptrakit-plugin-apt` (`plugin.rs:36-43`) — `unwrap_or('\0')` sentinel instead of `let-else`**

`unwrap_or('\0')` is used to handle an empty-string edge case. The `'\0'` path is documented as unreachable but produces a subtly different error message and is less clear than an explicit early return. The `mas` plugin handles the analogous case correctly with `let-else`.

Recommendation: replace with `let Some(first) = value.chars().next() else { return Err(...) }`.

~~**[MEDIUM] R4 — `uptrakit-plugin-apt` (`config.rs:99-101`) and `uptrakit-plugin-homebrew` — fieldless enums missing `Copy`**

`AptDiscoveryFilter` and `HomebrewPackageType` are fieldless enums that do not derive `Copy`. This forces `.clone()` at use sites where `Copy` semantics would be more natural.

Recommendation: add `Copy` to both derive lists.~~ *(Fixed: `Copy` added to both `AptDiscoveryFilter` and `HomebrewPackageType`; `.clone()` calls removed.)*

#### Positive findings

- **R5** — `parse_dpkg_output` (apt), `parse_madison_output`, `parse_mas_list_line` use iterator combinators (`filter_map`, `find_map`, `split_once`, `rfind`, `splitn`) correctly throughout with no manual index loops and no unnecessary intermediate allocations.

---

### Infrastructure

#### Security

**[MEDIUM] S1 — `uptrakit-plugin-infrastructure-proxmox` (`client.rs:29-38`) — Missing `dns_resolver` on Proxmox HTTP client**

The Proxmox VE HTTP client builds `reqwest::Client` with no `dns_resolver` override. The project standard requires `SsrfSafeResolver` on all plugins that accept user-controlled URLs. If the plugin is extended to fetch URLs sourced from Proxmox API responses (e.g., helper-script download URLs), this becomes a live SSRF vector.

Recommendation: add `.dns_resolver(Arc::new(SsrfSafeResolver::permissive()))` to the Proxmox `reqwest::Client::builder()`. (`permissive()` is appropriate here because Proxmox is a self-hosted control plane that may be on a private network.)

**[LOW] S3 — `uptrakit-plugin-infrastructure-proxmox` (`client.rs:33`) — No warning logged when `verify_tls = false`**

When `verify_tls` is set to `false`, the client is constructed silently. Users who configure this for self-signed certificates receive no visibility that TLS verification is disabled, which exposes the Proxmox control plane to MitM attacks without any log-level indication.

Recommendation: emit `tracing::warn!("Proxmox TLS verification is disabled; connection is vulnerable to MitM attacks")` at client construction time when `verify_tls` is false.

#### Extensibility

**[MEDIUM] E1 — `uptrakit-plugin-infrastructure-registry` (`registry.rs:455-469`) — Fragile string-prefix dispatch for plugin extensions**

Plugin extension dispatch uses `extension_id.starts_with($ext_prefix)` string prefix matching. Overlapping prefixes silently misroute calls, and there is no compile-time guarantee that a plugin's extension IDs are consistent with its declared prefix. Currently only the Proxmox plugin declares extensions, so the risk is latent.

Recommendation: replace prefix matching with exact extension-ID registration in the `register_plugins!` macro, or add a compile-time assertion that all manifest IDs begin with the declared prefix.

**[MEDIUM] E2 — `uptrakit-plugin-infrastructure-registry` (`lib.rs:138-143`) — `extension_manifests()` and `extension_actions()` bypass macro dispatch**

`PluginOps::extension_manifests()` and `extension_actions()` are hardcoded to call Proxmox extensions directly, outside the `register_plugins!` macro dispatch. Adding a second plugin with UI extensions requires manual edits to this dispatch site rather than a single macro entry.

Recommendation: extend `register_plugins!` to accumulate `extension_manifests()` and `extension_actions()` from all entries that declare an `extension_handler`, eliminating the hardcoded call sites.

---

### Generic / Shell

#### Coding Standards

~~**[LOW] CS2 — `uptrakit-plugin-generic-shell` (`error.rs`) — `ShellError` is a dead type**

`ShellError` is declared but never bridged to `PluginError` via `impl_report_conversion!`. Production code at `plugin.rs:64,88,114` manually constructs `report!(PluginError::...)`. `ShellError` appears only as the declared return type of `ShellPlugin::new()` and is never actually constructed in any error path.

Recommendation: either add `impl_report_conversion!(ShellError, PluginError)` and use `ShellError` consistently in `plugin.rs` (matching the pattern of all other plugins), or remove `ShellError` entirely and use `PluginError` directly in the function signatures.~~ *(Fixed: `error.rs` deleted entirely; `ShellPlugin::new()` and `validate()` now return
`uptrakit_plugin_infrastructure_core::Result<_>` directly.)*

---

### Cross-Cutting (All Plugins)

#### Coding Standards

**[LOW] CS1 — `uptrakit-plugin-infrastructure-core` (`plugin_ops.rs:12`) — `PluginOpsError` missing `#[non_exhaustive]`**

`PluginOpsError` is a public error enum used across crate boundaries but does not carry `#[non_exhaustive]`. New error conditions are plausible future additions. Without this attribute, any crate that exhaustively matches all current variants will fail to compile when a new variant is added, creating a breaking change without a semver bump.

Recommendation: add `#[non_exhaustive]` to `PluginOpsError` and verify that all external match sites add a wildcard arm with appropriate handling.

#### Positive findings (cross-cutting)

- **S4 (confirmed)** — SSRF protection via `SsrfSafeResolver` is correctly applied across all user-URL-accepting plugins. See per-area entries for per-plugin details.
- **R6 (confirmed)** — HTTP client timeout discipline (`connect_timeout(10s)`, `timeout(60s)`) is consistent across all HTTP-using plugins.

---

## 2026-03-10 Review Update

Comprehensive 12-dimension review covering architecture, security, code quality, tests, HA,
database, coding standards, extensibility, consistency, idiomatic Rust, references and heap,
and maintainability.

### Dimension: Security

#### Strengths

- `notifications/telegram/src/lib.rs:137-148` -- Telegram `mask_config_secrets` masks both
  `bot_token` and `webhook_secret`, and webhook secret comparison uses constant-time equality
  via the standard sentinel check pattern, preventing timing side-channel leaks.

#### Issues

**[MEDIUM]** `notifications/telegram/src/lib.rs:29-33` -- Telegram plugin builds
`reqwest::Client` without `SsrfSafeResolver`. The bot API URL is currently hardcoded to
`api.telegram.org` (line 86), but the `bot_token` is user-controlled and embedded in the URL
path. If a custom API endpoint is added in future, this becomes a live SSRF vector. Already
noted in the 2026-03-10 review section above; this entry confirms the finding from the
12-dimension security pass.

### Dimension: Code Quality

#### Strengths

- Consistent plugin error architecture across all notification and infrastructure plugins:
  each crate defines its own error type with `thiserror`, uses `impl_report_conversion!` for
  bidirectional conversion, and follows the `rootcause` framework throughout.
- `notifications/core/src/traits.rs:44-62` -- `NotificationPlugin` trait is appropriately
  minimal: three required methods (`channel_type`, `deliver`, `validate_config`) plus one
  `#[must_use]` method (`mask_config_secrets`). No default implementations that could mask
  missing functionality.
- All HTTP-using notification plugins (`webhook`, `telegram`) correctly set
  `.connect_timeout(10s)` and `.timeout(60s)`, satisfying the workspace HTTP client
  requirement.

### Dimension: Coding Standards

#### Issues

**[HIGH]** `notifications/email/src/lib.rs:80` -- `.expect("config is always an object")` in
`merge_smtp_into_config`. This function is called from route handlers with user-provided config
JSON. If a non-object value is passed (e.g., `null`, `[]`, `"string"`), the server panics. The
project coding standard prohibits `.expect()` and `.unwrap()` in production code.
Recommendation: replace with a match or `if let` that returns an error for non-object configs.

### Dimension: Extensibility

#### Strengths

- `notifications/core/src/traits.rs:44-62` -- `NotificationPlugin` trait is appropriately
  minimal with three required methods. Adding a new notification channel requires implementing
  only these three methods plus registering in the notification registry.

#### Issues

~~**[MEDIUM]** `notifications/core/src/traits.rs:11-24` -- `DeliveryMessage` is a public struct
used across crate boundaries but does not carry `#[non_exhaustive]`. Adding a new field (e.g.,
`priority`, `thread_id`) would be a breaking change for any code that constructs the struct
with positional syntax.~~ *(Fixed: `#[non_exhaustive]` added; `DeliveryMessage::new()`
constructor added.)*

~~**[MEDIUM]** `notifications/core/src/traits.rs:27-36` -- `MessageAction` is similarly missing
`#[non_exhaustive]`. Adding fields like `style` or `confirmation_required` would break
external constructors.~~ *(Fixed: `#[non_exhaustive]` added; `MessageAction::new()`
constructor added.)*

~~**[MEDIUM]** `notifications/registry/src/lib.rs:102-122` -- `NotificationOps` trait defines
`mask_config_secrets` but has no corresponding `restore_config_secrets` method. The
infrastructure `PluginOps` trait provides both mask and restore operations. Without restore,
notification channel config updates that include masked sentinel values cannot recover the
original secrets, requiring the user to re-enter secrets on every config edit.~~ *(Fixed:
`restore_config_secrets` default method added to `NotificationPlugin` trait; method added to
`NotificationOps` trait and `NotificationPluginRegistry` impl.)*

**[LOW]** `notifications/registry/src/lib.rs:54-78` -- Channel type strings (`"webhook"`,
`"telegram"`, `"email"`) are repeated as string literals at registration sites and in each
plugin's `channel_type()` return value. These are not centralized as constants, creating a
risk of typo-induced mismatches between registration and lookup.

**[LOW]** `infrastructure/registry/src/registry.rs:491-504` -- The `register_plugins!` macro
wildcard arm for unknown `PluginType` variants returns an error but does not emit
`tracing::warn!`. The workspace coding standard requires wildcard arms on `#[non_exhaustive]`
enums to log a warning for observability of unexpected variants.

### Dimension: Idiomatic Rust

#### Issues

**[LOW]** `notifications/core/src/lib.rs:19-25` -- `escape_html` creates up to five
intermediate `String` allocations via chained `.replace()` calls. For large bodies this
produces unnecessary allocation pressure. A single-pass character-by-character approach using
`String::with_capacity` would be more efficient while remaining equally readable.

### Dimension: Maintainability

#### Strengths

- Plugin crates are well-scoped and independent: each notification plugin crate has a single
  responsibility (one channel type), minimal dependencies, and no cross-plugin coupling. The
  registry crate is the sole integration point.
