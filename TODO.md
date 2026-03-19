# Uptrakit Project Roadmap

## About This Document

This roadmap tracks the development of Uptrakit, a self-hosted software update monitoring and management system.

**Current Status**: Early MVP stage with foundational infrastructure in place.

**Key Documentation**:

- [README.md](README.md) - Project overview and architecture
- [ARCHITECTURE.md](ARCHITECTURE.md) - System design and technology decisions
- [SECURITY.md](SECURITY.md) - Security policy and cryptographic details
- [CONTRIBUTING.md](CONTRIBUTING.md) - Development setup and contribution guidelines
- [docs/README.md](docs/README.md) - Documentation catalogue

**Legend**: Each pending item includes metadata —
**Category** (e.g. security, UI, plugins), **Impact** (Low / Medium / High),
**Effort** (Low / Medium / High).

______________________________________________________________________

## Phase 1: Foundation Layer — COMPLETE

All foundation work is done. Summary of what was delivered:

- **Database & Persistence** — SQLite + PostgreSQL, SeaORM migrations, connection pooling, JSON
  settings store.
- **Core Data Models** — Host, SoftwareItem, Version (semver + custom), UpdateRecord, validation,
  repositories/DAOs.
- **User Authentication & Authorization** — Argon2id passwords, JWT + session tokens, full RBAC
  (33 granular permissions, 8 built-in roles, 5 access presets), OIDC, rate limiting, audit logging.
  Auth API (register, login, logout, me), user management API (users, roles, permissions, presets).
  OpenAPI/Swagger docs.
- **Agent Authentication & Security** — mTLS with auto-issued client certificates, CA pinning,
  dual-CA rotation (6-month window, 24h check), partitioned CRLs, OCSP, certificate expiration
  monitoring, automated renewal (agent + server certs), revocation checking.
- **Wire Protocol** — Full message set (registration, discovery, version check, update commands,
  status, errors), serialization, routing, protocol versioning.
- **Agent Registration & Discovery** — Enrollment flow (auto/manual approval), certificate
  issuance, inventory tracking, heartbeat, status monitoring, metadata collection.
- **Plugin Trait System** — Plugin trait (detect, check, update, update lifecycle hooks, host
  compatibility), `PluginCatalog` + `declare_plugin!` macro, capability discovery
  (`PluginCapability`), configuration storage (`plugin_configs` table).
- **Role-Based Plugin Assignment** — `host_software_item_plugins` table with per-role assignments
  (`detect_version`, `fetch_releases`, `execute_update`, `pre_update_hook`, `post_update_hook`),
  `execution_site` column, per-host latest version tracking, controller-side and agent-side
  `fetch_releases`.
- **Update Lifecycle Plugins** — Standalone `hook_systemd` and `hook_shell` plugins assigned via
  `PreUpdateHook`/`PostUpdateHook` roles with ordinal-based ordering. `LifecycleHook`
  trait with `execute_pre_hook()` / `execute_post_hook()` methods. Replaces the old embedded
  hook system (predefined templates + custom commands in plugin config JSON).

### Plugin System: Follow-up Items

- [ ] Wire compatibility detection results to controller for dashboard display
  - **Category**: Plugins | **Impact**: Medium | **Effort**: Low-Medium
  - Compatibility detection runs on agents but results are not yet sent to the controller. Wire a
    new message type or extend `DiscoveryResults` so the dashboard can show per-host plugin
    compatibility status.
- [ ] Add `RebootRequired` wire message / post-update event system
  - **Category**: Plugins / Wire | **Impact**: Medium | **Effort**: Medium
  - A shell hook plugin could detect `/var/run/reboot-required`, but the result stays local.
    Add a wire message so the controller can display reboot-needed status and optionally trigger
    notifications.
- [x] ~~"Run arbitrary commands" plugin type (custom script plugin)~~ — Implemented as
  `uptrakit-plugin-generic-shell` with `version_command` / `update_command` and
  `{package_identifier}` / `{version}` / `{tag}` placeholders.
  - **Category**: Plugins | **Impact**: High | **Effort**: Medium
  - Allow users to define custom version-detect / update scripts without writing a Rust plugin.
    Needs a script definition format, output parsing conventions, and sandboxing considerations.
    The `CommandExecutor` trait already abstracts execution.
- [ ] Plugin compatibility status visible in Hosts UI per host
  - **Category**: UI / Plugins | **Impact**: Medium | **Effort**: Low-Medium
  - Once compatibility results are wired to the controller (above), add a column or badge to the
    Hosts page showing which plugins are compatible on each host.

### Plugin Type Settings: Follow-up Items

- [ ] Frontend: Plugin Type Settings UI via extension framework
  - **Category**: UI / Plugins | **Impact**: Medium | **Effort**: Medium
  - Build a Settings page section (or extension panel) that lists plugin types with type-level
    settings. Render the `type_settings_form_fields` schema from `PluginTypeInfo` as a structured
    form. Allow users to view, update, and reset type settings per plugin type.
- [ ] CLI: `plugin-type-settings list|show|update|reset` command group
  - **Category**: CLI / Plugins | **Impact**: Medium | **Effort**: Low
  - Add CLI commands wrapping the `GET/PUT/DELETE /api/v1/plugin-type-settings` REST API.
    `list` shows all plugin types with active type settings. `show <plugin_type>` displays the
    current settings. `update <plugin_type> --config '{...}'` upserts. `reset <plugin_type>`
    deletes the tenant-level override.
- [x] Cleanup: simplify plugin config discovery fields
  - **Category**: Plugins / Tech Debt | **Impact**: Low | **Effort**: Low
  - All plugins always emit `DiscoveryTarget` values. Per-plugin-type settings (e.g.
    `discovery_filter`, `package_type`) are configured as tenant-level type settings
    via `type_settings_form_schema()`.

______________________________________________________________________

## Phase 2: Core Features

Main functionality that delivers the core value proposition.

### Completed

- **Version Detection (Agent-Side)** — GitHub releases, APT, Homebrew, npm, Docker, Proxmox Helper
  Scripts plugins all implemented. Periodic inventory scanning via scheduler. Error reporting
  per-item.
- **Version Checking (Controller-Side)** — All plugin-specific checking implemented (GitHub,
  APT/Homebrew/npm, Docker registry with semver + digest, Proxmox). Version comparison logic, API
  rate-limit handling.
- **Plugin Implementations** — GitHub releases (assets, release notes, pre-releases), Docker
  (semver tag filtering, OCI auth, multi-registry), Proxmox Helper Scripts (detection, update,
  discovery).
- **Update Execution** — State machine (Pending → In Progress → Completed/Failed), progress
  reporting (UpdateOutput streaming), update logging, timeout handling.
- **Scheduling System** — Cron expressions, schedule persistence, background task runner, shared
  `uptrakit-scheduler-engine` crate, external scheduler binary with service enrollment, embedded
  scheduler feature, credential delivery, shared NATS crate.
- **Embedded Services Infrastructure** — Unified `EmbeddedServiceHost` in the controller for
  embedding any service (agent, mqtt, scheduler) in-process via mpsc channels. Coexistence
  policies (`YieldAlways`, `NeverYield`), auto-provisioning of `system_services` and tenant
  `services` DB records, `EmbeddedServiceNotifier` trait for WS handler hooks,
  `ServiceTransport` trait for transport-agnostic message sending, message processor bridge
  for dispatching embedded service messages through the standard handler pipeline. The embedded
  scheduler and embedded agent have been refactored to use this infrastructure.
