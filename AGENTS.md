# AGENTS -- AI Agent Guide for Uptrakit

This file provides structured context for AI coding agents working on the Uptrakit codebase. Read this first before
making any changes.

## Project summary

Uptrakit is an agent-based update tracking toolkit for self-hosted Linux environments. It tracks installed software
versions across remote hosts, checks for updates, and allows **manual, user-triggered** updates. It is **not** an
auto-updater.

Key components:

- **Controller** (server): API, Web UI, optional embedded scheduler, upstream version checking.
- **External Scheduler** (standalone binary): enrolls as a system service, receives DB/NATS/master-key credentials via
  WebSocket, runs scheduled tasks across all tenants independently.
- **MQTT Service** (standalone binary): MQTT/Home Assistant integration with lease-based multi-instance tenant
  distribution.
- **Agents**: lightweight daemons on each managed host; outbound-only secure WebSocket to the controller; local version
  detection and update execution via sudo allowlists.
- **Plugins**: first-party extension modules that detect, report, and update software; each crate implements the
  `Plugin` trait and is registered in `uptrakit-plugin-infrastructure-registry`.

For full project context, see [README.md](README.md). For contribution rules, see [CONTRIBUTING.md](CONTRIBUTING.md).
For system design and technology choices, see [ARCHITECTURE.md](ARCHITECTURE.md). For security policy and cryptographic
details, see [SECURITY.md](SECURITY.md). For the documentation catalogue, see [docs/README.md](docs/README.md).

## Documentation split

- **End-user docs** ([`docs/end-user/`](docs/end-user/)): overview, manual update workflow, Home Assistant/MQTT
  integration, deployment map, CLI usage guide, plugin configurations, update history, user management
  ([user-management.md](docs/end-user/user-management.md)), profile and API tokens, and autodiscovery
  (including [docs/end-user/deployment/reverse-proxy.md](docs/end-user/deployment/reverse-proxy.md)).
- **API & protocol docs** ([`docs/api/`](docs/api/)): AsyncAPI/wire protocol
  ([wire-protocol.md](docs/api/wire-protocol.md)), REST API endpoints ([http-web-api.md](docs/api/http-web-api.md)),
  user management API ([user-management.md](docs/api/user-management.md)),
  settings reconciliation ([settings-runtime.md](docs/api/settings-runtime.md)), auth flows
  ([auth-flows.md](docs/api/auth-flows.md)), and service/tenant operations
  ([services-operations.md](docs/api/services-operations.md)).
- **Security docs** (`docs/security/`): architecture, cryptography, PKI/certificates, auth/permissions,
  secrets/encryption, reverse proxy security, TOFU/TLS, filesystem/dependency hardening, and secure development
  guidance.
- **Architecture docs** ([`docs/architecture/`](docs/architecture/)): entity-level design for multi-tenancy, hosts,
  software items, and update history.
- **Development docs** (`docs/development/`): setup, testing, coding standards, PR process, dependency policy, plugin
  guidelines ([plugin-guidelines.md](docs/development/plugin-guidelines.md)), plugin system architecture
  ([plugin-system.md](docs/development/plugin-system.md)), AI usage expectations, and database migration authoring
  ([database-migrations.md](docs/development/database-migrations.md)).
- **Deployment guides**: reverse proxy deployment and per-proxy guides live under
  [`docs/end-user/deployment/`](docs/end-user/deployment/). Reverse proxy security model is at
  [`docs/security/reverse-proxy-security.md`](docs/security/reverse-proxy-security.md). Docker deployment guide at
  [`docs/end-user/deployment/docker.md`](docs/end-user/deployment/docker.md). Human documentation must link
  into those files rather than [AGENTS.md](AGENTS.md).

## Codebase layout

