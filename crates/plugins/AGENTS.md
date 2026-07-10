# AGENTS -- Plugin Crates Guide

Scoped guide for AI agents working under `crates/plugins/`. Read the root [AGENTS.md](../../AGENTS.md) first --
this file only covers conventions specific to plugin crates and does not repeat quality gates, commit
conventions, or workspace-wide Rust standards. The canonical, human-maintained reference for everything
plugin-related is [docs/development/plugin-guidelines.md](../../docs/development/plugin-guidelines.md); link
to it rather than re-deriving its content here. Related docs: [plugin-system.md](../../docs/development/plugin-system.md)
(role-based assignment data model), [autodiscovery-internals.md](../../docs/development/autodiscovery-internals.md)
(discovery pipeline), and [sudoers-management.md](../../docs/security/sudoers-management.md) (sudo security model).

## Descriptor pattern: `declare_plugin!` + `PluginDescriptor`

Every plugin crate exports a `DESCRIPTOR: PluginDescriptor` static via the `declare_plugin!` macro
(`crates/plugins/infrastructure/core/src/macros.rs`). The descriptor carries identity (`type_id`, `display_name`,
`family`, `scope`), config operations, role creators, capabilities, and optional sections (surface actions,
type settings, config test, sudo commands, migrations). Every compiled-in plugin must be listed in
`all_descriptors()` in `uptrakit-plugin-infrastructure-registry` (`crates/plugins/infrastructure/registry/src/registry.rs`)
-- this is the single authoritative list the `PluginCatalog` dispatches against.

Plugins implement narrow role traits rather than one monolithic `Plugin` trait. See
`crates/plugins/infrastructure/core/src/roles.rs` for the full set (`Discoverer`, `VersionDetector`,
`ReleaseFetcher`, `PackageIndexer`, `UpdateExecutor`, `LifecycleHook`, `NotificationTransport`,
`SoftwareItemLifecycle`, `HostLifecycle`, `HostReport`, `GuestExec`, and others) -- do not assume a plugin
implements all of them; check the descriptor's declared roles. Most software plugins implement
`VersionDetector` + `ReleaseFetcher` + `UpdateExecutor` together; discovery-only, fetch-only, and
enhancement-only plugins are valid partial-role configurations.

## Mandatory shared helpers (`infrastructure-core`)

New package-manager plugins **must** use these instead of hand-rolling command execution or update logic.
All are re-exported from the `uptrakit-plugin-infrastructure-core` crate root:

- `execute_and_capture(executor, cmd, context)` (`src/command.rs`) -- runs a command via `execute_quiet`,
  maps process-level errors to `PluginError::PluginInternal(context)`, and maps non-zero exit codes to
  `PluginError::CommandFailed(code)`. Do not use it where a non-zero exit has a meaningful non-error
  interpretation (e.g. `rpm -q` exit 1 = not installed) -- call `execute_quiet` directly in that case.
- Update helpers in `src/helpers.rs`:
  - `require_package_identifier(value, predicate)` -- one-line identifier validation wrapper.
  - `execute_command_update` / `CommandUpdateParams` -- single-package update execution.
  - `execute_batch_versioned_command` / `BatchVersionedParams` -- batch update with version-embedded args
    (e.g. `pkg@ver`).
  - `execute_batch_names_command` / `BatchNamesParams` -- batch update with names-only args.
  - `refresh_package_index_command` -- package index refresh (e.g. `apt-get update`).

Grep `crates/plugins/package-managers/*/src/` for existing callers before writing new update-execution
code -- nearly every case is already covered by one of these helpers.

## Package-identifier validation

Plugins with charset constraints on `package_identifier` (apt, apk, cargo, dnf, pacman, pkg, snap) declare a
local `const IDENTIFIER_RULES: PackageIdentifierRules` (struct in `crates/shared/types/src/package_identifier.rs`)
and expose a crate-level `pub fn validate_identifier(value: &str) -> Result<(), String>` that calls
`IDENTIFIER_RULES.validate(value)`, plus any plugin-specific extra checks. The `PluginConfig::validate_identifier`
associated function on the config struct delegates to this crate-level function. Plugins with no identifier
constraints still implement the associated function as a no-op returning `Ok(())`.