- **Embedded Agent** — Optional in-process agent (`embedded-agent` feature) for single-tenant
  deployments. Reuses `uptrakit-agent-core` for version checks, software discovery, and update
  execution. Yields to an external agent on the same host (machine-ID-based coexistence).
  Supports interactive updates when the `interactive` feature is also enabled.
- **Software Autodiscovery** — Event-driven trigger, `discovery_state` field, ignore table,
  `DiscoverSoftware` / `DiscoveryResults` wire messages, agent-core shared implementation,
  plugin-driven `DiscoveryTarget`, REST API (approve, discover, ignore), version check excludes
  pending items.

### Pending

- [ ] Version caching with TTL
  - **Category**: Core / Performance | **Impact**: Medium | **Effort**: Low-Medium
  - Cache `fetch_releases` responses with a configurable TTL to reduce API calls and speed up
    repeated checks. Per-host latest version is already on `host_software_items`; this adds
    upstream response caching.
- [ ] Channel support (stable, beta, nightly)
  - **Category**: Core | **Impact**: Medium | **Effort**: Medium-High
  - Allow software items to track a specific release channel. Affects version comparison,
    `fetch_releases` filtering, and UI/API.
- [ ] Retry logic for failed version checks
  - **Category**: Core / Reliability | **Impact**: Medium | **Effort**: Low-Medium
  - Automatic retry with exponential backoff when `fetch_releases` or `detect_version` fails.
    The `uptrakit-backoff` crate already exists. Currently relies on the next scheduled run.
- [ ] Proxmox Helper Scripts: script integrity verification
  - **Category**: Plugins / Security | **Impact**: Medium | **Effort**: Medium
  - Verify script checksums or signatures before execution to prevent tampered update scripts.
- [ ] Update rollback triggers
  - **Category**: Core | **Impact**: High | **Effort**: High
  - Define conditions under which an update is automatically rolled back (exit code, health check
    failure). Requires snapshot/rollback mechanism (see Phase 6).
- [ ] Update failure retries
  - **Category**: Core / Reliability | **Impact**: Medium | **Effort**: Low-Medium
  - Automatic retry for transient update failures with configurable max attempts. Currently
    requires manual re-trigger.
- [ ] Self-update: support updating agent and controller binaries
  - **Category**: Core | **Impact**: High | **Effort**: High
  - Allow the system to update its own binaries. Needs careful orchestration to avoid downtime
    (graceful restart infrastructure already exists).

### Concurrency Control

Completed: per-host update locks (cross-table check, partial unique index, CAS promotion),
concurrent version check handling.

- [ ] Global concurrent update limits
  - **Category**: Core / Reliability | **Impact**: Medium | **Effort**: Medium
  - Cap the total number of simultaneous updates across all hosts to prevent resource exhaustion
    on the controller and network.
- [ ] Lock timeout handling
  - **Category**: Core / Reliability | **Impact**: Medium | **Effort**: Low-Medium
  - Automatically release stale update locks after a configurable timeout to prevent deadlocks
    from crashed agents or lost connections.
- [ ] Update queue management
  - **Category**: Core | **Impact**: Medium | **Effort**: Medium
  - Queue updates that exceed concurrency limits and dispatch them in order as slots free up.
- [ ] Priority queue for updates
  - **Category**: Core | **Impact**: Low-Medium | **Effort**: Medium
  - Allow security updates or user-flagged items to jump the queue ahead of routine updates.
- [ ] Resource-based throttling
  - **Category**: Core / Performance | **Impact**: Low-Medium | **Effort**: Medium
  - Throttle updates based on host CPU/memory/disk to avoid overloading targets during updates.

### User Convenience

Completed: reverse proxy support (Traefik, Caddy, Nginx, NPM, Envoy, HAProxy), SIGHUP agent/MQTT
exit, graceful restart (`SO_REUSEPORT`, `--takeover-from`, `ServerRestarting` message), embedded
frontend (`embed-frontend`).

- [ ] CA certificate fallback handling
  - **Category**: Security / Reliability | **Impact**: Medium | **Effort**: Medium
  - Graceful recovery when the agent's pinned CA certificate becomes invalid (e.g. file
    corruption, accidental deletion). Currently the agent cannot reconnect.
- [ ] Per-software-item schedule configuration
  - **Category**: Core / UI | **Impact**: Medium-High | **Effort**: Medium
  - Allow different check/update schedules per software item instead of only global defaults.
    Schema change (`scheduled_tasks` per-item override) + settings UI.
- [ ] SIGHUP controller reload
  - **Category**: Core / Operations | **Impact**: Medium-High | **Effort**: Medium-High
  - Reload all reloadable settings on SIGHUP without disconnecting agents. Requires careful
    state management for TLS, DB pools, and in-flight requests. Send a "please reconnect" message
    if TLS config changes.
- [ ] Certificate renewal via API must activate for new connections
  - **Category**: Security / Operations | **Impact**: Medium | **Effort**: Medium
  - `POST /api/v1/settings/renew-server-certificate` generates a new cert but the listening
    socket still serves the old one until restart. Hot-swap the TLS acceptor.

______________________________________________________________________

## Phase 3: User Interfaces

### Completed

- **Web API** — Full REST API (hosts, software items, version checks, updates, history, schedules,
  settings, plugin configs, autodiscovery, notifications, audit logs, services, system services,
  enrollment tokens). Auth, rate limiting, OpenAPI/Swagger, WebSocket for real-time updates.
- **CLI Tool** — Device auth login (RFC 8628-style), auth/token management, hosts, software-items,
  check, update, history, services, system-services, scheduler, settings, software-ignores,
  notifications. Table/JSON/YAML output, filtering, interactive mode,
  config/credentials storage.
- **MQTT/Home Assistant Integration** — Separate MQTT binary with multi-instance lease-based
  tenant distribution, HA `update` entity discovery, version sensors, update command handling,
  configurable topics, connection resilience, auth support (encrypted at rest). Unified service
  enrollment and management.
- **Batch Actions (Group Operations)** — `POST /api/v1/{resource}/batch` endpoints for services,
  system services, software items, hosts, software ignores, and plugin configs.
  Partial-success semantics (max 100 IDs). OpenAPI client methods and CLI `batch` subcommands for
  all resources. Extension framework `batch_action` flag on `ActionDef`.
- **Host Tags** — Tenant-scoped tags for grouping and categorizing hosts. Full CRUD at
  `/api/v1/host-tags`, replace-all assignment via `PUT /api/v1/hosts/{id}/tags`, batch delete,
  auto-generated colors from curated palette, real-time SSE events, OpenAPI client, CLI
  `host-tags` subcommands, and frontend tag management page with inline tag badges on host lists.

### Web API — Pending

- [ ] System status endpoint
  - **Category**: API | **Impact**: Medium-High | **Effort**: Low
  - `GET /api/v1/status` returning DB health, connected agent/service counts, CA certificate
    expiry, scheduler status. Extends the existing `/healthz` and `/readyz` with richer data.

### Web UI — Pending

- [ ] Dashboard view
  - **Category**: UI | **Impact**: High | **Effort**: Medium
  - System overview page: host count, pending updates, recent activity feed, alert/notification
    display. Data is available via existing APIs; this is primarily frontend work.
- [ ] Host list: sortable/filterable columns
  - **Category**: UI | **Impact**: Medium | **Effort**: Low-Medium
  - The host list has pagination but no column sorting or advanced filtering. Add client-side or
    server-side sort by name, last-seen, OS, update count.