```text
uptrakit/
├── Cargo.toml                          # Workspace root (resolver = "3", members = "crates/*/*")
├── crates/
│   ├── core/
│   │   ├── agent/                      # uptrakit-agent                         (bin)  — agent daemon
│   │   ├── agent-ssh/                  # uptrakit-agent-ssh                     (bin)  — SSH-backed agent; parallel per-host version checks and updates over SSH (per-host concurrency guard + forwarder task + aggregate mpsc channel); host management CLI, SSH transport (russh), SshTarget parser, ~/.ssh/config resolution, remote host info collection & ReportHosts; SshStdioTunnel (bidirectional byte-stream over russh channel for Docker proxy); ExecuteBatchUpdate handler with freeze check; UI extension `ssh-agent.hosts` (list-hosts, bootstrap, bootstrap-proxmox, list-pve-hosts, sync-host, remove-host, list-discovered-guests, bootstrap-proxmox-guest actions; primary_actions: bootstrap + bootstrap-proxmox + bootstrap-proxmox-guest; ECIES E2E encryption for sensitive params in bootstrap and sync-host; sync-host supports optional auth override via form (password/private_key, custom username) for connecting as a privileged user; bootstrap-proxmox-guest auto-detects PVE host from guest's proxmox_node and auto-fills hostname from guest metadata); ServiceExtensionProxy for invoking controller-side plugin actions (proxmox.hosts/list-all-unmatched, proxmox.hosts/match); PVE node auto-detection during bootstrap with cluster deduplication (check_pve_token_exists → PveTokenStatus) + tenant-scoped PVE credentials (uptrakit-{tenant_id}@pve) + ReportPluginConfig; ExtensionContext struct bundles handler state (db, state_dir, private_key_der, service_id, tenant_id, bg_tx, extension_proxy); remote_exec.rs (SshRemoteExecutor, PveGuestExecutor implementing RemoteExecutor); bootstrap_proxmox.rs (guest bootstrap via PVE exec); CLI commands (host sync, host bootstrap) load persisted tenant_id from service.json for PVE operations
│   │   ├── controller/                 # uptrakit-controller                    (bin)  — central server; migration runner delegates to `uptrakit_shared_db::migration`
│   │   │   ├── src/db_migrate/         #   `db-migrate` subcommand: copies all data between DB backends; error.rs (DbMigrateError + Report<> Result), tables.rs (migrate_table<E>, copy_all, clean_all, verify_all for all 44 app tables), mod.rs (run() orchestrator)
│   │   │   ├── src/scheduler/          #   (cfg: embedded-scheduler) Embedded scheduler using uptrakit-scheduler-engine
│   │   │   └── src/embedded_frontend.rs #  (cfg: embed-frontend) Serves frontend from binary via rust-embed
│   │   ├── mqtt/                       # uptrakit-mqtt                          (bin)  — standalone MQTT service
│   │   ├── scheduler/                  # uptrakit-scheduler                     (bin)  — external scheduler binary; enrolls as a system service (system_service + scheduler + database_access + nats_access + master_key_access + graceful_shutdown), receives credentials, runs scheduled tasks across all tenants via direct DB + NATS
│   │   └── integration-tests/          # uptrakit-integration-tests             (test) — Docker-based integration tests: (1) reverse-proxy tests (real nginx/haproxy/traefik/caddy/envoy containers, CRL/OCSP revocation); (2) system integration tests (uptrakit-test:latest image, verifies enrollment and inter-component communication)
│   ├── plugins/
│   │   ├── infrastructure/
│   │   │   ├── core/                   # uptrakit-plugin-infrastructure-core                   (lib)  — plugin trait + SecretMasking; re-exports tokio::sync::mpsc; defines PluginCapability, HostCompatibility, UpdateHookContext, PreUpdateHookResult; batch types: BatchDetectItem/Result, BatchFetchItem/Result, BatchUpdateItem/Result
│   │   │   ├── registry/              # uptrakit-plugin-infrastructure-registry               (lib)  — plugin dispatch & validation; `daemon` feature (default) enables Docker local ops
│   │   │   └── proxmox/              # uptrakit-plugin-infrastructure-proxmox                (lib)  — Proxmox VE infrastructure plugin: controller-side REST API client for PVE (incl. guest agent file-read for machine_id), VM/CT discovery with best-effort machine_id collection (QEMU only), semi-automatic host matching with inline suggestions (MatchConfidence: High/Medium/Low, signals: machine_id, hostname+IP, hostname, IP, name), manual matching, extension manifests (proxmox.hosts page + proxmox.host-info panel), extension action handlers (list, discover, test-connection, match, approve-match, unmatch, get-info, list-all-unmatched); DB table: proxmox_host_mappings (incl. machine_id column); agent-side modules: pve_setup (PVE detection, cluster dedup via check_pve_token_exists → PveTokenStatus enum, tenant-scoped API credential creation via pveum with pve_user_realm(tenant_id)), guest_exec (command execution inside LXC/QEMU guests via pct exec / qm guest exec)
│   │   ├── releases/
│   │   │   ├── docker/                 # uptrakit-plugin-releases-docker                 (lib)  — Docker/OCI plugin: tag tracking, SHA digest tracking, image pull via bollard, container autodiscovery; `daemon` feature (default) gates bollard + local Docker ops; Docker-over-SSH via StdioTunnel proxy (unix socket bridge to `docker system dial-stdio`)
│   │   │   ├── github/                 # uptrakit-plugin-releases-github                 (lib)  — GitHub Releases plugin: controller-side fetch_releases only; owner/repo parsed from package_identifier at call time (format "owner/repo"); exports validate_identifier
│   │   │   ├── gitlab/                 # uptrakit-plugin-releases-gitlab                 (lib)  — GitLab Releases plugin: controller-side fetch_releases; supports nested namespaces (group/subgroup/project); project path percent-encoded for Projects API; upcoming_release:true skipped unless include_prereleases; PRIVATE-TOKEN auth header; exports validate_identifier (parse_project_path)
│   │   │   └── forgejo/                # uptrakit-plugin-releases-forgejo                (lib)  — Forgejo/Gitea Releases plugin: controller-side fetch_releases; api_base_url required (no default); Forgejo API (Authorization: token); same owner/repo format as GitHub; used by PHS discovery plugin with api_base_url="https://codeberg.org" for Codeberg-hosted PHS items; exports validate_identifier (parse_owner_repo)
│   │   ├── generic/
│   │   │   └── shell/                  # uptrakit-plugin-generic-shell                  (lib)  — generic agent-side plugin: version_command (detect_installed_version) + update_command (execute_update); supports {package_identifier}, {version}, {tag} placeholders; at least one field required
│   │   ├── package-managers/
│   │   │   ├── homebrew/               # uptrakit-plugin-package-manager-homebrew               (lib)  — Homebrew formulae/cask plugin; `HomebrewPackageType` enum: `Both` (default — discovers formulae + casks), `Formula`, `Cask`; implements DetectHostCompatibility (checks `which brew`); native batch_detect_installed_version + batch_fetch_releases (single `brew info --json=v2` call for all packages); DiscoveryTarget always emitted with `{"package_type": "formula"|"cask"}` in both plugin_config and config_override
│   │   │   ├── apt/                    # uptrakit-plugin-package-manager-apt                    (lib)  — APT (Debian/Ubuntu) plugin (discovery via dpkg/apt-mark, version detection via dpkg-query, latest via apt-cache madison, updates via sudo apt-get install); implements DetectHostCompatibility (checks `which apt-get`) and PostUpdateHook (checks /var/run/reboot-required); native batch_detect_installed_version (dpkg-query with all packages) + batch_fetch_releases (apt-cache madison with all packages)
│   │   │   ├── npm/                    # uptrakit-plugin-package-manager-npm                    (lib)  — npm global-package plugin; ControllerSideFetchReleases (queries registry.npmjs.org); discovery via `npm list -g --json`; updates via `sudo npm install -g <pkg>@<version>`; implements DetectHostCompatibility (checks `which npm`); validate_identifier exported for registry; native batch_detect_installed_version (single `npm list -g --depth=0 --json` call, filtered in memory)
│   │   │   ├── mas/                    # uptrakit-plugin-package-manager-mas                    (lib)  — Mac App Store plugin via `mas` CLI; agent-side only (no ControllerSideFetchReleases); discovery via `mas list`; version detection + release fetch via `mas list` + `mas outdated`; updates via `mas upgrade <id>`; implements DetectHostCompatibility (checks `which mas`); package_identifier = numeric App Store ID (digits only, max 15 chars); no sudo needed; native batch_detect_installed_version + batch_fetch_releases (single `mas list` + `mas outdated` calls, mapped in memory)
│   │   │   ├── pacman/                 # uptrakit-plugin-package-manager-pacman                 (lib)  — Arch Linux Pacman plugin; detection via `pacman -Q`; latest version via `pacman -Si`; updates via `sudo pacman -S --noconfirm`; database sync via `sudo pacman -Sy`; discovery via `pacman -Q` (all) or `pacman -Qe` (explicit); implements DetectHostCompatibility (checks `which pacman`); no PostUpdateHook (no /var/run/reboot-required on Arch); needs_setenv=false in sudoers; validate_identifier: lowercase [a-z0-9@._+-], starts with [a-z0-9], max 128 chars; batch_detect_installed_version (single `pacman -Q` call) + batch_fetch_releases (single `pacman -Si` call, output parsed as blank-line-separated blocks)
│   │   │   ├── pkg/                    # uptrakit-plugin-package-manager-pkg                    (lib)  — BSD pkg (pkgng) plugin for FreeBSD, TrueNAS SCALE, OPNsense, pfSense, DragonFly BSD; discovery via `pkg query -a "%n\t%v"` (all) or `pkg query -a "%a\t%n\t%v"` filtered by auto-flag==0 (manual); version detection via `pkg query "%v" <name>`; upstream version via `pkg rquery "%v" <name>` (local repo DB); updates via `sudo pkg install -y <name>`; index refresh via `sudo pkg update -q`; implements DetectHostCompatibility (checks `which pkg`); native batch_detect_installed_version (single `pkg query -a` call, filtered in memory) + batch_fetch_releases (single `pkg rquery "%n\t%v"` call); no PostUpdateHook; needs_setenv=false
│   │   │   ├── apk/                    # uptrakit-plugin-package-manager-apk                    (lib)  — APK (Alpine Linux) plugin; discovery via `apk list --installed` (all mode) or `/etc/apk/world` (world mode); version detection via `apk info -v`; latest version via `apk version`; updates via `sudo apk add <pkg>=<ver>`; implements DetectHostCompatibility (checks `which apk`) and RefreshPackageIndex (`sudo apk update`); package_identifier = Alpine package name (lowercase+digits+._+-, min 2 chars, max 100 chars, no `..`); native batch_detect_installed_version + batch_fetch_releases (single `apk info -v` / `apk version` call for all packages)
│   │   │   ├── snap/                   # uptrakit-plugin-package-manager-snap                   (lib)  — Snap (snapd) plugin for Linux; agent-side only; discovery via `snap list` (excludes system snaps: core*, snapd, bare); version detection via `snap list <name>`; batch_detect_installed_version via single `snap list` parsed into map; release fetch via `snap info <name>` (channels: section parsing); updates via `sudo snap refresh <name>` (optional --channel=); native execute_batch_update (single `snap refresh name1 name2 ...`); implements DetectHostCompatibility (checks `which snap`); package_identifier = snap name (lowercase, digits, hyphens, 2-40 chars); requires sudo for refresh; no package index refresh step (snapd manages cache internally)
│   │   │   └── cargo/                  # uptrakit-plugin-package-manager-cargo                  (lib)  — Cargo install plugin; tracks Rust binaries installed via `cargo install`; discovery + version detection via `cargo install --list` (parse non-indented `<name> v<version>:` headers); ControllerSideFetchReleases via crates.io sparse index (`https://index.crates.io/{prefix}/{name}`, `tame-index` for URL/parsing); batch_fetch_releases bounded to 10 concurrent requests via `buffer_unordered(10)`; updates via `cargo install <name> --version <ver>` (no sudo, installs to `~/.cargo/bin`); implements DetectHostCompatibility (checks `which cargo` exit code); package_identifier = crate name (1–64 chars, starts with letter/underscore, `[A-Za-z0-9_-]`); is_discover_all_mode() when config is default `{}`; custom registry_url uses SsrfSafeResolver::permissive(), default uses SsrfSafeResolver::new()
│   │   ├── notifications/
│   │   │   ├── core/                   # uptrakit-notification-plugin-core       (lib)  — NotificationPlugin trait (deliver, validate_config, mask_config_secrets, extension_manifests, extension_actions), DeliveryMessage, MessageAction, NotificationPluginError, escape_html()
│   │   │   ├── webhook/               # uptrakit-notification-plugin-webhook    (lib)  — Webhook plugin (SSRF validation + header blocklist + HMAC-SHA256 signing)
│   │   │   ├── telegram/              # uptrakit-notification-plugin-telegram   (lib)  — Telegram plugin with inline keyboard (feature-gated)
│   │   │   ├── email/                 # uptrakit-notification-plugin-email      (lib)  — Email plugin (SMTP via mail-send, SmtpSettingsSnapshot, merge_smtp_into_config(global, tenant, config)); extension manifests for channel management + global SMTP defaults (feature-gated)
│   │   │   └── registry/             # uptrakit-notification-plugin-registry   (lib)  — NotificationPluginRegistry, NotificationOps trait, NotificationRegistryConfig; delegates extension_manifests()/extension_actions() to individual plugins; re-exports core types
│   │   └── discovery/
│   │       └── proxmox-helper-scripts/ # uptrakit-plugin-discovery-proxmox-helper-scripts (lib)  — PVE helper-scripts plugin (discovery-only: fetches CT scripts, analyzes for GitHub/Codeberg/npm/APT upstream; emits ReleasesGithub+GenericShell targets for GitHub-managed items, ReleasesForgejo+GenericShell targets for Codeberg-managed items (api_base_url="https://codeberg.org"; uses Forgejo plugin since Codeberg runs Forgejo), PackageManagerNpm target for npm-managed items, PackageManagerApt target for APT-managed items)
│   ├── shared/
│   │   ├── agent-core/                 # uptrakit-agent-core                    (lib)  — shared agent logic: version check, update execution, batch updates; spawn_background()/send_background_result() for non-blocking event loop; run_check_versions/run_discover_software/run_execute_batch_update (compute-only); handle_execute_update/handle_graceful_shutdown; start_update() for per-host parallel use by SSH agent; batch_check_versions() groups assignments by (PluginType, effective_config), calls batch_detect_installed_version in parallel, refreshes package index once per fetch group, then calls batch_fetch_releases in parallel
│   │   ├── command/                    # uptrakit-command                       (lib)  — CommandExecutor trait + LocalCommandExecutor; SudoAwareCommandExecutor (wraps any executor, prepends sudo based on SudoContext); SudoPolicy enum (auto/force_with/force_without); CommandSpec.privileged flag; StdioTunnel trait (bidirectional byte-stream tunnel for remote command I/O); RemoteExecutor trait + RemoteCommandResult (transport-agnostic remote command execution for SSH and PVE guest exec)
│   │   ├── crypto/                     # uptrakit-crypto                        (lib)  — AES-256-GCM at-rest encryption with envelope encryption (KEK wraps DEKs); EncryptedString, init_master_key, DataKeyRing; ENC:v1/v2/v3 formats (v3 = current default with DEK + AAD); column AAD registry (register_column_aad); DEK wrap/unwrap; O(1) master key rotation support
│   │   ├── db/                         # uptrakit-shared-db                     (lib)  — SeaORM entities (hosts, host_tags, host_tag_assignments, software_items, host_software_items, software_ignores, update_history, plugin_type_settings, etc.); `migration` feature flag exposes `uptrakit_shared_db::migration::{Migrator, run_migrations}`; `migration::helpers` module provides reusable SQLite table-recreation helpers (set_foreign_keys, check_crash_recovery, drop_original, rename_temp, is_sqlite)
│   │   ├── directories/                # uptrakit-directories                   (lib)  — cross-platform directory management
│   │   ├── extension-framework/        # uptrakit-extension-framework            (lib)  — UI extension framework types: ExtensionManifest, ActionDef, FieldDef, FormDef, RowVisibleWhen, RowCondition, wire payloads; PanelPosition (adjacently tagged serde: {"type":"tab"}); tab_group for grouped tab rendering; ActionDef supports `confirm_entity_field` for destructive action confirmation dialogs; standalone crate so plugins don't depend on uptrakit-internal-wire
│   │   ├── macros/                     # uptrakit-shared-macros                 (lib)  — shared macros (impl_report_conversion!)
│   │   ├── types/                      # uptrakit-shared-types                  (lib)  — shared value types (PluginRole, PluginType, etc.); network::is_private_host()/is_private_ip() for SSRF validation; ssrf::SsrfSafeResolver (feature `http-ssrf`) for DNS rebinding protection; feature-gated: sea-orm, openapi, http-ssrf
│   │   ├── web-api-types/              # uptrakit-web-api-types                 (lib)  — shared HTTP request/response types
│   │   ├── openapi-client/             # uptrakit-openapi-client                (lib)  — typed HTTP client; full REST API + SSE streaming coverage; re-exports web-api-types, reqwest::Error; feature `mock` adds MockApiServer+MockEndpoint for integration testing; sse.rs provides lightweight SSE parser; update_output_stream.rs provides typed stream_update_output() method; device_auth_stream.rs provides SSE-first device auth; events_stream.rs provides typed admin event SSE client
│   │   ├── nats/                       # uptrakit-nats                          (lib)  — shared NATS primitives: NatsEventEnvelope, NatsConnection, subject routing, stream setup
│   │   ├── scheduler-engine/           # uptrakit-scheduler-engine              (lib)  — scheduler core: poll loop, claim mechanism, interval+jitter scheduling (interval.rs: compute_next_run_at), TaskExecutor trait, SchedulerNotifier trait, 6 built-in executors (AuthCleanup, StaleLeaseCleanup, DetectVersion, FetchReleases, ServiceCertCheck, CrlRenewal); tasks categorised as internal (CrlRenewal, CaRotationCheck, ServiceCertCheck — embedded scheduler only) vs external (AuthCleanup, StaleLeaseCleanup, FetchReleases, DetectVersion — deferrable to external scheduler); `external_scheduler_connected: Arc<AtomicBool>` flag skips external tasks when set; FetchReleasesExecutor Phase B sends fetch assignments for host_software_items so that latest_version is populated
│   │   ├── service-sdk/                # uptrakit-service-sdk                   (lib)  — service lifecycle, SDK-managed event loop, signal handling, enrollment, identity (ServiceIdentityState: service_id + enrollment_secret + tenant_id in service.json), TLS, CA bootstrap, main helpers; default_resolve_shutdown(); `decrypt_sensitive_params<T>()` generic ECIES sealed-box decryption for extension sensitive params; `zeroconf` feature (default): mDNS/DNS-SD discovery module (browse for `_uptrakit._tcp.local.`, cache in `discovery.json` with 0o600 permissions); when `--url` omitted + feature enabled, auto-discovers controller on LAN
│   │   ├── audit-log/                  # uptrakit-audit-log                      (lib)  — AuditLogBackend trait, AuditEntry, AuditFilter, AuditLogDispatcher; backends: NoopBackend, DatabaseBackend (cfg db), JournaldBackend (cfg journald), MultiplexBackend; fire-and-forget dispatcher pattern
│   │   ├── update-hooks/               # uptrakit-update-hooks                  (lib)  — update hook resolution and config merge logic (extracted from web-api)
│   │   └── wire/                       # uptrakit-internal-wire                 (lib)  — service↔controller wire protocol; `Capability` enum + capability negotiation; `ServiceProfile` enum + from_capabilities(); `duration_seconds` serde module for Duration↔u32 fields; report pagination (`paginate.rs` Paginatable trait + `report_tracker.rs` ReportTracker); re-exports `uptrakit-extension-framework` as `extension` module
│   └── ui/
│       ├── cli/                        # uptrakit-cli                           (bin+lib) — CLI interface; uses openapi-client for all API calls (hosts, host-tags, services, software-items, plugin-configs, software-ignores, checks, updates, batch-updates, history, scheduler, settings); `update trigger --follow` and `history tail` use SSE streaming; `update batch-host/batch-item --follow` and `update-batches follow` use batch progress SSE; lib target exposes modules for integration tests
│       ├── web-api/                    # uptrakit-web-api                       (lib)  — HTTP API layer; routes, middleware (security_headers, request_id, request_log, resolve_ip, rate_limit, resolve_proxy_headers, require_auth, audit_log, permission, tenant_context), AppState, router; /healthz (liveness) + /readyz (readiness: DB + CA checks); event_broadcaster.rs (per-tenant admin event SSE), device_flow_broadcaster.rs (device auth SSE); re-exports auth/queries from sibling crates; test_harness/ shared integration test fixtures (TestApp, TestClient, DB/HTTP helpers); integration_tests/ REST API + WebSocket integration tests (#[cfg(all(test, feature = "db-sqlite"))])
│       ├── web-api-auth/               # uptrakit-web-api-auth                  (lib)  — authentication subsystem: auth module (JWT, sessions, OIDC, tokens, permissions), SettingKey, settings_store
│       └── web-api-queries/            # uptrakit-web-api-queries               (lib)  — database query logic: all query modules, TenantDb, ServiceNotifier trait
├── frontend/                           # SvelteKit SPA (Skeleton UI v4 + Tailwind CSS v4)
│   ├── src/
│   │   ├── lib/                        # Shared modules: api client, auth store, types, utils, notifications, sse.ts (SSE: update output + admin events); stores/events.svelte.ts (centralized admin event SSE store)
│   │   │   └── components/             # Shared UI: ConfirmDialog, ModalBackdrop (focus-trapped), ContextMenu (viewport-aware, keyboard-navigable), Pagination (page numbers + ellipsis + total count), TerminalOutput (xterm.js wrapper with dark/light theme)
│   │   └── routes/                     # SvelteKit file-based routes
│   │       ├── profile/                #   /profile — account info + API token management (create/revoke)
│   │       ├── history/                #   /history — update history with filters (host, software item, status) + trigger update button
│   │       ├── scheduler/              #   /scheduler — scheduler task management (edit interval/jitter, enable/disable, trigger now)
│   │       ├── plugin-configs/         #   /plugin-configs — plugin config CRUD + autodiscovery ignore rules management
│   │       ├── software/               #   /software — software item list; Pending tab for discovered items; Edit action (name/enabled)
│   │       └── settings/               #   Settings sub-components (Registration, Auth, MQTT, OIDC, Certs, Enrollment); global CA rotation; MQTT client limit
│   ├── package.json                    # npm scripts: build, check, lint, format, format:check
│   ├── svelte.config.js                # SvelteKit config (static adapter)
│   ├── tsconfig.json
│   ├── vite.config.ts
│   └── vitest.config.ts
├── docker/
│   ├── .dockerignore                   # Docker build context exclusions
│   ├── Dockerfile                      # Multi-stage build (ARG PACKAGE/BINARY/FEATURES)
│   └── Dockerfile.test                 # Multi-binary test image (all 5 binaries, no ENTRYPOINT)
├── docker-compose.yml                  # Compose with profiles: postgres, mqtt, ssh, scheduler, full
├── .env.example                        # Template for docker-compose environment
├── .github/
│   ├── workflows/ci.yml                # CI: fmt check, clippy, tests, reverse-proxy Docker tests, system integration tests, frontend lint + format + check + build
│   ├── workflows/docker.yml            # CI: multi-arch Docker image builds, push to GHCR
│   ├── workflows/release-please.yml    # Release: version bumps, changelog, binary artifact builds + attestation
│   ├── release-please-config.json      # release-please package configuration
│   ├── .release-please-manifest.json   # Current version tracked by release-please
│   └── dependabot.yml                  # Weekly Cargo + npm dependency updates
├── Cross.toml                          # cross-compilation config for ARM64 Linux (aws-lc-sys deps)
├── CONTRIBUTING.md
├── README.md
└── AGENTS.md                           # This file
```

All crates use **edition = "2024"**. Some specify `rust-version = "1.91"`.

### Controller feature flags

| Feature | Default | Description |
| --- | --- | --- |
| `db-sqlite` | Yes | SQLite backend |
| `db-postgres` | No | PostgreSQL backend |
| `db-mysql` | No | MySQL backend |
| `db-all` | No | All database backends |
| `oidc` | Yes | OpenID Connect authentication support. Disabling removes the `openidconnect` crate and all OIDC routes/stores, significantly reducing compile-time dependencies. Propagates to `uptrakit-web-api/oidc`. |
| `embedded-scheduler` | Yes | Embeds the scheduler engine in the controller process. Defers external tasks when an external scheduler connects; internal tasks (CRL renewal, CA rotation, service cert check) always run. Adds `uptrakit-scheduler-engine` dependency. |
| `nats` | No | Enables NATS JetStream transport for cross-controller messaging. Propagates to `uptrakit-web-api/nats`. |
| `swagger-ui` | No | Swagger UI at `/swagger-ui` |
| `embed-frontend` | Yes | Embeds the SvelteKit frontend build into the binary via `rust-embed`. Requires `frontend/build/` to exist at compile time. Removes the `--static-dir` CLI argument. See [Embedded Frontend](docs/development/embedded-frontend.md). |
| `notifications-all` | Yes | Enables all optional notification plugins (Telegram, email). Expands to `notifications-telegram` + `notifications-email` + `uptrakit-web-api/notifications-all`. |
| `notifications-telegram` | No | Telegram notification plugin (enabled transitively via `notifications-all`). Propagates to `uptrakit-web-api/notifications-telegram`. |
| `notifications-email` | No | Email notification plugin via SMTP (enabled transitively via `notifications-all`). Propagates to `uptrakit-web-api/notifications-email`. |
| `interactive` | Yes | Interactive (PTY-based) update sessions with stdin forwarding. Propagates to `uptrakit-web-api/interactive`. Adds the interactive WebSocket endpoint and `InteractiveSessionRegistry`. See [Interactive Updates](docs/development/interactive-updates.md). |
| `zeroconf` | Yes | mDNS/DNS-SD zero-configuration advertising. Enables the `--zeroconf` CLI flag and the advertiser module. Uses the `mdns-sd` crate. See [Zeroconf Discovery](docs/development/zeroconf-discovery.md). |

### Web-API feature flags

| Feature | Default | Description |
| --- | --- | --- |
| `oidc` | Yes | OpenID Connect authentication. Propagates to `uptrakit-web-api-auth/oidc`. Gates the `openidconnect` dependency and all OIDC-specific modules (`oidc_auth`, `oidc_providers`, `oidc_state`), routes, OpenAPI schemas, rate limit entries, and `AppState` stores. Non-OIDC types (`AuthMethod::Oidc`, `require_token_for_oidc`, OIDC DB entities) remain unconditional. |
| `swagger-ui` | No | Swagger UI at `/swagger-ui` |
| `db-sqlite` | No | SQLite backend. Propagates to `uptrakit-web-api-queries/db-sqlite`. |
| `db-postgres` | No | PostgreSQL backend. Propagates to `uptrakit-web-api-queries/db-postgres`. |
| `db-mysql` | No | MySQL backend. Propagates to `uptrakit-web-api-queries/db-mysql`. |
| `db-all` | No | All database backends. Propagates to `uptrakit-web-api-queries/db-all`. |
| `interactive` | No | Interactive update WebSocket endpoint (`/api/v1/update-history/{id}/interactive`), `InteractiveSessionRegistry`. Propagates to `uptrakit-command/interactive` via `uptrakit-agent-core`. |

### Build profiles

| Profile | Usage | Notes |
| --- | --- | --- |
| `dev` (default) | `cargo build` | `debug = "line-tables-only"`, deps at `opt-level = 1`, `aws-lc-sys` at `opt-level = 3`. macOS uses `split-debuginfo=unpacked` (via `.cargo/config.toml`). |
| `release` | `cargo build --release` | `lto = "fat"`, `codegen-units = 1`, `strip = true`. Production-grade but slow to build. |
| `release-fast` | `cargo build --profile release-fast` | Inherits `release` with `lto = false`, `codegen-units = 16`, `strip = false`. For iterative release testing — not production. |

See [Build Speed Optimizations](docs/development/setup.md#build-speed-optimizations) for details.

### Release workflow

Releases are automated via [release-please](https://github.com/googleapis/release-please). Pushing
conventional commits to `main` triggers a release PR. Merging the PR creates a GitHub release with
a `v0.0.x` tag, which triggers binary artifact builds (7 binaries x 4 targets) and Docker image
builds.

Key files:

- `.github/release-please-config.json` — release-please configuration
- `.github/.release-please-manifest.json` — current version tracker
- `.github/workflows/release-please.yml` — release + artifact build workflow
- `Cross.toml` — ARM64 Linux cross-compilation settings

See [docs/development/releases.md](docs/development/releases.md) for full details.

## General MUST FOLLOW Rules for AI Coding Agents

### Quality Gates

All changes must pass defined quality gates. See [docs/development/quality-gates.md](docs/development/quality-gates.md) for details.

Git hooks (managed by [`husky-rs`](https://crates.io/crates/husky-rs)) enforce a subset of these
gates locally on commit and push. They auto-install via `core.hooksPath = .husky` on the first
`cargo build`/`cargo test` run. Set `NO_HUSKY_HOOKS=1` to prevent installation in CI or hermetic
build environments.

#### AI execution guidance

- Always run quality gates relevant to modified areas before finalizing.
- **Lint Markdown when `.md` files are changed** with `markdownlint --config .markdownlint.json '**/*.md'`.
  The `.markdownlintignore` file excludes `node_modules/`, `target/`, `.claude/`, and `CODEREVIEW.md`.
  Do not add exceptions to `.markdownlintignore` or `.markdownlint.json` without explicit approval.
- Scope-based execution is allowed for local iteration:
  - frontend-only changes: run frontend checks (`npm run lint`, `npm run format:check`, `npm run check`, `npm run build`).
  - Rust/backend-only changes: run Rust checks/tests/linters.
  - markdown changes: run `markdownlint --config .markdownlint.json '**/*.md'`.
  - mixed changes: run all relevant gates for every area touched.
- If anything related to reverse proxy behavior changes, run ignored reverse proxy integration tests:
  - `cargo test -p uptrakit-integration-tests --test reverse_proxy -- --ignored`
- Treat the reverse proxy trigger list broadly, including (non-exhaustive):
  - mTLS and certificate forwarding/extraction
  - auth behavior behind proxies
  - IP detection / `ClientIp`, forwarded headers, trusted-proxy logic
  - reverse proxy middleware/settings and related TLS behavior
- If anything related to enrollment, wire protocol, service lifecycle, or inter-component
  communication changes, run the system integration tests (requires Docker and pre-built image):
  - `docker build -f docker/Dockerfile.test -t uptrakit-test:latest .`
  - `cargo test -p uptrakit-integration-tests -- --ignored`

### Dependency registration

All new dependencies — external third-party crates and internal workspace crates alike — must be
added to `[workspace.dependencies]` in the root `Cargo.toml` **first**. Individual crate
`Cargo.toml` files must reference them via `workspace = true`. Never pin a version number or path
locally inside a crate's own `Cargo.toml`.

```toml
# ✅ Correct
[dependencies]
serde = { workspace = true }
uptrakit-internal-wire = { workspace = true }

