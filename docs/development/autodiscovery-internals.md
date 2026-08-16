# Autodiscovery Internals

This document is for developers changing autodiscovery behavior: adding a new discovery-capable
plugin, changing how `DiscoveryTarget` values are synthesized, touching the discovery allowlist,
or modifying the periodic rediscovery job. It captures invariants that are easy to violate by
accident because they are not enforced by the type system.

For the end-user-facing explanation of autodiscovery (workflow, ignore list, allowlist usage),
see [Autodiscovery (End-user Guide)](../end-user/autodiscovery.md). For the HTTP API surface, see
[Autodiscovery API Reference](../api/autodiscovery.md) and
[Discovery Allowlist API Reference](../api/discovery-allowlist.md). For general plugin
architecture (roles, `declare_plugin!`, capability derivation, `PluginConfig`), see
[Plugin Guidelines](plugin-guidelines.md) and [Plugin System Architecture](plugin-system.md) —
this document only covers autodiscovery-specific invariants not already documented there.

## 1. Discovery triggers

Discovery is event-driven and periodic:

- New host registration triggers discovery automatically.
- Explicit API calls: `POST /api/v1/hosts/{id}/discover` and
  `POST /api/v1/plugin-configs/{id}/discover` (see
  [Autodiscovery API Reference](../api/autodiscovery.md) for request/response shapes).
- Automatically every 6 hours via the `discover_software` scheduled task
  (`DiscoverSoftwareExecutor`).