- [ ] Software list: grouped view by host or plugin
  - **Category**: UI | **Impact**: Medium | **Effort**: Medium
  - Software items show a plugin column but lack a grouped/tree view. Add collapsible groups by
    host or by plugin type.
- [ ] Schedule configuration UI: visual schedule builder
  - **Category**: UI | **Impact**: Medium | **Effort**: Medium
  - Replace the text-based cron input with a visual picker (day-of-week, hour, interval). Enable/
    disable and manual trigger already work.
- [x] ~~Settings UI: user management~~ — Backend complete: user management API with granular
  RBAC (32 permissions, 8 roles, 5 presets), lockout prevention, user activation/deactivation.
  Frontend page pending.
  - **Category**: UI / Security | **Impact**: Medium-High | **Effort**: Medium
  - Backend user management API is complete. Frontend page for managing users, roles, and presets
    is pending.
- [x] Notification configuration UI
  - **Category**: UI | **Impact**: Medium-High | **Effort**: Medium
  - Per-transport channel management tabs via extension framework (webhook, telegram, email).
    Built-in notification rules and delivery log tabs. SMTP configuration via extension
    pre-load action.
- [ ] Batch update UI
  - **Category**: UI | **Impact**: Medium | **Effort**: Medium
  - Visual interface for creating and monitoring host-wide and item-wide batch updates. SSE
    streaming and CLI support already exist.

### CLI — Pending

- [ ] `uptrakit status` command
  - **Category**: CLI | **Impact**: Medium | **Effort**: Low
  - Quick system health summary in the terminal. Depends on the system status API endpoint.
- [ ] `uptrakit agent` subcommand group
  - **Category**: CLI | **Impact**: Medium | **Effort**: Medium
  - Commands proxied to a specific agent, plus `uptrakit agent install` for local agent setup.
- [ ] `uptrakit controller` subcommand group
  - **Category**: CLI | **Impact**: Medium | **Effort**: Medium
  - Commands proxied to the controller (diagnostics, config dump), plus install helper.

______________________________________________________________________

## Phase 4: SSH Agent

Completed: `crates/core/agent-ssh/` crate, wire protocol reuse, multi-host management, CLI host
management (`host add/list/show/update/remove/bootstrap`), `CommandExecutor` trait abstraction,
password + key-based auth (Ed25519/RSA/ECDSA), SSH agent forwarding, TOFU host key verification,
connection pooling (300s TTL), custom SSH ports, remote plugin execution (`SshCommandExecutor`),
update streaming, per-host update locking, least-privilege user creation (bootstrap), shell
injection prevention (`shell_escape()`), encrypted SSH key storage, UI extension `ssh-agent.hosts`
(host management via extensions framework), ECIES sealed-box encryption for sensitive extension
parameters, dynamic CLI subcommands for extensions (manifest-driven argument parsing),
per-command sudoers generation (`host bootstrap` / `host sync` grant only `NOPASSWD` access for
specific commands declared by registered plugins via `required_sudo_commands()` — not blanket
`ALL`), embedded mode (`embedded-ssh-agent` controller feature: runs inside the controller
process via `EmbeddedServiceHost::add()`, `YieldOnSameAppName` coexistence, transport-generic
`ServiceTransport` trait for dual WebSocket/in-process operation, lib+bin crate split).

### Pending

- [ ] Jump host / bastion support
  - **Category**: SSH / Networking | **Impact**: Medium | **Effort**: Medium-High
  - Allow reaching hosts behind NAT or firewalls via an intermediate bastion host. Needs
    `ProxyJump`-style config in `russh`.
- [ ] Timeout and kill long-running remote commands
  - **Category**: SSH / Reliability | **Impact**: Medium | **Effort**: Medium
  - Enforce per-command timeouts over SSH and send SIGTERM/SIGKILL to the remote process.
    Currently commands can run indefinitely.
- [ ] Handle connection drops mid-command gracefully
  - **Category**: SSH / Reliability | **Impact**: Medium | **Effort**: Medium
  - Detect SSH disconnects during update execution, report failure to controller, and avoid
    leaving orphan processes on the remote host.
- [ ] Plugin-level SSH compatibility flag
  - **Category**: Plugins / SSH | **Impact**: Low-Medium | **Effort**: Low
  - Add a `PluginCapability::SshCompatible` flag so the registry can filter plugins that are
    safe to run over SSH.
- [x] ~~Sudo allowlist for SSH-managed hosts~~ — Per-command sudoers entries are now generated by
  `host bootstrap` / `host sync`. Only commands declared by registered plugins via
  `required_sudo_commands()` are granted `NOPASSWD` access — not blanket `ALL`.
  - **Category**: SSH / Security | **Impact**: High | **Effort**: Medium
  - Bootstrap currently creates `NOPASSWD: ALL`. Restrict to specific update commands only
    (matching the regular agent sudoers policy).
- [ ] SSH session audit trail
  - **Category**: SSH / Security | **Impact**: Medium | **Effort**: Medium
  - Log every SSH session (host, user, command, timestamp, exit code) without capturing secrets.
    Feed into the existing audit log infrastructure.
- [ ] Concurrent SSH session limits
  - **Category**: SSH / Reliability | **Impact**: Medium | **Effort**: Low-Medium
  - Cap concurrent SSH sessions per host and globally to prevent resource exhaustion on both the
    SSH agent and remote hosts.
- [ ] Host key fingerprint display in controller UI
  - **Category**: SSH / UI | **Impact**: Low-Medium | **Effort**: Low
  - Show (read-only) host key fingerprints in the controller web UI for verification.
- [ ] Config file format for SSH target hosts
  - **Category**: SSH / Operations | **Impact**: Medium | **Effort**: Medium
  - Declarative config file (TOML/YAML) defining target hosts, SSH credentials, and per-host
    overrides, as an alternative to CLI `host add`.
- [ ] CLI flags for SSH overrides
  - **Category**: SSH / CLI | **Impact**: Low-Medium | **Effort**: Low
  - Override key path, known_hosts path, and concurrency limits from the command line.
- [ ] SSH health checks
  - **Category**: SSH / Reliability | **Impact**: Medium | **Effort**: Low-Medium
  - Periodic SSH connectivity test to each managed host, with status reported to the controller.
    Connection pool already exists; just needs a scheduled ping + status field.
- [ ] SSH-managed hosts in controller UI and API
  - **Category**: SSH / UI | **Impact**: Medium | **Effort**: Medium
  - Show SSH-managed hosts alongside regular agents with a transport type indicator (badge/icon).
- [ ] Frontend ECIES encryption for sensitive extension form fields
  - **Category**: Extensions / Security | **Impact**: High | **Effort**: Medium
  - Client-side ECIES encryption in the frontend for fields marked `sensitive: true` in extension
    manifests. Currently only the CLI path can leverage E2E encryption.

______________________________________________________________________

## Phase 5: Plugin Ecosystem

Completed: APT, Docker, GitHub, GitLab, Forgejo, npm, Homebrew, Pacman, APK (Alpine), pkg
(FreeBSD), Snap, cargo-install, MAS (Mac App Store), Generic Shell plugins, role-based multi-plugin
assignment, plugin development guide, plugin system architecture docs.

### Additional Plugins

#### Package Manager Plugins

Ranked by potential audience coverage and likeliness of using uptrakit for update tracking.