# ❌ Wrong — do not do this
serde = "1"
uptrakit-internal-wire = { path = "../../shared/wire" }
```

See [docs/development/dependency-policy.md](docs/development/dependency-policy.md) for the full
policy including feature specification rules and optional dependency guidelines.

### Commit Messages

Conventional Commits are required. See [docs/development/commit-messages.md](docs/development/commit-messages.md) for details.

### Architecture rules and invariants

These are non-negotiable design constraints. Do not violate them.

1. **Updates are never automatic.** The scheduler triggers version *checks* only. Update execution requires explicit
   user action (via UI, CLI, or MQTT/Home Assistant).
1. **Agents initiate outbound-only connections.** Agents connect to the controller via secure WebSocket
   (`/api/v1/ws/service`). They never listen on any port or accept inbound connections.
1. **Agents run unprivileged.** They run as a dedicated user (e.g. `uptrakit`). Only the specific commands declared
   by registered plugins via `required_sudo_commands()` are granted `NOPASSWD` sudo access — not blanket `ALL`.
   Sudoers files are generated by `host bootstrap` / `host sync` and contain one entry per resolved command.
   See [Sudoers Management](docs/security/sudoers-management.md).
1. **Plugin split and role-based assignment.** Each `(host, software_item)` pair has per-role plugin assignments
   stored in the `host_software_item_plugins` table. Three roles exist: `detect_version`, `fetch_releases`, and
   `execute_update` (see `PluginRole` enum in `crates/shared/types/src/plugin_role.rs`). Each assignment carries an
   `execution_site` column (`auto`, `agent`, or `controller`) that determines where the operation runs. Plugins
   declaring the `ControllerSideFetchReleases` capability (e.g. GitHub, Docker, npm) have their `fetch_releases`
   executed on the controller by default; local package-index plugins (Homebrew, APT) run agent-side via
   `RefreshPackageIndex` + `fetch_releases()` and report `latest_version` in `VersionCheckResult`. Per-host version
   tracking (`installed_version`, `latest_version`) lives on `host_software_items` for all items (both featured
   and non-featured). The old centralised `available_versions` table has been removed. Keep this boundary clear.
1. **No shell injection.** Any path that constructs or executes shell commands must validate inputs. Custom scripts are
   treated as untrusted input.
1. **No secrets in logs.** Never log tokens, passwords, API keys, or other credentials. All secret fields in HTTP API
   types (`uptrakit-web-api-types`) must use `SecretString` instead of `String`. See
   [Secrets Handling](docs/security/secrets-and-encryption.md).
1. **Logging goes to journald or stdout.** No internal log storage. Full command output is not captured internally --
   only high-level summaries are retained for display.
1. **Tracing spans use `skip_all`.** All `#[tracing::instrument]` annotations must use `skip_all` and explicitly
   list relevant fields. Never auto-capture function arguments. HTTP handlers inherit from the `http.request`
   span created by the request-id middleware. Wire protocol envelopes carry `TraceContext` for distributed tracing.
   See [Tracing Conventions](docs/development/tracing.md).
1. **No overlapping update actions per host.** At most one active (`Pending` or `InProgress`) update may run on a
   host at any time. All update types share the single `update_history` table. This is enforced by:
   - **Application-layer check** — `validate_update_preconditions` queries `update_history` and returns
     `TriggerUpdateError::HostUpdateInProgress` (HTTP 409) if any active row exists.
   - **DB-layer constraint** — a partial unique index `uix_update_history_host_active` on
     `update_history(host_id) WHERE status IN ('pending', 'in_progress')` prevents duplicate active rows
     even under concurrent controller processes (belt-and-suspenders against the application-layer check).
   - **Batch sequential dispatch** — batch items beyond the first per host are inserted as `Queued` (excluded from
     the unique index). `dispatch_next_in_batch` promotes them to `Pending` via a CAS UPDATE (`WHERE status =
     'queued'`), so two controllers cannot double-dispatch the same item. `UpdateStatus::Queued` is NOT a terminal
     state; terminal states are `Completed` and `Failed`.
1. **No raw SQL.** Use the structures and methods provided by Sea ORM and sea_query builders everywhere, including
   migrations. Partial unique indexes use `Index::create().and_where()`, composite foreign keys use
   `ForeignKey::create().from_tbl().from_col().to_col()`, and `INSERT...SELECT` uses `Query::insert().select_from()`.
   **Approved exceptions** (each must have an inline comment naming the limitation):
   - Rate limiter (`crates/ui/web-api-auth/src/auth/rate_limit.rs`): `CASE WHEN` in `ON CONFLICT DO UPDATE` — SeaORM's
     `on_conflict` builder doesn't support conditional expressions. Fully parameterized (no injection risk).
   - SQLite-specific functions (`strftime`, `typeof`) in migrations — no sea_query equivalent.
   - `PRAGMA foreign_keys` in migrations — SQLite-specific pragma with no sea_query equivalent
     (use `helpers::set_foreign_keys()` from `migration::helpers`).
   - `CREATE TABLE new AS SELECT * FROM old` in tests — SQLite-specific shorthand for crash simulation.
   - `CASE` expressions in `INSERT...SELECT` during table recreation — sea_query's builder
     does not support `CASE` in the SELECT column list.
   See [database-migrations.md](docs/development/database-migrations.md) for the full exceptions table
   and the table recreation guide with shared helpers.
1. **Cover new logic with tests.** Cover success and failure paths.
1. **Document everything.** Any code change must be properly documented either in the code, or in the separate
   documentation. Any changes to the agent-controller wire protocol must be documented in
   `crates/shared/wire/asyncapi.yaml` and reflected in [docs/api/wire-protocol.md](docs/api/wire-protocol.md).