The periodic task sends `DiscoverSoftware` to every active agent-backed host and soft-deletes
(`deactivated_at`) any `host_software_items` junction rows absent from the latest discovery
snapshot. See [Autodiscovery: Periodic Software Rediscovery](../end-user/autodiscovery.md#periodic-software-rediscovery)
for the user-facing behavior (disappeared packages, schedule configuration, auto-updating casks
excluded from discovery).

## 2. No approval workflow

All discovered items are created immediately with `enabled: true`. There is no pending state.
The `featured` flag controls visibility (individual entries vs. aggregated per-host summaries) —
see [Plugin Guidelines: Featured flag routing](plugin-guidelines.md#featured-flag-routing) for
the full table of which plugins set which value.

**Invariant:** re-discovery never overwrites a non-NULL `installed_version` on an active
`host_software_item` row (`find_or_create_software_item`, both the Phase-1 matched-update and the
pre-insert existing-row branches). Version fields are written only on fresh inserts, on
link-level reactivation (the matched row had `deactivated_at` set), and when the stored version
is NULL. Presence/provenance stamps (`last_discovered_at`, `discovery_source`,
`missing_since = NULL`) are written on every pass. For active items the `DetectVersion` scheduled
task is the sole version writer. This matters when debugging why a re-discovery run did not
refresh a version: preservation is the designed behavior for any active registered item, not a
bug in the rediscovery path.

## 3. Ignore list is separate from deletion

`DELETE /api/v1/software-items/{id}/hosts/{host_id}?ignore=true` removes the host assignment and
creates a `software_ignores` row keyed on the software item's `(tenant_id, name)`. A single
name-based ignore rule suppresses all future discoveries for that name across all plugin configs
and targets.

Without `?ignore=true`, unassigning is a plain delete with no ignore rule. Deleting a software
item (`DELETE /api/v1/software-items/{id}`) never creates ignore rules — only the host-assignment
delete path with the query parameter does.

See [Autodiscovery API Reference: `?ignore=true`](../api/autodiscovery.md#the-ignoretrue-query-parameter-on-delete-apiv1software-itemsidhostshost_id)
for the endpoint contract.

## 4. Plugin-driven discovery targets

Discovery results use structured `DiscoveryTarget` values
(`crates/shared/types/src/discovery_target.rs`) instead of opaque `extra` metadata. Each
`DiscoveredSoftware` item can carry a `targets: Vec<DiscoveryTarget>` that tells the controller
exactly which plugin configs and role assignments to create — no plugin-specific synthesis logic
lives in the web-API.

The controller processes discovery results generically via two paths (see
[Plugin Guidelines: Emitting `DiscoveryTarget` values](plugin-guidelines.md#emitting-discoverytarget-values)
for the general emission pattern and code example):

- **Target-based** (non-empty `targets`): for each target, find-or-create the plugin config and
  create role assignments per the target's `roles` list.
- **Config-ID-based** (empty `targets`, `plugin_config_id` set): use the discovering plugin's own
  config for all three roles.

The rest of this section documents the exact per-plugin emission shapes, which are not covered
elsewhere.

### PHS (Proxmox Helper Scripts)

PHS always emits `DiscoveryTarget` values. During discovery, it matches each container's CT-script
URL against a fixed `SOURCES` allowlist covering `community-scripts/ProxmoxVE`,
`community-scripts/ProxmoxVED`, `tteck/Proxmox`, and `worried-networking/uptrakit` (under
`scripts/pvehs/`), in both the `raw.githubusercontent.com` and `github.com/…/raw/…` URL forms, then
fetches and analyzes the matched CT script; see `SOURCES` in
`crates/plugins/discovery/proxmox-helper-scripts/src/discovery.rs` rather than a copy here.
Software identity is the bare slug — when the same slug appears from two sources, the first
occurrence wins. A definitive HTTP 404 fetching a CT or install script logs a warning and skips
that slug (the script may have vanished upstream); any other fetch failure aborts the discovery run
to avoid a partial snapshot. Install-script URLs are derived from the matched source via
`PhsSource::install_url`. When analyzing a CT script, shell line continuations (a trailing `\`) are
joined before the GitHub and Codeberg release-call collectors only — every other extractor (npm,
APT, the `GH_REPO=`/`CODEBERG_REPO=` variable forms) still sees the original, unjoined line
structure. The PHS shell constants live in
`crates/plugins/discovery/proxmox-helper-scripts/src/plugin.rs`.

**GitHub-managed apps** emit **two** `DiscoveryTarget` values:

1. `plugin_type: ReleasesGithub`, roles `[FetchReleases]`, config without `owner`/`repo` (only
   `tag_strip_prefix`, `include_prereleases`, `asset_patterns`), and `package_identifier:
Some("owner/repo")` override.
2. `plugin_type: GenericShell`, roles `[DetectVersion, ExecuteUpdate]`, config with
   `version_command` (`sudo /usr/local/bin/uptrakit-phs-version {package_identifier}`),
   `update_command` (`sudo PHS_SILENT=1 TERM=xterm /usr/bin/update`), and
   `prefer_interactive: true`.

   `sudo` is embedded in the command string because the Shell plugin uses
   `CommandSpec::shell()`, where `.privileged()` has no effect. `prefer_interactive: true` causes
   the controller to automatically set `interactive: true` in `ExecuteUpdatePayload` (see
   `config_prefers_interactive` in `update_dispatch.rs`), allocating a PTY so `/dev/tty` is
   available for prompts that `PHS_SILENT=1` does not suppress (e.g. the low-storage warning
   `read -r prompt < /dev/tty`).

**Codeberg-managed apps** (detected via `check_for_codeberg_release` or `CODEBERG_REPO=`) emit
**two** `DiscoveryTarget` values:

1. `plugin_type: ReleasesForgejo`, roles `[FetchReleases]`, config with `api_base_url:
"https://codeberg.org"` (Codeberg runs the Forgejo platform), `tag_strip_prefix: "v"`, and
   `package_identifier: Some("owner/repo")` override. The plugin config name is `"Codeberg
Releases"` to distinguish it from generic Forgejo instances.
2. `plugin_type: GenericShell`, roles `[DetectVersion, ExecuteUpdate]` — same PHS Shell target as
   for GitHub-managed items.

**npm-managed apps** emit **two** `DiscoveryTarget` values:

1. `plugin_type: PackageManagerNpm`, roles `[DetectVersion, FetchReleases]` (no `ExecuteUpdate`),
   config `{}`, name `"NPM (auto)"`, and `package_identifier: Some("<npm-package>")`.
2. `plugin_type: GenericShell`, roles `[ExecuteUpdate]`, same PHS Shell config as GitHub/Codeberg
   items (`version_command` + `update_command`), name `"PHS Shell"`, no `package_identifier`.
   Updates always go through `/usr/bin/update`, not `npm install -g`.

**APT-managed apps** emit **two** `DiscoveryTarget` values:

1. `plugin_type: PackageManagerApt`, roles `[DetectVersion, FetchReleases]` (no `ExecuteUpdate`),
   config `{}`, name `"APT (auto)"`, no `package_identifier`.
2. `plugin_type: GenericShell`, roles `[ExecuteUpdate]`, same PHS Shell config as above. Updates
   always go through `/usr/bin/update`, not `apt-get install`.

Apps whose scripts contain neither GitHub nor Codeberg patterns nor a specific `apt install` line
are skipped. The PHS plugin config itself (`discovery.proxmox-helper-scripts`, always `{}`) is
retained as an anchor for discovery runs but never linked directly to `SoftwareItem` host
assignments.

### Homebrew

Always emits per-item `DiscoveryTarget` values with `plugin_type: PackageManagerHomebrew` and
config `{"package_type": "formula"}` or `{"package_type": "cask"}`, plus display names `"Homebrew
(Formulae)"` and `"Homebrew (Casks)"`.

### Docker

Always emits one `DiscoveryTarget` per discovered item with `plugin_type: ReleasesDocker`, config
`{}`, name `"Docker"`, and all three roles.

### APT

Always emits one `DiscoveryTarget` per discovered item with `plugin_type: PackageManagerApt`,
config `{}`, name `"APT"`, and all three roles. (`discovery_filter` in type settings controls
whether discovery reports all installed packages or only manually-installed ones.)

### Cargo

Always emits one `DiscoveryTarget` per discovered crate with `plugin_type: PackageManagerCargo`,
config `{}`, name `"cargo"`, and all three roles.

### npm

Always emits one `DiscoveryTarget` per discovered package with `plugin_type: PackageManagerNpm`,
config `{}`, name `"npm"`, and all three roles.

### Snap

Always emits one `DiscoveryTarget` per discovered snap with `plugin_type: PackageManagerSnap`,
config `{}`, name `"Snap"`, and all three roles.

### The `extra` field

The `extra` field on `DiscoveredSoftware` is purely informational metadata (e.g. Docker's
`{"containers": ["web-server"]}`) — the controller never interprets it for config synthesis.

## 5. Discovery capability derivation

Call `state.plugin_ops.discovery_plugin_types()` (or `PluginCatalog::discovery_plugin_types()`
statically) to get the current list of discovery-capable plugin types. This is derived
automatically from each plugin's `capabilities()` method via the catalog — no static list is
maintained separately. Do not hand-maintain a list of discovery-capable plugin type strings
anywhere in route handlers or query helpers.

## 6. Package identifier validation

Plugin-specific constraints on the `package_identifier` field (e.g. Homebrew's allowed character
set) must be implemented as:

1. A crate-level `pub fn validate_identifier(value: &str) -> std::result::Result<(), String>` in
   the plugin crate.
2. An associated function on the config struct that delegates to it:

   ```rust
   impl MyConfig {
       pub fn validate_identifier(value: &str) -> std::result::Result<(), String> {
           crate::validate_identifier(value)
       }
   }
   ```

Plugins with no identifier constraints must still implement the associated function as a no-op
returning `Ok(())`. The `declare_plugin!` macro wires each config struct's associated function
into the `PluginDescriptor`; `PluginCatalog` dispatches through the descriptor list — no manual
match arm is required.

The `PluginOps` convenience alias (composed of `PluginMetadataOps + PluginConfigOps +
PluginSurfaceActionOps + PluginSurfaceOps + NotificationOps + SoftwareItemLifecycleOps`, defined
in `infrastructure-core` behind the `plugin-ops` feature, re-exported by
`infrastructure-registry`) exposes this as `validate_package_identifier_str(plugin_type: &str,
value: &str)` for trait-object dispatch. Crates that only need `PluginOps` (e.g.
`web-api-queries`) should depend on `infrastructure-core` with `features = ["plugin-ops"]` rather
than the full registry.

Never add plugin-specific validation logic directly to web API query helpers or route handlers.
See [Plugin Guidelines: Package identifier validation](plugin-guidelines.md#package-identifier-validation)
for the general two-API dispatch pattern (static vs. trait-object) and the checklist for adding a
new plugin's identifier rules.

## 7. Sudo command declarations

Plugins declare required sudo commands via `required_sudo_commands() -> Vec<SudoCommandEntry>` on
their `Plugin` impl. Each `SudoCommandEntry` carries a bare command name (or display identifier
for helper scripts), a human-readable explanation, and an optional `args_suffix`. For most
commands, **never hardcode absolute paths** — they are resolved on the target host via `command
-v` at bootstrap time.

For the base contract, trait signature, and a basic implementation example, see
[Plugin Guidelines: Declaring privileged commands with `required_sudo_commands()`](plugin-guidelines.md#declaring-privileged-commands-with-required_sudo_commands).
The rest of this section covers two variations not shown there.

**Restricting subcommands:** when a command needs only specific subcommands (e.g. `systemctl
stop` and `systemctl start` but not `systemctl disable`), set `args_suffix: Some("stop *")`. The
resolved path becomes `/usr/bin/systemctl stop *` in the sudoers file — positional matching
prevents other subcommands.

**Helper scripts:** when a simple sudoers command would be too broad (e.g. granting `cat` would
allow reading any file), use `SudoCommandEntry::new(command,
explanation).with_helper_script(SudoHelperScript::new(install_path, content))`. Bootstrap
installs the script at `install_path` with mode `0755` and uses that path as the sudoers command;
the script itself validates arguments to enforce the least-privilege contract that sudoers
wildcards cannot safely express (`*` matches `/` in sudoers).

**Never hardcode `sudo` in `CommandSpec`** — instead call `.privileged()` on the spec. Shell-mode
commands (`CommandSpec::shell`) must embed `sudo` in the command string directly because
`.privileged()` has no effect on shell mode (this is the same rule that produces the PHS
`GenericShell` targets' embedded `sudo` in [section 4](#4-plugin-driven-discovery-targets)).
`PluginCatalog::all_required_sudo_commands()` aggregates all declarations for use by the SSH
agent's sudoers generation logic. See also
[Sudoers Management](../security/sudoers-management.md).

## 8. Plugin capabilities

The `PluginCapability` enum is defined in `crates/shared/types/src/plugin_capability.rs` and has
16 variants. For the capability derivation mechanism (auto-derived from declared roles via
`declare_plugin!`) and the general capability table, see
[Plugin Guidelines: Plugin Capabilities](plugin-guidelines.md#plugin-capabilities) and
[Plugin System Architecture: Plugin Capabilities](plugin-system.md#plugin-capabilities). This
section documents capability details relevant to autodiscovery and update routing that are not
covered there.

### Role-assignment capabilities

Used by the UI `EditHostAssignmentModal` to filter plugin configs for each standard-role dropdown
via `GET /api/v1/plugin-types`:

- `VersionDetection` — the plugin implements `detect_installed_version()` and can be assigned to
  the `detect_version` role on a host software assignment. Serializes as `"version_detection"`.
  Implemented by all package-manager plugins and `ShellPlugin` (`generic.shell`).
- `ReleaseFetching` — the plugin implements `fetch_releases()` and can be assigned to the
  `fetch_releases` role. Serializes as `"release_fetching"`. Implemented by all package-manager
  plugins and all releases plugins (`github`, `gitlab`, `forgejo`, `docker`). Note: `gitlab` and
  `forgejo` do not declare `UpdateExecution` because they have no `execute_update`
  implementation.
- `UpdateExecution` — the plugin implements `execute_update()` and can be assigned to the
  `execute_update` role. Serializes as `"update_execution"`. Implemented by all package-manager
  plugins, `ShellPlugin`, `github`, and `docker`.

### Autodiscovery capabilities

- `DiscoverLocalSoftware` — the plugin can discover locally installed software.
- `RefreshPackageIndex` — the plugin can refresh/sync a local package index from remote sources.
- `DetectHostCompatibility` — the plugin implements `detect_host_compatibility()` which returns a
  `HostCompatibility` enum (`Compatible` or `Incompatible { reason: String }`). Both
  `HostCompatibility` and `PluginError` carry `#[non_exhaustive]`; `PluginError::is_retryable()`
  classifies transient errors (command spawn/wait, timeouts, capture failures, internal errors)
  for the version check retry logic in `crates/shared/agent-core/src/version_check.rs`. External
  match sites must include a wildcard arm (see `coding-standards.md` § Public Enum
  Extensibility). Implemented by: `AptPlugin` (checks `which apt-get`) and `HomebrewPlugin`
  (checks `which brew`).

### Update lifecycle and hooks

- `UpdateLifecycle` — the plugin implements the `LifecycleHook` narrow trait (via
  `as_update_lifecycle()` accessor). Provides `execute_pre_hook()` (returns
  `PreUpdateHookResult`: proceed or abort with reason) and `execute_post_hook()` (non-fatal,
  errors logged as warnings). The `UpdateLifecycleContext` contains `package_identifier`,
  `to_version`, `from_version`, `release_info`, and `update_succeeded` (`None` during pre-hooks,
  `Some(bool)` during post-hooks). Implemented by `SystemdHookPlugin` and `ShellHookPlugin`. See
  [Update Lifecycle Plugins](update-hooks.md) for the full hook contract.
- `ControllerSideFetchReleases` — the plugin's `fetch_releases()` requires no local system state
  and can run on the controller instead of the agent. Implemented by `GitHubPlugin`,
  `DockerPlugin`, `NpmPlugin`, and `CargoPlugin`. This capability interacts with the
  `execution_site` field on `host_software_item_plugins`: `auto` (default) delegates to the
  controller when this capability is present, `agent` forces agent-side execution, `controller`
  forces controller-side execution. See
  [Plugin System Architecture: Execution Site Decision Logic](plugin-system.md#execution-site-decision-logic)
  for the full phase-based scheduling algorithm.

### Other capabilities

- `NotificationDelivery` — the plugin delivers notifications via a transport channel.
- `HostLifecycle` — the plugin manages infrastructure host lifecycle (bootstrap, sync).
- `HostReport` — the plugin receives host report callbacks from the agent.
- `GuestExec` — the plugin provides guest execution capabilities (e.g. run commands inside VMs).
- `ServiceMigrations` — the plugin contributes service-side database migrations.
- `ControllerMigrations` — the plugin contributes controller-side database migrations.
- `ConfigTest` — the plugin supports dry-run configuration testing via `POST
/api/v1/plugin-configs/test`. Declared by all 17 plugins (10 package managers, 4 release
  plugins, 2 hook plugins, generic shell). Controller-side plugins
  (`ControllerSideFetchReleases`) test connectivity without a host; agent-side plugins require a
  `host_id` and run tests on the target host. The proxy pattern (`ConfigTestProxy` in
  `crates/ui/web-api/src/config_test_proxy.rs`) mirrors `ServiceSurfaceProxy`. Wire messages:
  `TestPluginConfig` / `TestPluginConfigResult` (session-targeted, not NATS-publishable). Gated
  by the `plugin-configs:trigger` action (`CanTriggerPluginConfigs`). See
  [Config Testing](config-testing.md) for the full endpoint and test-kind reference.

Update lifecycle hooks are standalone plugin assignments with roles `PreUpdateHook` and
`PostUpdateHook` on `host_software_item_plugins`, ordered by `ordinal`. See
[Update Lifecycle Plugins](update-hooks.md).

### Batch trait methods

All have default sequential fallbacks; override for efficiency. See
[Plugin Guidelines: Batch Updates](plugin-guidelines.md#batch-updates) and
[Plugin Guidelines: Batch Version Check](plugin-guidelines.md#batch-version-check) for the full
type definitions, fetch-failure preservation rules, and per-plugin implementation walkthroughs
(APT, Homebrew, npm). Summary of the three methods relevant to autodiscovery-adjacent version
checking:

- `batch_detect_installed_version(&[BatchDetectItem]) -> Result<Vec<BatchDetectResult>>` —
  detect installed versions for multiple packages. Default calls `detect_installed_version` per
  item. Override when the package manager accepts a list in one command (APT: `dpkg-query pkg1
pkg2`; Homebrew: `brew info --json=v2 pkg1 pkg2`; npm: `npm list -g --depth=0 --json` + memory
  filter).
- `batch_fetch_releases(&[BatchFetchItem]) -> Result<Vec<BatchFetchResult>>` — fetch upstream
  releases for multiple packages. Default calls `fetch_releases` per item. Override when the
  local package index supports multi-package queries (APT: `apt-cache madison pkg1 pkg2`;
  Homebrew: `brew info --json=v2 pkg1 pkg2`). Do **not** override for API-based plugins with
  per-package HTTP endpoints (GitHub, GitLab, npm registry).
- `execute_batch_update(&[BatchUpdateItem], output_tx) -> Result<Vec<BatchUpdateResult>>` —
  update multiple packages in one command. Default calls `execute_update` per item. Implemented
  by APT, Homebrew, and npm.

Agent-core `batch_check_versions()` groups `VersionCheckAssignment`s by `(PluginTypeId,
effective_config_json)` and calls these batch methods once per group via `join_all`.
`RefreshPackageIndex` is called at most once per unique fetch group (before `batch_fetch_releases`
runs); it is not called for detect-only groups. Scheduler Phase A groups by `plugin_config_id`
only and calls `batch_fetch_releases` once per config.

## 9. Discovery allowlist

Two tables, `tenant_discovery_allowlist` and `host_discovery_allowlist`, gate which discovery
plugin types execute during `trigger_discovery_for_agent_host()`:

- **Unconfigured (no entries for the tenant):** all discovery-capable plugin types run
  (backward-compatible default).
- **Tenant-wide entries exist:** only the listed plugin types run tenant-wide.
- **Host-specific entries exist for the target host:** those entries fully override the tenant
  list for that host (the host list replaces — not extends — the tenant list).

This applies to auto-discovery on new host registration and to `POST
/api/v1/hosts/{id}/discover`. It does **not** apply to `POST
/api/v1/plugin-configs/{id}/discover` (explicit plugin-config invocation bypasses the allowlist
intentionally). Duplicate entries are idempotent — the server returns the existing entry rather
than erroring. Only plugin types that have the `DiscoverLocalSoftware` capability and are not
unknown `PluginTypeId` values (i.e., strings not present in the catalog) can be added to the
allowlist; all other types are rejected with HTTP 400. (`PluginTypeId::new(...)` always accepts
any string; validation against known plugins happens via catalog lookup.)

The full priority-order semantics and CLI/API usage for operators are already documented in
[Autodiscovery: Controlling Which Plugins Run Discovery](../end-user/autodiscovery.md#controlling-which-plugins-run-discovery)
and [Discovery Allowlist API Reference: Allowlist Semantics](../api/discovery-allowlist.md#allowlist-semantics)
— this section exists only to state the invariant precisely for implementers (e.g. what "override"
means at the query level, and the idempotency/validation contract for the write endpoints).

## 10. Partial unique indexes

`software_items` uses a partial unique index `(tenant_id, name) WHERE deactivated_at IS NULL` —
prevents duplicate item names within a tenant while allowing re-creation after soft-delete.

`host_software_item_plugins` uses a unique index `(host_id, software_item_id, role, ordinal)` —
prevents duplicate role assignments for the same host-software-item pair.

Keep these in mind when writing migrations or queries that touch either table: a plain unique
index (without the partial `WHERE` clause) on `software_items` would incorrectly block
re-creating a previously soft-deleted item under the same name.