- [ ] DNF plugin (Fedora, RHEL, Rocky Linux, AlmaLinux)
  - **Category**: Plugins | **Impact**: High | **Effort**: Medium
  - Package manager plugin for Red Hat-family Linux — the dominant distribution family in
    enterprise environments. Detect via `dnf list installed`, fetch via `dnf check-update` after
    `dnf makecache` (`RefreshPackageIndex`), update via `sudo dnf upgrade -y`. Batch-capable
    (space-separated package lists). Reboot detection (`needs-restarting -r` or
    `/var/run/reboot-required`) can be handled by a `hook_shell` lifecycle plugin.
    Discovery via `dnf list installed`. DetectHostCompatibility: `which dnf`.
- [x] ~~Pacman plugin (Arch Linux, Manjaro, EndeavourOS)~~ — Implemented as
  `uptrakit-plugin-package-manager-pacman`.
  - **Category**: Plugins | **Impact**: High | **Effort**: Medium
  - Package manager plugin for Arch-family rolling-release Linux. Rolling releases mean constant
    version churn — a strong use case for uptrakit tracking. Detect via `pacman -Qi`, fetch via
    `pacman -Si` after `sudo pacman -Sy` (`RefreshPackageIndex`), update via
    `sudo pacman -S --noconfirm`. Discovery via `pacman -Qe` (explicitly installed packages).
    DetectHostCompatibility: `which pacman`.
- [x] ~~APK plugin (Alpine Linux)~~ — Implemented as `uptrakit-plugin-package-manager-apk`.
  - **Category**: Plugins | **Impact**: High | **Effort**: Low-Medium
  - Package manager plugin for Alpine Linux, pervasive in LXC containers on Proxmox hosts — high
    synergy with the existing Proxmox infrastructure plugin. Detect via `apk list -I`, fetch via
    `apk list -u` after `sudo apk update` (`RefreshPackageIndex`), update via
    `sudo apk add <pkg>=<version>`. Batch-capable (space-separated names). Discovery via
    `apk info -v`. DetectHostCompatibility: `which apk`.
- [ ] pip plugin (Python packages)
  - **Category**: Plugins | **Impact**: Medium-High | **Effort**: Medium
  - Tracks Python tools installed globally or in `~/.local/bin` (`certbot`, `ansible`, `awscli`,
    etc.) — extremely common on self-hosted servers. `ControllerSideFetchReleases` via PyPI JSON
    API (`pypi.org/pypi/<pkg>/json`), no auth required. Detect via `pip show`, batch detect via
    `pip list --format=json` (filter in memory). Discovery via `pip list --not-required`.
    DetectHostCompatibility: `which pip` or `which pip3`.
- [x] ~~pkg plugin (FreeBSD)~~ — Implemented as `uptrakit-plugin-package-manager-pkg`.
  - **Category**: Plugins | **Impact**: Medium | **Effort**: Medium
  - FreeBSD package manager. FreeBSD powers dedicated self-hosted appliances (TrueNAS CORE,
    OPNsense-derived setups). Detect via `pkg info`, fetch via `pkg rquery '%v'` after
    `sudo pkg update` (`RefreshPackageIndex`), update via `sudo pkg install -y`. Discovery via
    `pkg query '%n %v' -a`. DetectHostCompatibility: `which pkg` combined with `pkg -N`
    (bootstrapped check).
- [x] ~~Snap plugin~~ — Implemented as `uptrakit-plugin-package-manager-snap`.
  - **Category**: Plugins | **Impact**: Medium | **Effort**: Medium
  - Snap packages on Ubuntu servers. Most useful when snap auto-update is disabled or for auditing
    the installed channel/version. Channel-based versioning (stable, candidate, beta, edge) —
    exact version pinning is unsupported by snap design; updates refresh to the latest on the
    tracked channel via `sudo snap refresh <snap>`. Detect via `snap list`, batch detect from a
    single `snap list` call (filter in memory). Discovery via `snap list`. DetectHostCompatibility:
    `which snap`.
- [ ] Flatpak plugin
  - **Category**: Plugins | **Impact**: Medium | **Effort**: Medium
  - Flatpak apps on managed Linux desktops and kiosk systems. App IDs use reverse-DNS notation
    (`com.example.App`) — validate identifiers accordingly. Detect via
    `flatpak info --show-metadata`, fetch via `flatpak remote-info`. Batch detect from a single
    `flatpak list --app` call (filter in memory). Discovery via `flatpak list --app`.
    DetectHostCompatibility: `which flatpak`.
- [ ] gem plugin (Ruby gems)
  - **Category**: Plugins | **Impact**: Medium | **Effort**: Medium
  - System Ruby gems for self-hosted apps (Redmine, Mastodon, Jekyll). `ControllerSideFetchReleases`
    via RubyGems.org REST API (`rubygems.org/api/v1/gems/<name>.json`), mirroring the npm plugin
    pattern. Detect via `gem list`, update via `sudo gem install <name>:<version>`. Batch detect
    from a single `gem list --local` call (filter in memory). DetectHostCompatibility: `which gem`.
- [ ] Zypper plugin (openSUSE, SUSE Enterprise)
  - **Category**: Plugins | **Impact**: Medium | **Effort**: Medium
  - Package manager for openSUSE Leap/Tumbleweed and SUSE Linux Enterprise. SUSE environments have
    strong patch-compliance culture that aligns with uptrakit's value proposition. Detect via
    `zypper info`, fetch via `zypper -n list-updates` after `sudo zypper refresh`
    (`RefreshPackageIndex`), update via `sudo zypper install -y`. Discovery via
    `zypper -n search -i`. DetectHostCompatibility: `which zypper`.
- [x] ~~cargo-install plugin (Rust binaries)~~ — Implemented as
  `uptrakit-plugin-package-manager-cargo`.
  - **Category**: Plugins | **Impact**: Low-Medium | **Effort**: Medium
  - Tracks binaries installed via `cargo install` (`bat`, `ripgrep`, `fd`, `bottom`, etc.). Reads
    `~/.cargo/.crates2.json` for installed package metadata — no external command needed for
    detection. `ControllerSideFetchReleases` via crates.io REST API; must include the required
    `User-Agent` header per crates.io policy. Update via
    `cargo install <name> --version <ver> --locked`. DetectHostCompatibility: `which cargo`.
- [x] ~~MAS plugin (Mac App Store)~~ — Implemented as `uptrakit-plugin-package-manager-mas`.
  - **Category**: Plugins | **Impact**: Low-Medium | **Effort**: Low-Medium
  - Mac App Store package manager plugin for macOS. Tracks apps installed via `mas` CLI. Detect
    via `mas list`, update via `mas upgrade <id>`. DetectHostCompatibility: `which mas`.

#### Other Plugins

- [x] ~~Custom script plugin~~ — Implemented as `uptrakit-plugin-generic-shell` (same as Phase 1
  follow-up item above).
  - **Category**: Plugins | **Impact**: High | **Effort**: Medium
  - "Run arbitrary commands" plugin type with user-defined scripts for version detection, checking,
    and update execution. Needs script definition format, output parsing, and sandboxing.
- [ ] AppImage plugin
  - **Category**: Plugins | **Impact**: Low | **Effort**: Medium-High
  - AppImage version tracking. No standard package manager; needs custom detection and update
    mechanism (e.g. AppImageUpdate or GitHub releases).
- [ ] Chocolatey plugin (Windows)
  - **Category**: Plugins | **Impact**: Medium | **Effort**: Medium
  - Windows package manager plugin using `choco list` / `choco upgrade`.

#### Enhancement Plugins