Homebrew and npm are older plugins that still hand-roll their `validate_identifier` logic directly (path-segment
checks, `@scope/name` parsing) rather than using `PackageIdentifierRules`. Do not copy their pattern for new
plugins -- prefer `PackageIdentifierRules` unless the format genuinely cannot be expressed as a charset, length,
and first-char rule (as with npm's scoped-package syntax).

Never put plugin-specific identifier validation in web-api query helpers or route handlers -- it belongs in the
plugin crate and is dispatched through the descriptor/catalog.

## Sudo command rules

Plugins that need passwordless root execution declare it via `required_sudo_commands() -> Vec<SudoCommandEntry>`
(`SudoCommandEntry` in `crates/plugins/infrastructure/core/src/traits.rs`), wired into the descriptor's `sudo`
field. Rules:

- **Never hardcode absolute paths.** `command` is a bare name (e.g. `"apt-get"`, `"systemctl"`); it is resolved
  via `command -v <name>` on the target host at bootstrap/sync time.
- **Restrict subcommands with `args_suffix`** when a command should only be allowed for specific subcommands
  (e.g. `args_suffix: Some("stop *")` restricts to `systemctl stop *`, not `systemctl disable`).
- **Use helper scripts when sudoers wildcards would be too broad.** If a bare command grant would allow more
  than intended (sudoers `*` matches `/`), install a validating helper script via
  `SudoCommandEntry::new(...).with_helper_script(SudoHelperScript::new(install_path, content))` instead.
- **Never hardcode `sudo` in `CommandSpec`.** Call `.privileged()` on the spec instead, and declare the
  corresponding entry in `required_sudo_commands()` so the sudoers file stays minimal. Exception:
  `CommandSpec::shell()` must embed `sudo` directly in the command string, since `.privileged()` has no effect
  in shell mode.

See [Sudoers Management](../../docs/security/sudoers-management.md) for the full security rationale.

## HTTP clients

Controller-side plugins that make HTTP calls build their client via `build_plugin_http_client(cfg)`
(`crates/plugins/infrastructure/core/src/http_client.rs`, feature `http-client`) rather than calling
`reqwest::Client::builder()` directly. It centralizes SSRF protection, TLS hardening (WebPKI roots), and
timeouts: `connect_timeout` is a fixed 10s, `timeout` defaults to 60s. Pass `SsrfMode::Strict` (uses
`SsrfSafeResolver::new()`) for any client that will contact user-controlled URLs; use `SsrfMode::Permissive`
(`SsrfSafeResolver::permissive()`) only for plugins/deployments that intentionally allow private-network
targets (e.g. a self-hosted registry URL a tenant configured themselves). Apply auth headers per-request, not
as client default headers, to avoid credential leakage across redirects.

## Capabilities

`PluginCapability` values are declared as part of the `declare_plugin!` invocation and stored on the static
`PluginDescriptor.capabilities` field; they are not something a plugin computes at runtime. Discovery capability
specifically should never be tracked as a separate static list anywhere else in the codebase -- callers derive
"which plugin types support discovery" from the catalog (`PluginCatalog` / `discovery_plugin_types()`), which
reads it off each plugin's declared capabilities.

For tests, use `FixedOutputExecutor` / `RoutedOutputExecutor` and the `test_runtime()` / `test_runtime_with_executor()`
factories from `crates/plugins/infrastructure/core/src/testing.rs` (feature `testing`) instead of writing local
mock `CommandExecutor` / `HostRuntime` structs. `FixedOutputExecutor::success(output)` / `::failure(exit_code)`
cover single fixed-response cases; `RoutedOutputExecutor::success(pairs)` / `::new(triples)` cover
command-routed multi-response cases.

## Batch trait methods

Override `batch_detect_installed_version` / `batch_fetch_releases` / `execute_batch_update` only when the
underlying package manager genuinely supports a multi-package call (e.g. `dpkg-query` with all package names,
`apt-cache madison`, `dnf repoquery`, `pacman -Si`). Do not override `batch_fetch_releases` for plugins whose
upstream is a per-package HTTP API (GitHub, GitLab, Forgejo releases) -- those already get concurrency via
per-item requests (see Cargo's `buffer_unordered(10)` pattern for a bounded-concurrency example), and a fake
"batch" wrapper around N sequential HTTP calls adds complexity without the efficiency win a real batch call
provides. The default trait implementations already fall back to sequential per-item calls, so omitting an
override is always correct behavior, just not always the most efficient one.

## Wire-safe enums and `#[non_exhaustive]`

`PluginCapability`, `HostCompatibility`, and `PluginError` (all defined in or re-exported from
`infrastructure-core`) carry `#[non_exhaustive]`. Any new plugin-facing enum that crosses a crate boundary
(especially anything serialized over the wire protocol or REST API) should follow the same convention. See
[docs/development/coding-standards.md](../../docs/development/coding-standards.md) for the full
`#[non_exhaustive]` and `Other(String)` wire-safe-enum conventions -- do not restate them here.

## Maintaining this file

Keep this file at or under 250 lines (enforced by CI). Do not add a code-structure inventory table or
hardcoded crate/plugin counts -- those go stale immediately and duplicate what `git ls` and the root AGENTS.md
codebase layout already provide. When a convention here becomes outdated, verify against the actual code in
`crates/plugins/infrastructure/core/src/` before editing, since `docs/development/plugin-guidelines.md` itself
lags behind some recent refactors (e.g. the move from `register_plugins!`/`PluginRegistry` to
`declare_plugin!`/`all_descriptors()`/`PluginCatalog`).
