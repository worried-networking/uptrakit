# AGENTS -- AI Agent Guide for Uptrakit

This file provides structured context for AI coding agents working on the Uptrakit codebase. Read this first before
making any changes.

Read [CONTEXT.md](CONTEXT.md) — controlled vocabulary; use its terms in code, docs, commits.

## Project summary

Uptrakit is an agent-based update tracking toolkit for self-hosted Linux environments. It tracks installed software
versions across remote hosts, checks for updates, and allows **manual, user-triggered** updates. It is **not** an
auto-updater.

Key components:

- **Controller** (server): API, Web UI, optional embedded scheduler, upstream version checking.
- **External Scheduler** (bin): enrolls as a system service, receives DB/NATS/master-key credentials via WebSocket, runs
  scheduled tasks across all tenants independently.
- **MQTT Service** (bin): MQTT/Home Assistant integration with surface-based client management.
- **Agents**: unprivileged daemons on each managed host; outbound-only secure WebSocket; local version detection and
  update execution via sudo allowlists.
- **Plugins**: first-party modules that detect, report, and update software; each crate uses `declare_plugin!` to export
  a `PluginDescriptor` static, listed in `all_descriptors()` inside the unified `PluginCatalog`.

For full project context, see [README.md](README.md); contribution rules, [CONTRIBUTING.md](CONTRIBUTING.md); system
design, [ARCHITECTURE.md](ARCHITECTURE.md); security policy, [SECURITY.md](SECURITY.md). The full documentation
catalogue lives at [docs/README.md](docs/README.md).

## Quick-start commands

Canonical source for Rust command definitions: [docs/development/quality-gates.md](docs/development/quality-gates.md).

### Rust / backend

```sh
cargo fmt --all                                                      # Format
cargo check --no-default-features --features db-sqlite               # Lint with minimal feature-set
cargo check --all-features                                           # Lint
cargo clippy --all-targets --no-default-features --features db-sqlite # Clippy over minimal feature-set
cargo clippy --all-targets --all-features                            # Clippy
cargo test --all-features                                            # Tests
cargo deny check                                                     # Validate new dependencies
bash ci/verify_no_security_audit.sh                                  # No legacy security_audit / raw action literals
bash ci/verify_typed_audit_actions.sh                                # Audit action parsing/building at explicit boundaries
bash ci/verify_handler_state_contract.sh                             # No handler mixes State<Arc<AppState>> with sub-state
python3 ci/verify_db_access_policy.py                                # db_access_policy.toml consistent with routes/
bash ci/verify_agents_md_budget.sh                                   # AGENTS.md size budget gate
bash ci/verify_no_raw_body_extractors.sh                             # Request bodies go through Unvalidated<T>/Validated<T>; raw Json/Form banned
python3 ci/verify_no_orphan_modules.py                               # Every tracked .rs reachable via mod resolution (no orphan modules)
bash ci/verify_no_new_cfg_not_feature.sh                             # Additive-only feature flags: no new negated-feature cfg outside allowlist
python3 ci/verify_action_security_declarations.py                    # Operation oauth2 scope lists match handler action extractors
bash ci/verify_adr_numbers.sh                                        # No duplicate ADR numbers
bash scripts/regen-adr-toc.sh --check                                # docs/adr/README.md staleness + links
```

> **Note:** `--all-features` includes `embed-frontend`, which requires `frontend/build/`. Build the frontend first
> (`cd frontend && npm ci && npm run build`) before running `--all-features` checks.

### Frontend

```sh
cd frontend
npm run lint          # ESLint
npm run format:check  # Prettier — read-only
npm run check         # Svelte type-check via svelte-check
npm run build         # SvelteKit build — required before `cargo build` with `embed-frontend`
```

### Markdown and integration tests (Docker)

```sh
markdownlint --config .markdownlint.json '**/*.md'                       # Lint Markdown
cargo test -p uptrakit-integration-tests --test reverse_proxy -- --ignored # Reverse-proxy changes
cargo test -p uptrakit-integration-tests --test database -- --ignored   # DB/migration/REST changes
docker build -f docker/Dockerfile.test -t uptrakit-test:latest .        # Build image, then:
cargo test -p uptrakit-integration-tests -- --ignored                   # Enrollment/wire/lifecycle changes
```

