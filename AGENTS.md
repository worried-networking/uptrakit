# AGENTS -- AI Agent Guide for Uptrakit

This file provides structured context for AI coding agents working on the Uptrakit codebase. Read this first before
making any changes.

## Project summary

Uptrakit is an agent-based update tracking toolkit for self-hosted Linux environments. It tracks installed software
versions across remote hosts, checks for updates, and allows **manual, user-triggered** updates. It is **not** an
auto-updater.

Key components:

- **Controller** (server): API, Web UI, optional embedded scheduler, upstream version checking.
- **External Scheduler** (standalone binary): enrolls as a service, receives DB/NATS/master-key credentials via
  WebSocket, runs scheduled tasks independently.
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
  integration, deployment map, CLI usage guide, plugin configurations, update history, profile and API
  tokens, and autodiscovery (including
  [docs/end-user/deployment/reverse-proxy.md](docs/end-user/deployment/reverse-proxy.md)).
- **API & protocol docs** ([`docs/api/`](docs/api/)): AsyncAPI/wire protocol
  ([wire-protocol.md](docs/api/wire-protocol.md)), REST API endpoints ([http-web-api.md](docs/api/http-web-api.md)),
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
  [`docs/security/reverse-proxy-security.md`](docs/security/reverse-proxy-security.md). Human documentation must link
  into those files rather than [AGENTS.md](AGENTS.md).

## Codebase layout