1. **Wire protocol payloads must implement `WireValidate`.** Any new wire protocol payload struct with `Vec<T>` or
   `String` fields must implement the `WireValidate` trait in `crates/shared/wire/src/wire_validate_impls.rs`. The
   trait validates per-field and per-collection size limits after deserialization. Add limit constants in
   `crates/shared/wire/src/limits.rs`. Use `check_vec_len()`, `check_string_len()`, and `check_opt_string_len()`
   helpers. See [Wire Protocol — Payload Size Limits](docs/api/wire-protocol.md#payload-size-limits).
1. **Large report payloads must use `send_auto_paginate()`.** When sending `DiscoveryResults`,
   `VersionCheckResults`, `ReportHosts`, or `BatchUpdateResult` from a service, always use
   `conn.send_auto_paginate(msg)` instead of `conn.send(msg)`. This automatically splits payloads exceeding
   768 KB into pages. New paginatable types must implement the `Paginatable` trait in
   `crates/shared/wire/src/paginate.rs`. See [Wire Protocol — Report Pagination](docs/api/wire-protocol.md#report-pagination).
1. **Command-bearing plugin config fields must be validated.** Plugin configs with command strings
   (`version_command`, `update_command`, `post_pull_command`, hook `commands` arrays) must validate command length
   via `validate_command_length()` from `uptrakit-shared-types::command_validation`. Hook command counts must be
   checked against `MAX_HOOK_COMMANDS_PER_PHASE`.
1. **Version/build metadata contract is unified.** All workspace binaries (`uptrakit-controller`, `uptrakit-agent`,
   `uptrakit-agent-ssh`, `uptrakit-mqtt`, `uptrakit-scheduler`, `uptrakit`) must expose consistent `--version`
   metadata output. Enabled features are derived at
   build time from `CARGO_CFG_FEATURE` via `uptrakit_build_info::emit_enabled_features_env()` and passed through
   `UPTRAKIT_BUILD_ENABLED_FEATURES`; do not hardcode feature lists per binary.
1. **Do not add any `#[allow()]`** without explicit confirmation. There are currently no approved exceptions in the
   codebase; all previously allowed lints have been resolved via parameter structs, `FromStr` implementations, or dead
   code removal. Workspace lints (`[workspace.lints]` in root `Cargo.toml`) enforce `warnings = "deny"` and
   `clippy::all = "deny"` across all crates via `[lints] workspace = true`.
1. **Feature flags are additive only.** `#[cfg(not(feature = "X"))]` is **prohibited**. This attribute makes feature
   `X` subtract from the binary, breaking additive semantics and producing incorrect builds when features are combined.
   Use the `cfg!()` macro in expression position instead: `if !cfg!(feature = "embed-frontend") { ... }`. The expression
   form compiles all code paths regardless of enabled features; the dead branch is eliminated by the optimizer. The sole
   allowed exception is `#[cfg(feature = "X")]` (without `not`) on purely additive blocks — code that only exists when
   the feature is enabled. See [Feature Flags](docs/development/coding-standards.md#feature-flags) in the coding
   standards for patterns and examples.
1. **Use `FromStr` for all string-to-type conversions.** Do not add ad-hoc `parse(&str)` methods. Follow the pattern in
   [docs/development/coding-standards.md](docs/development/coding-standards.md) (section "String-to-Type Conversions"):
   typed `Parse{TypeName}Error`, `impl FromStr`, and `s.parse::<MyType>()` at call sites. Route handlers accepting
   UUID path parameters must use `Path<Uuid>` (not `Path<String>` with manual `Uuid::parse_str`). See
   [Coding Standards](docs/development/coding-standards.md) (section "Typed Path Extractors").
1. **Keep the openapi-client in sync with web-api endpoints.** Any web-api endpoint addition or change
   must be reflected in the `uptrakit-openapi-client` crate: new endpoints get client methods, changed
   signatures/response types are updated, removed endpoints have their client methods removed. Excluded
   endpoints: WebSocket, OIDC browser callback, OCSP binary protocol. The SSE streaming endpoint
   (`GET /api/v1/update-history/{id}/output/stream`) is included — see `update_output_stream.rs` and
   `sse.rs`. All entity ID parameters must use `&Uuid` (not `&str`), and all response ID fields must be
   `Uuid` (not `String`) — the only exception is `SystemAlert::id` which uses hardcoded string
   identifiers. See
   [docs/development/openapi-client.md](docs/development/openapi-client.md) for the full method reference.
1. **Do not use `unsafe`, `unwrap` or `panic!`.** Always prefer safe and graceful solutions. Follow the error handling
   requirements in [docs/development/coding-standards.md](docs/development/coding-standards.md): define typed errors
   with `thiserror` and attach/propagate context with `rootcause` (including match-with-fallback and serialization
   helper patterns where applicable).
   **Approved exceptions**: `RwLock::read().unwrap()` and `RwLock::write().unwrap()` on `std::sync::RwLock`
   are safe because `panic = "abort"` in the release profile makes lock poisoning impossible. However,
   prefer `parking_lot::Mutex` (workspace dependency) over `std::sync::Mutex` in all async code —
   `parking_lot::Mutex::lock()` returns the guard directly with no `Result`, so no `.unwrap()` is
   needed at all. See [Coding Standards — Synchronous Locks in Async Code](docs/development/coding-standards.md#synchronous-locks-in-async-code).
1. **Use `StatusCode` for HTTP status codes.** Never compare against numeric literals (`== 404`, `>= 400`). Use
   `reqwest::StatusCode` variants (`StatusCode::NOT_FOUND`, `StatusCode::FORBIDDEN`) and helper methods
   (`.is_client_error()`, `.is_success()`). Store status codes as `StatusCode`, not `u16`, in error enums and structs.
   See [Coding Standards](docs/development/coding-standards.md).
1. **Use `SsrfSafeResolver` for all outbound HTTP clients.** Any `reqwest::Client` that sends requests to
   user-controlled URLs (plugin API base URLs, webhook URLs, registry endpoints) must use
   `.dns_resolver(Arc::new(SsrfSafeResolver::new()))` to prevent DNS rebinding attacks. For self-hosted deployments
   that intentionally allow private URLs, use `SsrfSafeResolver::permissive()` instead. The resolver is in
   `uptrakit_shared_types::ssrf` behind the `http-ssrf` feature. See
   [Secure Development — SSRF Protection](docs/security/secure-development.md#ssrf-protection).
1. **Use typed permission extractors for route authorization.** Never call `user.has_permission(...)` directly in
   handler bodies. Instead, declare the required permission via an Axum extractor in the handler signature (e.g.
   `CanViewHosts(_user): CanViewHosts`). There are 32 granular extractors (e.g. `CanViewServices`,
   `CanApproveServices`, `CanCreateSoftware`, `CanTriggerUpdates`, `CanManageUsers`). The extractors are defined in
   `crates/ui/web-api/src/middleware/permission.rs` via the `permission_extractor!` macro. Each protected endpoint
   must also carry the matching `x-required-permission` OpenAPI extension in its `#[utoipa::path]` annotation (e.g.
   `extensions(("x-required-permission" = json!("view_hosts")))`). See
   [Authentication and Authorization](docs/security/auth-and-authorization.md).
1. **Do not test upstream crate behavior.** Tests must verify internal logic only -- not the behavior of dependencies
   like `thiserror` formatting, `serde` roundtrips on plain derives, or `argon2` salt randomness. See the decision
   table in [Testing Expectations](docs/development/testing.md).
1. **Time-dependent tests must use `start_paused = true` — never real sleeps.** A test is
   *time-dependent* when it calls any of `tokio::time::sleep()`, `tokio::time::timeout()`,
   `tokio::time::advance()`, `tokio::time::Instant::now()`, or `tokio::time::interval()` inside the
   test body. Such tests must use virtual time via `#[tokio::test(start_paused = true)]` and
   `tokio::time::advance()` for deterministic, fast execution. Tests that do not call any Tokio time
   API do **not** need `start_paused = true` — adding it to non-time-dependent tests is incorrect.
   Do not call `tokio::time::pause()` explicitly inside the test body; the attribute starts the runtime
   paused from the very beginning.
   **Exception 1:** Docker-based integration tests (`#[ignore]`) that wait for real external processes.
   **Exception 2:** Tests that use SQLx/SeaORM database connections must NOT use `start_paused = true` —
   Tokio's auto-advance fires pool-internal timers prematurely, causing spurious
   `ConnectionAcquire(Timeout)` failures under stress (nextest `--stress-count`). See
   [Testing](docs/development/testing.md).
   **Exception 3:** Code that calls `OffsetDateTime::now_utc()` (wall-clock time, not Tokio time)
   cannot use `start_paused = true` — it has no effect on real wall-clock time. Instead, inject a
   `Arc<dyn Fn() -> OffsetDateTime + Send + Sync>` clock and advance it in tests using
   `parking_lot::Mutex<OffsetDateTime>`. See `RateLimitStore::with_clock` for the canonical pattern and
   [Testing § Wall-Clock Time Injection](docs/development/testing.md#wall-clock-time-injection).
1. **New API endpoint tests must use the shared `TestApp` harness.** The `test_harness/` module
   (`crates/ui/web-api/src/test_harness/`) provides `TestApp` (in-memory SQLite + migrated schema +
   seeded tenant + fully wired Axum router), `TestClient` (ergonomic HTTP client via `tower::oneshot`),
   and fixture helpers (`register_user`, `insert_service`, `seed_permissions_for_owner`, etc.). Never
   duplicate `test_state()` or `build_test_state()` inline. All integration tests live in
   `integration_tests/` and are gated behind `#[cfg(all(test, feature = "db-sqlite"))]`. See
   [Testing § REST API Integration Tests](docs/development/testing.md#rest-api-integration-tests).
1. **Use `TenantDb` helpers for all tenant-scoped queries.** Never call `Entity::find().all(tenant_db.db())` directly
   on a `TenantScoped` entity — `tenant_db.db()` carries no tenant filter and loads all tenants' data. For entities
   that implement `TenantScoped`, always use `tenant_db.find::<E>()`, `.find_by_id::<E>(id)`, `.update_many::<E>()`,
   or `.delete_many::<E>()`. For join-table entities without `tenant_id` (e.g. `service_host`), use
   `tenant_db.find_via_tenant_join::<Target, Scoped>(relation)` which enforces isolation by JOINing through a
   `TenantScoped` parent entity. See [Coding Standards](docs/development/coding-standards.md) (section
   "Tenant-Safe Database Queries").
1. **Batch queries instead of per-item loops.** Never issue a SELECT (or UPDATE) per item inside a loop — this is an
   N+1 anti-pattern. Load collections with `.is_in(ids)`, then join in memory with `HashMap`. For bulk updates, use
   `Entity::update_many().filter(Column::Id.is_in(ids)).col_expr(...).exec(db)` in a single statement. See
   [Coding Standards](docs/development/coding-standards.md) (section "Tenant-Safe Database Queries", Rule 4).

### Autodiscovery subsystem

Autodiscovery automatically detects software installed on agent hosts and surfaces them as **pending** software items
for user review. Key invariants:

1. **Discovery is event-driven and periodic.** It triggers on new host registration, via explicit API calls
   (`POST /api/v1/hosts/{id}/discover`, `POST /api/v1/plugin-configs/{id}/discover`), and automatically
   every 6 hours via the `discover_software` scheduled task (`DiscoverSoftwareExecutor`).
   The periodic task sends `DiscoverSoftware` to every active agent-backed host and soft-deletes
   (`deactivated_at`) any `host_software_items` junction rows absent from the latest discovery snapshot.

2. **No approval workflow.** All discovered items are created immediately with `enabled: true`. The
   `featured` flag controls visibility: featured items appear individually in the Software list,
   non-featured items appear as aggregated per-host summaries.
   **Invariant:** Periodic re-discovery (`find_or_create_software_item` Phase 1) only updates
   `installed_version` on `host_software_item` rows for items that were originally created by
   autodiscovery. Items with manually assigned plugin configs are skipped -- their version detection
   is handled by the `DetectVersion` scheduled task using the user's assigned plugin config.

3. **Ignore list is separate from deletion.** `DELETE /api/v1/software-items/{id}/hosts/{host_id}?ignore=true`
   removes the host assignment and creates a `software_ignores` row keyed on the software item's
   `(tenant_id, name)`. A single name-based ignore rule suppresses all future discoveries for that
   name across all plugin configs and targets. Without `?ignore=true`, unassigning is a plain delete
   with no ignore rule. Deleting a software item (`DELETE /api/v1/software-items/{id}`) never creates
   ignore rules.

4. **Plugin-driven discovery targets.** Discovery results use structured `DiscoveryTarget` values
   (`crates/shared/types/src/discovery_target.rs`) instead of opaque `extra` metadata. Each
   `DiscoveredSoftware` item can carry a `targets: Vec<DiscoveryTarget>` that tells the controller
   exactly which plugin configs and role assignments to create — no plugin-specific synthesis logic
   in the web-API.

   The controller processes discovery results generically via two paths:
   - **Target-based** (non-empty `targets`): for each target, find-or-create the plugin config and
     create role assignments per the target's `roles` list.
   - **Config-ID-based** (empty `targets`, `plugin_config_id` set): use the discovering plugin's own
     config for all three roles.

   **PHS (Proxmox Helper Scripts)** always emits `DiscoveryTarget` values. During discovery, it
   fetches each container's CT script from `raw.githubusercontent.com` and analyzes it:
   - GitHub-managed apps emit **two** `DiscoveryTarget` values:
     1. `plugin_type: ReleasesGithub`, roles `[FetchReleases]`, config without `owner`/`repo`
        (only `tag_strip_prefix`, `include_prereleases`, `asset_patterns`), and
        `package_identifier: Some("owner/repo")` override.
     2. `plugin_type: GenericShell`, roles `[DetectVersion, ExecuteUpdate]`, config with
        `version_command` (`sudo /usr/local/bin/uptrakit-phs-version {package_identifier}`),
        `update_command` (`sudo PHS_SILENT=1 TERM=xterm /usr/bin/update`), and
        `prefer_interactive: true`. `sudo` is embedded because the Shell plugin uses
        `CommandSpec::shell()`, where `privileged` has no effect. `prefer_interactive: true`
        causes the controller to automatically set `interactive: true` in `ExecuteUpdatePayload`
        (see `config_prefers_interactive` in `update_triggers.rs`), allocating a PTY so
        `/dev/tty` is available for prompts that `PHS_SILENT=1` does not suppress (e.g. the
        low-storage warning `read -r prompt < /dev/tty`).
     The PHS shell constants live in `crates/plugins/discovery/proxmox-helper-scripts/src/plugin.rs`.
   - Codeberg-managed apps (detected via `check_for_codeberg_release` or `CODEBERG_REPO=`) emit **two**
     `DiscoveryTarget` values:
     1. `plugin_type: ReleasesForgejo`, roles `[FetchReleases]`, config with
        `api_base_url: "https://codeberg.org"` (Codeberg runs the Forgejo platform),
        `tag_strip_prefix: "v"`, and `package_identifier: Some("owner/repo")` override.
        The plugin config name is `"Codeberg Releases"` to distinguish it from generic Forgejo instances.
     2. `plugin_type: GenericShell`, roles `[DetectVersion, ExecuteUpdate]` — same PHS Shell target
        as for GitHub-managed items.
   - npm-managed apps emit **two** `DiscoveryTarget` values:
     1. `plugin_type: PackageManagerNpm`, roles `[DetectVersion, FetchReleases]` (no `ExecuteUpdate`),
        config `{}`, name `"NPM (auto)"`, and `package_identifier: Some("<npm-package>")`.
     2. `plugin_type: GenericShell`, roles `[ExecuteUpdate]`, same PHS Shell config as GitHub/Codeberg
        items (`version_command` + `update_command`), name `"PHS Shell"`, no `package_identifier`.
     Updates always go through `/usr/bin/update`, not `npm install -g`.
   - APT-managed apps emit **two** `DiscoveryTarget` values:
     1. `plugin_type: PackageManagerApt`, roles `[DetectVersion, FetchReleases]` (no `ExecuteUpdate`),
        config `{}`, name `"APT (auto)"`, no `package_identifier`.
     2. `plugin_type: GenericShell`, roles `[ExecuteUpdate]`, same PHS Shell config as above.
     Updates always go through `/usr/bin/update`, not `apt-get install`.
   - Apps whose scripts contain neither GitHub nor Codeberg patterns nor a specific `apt install` line are skipped.
   The PHS plugin config itself (`discovery_proxmox_helper_scripts`, always `{}`) is retained as an anchor for
   discovery runs but never linked directly to `SoftwareItem` host assignments.

   **Homebrew** in discover-all mode (no pre-existing config) emits per-item `DiscoveryTarget` values
   with `plugin_type: PackageManagerHomebrew` and config `{"package_type": "formula"}` or `{"package_type": "cask"}`,
   plus display names `"Homebrew (Formulae)"` and `"Homebrew (Casks)"`. When running with an existing
   config, targets are empty and the controller uses the config-ID path.

   **Docker** uses `DockerConfig::is_discover_all_mode()` to decide whether to emit targets.
   When the plugin is invoked without a pre-existing config (all config fields at defaults — i.e.
   the server sent `plugin_config_id: None` with `config: {}`), each discovered item emits one
   `DiscoveryTarget` with `plugin_type: ReleasesDocker`, config `{}`, name `"Docker"`, and all
   three roles.  When a real config is present (`plugin_config_id: Some(_)`), targets are empty and
   the controller uses the config-ID path.

   **APT** uses `AptConfig::is_discover_all_mode()` (true when `discovery_filter` is `None`,
   i.e. the default empty config `{}`) to decide whether to emit targets. When
   `plugin_config_id: None` with `config: {}`, each discovered item discovers **all** installed
   dpkg packages and emits one `DiscoveryTarget` with `plugin_type: PackageManagerApt`, config
   `{}`, name `"APT"`, and all three roles. When a real config is present (`plugin_config_id:
   Some(_)`) with `discovery_filter: "all"` or `discovery_filter: "manual"`, targets are empty
   and items use the config-ID path. (`discovery_filter: "manual"` restricts discovery to packages
   reported by `apt-mark showmanual`.)

   The `extra` field on `DiscoveredSoftware` is purely informational metadata (e.g. Docker's
   `{"containers": ["web-server"]}`) — the controller never interprets it for config synthesis.

5. **Discovery capability is derived from the registry.** Call `state.plugin_ops.discovery_plugin_types()`
   (or `PluginRegistry::discovery_plugin_types()` statically) to get the current list of discovery-capable
   plugin types. This is derived automatically from each plugin's `capabilities()` method via the registry —
   no static list is maintained separately.

6. **Package identifier validation goes through `PluginRegistry`.** Plugin-specific constraints on the
   `package_identifier` field (e.g. Homebrew's allowed character set) must be implemented as:
   (a) a crate-level `pub fn validate_identifier(value: &str) -> std::result::Result<(), String>` in the plugin crate,
   (b) an associated function on the config struct that delegates to it:
   `impl MyConfig { pub fn validate_identifier(value: &str) -> std::result::Result<(), String> { crate::validate_identifier(value) } }`.
   Plugins with no identifier constraints must still implement the associated function as a no-op returning `Ok(())`.
   The `register_plugins!` macro auto-generates `PluginRegistry::validate_package_identifier()` by dispatching through
   each config struct's associated function — no manual match arm is required in the registry.
   The `PluginOps` trait (defined in `infrastructure-core` behind the `plugin-ops` feature, re-exported by
   `infrastructure-registry`) exposes this as `validate_package_identifier_str(plugin_type: &str, value: &str)` for
   trait-object dispatch. Crates that only need `PluginOps` (e.g. `web-api-queries`) should depend on
   `infrastructure-core` with `features = ["plugin-ops"]` rather than the full registry.
   Never add plugin-specific validation logic directly to web API query helpers or route
   handlers. See [Plugin Guidelines](docs/development/plugin-guidelines.md) for the full extension pattern.

7. **Plugins declare required sudo commands via `required_sudo_commands()`.** Any plugin that needs root-level
   command execution must override `required_sudo_commands() -> Vec<SudoCommandEntry>` on its `Plugin` impl.
   Each `SudoCommandEntry` carries a bare command name (or display identifier for helper scripts), a human-readable
   explanation, and an optional `args_suffix`. For most commands, **never hardcode absolute paths** — they are
   resolved on the target host via `command -v` at bootstrap time.

   **Restricting subcommands:** When a command needs only specific subcommands (e.g. `systemctl stop` and
   `systemctl start` but not `systemctl disable`), set `args_suffix: Some("stop *")`. The resolved path
   becomes `/usr/bin/systemctl stop *` in the sudoers file — positional matching prevents other subcommands.

   **Helper scripts:** When a simple sudoers command would be too broad (e.g. granting `cat` would allow
   reading any file), use `SudoCommandEntry::new(command, explanation).with_helper_script(SudoHelperScript::new(install_path, content))`.
   Bootstrap installs the script at `install_path` with mode `0755` and uses that path as the sudoers command; the
   script itself validates arguments to enforce the least-privilege contract that sudoers wildcards cannot safely
   express (`*` matches `/` in sudoers).

   **Never hardcode `sudo` in `CommandSpec`** — instead call `.privileged()`
   on the spec. Shell-mode commands (`CommandSpec::shell`) must embed `sudo` in the command string directly because
   `.privileged()` has no effect on shell mode. `PluginRegistry::all_required_sudo_commands()` aggregates all
   declarations for use by the SSH agent's sudoers generation logic. See
   [Plugin Guidelines](docs/development/plugin-guidelines.md) and [Sudoers Management](docs/security/sudoers-management.md).

8. **Plugin capabilities.** The `PluginCapability` enum (in `crates/plugins/infrastructure/core/src/types.rs`) has six variants:

   - `DiscoverLocalSoftware` — the plugin can discover locally installed software.
   - `RefreshPackageIndex` — the plugin can refresh/sync a local package index from remote sources.
   - `DetectHostCompatibility` — the plugin implements `detect_host_compatibility()` which returns a
     `HostCompatibility` enum (`Compatible` or `Incompatible { reason: String }`). Both
     `HostCompatibility` and `PluginError` carry `#[non_exhaustive]`; `PluginError::is_retryable()`
     classifies transient errors (command spawn/wait, timeouts, capture failures, internal errors)
     for the version check retry logic in `crates/shared/agent-core/src/version_check.rs`.
     External match sites must include a wildcard arm (see `coding-standards.md` § Public Enum
     Extensibility). Implemented by:
     `AptPlugin` (checks `which apt-get`) and `HomebrewPlugin` (checks `which brew`).
   - `PreUpdateHook` — the plugin implements `pre_update_hook(context: &UpdateHookContext)` which
     returns `PreUpdateHookResult` (`Proceed` or `Abort { reason: String }`). An `Abort` cancels the
     update before it executes.
   - `PostUpdateHook` — the plugin implements `post_update_hook(context: &UpdateHookContext)` which
     is non-fatal. Implemented by `AptPlugin` (checks `/var/run/reboot-required` and logs a warning).
   - `ControllerSideFetchReleases` — the plugin's `fetch_releases()` requires no local system state
     and can run on the controller instead of the agent. Implemented by `GithubPlugin` and
     `DockerPlugin`. This capability interacts with the `execution_site` field on
     `host_software_item_plugins`: `auto` (default) delegates to the controller when this capability
     is present, `agent` forces agent-side execution, `controller` forces controller-side execution.

   The `UpdateHookContext` struct contains `package_identifier`, `to_version`, and `from_version`.
   These plugin-level hooks are distinct from the user-configured `hooks` in plugin config JSON
   (documented in [Update Hooks](docs/development/update-hooks.md)).

   **`HookCommand` dispatch — skip-not-abort on unknown variants.** `HookCommand` is
   `#[non_exhaustive]`. Both `run_hook_command_inner` and `run_hook_for_batch_inner` in
   `crates/shared/agent-core/src/update.rs` contain a `_ =>` wildcard arm that **must** warn and
   return `Ok` (skip the hook), never `Err`. An older agent that receives a hook variant added by a
   newer controller must not abort the update — skipping the unrecognised hook preserves the ability
   to roll out new hook types without requiring all agents to be updated first. Do not change these
   arms to return errors.

   **Batch trait methods** (all have default sequential fallbacks; override for efficiency):

   - `batch_detect_installed_version(&[BatchDetectItem]) -> Result<Vec<BatchDetectResult>>` — detect
     installed versions for multiple packages. Default calls `detect_installed_version` per item.
     Override when the package manager accepts a list in one command (APT: `dpkg-query pkg1 pkg2`;
     Homebrew: `brew info --json=v2 pkg1 pkg2`; npm: `npm list -g --depth=0 --json` + memory filter).
   - `batch_fetch_releases(&[BatchFetchItem]) -> Result<Vec<BatchFetchResult>>` — fetch upstream
     releases for multiple packages. Default calls `fetch_releases` per item. Override when the local
     package index supports multi-package queries (APT: `apt-cache madison pkg1 pkg2`; Homebrew:
     `brew info --json=v2 pkg1 pkg2`). Do **not** override for API-based plugins with per-package
     HTTP endpoints (GitHub, GitLab, npm registry).
   - `execute_batch_update(&[BatchUpdateItem], output_tx) -> Result<Vec<BatchUpdateResult>>` —
     update multiple packages in one command. Default calls `execute_update` per item. Implemented by
     APT, Homebrew, and npm.

   Agent-core `batch_check_versions()` groups `VersionCheckAssignment`s by `(PluginType,
   effective_config_json)` and calls these batch methods once per group via `join_all`.
   `RefreshPackageIndex` is called at most once per unique fetch group (before `batch_fetch_releases`
   runs); it is not called for detect-only groups. Scheduler Phase A groups by `plugin_config_id`
   only and calls `batch_fetch_releases` once per config.

9. **Discovery allowlist controls which plugin types run.** Two tables, `tenant_discovery_allowlist`
   and `host_discovery_allowlist`, gate which discovery plugin types execute during
   `trigger_discovery_for_agent_host()`:

   - **Unconfigured (no entries for the tenant):** all discovery-capable plugin types run (backward-compatible default).
   - **Tenant-wide entries exist:** only the listed plugin types run tenant-wide.
   - **Host-specific entries exist for the target host:** those entries fully override the tenant list for that
     host (the host list replaces — not extends — the tenant list).

   This applies to auto-discovery on new host registration and to `POST /api/v1/hosts/{id}/discover`.
   It does **not** apply to `POST /api/v1/plugin-configs/{id}/discover` (explicit plugin-config invocation bypasses
   the allowlist intentionally). Duplicate entries are idempotent — the server returns the existing entry
   rather than erroring. Only plugin types that have the `DiscoverLocalSoftware` capability and are not
   `PluginType::Other(...)` can be added to the allowlist; all other types are rejected with HTTP 400.

10. **Partial unique indexes.** `software_items` uses a partial unique index
   `(tenant_id, name) WHERE deactivated_at IS NULL` — prevents duplicate item names within a tenant while
   allowing re-creation after soft-delete. `host_software_item_plugins` uses a unique index
   `(host_id, software_item_id, role, ordinal)` — prevents duplicate role assignments for the same
   host-software-item pair.

#### Key files

| File | Purpose |
| --- | --- |
| `crates/shared/types/src/plugin_role.rs` | `PluginRole` enum (`DetectVersion`, `FetchReleases`, `ExecuteUpdate`, `Other`) |
| `crates/shared/types/src/update_category.rs` | `UpdateCategory` enum (`Security`, `Bugfix`, `Feature`, `Unknown`) — classifies available updates |
| `crates/shared/types/src/discovered_software.rs` | `DiscoveredSoftware` type (with `targets: Vec<DiscoveryTarget>`) |
| `crates/shared/types/src/discovery_target.rs` | `DiscoveryTarget` struct (plugin type, config, name, roles, overrides) |
| `crates/shared/db/src/entity/host_software_item_plugin.rs` | SeaORM entity for role-based plugin assignments |
| `crates/shared/db/src/entity/software_ignore.rs` | SeaORM entity for ignore rules |
| `crates/shared/agent-core/src/client.rs` | `run_discover_software()` / `spawn_background()` agent-side discovery logic |
| `crates/ui/web-api-queries/src/queries/autodiscovery.rs` | DB helpers + `process_discovery_results()` |
| `crates/ui/web-api/src/routes/software_ignores.rs` | Ignore list CRUD routes |
| `crates/ui/web-api/src/routes/service_ws/handler/discovery.rs` | `trigger_discovery_for_agent_host()` helper — applies allowlist before dispatching |
| `crates/shared/db/src/entity/tenant_discovery_allowlist.rs` | SeaORM entity for tenant-wide discovery allowlist |
| `crates/shared/db/src/entity/host_discovery_allowlist.rs` | SeaORM entity for per-host discovery allowlist |
| `crates/ui/web-api-queries/src/queries/discovery_allowlist.rs` | DB helpers: list, add, remove, and `load_*_allowlist_set()` for filter lookups |
| `crates/ui/web-api/src/routes/discovery_allowlist.rs` | Route handlers for tenant/host allowlist CRUD |
| `docs/api/autodiscovery.md` | Full API reference for autodiscovery endpoints |
| `docs/api/discovery-allowlist.md` | Full API reference for discovery allowlist endpoints |
| `docs/end-user/autodiscovery.md` | End-user guide (discovery workflow, review, ignore list, allowlist) |
| `docs/end-user/plugin-configs.md` | End-user guide for plugin config CRUD and discovery |
| `docs/end-user/cli-usage.md` | CLI command reference including `plugin-configs`, `autodiscovery`, and `discovery-allowlist` groups |

### Plugin type settings (two-tier config model)

Plugin configuration uses a **two-tier model**: **type settings** (tenant-level defaults per plugin type) and
**plugin configs** (named configuration profiles with credentials/endpoints). Type settings store discovery
preferences and behavioral defaults (e.g. APT `discovery_filter`, Homebrew `package_type`) that apply to all
instances of a plugin type within a tenant. Plugin configs store credentials, API endpoints, and per-profile
settings that vary between configurations.

The `plugin_type_settings` table (`crates/shared/db/src/entity/plugin_type_setting.rs`) stores one row per
`(tenant_id, plugin_type)` pair with a JSON `config` column. When no row exists, the plugin type's built-in
defaults apply.

**Three-layer config merge.** When the system needs the effective configuration for a plugin operation,
`resolve_effective_config()` (`crates/ui/web-api-queries/src/queries/plugin_configs.rs`) merges three layers:

1. **Type settings** (tenant-level defaults from `plugin_type_settings`) -- broadest scope.
2. **Profile config** (from the `plugin_configs` row) -- named configuration.
3. **Assignment config** (per-host override from `host_software_item_plugins.config`) -- narrowest scope.

Each layer's JSON is shallow-merged on top of the previous one. Fields present in a narrower layer override
the same field from a broader layer.

**`ConfigFormSchema` trait extensions.** Plugins that support type settings implement two additional methods:

- `type_settings_form_schema() -> Vec<FieldDef>` -- returns the form field definitions for the type settings
  UI (e.g. `discovery_filter` for APT, `package_type` for Homebrew).
- `type_settings_sample() -> serde_json::Value` -- returns a sample/default JSON for the type settings.

The `register_plugins!` macro auto-generates `type_settings_form_schema()` and `type_settings_sample_for()`
dispatch methods on `PluginRegistry`, plus the `PluginOps` trait methods `type_settings_form_schema_str()`
and `type_settings_sample_for_str()` for trait-object dispatch.

The `PluginTypeInfo` response (from `GET /api/v1/plugin-types`) includes `type_settings_form_fields` and
`type_settings_sample` fields so the frontend can render a settings form.

**REST API endpoints:**

- `GET /api/v1/plugin-type-settings` -- list all plugin types with active type settings for the tenant.
- `GET /api/v1/plugin-type-settings/:plugin_type` -- get the current type settings for a plugin type.
- `PUT /api/v1/plugin-type-settings/:plugin_type` -- upsert type settings (create or update).
- `DELETE /api/v1/plugin-type-settings/:plugin_type` -- reset to built-in defaults (deletes the row).

All endpoints require `update_software` permission.

Key files:

| File | Purpose |
| --- | --- |
| `crates/shared/db/src/entity/plugin_type_setting.rs` | SeaORM entity for `plugin_type_settings` table |
| `crates/plugins/infrastructure/core/src/form_schema.rs` | `ConfigFormSchema` trait with `type_settings_form_schema()` |
| `crates/ui/web-api-queries/src/queries/plugin_configs.rs` | `resolve_effective_config()` three-layer merge |
| `crates/ui/web-api/src/routes/plugin_type_settings.rs` | Route handlers for type settings CRUD |

### Home Assistant MQTT discovery

The MQTT service can publish [Home Assistant MQTT Discovery](https://www.home-assistant.io/integrations/mqtt/#mqtt-discovery)
topics for each tracked software item, creating `update` entities in HA — one per `(software_item, host)` pair.
It also publishes **per-host summary entities** summarising all non-featured software items for that host.

Key invariants:

1. **HA Discovery is opt-in per MQTT client.** Two columns on `mqtt_clients` control it:
   `ha_discovery BOOL` and `ha_discovery_prefix TEXT DEFAULT 'homeassistant'`. This flag controls
   **only** the publication of `{ha_prefix}/update/.../config` discovery topics. State and version
   topics under `{topic_prefix}` are always published for all connected, enabled clients.
2. **State push is controller-initiated.** The controller sends `SoftwareStates` (wire type
   `software_states`) to MQTT services whenever version data changes. Push triggers: version check
   completed, update triggered (REST/MQTT/scheduler), `update_started` received from agent, update
   result received, batch update triggered or completed. The `update_in_progress` field in each
   host entry reflects whether a `Pending` or `InProgress` update exists at query time. The MQTT
   service stores the states in memory and publishes state, `latest_version`, and `attributes` (JSON
   `in_progress` flag) retained topics to the broker for **all** connected clients, plus HA discovery
   config topics for HA-enabled clients.
3. **`SoftwareStates` is safe for cross-controller delivery.** It contains no credentials and is published
   to NATS (when configured) with `target_capability = "mqtt_bridge"` so only MQTT services receive it.
4. **Reconnect resilience.** On every `ConnAck` the MQTT service emits a `Reconnected` event, causing
   `TenantManager` to republish all state/version topics (for all clients) and HA discovery config topics
   (for HA-enabled clients) from the in-memory cache.
5. **HA restart resilience.** HA publishes `"online"` to `{ha_discovery_prefix}/status` on startup
   (birth message). The MQTT service subscribes to this topic and republishes **only** the HA discovery
   config topics when `"online"` is received (`HaOnline` event). State and version topics are retained on
   the broker and do not need re-sending after an HA restart.
6. **Updates triggered via MQTT (software items).** When a user presses Install in HA on a software-item
   entity, HA publishes `"install"` to the entity's command topic. The MQTT service resolves `to_version`
   from the in-memory state cache and sends `ServiceMessage::MqttTriggerUpdate` to the controller. The
   controller validates the request and dispatches `execute_update` to the agent. On failure the
   controller sends `error` back (soft error — WebSocket is not closed).
7. **Updates triggered via MQTT (host batch).** When a user presses Install in HA on the per-host
   packages entity or the security updates entity, the MQTT service sends
   `ServiceMessage::MqttTriggerHostBatchUpdate` to the controller (with `security_only = true` for
   the security entity). The controller finds all qualifying outdated non-featured items, creates an
   `update_batch`, and dispatches `execute_batch_update` to the agent. On completion
   the controller pushes `software_states`
   again to reflect the updated `installed_version` values and `update_in_progress = false`.
8. **Actor attribution.** Updates triggered via MQTT have `actor_type = "mqtt"` and
   `actor_id = <mqtt_client_id>` in the `update_history` record.

#### MQTT topic scheme

All topics use the MQTT client's `topic_prefix` field. All host-scoped topics share the
`{prefix}/hosts/{host_id}` prefix (implemented via `host_topic_prefix()` in `ha_discovery.rs`).

**Per-host identity and metadata topics** (`{h}` = host UUID):

| Topic | Retained | Direction | Purpose |
| --- | :---: | --- | --- |
| `{prefix}/hosts/{host_id}/hostname` | ✓ | publish | Raw hostname string |
| `{prefix}/hosts/{host_id}/friendly_name` | ✓ | publish | User-defined display name |
| `{prefix}/hosts/{host_id}/info` | ✓ | publish | JSON: `{"os_type":…,"os_version":…,"architecture":…}` |
| `{prefix}/hosts/{host_id}/tags` | ✓ | publish | JSON array of tag name strings |
| `{prefix}/hosts/{host_id}/agent` | ✓ | publish | JSON: `{"last_seen":…,"version":…}` |
| `{prefix}/hosts/{host_id}/connectivity/state` | ✓ | publish | `"online"` or `"offline"` (event-driven, via `HostConnectivityUpdated`) |
| `{prefix}/hosts/{host_id}/connectivity/attributes` | ✓ | publish | JSON: `{"last_seen":…,"version":…}` |

**Software item topics** (`{t}` = tenant UUID hex, `{i}` = item UUID hex, `{h}` = host UUID hex):

| Topic | Retained | Direction | Purpose |
| --- | :---: | --- | --- |
| `{prefix}/hosts/{host_id}/items/{item_id}/state` | ✓ | publish | Installed version string |
| `{prefix}/hosts/{host_id}/items/{item_id}/latest_version` | ✓ | publish | Latest available version string |
| `{prefix}/hosts/{host_id}/items/{item_id}/attributes` | ✓ | publish | JSON: `{"in_progress":…,"update_category":…,"release_date":…,"last_checked_at":…}` |
| `{prefix}/hosts/{host_id}/items/{item_id}/set` | — | subscribe | Receives `"install"` from HA |
| `{ha_prefix}/update/uptrakit/{t}_{h}_{i}/config` | ✓ | publish | HA discovery config (JSON) |
| `{ha_prefix}/status` | — | subscribe | HA birth/will (`"online"` / `"offline"`) |

**Host package topics** (`{t}` = tenant UUID hex, `{h}` = host UUID hex):

| Topic | Retained | Direction | Purpose |
| --- | :---: | --- | --- |
| `{prefix}/hosts/{host_id}/state` | ✓ | publish | `"unknown"` when updates pending, `"up-to-date"` otherwise |
| `{prefix}/hosts/{host_id}/latest_version` | ✓ | publish | `"N available"` when updates pending, `"up-to-date"` otherwise |
| `{prefix}/hosts/{host_id}/attributes` | ✓ | publish | JSON: `{"in_progress":…,"pending_count":N,"total_count":N,"bugfix_count":N,"feature_count":N}` |
| `{prefix}/hosts/{host_id}/set` | — | subscribe | Receives `"install"` → triggers batch update (all non-featured items) |
| `{ha_prefix}/update/uptrakit/{t}_{h}_pkgs/config` | ✓ | publish | HA discovery config for host summary entity (disabled by default) |
| `{prefix}/hosts/{host_id}/security/state` | ✓ | publish | `"unknown"` when security updates pending, `"up-to-date"` otherwise |
| `{prefix}/hosts/{host_id}/security/latest_version` | ✓ | publish | `"N available"` when security updates pending, `"up-to-date"` otherwise |
| `{prefix}/hosts/{host_id}/security/attributes` | ✓ | publish | JSON: `{"in_progress": bool, "pending_count": N}` |
| `{prefix}/hosts/{host_id}/security/set` | — | subscribe | Receives `"install"` → triggers security-only batch update |
| `{ha_prefix}/update/uptrakit/{t}_{h}_sec/config` | ✓ | publish | HA discovery config for security updates entity (disabled by default) |
| `{ha_prefix}/binary_sensor/uptrakit/{t}_{h}_conn/config` | ✓ | publish | HA discovery config for connectivity `binary_sensor` (enabled by default) |

All entities for a given host share a single HA device: `uptrakit_host_{t}_{h}` (name = `friendly_name`).

Software item entities: unique_id `uptrakit_{t}_{h}_{i}`, entity name = `{item_name}`,
`default_entity_id` = `update.uptrakit_{fn_slug}_{item_slug}`.

Host summary entities: unique_id `uptrakit_{t}_{h}_pkgs`,
entity name `"{friendly_name} packages"`, `default_entity_id` = `update.uptrakit_{fn_slug}_packages`.
Both host summary entities are **disabled by default** in HA (`"enabled_by_default": false`).

Security update entities: unique_id `uptrakit_{t}_{h}_sec`,
entity name `"{friendly_name} security updates"`,
`default_entity_id` = `update.uptrakit_{fn_slug}_security_updates`. Install triggers a `security_only = true` batch.

Connectivity sensor: unique_id `uptrakit_{t}_{h}_conn`, entity name `"{friendly_name} agent"`,
platform `binary_sensor`, `device_class = "connectivity"`. **Enabled by default** in HA.
Published immediately on agent connect/disconnect via `ControllerMessage::HostConnectivityUpdated`
(NATS-routed to all MQTT services, not computed from a DB scan).

#### Key files

| File | Purpose |
| --- | --- |
| `crates/core/mqtt/src/ha_discovery.rs` | Pure HA topic/config helpers for software items, host summaries, security entities, metadata topics, and connectivity `binary_sensor`; `parse_command_topic`, `parse_host_packages_command_topic`, `parse_host_security_command_topic` |
| `crates/core/mqtt/src/tenant_manager.rs` | `TenantManager`: software state + host summary state cache + host metadata cache + connectivity cache (`connectivity_cache: HashMap<(tenant_id, host_id), ConnectivityState>`); `publish_host_metadata`, `publish_connectivity_for_host`, `handle_host_connectivity_updated`; all publish methods use `publish_or_abort!` macro to abort batch on first error (prevents O(N × timeout) latency when broker is down) |
| `crates/core/mqtt/src/mqtt_client.rs` | `MqttServiceEvent` enum, `publish_retained` (5 s `OPERATION_TIMEOUT`), `subscribe_topic` (5 s timeout), `shutdown` (5 s timeout on offline publish + disconnect), HA status topic handling; timeouts prevent indefinite blocking when broker connection is down |
| `crates/core/mqtt/src/main.rs` | `on_service_event` dispatch; `ControllerMessage::SoftwareStates` handler; `ControllerMessage::HostConnectivityUpdated` handler; `MqttTriggerHostBatchUpdate` dispatch |
| `crates/ui/web-api-queries/src/queries/mqtt_software_states.rs` | Bulk query loading enabled software items + host summaries (from `host_software_item`) + `build_host_metadata` (OS info, tags, agent info) |
| `crates/ui/web-api/src/notification_service.rs` | `push_software_states_for_tenant` (local broadcast + optional NATS publish); `send_connectivity_update` (wraps `HostConnectivityUpdated`, local broadcast + NATS) |
| `crates/ui/web-api/src/routes/service_ws/handler/mqtt.rs` | `MqttTriggerUpdate` and `MqttTriggerHostBatchUpdate` handlers |
| `crates/ui/web-api-queries/src/queries/update_triggers.rs` | `trigger_update_for_host` (refactored into `validate_update_preconditions`, `create_update_history_record`, `dispatch_update_to_agent` layers); shared by REST, MQTT, and batch handlers; resolves `interactive` flag (caller + `config_prefers_interactive`) before `create_update_history_record` so the column is persisted accurately |
| `crates/ui/web-api-queries/src/queries/update_batches.rs` | Batch update logic: `find_outdated_items_for_host`, `create_batch`, `dispatch_next_in_batch`, `trigger_all_host_batch_updates_for_host` |
| `crates/ui/web-api/src/routes/update_batches.rs` | Batch update route handlers + SSE batch progress endpoint |
| `crates/ui/web-api/src/batch_progress_broadcaster.rs` | `BatchProgressBroadcaster`: per-batch `broadcast` channels for SSE streaming |
| `crates/shared/web-api-types/src/update_batches.rs` | Batch API types (`HostBatchUpdateRequest`, `ItemBatchUpdateRequest`, `BatchUpdateResponse`, etc.) |
| `crates/shared/db/src/entity/update_batch.rs` | `UpdateBatch` SeaORM entity with `BatchStatus` enum |
| `crates/shared/openapi-client/src/update_batches.rs` | Typed HTTP client methods for batch endpoints |
| `crates/shared/openapi-client/src/batch_progress_stream.rs` | SSE streaming client for batch progress events |
| `crates/ui/cli/src/commands/batch_update.rs` | CLI batch update commands |
| `docs/end-user/home-assistant-mqtt.md` | Full end-user setup guide including host summary entities, metadata topics, and connectivity sensor |
| `docs/api/wire-protocol.md` | `software_states`, `host_connectivity_updated`, `mqtt_trigger_update`, and `mqtt_trigger_host_batch_update` payload docs |
| `crates/shared/wire/asyncapi.yaml` | AsyncAPI schemas for all messages and schemas |

### Service ping interval

The ping interval is controller-managed and per-service configurable. The `services` DB table has a nullable
`ping_interval_seconds INTEGER` column. The controller reads this value per-service and falls back to
profile-based defaults (300s for `Agent` profile, 15s for `MqttBridge` profile) when the column is `NULL`.
Defaults are provided by `ServiceProfile::default_ping_interval_secs()`.

Key integration points:

- **Wire protocol**: `ServiceSettingsPayload.ping_interval` is a required `Duration` field serialized as `u32` seconds
  via `#[serde(with = "duration_seconds")]`. The `duration_seconds` module in `uptrakit-internal-wire` converts between
  `std::time::Duration` and `u32` seconds on the wire. `ServiceSettingsPayload.tenant_id` is an `Option<Uuid>` present
  for tenant-scoped services (agents, SSH agents) and absent for system services.
- **SDK event loop**: The ping timer starts as `None` and is created when the first `ServiceSettings` message arrives
  with the controller-provided `ping_interval`. The `ServiceHandler::ping_interval()` trait method has been removed.
- **REST API**: `PUT /api/v1/services/{id}` accepts `UpdateServiceRequest { ping_interval_seconds: Option<u32> }`.
  Set to `0` to clear the override, positive value to override, omit to keep current.
- **CLI**: `uptrakit services update <id> --ping-interval <seconds>`.
- **OpenAPI client**: `update_service(&self, id: &Uuid, req: &UpdateServiceRequest) -> Result<ServiceResponse>`.
- **Frontend**: Service page context menu includes "Edit Ping Interval" dialog.
- **`ServiceResponse`**: Includes `ping_interval_seconds: Option<u32>` (`None` means the profile-based default is used).

### Capability-based service identity

Services are identified by their **capability set** rather than a fixed type enum. The former `ServiceType` enum
(`Agent`, `Mqtt`, `SshAgent`) and its backing file `crates/shared/types/src/service_type.rs` have been removed.

#### Capability set

Each service declares a `BTreeSet<Capability>` at enrollment time. The set is persisted as a JSON array string in the
`services.capabilities` DB column.

| Capability | Wire String | Agent | SSH Agent | MQTT | Scheduler | Controller |
| --- | --- | :---: | :---: | :---: | :---: | :---: |
| `SoftwareDiscovery` | `software_discovery` | yes | yes | -- | -- | yes |
| `UpdateHooks` | `update_hooks` | yes | yes | -- | -- | yes |
| `GracefulShutdown` | `graceful_shutdown` | yes | yes | yes | yes | yes |
| `MqttBridge` | `mqtt_bridge` | -- | -- | yes | -- | yes |
| `SshRemote` | `ssh_remote` | -- | yes | -- | -- | yes |
| `SystemService` | `system_service` | -- | -- | yes | yes | yes |
| `Scheduler` | `scheduler` | -- | -- | -- | yes | yes |
| `DatabaseAccess` | `database_access` | -- | -- | -- | yes | yes |
| `NatsAccess` | `nats_access` | -- | -- | -- | yes | yes |
| `MasterKeyAccess` | `master_key_access` | -- | -- | -- | yes | yes |
| `CaManagement` | `ca_management` | -- | -- | -- | -- | yes |
| `InteractiveUpdates` | `interactive_updates` | yes | yes | -- | -- | yes |
| `Other(String)` | *(unknown)* | -- | -- | -- | -- | -- |

The `interactive` Cargo feature is now a default feature on all three binary crates (agent, agent-ssh, controller).

`Other(String)` is a forward-compat catch-all received from newer peers; it never participates in intersection
(`Capability::is_known()` returns `false` for it).

#### ServiceProfile (derived, never stored)

`ServiceProfile` is a runtime-only enum derived from capabilities via `ServiceProfile::from_capabilities()`. It drives
behavioral defaults (ping interval, shutdown timeout, human-readable label). It is **never persisted** in the database.

| Profile | Key capability | Services | Default ping | Shutdown timeout |
| --- | --- | --- | --- | --- |
| `MqttBridge` | `Capability::MqttBridge` | MQTT service | 15 s | None |
| `Scheduler` | `Capability::Scheduler` | External scheduler | 60 s | 30 s |
| `Agent` | `Capability::SoftwareDiscovery` | Local agent, SSH agent | 300 s | 120 s |
| `Unknown` | (none of the above) | Unrecognized | 300 s | 120 s |

`ServiceProfile::service_label(has_ssh_remote)` provides the human-readable label: "Agent", "SSH Agent",
"MQTT Bridge", "Scheduler", or "Unknown".

#### Capability negotiation (wire protocol)

1. Controller sends `service_settings` with `capabilities: [...]` after mTLS authentication.
2. Service sends `report_hosts` / `register` with its own `capabilities: [...]`.
3. Each side computes `agreed = intersection(controller_caps, service_caps)` excluding `Other` values.
4. The agreed set is stored on the connection via `ControllerConnection::set_agreed_capabilities()`.

#### Enrollment tokens

Multiple named enrollment tokens are stored in the `enrollment_tokens` table (`crates/shared/db/src/entity/enrollment_token.rs`).
Each token supports capability scoping, usage limits (`max_uses`), and TTL (`expires_at`). The former single-token model
(`SettingKey::EnrollmentTokenHash`, `service_enrollment.token_hash`) has been removed entirely.

REST API: `POST/GET /api/v1/enrollment-tokens`, `GET/DELETE /api/v1/enrollment-tokens/{id}` (requires `ManageAgents`).
OpenAPI client: `crates/shared/openapi-client/src/enrollment_tokens.rs`.
CLI: `uptrakit enrollment-tokens list|create|show|revoke`.

During enrollment, the controller iterates all active tokens for the tenant and verifies the provided secret against each
Argon2id hash. On match, it checks capability intersection and atomically increments `current_uses`. The `enrollment_token_id`
FK on the `services` table provides an audit trail of which token enrolled each service.

#### Wire protocol changes

`EnrollPayload` carries `capabilities: BTreeSet<Capability>` instead of the former `service_type: ServiceType`.
The `ServiceHandler::SERVICE_TYPE` constant has been removed from the service SDK trait.

#### Service connections

`service_connections.rs` provides a single `register()` method (replacing `register_agent()`,
`register_ssh_agent()`, `register_mqtt()`) and `broadcast_by_capability()` (replacing `broadcast_by_type()`).
`force_disconnect()` cancels the session's `CancellationToken` and removes the connection entry,
used by deactivate/reject/merge routes for immediate WebSocket session termination.

#### Controller events

The `controller_events` DB table has been dropped. Cross-controller event routing now uses NATS JetStream
(feature-gated behind the `nats` Cargo feature). When NATS is not configured, the controller operates in
single-instance mode with local-only delivery.

The NATS server URL is persisted in the global `settings` table under key `nats.url` (AES-256-GCM encrypted).
It is reconciled with the `--nats-url` CLI flag at startup using the standard 5-case priority. Changing the
URL via `PUT /api/v1/settings/nats` or `uptrakit settings nats set` requires a controller restart to take
effect. The `SettingsSnapshot.nats_url` field is a `MaskedUrl` that redacts the password in all
serialized/logged output. See `crates/ui/web-api-auth/src/setting_key.rs` (`NatsUrl` variant) and
`docs/development/nats-integration.md` for full details.

Zeroconf settings (`zeroconf.enabled`, `zeroconf.url`, `zeroconf.pki_addr`) are persisted in the global `settings`
table and reconciled with CLI flags (`--zeroconf`, `--zeroconf-url`, `--zeroconf-pki-addr`) at startup using the
standard 5-case priority. The in-memory cache is `ZeroconfSnapshot` (in `SettingsSnapshot`). REST API:
`GET/PUT /api/v1/global-settings/zeroconf`. See `docs/development/zeroconf-discovery.md` for full details.

#### REST API

`ServiceResponse` contains `capabilities: Vec<String>` and `service_label: String` instead of the former
`service_type: ServiceType`. The list endpoint filter parameter is `?capability=` instead of `?type=`.

### Two-tier service model

The controller manages two independent service tiers:

| Tier | Table | Scoped to | REST path |
| --- | --- | --- | --- |
| Tenant services | `services` | `tenant_id` | `/api/v1/services` |
| System services | `system_services` | Global | `/api/v1/system-services` |

Enrollment is routed by the presence of `Capability::SystemService` (`"system_service"`) in
`EnrollPayload.capabilities`. When present, `enroll_service()` calls `do_enroll_system_service()`
and writes to `system_services`; otherwise it calls `do_enroll()` and writes to `services`.

The `is_system: bool` flag derived at enrollment threads through every subsequent WebSocket
operation: certificate lookup (tries `service_certificates` then `system_service_certificates`),
activity recording, status checks, credential delivery, and registered-connection management.

The MQTT bridge now enrolls as a system service (`system_service + mqtt_bridge + graceful_shutdown`)
and appears in `/api/v1/system-services`, not `/api/v1/services`.

#### System services key files

| File | Purpose |
| --- | --- |
| `crates/shared/db/src/entity/system_service.rs` | SeaORM entity for `system_services` table |
| `crates/shared/db/src/entity/system_service_certificate.rs` | SeaORM entity for `system_service_certificates` table |
| `crates/shared/db/src/entity/system_enrollment_token.rs` | SeaORM entity for `system_enrollment_tokens` table |
| `crates/ui/web-api-queries/src/queries/system_services.rs` | DB query helpers (list, get, approve, reject, deactivate, update) |
| `crates/ui/web-api-queries/src/queries/system_enrollment_tokens.rs` | DB query helpers for system enrollment tokens |
| `crates/ui/web-api/src/routes/system_services.rs` | Route handlers for `/api/v1/system-services` |
| `crates/ui/web-api/src/routes/system_enrollment_tokens.rs` | Route handlers for `/api/v1/system-enrollment-tokens` |
| `crates/shared/web-api-types/src/system_services.rs` | `SystemServiceResponse`, `UpdateSystemServiceRequest`, `ListSystemServicesQuery` |
| `crates/shared/web-api-types/src/system_enrollment_tokens.rs` | `CreateSystemEnrollmentTokenRequest`, `SystemEnrollmentTokenCreatedResponse`, `SystemEnrollmentTokenResponse` |
| `crates/shared/openapi-client/src/system_services.rs` | Typed HTTP client methods for system service endpoints |
| `crates/shared/openapi-client/src/system_enrollment_tokens.rs` | Typed HTTP client methods for system enrollment token endpoints |
| `crates/ui/cli/src/commands/system_services.rs` | CLI `system-services` subcommand |
| `crates/ui/cli/src/commands/system_enrollment_tokens.rs` | CLI `system-enrollment-tokens` subcommand |
| `docs/architecture/system-services.md` | Full architecture documentation |

#### Credential guard

Four capabilities (`database_access`, `nats_access`, `master_key_access`, `ca_management`) require
`system_service` to be present in the same capability set. The guard runs in `do_enroll()` before
any DB write and rejects with `AgentRouteError::Forbidden` if a service requests system credentials
without `system_service`. This prevents tenant services from receiving infrastructure secrets.

#### System enrollment tokens

Multiple named system enrollment tokens are stored in the `system_enrollment_tokens` table
(`crates/shared/db/src/entity/system_enrollment_token.rs`). Tokens are backend-generated random
secrets, Argon2id-hashed at rest, and shown only once at creation. Each token supports optional
usage limits (`max_uses`) and TTL (`expires_at`).

At enrollment, if the service provides a token, `find_active_system_tokens()` retrieves all
non-revoked, non-expired tokens with remaining uses, then `password::verify_password()` performs
Argon2id verification. On match, `current_uses` is atomically incremented and
`system_enrollment_token_id` is recorded on the `system_services` row (audit-only, no FK
constraint — tokens can be revoked/deleted after the service has enrolled). A matching token
produces `Approved` status; no match produces `Forbidden`; no token produces `Pending`.

REST API: `POST/GET /api/v1/system-enrollment-tokens`, `GET/DELETE /api/v1/system-enrollment-tokens/{id}`
(requires `manage_system_services`).
OpenAPI client: `crates/shared/openapi-client/src/system_enrollment_tokens.rs`.
CLI: `uptrakit system-enrollment-tokens list|create|show|revoke`.

#### Frontend

The frontend filters services by capability instead of type and displays `service_label` instead of `service_type`.

#### Key files

| File | Purpose |
| --- | --- |
| `crates/shared/wire/src/capabilities.rs` | `Capability` enum, serde, `is_known()` |
| `crates/shared/wire/src/payloads.rs` | `EnrollPayload` with `capabilities` field |
| `crates/shared/wire/src/service_profile.rs` | `ServiceProfile` enum, `from_capabilities()`, `parse_capabilities()`, `serialize_capabilities()` |
| `crates/ui/web-api/src/service_connections.rs` | `register()`, `broadcast_by_capability()` |
| `crates/shared/db/src/entity/enrollment_token.rs` | `enrollment_tokens` SeaORM entity |
| `crates/ui/web-api/src/routes/enrollment_tokens.rs` | Enrollment token REST endpoints |
| `crates/ui/web-api-queries/src/queries/enrollment_tokens.rs` | Enrollment token DB queries |
| `crates/shared/service-sdk/src/connection.rs` | `agreed_capabilities` field + accessors |
| `crates/shared/service-sdk/src/event_loop.rs` | Capability intersection in `ServiceSettings` handler |
| `crates/ui/web-api/src/routes/service_ws/protocol.rs` | `controller_capabilities()`, `ServiceSettingsPayload` construction |
| `crates/ui/web-api/src/nats_transport.rs` | NATS JetStream transport (feature-gated) |
| `crates/ui/web-api/src/event_delivery.rs` | Shared delivery routing logic |
| `crates/shared/wire/asyncapi.yaml` | Schema for `capabilities` arrays in messages |
| `docs/api/wire-protocol.md` | Full capability negotiation documentation |

### Batch actions (group operations)

All management endpoints support batch operations via `POST /api/v1/{resource}/batch`. The request body
contains an `action` string and a `ids` UUID array (max 100). Responses use partial-success semantics:
each item independently succeeds or fails.

Endpoints: services, system-services, software-items, hosts, hosts/{host_id}/packages,
autodiscovery/ignores, plugin-configs. Full endpoint table and side-effect documentation in
[docs/api/batch-actions.md](docs/api/batch-actions.md).

Extensions can mark `ActionDef` as batch-capable via `.batch()` (sets `batch_action: true`). The SSH
agent marks `sync-host` and `remove-host` as batch-capable.

The frontend adds multi-select checkboxes to all list pages (services, system-services, software,
hosts, plugin-configs, software ignores) and extension DataTables. Selection
uses `SvelteSet<string>` (required by `svelte/prefer-svelte-reactivity` ESLint rule). A shared
`BatchActionBar` appears when items are selected; `BatchResultDialog` shows partial-success
results. See [docs/development/frontend-components.md](docs/development/frontend-components.md)
for component details.

#### Key files

| File | Purpose |
| --- | --- |
| `crates/shared/web-api-types/src/batch_actions.rs` | `BatchActionRequest`, `BatchActionResponse`, `BatchActionSuccess`, `BatchActionFailure`; `Validate` impl (max 100 IDs) |
| `crates/ui/web-api-queries/src/queries/services.rs` | `batch_approve_services`, `batch_reject_services`, `batch_deactivate_services` |
| `crates/ui/web-api-queries/src/queries/system_services.rs` | `batch_approve_system_services`, `batch_reject_system_services`, `batch_deactivate_system_services` |
| `crates/ui/web-api-queries/src/queries/software_items.rs` | `batch_delete_software_items` |
| `crates/ui/web-api-queries/src/queries/hosts.rs` | `batch_deactivate_hosts` |
| `crates/ui/web-api-queries/src/queries/software_ignores.rs` | `batch_delete_ignore_rules` |
| `crates/ui/web-api-queries/src/queries/plugin_configs.rs` | `batch_delete_plugin_configs` |
| `crates/ui/web-api/src/routes/services.rs` | `batch_services` handler |
| `crates/ui/web-api/src/routes/system_services.rs` | `batch_system_services` handler |
| `crates/ui/web-api/src/routes/software_items.rs` | `batch_software_items` handler |
| `crates/ui/web-api/src/routes/hosts.rs` | `batch_hosts` handler |
| `crates/ui/web-api/src/routes/software_ignores.rs` | `batch_software_ignores` handler |
| `crates/ui/web-api/src/routes/plugin_configs.rs` | `batch_plugin_configs` handler |
| `crates/shared/openapi-client/src/paths.rs` | `BATCH` path constants for all resources |
| `crates/shared/extension-framework/src/lib.rs` | `ActionDef.batch_action` field |
| `frontend/src/lib/types.ts` | `BatchActionRequest`, `BatchActionResponse` TypeScript types |
| `frontend/src/lib/api.ts` | `batchServices`, `batchHosts`, etc. API client functions |
| `frontend/src/lib/components/BatchActionBar.svelte` | Shared batch action toolbar (fixed-position, selected count + action buttons) |
| `frontend/src/lib/components/BatchResultDialog.svelte` | Shared partial-success results dialog |
| `frontend/src/lib/components/extensions/SchemaTable.svelte` | Extension DataTable with batch support for `batch_action: true` actions |

### Error handling quick reference

Every boundary (crate or module) must define its own typed error enum. Here is the minimal setup and decision guide.
Full reference with 19 patterns, anti-patterns, error chain diagrams, and a complete real-world example:
[docs/development/error-handling.md](docs/development/error-handling.md).

**Required imports:**

```rust
use rootcause::prelude::*;      // Report, markers, report!, bail!, ResultExt, IteratorExt, handlers, IntoRootcause
use thiserror::Error;            // #[derive(Debug, Error)]
use uptrakit_shared_macros::impl_report_conversion;  // cross-boundary conversions
```

**Boundary checklist:**

1. Define `#[derive(Debug, Error)] pub enum MyError { ... }`
1. Define `pub type Result<T> = std::result::Result<T, Report<MyError>>;`
1. Add `impl_report_conversion!` for every foreign error type your boundary encounters.

**`#[from]` vs `impl_report_conversion!`:** Omit `#[from]` on DB/foreign-error variants when the return type is
`Report<T>`. The `From` impl generated by `#[from]` is never called — the `?` operator cannot see through
`Report<T>`. Use `impl_report_conversion!` as the sole conversion mechanism and omit `#[from]` to avoid dead code.

**`Result<T>` alias coverage:** The `pub type Result<T>` alias must cover **all** functions in a query module,
including simple read-only functions that only fail with `sea_orm::DbErr`. Do not use a bare
`std::result::Result<T, SomeError>` signature for "simple" functions — use the module's unified `Result<T>`.

**Complete minimal example:**

```rust
use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_shared_macros::impl_report_conversion;

#[derive(Debug, Error)]
pub enum WidgetError {
    #[error("database error: {0}")]
    Database(sea_orm::DbErr),
    #[error("widget not found: {0}")]
    NotFound(uuid::Uuid),
}

pub type Result<T> = std::result::Result<T, Report<WidgetError>>;

impl_report_conversion!(sea_orm::DbErr => WidgetError::Database);

pub async fn get_widget(db: &DatabaseConnection, id: uuid::Uuid) -> Result<Widget> {
    Widget::find_by_id(id)
        .one(db).await.context_to()?
        .ok_or_else(|| report!(WidgetError::NotFound(id)))
}
```

**`bail!()` vs `report!()`:**

- `bail!(MyError::Variant(...))` — use for guard-clause early returns (replaces `return Err(report!(...))`).
- `report!(MyError::Variant(...))` — use inside `.ok_or_else()`, `.map_err()`, or when building a `Report` without
  returning.
- Do **not** use `Report::new()` directly; always use the `report!()` macro.

**Decision table — which context method to use:**

| Scenario | Method | Effect |
| --- | --- | --- |
| Foreign error has `ReportConversion` impl | `.context_to()` | Delegates to impl |
| Wrap low-level error with high-level meaning | `.context(Higher::Variant)` | Adds new parent node; original stays as child |
| Change error type in-place (1:1 mapping) | `.context_transform(\|e\| ...)` | Replaces context type; children preserved |
| One-off conversion, no impl needed | `.map_err(\|e\| report!(...))` | Manual wrap |
| Guard clause / early return | `bail!(...)` | Return immediately |

**Approved exceptions:**

- `Mutex::lock().unwrap()`, `RwLock::read().unwrap()`, `RwLock::write().unwrap()` — safe because `panic = "abort"` in
  release.
- String-based error variants for external types that don't impl `std::error::Error` (e.g. `aws_lc_rs::Unspecified`):
  `.map_err(|e| report!(Err::Variant(e.to_string())))`.
- Clap `value_parser` functions — `Result<T, String>` is required by the clap API (Pattern 14).
- HTTP input validation helpers — thin functions producing user-facing HTTP 400 error messages where the string goes
  directly into `error_response()` (Pattern 15).
- Display fallbacks — `unwrap_or_else` / `unwrap_or_default` for non-critical display/formatting (Pattern 16).

### Tracing initialization

**Libraries must never configure the global tracing dispatcher.**
Only binary `main()` functions may call `tracing_subscriber::fmt().init()` or any equivalent. Configuring the global
subscriber from a library causes a panic if anything else in the process has already set it (e.g. test harness,
another library).

**`uptrakit-service-sdk` does not provide `init_tracing()`.** Each binary (`uptrakit-agent`, `uptrakit-agent-ssh`,
`uptrakit-mqtt`, `uptrakit-scheduler`) owns its own `init_tracing()` helper in `src/main.rs`. `tracing-subscriber`
must appear in each binary's `[dependencies]`, not in shared library crates.

**Pattern:** Use `EnvFilter::from_default_env().add_directive(...)` with an `if let Ok(d) = directive.parse()`
fallback — do not use `.expect()` on the parse. This prevents a panic if the verbosity directive string is ever
malformed.

### Directory management

All binaries (controller, agent, MQTT service, scheduler) use the `uptrakit-directories` crate for cross-platform directory
resolution. The crate uses the `directories` crate (`ProjectDirs`) to follow platform conventions:

| Platform | Config directory | State directory |
| --- | --- | --- |
| Linux | `~/.config/{app}/` (XDG) | `~/.local/state/{app}/` (XDG) |
| macOS | `~/Library/Application Support/org.uptrakit.{app}/` | `~/Library/Application Support/org.uptrakit.{app}/` |
| Windows | `{FOLDERID_RoamingAppData}\uptrakit\{app}\` | `{FOLDERID_LocalAppData}\uptrakit\{app}\` |

Where `{app}` is one of: `controller`, `agent`, `agent-ssh`, `mqtt`, `scheduler`.

#### Config vs state separation

| Directory | Contents | Characteristics |
| --- | --- | --- |
| **Config** | Rarely-changing, persistent configuration | External CA certificates, user-provided TLS certs |
| **State** | Runtime state that may change frequently | SQLite DB, JWT keys, service identity, private keys, issued certificates |

**Controller:**

- Config: External CA certificate/key (if configured), server TLS certificate/key
- State: SQLite database (includes managed CA history, JWT signing key)

**Agent/MQTT Service:**

- Config: Controller's CA certificate
- State: Service ID, private key, issued certificate

**SSH Agent:**

- Config: Controller's CA certificate
- State: Service ID, private key, issued certificate, local SQLite DB (`agent-ssh.db` with encrypted SSH credentials)
- Runtime: `SshAgentHandler` holds `in_flight_update: Option<InFlightUpdate>` to enforce one-update-at-a-time; the SSH
  agent is feature-complete for version checks and updates over SSH (delegates to `uptrakit-agent-core`)

#### CLI directory flags

All binaries support `--config-dir` and `--state-dir` CLI flags (and corresponding `UPTRAKIT_CONFIG_DIR` /
`UPTRAKIT_STATE_DIR` environment variables) to override the platform defaults. Both support `~` expansion for home
directory paths.

#### CLI authentication environment variables

The `uptrakit` CLI binary also supports:

| Variable | Description |
| --- | --- |
| `UPTRAKIT_SERVER` | Controller URL (equivalent to `--server`) |
| `UPTRAKIT_TOKEN` | API token (equivalent to `--token`) |
| `UPTRAKIT_TIMEOUT` | API request timeout in seconds (equivalent to `--timeout`; default: 30). Useful for CI pipelines or operations that may take longer than 30 s. |

**Priority:** CLI flag > environment variable > stored credentials file. Using `UPTRAKIT_TOKEN` is preferred over
`--token` in automation to avoid exposing tokens in process listings.

#### Secure permissions

All created files and directories use secure permissions:

- **Directories**: 0o700 (owner read/write/execute only)
- **Files**: 0o600 (owner read/write only)

The `uptrakit-directories` crate provides helper functions (permissions are set **atomically at creation time** on
Unix, eliminating TOCTOU windows):

- `create_secure_dir(path)` -- async; creates directory with 0o700 permissions using `tokio::fs`
- `write_secure_file(path, data)` / `write_secure_file_str(path, str)` -- async; atomically writes file with 0o600
  permissions (write-to-temp-then-rename on same filesystem)
- `AppDirs::resolve(app_kind, config_override, state_override)` -- resolves directories for an application
- `AppDirs::config_path(name)` / `AppDirs::state_path(name)` -- returns `Result<PathBuf>` after validating `name`
  against path traversal (rejects path separators, `..`, `.`, empty strings, absolute paths)
- `AppDirs::ensure_config_dir()` -- async; creates config directory with secure permissions
- `AppDirs::ensure_state_dir()` -- async; creates state directory with secure permissions
- `AppDirs::ensure_dirs()` -- async; creates both directories with secure permissions

All crates writing sensitive files (private keys, certificates, CA bundles) **must** use these helpers instead of raw
`fs::write` / `tokio::fs::write`.

#### Key files

| File | Purpose |
| --- | --- |
| `crates/shared/directories/src/lib.rs` | Cross-platform directory resolution and secure file/directory operations |

## Notification subsystem

The controller includes a channel-agnostic notification subsystem. Event producers emit `NotificationEvent` values
(internal, never exposed to channels). A fire-and-forget `NotificationDispatcher` matches events against
tenant-scoped rules, builds a `DeliveryMessage`, and hands it to the appropriate channel for delivery.
The dispatcher uses a **bounded** `mpsc::channel(DISPATCHER_CHANNEL_CAPACITY)` (capacity 4096); events that
overflow are dropped with a `tracing::warn!` rather than causing unbounded heap growth.

### Key crates and modules

| Crate/module | Purpose |
| --- | --- |
| `crates/plugins/notifications/core/` | `NotificationPlugin` trait (with `restore_config_secrets` default method), `DeliveryMessage` (`#[non_exhaustive]`, `::new()`), `MessageAction` (`#[non_exhaustive]`, `::new()`), `NotificationPluginError`, `escape_html()` |
| `crates/plugins/notifications/webhook/` | Webhook plugin (SSRF validation + header blocklist + HMAC-SHA256 signing) |
| `crates/plugins/notifications/telegram/` | Telegram plugin with inline keyboard |
| `crates/plugins/notifications/email/` | Email plugin (SMTP via mail-send, `SmtpSettingsSnapshot`, `merge_smtp_into_config()`) |
| `crates/plugins/notifications/registry/` | `NotificationPluginRegistry`, `NotificationOps` trait (includes `restore_config_secrets`, `extension_manifests`, `extension_actions`), `NotificationRegistryConfig`; re-exports core types |
| `crates/plugins/notifications/registry/src/extensions/` | Per-transport `ExtensionManifest` and `ActionDef` definitions (webhook, telegram, email); only crate with transport-specific UI knowledge |
| `crates/ui/web-api/src/routes/notification_extensions.rs` | Generic extension data action handler (channel listing with config flattening) + SMTP settings handler |
| `crates/shared/web-api-types/src/notifications.rs` | Shared request/response types, `NotificationEventType`, `NotificationChannelType`, `NotificationDeliveryStatus` enums |
| `crates/ui/web-api/src/notifications/` | Internal `NotificationEvent`, `NotificationDispatcher`, `message_builder` |
| `crates/ui/web-api/src/routes/notifications.rs` | REST API route handlers (channels, rules, log, telegram callback) |
| `crates/ui/web-api-queries/src/queries/notifications.rs` | CRUD query helpers using `TenantDb` |
| `crates/shared/openapi-client/src/notifications.rs` | Typed HTTP client methods |
| `crates/ui/cli/src/commands/notifications.rs` | CLI `notifications` command group |

### Feature flags

| Feature | Crate | Default | Notes |
| --- | --- | --- | --- |
| `webhook` | notification-plugin-registry | yes | Always compiled |
| `telegram` | notification-plugin-registry | no | Requires `teloxide-core` |
| `email` | notification-plugin-registry | no | SMTP via mail-send (rustls) |
| `notifications-telegram` | web-api, controller | no | Propagated to notification-plugin-registry |
| `notifications-email` | web-api, controller | no | Propagated to notification-plugin-registry; requires global SMTP settings configured via `PUT /api/v1/settings/smtp` |

### Event types

`update_available`, `update_completed`, `update_failed`, `new_software_discovered`, `new_service_enrolled`,
`ca_rotated`, `batch_update_completed`, `batch_update_partially_completed`, `stdin_attention`.
Events are wired into existing WebSocket handlers (`messages.rs`, `updates.rs`), `services.rs`,
and `settings_ca.rs`. The `stdin_attention` event is dispatched when an interactive update appears
to be waiting for stdin input.

### Permissions

`ViewNotifications` (view channels, rules, log) and `ManageNotifications` (create/edit/delete channels and rules).

### User management

The system uses 32 granular permissions grouped into 8 built-in roles (`viewer`, `operator`,
`service_manager`, `software_manager`, `host_manager`, `settings_manager`, `command_manager`,
`system_administrator`). Five access presets (`read_only`, `operator`, `manager`, `administrator`,
`owner`) provide convenient role bundles. The first registered user receives all 8 roles (owner preset);
subsequent users receive only `viewer`.

User management endpoints (`/api/v1/users`, `/api/v1/roles`, `/api/v1/permissions`,
`/api/v1/access-presets`) require the `ManageUsers` permission. Lockout prevention rejects
changes that would leave no user with `manage_users`.

Key files: `crates/shared/types/src/permissions.rs` (32 `Permission` variants),
`crates/shared/types/src/access_preset.rs` (`AccessPreset` enum),
`crates/ui/web-api/src/routes/users.rs`, `crates/ui/web-api/src/routes/access_presets.rs`,
`crates/shared/web-api-types/src/users.rs`, `crates/shared/web-api-types/src/access_presets.rs`.

See [Authentication and Authorization](docs/security/auth-and-authorization.md) for the full
permission model and [User Management API](docs/api/user-management.md) for the endpoint reference.

### Adding a new channel

1. Create a new crate under `crates/plugins/notifications/<name>/`
2. Implement `NotificationPlugin` trait
3. Register in `NotificationPluginRegistry::new()` behind `#[cfg(feature = "...")]`
4. Add feature in `crates/plugins/notifications/registry/Cargo.toml`
5. Add variant to `NotificationChannelType` enum in web-api-types
6. Propagate feature: `web-api/Cargo.toml` → `controller/Cargo.toml`
7. HTML-escape all user-controlled values in `body_html` via `uptrakit_notification_plugin_core::escape_html()`

See [Notifications Development](docs/development/notifications.md) for full details.

## Audit log subsystem

The controller records all authenticated HTTP requests through a pluggable audit log subsystem. It follows the same
fire-and-forget `mpsc::UnboundedSender` dispatcher pattern as notifications.

### Key crates and modules

| Crate/module | Purpose |
| --- | --- |
| `crates/shared/audit-log/` | `AuditLogBackend` trait, `AuditEntry`, `AuditFilter`, `AuditLogDispatcher`, `NoopBackend`, `DatabaseBackend`, `JournaldBackend`, `MultiplexBackend` |
| `crates/shared/db/src/entity/audit_log.rs` | SeaORM entity for `audit_logs` table (tenant-scoped, no FK on `tenant_id`) |
| `crates/shared/db/src/entity/system_audit_log.rs` | SeaORM entity for `system_audit_logs` table (global) |
| `crates/ui/web-api/src/middleware/audit_log.rs` | Axum middleware (runs inside `require_auth`); detects system routes by prefix (`/api/v1/global-settings/`, `/api/v1/system-services`) |
| `crates/ui/web-api-queries/src/queries/audit_logs.rs` | `list_tenant_audit_logs` + `list_system_audit_logs` with filter/pagination support |
| `crates/ui/web-api/src/routes/audit_logs.rs` | `GET /api/v1/audit-logs` (`CanViewAuditLogs`) and `GET /api/v1/system-audit-logs` (`CanViewSystemAuditLogs`) |
| `crates/shared/web-api-types/src/audit_logs.rs` | `AuditLogResponse`, `SystemAuditLogResponse`, `AuditLogListParams` |
| `crates/shared/openapi-client/src/audit_logs.rs` | `list_audit_logs` + `list_system_audit_logs` client methods |
| `crates/ui/cli/src/commands/audit_logs.rs` | `audit-logs list` (tenant) and `audit-logs system list` (system) CLI subcommands |
| `crates/ui/web-api-auth/src/setting_key.rs` | `AuditLogFilter` + `AuditLogRetentionDays` setting keys |
| `crates/ui/web-api/src/app_state.rs` | `audit_log_filter` + `audit_log_dispatcher` fields |
| `crates/core/controller/src/cli.rs` | `AuditLogBackendArg`, `AuditLogFilterArg` enums + CLI flags |
| `crates/core/controller/src/main.rs` | Backend construction + AppState wiring |
| `crates/core/controller/src/startup.rs` | `init_audit_database()` for separate audit DB |
| `crates/shared/scheduler-engine/src/executors/audit_log_cleanup.rs` | Retention cleanup (90-day default) |

### Feature flags

| Feature | Crate | Default | Notes |
| --- | --- | --- | --- |
| `db` | audit-log | no | Enables `DatabaseBackend` (sea-orm + shared-db) |
| `journald` | audit-log | no | Enables `JournaldBackend` (tracing-journald) |
| `journald` | controller | no | Propagated; adds `tracing-journald` dep |

### System-route detection

The middleware detects global-infrastructure routes and sets `tenant_id = None` so entries go to
`system_audit_logs` instead of `audit_logs`. Detection is by URL prefix:

- `/api/v1/global-settings/` (or exactly `/api/v1/global-settings`) → `system_audit_logs`
- `/api/v1/system-services/` (or exactly `/api/v1/system-services`) → `system_audit_logs`
- All other authenticated routes → `audit_logs`

When adding a new global-infrastructure endpoint group under a new prefix, update the detection
logic in `crates/ui/web-api/src/middleware/audit_log.rs`.

### Middleware placement

The `audit_log` middleware is an **inner** route_layer on `auth_routes`, declared before `require_auth`. This means
it runs **after** auth (inner layers execute after outer layers in Axum):

```rust
let auth_routes = auth_routes
    .route_layer(audit_log_layer)    // inner: runs AFTER require_auth
    .route_layer(require_auth_layer); // outer: runs FIRST
```

### Setting keys

`AuditLogFilter` (`audit_log.filter`) — per-tenant override of the global `--audit-log-filter` CLI flag.
`AuditLogRetentionDays` (`audit_log.retention_days`) — per-tenant retention period (future use).

### Default `NoopBackend` in tests

`AppState` uses `unwrap_or_else` defaults: `AuditFilter::default()` and
`AuditLogDispatcher::new(Arc::new(NoopBackend))`. Existing tests require zero changes.

### `reject_dangerous_commands` flag

`AppState.reject_dangerous_commands: bool` — dangerous command rejection is **enabled by
default**. Plugin config create/update requests containing dangerous command patterns are
rejected with HTTP 400. Operators can disable this with the `--allow-dangerous-commands` CLI
flag or `UPTRAKIT_ALLOW_DANGEROUS_COMMANDS` env var, which sets the internal flag to `false`.
The CLI flag inversion happens in `crates/core/controller/src/main.rs`:
`reject_dangerous_commands(!args.allow_dangerous_commands)`.

### Design decisions

- **No FK on `audit_logs.tenant_id`** — audit records are immutable and must survive tenant deletion for compliance.
- **`AuditActorType` is internal-only** — follows `ActorType`/`BatchType` pattern: `Copy`, `as_str()` + `Display`,
  not `#[non_exhaustive]`, no `Other(String)`.
- **Multiple backends via repeatable CLI flag** — `--audit-log-backend db --audit-log-backend journald` fans out
  concurrently via `MultiplexBackend`.
- **No request/response body logging** — only metadata (method, path, status, actor, IP, duration).

See [Audit Logs Development](docs/development/audit-logs.md) and [Audit Logs Security](docs/security/audit-logs.md)
for full details.

## UI Extensions Framework

The extensions framework allows connected services and plugins to dynamically extend the
web UI, REST API, and CLI with custom functionality. Extensions are described by
`ExtensionManifest` structs registered at runtime (services) or compile-time (plugins).

### Key files

| File | Purpose |
| --- | --- |
| `crates/shared/extension-framework/src/lib.rs` | Extension types: `ExtensionManifest`, `ExtensionUi`, `ActionDef`, `FieldDef`, etc. (`uptrakit-extension-framework`) |
| `crates/shared/wire/src/extension.rs` | Re-exports `uptrakit-extension-framework` for backward compatibility |
| `crates/ui/web-api/src/extension_registry.rs` | Runtime registry: tracks manifests and provider sets |
| `crates/ui/web-api/src/extension_proxy.rs` | Controller-side request/response proxy via oneshot channels (frontend → service) |
| `crates/shared/service-sdk/src/extension_proxy.rs` | Service-side proxy for invoking controller plugin actions (service → controller) |
| `crates/ui/web-api/src/routes/extensions.rs` | REST endpoints: list, providers, invoke |
| `crates/shared/service-sdk/src/lifecycle.rs` | `ServiceHandler::on_extension_request` + `on_extension_response` default impls |
| `crates/shared/service-sdk/src/event_loop.rs` | Dispatches `ExtensionRequest` + `ExtensionResponse` to handler |
| `crates/ui/cli/src/commands/extensions.rs` | CLI: `extensions list`, `providers`, `invoke` |
| `frontend/src/lib/extensions.svelte.ts` | Svelte extension store |
| `frontend/src/lib/components/extensions/` | Schema-driven UI components |

### Registration rules

- Same extension ID from same `service_app_name`: allowed (providers are deduplicated).
- Same extension ID from different `service_app_name`: rejected with `ErrorCode::BadRequest`.
- On disconnect: service removed from provider set; extension removed if no providers remain.

### Targeting model

- **`Universal`**: any connected instance can handle actions (controller picks one).
- **`Targeted`**: user must select a specific service instance; frontend shows a selector.

### Capability

Extensions require the `UiExtensions` capability. Services without this capability cannot
register extensions or receive extension requests.

### Frontend consistency

Extension pages must have the same look and feel as built-in pages. Extension components
(`SchemaTable`, `SchemaForm`, `SchemaKeyValue`, `ActionButton`) use the same Skeleton UI
classes and shared components (`Pagination`, `Modal`, `ConfirmDialog`) as built-in pages.
Key conventions:

- **Tables**: `<div class="table-wrap"><table class="table">` — same as built-in pages.
- **Forms**: Skeleton's `.label` class wrapping each field — same as built-in modal forms.
- **Empty states**: Two-line pattern inside `<td colspan>` — title + subtitle.
- **Page headings**: `<h1 class="h1 mb-6">` — same as all built-in pages.
- **Pagination**: Shared `Pagination` component with page number buttons, ellipsis gaps,
  and total count.

### Pagination convention

All `data_table` data actions must return paginated responses in the format
`{ items, total, page, per_page, total_pages }`. The frontend sends `page` and `per_page`
parameters with every request. Backend handlers must use DB-level `offset`/`limit` pagination
(not in-memory slicing). The `ExtensionUi::DataTable` variant has an optional `default_per_page`
field to override the frontend default of 20 items per page.

### Bidirectional invocation

Services can invoke controller-side plugin actions via `ServiceMessage::ExtensionRequest`.
The controller dispatches to the plugin and responds with `ControllerMessage::ExtensionResponse`.
The `ServiceExtensionProxy` (in `uptrakit-service-sdk`) provides the oneshot-channel correlation
pattern. This enables cross-plugin coordination (e.g., SSH agent querying the Proxmox plugin)
without direct crate dependencies. Both message types are session-targeted (not NATS-publishable).

See [Extensions Development](docs/development/extensions.md), [Extensions Architecture](docs/architecture/extensions.md),
and [Extensions Security](docs/security/extensions.md) for full details.

## Detailed Documentation References

For more in-depth information on specific topics, refer to the following documents:

### Security

- [PKI and Certificate Lifecycle](docs/security/pki-certificates.md)
- [Secrets Handling and Encryption](docs/security/secrets-and-encryption.md) (includes master key verification for HA safety)
- [TOFU and TLS Hardening](docs/security/tofu-tls.md)
- [Authentication and Authorization](docs/security/auth-and-authorization.md)
- [Cryptography](docs/security/cryptography.md)
- [Security Architecture](docs/security/security-architecture.md)
- [Filesystem and Dependency Security](docs/security/filesystem-dependency-security.md)
- [Reverse Proxy Security](docs/security/reverse-proxy-security.md)
- [SSH Agent Secrets](docs/security/ssh-agent-secrets.md)
- [Sudoers Management](docs/security/sudoers-management.md)
- [Notifications Security](docs/security/notifications-security.md)
- [Audit Logs Security](docs/security/audit-logs.md)
- [Extensions Security](docs/security/extensions.md)

### End-user Guides

- [CLI Usage Guide](docs/end-user/cli-usage.md)
- [Plugin Configurations](docs/end-user/plugin-configs.md)
- [Update History](docs/end-user/update-history.md)
- [Profile and API Tokens](docs/end-user/profile-tokens.md)
- [Autodiscovery](docs/end-user/autodiscovery.md)
- [Update Workflow](docs/end-user/update-workflow.md)
- [Home Assistant and MQTT Integration](docs/end-user/home-assistant-mqtt.md)
- [Extensions](docs/end-user/extensions.md)

### Development Guidelines

- [Quality Gates](docs/development/quality-gates.md)
- [Commit Messages](docs/development/commit-messages.md)
- [CLI Output Formatting](docs/development/cli-output.md)
- [Graceful Restart](docs/development/graceful-restart.md)
- [Cross-Controller Communication](docs/development/cross-controller-comm.md)
- [NATS Integration](docs/development/nats-integration.md)
- [Coding Standards](docs/development/coding-standards.md)
- [Error Handling](docs/development/error-handling.md)
- [Testing Expectations](docs/development/testing.md)
- [Plugin Guidelines](docs/development/plugin-guidelines.md)
- [Plugin System Architecture](docs/development/plugin-system.md)
- [Command Executor](docs/development/command-executor.md)
- [Update Hooks](docs/development/update-hooks.md)
- [Service Lifecycle](docs/development/service-lifecycle.md)
- [OpenAPI Client](docs/development/openapi-client.md)
- [Embedded Frontend](docs/development/embedded-frontend.md)
- [Logging](docs/development/logging.md)
- [Tracing Conventions](docs/development/tracing.md)
- [Notifications](docs/development/notifications.md)
- [Audit Logs](docs/development/audit-logs.md)
- [Docker](docs/development/docker.md)
- [Extensions](docs/development/extensions.md)
- [Proxmox Bootstrap Privileges](docs/development/proxmox-bootstrap.md)

### Architecture

- [Multi-Tenancy](docs/architecture/multi-tenancy.md)
- [Host Entity](docs/architecture/host-entity.md)
- [Software Item Entity](docs/architecture/software-item-entity.md)
- [Update History Entity](docs/architecture/update-history-entity.md)
- [Scheduler](docs/architecture/scheduler.md)
- [Scheduler Engine](docs/development/scheduler-engine.md)
- [External Scheduler Deployment](docs/end-user/deployment/external-scheduler.md)
- [SSH Agent](docs/architecture/ssh-agent.md)
- [Extensions](docs/architecture/extensions.md)
- [Host Tags](docs/architecture/host-tags.md)

### API and Protocol

- [Wire Protocol](docs/api/wire-protocol.md)
- [Authentication Flows](docs/api/auth-flows.md)
- [Settings Runtime](docs/api/settings-runtime.md)
- [HTTP Web API](docs/api/http-web-api.md)
- [Services and Operations](docs/api/services-operations.md)
- [Autodiscovery](docs/api/autodiscovery.md)
- [Extensions](docs/api/extensions.md)
- [Host Tags](docs/api/host-tags.md)