Run integration tests only when changes touch the triggering areas. After any backend route or REST-contract change,
run `./scripts/regen-api.sh` and commit `crates/ui/web-api/openapi.json` + `frontend/src/lib/api/generated/` — CI gates
on staleness of both. After any wire-type change, run `./scripts/regen-asyncapi.sh` and commit
`crates/shared/wire/asyncapi.yaml` — CI gates on staleness via the `asyncapi_yaml_is_up_to_date` golden test.

## Codebase layout

```text
uptrakit/
├── Cargo.toml                          # Workspace root (resolver = "3", members = "crates/*/*")
├── crates/
│   ├── core/
│   │   ├── agent/                      # uptrakit-agent            (bin) — agent daemon
│   │   ├── agent-ssh/                  # uptrakit-agent-ssh        (bin) — thin CLI shell; logic lives in uptrakit-agent-ssh-runtime (ADR-0005)
│   │   ├── controller/                 # uptrakit-controller       (bin) — central server; embedded-service infra under src/{embedded,scheduler,agent,ssh_agent}
│   │   ├── mqtt/                       # uptrakit-mqtt             (bin) — standalone MQTT service
│   │   ├── mqtt-runtime/               # uptrakit-mqtt-runtime     (lib) — shared MQTT runtime (tenant manager, HA discovery, state publisher)
│   │   ├── scheduler/                  # uptrakit-scheduler       (bin) — external scheduler; enrolls as a system service
│   │   └── integration-tests/          # uptrakit-integration-tests (test) — Docker-based reverse-proxy + system integration tests
│   ├── plugins/                        # See crates/plugins/AGENTS.md for authoring rules
│   │   ├── infrastructure/
│   │   │   ├── core/                   # uptrakit-plugin-infrastructure-core     (lib) — descriptor + role traits + PluginCatalog + shared helpers
│   │   │   ├── registry/               # uptrakit-plugin-infrastructure-registry (lib) — plugin dispatch & validation
│   │   │   └── proxmox/                # uptrakit-plugin-infrastructure-proxmox  (lib) — Proxmox VE infrastructure plugin
│   │   ├── releases/                   # docker, github, gitlab, forgejo release-source plugins
│   │   ├── generic/shell/              # uptrakit-plugin-generic-shell           (lib) — version/update via shell commands
│   │   ├── package-managers/           # homebrew, apt, npm, mas, pacman, pkg, apk, dnf, snap, cargo
│   │   ├── hooks/                      # systemd, shell update-lifecycle hook plugins
│   │   ├── notifications/              # core, webhook, telegram, email notification plugins
│   │   ├── enhancements/dashboard-icons/ # uptrakit-plugin-enhancement-dashboard-icons (lib) — icon-URL assignment
│   │   └── discovery/proxmox-helper-scripts/ # PVE helper-scripts discovery plugin
│   ├── shared/
│   │   ├── agent-core/                 # uptrakit-agent-core       (lib) — shared agent logic: version check, update, batch
│   │   ├── command/                    # uptrakit-command          (lib) — CommandExecutor, sudo-aware execution, StdioTunnel, RemoteExecutor
│   │   ├── crypto/                     # uptrakit-crypto           (lib) — AES-256-GCM at-rest encryption, envelope encryption, EncryptedString
│   │   ├── db/                         # uptrakit-shared-db        (lib) — SeaORM entities, TenantDb, migrations (feature `migration`)
│   │   ├── db-tx/                      # uptrakit-db-tx            (lib) — begin_immediate(), sole transaction opener (clippy-enforced)
│   │   ├── directories/               # uptrakit-directories      (lib) — cross-platform directory management
│   │   ├── surfaces/                   # uptrakit-surfaces         (lib) — shared UI surface contract types
│   │   ├── macros/                     # uptrakit-shared-macros    (lib) — impl_report_conversion!
│   │   ├── types/                      # uptrakit-shared-types     (lib) — shared value types, PluginTypeId, SSRF helpers, PackageIdentifierRules
│   │   ├── web-api-types/              # uptrakit-web-api-types    (lib) — shared HTTP request/response types
│   │   ├── openapi-client/            # uptrakit-openapi-client   (lib) — typed HTTP client + SSE streaming
│   │   ├── nats/                       # uptrakit-nats             (lib) — shared NATS primitives
│   │   ├── scheduler-engine/          # uptrakit-scheduler-engine (lib) — poll loop, claim mechanism, built-in executors
│   │   ├── tracing-init/              # uptrakit-tracing-init     (lib) — TracingBuilder + init helpers (canonical tracing setup)
│   │   ├── service-sdk/               # uptrakit-service-sdk      (lib) — service lifecycle, event loop, enrollment, identity, zeroconf
│   │   ├── audit-log/                 # uptrakit-audit-log        (lib) — semantic audit domain, emitters, dispatcher, backends
│   │   ├── update-hooks/             # uptrakit-config-merge     (lib) — resolve_effective_config(), merge_config()
│   │   ├── backoff/                   # uptrakit-backoff          (lib) — exponential backoff (reconnect_backoff_builder)
│   │   ├── zeroconf/                   # uptrakit-zeroconf        (lib) — mDNS/DNS-SD contract + browse (SERVICE_TYPE, TXT keys)
│   │   └── wire/                       # uptrakit-wire             (lib) — service↔controller wire protocol, Capability, ServiceProfile
│   └── ui/                             # See crates/ui/web-api/AGENTS.md for web-api handler conventions
│       ├── cli/                        # uptrakit-cli             (bin+lib) — CLI over openapi-client; SSE for --follow
│       ├── web-api/                    # uptrakit-web-api         (lib) — HTTP API: routes, middleware, AppState + sub-states, service WS
│       ├── web-api-auth/              # uptrakit-web-api-auth    (lib) — JWT, sessions, OIDC, tokens, permissions, settings store
│       └── web-api-queries/           # uptrakit-web-api-queries (lib) — DB query modules, ServiceNotifier; re-exports TenantDb
├── frontend/                           # SvelteKit SPA (Skeleton UI v4 + Tailwind v4)
│                                       # See frontend/AGENTS.md for toolchain + Svelte/TypeScript conventions
├── docker/                             # Dockerfile, Dockerfile.test (multi-binary test image)
├── docker-compose.yml                  # Compose profiles: postgres, mqtt, ssh, scheduler, full
├── .github/workflows/                  # ci.yml, docker.yml, release-please.yml
├── Cross.toml                          # ARM64 Linux cross-compilation config
├── CONTRIBUTING.md
├── README.md
└── AGENTS.md                           # This file
```