```text
uptrakit/
├── Cargo.toml                          # Workspace root (resolver = "3", members = "crates/*/*")
├── crates/
│   ├── core/
│   │   ├── agent/                      # uptrakit-agent                         (bin)  — agent daemon
│   │   ├── agent-ssh/                  # uptrakit-agent-ssh                     (bin)  — SSH-backed agent; parallel per-host version checks and updates over SSH (per-host concurrency guard + forwarder task + aggregate mpsc channel); host management CLI, SSH transport (russh), SshTarget parser, ~/.ssh/config resolution, remote host info collection & ReportHosts; SshStdioTunnel (bidirectional byte-stream over russh channel for Docker proxy)
│   │   ├── controller/                 # uptrakit-controller                    (bin)  — central server; migration runner delegates to `uptrakit_shared_db::migration`
│   │   │   ├── src/db_migrate/         #   `db-migrate` subcommand: copies all data between DB backends; error.rs (DbMigrateError + Report<> Result), tables.rs (migrate_table<E>, copy_all, clean_all, verify_all for all 34 app tables), mod.rs (run() orchestrator)
│   │   │   ├── src/scheduler/          #   (cfg: embedded-scheduler) Embedded scheduler using uptrakit-scheduler-engine
│   │   │   └── src/embedded_frontend.rs #  (cfg: embed-frontend) Serves frontend from binary via rust-embed
│   │   ├── mqtt/                       # uptrakit-mqtt                          (bin)  — standalone MQTT service
│   │   └── scheduler/                  # uptrakit-scheduler                     (bin)  — external scheduler binary; enrolls as a service, receives credentials, runs scheduled tasks via direct DB + NATS
│   ├── plugins/
│   │   ├── infrastructure/
│   │   │   ├── core/                   # uptrakit-plugin-infrastructure-core                   (lib)  — plugin trait + SecretMasking; re-exports tokio::sync::mpsc; defines PluginCapability, HostCompatibility, UpdateHookContext, PreUpdateHookResult
│   │   │   └── registry/              # uptrakit-plugin-infrastructure-registry               (lib)  — plugin dispatch & validation; `daemon` feature (default) enables Docker local ops
│   │   ├── releases/
│   │   │   ├── docker/                 # uptrakit-plugin-releases-docker                 (lib)  — Docker/OCI plugin: tag tracking, SHA digest tracking, image pull via bollard, container autodiscovery; `daemon` feature (default) gates bollard + local Docker ops; Docker-over-SSH via StdioTunnel proxy (unix socket bridge to `docker system dial-stdio`)
│   │   │   └── github/                 # uptrakit-plugin-releases-github                 (lib)  — GitHub Releases plugin: controller-side fetch_releases only; owner/repo parsed from package_identifier at call time (format "owner/repo"); exports validate_identifier
│   │   ├── generic/
│   │   │   └── shell/                  # uptrakit-plugin-generic-shell                  (lib)  — generic agent-side plugin: version_command (detect_installed_version) + update_command (execute_update); supports {package_identifier}, {version}, {tag} placeholders; at least one field required
│   │   ├── package-managers/
│   │   │   ├── homebrew/               # uptrakit-plugin-package-manager-homebrew               (lib)  — Homebrew formulae/cask plugin; implements DetectHostCompatibility (checks `which brew`)
│   │   │   ├── apt/                    # uptrakit-plugin-package-manager-apt                    (lib)  — APT (Debian/Ubuntu) plugin (discovery via dpkg/apt-mark, version detection via dpkg-query, latest via apt-cache madison, updates via sudo apt-get install); implements DetectHostCompatibility (checks `which apt-get`) and PostUpdateHook (checks /var/run/reboot-required)
│   │   │   └── npm/                    # uptrakit-plugin-package-manager-npm                    (lib)  — npm global-package plugin; ControllerSideFetchReleases (queries registry.npmjs.org); discovery via `npm list -g --json`; updates via `sudo npm install -g <pkg>@<version>`; implements DetectHostCompatibility (checks `which npm`); validate_identifier exported for registry
│   │   └── discovery/
│   │       └── proxmox-helper-scripts/ # uptrakit-plugin-discovery-proxmox-helper-scripts (lib)  — PVE helper-scripts plugin (discovery-only: fetches CT scripts, analyzes for GitHub/npm/APT upstream; emits ReleasesGithub+GenericShell targets for GitHub-managed items, PackageManagerNpm target for npm-managed items, PackageManagerApt target for APT-managed items)
│   ├── shared/
│   │   ├── agent-core/                 # uptrakit-agent-core                    (lib)  — shared agent logic: version check, update execution, handle_check_versions/execute_update/graceful_shutdown; start_update() for per-host parallel use by SSH agent
│   │   ├── command/                    # uptrakit-command                       (lib)  — CommandExecutor trait + LocalCommandExecutor; SudoAwareCommandExecutor (wraps any executor, prepends sudo based on SudoContext); SudoPolicy enum (auto/force_with/force_without); CommandSpec.privileged flag; StdioTunnel trait (bidirectional byte-stream tunnel for remote command I/O)
│   │   ├── crypto/                     # uptrakit-crypto                        (lib)  — AES-256-GCM at-rest encryption (EncryptedString, init_master_key)
│   │   ├── db/                         # uptrakit-shared-db                     (lib)  — SeaORM entities; `migration` feature flag exposes `uptrakit_shared_db::migration::{Migrator, run_migrations}`
│   │   ├── directories/                # uptrakit-directories                   (lib)  — cross-platform directory management
│   │   ├── macros/                     # uptrakit-shared-macros                 (lib)  — shared macros (impl_report_conversion!)
│   │   ├── types/                      # uptrakit-shared-types                  (lib)  — shared value types (PluginRole, PluginType, etc.); feature-gated: sea-orm, openapi
│   │   ├── web-api-types/              # uptrakit-web-api-types                 (lib)  — shared HTTP request/response types
│   │   ├── openapi-client/             # uptrakit-openapi-client                (lib)  — typed HTTP client; full REST API + SSE streaming coverage; re-exports web-api-types, reqwest::Error; feature `mock` adds MockApiServer+MockEndpoint for integration testing; sse.rs provides lightweight SSE parser; update_output_stream.rs provides typed stream_update_output() method
│   │   ├── nats/                       # uptrakit-nats                          (lib)  — shared NATS primitives: NatsEventEnvelope, NatsConnection, subject routing, stream setup
│   │   ├── scheduler-engine/           # uptrakit-scheduler-engine              (lib)  — scheduler core: poll loop, claim mechanism, cron utils, TaskExecutor trait, SchedulerNotifier trait, 4 built-in executors
│   │   ├── service-sdk/                # uptrakit-service-sdk                   (lib)  — service lifecycle, SDK-managed event loop, signal handling, enrollment, identity, TLS, CA bootstrap, main helpers; default_resolve_shutdown(), init_tracing()
│   │   ├── update-hooks/               # uptrakit-update-hooks                  (lib)  — update hook resolution and config merge logic (extracted from web-api)
│   │   └── wire/                       # uptrakit-internal-wire                 (lib)  — service↔controller wire protocol; `Capability` enum + capability negotiation; `ServiceProfile` enum + from_capabilities(); `duration_seconds` serde module for Duration↔u32 fields
│   └── ui/
│       ├── cli/                        # uptrakit-cli                           (bin+lib) — CLI interface; uses openapi-client for all API calls (hosts, services, software-items, plugin-configs, autodiscovery, checks, updates, history, scheduler, settings); `update trigger --follow` and `history tail` use SSE streaming; lib target exposes modules for integration tests
│       └── web-api/                    # uptrakit-web-api                       (lib)  — HTTP API; split into app_state.rs, ca_snapshot.rs, router.rs modules; auth/auth_method.rs hosts AuthMethod
├── frontend/                           # SvelteKit SPA (Skeleton UI v4 + Tailwind CSS v4)
│   ├── src/
│   │   ├── lib/                        # Shared modules: api client, auth store, types, utils, notifications, sse.ts (SSE connection utility)
│   │   │   └── components/             # Shared UI: ConfirmDialog, ModalBackdrop (focus-trapped), ContextMenu (viewport-aware, keyboard-navigable), Pagination, TerminalOutput (xterm.js wrapper with dark/light theme)
│   │   └── routes/                     # SvelteKit file-based routes
│   │       ├── profile/                #   /profile — account info + API token management (create/revoke)
│   │       ├── history/                #   /history — update history with filters (host, software item, status) + trigger update button
│   │       ├── scheduler/              #   /scheduler — scheduler task management (edit cron, enable/disable, trigger now)
│   │       ├── plugin-configs/         #   /plugin-configs — plugin config CRUD + autodiscovery ignore rules management
│   │       ├── software/               #   /software — software item list; Pending tab for discovered items; Edit action (name/enabled)
│   │       └── settings/               #   Settings sub-components (Registration, Auth, MQTT, OIDC, Certs, Enrollment); global CA rotation; MQTT client limit
│   ├── package.json                    # npm scripts: build, check, lint, format, format:check
│   ├── svelte.config.js                # SvelteKit config (static adapter)
│   ├── tsconfig.json
│   ├── vite.config.ts
│   └── vitest.config.ts
├── .github/
│   ├── workflows/ci.yml                # CI: fmt check, clippy, tests, reverse-proxy Docker tests, frontend lint + format + check + build
│   └── dependabot.yml                  # Weekly Cargo + npm dependency updates
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
| `embedded-scheduler` | No | Embeds the scheduler engine in the controller process. Auto-disables when an external scheduler connects. Adds `uptrakit-scheduler-engine` dependency. |
| `nats` | No | Enables NATS JetStream transport for cross-controller messaging. Propagates to `uptrakit-web-api/nats`. |
| `swagger-ui` | No | Swagger UI at `/swagger-ui` |
| `embed-frontend` | No | Embeds the SvelteKit frontend build into the binary via `rust-embed`. Requires `frontend/build/` to exist at compile time. Removes the `--static-dir` CLI argument. See [Embedded Frontend](docs/development/embedded-frontend.md). |

### Web-API feature flags

| Feature | Default | Description |
| --- | --- | --- |
| `oidc` | Yes | OpenID Connect authentication. Gates the `openidconnect` dependency and all OIDC-specific modules (`oidc_auth`, `oidc_providers`, `oidc_state`), routes, OpenAPI schemas, rate limit entries, and `AppState` stores. Non-OIDC types (`AuthMethod::Oidc`, `require_token_for_oidc`, OIDC DB entities) remain unconditional. |
| `swagger-ui` | No | Swagger UI at `/swagger-ui` |
| `db-sqlite` | No | SQLite backend |
| `db-postgres` | No | PostgreSQL backend |
| `db-mysql` | No | MySQL backend |
| `db-all` | No | All database backends |

## General MUST FOLLOW Rules for AI Coding Agents

### Quality Gates

All changes must pass defined quality gates. See [docs/development/quality-gates.md](docs/development/quality-gates.md) for details.

#### AI execution guidance

- Always run quality gates relevant to modified areas before finalizing.
- **Always lint Markdown** with `markdownlint --config .markdownlint.json '**/*.md'` for all changes (not just docs).
  The `.markdownlintignore` file excludes `node_modules/`, `target/`, `.claude/`, and `CODEREVIEW.md`.
  Do not add exceptions to `.markdownlintignore` or `.markdownlint.json` without explicit approval.
- Scope-based execution is allowed for local iteration:
  - frontend-only changes: run frontend checks (`npm run lint`, `npm run format:check`, `npm run check`, `npm run build`) and markdownlint.
  - Rust/backend-only changes: run Rust checks/tests/linters and markdownlint.
  - mixed changes: run both Rust and frontend gates plus markdownlint.
- If anything related to reverse proxy behavior changes, run ignored reverse proxy integration tests:
  - `cargo test -p uptrakit-controller reverse_proxy -- --ignored`
- Treat the reverse proxy trigger list broadly, including (non-exhaustive):
  - mTLS and certificate forwarding/extraction
  - auth behavior behind proxies
  - IP detection / `ClientIp`, forwarded headers, trusted-proxy logic
  - reverse proxy middleware/settings and related TLS behavior

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
   Sudoers files are generated by `host bootstrap` / `host update-sudoers` and contain one entry per resolved command.
   See [Sudoers Management](docs/security/sudoers-management.md).
1. **Plugin split and role-based assignment.** Each `(host, software_item)` pair has per-role plugin assignments
   stored in the `host_software_item_plugins` table. Three roles exist: `detect_version`, `fetch_releases`, and
   `execute_update` (see `PluginRole` enum in `crates/shared/types/src/plugin_role.rs`). Each assignment carries an
   `execution_site` column (`auto`, `agent`, or `controller`) that determines where the operation runs. Plugins
   declaring the `ControllerSideFetchReleases` capability (e.g. GitHub, Docker, npm) have their `fetch_releases`
   executed on the controller by default; local package-index plugins (Homebrew, APT) run agent-side via
   `RefreshPackageIndex` + `fetch_releases()` and report `latest_version` in `VersionCheckResult`. Per-host version
   tracking (`installed_version`, `latest_version`) lives on `host_software_items`; the old centralised
   `available_versions` table has been removed. Keep this boundary clear.
1. **No shell injection.** Any path that constructs or executes shell commands must validate inputs. Custom scripts are
   treated as untrusted input.
1. **No secrets in logs.** Never log tokens, passwords, API keys, or other credentials. All secret fields in HTTP API
   types (`uptrakit-web-api-types`) must use `SecretString` instead of `String`. See
   [Secrets Handling](docs/security/secrets-and-encryption.md).
1. **Logging goes to journald or stdout.** No internal log storage. Full command output is not captured internally --
   only high-level summaries are retained for display.
1. **No overlapping update actions per host.** The scheduler must ensure that two update operations for the same host
   never run concurrently.
1. **No raw SQL.** Use the structures and methods provided by Sea ORM everywhere. **Approved exception:** The rate
   limiter (`crates/ui/web-api/src/auth/rate_limit.rs`) uses `sea_orm::Statement::from_sql_and_values()` for an atomic
   `INSERT ... ON CONFLICT DO UPDATE SET ... CASE WHEN` upsert, because SeaORM's `on_conflict` builder doesn't support
   conditional expressions. The statement is fully parameterized (no injection risk).
1. **Cover new logic with tests.** Cover success and failure paths.
1. **Document everything.** Any code change must be properly documented either in the code, or in the separate
   documentation. Any changes to the agent-controller wire protocol must be documented in
   `crates/shared/wire/asyncapi.yaml` and reflected in [docs/api/wire-protocol.md](docs/api/wire-protocol.md).
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
   **Approved exceptions**: `Mutex::lock().unwrap()`, `RwLock::read().unwrap()`, and `RwLock::write().unwrap()` are safe
   because `panic = "abort"` in the release profile makes lock poisoning impossible.
1. **Use `StatusCode` for HTTP status codes.** Never compare against numeric literals (`== 404`, `>= 400`). Use
   `reqwest::StatusCode` variants (`StatusCode::NOT_FOUND`, `StatusCode::FORBIDDEN`) and helper methods
   (`.is_client_error()`, `.is_success()`). Store status codes as `StatusCode`, not `u16`, in error enums and structs.
   See [Coding Standards](docs/development/coding-standards.md).
1. **Use typed permission extractors for route authorization.** Never call `user.has_permission(...)` directly in
   handler bodies. Instead, declare the required permission via an Axum extractor in the handler signature (e.g.
   `CanViewHosts(_user): CanViewHosts`). The extractors are defined in
   `crates/ui/web-api/src/middleware/permission.rs` via the `permission_extractor!` macro. Each protected endpoint
   must also carry the matching `x-required-permission` OpenAPI extension in its `#[utoipa::path]` annotation (e.g.
   `extensions(("x-required-permission" = json!("view_hosts")))`). See
   [Authentication and Authorization](docs/security/auth-and-authorization.md).