- [x] ~~Dashboard Icons plugin~~ — Implemented as
  `uptrakit-plugin-enhancement-dashboard-icons`.
  - **Category**: Plugins / Enhancement | **Impact**: Medium | **Effort**: Medium
  - Automatic icon URL assignment for software items using the [Dashboard Icons](https://dashboardicons.com/)
    community project. Pre-caches icon slugs via GitHub Trees API with 6-hour refresh.
    Per-tenant setting (disabled by default). Hooks into both manual creation and
    autodiscovery paths via `SoftwareItemLifecyclePlugin` subtrait.

### Plugin Framework

- [ ] Plugin testing framework
  - **Category**: Plugins / Testing | **Impact**: Medium | **Effort**: Medium
  - Shared test harness with mock version sources, mock `CommandExecutor`, and assertion helpers
    for plugin authors.
- [ ] Plugin validation tools
  - **Category**: Plugins / DX | **Impact**: Low-Medium | **Effort**: Medium
  - CLI tool or `cargo test` integration that validates a plugin's trait implementation,
    capability declarations, and config schema.
- [ ] Plugin hot-reloading
  - **Category**: Plugins / Architecture | **Impact**: Low | **Effort**: High
  - Dynamic plugin loading without agent/controller restart. Requires ABI-stable plugin interface
    (e.g. `libloading` or WASM).
- [ ] Plugin marketplace/registry concept
  - **Category**: Plugins / Architecture | **Impact**: Low | **Effort**: High
  - Central registry for discovering and installing community plugins. Needs versioning, signing,
    and distribution infrastructure.
- [ ] Plugin versioning
  - **Category**: Plugins / Architecture | **Impact**: Low-Medium | **Effort**: Medium
  - Track plugin versions independently from the main binary. Enables independent plugin updates
    and compatibility matrices.
- [ ] Plugin examples and templates
  - **Category**: Plugins / Docs | **Impact**: Medium | **Effort**: Low-Medium
  - Simple plugin template (single detect/check/update) and complex plugin example
    (multi-capability, config schema, SSH-compatible).
- [ ] Plugin API reference docs
  - **Category**: Plugins / Docs | **Impact**: Medium | **Effort**: Low-Medium
  - Generated API reference for the Plugin trait, `PluginCapability`, `CommandExecutor`, and
    registry macros.
- [ ] Plugin troubleshooting guide
  - **Category**: Plugins / Docs | **Impact**: Low-Medium | **Effort**: Low
  - Common issues (capability mismatch, execution site, SSH incompatibility) and solutions.

______________________________________________________________________

## Phase 6: Advanced Features

### Multi-Channel Support

- [ ] Release channel abstraction
  - **Category**: Core / Architecture | **Impact**: Medium | **Effort**: Medium-High
  - Stable, beta, nightly, and custom channels. Per-software-item channel selection, channel-aware
    version checking, channel switching rules, migration workflows, and configuration UI.

### Rollback Capabilities

- [ ] Rollback mechanism
  - **Category**: Core | **Impact**: High | **Effort**: High
  - Snapshot creation before updates, configurable rollback trigger conditions (exit code, health
    check), rollback execution via plugin trait method, history tracking, UI, and automatic
    rollback on failure.
- [ ] Proxmox VE pre-update snapshots and rollback (Phase 2)
  - **Category**: Plugins / Infrastructure | **Impact**: High | **Effort**: High
  - Extend the Proxmox VE plugin (`infrastructure_proxmox`) to create VM/CT snapshots before
    updates and roll back on failure. Requires new Proxmox API client methods (create snapshot,
    list snapshots, rollback snapshot), snapshot lifecycle management, integration with the
    update pipeline, and UI controls. Phase 1 (discovery + manual matching) is complete.

### Interactive Updates

Completed: bidirectional terminal I/O for update sessions via PTY allocation, stdin forwarding,
attention detection, dedicated WebSocket endpoint, single-writer session management,
`InteractiveUpdates` capability, `StdinAttention` notification event, CLI `--interactive` flag.
Feature-gated behind the `interactive` Cargo feature.

### Update Batching & Orchestration

Completed: batch update system (host-wide + item-wide), update category classification, batch
progress tracking (SSE), batch notification events, unified software tracking.

- [ ] Update dependencies
  - **Category**: Core / Orchestration | **Impact**: Medium | **Effort**: High
  - Define ordering constraints: update A must complete before update B starts. Support cross-host
    dependencies for coordinated rollouts.
- [ ] Canary deployment patterns
  - **Category**: Core / Orchestration | **Impact**: Medium | **Effort**: High
  - Roll out to a small subset of hosts first, verify health, then proceed to the rest. Needs
    health check integration and automatic promotion/rollback.
- [ ] Rolling update strategies
  - **Category**: Core / Orchestration | **Impact**: Medium | **Effort**: Medium-High
  - N-at-a-time parallel execution with configurable failure thresholds. Currently batches run
    sequentially per-host.

### Notification System

Completed: channel-agnostic dispatcher, webhook + Telegram + email plugins, scope-based rule
matching, notification history, actionable notifications, REST API, CLI, OpenAPI client.
Plugin architecture under `crates/plugins/notifications/` with `declare_plugin!` + `NotificationTransport` role trait.

- [ ] Slack notification plugin
  - **Category**: Notifications | **Impact**: Medium-High | **Effort**: Medium
  - Slack integration via `slack-morphism` or Incoming Webhooks. Create a new crate under
    `crates/plugins/notifications/slack/` implementing `declare_plugin!` + `NotificationTransport`.
- [ ] Discord notification plugin
  - **Category**: Notifications | **Impact**: Medium | **Effort**: Medium
  - Discord bot or webhook integration via `twilight-http` or simple HTTP POST.
- [ ] Pushover notification plugin
  - **Category**: Notifications | **Impact**: Low-Medium | **Effort**: Low
  - Simple HTTP POST to Pushover API. Minimal implementation effort.
- [ ] Gotify notification plugin
  - **Category**: Notifications | **Impact**: Low-Medium | **Effort**: Low
  - Simple HTTP POST to self-hosted Gotify server. Popular in the self-hosted community.
- [ ] ntfy notification plugin
  - **Category**: Notifications | **Impact**: Medium | **Effort**: Low
  - Simple HTTP POST to ntfy.sh or self-hosted ntfy. Very popular self-hosted choice, minimal
    code needed.
- [ ] Matrix notification plugin
  - **Category**: Notifications | **Impact**: Low-Medium | **Effort**: Medium
  - Matrix room notifications via the Matrix client-server API.
- [ ] Microsoft Teams notification plugin
  - **Category**: Notifications | **Impact**: Medium | **Effort**: Medium
  - Teams Incoming Webhook or Workflow integration.
- [ ] PagerDuty notification plugin
  - **Category**: Notifications | **Impact**: Low-Medium | **Effort**: Medium
  - PagerDuty Events API v2 integration for incident-based alerting on update failures.
- [ ] Notification callback rate limiting
  - **Category**: Notifications / Security | **Impact**: Low-Medium | **Effort**: Low
  - Rate-limit incoming notification callbacks on the generic
    `POST /api/v1/notifications/callback/{channel_type}/{channel_id}` endpoint to prevent abuse.
    The callback handler exists but has no per-user or global throttle.

### Update Windows

- [ ] Maintenance window concept
  - **Category**: Core / Operations | **Impact**: Medium-High | **Effort**: Medium-High
  - Time-based windows, day-of-week restrictions, blackout periods, timezone handling. Updates
    triggered outside a window get queued until it opens. Window validation for scheduled updates.
    Configuration UI.

______________________________________________________________________

## Phase 7: Security Enhancements

### Completed

- **mTLS** — Automated certificate issuance (CSR → CA signing → delivery), CRL + OCSP revocation,
  expiration monitoring + automated renewal.
- **CA Management** — Rotation automation (6-month window, cron-based), multi-CA validation
  (active + previous CA bundle, gradual migration).
- **Agent Authentication** — Certificate-based identity extraction/mapping/persistence, secure
  enrollment flow (token generation, expiration, approval workflow).
- **Audit Logging Infrastructure** — HTTP request logging, pluggable backends (DB, journald, noop),
  multiplex, fire-and-forget dispatcher, global/per-tenant filter modes, separate audit DB, immutable
  storage, 90-day retention, read API.
- **Additional Security** — Rate limiting, brute force protection, security headers, input
  validation, secrets management (AES-256-GCM, envelope encryption v3, master key rotation).

### CA Management — Pending

- [ ] CA certificate backup and recovery
  - **Category**: Security / Operations | **Impact**: High | **Effort**: Medium-High
  - Automated CA backup to a secure location with documented recovery procedures. Critical for
    disaster recovery — losing the CA key means all agent certificates become unverifiable.
- [ ] Rollback capability for failed CA rotations
  - **Category**: Security / Reliability | **Impact**: Medium | **Effort**: Medium
  - If rotation fails mid-way (e.g. agents can't reach the new CA bundle), automatically revert
    to the previous CA as the active signer.

### Agent Authorization — Pending

- [ ] Agent authorization policies
  - **Category**: Security | **Impact**: Medium-High | **Effort**: Medium-High
  - Role-based access control for agents (e.g. "this agent can only update packages X, Y, Z"),
    per-agent permissions, and policy enforcement points in the wire protocol handler.

### Audit Logging — Pending

- [ ] Authentication event logging
  - **Category**: Security / Audit | **Impact**: High | **Effort**: Medium
  - Log failed logins, failed API token/JWT attempts, token refresh failures, and registration
    attempts. These occur outside `require_auth` middleware and need explicit audit calls in
    `routes/auth.rs` and `middleware/require_auth.rs`.
- [ ] OIDC auth flow logging
  - **Category**: Security / Audit | **Impact**: Medium | **Effort**: Medium
  - Log OIDC authorize initiation, callback completion, account linking, token exchange,
    registration, and auth failures in `routes/oidc_auth.rs`.
- [ ] Device auth flow logging
  - **Category**: Security / Audit | **Impact**: Medium | **Effort**: Low-Medium
  - Log device code creation, polling, approval/denial, expiration in `routes/device_auth.rs`.
- [ ] WebSocket service operation logging
  - **Category**: Security / Audit | **Impact**: Medium-High | **Effort**: Medium-High
  - Log service WS connections, enrollment lifecycle, certificate operations, discovery reporting,
    version check processing, update lifecycle, batch updates, and certificate renewal requests.
    These bypass HTTP audit middleware entirely.
- [ ] MQTT operation logging
  - **Category**: Security / Audit | **Impact**: Medium | **Effort**: Medium
  - Log MQTT registration, tenant assignment/release, heartbeats, and MQTT-triggered updates
    in `routes/service_ws/handler/mqtt.rs`.
- [ ] System service operation logging
  - **Category**: Security / Audit | **Impact**: Medium | **Effort**: Low-Medium
  - Log system service enrollment completion and credential delivery in
    `routes/service_ws/handler/mod.rs`.
- [ ] CA and PKI operation logging
  - **Category**: Security / Audit | **Impact**: High | **Effort**: Medium
  - Log CA rotation triggering/broadcast, certificate issuance, revocation, CRL generation, and
    public PKI endpoint access.
- [ ] Scheduler background task logging
  - **Category**: Security / Audit | **Impact**: Medium | **Effort**: Medium
  - Log execution of auth cleanup, cert checks, CRL renewal, stale lease cleanup, audit log
    retention, version detection, and release fetch.
- [ ] Semantic operation logging
  - **Category**: Security / Audit | **Impact**: High | **Effort**: Medium-High
  - Beyond raw HTTP request logging: log the semantic meaning of operations (settings changes with
    old/new values, user CRUD, OIDC provider management, enrollment token lifecycle, notification
    channel/rule CRUD, service approval/rejection, software/plugin management, batch
    update initiation).
- [ ] Public callback endpoint logging
  - **Category**: Security / Audit | **Impact**: Low-Medium | **Effort**: Low
  - Log notification callback execution (action token verification) on the generic
    `POST /api/v1/notifications/callback/{channel_type}/{channel_id}` endpoint in
    `routes/notifications.rs`.
- [ ] Tamper-evident log storage
  - **Category**: Security / Audit | **Impact**: Medium | **Effort**: High
  - Log signing and integrity verification to detect tampering of audit records.
- [ ] Per-tenant retention overrides
  - **Category**: Security / Audit | **Impact**: Low-Medium | **Effort**: Low
  - The `audit_log.retention_days` setting key is defined but the cleanup executor doesn't read
    per-tenant overrides yet.
- [ ] Log archival and search
  - **Category**: Security / Audit | **Impact**: Medium | **Effort**: Medium-High
  - Archive old logs to cold storage and add full-text search/analysis capabilities.

### Additional Security — Pending

- [ ] Credential rotation
  - **Category**: Security | **Impact**: Medium | **Effort**: Medium
  - Automated rotation of stored credentials (MQTT passwords, SMTP credentials, plugin API keys)
    with zero-downtime switchover.
- [ ] Vault integration
  - **Category**: Security | **Impact**: Medium | **Effort**: Medium-High
  - Optional HashiCorp Vault (or compatible) backend for secrets storage instead of local
    AES-256-GCM encryption.
- [ ] Security scanning in CI/CD
  - **Category**: Security / CI | **Impact**: Medium-High | **Effort**: Medium
  - Add `cargo-audit` for dependency vulnerability scanning, SAST tools for static analysis, and
    container image scanning to the CI pipeline.

______________________________________________________________________

## Phase 8: Quality & Reliability

### Completed

- **Integration Tests** — REST API, WebSocket, system integration (Docker/testcontainers),
  OCSP/CRL revocation checking with reverse proxies.
- **Error Recovery** — Connection retry with exponential backoff, automatic transient error
  recovery.
- **Reliability** — Health check endpoints (`/healthz` + `/readyz` with DB and CA checks),
  graceful shutdown (agent waits for in-flight updates).

### Testing — Pending

- [ ] Expand unit test coverage to 80%+
  - **Category**: Testing | **Impact**: High | **Effort**: High
  - Target 80%+ coverage for core logic. Focus on error handling paths and edge cases.
- [ ] Integration tests: agent-controller message exchange
  - **Category**: Testing | **Impact**: Medium-High | **Effort**: Medium
  - Test full message exchange beyond enrollment (version checks, update commands, discovery
    results).
- [x] Integration tests: database operations
  - **Category**: Testing | **Impact**: Medium | **Effort**: Medium
  - Multi-DB integration tests (SQLite, PostgreSQL) using testcontainers. 61 tests
    per backend covering auth flows, CRUD operations, batch actions, and error cases.
- [ ] Integration tests: plugin implementations
  - **Category**: Testing | **Impact**: Medium | **Effort**: Medium
  - Test each plugin against real or mocked package managers / registries.
- [ ] Integration tests: end-to-end update workflows
  - **Category**: Testing | **Impact**: High | **Effort**: Medium-High
  - Full update cycle from version detection through execution to completion, including batch
    updates and failure recovery.
- [ ] Load testing
  - **Category**: Testing / Performance | **Impact**: Medium | **Effort**: Medium-High
  - Many-agents, concurrent-updates, and high-frequency-checks scenarios to find bottlenecks.
- [ ] Chaos testing
  - **Category**: Testing / Reliability | **Impact**: Medium | **Effort**: High
  - Network failure, database failure, and agent crash scenarios to verify graceful degradation.
- [ ] Test fixtures and mocks
  - **Category**: Testing / DX | **Impact**: Medium | **Effort**: Medium
  - Shared mock plugins, mock version sources, and test data generators for use across crates.

### Error Recovery — Pending

- [ ] Graceful degradation for partial failures
  - **Category**: Reliability | **Impact**: Medium | **Effort**: Medium
  - Continue operating with reduced functionality when subsystems fail (e.g. notification
    delivery failure shouldn't block updates).
- [ ] Circuit breaker pattern for external services
  - **Category**: Reliability | **Impact**: Medium | **Effort**: Medium
  - Stop calling external APIs (GitHub, Docker Hub, package registries) after repeated failures,
    with automatic recovery after a cooldown.
- [ ] Idempotent operations
  - **Category**: Reliability / Architecture | **Impact**: Medium | **Effort**: Medium-High
  - Ensure all state-changing operations can be safely retried without side effects (important
    for HA and network partitions).
- [ ] Operation replay capabilities
  - **Category**: Reliability | **Impact**: Low-Medium | **Effort**: High
  - Record and replay failed operations for debugging and recovery.

### Performance Optimization — Pending

- [ ] Profile and optimize hot paths
  - **Category**: Performance | **Impact**: Medium | **Effort**: Medium
  - Use `perf` / `flamegraph` to identify and optimize CPU-intensive code paths.
- [ ] Caching strategies
  - **Category**: Performance | **Impact**: Medium | **Effort**: Medium
  - In-memory caches for frequently accessed data (settings, plugin configs, host metadata) with
    invalidation.
- [ ] Database query optimization
  - **Category**: Performance | **Impact**: Medium | **Effort**: Medium
  - Add missing indexes, optimize N+1 queries, tune connection pool sizes.
- [ ] Memory footprint reduction
  - **Category**: Performance | **Impact**: Low-Medium | **Effort**: Medium
  - Profile heap usage and reduce allocations in hot paths (wire message handling, SSE
    broadcasting).
- [ ] Agent-controller communication optimization
  - **Category**: Performance | **Impact**: Medium | **Effort**: Medium-High
  - Message batching and optional compression for high-frequency message paths.
- [ ] Performance monitoring
  - **Category**: Performance / Observability | **Impact**: Medium | **Effort**: Medium
  - Request timing, database query timing, and resource usage metrics exposed via an endpoint
    or logs.

### Reliability — Pending

- [ ] State recovery on restart
  - **Category**: Reliability | **Impact**: Medium | **Effort**: Medium
  - Recover in-progress updates, pending batches, and scheduled task state after a controller
    crash and restart.
- [ ] Data integrity checks
  - **Category**: Reliability | **Impact**: Medium | **Effort**: Medium
  - Periodic consistency checks (orphaned records, FK integrity, encryption health).
- [ ] Automatic backup and restore
  - **Category**: Reliability / Operations | **Impact**: Medium-High | **Effort**: Medium-High
  - Scheduled database backups with documented restore procedure. Critical for production
    deployments.

______________________________________________________________________

## Phase 9: Documentation & Operations

### Completed

- **API Documentation** — OpenAPI/Swagger spec, REST endpoint docs (150+), WebSocket/wire protocol
  docs (50KB), `uptrakit-openapi-client` typed client crate.
- **User Documentation** — CLI guide, MQTT/Home Assistant integration guide.
- **Security Documentation** — Certificate management guide (lifecycle, revocation), security best
  practices guide (deployment, network, secrets, audit logging).
- **Deployment Documentation** — Docker deployment guide.
- **Contributor Documentation** — CONTRIBUTING.md, development setup, architecture docs, testing
  strategy.

### Pending

- [ ] API request/response examples
  - **Category**: Docs / API | **Impact**: Medium | **Effort**: Low-Medium
  - Add curl examples for common API operations. Schemas are documented but practical examples
    are sparse.
- [ ] Getting started guide
  - **Category**: Docs / End-User | **Impact**: High | **Effort**: Low-Medium
  - Step-by-step guide from installation to first monitored software item. Critical for adoption.
- [ ] Installation guide
  - **Category**: Docs / End-User | **Impact**: High | **Effort**: Medium
  - Controller installation, agent installation, and configuration walkthrough for supported
    platforms.
- [ ] Web UI user guide
  - **Category**: Docs / End-User | **Impact**: Medium | **Effort**: Medium
  - Walkthrough of all UI pages with screenshots and common workflows.
- [ ] FAQ
  - **Category**: Docs / End-User | **Impact**: Medium | **Effort**: Low
  - Answers to common questions (supported plugins, HA setup, agent enrollment, MQTT setup).
- [ ] Troubleshooting guide
  - **Category**: Docs / End-User | **Impact**: Medium-High | **Effort**: Medium
  - Common issues and solutions: agent won't connect, certificate errors, update failures, MQTT
    discovery not working.
- [ ] Video tutorials
  - **Category**: Docs / End-User | **Impact**: Medium | **Effort**: High
  - Recorded walkthroughs for installation, configuration, and common workflows.
- [ ] mTLS setup guide
  - **Category**: Docs / Security | **Impact**: Medium-High | **Effort**: Medium
  - CA certificate generation, agent certificate provisioning, and renewal procedures for users
    bringing their own CA.
- [ ] CA rotation procedures documentation
  - **Category**: Docs / Security | **Impact**: Medium | **Effort**: Medium
  - Pre-rotation checklist, execution steps, post-rotation verification, rollback procedures.
- [ ] Certificate backup and recovery documentation
  - **Category**: Docs / Security | **Impact**: Medium | **Effort**: Low-Medium
  - Procedures for backing up and restoring CA certificates and keys.
- [ ] Agent authentication documentation
  - **Category**: Docs / Security | **Impact**: Medium | **Effort**: Medium
  - Enrollment workflow, identity management, and authorization policies explained for operators.
- [ ] Deployment guide
  - **Category**: Docs / Operations | **Impact**: High | **Effort**: Medium
  - System requirements, network requirements, security considerations for production deployments.
- [ ] Kubernetes deployment guide
  - **Category**: Docs / Operations | **Impact**: Medium-High | **Effort**: Medium
  - Helm chart or raw manifests, ConfigMap/Secret management, readiness/liveness probes, scaling.
- [ ] Systemd service setup documentation
  - **Category**: Docs / Operations | **Impact**: Medium-High | **Effort**: Low-Medium
  - Unit files for controller, agent, scheduler, MQTT service. Hardening options (`ProtectSystem`,
    `NoNewPrivileges`, etc.).
- [ ] Upgrade guide
  - **Category**: Docs / Operations | **Impact**: Medium-High | **Effort**: Medium
  - Version-to-version upgrade steps, migration notes, breaking change handling.
- [ ] Backup and restore guide
  - **Category**: Docs / Operations | **Impact**: Medium-High | **Effort**: Medium
  - Database backup, CA key backup, configuration backup, and tested restore procedures.
- [ ] PR template and guidelines
  - **Category**: Docs / Contributing | **Impact**: Low-Medium | **Effort**: Low
  - GitHub PR template with checklist (tests, docs, migration, clippy, deny).

______________________________________________________________________

## Phase 10: Project Infrastructure

### Completed

- **CI/CD** — Multi-platform builds (x86_64 + aarch64, Linux + macOS), cross-compilation, test
  execution (clippy, tests, integration tests, frontend checks), cargo-deny, Dependabot,
  release-please (changelog, semver, conventional commits).
- **Release Automation** — Multi-platform binary releases (7 binaries x 4 targets), GitHub artifact
  attestation, multi-arch container images, registry publishing, release notes.
- **Observability** — Log levels, correlation IDs (request-id middleware, TraceContext), tracing
  spans (`#[instrument]`), distributed tracing preparation.

### CI/CD — Pending

- [ ] Coverage reporting
  - **Category**: CI | **Impact**: Medium | **Effort**: Medium
  - Add `cargo-llvm-cov` or `tarpaulin` to CI and publish coverage reports (e.g. Codecov).
- [ ] cargo-audit integration
  - **Category**: CI / Security | **Impact**: Medium-High | **Effort**: Low
  - Add `cargo audit` to CI pipeline for known vulnerability detection in dependencies. Quick
    win alongside existing `cargo deny`.
- [ ] SAST tools
  - **Category**: CI / Security | **Impact**: Medium | **Effort**: Medium
  - Static application security testing (e.g. `cargo-geiger` for unsafe usage, custom lint rules).
- [ ] Container image scanning
  - **Category**: CI / Security | **Impact**: Medium | **Effort**: Low-Medium
  - Scan published container images for known vulnerabilities (Trivy, Grype, or similar).

### Release Automation — Pending

- [ ] Release checklist
  - **Category**: Operations | **Impact**: Low-Medium | **Effort**: Low
  - Documented pre-release and post-release checklist for manual verification steps.

### Monitoring & Observability — Pending

- [ ] JSON structured logging
  - **Category**: Observability | **Impact**: High | **Effort**: Low
  - Add a `--log-format json` flag to all binaries. Most of the tracing infra is already in place;
    just needs a `tracing-subscriber` JSON layer toggle. Critical for log aggregation (ELK, Loki).
- [ ] Prometheus metrics
  - **Category**: Observability | **Impact**: Medium-High | **Effort**: Medium
  - Expose Prometheus-compatible metrics endpoint (`/metrics`): request latencies, update counts,
    agent connection gauge, error rates. Use `metrics` + `metrics-exporter-prometheus`.
- [ ] OpenTelemetry integration
  - **Category**: Observability | **Impact**: Medium-High | **Effort**: Medium
  - Add `tracing-opentelemetry` exporter layer for distributed tracing. All spans and instruments
    are already in place; just wire up the subscriber layer and OTLP exporter.
- [ ] Monitoring dashboards
  - **Category**: Observability | **Impact**: Medium | **Effort**: Medium
  - Grafana dashboard templates for system health, performance, and security metrics.
- [ ] Alerting rules
  - **Category**: Observability | **Impact**: Medium | **Effort**: Medium
  - Prometheus alerting rules or notification system integration for certificate expiration, CA
    rotation failures, agent auth failures, update failures, and system health.

### Developer Experience — Pending

- [ ] Development containers
  - **Category**: DX | **Impact**: Medium | **Effort**: Medium
  - Devcontainer config for VS Code / Codespaces with all build dependencies pre-installed.
- [ ] Mock services for local development
  - **Category**: DX | **Impact**: Medium | **Effort**: Medium
  - Mock MQTT broker, mock package registry, and mock GitHub API for local end-to-end testing
    without external dependencies.
- [ ] Hot reloading for development
  - **Category**: DX | **Impact**: Medium | **Effort**: Medium
  - `cargo-watch` config or similar for automatic rebuild on code changes during development.
- [ ] Debugging tools
  - **Category**: DX | **Impact**: Low-Medium | **Effort**: Medium
  - Diagnostic endpoints, debug logging presets, and state dump commands.
- [ ] Consistent error messages
  - **Category**: DX / UX | **Impact**: Medium | **Effort**: Medium
  - Standardize error message format across all binaries and API responses with error codes and
    actionable guidance.
- [ ] Development helper scripts
  - **Category**: DX | **Impact**: Medium | **Effort**: Low-Medium
  - Scripts for common tasks: seed test data, reset DB, generate test certificates, run all
    quality gates.

______________________________________________________________________

## Future Considerations

Items to consider for future versions but not currently prioritized:

- **Multi-tenant enhancements** — Tenant management API (CRUD), multi-tenant JWT (per-tenant
  permissions), tenant switching UI, API token scoping per tenant. Foundation is complete (tenants
  table, `tenant_id` FK, TenantContext extractor, tenant-aware MQTT, tenant-agnostic system
  services).
- **High availability** — Partial HA groundwork is done (DB-backed auth flows, version-gated
  settings cache, CRL cross-instance propagation, NATS JetStream push notifications, DB-backed
  JWT signing key, master key mismatch detection, external scheduler with optimistic locking).
  Remaining: token denylist HA sync, full active-active controller support.
- **Embedded MQTT service** — Use `EmbeddedServiceHost::add()` to embed the MQTT bridge service
  in the controller for single-binary homelab deployments. Infrastructure is ready.
- **Embedded agent** — Use `EmbeddedServiceHost::add()` to embed the agent for single-binary
  deployments. Infrastructure is ready.
- **Agent clustering** — Multiple agents cooperating on a single host or agent pools.
- **Update preview / dry-run mode** — Simulate an update without executing it. Note: plugin
  config testing (dry-run validation of plugin configurations) is implemented via
  `POST /api/v1/plugin-configs/test` — this item refers to full update simulation.
- **Compliance reporting** — Export update audit trails for compliance frameworks.
- **Terraform / Ansible provider integrations** — Infrastructure-as-code for Uptrakit config.
- **GitOps integration** — Declarative configuration management via Git repositories.
- **Mobile app** — Native mobile app for monitoring and approving updates.
- **Browser extension** — Quick status checks from the browser toolbar.

______________________________________________________________________

## Deferred Dependency Upgrades

Dependencies blocked by upstream and requiring external changes before they can move forward.

- [ ] **`strum` 0.27 → 0.28** — blocked by `sea-orm rc.x`, which pins `strum = "^0.27"`. Revisit
  once sea-orm ships a release that accepts `strum ^0.28`.

- [ ] **`rand` 0.9 → 0.10** — blocked by `russh`, `rsa`, and `crypto-bigint` depending on
  `rand_core = "0.10.0-rc-3"`. Revisit once the russh/RustCrypto stack stabilises on stable
  `rand_core 0.10`.

- [ ] **`der` 0.7 / `const-oid` 0.9 / `spki` 0.7 / `x509-cert` 0.2 / `x509-ocsp` 0.2** —
  blocked by `rcgen 0.14` and `x509-ocsp 0.2` requiring `der ^0.7`. Revisit once `rcgen` and
  `x509-ocsp` release versions compatible with `der ^0.8`.

______________________________________________________________________

## Notes

- This roadmap is a living document and should be updated as priorities shift
- Items can be reordered based on user feedback and project needs
- Some items may be split into smaller tasks during implementation
- Cross-phase dependencies should be carefully managed
- Security and quality items should be addressed continuously, not just in their dedicated phases