All crates use **edition = "2024"**; some specify `rust-version = "1.91"`.

**Feature flags, build profiles, release workflow:** controller/web-api feature-flag reference lives in
[docs/development/feature-flags.md](docs/development/feature-flags.md); build profiles and build-speed settings in
[docs/development/setup.md](docs/development/setup.md#build-speed-optimizations); the release-please workflow in
[docs/development/releases.md](docs/development/releases.md).

## General MUST FOLLOW Rules for AI Coding Agents

### Quality Gates

All changes must pass the defined quality gates — see [docs/development/quality-gates.md](docs/development/quality-gates.md).
Git hooks (via `husky-rs`) enforce a subset on commit/push, auto-installed on first `cargo build`/`cargo test`
(`NO_HUSKY_HOOKS=1` to skip). Run the gates relevant to what you touched; do not add exceptions to `.markdownlintignore`
or `.markdownlint.json` without approval.

**OpenAPI param authoring:** author query/request params via `params(<IntoParamsStruct>)`, never a hand-maintained
`params(("field" = …, Query, …))` list ([ADR-0025](docs/adr/0025-drift-proof-openapi-params.md); enforced by
`ci/verify_no_inline_query_params.sh`). See `docs/development/coding-standards.md`.

**Do not regress architecture quality.** Architecture is enforced by CI gates (`cargo deny check`,
`python3 ci/check_plugin_semantic_boundary.py`). A task is incomplete if it introduces a dependency cycle, leaks a
plugin/production boundary, or adds unused dependencies without reason.

### Dependency registration

All new dependencies (third-party and internal workspace crates) go in `[workspace.dependencies]` in the root
`Cargo.toml` **first**; crate manifests reference them via `workspace = true`. Never pin a version or path locally. See
[docs/development/dependency-policy.md](docs/development/dependency-policy.md).

### Commit messages

Conventional Commits are required — see [docs/development/commit-messages.md](docs/development/commit-messages.md).

### Architecture rules and invariants

These are non-negotiable design constraints. Do not violate them.

1. **Updates are never automatic.** The scheduler triggers version _checks_ only; update execution requires explicit user action (UI, CLI, or
   MQTT/Home Assistant).
1. **Agents initiate outbound-only connections.** Agents connect to the controller via secure WebSocket (`/api/v1/ws/service`); they never listen
   on any port or accept inbound connections.
1. **Agents run unprivileged.** They run as a dedicated user; only commands declared by plugins via `required_sudo_commands()` get `NOPASSWD` sudo
   — never blanket `ALL`. See [Sudoers Management](docs/security/sudoers-management.md).
1. **Plugin split and role-based assignment.** Each `(host, software_item)` pair has per-role assignments (`detect_version`, `fetch_releases`,
   `execute_update`) in `host_software_item_plugins`, each with an `execution_site` column (`auto`/`agent`/`controller`). Per-host version tracking
   lives on `host_software_items`. See `PluginRole` (`crates/shared/types/src/plugin_role.rs`).
1. **No shell injection.** Any path that constructs or executes shell commands must validate inputs; custom scripts are untrusted input.
1. **No secrets in logs.** Never log tokens, passwords, API keys, or credentials. Secret fields in `uptrakit-web-api-types` must use
   `SecretString`, not `String`. See [Secrets Handling](docs/security/secrets-and-encryption.md).
1. **Logging goes to journald or stdout.** No internal log storage; full command output is not captured internally, only high-level summaries are
   retained for display. See [Logging](docs/development/logging.md).
1. **Tracing spans use `skip_all`.** All `#[tracing::instrument]` must use `skip_all` and explicitly list relevant fields — never auto-capture
   arguments. Wire envelopes carry `TraceContext`. See [Tracing Conventions](docs/development/tracing.md).
1. **No overlapping update actions per host.** At most one active (`Pending`/`InProgress`) update per host (all types share `update_history`),
   enforced by `validate_update_preconditions` (HTTP 409) and the partial unique index `uix_update_history_host_active`. Terminal states are
   `Completed`, `Failed`, `Interrupted` (ADR-0024); `Queued` is not terminal. See [Update History
   Entity](docs/architecture/update-history-entity.md).
1. **No raw SQL.** Use SeaORM and `sea_query` builders everywhere, including migrations. The approved exceptions (each needing an inline comment
   naming the limitation) and the table-recreation guide live in [database-migrations.md](docs/development/database-migrations.md).
1. **Cover new logic with tests.** Cover both success and failure paths. See [Testing](docs/development/testing.md).
1. **Document everything.** Any code change must be documented in code or docs. Wire-protocol changes must be documented in
   `crates/shared/wire/asyncapi.yaml` and reflected in [wire-protocol.md](docs/api/wire-protocol.md). Architectural decisions are recorded as ADRs
   created with `adrs new "Title"` — never hand-allocate an ADR number or hand-edit `docs/adr/README.md`; see [ADR
   process](docs/development/architecture-decision-records.md).
1. **Wire protocol payloads must implement `WireValidate`.** Any new wire payload with `Vec<T>` or `String` fields must implement `WireValidate` in
   `wire_validate_impls.rs` (add limit constants in `limits.rs`). See [Payload Size Limits](docs/api/wire-protocol.md#payload-size-limits).
1. **Large report payloads must use `send_auto_paginate()`.** When sending `DiscoveryResults`, `VersionCheckResults`, `ReportHosts`, or
   `BatchUpdateResult`, call `conn.send_auto_paginate(msg)`; new paginatable types implement `Paginatable`. See [Report
   Pagination](docs/api/wire-protocol.md#report-pagination).
1. **Command-bearing plugin config fields must be validated.** Configs with command strings (`version_command`, `update_command`, hook `commands`)
   must validate length via `validate_command_length()` (`uptrakit-shared-types::command_validation`); hook counts against
   `MAX_HOOK_COMMANDS_PER_PHASE`.
1. **Version/build metadata contract is unified.** All workspace binaries expose consistent `--version` output; enabled features are derived at
   build time via `uptrakit_build_info::emit_enabled_features_env()` — never hardcode per-binary.
1. **Do not add any `#[allow()]`** without explicit confirmation. Workspace lints enforce `warnings = "deny"` and `clippy::all = "deny"`. Sole
   exception (feature-gated dead code with an inline comment): [Lint
   Suppressions](docs/development/coding-standards.md#lint-suppressions-for-feature-gated-items).
1. **Feature flags are additive only.** `#[cfg(not(feature = "X"))]` is prohibited; use `cfg!(feature = "X")` in expression position.
   `#[cfg(feature = "X")]` (without `not`) is allowed only on purely additive blocks. Plugin descriptor contributions
   are additionally feature-monotonic — a feature may only add contributions, never suppress them (ADR-0032). See
   [Feature Flags](docs/development/coding-standards.md#feature-flags).
1. **Use `FromStr` for all string-to-type conversions.** No ad-hoc `parse(&str)` methods: define a typed `Parse{TypeName}Error`, `impl FromStr`,
   call `s.parse::<MyType>()`. UUID path params use `Path<Uuid>`. See [Coding Standards](docs/development/coding-standards.md).
1. **Keep the openapi-client in sync with web-api endpoints.** Any endpoint addition/change/removal must be reflected in `uptrakit-openapi-client`
   (excluding WebSocket, OIDC browser callback, OCSP, and the RFC-discovered OAuth protocol + browser consent-flow endpoints).
   See [OpenAPI Client](docs/development/openapi-client.md).
1. **Do not use `unsafe`, `unwrap` or `panic!`.** Prefer typed errors with `thiserror`, context via `rootcause`. Use `parking_lot::Mutex`/`RwLock`
   in all async code (never `std`/`tokio` variants); drop guards before any `.await`. See [Synchronous Locks in Async
   Code](docs/development/coding-standards.md#synchronous-locks-in-async-code).
1. **Use `StatusCode` for HTTP status codes.** Never compare numeric literals; use `reqwest::StatusCode` variants and helper methods, and store
   status as `StatusCode` (not `u16`). See [Coding Standards](docs/development/coding-standards.md).
1. **Use `SsrfSafeResolver` for all outbound HTTP clients.** Any `reqwest::Client` sending to user-controlled URLs must set
   `.dns_resolver(Arc::new(SsrfSafeResolver::new()))` (or `::permissive()` for self-hosted; in `uptrakit_shared_types::ssrf` behind `http-ssrf`).
   See [SSRF Protection](docs/security/secure-development.md#ssrf-protection).
1. **Use typed action extractors for route authorization.** Never perform an inline authorization check in a handler body (there is no
   `has_permission`-style method); declare the requirement via an Axum extractor. Route families default to `action_extractor!`
   (`crates/ui/web-api/src/middleware/action.rs`), backed by the
   `AccessEngine`, declaring a native `security(...)` OpenAPI requirement. See
   [Authentication and Authorization](docs/security/auth-and-authorization.md).
1. **Surface actions are enforced at read/invoke time.** `required_action` (a catalog `resource:verb` action
   string, parsed to `Action` at registration admission) on surface descriptors and interactions is enforced
   server-side via `AccessEngine` before dispatch, for both plugin- and service-backed surfaces; provider-origin
   (service-initiated) calls are denied for interactions that are action-gated or sit on an action-gated surface
   unless the interaction opts in via `provider_invocable` — see
   [Surface Security](docs/security/surfaces.md#provider-origin-invocation).
1. **Do not test upstream crate behavior.** Tests verify internal logic only, not dependency behavior (`thiserror` formatting, `serde` roundtrips
   on plain derives, `argon2` randomness). See the decision table in [Testing](docs/development/testing.md).
1. **Time-dependent tests must use `start_paused = true` — never real sleeps.** A test is time-dependent when it calls a `tokio::time::*` API; use
   `#[tokio::test(start_paused = true)]` + `tokio::time::advance()`. Tests with no Tokio time API must NOT add it. Exceptions are documented in
   [Testing](docs/development/testing.md).
1. **New API endpoint tests must use the shared `TestApp` harness.** `crates/ui/web-api/src/test_harness/` provides `TestApp`, `TestClient`, and
   fixture helpers; never duplicate `test_state()`/`build_test_state()` inline. See [REST API Integration
   Tests](docs/development/testing.md#rest-api-integration-tests).
1. **Use `TenantDb` helpers for all tenant-scoped queries.** `TenantDb` (`crates/shared/db/src/tenant_db.rs`) enforces the tenant filter — never
   call `Entity::find().all(tenant_db.db())` on a `TenantScoped` entity. Use `.find::<E>()` etc.; for join tables without `tenant_id`, use
   `find_via_tenant_join::<Target, Scoped>(relation)`. See [Coding Standards](docs/development/coding-standards.md) (Tenant-Safe Database Queries).
1. **Batch queries instead of per-item loops.** Never SELECT/UPDATE per item in a loop (N+1). Load with `.is_in(ids)` then join in memory; for bulk
   updates use a single `update_many().filter(Column::Id.is_in(ids))`. See [Coding Standards](docs/development/coding-standards.md).

## Subsystem contracts

Each stub below states the load-bearing invariants; follow the link for mechanics and file inventories.

### Autodiscovery

Autodiscovery detects installed software on agent hosts and surfaces it as pending software items. Invariants: discovery is event-driven + periodic
(every 6h); **no approval workflow** — items are created immediately with `enabled: true`; re-discovery never overwrites a non-NULL
`installed_version` on an active `host_software_item` link (creation, link-level reactivation, and NULL-fill still
write); the ignore list is separate from deletion (name-based `software_ignores`, only `?ignore=true` creates one); targets are
structured `DiscoveryTarget` values (not opaque `extra`); discovery capability and `package_identifier` validation both derive from `PluginCatalog`;
`tenant_discovery_allowlist`/`host_discovery_allowlist` tables gate which plugin types run (host overrides tenant; unconfigured = all). Full catalog,
PHS emissions, sudo/batch detail: [docs/development/autodiscovery-internals.md](docs/development/autodiscovery-internals.md).

### Plugin system

Two-tier config model: **type settings** (tenant-level defaults per plugin type, `plugin_type_settings` table) and **plugin configs** (named
credential/endpoint profiles). Effective config is a three-layer shallow merge via `resolve_effective_config()` — type settings → profile config →
per-host override (narrowest wins). Configs implement `PluginConfig`; type-settings-capable plugins add `TypeSettings` via
`declare_plugin!(type_settings: true)`. See [plugin-system.md](docs/development/plugin-system.md) and [plugin-guidelines.md](docs/development/plugin-guidelines.md).

### MQTT / Home Assistant discovery

The MQTT service publishes HA MQTT Discovery topics and per-host summary entities. Invariants: **HA discovery is opt-in per MQTT client**
(`ha_discovery` gates only `.../config` topics; state/version topics always publish); **state push is controller-initiated** (`SoftwareStates` on
version/update changes); `SoftwareStates` carries no credentials, NATS-published with `target_capability = "update_tracking"`; MQTT-triggered updates
set `actor_type = "mqtt"`; the service uses `WorkloadClaims` to exclusively own config keys across instances. Topic scheme, naming, resilience:
[docs/end-user/home-assistant-mqtt.md](docs/end-user/home-assistant-mqtt.md); wire detail: [docs/api/wire-protocol.md](docs/api/wire-protocol.md).

### Service ping interval

Controller-managed and per-service configurable via the nullable `services.ping_interval_seconds` column, falling back to
`ServiceProfile::default_ping_interval_secs()` per profile when `NULL`. The wire field `ServiceSettingsPayload.ping_interval` is a `Duration`
serialized as `u32` seconds; the SDK creates the ping timer only when the first `ServiceSettings` arrives. See
[docs/api/services-operations.md](docs/api/services-operations.md#ping-interval-mechanics).

### Capability-based service identity

Services are identified by their **capability set** (`BTreeSet<Capability>`, persisted as a JSON array in `services.capabilities`), not a fixed type
enum. `ServiceProfile` derives from capabilities via `from_capabilities()` and is **never persisted**. Each side computes `agreed =
intersection(controller, service)` excluding forward-compat `Other(String)` values. Enrollment uses named `enrollment_tokens` (Argon2id-hashed,
capability-scoped, usage/TTL-limited). Capability matrix, negotiation flow, connection internals: [docs/api/wire-protocol.md](docs/api/wire-protocol.md).

### Two-tier service model

The controller manages two independent tiers: **tenant services** (`services`, `tenant_id`-scoped, `/api/v1/services`) and **system services**
(`system_services`, global, `/api/v1/system-services`), routed at enrollment by `Capability::SystemService`. A **credential guard** requires
`system_service` alongside any of `database_access`, `nats_access`, `master_key_access`, `ca_management` — rejecting tenant services requesting infra
secrets. System enrollment uses `system_enrollment_tokens` (Argon2id-hashed, shown once). See [system-services.md](docs/architecture/system-services.md).

### Batch actions

All management endpoints support batch operations via `POST /api/v1/{resource}/batch` with an `action` string and a `ids` UUID array (max 100,
enforced by `BatchActionRequest`'s `Validate` impl). Responses use partial-success semantics (each item independently succeeds/fails). Surface
providers can expose selection-driven interactions with explicit permission gates. Endpoint list and side effects:
[docs/api/batch-actions.md](docs/api/batch-actions.md).

### Error handling

Every boundary defines its own typed error enum plus `pub type Result<T> = std::result::Result<T, Report<MyError>>;` (the alias must cover **all**
functions in a query module, including read-only ones). Use `rootcause` (`report!()` / `bail!()`, never `Report::new()`) and `impl_report_conversion!`
for cross-boundary conversions. Approved `Mutex::lock().unwrap()` is safe under `panic = "abort"`. Decision table, patterns, exceptions:
[docs/development/error-handling.md](docs/development/error-handling.md).

### Tracing initialization

**Libraries must never configure the global tracing dispatcher** — only binary `main()` may. All tracing init lives in `uptrakit-tracing-init`
(`TracingBuilder`, `init_cli_tracing`, `init_test_tracing`); `uptrakit-service-sdk` re-exports it for daemons. Do not add per-binary
`init_tracing()` helpers. See [docs/development/tracing.md](docs/development/tracing.md).

### Directory management

All binaries use `uptrakit-directories` for cross-platform config/state resolution (XDG on Linux, `ProjectDirs` elsewhere), overridable via
`--config-dir`/`--state-dir` or `UPTRAKIT_CONFIG_DIR`/`UPTRAKIT_STATE_DIR`. Config holds rarely-changing data (external certs); state holds the SQLite
DB, keys, and identity. All secret-file writes **must** use the crate's secure helpers (`create_secure_dir` 0o700, `write_secure_file` 0o600) instead
of raw `fs::write`. See [docs/development/directories.md](docs/development/directories.md).

### Notifications

Channel-agnostic subsystem: producers emit internal `NotificationEvent` values; a fire-and-forget `NotificationDispatcher` (bounded `mpsc`, overflow
dropped with a warn) matches tenant-scoped rules and hands a `DeliveryMessage` to the channel. Channels are plugins under
`crates/plugins/notifications/` (`declare_plugin!` + `NotificationTransport` role trait); `channel_type` is a runtime-validated string, never an enum.
HTML-escape all user-controlled values via `escape_html()`. See [docs/development/notifications.md](docs/development/notifications.md); platform-wide
authorization (action catalog in `crates/shared/types/src/access/catalog.rs`) is in
[docs/security/auth-and-authorization.md](docs/security/auth-and-authorization.md).

### Audit log subsystem

V2 semantic audit log. Two compile-time-enforced action classes: **Stateful** (entity transition, requires `before`/`after` snapshot, in-transaction
via `emit_stateful` under `BEGIN IMMEDIATE` + `flush_after_commit()`) and **Event** (discrete fact, snapshots forbidden, async via `emit_event`).
Snapshot targets derive `AuditView` (secret-safe JSON). Every state-changing site must appear in `audit-catalog.toml` with an `action`/`skip`
(enforced by `cargo xtask audit-coverage-check`). Banned: `target: "security_audit"` producers, raw `action_type` literals, service-forwarded
Stateful events, `emit_best_effort`. See [docs/development/audit-logs.md](docs/development/audit-logs.md) and
[docs/security/audit-logs.md](docs/security/audit-logs.md).

### Shared surface runtime

Connected services and plugins extend the web UI via the shared surface runtime. Providers register surface contracts (services at runtime, plugins
at startup); the controller enforces admission, routing, permissions, and rollout state. Registration and interaction messaging require the
`UiSurfaces` capability; built-in and provider-backed surfaces share one rendering path (`/surfaces/{surface_id}`). Services invoke controller-side
plugin interactions via `ServiceMessage::SurfaceActionRequest` → `ControllerMessage::SurfaceActionResponse` (oneshot-correlated by
`ServiceSurfaceProxy`). See [surfaces.md](docs/development/surfaces.md) and [security/surfaces.md](docs/security/surfaces.md).

## Detailed documentation references

Every other topic (security, end-user guides, development standards, architecture, API/protocol) is indexed in
[docs/README.md](docs/README.md). ADRs in `docs/adr/` are created and validated with the `adrs` CLI — see
[Architecture Decision Records](docs/development/architecture-decision-records.md).

## Maintaining this file

`AGENTS.md` is the AI agent's entry point: an index of invariants and pointers, not a mirror of the codebase. Its value
is density and accuracy — every fact has exactly one canonical home, and this file links to it rather than copying it.

### Size budget (CI-enforced)

The root `AGENTS.md` must stay ≤ 500 lines (and ≤ 60 KB); any scoped `*/AGENTS.md` must stay ≤ 250 lines. Enforced by
`bash ci/verify_agents_md_budget.sh` (CI and pre-commit). Authored target for this root file is ~400 lines.

### Rules

- **No code-structure inventory in any AGENTS.md.** The primary anti-drift control: never add per-file row tables that
  mirror module layout (the 339→1944-line regrowth was driven by handler-table sync commits). Orient with prose that
  points at a source file, not a table. Applies to scoped files too.
- **No hardcoded counts.** Never write "32 extractors", "17 variants", "49 tables" — counts drift the moment code
  changes. Point at the defining source file instead.
- **One canonical home; link, don't copy.** If a fact belongs in a `docs/` file, keep it there and link. Do not
  reproduce endpoint references, testing patterns, or convention prose here.
- **Subsystem entries are stubs, not deep dives.** Each states its load-bearing invariants as one-liners plus a link to
  the canonical doc; mechanics, matrices, and file inventories live in that doc.

### What belongs here

- Project summary, glossary pointer, quick-start commands, and the one-line-annotated codebase layout tree.
- Compressed MUST-FOLLOW rules: each keeps its bold lead-in verbatim (external docs cite them) + 1–2 sentences + a link.
- Subsystem-contract stubs (invariant one-liners + link), plus recurring AI-code anti-patterns (add a rule when the same
  mistake appears twice).

### When to update

Update when a crate is added/removed (one tree line), a new architecture invariant is established (a MUST-FOLLOW rule),
a subsystem's invariants change, or a command/flag changes (update
[docs/development/quality-gates.md](docs/development/quality-gates.md) as canonical source **and** the Quick-start block
in the same commit). Do not update for changes internal to one file, doc-only tweaks, or `docker/`/`scripts/` changes
that don't affect agent-facing commands or invariants. Run `markdownlint` after every edit.

### Subfolder AGENTS.md files

Scoped files exist where an area uses a different toolchain or distinct conventions; each must link back to this root
file in its opening paragraph. To add one, create the file with that back-link and add a row below.

| File                          | Scope                                                               |
| ----------------------------- | ------------------------------------------------------------------- |
| `frontend/AGENTS.md`          | SvelteKit frontend (`npm` toolchain, Svelte/TypeScript conventions) |
| `crates/ui/web-api/AGENTS.md` | web-api handler modules, sub-state extractors, OpenAPI placement    |
| `crates/plugins/AGENTS.md`    | plugin authoring: `declare_plugin!`, shared helpers, SSRF, sudo     |