1. **Do not test upstream crate behavior.** Tests must verify internal logic only -- not the behavior of dependencies
   like `thiserror` formatting, `serde` roundtrips on plain derives, or `argon2` salt randomness. See the decision
   table in [Testing Expectations](docs/development/testing.md).
1. **Tests must use `start_paused = true` -- never real sleeps.** All time-dependent tests must use virtual time via
   `#[tokio::test(start_paused = true)]` and `tokio::time::advance()` for deterministic, fast execution. Do not call
   `tokio::time::pause()` explicitly inside the test body; the attribute starts the runtime paused from the very
   beginning. **Exception 1:** Docker-based integration tests (`#[ignore]`) that wait for real external processes.
   **Exception 2:** Tests that use SQLx/SeaORM database connections must NOT use `start_paused = true` — Tokio's
   auto-advance fires pool-internal timers prematurely, causing spurious `ConnectionAcquire(Timeout)` failures under
   stress (nextest `--stress-count`). See [Testing](docs/development/testing.md).
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

1. **Discovery is event-driven, not periodic.** It triggers on new host registration and via explicit API calls.
   There is no periodic discovery scheduler task.

2. **`discovery_state` lifecycle:** `null` (manual, full tracking) → `pending` (discovered, `enabled = false`,
   excluded from version checks) → `approved` (reviewed, `enabled = true`, included in version checks). Deleting a
   `pending` item is a plain soft-delete; the item is re-discoverable unless an ignore rule exists.

3. **Ignore list is separate from deletion.** `DELETE /api/v1/software-items/{id}/hosts/{host_id}?ignore=true`
   removes the host assignment and creates an `autodiscovery_ignore` row keyed on the assignment's
   `(plugin_config_id, package_identifier)`. Without `?ignore=true`, unassigning is a plain delete with no ignore
   rule. Deleting a software item (`DELETE /api/v1/software-items/{id}`) never creates ignore rules. Bulk-discard
   endpoints (`DELETE /api/v1/hosts/{id}/discovered`, `DELETE /api/v1/plugin-configs/{id}/discovered`) also
   perform plain soft-deletes — no ignore rules created.

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
        `version_command` (`sudo /usr/local/bin/uptrakit-phs-version {package_identifier}`) and
        `update_command` (`sudo /usr/local/bin/uptrakit-phs-update`). `sudo` is embedded in
        both commands because the Shell plugin uses `CommandSpec::shell()`, where the
        `privileged` flag has no effect. Each command delegates to a dedicated helper script
        installed by bootstrap:
        - `uptrakit-phs-version <slug>`: validates the slug and reads `/root/.<slug>`
          for version detection.
        - `uptrakit-phs-update`: runs `env PHS_SILENT=1 /usr/bin/update` for unattended
          container updates (no arguments, so no argument validation is needed).
        Both are declared via `ProxmoxHelperScriptsPlugin::required_sudo_commands()` using
        `SudoHelperScript { install_path, content }` — bootstrap installs both scripts.
     The PHS shell constants live in `crates/plugins/discovery/proxmox-helper-scripts/src/plugin.rs`.
   - APT-managed apps emit a `DiscoveryTarget` with `plugin_type: PackageManagerApt`, empty config `{}`, and
     name `"APT (auto)"`.
   - Apps whose scripts contain neither GitHub patterns nor a specific `apt install` line are skipped.
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

   **APT** emits empty `targets` when running with an existing config (config-ID path).

   The `extra` field on `DiscoveredSoftware` is purely informational metadata (e.g. Docker's
   `{"containers": ["web-server"]}`) — the controller never interprets it for config synthesis.

5. **Discovery capability is derived from the registry.** Call `state.plugin_ops.discovery_plugin_types()`
   (or `PluginRegistry::discovery_plugin_types()` statically) to get the current list of discovery-capable
   plugin types. This is derived automatically from each plugin's `capabilities()` method via the registry —
   no static list is maintained separately.

6. **Package identifier validation goes through `PluginRegistry`.** Plugin-specific constraints on the
   `package_identifier` field (e.g. Homebrew's allowed character set) must be implemented in the plugin crate as
   `pub fn validate_identifier(value: &str) -> Result<(), String>` and wired through
   `PluginRegistry::validate_package_identifier(plugin_type, value)`. Never add plugin-specific validation logic
   directly to web API query helpers or route handlers. The `PluginOps` trait exposes this as
   `validate_package_identifier_str(plugin_type: &str, value: &str)` for trait-object dispatch. See
   [Plugin Guidelines](docs/development/plugin-guidelines.md) for the full extension pattern.

7. **Plugins declare required sudo commands via `required_sudo_commands()`.** Any plugin that needs root-level
   command execution must override `required_sudo_commands() -> Vec<SudoCommandEntry>` on its `Plugin` impl.
   Each `SudoCommandEntry` carries a bare command name (or display identifier for helper scripts) and a human-readable
   explanation. For most commands, **never hardcode absolute paths** — they are resolved on the target host via
   `command -v` at bootstrap time. When a simple sudoers command would be too broad (e.g. granting `cat` would allow
   reading any file), use `SudoCommandEntry::helper_script: Some(SudoHelperScript { install_path, content })` instead.
   Bootstrap installs the script at `install_path` with mode `0755` and uses that path as the sudoers command; the
   script itself validates arguments to enforce the least-privilege contract that sudoers wildcards cannot safely
   express (`*` matches `/` in sudoers). **Never hardcode `sudo` in `CommandSpec`** — instead call `.privileged()`
   on the spec. Shell-mode commands (`CommandSpec::shell`) must embed `sudo` in the command string directly because
   `.privileged()` has no effect on shell mode. `PluginRegistry::all_required_sudo_commands()` aggregates all
   declarations for use by the SSH agent's sudoers generation logic. See
   [Plugin Guidelines](docs/development/plugin-guidelines.md) and [Sudoers Management](docs/security/sudoers-management.md).

8. **Plugin capabilities.** The `PluginCapability` enum (in `crates/plugins/infrastructure/core/src/types.rs`) has six variants:

   - `DiscoverLocalSoftware` — the plugin can discover locally installed software.
   - `RefreshPackageIndex` — the plugin can refresh/sync a local package index from remote sources.
   - `DetectHostCompatibility` — the plugin implements `detect_host_compatibility()` which returns a
     `HostCompatibility` enum (`Compatible` or `Incompatible { reason: String }`). Implemented by:
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
| `crates/shared/types/src/software_discovery_state.rs` | `SoftwareDiscoveryState` enum |
| `crates/shared/types/src/discovered_software.rs` | `DiscoveredSoftware` type (with `targets: Vec<DiscoveryTarget>`) |
| `crates/shared/types/src/discovery_target.rs` | `DiscoveryTarget` struct (plugin type, config, name, roles, overrides) |
| `crates/shared/db/src/entity/host_software_item_plugin.rs` | SeaORM entity for role-based plugin assignments |
| `crates/shared/db/src/entity/autodiscovery_ignore.rs` | SeaORM entity for ignore rules |
| `crates/shared/agent-core/src/discovery.rs` | `handle_discover_software()` agent-side logic |
| `crates/ui/web-api/src/queries/autodiscovery.rs` | DB helpers + `process_discovery_results()` |
| `crates/ui/web-api/src/routes/autodiscovery.rs` | Ignore list CRUD routes |
| `crates/ui/web-api/src/routes/service_ws/handler/discovery.rs` | `trigger_discovery_for_agent_host()` helper — applies allowlist before dispatching |
| `crates/shared/db/src/entity/tenant_discovery_allowlist.rs` | SeaORM entity for tenant-wide discovery allowlist |
| `crates/shared/db/src/entity/host_discovery_allowlist.rs` | SeaORM entity for per-host discovery allowlist |
| `crates/ui/web-api/src/queries/discovery_allowlist.rs` | DB helpers: list, add, remove, and `load_*_allowlist_set()` for filter lookups |
| `crates/ui/web-api/src/routes/discovery_allowlist.rs` | Route handlers for tenant/host allowlist CRUD |
| `docs/api/autodiscovery.md` | Full API reference for autodiscovery endpoints |
| `docs/api/discovery-allowlist.md` | Full API reference for discovery allowlist endpoints |
| `docs/end-user/autodiscovery.md` | End-user guide (discovery workflow, review, ignore list, allowlist) |
| `docs/end-user/plugin-configs.md` | End-user guide for plugin config CRUD and discovery |
| `docs/end-user/cli-usage.md` | CLI command reference including `plugin-configs`, `autodiscovery`, and `discovery-allowlist` groups |

### Home Assistant MQTT discovery

The MQTT service can publish [Home Assistant MQTT Discovery](https://www.home-assistant.io/integrations/mqtt/#mqtt-discovery)
topics for each tracked software item, creating `update` entities in HA — one per `(software_item, host)` pair.

Key invariants:

1. **Discovery is opt-in per MQTT client.** Two columns on `mqtt_clients` control it:
   `ha_discovery BOOL` and `ha_discovery_prefix TEXT DEFAULT 'homeassistant'`.
2. **State push is controller-initiated.** The controller sends `SoftwareStates` (wire type
   `software_states`) to MQTT services whenever version data changes (version check completed, update
   result received). The MQTT service stores the states in memory and publishes to the broker.
3. **`SoftwareStates` is safe for cross-controller delivery.** It contains no credentials and is published
   to NATS (when configured) with `target_capability = "mqtt_bridge"` so only MQTT services receive it.
4. **Reconnect resilience.** On every `ConnAck` the MQTT service emits a `Reconnected` event, causing
   `TenantManager` to republish all discovery configs and state topics from the in-memory cache.
5. **HA restart resilience.** HA publishes `"online"` to `{ha_discovery_prefix}/status` on startup
   (birth message). The MQTT service subscribes to this topic and republishes discovery configs when
   `"online"` is received (`HaOnline` event).
6. **Updates triggered via MQTT.** When a user presses Install in HA, HA publishes `"install"` to the
   entity's command topic. The MQTT service resolves `to_version` from the in-memory state cache and
   sends `ServiceMessage::MqttTriggerUpdate` to the controller. The controller validates the request
   and dispatches `execute_update` to the agent. On failure the controller sends `error` back (soft
   error — WebSocket is not closed).
7. **Actor attribution.** Updates triggered via MQTT have `actor_type = "mqtt"` and
   `actor_id = <mqtt_client_id>` in the `update_history` record.

#### MQTT topic scheme

All topics use the MQTT client's `topic_prefix` field.

| Topic | Retained | Direction | Purpose |
| --- | :---: | --- | --- |
| `{prefix}/update/{item_id}/{host_id}/state` | ✓ | publish | Installed version string |
| `{prefix}/update/{item_id}/{host_id}/latest_version` | ✓ | publish | Latest available version string |
| `{prefix}/update/{item_id}/{host_id}/set` | — | subscribe | Receives `"install"` from HA |
| `{ha_prefix}/update/uptrakit_{t}_{i}_{h}/config` | ✓ | publish | HA discovery config (JSON) |
| `{ha_prefix}/status` | — | subscribe | HA birth/will (`"online"` / `"offline"`) |

`{t}`, `{i}`, `{h}` are UUID hex strings with dashes removed. The `unique_id` format is
`uptrakit_{t}_{i}_{h}`.

#### Key files

| File | Purpose |
| --- | --- |
| `crates/core/mqtt/src/ha_discovery.rs` | Pure HA topic/config helpers + `parse_command_topic` |
| `crates/core/mqtt/src/tenant_manager.rs` | `TenantManager`: software state cache, `publish_ha_state_full`, `resolve_update_trigger` |
| `crates/core/mqtt/src/mqtt_client.rs` | `MqttServiceEvent` enum, `publish_retained`, `subscribe_topic`, HA status topic handling |
| `crates/core/mqtt/src/main.rs` | `on_service_event` dispatch; `ControllerMessage::SoftwareStates` handler |
| `crates/ui/web-api/src/queries/mqtt_software_states.rs` | Bulk query loading enabled software items with per-host version data |
| `crates/ui/web-api/src/notification_service.rs` | `push_software_states_for_tenant` (local broadcast + optional NATS publish) |
| `crates/ui/web-api/src/routes/service_ws/handler/mqtt.rs` | `MqttTriggerUpdate` handler; states push after `TenantAssignments`, `VersionCheckResults`, and `UpdateResult` |
| `crates/ui/web-api/src/queries/update_triggers.rs` | `trigger_update_for_host` shared by REST and MQTT WS handlers |
| `docs/end-user/home-assistant-mqtt.md` | Full end-user setup guide |
| `docs/api/wire-protocol.md` | `software_states` and `mqtt_trigger_update` payload docs |
| `crates/shared/wire/asyncapi.yaml` | AsyncAPI schemas for both new messages |

### Service ping interval

The ping interval is controller-managed and per-service configurable. The `services` DB table has a nullable
`ping_interval_seconds INTEGER` column. The controller reads this value per-service and falls back to
profile-based defaults (300s for `Agent` profile, 15s for `MqttBridge` profile) when the column is `NULL`.
Defaults are provided by `ServiceProfile::default_ping_interval_secs()`.

Key integration points:

- **Wire protocol**: `ServiceSettingsPayload.ping_interval` is a required `Duration` field serialized as `u32` seconds
  via `#[serde(with = "duration_seconds")]`. The `duration_seconds` module in `uptrakit-internal-wire` converts between
  `std::time::Duration` and `u32` seconds on the wire.
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
| `Scheduler` | `scheduler` | -- | -- | -- | yes | yes |
| `DatabaseAccess` | `database_access` | -- | -- | -- | yes | yes |
| `NatsAccess` | `nats_access` | -- | -- | -- | yes | yes |
| `MasterKeyAccess` | `master_key_access` | -- | -- | -- | yes | yes |
| `CaManagement` | `ca_management` | -- | -- | -- | yes | yes |
| `Other(String)` | *(unknown)* | -- | -- | -- | -- | -- |

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

#### Controller events

The `controller_events` DB table has been dropped. Cross-controller event routing now uses NATS JetStream
(feature-gated behind the `nats` Cargo feature). When NATS is not configured, the controller operates in
single-instance mode with local-only delivery.

#### REST API

`ServiceResponse` contains `capabilities: Vec<String>` and `service_label: String` instead of the former
`service_type: ServiceType`. The list endpoint filter parameter is `?capability=` instead of `?type=`.

#### Frontend

The frontend filters services by capability instead of type and displays `service_label` instead of `service_type`.

#### Key files

| File | Purpose |
| --- | --- |
| `crates/shared/wire/src/lib.rs` | `Capability` enum, serde, `is_known()`, `EnrollPayload` with `capabilities` field |
| `crates/shared/wire/src/service_profile.rs` | `ServiceProfile` enum, `from_capabilities()`, `parse_capabilities()`, `serialize_capabilities()` |
| `crates/ui/web-api/src/service_connections.rs` | `register()`, `broadcast_by_capability()` |
| `crates/shared/db/src/entity/enrollment_token.rs` | `enrollment_tokens` SeaORM entity |
| `crates/ui/web-api/src/routes/enrollment_tokens.rs` | Enrollment token REST endpoints |
| `crates/ui/web-api/src/queries/enrollment_tokens.rs` | Enrollment token DB queries |
| `crates/shared/service-sdk/src/connection.rs` | `agreed_capabilities` field + accessors |
| `crates/shared/service-sdk/src/event_loop.rs` | Capability intersection in `ServiceSettings` handler |
| `crates/ui/web-api/src/routes/service_ws/protocol.rs` | `controller_capabilities()`, `ServiceSettingsPayload` construction |
| `crates/ui/web-api/src/nats_transport.rs` | NATS JetStream transport (feature-gated) |
| `crates/ui/web-api/src/event_delivery.rs` | Shared delivery routing logic |
| `crates/shared/wire/asyncapi.yaml` | Schema for `capabilities` arrays in messages |
| `docs/api/wire-protocol.md` | Full capability negotiation documentation |

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

**Complete minimal example:**

```rust
use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_shared_macros::impl_report_conversion;

#[derive(Debug, Error)]
pub enum WidgetError {
    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),
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
`uptrakit-mqtt`) owns its own `init_tracing()` helper in `src/main.rs`. `tracing-subscriber` must appear in each
binary's `[dependencies]`, not in shared library crates.

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

### End-user Guides

- [CLI Usage Guide](docs/end-user/cli-usage.md)
- [Plugin Configurations](docs/end-user/plugin-configs.md)
- [Update History](docs/end-user/update-history.md)
- [Profile and API Tokens](docs/end-user/profile-tokens.md)
- [Autodiscovery](docs/end-user/autodiscovery.md)
- [Update Workflow](docs/end-user/update-workflow.md)
- [Home Assistant and MQTT Integration](docs/end-user/home-assistant-mqtt.md)

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

### Architecture

- [Multi-Tenancy](docs/architecture/multi-tenancy.md)
- [Host Entity](docs/architecture/host-entity.md)
- [Software Item Entity](docs/architecture/software-item-entity.md)
- [Update History Entity](docs/architecture/update-history-entity.md)
- [Scheduler](docs/architecture/scheduler.md)
- [Scheduler Engine](docs/development/scheduler-engine.md)
- [External Scheduler Deployment](docs/end-user/deployment/external-scheduler.md)
- [SSH Agent](docs/architecture/ssh-agent.md)

### API and Protocol

- [Wire Protocol](docs/api/wire-protocol.md)
- [Authentication Flows](docs/api/auth-flows.md)
- [Settings Runtime](docs/api/settings-runtime.md)
- [HTTP Web API](docs/api/http-web-api.md)
- [Services and Operations](docs/api/services-operations.md)
- [Autodiscovery](docs/api/autodiscovery.md)
