# Code Review: Workspace (Root)

- **Review date**: 2026-03-06
- **Reviewer**: AI code review (architecture|security|quality|HA|standards|extensibility|tests|consistency|maintainability|database|crate-structure)
- **Parallel review date**: 2026-03-06
- **Parallel reviewers**: 10 AI agents (architecture, security, code quality, tests, HA, database, coding standards, extensibility, consistency, maintainability)
- **Comprehensive 12-dimension review date**: 2026-03-10
- **12-dimension reviewers**: 12 AI agents (architecture, security, code quality, tests, high availability, database, coding standards, extensibility, consistency, idiomatic Rust, references & heap, maintainability)
- **Branch**: docs/codereview-backend

## Summary

The Uptrakit backend (~133K LoC, 35 crates) is a well-structured Rust workspace implementing an
agent-based update tracking toolkit. The codebase demonstrates mature Rust engineering practices:
workspace-level `clippy::all = "deny"` and `warnings = "deny"` lints, consistent error handling
via `rootcause` + `thiserror` + `impl_report_conversion!`, and strong type-system enforcement of
security invariants (`EncryptedString`, `SecretString`, `TenantScoped`).

This root review was updated on 2026-03-01 to incorporate five additional dimensions across all
per-crate reviews: **tests** (functionality coverage + testing philosophy conformity), **consistency**
(cross-cutting API and protocol inconsistencies), **maintainability** (long-term structural debt),
**database** (query safety, transaction correctness, migration patterns), and **crate structure**
(splitting/merging opportunities, dependency graph analysis). Per-crate files were also updated with
findings from new code committed after 2026-02-28: `ZeroizeOnDrop` on `MaskedEmail`
(`5da34db`), NATS CLI subcommand (`33266c1`), and NATS openapi-client methods (`987f110`). Full
reviews for the `forgejo` and `gitlab` release plugins were created on this date.

Updated on 2026-03-02 to incorporate findings from the batch updates feature
(`update_batches` table, `BatchStatus` enum, `UpdateCategory` enum, batch progress SSE
streaming, sequential per-host dispatch), the update category feature (security update
detection in APT plugin, `update_category` column on `host_software_items` and
`update_history`), and the refactored update trigger pipeline (`trigger_update_for_host`
split into three composable layers: validate, create_record, dispatch).

Updated on 2026-03-06 with findings from a fresh 11-dimension parallel review pass. The
exponential backoff fix for the NATS consumer (`ad2e9cf2`), agent-core batch unit tests
(`b83bc361`), `system_enrollment_tokens` / `system_services` query tests (`07ae7922`), and
`software_items` HTTP edge-case tests (`db21ea8f`) are reflected. New **critical** finding:
`notifications/dispatcher.rs` uses `mpsc::unbounded_channel()` with no capacity bound, creating
unbounded memory growth risk under cascading failure or high event volume — should be replaced
with a bounded channel. New **high** finding: `TenantManager::shutdown_all()` in the MQTT service
has a final-status delivery race during graceful shutdown (receiver may be dropped before `Offline`
status is sent). New **high** architectural finding: `uptrakit-web-api/src/lib.rs` publicly exposes
24+ internal modules (all `routes::*`, all `middleware::*`, broadcasters, extractors) that should
be `pub(crate)` — only the router factory functions and `AppState` need `pub` visibility. Additional
list-endpoint consistency gaps identified: `discovery_allowlist`, `host_packages` ignore-rules, and
scheduler task endpoints all return flat `Vec<T>` instead of the project-standard
`PaginatedResponse<T>`. The npm plugin hardcodes the registry URL (preventing private registry use)
and has no HTTP retry logic for transient failures. The `ServiceHandler` trait is non-object-safe
due to associated constants but carries no documenting comment warning future implementors.

The dependency graph is a clean DAG with no circular dependencies. Feature flags are used
judiciously for database backends, OIDC, NATS, and embedded components. The plugin system is
well-designed with a macro-generated registry. The service SDK provides an excellent abstraction
for the enrollment-lifecycle-reconnect flow shared across all service binaries.

Updated on 2026-03-10 with findings from a comprehensive 12-dimension parallel review covering
architecture, security, code quality, tests, high availability, database, coding standards,
extensibility, consistency, idiomatic Rust, references and heap allocation, and maintainability.
Key new findings: the login endpoint leaks deactivated user existence via timing and response
differentiation (D2 Security, MEDIUM); Telegram, Proxmox Helper Scripts, and GitHub download
plugins are missing SSRF-safe DNS resolvers on HTTP clients (D2 Security, MEDIUM); 10 prohibited
`thiserror` Display format tests were found that test the `thiserror` crate rather than
application logic (D4 Tests, MEDIUM); 41% of source files lack any test code (D4 Tests, HIGH);
production `.expect()` persists in the email plugin `merge_smtp_into_config` (D7 Standards, HIGH);
5 `cfg(not(feature))` violations remain (D7 Standards, MEDIUM); 11 `#[allow(clippy::type_complexity)]`
suppressions exist in `web-api-queries` (D7 Standards, MEDIUM); `UpdateFinalStatus` and
`DisconnectReason` are missing `Other(String)` catch-all variants required for wire-safe enums
(D8 Extensibility, HIGH); `HookCommand` is missing a catch-all (D8 Extensibility, MEDIUM);
`NotificationOps` is missing `restore_config_secrets` (D8 Extensibility, MEDIUM); 13+ public
structs in `plugin-infrastructure-core` are missing `#[non_exhaustive]` (D8 Extensibility, MEDIUM);
triple clone in MQTT `tenant_manager` on every state push (D11 Heap, MEDIUM); double `format!()`
in HA discovery topics (D11 Heap, MEDIUM); CLI `main.rs` at 5,915 lines needs decomposition
(D12 Maintainability, HIGH).

Key areas for improvement: the `web-api` crate at ~38K LoC is approaching "god crate" territory
and would benefit from decomposition; and remaining HA concerns should be addressed before
multi-instance deployment. The previously-reported SSRF in Docker auth realm, the OIDC
privilege escalation via `unwrap_or(0)`, all `#[cfg(not(feature))]` violations, the unbounded
MQTT event channel, and the scheduler claim leak on cancellation have been fixed. The OIDC
registration DB error masking (`unwrap_or(1)`), `count_linked_hosts` DB error swallowing,
`Report::new()` macro violations, invalid UUID query parameter handling, HTTP status code
violations on soft-delete and idempotent-create endpoints, and the `require_auth.rs`
permission-fetch fallback have since been fixed. The missing 5 capabilities in
`controller_capabilities()`, the absent `protocol_version` field in wire envelopes, the
insufficient 5-second scheduler shutdown timeout, the `ScheduledTaskType` rolling-upgrade
safety (`#[non_exhaustive]` + `find_due_tasks` skip-unknown), and the `machine_id`
empty-string sentinel in `ssh_host.rs` have also been fixed. The SSH pool TOCTOU race under
concurrent connection acquisition, the NATS `ServiceCredentials` leakage (now guarded by
`is_nats_publishable()`), the broken TOCTOU suppression in `create_or_ignore_ignore_rule`,
the empty output returned by `list_update_history` for streamed records, the rate-limit test
DB backdating, and all remaining `Report::new()` / `.expect()` violations in service-sdk,
agent-ssh, controller, and cli have been fixed. The `generate_secure_token()` UUID fallback in
OIDC auth (now returns HTTP 500 on RNG failure), the missing `#[serde(other)] Unknown` catch-all
on `ServiceMessage` and `ControllerMessage` (serde now decodes unknown message types gracefully
without hard errors, enabling rolling upgrades), the manually maintained
`validate_package_identifier` match block in the registry (now fully auto-generated by the
`register_plugins!` macro via per-config associated functions), the absence of an operator
warning when connecting to NATS over plaintext `nats://` (now emits `tracing::warn!`), and the
`lookup_by_secret` bearer-secret query without a `service_id` narrowing filter (now scoped via
optional URL query parameter during the enrollment window), the master key hex string (now wrapped
in `Zeroizing<String>` in controller startup), the dead `ServiceMessage`/`ControllerMessage`
without catch-all variants (now carry an `Unknown` serde catch-all), the manually maintained
`validate_package_identifier` (now auto-generated by the `register_plugins!` macro), and the
legacy file-based `JwtManager::load_or_generate` (removed; DB-backed key generation is the only
path) have been fixed. The in-memory-only token denylist (revocations now persisted to the
`revoked_token_jtis`/`revoked_token_users` DB tables and propagated to peer instances via
`ControllerMessage::TokenRevoked` over NATS), the OIDC registration code exposed in redirect URL
query parameters (now transmitted via URL hash fragment, invisible to server-side logs), the
duplicated `tokio::time::timeout` logic in `command/src/executor.rs` (extracted to a shared
`apply_timeout` helper), the enrollment retry in `service-sdk` catching only `ReceiveClosed`
(now retries all transient network errors via `EnrollmentError::is_transient_network()`), the
`Scheduler::register` silently overwriting executors (now guarded by `debug_assert!`), and the
NATS startup retry being absent (already implemented with exponential backoff in
`nats/src/connection.rs`) have been fixed. The Telegram webhook secret now uses SHA-256 +
constant-time `ct_eq` (no timing side-channel), the non-atomic batch completion
(`maybe_complete_batch`) is now wrapped in a DB transaction with a terminal-state guard,
the SSE batch progress and update output streams integrate `CancellationToken` for clean
shutdown, and the discovery allowlist TOCTOU has been fully resolved: unique DB constraints prevent
duplicate storage, and the application code now handles unique constraint violations from
concurrent inserts via a follow-up SELECT, achieving true idempotency. The missing tenant
filter in `trigger_scheduled_task`'s `update_many()` call has been fixed. The hidden
`.expect()` panic in `create_enrollment_token` has been replaced with proper `map_err`/`?`
error propagation. The `m20260306_000002_update_batches` migration now correctly restores
the `idx_update_history_created_at` index after dropping and recreating `update_history`.
The anonymous tuple `FetchGroupValue` in `scheduler-engine/src/executors/version_check.rs`
has been replaced with named structs. The `find_log_by_action_token` function now carries a
cross-tenant design rationale doc comment. The `UpdateResult` message from agents now uses
`conn.send()` (error-propagating) instead of `conn.send_best_effort()`, preventing
permanent in-progress state on send failure. `BatchStatus` and `UpdateCategory` now carry
an `Other(String)` catch-all for forward-compatible deserialization. All ten `Validate`
implementations in `web-api-types` now have test coverage. The `truncate()` helper in
`cli/batch_update.rs` now uses `chars().count()` / `chars().take()` to avoid panic on
multi-byte UTF-8 boundaries. `reencrypt_legacy_plaintext` and its five per-table helpers
now have 12 integration tests. The duplicated `NoopCommandExecutor` (inline in
`web-api/src/routes/software_items.rs` and `scheduler-engine/src/executors/fetch_releases.rs`)
has been extracted to `uptrakit_command::NoopCommandExecutor` and re-exported from `lib.rs`;
both consumers now import it from the canonical location. The OIDC `oidc_complete_registration`
function now validates the `registration_token` before consuming the one-time-use code — an
invalid token no longer permanently burns the code. The webhook `deliver()` method now enforces
`BLOCKED_HEADERS` as a defence-in-depth check (previously only `validate_config()` checked
them), using a shared `check_header_allowed()` helper that eliminates the duplicated blocklist.
`ChannelError` now carries `#[non_exhaustive]`. The
`UpdateNotificationRuleRequest` `host_id`, `software_item_id`, and `plugin_type` fields now use
the `Option<serde_json::Value>` nullable-update pattern so clients can clear scope filters to
NULL. A composite index `idx_update_history_host_item_status` on
`(host_id, software_item_id, status)` has been added via
`m20260311_000001_update_history_status_index` to eliminate the full table scan in
`validate_update_preconditions`. `validate_update_preconditions` (10 scenarios) and the
`find_outdated_*` helpers (6 scenarios) now have comprehensive unit tests using in-memory
SQLite. The dead `ParseCapabilityError` type has been removed.
The silent machine-ID fallback in `agent/src/host_info.rs` now logs a warning. The
byte-offset UUID slicing in `mqtt/src/main.rs` now uses safe `.get()`. `NatsEventEnvelope`
now derives `Debug`. The `validate_package_identifier_str` in the plugin registry now
returns an error for unknown plugin types instead of silently accepting. Bare "Not found"
messages in `settings_mqtt.rs` now read "MQTT client not found". The per-message executor
allocation in the agent has been refactored to a shared `Arc` on the handler. The duplicated
`execute`/`execute_quiet` logic in `agent-ssh/ssh_executor.rs` has been extracted to a
shared `run_remote()` method. `DEFAULT_POLL_INTERVAL_SECS` in `scheduler-engine` is now a
proper `Duration` constant. `STALE_CLAIM_SECONDS` now has a doc comment explaining its
relationship to `TASK_EXECUTION_TIMEOUT`. `uptrakit-backoff` and `uptrakit-web-api-types`
now carry `publish = false`.

## Per-Crate Review Files

| Crate | Review |
| --- | --- |
| `crates/core/controller` | [CODEREVIEW.md](crates/core/controller/CODEREVIEW.md) |
| `crates/core/agent-ssh` | [CODEREVIEW.md](crates/core/agent-ssh/CODEREVIEW.md) |
| `crates/core/agent` | [CODEREVIEW.md](crates/core/agent/CODEREVIEW.md) |
| `crates/core/mqtt` | [CODEREVIEW.md](crates/core/mqtt/CODEREVIEW.md) |
| `crates/core/scheduler` | [CODEREVIEW.md](crates/core/scheduler/CODEREVIEW.md) |
| `crates/core/integration-tests` | [CODEREVIEW.md](crates/core/integration-tests/CODEREVIEW.md) |
| `crates/shared` (umbrella: macros, directories, build-info, update-hooks) | [CODEREVIEW.md](crates/shared/CODEREVIEW.md) |
| `crates/shared/agent-core` | [CODEREVIEW.md](crates/shared/agent-core/CODEREVIEW.md) |
| `crates/shared/command` | [CODEREVIEW.md](crates/shared/command/CODEREVIEW.md) |
| `crates/shared/crypto` | [CODEREVIEW.md](crates/shared/crypto/CODEREVIEW.md) |
| `crates/shared/db` | [CODEREVIEW.md](crates/shared/db/CODEREVIEW.md) |
| `crates/shared/nats` | [CODEREVIEW.md](crates/shared/nats/CODEREVIEW.md) |
| `crates/shared/openapi-client` | [CODEREVIEW.md](crates/shared/openapi-client/CODEREVIEW.md) |
| `crates/shared/scheduler-engine` | [CODEREVIEW.md](crates/shared/scheduler-engine/CODEREVIEW.md) |
| `crates/shared/service-sdk` | [CODEREVIEW.md](crates/shared/service-sdk/CODEREVIEW.md) |
| `crates/shared/types` | [CODEREVIEW.md](crates/shared/types/CODEREVIEW.md) |
| `crates/shared/notification-channels` | [CODEREVIEW.md](crates/shared/notification-channels/CODEREVIEW.md) |
| `crates/shared/web-api-types` | [CODEREVIEW.md](crates/shared/web-api-types/CODEREVIEW.md) |
| `crates/shared/wire` | [CODEREVIEW.md](crates/shared/wire/CODEREVIEW.md) |
| `crates/ui/web-api` | [CODEREVIEW.md](crates/ui/web-api/CODEREVIEW.md) |
| `crates/ui/cli` | [CODEREVIEW.md](crates/ui/cli/CODEREVIEW.md) |
| `crates/plugins` (umbrella: generic/shell + cross-cutting) | [CODEREVIEW.md](crates/plugins/CODEREVIEW.md) |
| `crates/plugins/infrastructure/core` | [CODEREVIEW.md](crates/plugins/infrastructure/core/CODEREVIEW.md) |
| `crates/plugins/infrastructure/registry` | [CODEREVIEW.md](crates/plugins/infrastructure/registry/CODEREVIEW.md) |
| `crates/plugins/releases/docker` | [CODEREVIEW.md](crates/plugins/releases/docker/CODEREVIEW.md) |
| `crates/plugins/releases/github` | [CODEREVIEW.md](crates/plugins/releases/github/CODEREVIEW.md) |
| `crates/plugins/releases/forgejo` | [CODEREVIEW.md](crates/plugins/releases/forgejo/CODEREVIEW.md) |
| `crates/plugins/releases/gitlab` | [CODEREVIEW.md](crates/plugins/releases/gitlab/CODEREVIEW.md) |
| `crates/plugins/package-managers/apt` | [CODEREVIEW.md](crates/plugins/package-managers/apt/CODEREVIEW.md) |
| `crates/plugins/package-managers/npm` | [CODEREVIEW.md](crates/plugins/package-managers/npm/CODEREVIEW.md) |
| `crates/plugins/package-managers/homebrew` | [CODEREVIEW.md](crates/plugins/package-managers/homebrew/CODEREVIEW.md) |
| `crates/plugins/discovery/proxmox-helper-scripts` | [CODEREVIEW.md](crates/plugins/discovery/proxmox-helper-scripts/CODEREVIEW.md) |

## Architecture

### Strengths

- `Cargo.toml` (workspace root) -- `resolver = "3"` and `edition = "2024"` set once; all 35
  crates inherit via `edition.workspace` — no per-crate drift possible.
- `Cargo.toml:13-60` (`[workspace.dependencies]`) -- All 35 crates share major external
  dependencies pinned here with exact version ranges and default-feature overrides. Per-crate
  declarations only specify `features = [...]`, eliminating duplicate version declarations.
- `Cargo.toml:62-67` (`[profile.release]`) -- `lto = "fat"`, `codegen-units = 1`,
  `panic = "abort"`, `strip = true` — production-hardened single-binary output.
- `[workspace.package]` carries `license`, `authors`, `repository`, and `version`.
- Clean DAG dependency graph flowing from leaf types through shared libraries to core binaries.
  The four-domain layout (`core/`, `plugins/`, `shared/`, `ui/`) enforces a natural dependency
  gradient.
- Feature flags compose correctly: `db-sqlite`/`db-postgres`/`db-mysql` for backends, `oidc` for
  OIDC support, `nats` for NATS transport, `embedded-scheduler` for in-process scheduling.
- `ServiceHandler` trait in `service-sdk` is the architectural linchpin enabling agent, MQTT,
  SSH agent, and scheduler to share enrollment/lifecycle logic with minimal per-service code.
- `TenantScoped` marker trait (`db/src/entity/tenant_scoped.rs:9-16`) enables `TenantDb`
  extractor to automatically apply tenant filtering, eliminating an entire class of isolation
  bugs. Compile-time tenant filtering; tenant data leakage is structurally impossible through
  typed paths.
- Plugin system uses trait-based dispatch with `register_plugins!` macro that generates all six
  dispatch methods from a single declaration.
- `[workspace.lints]` enforces `warnings = "deny"` and `clippy::all = "deny"` across all 35
  crates via `[lints] workspace = true`.
- UUID v7 primary keys throughout all entities -- time-ordered, index-friendly, no hot-spot
  write contention.
- Partial (filtered) unique indexes on `software_items` and `plugin_configs`
  (`WHERE deactivated_at IS NULL`) -- prevents name collisions among active records while
  allowing post-soft-delete re-creation.
- Every foreign key has an explicit `ON DELETE` action -- no implicit cascade surprises.
- `m20260209_000001_initial.rs` -- `down()` drops all tables in correct reverse FK order.
- Referential integrity CHECK constraint on `sessions`:
  `auth_method != 'oidc' OR oidc_provider_id IS NOT NULL` -- enforced at DB level.
- Transactions used consistently for all multi-step mutations; `lock_exclusive()` acquired
  before `merge_service`.
- Batch plugin config loading in `list_ignore_rules` and JOIN-based `load_plugins` eliminate
  N+1 patterns.

### Issues

**[CRITICAL]** `crates/ui/web-api/src/settings_store.rs:13` and
`crates/core/controller/src/startup.rs:164-197` -- Settings persistence logic lives in the UI
layer (`web-api/settings_store.rs`) but is called from the core binary (`controller/startup.rs`).
This creates a logical layering inversion where the controller depends on web-api internals.

**[HIGH]** `crates/ui/web-api/src/lib.rs:1-24` -- At ~38K LoC, `web-api` is the largest crate
(~29% of total). Contains authentication, authorization, middleware, routes, queries, settings,
MQTT coordination, NATS transport, OCSP, PKI, notifications, batch progress broadcasting,
and update output broadcasting.

**[HIGH]** `crates/core/controller/Cargo.toml:22-60` -- Controller depends directly on
`uptrakit-web-api` (a UI crate), coupling the core binary to the entire web-api surface.

~~**[HIGH]** `crates/shared/wire/src/lib.rs` -- At 3,798 lines in a single file, the entire wire
protocol definition lives in one module. Should be decomposed into domain modules.~~ *(Fixed:
split into domain modules: `capabilities.rs`, `messages.rs`, `payloads.rs`, `envelope.rs`,
`serde_helpers.rs`, `close_reason.rs`, `service_profile.rs`, etc. `lib.rs` is re-exports only.)*

**[HIGH]** `crates/ui/web-api/src/lib.rs:1-34` -- Twenty-four modules are declared `pub` at the
crate root (`app_state`, `batch_progress_broadcaster`, `ca_snapshot`, `cert_signer`,
`device_flow_broadcaster`, `error_response`, `event_broadcaster`, `event_delivery`,
`extension_proxy`, `extension_registry`, `extract`, `middleware`, `mqtt_client_store`,
`mqtt_lease_coordinator`, `nats_transport`, `notification_service`, `notifications`, `ocsp`,
`pki_utils`, `router`, `routes`, `service_connections`, `settings`, `tenant_db`,
`update_output_broadcaster`). All are internal implementation details consumed only by the
controller binary. Only `AppState`, `build_router`, `build_pki_router`, `api_not_found`,
`SettingKey`, `CaKeyStoreRef`, `CaSnapshotReceiver`, and `ServiceCredentialSources` warrant
`pub` exports. The remaining modules should be `pub(crate)` once the controller → web-api
upward dependency is resolved via crate extraction. See per-crate review in
`crates/ui/web-api/CODEREVIEW.md`.

**[HIGH]** `crates/ui/web-api/src/app_state.rs:37-96` -- `AppState` has 32+ public fields, most
with `pub` visibility. The struct spans PKI, auth, database, notifications, extensions, OIDC
stores, SSE broadcasters, audit logging, and configuration. Consider grouping related fields
into sub-structs (e.g., `PkiState`, `AuthState`, `NotificationState`, `SseBroadcasters`) to
improve readability and allow partial injection in tests. The builder has 32 optional fields
with runtime `.ok_or()` checks -- a typestate builder pattern would catch missing fields at
compile time. *Note: 2026-03-06 parallel review confirmed 32+ fields, up from 26 reported
previously.*

**[MEDIUM]** `crates/shared/db/src/entity/oidc_provider.rs:89` -- Soft-delete column named
`deleted_at` instead of `deactivated_at`. All other 7 soft-deletable entities use
`deactivated_at`. Inconsistency prevents generic soft-delete utility.

**[MEDIUM]** `crates/shared/db/src/entity/update_history.rs:28` -- Dual output storage:
`output` column and `update_output_lines` child table. No DB constraint enforcing which storage
path is canonical.

**[MEDIUM]** `crates/core/controller/src/migration/m20260209_000001_initial.rs:615-621` -- Raw
SQL in migration seed uses `CURRENT_TIMESTAMP` which behaves differently across backends.

**[LOW]** `Cargo.toml` (workspace root) -- No `rust-version` MSRV declared. Without an MSRV
declaration, edition-2024 features may silently break older toolchains.

**[HIGH]** `crates/ui/web-api-queries/Cargo.toml:37` and
`crates/ui/web-api-queries/src/queries/software_items.rs:8` -- `web-api-queries` depends on
`uptrakit-plugin-infrastructure-registry`, which transitively pulls in all 11 plugin crates
(GitHub, Docker, APT, Homebrew, npm, MAS, GitLab, Forgejo, Proxmox, PHS, Shell). The queries
crate uses `PluginRegistry` only for `validate_config`, `mask_config_secrets`,
`restore_config_secrets`, and `PluginOps` trait methods. Any change to any plugin crate triggers
recompilation of the entire queries layer. Consider extracting the `PluginOps` trait into
`plugin-infrastructure-core` and having `web-api-queries` depend only on `core`, not `registry`.
The registry can provide the concrete implementation via `Arc<dyn PluginOps>` injection. The
same concern applies to `scheduler-engine` (`Cargo.toml:30`), which depends on
`plugin-infrastructure-registry` but only needs `create_plugin` and `validate_config` for
`FetchReleasesExecutor`. *(2026-03-06 parallel review: architecture + maintainability agents)*

**[MEDIUM]** `Cargo.toml` (workspace root) -- `sea-orm` and `sea-orm-migration` are pinned at
`2.0.0-rc.35`, a release candidate. The project's entire data layer (29 migrations, 55 entity
modules) depends on a pre-release ORM. Breaking changes between RC releases can require migration
rework, and ecosystem libraries may not keep pace with RC API changes. The workspace-wide pin
mitigates version skew, but a verification pass should be planned when SeaORM 2.0 stable ships.
*(2026-03-06 parallel review: maintainability + architecture agents)*

**[LOW]** `api_tokens` table -- No `expires_at` column. API tokens valid indefinitely once
issued.

## Security and Safety

### Strengths

- AES-256-GCM with random 96-bit nonces, master key in `OnceLock<Zeroizing<[u8; 32]>>`,
  `EncryptedString` with redacted Debug/Display, and `ENC:v1:` prefix for format versioning.
- Argon2id with OWASP parameters (19 MiB, 2 iterations) for password and enrollment token
  hashing.
- JWT with HS256, 15-minute expiry, required `iss`/`aud` claims, and explicit legacy rejection.
- Refresh token rotation in DB transactions with replay detection,
  HttpOnly/Secure/SameSite=Strict cookies.
- mTLS with ECDSA P-256 and `aws-lc-rs` provider. File permissions 0o700/0o600 for secrets.
- OIDC: PKCE (S256), single-use CSRF state with 10-minute TTL, email verification enforcement,
  encrypted PKCE verifiers at rest, Referrer-Policy on redirects.
- Shell escape uses POSIX single-quote wrapping. Direct exec mode bypasses shell entirely.
  `kill_on_drop(true)` and 10 MB output limit on commands.
- Proxy headers stripped from non-proxy requests. Certificate issuer CN verified against known
  CAs.
- No `unsafe` code outside of test utilities (2 instances in `directories` for env var tests).
- No secret values found in tracing calls across the entire codebase.
- `permission_extractor!` macro generates typed Axum extractors for all 9 permission levels.
  Authorization auditable by function signature alone.
- `SecretString` at all API input and output boundaries across `web-api-types`.
- CA private keys stored AES-256-GCM encrypted in DB and held as `Zeroizing<String>`.

### Issues

**[MEDIUM]** `crates/shared/service-sdk/src/tls.rs` -- TOFU TLS verifier accepts any server
certificate during initial CA fetch. MITM window during initial enrollment.

**[LOW]** `crates/shared/types/src/network.rs` -- DNS rebinding risk in SSRF validation.
`is_private_host()` validates the hostname/IP at configuration time, not at DNS resolution time.
A DNS name that resolves to a public IP at validation time could be rebound to a private IP at
request time. Mitigating factors: (1) webhook channel disables redirects; (2) reqwest's default
DNS resolver does not re-resolve during the same connection; (3) blocked DNS patterns catch
common rebinding names (`*.localhost`, `*.internal`, `*.local`). Residual risk: a custom domain
that passes validation but later resolves to 127.0.0.1 at delivery time could reach internal
services. Consider adding a custom DNS resolver or `reqwest` connection callback that validates
the resolved IP address at connection time. *(2026-03-06 parallel review: security agent)*

**[LOW]** `crates/shared/crypto/src/lib.rs:151-154,170` -- `DataKeyRing::new` uses `assert!` to
validate that `active_key_id` is present in the `keys` map, and `active_key()` at line 170 uses
`.expect("active key must exist in ring")`. The coding standard prohibits `unwrap()`, `expect()`,
and `panic!()` in production code. While documented with `# Panics`, this is production crypto
code where a startup failure should return `Result<Self, Report<CryptoError>>` rather than panic.
The `assert!` compiles in release builds. Add a `MissingActiveKey` variant to `CryptoError` and
convert `new` to return `Result`. The `active_key()` method can remain as-is since it is only
reachable after `new` succeeds and the ring is immutable. *(2026-03-06 parallel review: code
quality + security agents)*

**[LOW]** `crates/shared/directories/src/lib.rs:829,837`
 -- Test-only `unsafe` uses unguarded
`std::env::set_var` in potentially parallel tests. Data race under `cargo nextest`.

## Code Quality

### Strengths

- Consistent `rootcause` + `thiserror` + `impl_report_conversion!` error handling across all 35
  crates. Every crate with errors defines a custom error type with `#[derive(Debug, Error)]`.
- 2,363 test functions across 217 files with meaningful assertions and realistic scenarios.
- ~~Zero `#[allow(dead_code)]` annotations. Zero `#[allow(clippy::...)]` suppressions.~~
  *Corrected 2026-03-06: 5 `#[allow()]` instances found; see Coding Standards Issues below.*
- 76% of files have doc comments. All public traits, errors, and wire protocol types documented.
- `Zeroizing` wrapper used consistently for key material in the crypto crate.
- `CaKeyStore` `Debug` implementation manually redacts all key fields to `[REDACTED]` -- verified
  by dedicated test.
- Discrete startup phases use distinct typed structs (`ReconciledSettings`, `ValidatedConfig`,
  `PkiRuntime`).
- All domain-significant durations centralized in `durations.rs` with doc-comments.
- `#[tokio::test(start_paused = true)]` used correctly for all time-dependent tests in
  controller executors, command executor, and cert handler.
- Docker integration tests carry `#[ignore = "Docker integration test..."]` with exact runbook
  invocation commands.
- In-process SQLite via SeaORM for database tests -- full schema semantics, no external process.
- Security-sensitive paths have targeted test coverage: JWT wrong-secret rejection, denylist
  revocation, OIDC state one-time-use, device-flow consumption, session double-approve.
- Mock server infrastructure (`openapi-client/src/mock.rs`) well-designed for integration tests.

### Issues

**[HIGH]** `crates/ui/cli/src/main.rs` -- At 5,894 lines (including ~1,500 lines of tests),
this is the largest single file. The file defines the entire CLI command tree (clap structs,
subcommands, argument parsing, and dispatch). The commands themselves are extracted into
`commands/` modules, but the top-level `main.rs` contains all clap enum definitions. Splitting
the `Commands` enum and its subcommand enums into `args.rs` or `args/mod.rs` would improve
readability. *(Updated 2026-03-06: line count increased from 4,793 to 5,894.)*

**[HIGH]** Route handlers in `crates/ui/web-api/src/routes/` -- Multiple handlers have no
inline unit tests. `hosts.rs`, `agents.rs`, `settings_ca.rs`, `settings_mqtt.rs`,
`oidc_providers.rs`, `server_cert.rs`, `settings_auth.rs`, and `ocsp.rs` carry no
`#[cfg(test)]` module.

**[MEDIUM]** `test_state(db)` / `test_db()` / `NoopCertSigner` construction duplicated across
17+ test modules. Shared `test_helpers` module would eliminate duplication.

**[MEDIUM]** `crates/ui/cli/tests/command_execution.rs` -- Only `hosts`, `services`, and
`software_items` namespaces covered by integration tests.

**[HIGH]** `crates/ui/web-api/src/queries/update_batches.rs` (699 lines) and
`src/queries/update_triggers.rs` (436 lines) -- `validate_update_preconditions` and the
`find_outdated_*` helpers now have unit tests; `routes/update_batches.rs` and `create_batch`
remain untested.

**[MEDIUM]** `crates/shared/openapi-client/src/lib.rs:687-885` -- Retry-backoff tests do not
verify delay durations. Eight tests assert only eventual success, not backoff timing.

**[MEDIUM]** Dual `#[from]` + `impl_report_conversion!` in 5 files -- The error-handling docs
state: "Having both `#[from]` and `impl_report_conversion!` on the same variant is dead code:
the `From` impl is never called." When callers use `.context_to()?` (the prescribed pattern),
only `impl_report_conversion!` is exercised. Remove `#[from]` from all variants that have a
corresponding `impl_report_conversion!`. Affected files:
  - `crates/shared/agent-core/src/error.rs:31+36-38` -- `AgentCoreError::Io`
  - `crates/shared/scheduler-engine/src/error.rs:9+18` -- `SchedulerError::Database`
  - `crates/shared/openapi-client/src/error.rs:8-11+37-40` -- `ClientError::Http` and `ClientError::Json`
  - `crates/shared/crypto/src/lib.rs:63,72+89-92` -- `CryptoError::HexDecode` and `CryptoError::InvalidUtf8`
  - `crates/shared/service-sdk/src/error.rs:76-92+169-186` -- `EnrollmentError` variants and `TlsError` variants
*(2026-03-06 parallel review: code quality agent)*

**[MEDIUM]** Missing `Validate` implementations on several request types -- The project standard
states all HTTP request types implement `Validate` and route handlers call `.validate()`.
Several request types lack `Validate` implementations:
  - `UpdateHostRequest` (`web-api-types/src/hosts.rs`) -- no length/content validation on `friendly_name`. Route handler at `hosts.rs:119` does not call `body.validate()`.
  - `UpdateSoftwareItemRequest` -- validation exists in the query layer (`EmptyName`), but not via the `Validate` trait at the API boundary.
  - `UpdatePluginConfigRequest` -- no `Validate` impl; empty name could be set.
  - `AssignHostsRequest` -- no `Validate` impl; no check for empty `host_assignments` or empty `plugins` per assignment.
  - `TriggerUpdateRequest` -- no `Validate` impl; `to_version` could be an empty string.
*(2026-03-06 parallel review: consistency agent)*

**[LOW]** `crates/ui/web-api/src/routes/update_history.rs:169` -- DB error silently swallowed
in SSE handler. `.all(tenant_db.db()).await.unwrap_or_default()` converts a database error
into an empty vector. A transient DB failure would silently return zero output lines, making
completed updates appear to have no output. Match on the result, log the DB error, and emit
an SSE error event. *(2026-03-06 parallel review: code quality agent)*

## High Availability

### Strengths

- `CancellationToken` used for cooperative shutdown across all services. `BackgroundTasks` in
  controller provides named handles with both cooperative and forceful shutdown modes.
- `ServerRestarting` notifications scattered over 5 seconds to prevent thundering herd.
- Exponential backoff with jitter in `service-sdk/src/backoff.rs` prevents thundering herd on
  reconnect. All services use this consistently.
- Optimistic locking via `UPDATE ... WHERE locked_by IS NULL` in scheduler prevents double
  execution across instances. Stale claim recovery (10-minute threshold) handles crashes.
- MQTT client has LWT for offline status, bounded per-service channels (capacity 32), and
  connection deduplication in `ServiceConnectionRegistry`.
- Settings use `watch` channel for atomic snapshot publishing with serialized writes.
- `send()` acquires read lock, clones sender, drops lock before async send -- no lock held
  across await points.
- Event poller advances cursor only past successfully delivered events. `MAX_DELIVERY_RETRIES`
  (3) prevents single bad event from blocking all delivery.
- User-level token revocation uses latest-timestamp guard (`if until > *entry`) to prevent
  concurrent revocation from narrowing the window.
- MQTT event delivery uses targeted routing via `mqtt_client_index` -- routes tenant-specific
  messages to specific MQTT service instance.

### Issues

**[CRITICAL]** `crates/ui/web-api/src/notifications/dispatcher.rs:30` --
`NotificationDispatcher` uses `mpsc::unbounded_channel()` with no capacity bound. Under
sustained high event volume or cascading failures (e.g., a burst of update events when many
agents reconnect simultaneously), the MPSC queue grows without bound, risking OOM. Replace
with `mpsc::channel(N)` and a named capacity constant. On send failure emit `tracing::warn!`
and drop the message, making backpressure explicit rather than implicit heap growth. See
per-crate review in `crates/ui/web-api/CODEREVIEW.md`.

**[HIGH]** `crates/core/mqtt/src/tenant_manager.rs:81-93` -- `TenantManager::shutdown_all()`
races with the event channel receiver. `std::mem::take` at line 82 drains `self.clients`
before `report_status` at line 90 attempts to send `Offline` status via `self.event_tx`. If
the `MqttHandler` receiver has already been torn down, the final `Offline` delivery is
silently lost. In contrast, `stop_client` (line 70) calls `report_status` while the channel
is live. The divergent delivery guarantees across the two shutdown paths are undocumented. See
per-crate review in `crates/core/mqtt/CODEREVIEW.md`.

**[HIGH]** `crates/ui/web-api/src/batch_progress_broadcaster.rs` -- Instance-local only. In a
multi-instance deployment, SSE clients connected to instance A receive no live events for
batches processed by instance B.

**[MEDIUM]** `crates/ui/web-api/src/batch_progress_broadcaster.rs:64-66` and
`update_output_broadcaster.rs:48-49` -- Orphaned channel leak. If batch/update never reaches
terminal state, broadcast channel persists indefinitely.

~~**[MEDIUM]** `crates/ui/web-api/src/nats_transport.rs:162-164` -- NATS consumer fetch error
retry uses fixed 1-second delay with no exponential backoff or jitter.~~

> **Fixed:** `run_consumer` now uses `uptrakit_backoff::Backoff::new(1s, 30s)` with
> exponential backoff and jitter. The backoff resets on every successful fetch.

**[MEDIUM]** `crates/shared/service-sdk/src/event_loop.rs:244-246` -- `tick().await` inside
`handle_service_settings` suspends the event loop, blocking incoming WebSocket reads and pings.

**[CRITICAL]** No crash recovery for in-flight updates (HA-11) -- If an agent crashes while
executing an update (`execute_update` in `agent-core/src/update.rs`), the `update_history` row
remains in `InProgress` status. There is no scheduled task or timeout mechanism that transitions
stale `InProgress` updates to `Failed`. The partial unique index `uix_update_history_host_active`
on `host_id` (from `m20260313_000001_per_host_update_locking.rs`) then blocks any new updates for
that host until manual cleanup. This is the most significant HA gap identified. **Recommended
mitigation:** Add a scheduled task (e.g., `StaleUpdateCleanup`) that transitions `InProgress`
updates older than `TASK_EXECUTION_TIMEOUT` (e.g., 2 hours) to `Failed` status.
*(2026-03-06 parallel review: HA agent)*

**[HIGH]** In-flight update executions not drained on agent shutdown (HA-1) -- The `on_shutdown`
callback in `ServiceHandler` (`service-sdk/src/lifecycle.rs:207-218`) receives a
`shutdown_timeout` parameter (a `Duration`), but there is no mechanism to wait for an in-flight
`execute_update` to complete before the WebSocket closes. If an agent receives SIGTERM while
executing an update, the update is left in `InProgress` status. Implement update execution
draining in `on_shutdown()` by waiting (up to `shutdown_timeout`) for any in-flight
update to complete before returning `LoopOutcome`.
*(2026-03-06 parallel review: HA agent)*

**[MEDIUM]** No concurrency limit on parallel version-check subprocess invocations per agent
(HA-13) -- In `batch_check_versions()` at `agent-core/src/version_check.rs:155-233`, all detect
groups are run in parallel via `join_all()`. With many plugin groups, this could spawn many
concurrent subprocess calls (`dpkg-query`, `brew info`, `npm list`, etc.) simultaneously. A
`tokio::sync::Semaphore` limiting concurrent invocations to 4-8 parallel groups would prevent
resource exhaustion. *(2026-03-06 parallel review: HA agent)*

## Coding Standards

### Strengths

- All 35 crates use `workspace = true` for shared dependencies and `[lints] workspace = true`.
- `TenantDb` wrapper used consistently across all tenant-scoped web-api routes.
- All string-to-type conversions use `FromStr` with dedicated `Parse{Type}Error` types.
- Batch loading with `is_in(ids)` and `HashMap` lookups used in established query modules.
- Database-backed rate limiting with atomic SQL upsert, HA-safe across controller instances.
- Structured tracing with proper levels. No sensitive data in log output.
- `bail!` / `report!` / `context_to` / `impl_report_conversion!` pattern uniform; `Report::new()`
  anti-pattern absent across all 35 crates; `Result<T, String>` absent from all library
  boundaries.
- No `StatusCode` numeric literal comparisons anywhere in the codebase.
- 70+ endpoints carry `x-required-permission` OpenAPI extension annotations.
- Zero `anyhow` usage across all 35 crates.

### Issues

**[HIGH]** Four `#[cfg(not(feature = ...))]` violations -- The coding standard prohibits
`#[cfg(not(feature = "X"))]` (additive-only feature flags). Four instances found:
  1. `crates/core/controller/src/main.rs:94` -- `#[cfg(not(feature = "journald"))]` should use `cfg!()` macro pattern
  2. `crates/core/controller/src/main.rs:304` -- `#[cfg(not(feature = "nats"))]`
  3. `crates/ui/web-api/src/batch_progress_broadcaster.rs:114` -- `#[cfg(not(feature = "nats"))]`
  4. `crates/ui/web-api/src/routes/settings_global_combined.rs:76` -- `#[cfg(not(feature = "nats"))]`

All should be converted to the approved `cfg!()` macro pattern or `if cfg!(feature = "...")`
blocks. While the current usage provides fallback values (not subtracting behavior), maintaining
a bright-line rule against `#[cfg(not(...))]` prevents future misuse. These span controller and
web-api crates, making this a cross-cutting concern.
*(2026-03-06 parallel review: coding standards + architecture agents)*

**[HIGH]** Five `#[allow()]` violations -- The standard states no Clippy suppression is approved
(AGENTS.md invariant 13):
  1. `crates/ui/web-api/src/extension_proxy.rs:114` -- `#[allow(clippy::too_many_arguments)]` on `invoke` method with 8 parameters. Use a parameter struct per the "Parameter Struct Pattern" standard.
  2. `crates/plugins/package-managers/mas/src/plugin.rs:143` -- `#[allow(dead_code)]` on `config` field. Remove unused field or add a trivial accessor.
  3. `crates/core/agent-ssh/src/remote_exec.rs:49,60` -- `#[allow(dead_code)]` on `PveGuestExecutor` struct and `new()`. Pre-committed dead code for Phase 6 should be behind a feature flag or removed until needed.
  4. `crates/ui/web-api/src/test_harness/mod.rs:23` -- `#[allow(dead_code)]` on `TestApp` struct (unnecessary if fields are used by integration tests).
  5. `crates/ui/web-api/src/routes/service_ws/protocol.rs:93` -- `#[allow(dead_code)]` on a `#[cfg(test)]` method.

These span web-api, plugins, and agent-ssh crates. The previously reported "zero `#[allow()]`
suppressions" was incorrect. *(2026-03-06 parallel review: coding standards agent)*

## Extensibility

### Strengths

- Plugin system: trait with default implementations, capability-based feature declaration,
  macro-generated registry dispatch, JSON configuration with `SecretMasking`. Adding a new plugin
  requires one line in `register_plugins!`.
- Wire protocol: `Other(String)` catch-all on `PluginType`, `PluginRole`, `Capability`, and
  `CloseReason` for forward-compatible deserialization.
- `ServiceHandler` trait uses associated type `ServiceEvent` (defaulting to `Infallible`),
  associated constants, and default implementations for clean extensibility.
- Feature gates on `shared-types` (`sea-orm`, `openapi`), `shared-db` (`migration`,
  `db-sqlite/postgres/mysql`), and `plugin-registry` (`daemon`, `ssh`).
- Migrations follow date-stamped naming with `up()`/`down()` methods. JSON columns for
  schema-free configuration evolution.
- `SecretMasking` trait with no-op defaults -- plugins without secrets need no masking code.
- Sequence validation decoupled from full deserialization enables forward-compatible message
  handling.

### Issues

**[MEDIUM]** `crates/shared/wire/src/messages.rs` -- `ServiceMessage` and
`ControllerMessage` mix agent-specific and MQTT-specific variants.

**[MEDIUM]** `crates/shared/service-sdk/src/lifecycle.rs:79,89` -- `ServiceHandler` is not
object-safe due to associated constants. No documentation or `where Self: Sized` guards.

**[MEDIUM]** `crates/shared/types/src/plugin_types.rs` -- `PluginType` requires 6 match-arm
updates per new variant (`as_str()`, `display_name()`, `FromStr`, `From<String>`,
`From<PluginType> for String`, `Serialize/Deserialize`). This is manual and error-prone. A
declarative macro or strum-like derivation (adapted for the `Other(String)` pattern) could
reduce this to a single declaration per variant. *(2026-03-06 parallel review: extensibility
agent)*

**[LOW]** `crates/shared/types/src/plugin_types.rs` -- `AttestationStatus` is
`#[non_exhaustive]` but lacks `Other(String)` for wire safety. If attestation status values are
added by a newer controller, an older agent would fail to deserialize them. Given that
`AttestationStatus` is sent in `ReleaseInfo` over the wire (`ExecuteUpdatePayload`), this is
a latent deserialization risk. *(2026-03-06 parallel review: extensibility agent)*

**[LOW]** `crates/shared/audit-log/src/entry.rs` -- `AuditEntry` struct is not
`#[non_exhaustive]`. Adding fields would require updating all constructors in consuming crates.
Since the audit-log crate is used across the workspace, this is inconsistent with the project's
conventions. Similarly, `AuditActorType` in the same file lacks `#[non_exhaustive]` and could
gain new variants (e.g., `ServiceAccount`, `System`) as the auth model evolves.
*(2026-03-06 parallel review: extensibility agent)*

## Crate Structure

### Splitting Candidates

#### `uptrakit-web-api` (~38K LoC, ~98 files)

**Current concerns:** The crate contains 8 distinct domains: authentication (JWT, OIDC, sessions,
device flow, token denylist), authorization (permissions, rate limiting), routes (30+ handler
files), queries (10+ DB query modules), settings (settings store, reconciliation), MQTT
coordination (lease coordinator, MQTT client store), PKI/OCSP (CA snapshot, CRL, OCSP, cert
signer), and cross-controller transport (NATS transport, notification service, event delivery).

**Proposed split:**

| New crate | Contents | Current files |
| --- | --- | --- |
| `uptrakit-web-api-auth` | JWT, sessions, token denylist, password hashing, device flow, OIDC state stores, rate limiting | `src/auth/` (16 files) |
| `uptrakit-web-api-core` | `AppState`, `Settings`, `TenantDb`, `SettingKey`, settings store, CA snapshot, cert signer, error helpers | `app_state.rs`, `settings.rs`, `settings_store.rs`, `tenant_db.rs`, `ca_snapshot.rs`, `cert_signer.rs`, `setting_key.rs` |
| `uptrakit-web-api-routes` | All HTTP route handlers and the router | `src/routes/` (30 files), `router.rs` |
| `uptrakit-web-api` (rump) | NATS transport, notification service, MQTT coordination, OCSP, update broadcaster, event delivery | remaining files |

**Priority:** MEDIUM — the crate is functional and its internal structure is clean; the split
has high migration cost (all import paths change, controller must be re-wired) with moderate
benefit (faster incremental compilation, clearer ownership boundaries).

**Estimated benefit:** Faster compilation of route-only changes (routes depend on auth and core
but not on each other). Cleaner separation allows `uptrakit-web-api-auth` to be tested without
instantiating `AppState`. Independent versioning of the auth layer is possible.

**Risk:** `AppState` is referenced in every route handler; a split requires either passing
sub-state structs through Axum extractors (significant refactor) or keeping `AppState` in a
shared crate that all others depend on (which reduces the compile-time benefit). This is a
high-churn change that should be deferred until the `AppState.pub` field problem is resolved
first.

---

#### ~~`uptrakit-shared-wire` (3,798 lines, single file)~~ -- DONE

*(Fixed: the crate has been split into domain modules: `src/capabilities.rs`, `src/messages.rs`,
`src/payloads.rs`, `src/envelope.rs`, `src/serde_helpers.rs`, `src/close_reason.rs`,
`src/service_profile.rs`, and more. `src/lib.rs` contains only re-exports. All types remain in the
same crate; consumers see no change to import paths.)*

---

#### `uptrakit-controller` (startup complexity)

**Current concerns:** The controller binary directly instantiates and owns three distinct
infrastructure subsystems that have no logical relationship to each other at the binary level:
the PKI lifecycle (`pki.rs`, `crl_manager.rs`), the scheduler (`scheduler/mod.rs`), and the
re-encryption pass (`reencrypt.rs`). The scheduler block is already partially abstracted via
`uptrakit-scheduler-engine`; PKI is not.

**Recommendation:** Extract PKI utilities to `uptrakit-shared-pki` (absorbing
`controller/src/pki.rs` DER encoding helpers and `web-api/src/pki_utils.rs`). The controller
and web-api both import PKI utilities, so this would eliminate the current layering inversion
where the controller must depend on `uptrakit-web-api` partly to access `pki_utils`.

**Priority:** LOW — requires resolving the `AppState`/web-api layering issue first. The PKI
extraction is blocked by the fact that `web-api` owns `CaKeyStore` and `CaPublicSnapshot`, which
are used by both the controller and web-api routes.

**Risk:** Medium. PKI is security-critical; any refactor of the key store types requires careful
audit of all call sites.

---

### Merging Candidates

#### `uptrakit-backoff` → into `uptrakit-service-sdk`

**Rationale:** `uptrakit-backoff` is 105 lines with a single public struct (`Backoff`). Its only
two consumers are `uptrakit-service-sdk` and `uptrakit-nats`. The crate exists as a separate
entity to avoid a dependency loop (service-sdk → backoff, nats → backoff), but the same
isolation is achievable by moving the `Backoff` struct into a `backoff` sub-module of
`uptrakit-service-sdk` and having `uptrakit-nats` depend on `uptrakit-service-sdk` instead.

**Priority:** LOW — the current arrangement is correct (no circular deps) and the overhead of an
extra crate is minimal in a 35-crate workspace. Merging is cosmetic.

**Risk:** Low. `uptrakit-nats` would gain a dependency on `uptrakit-service-sdk` (adding TLS,
WebSocket, and Clap as transitive deps), which may be undesirable. Keep as-is until `nats`
needs more SDK functionality.

---

#### `uptrakit-shared-macros` (142 lines) — keep as-is

**Assessment:** Although `uptrakit-shared-macros` is a single 142-line file containing one
macro, it is used by 23 crates across the workspace. Its tiny size is not a maintenance burden;
the macro is proc-macro-adjacent in concept (but implemented as a declarative macro in a normal
crate, which is correct). Merging it into `uptrakit-shared-types` would create a dependency
cycle (`types` → `macros` is fine, but many crates that use `macros` do not need `types`).
Keep as a separate leaf crate.

---

#### `uptrakit-update-hooks` (768 lines) → into `uptrakit-web-api-types` or `uptrakit-agent-core`

**Rationale:** `uptrakit-update-hooks` has exactly one consumer: `uptrakit-web-api`. Its only
dependency is `uptrakit-internal-wire` and `uptrakit-web-api-types`. The module is logically
"hook configuration processing for update execution" — which belongs in `uptrakit-agent-core`
or `uptrakit-web-api-types`. Moving it to `uptrakit-agent-core` is the better fit because
hook resolution is consumed by agents during update execution.

**Priority:** MEDIUM — reduces the crate count by one and moves hook logic closer to its
consumers. The current arrangement has `web-api` depending on `update-hooks` for what is
conceptually agent-level logic.

**Risk:** Low. No public API change; only import paths change.

---

#### `uptrakit-build-info` (216 lines) — keep as-is

**Assessment:** `uptrakit-build-info` is consumed by 7 crates (all binaries and `service-sdk`)
as both a regular dependency and a build dependency. It provides compile-time build metadata
embedded via `build.rs`. Merging into any other crate would require those crates to also carry
a `build.rs`, unnecessarily complicating them. Keep as a separate leaf crate.

---

### Module Extraction (without crate split)

#### `uptrakit-shared-wire` — split `src/lib.rs` into sub-modules

Covered above under Splitting Candidates. This is the highest-priority structural improvement
that can be made to the wire crate without any breaking changes to consumers.

#### `uptrakit-scheduler-engine` — extract `version_check.rs` query helpers

`src/executors/version_check.rs` at 653 lines is the largest file in the scheduler-engine
crate and contains both the executor logic and inline query row types
(`ControllerFetchRow`, `AgentAssignmentRow`, `FetchGroupKey`). Extracting the query row
definitions and query functions to `src/queries/version_check.rs` would follow the same
pattern used in `web-api` (routes vs queries) and make the executor logic easier to read.

#### `uptrakit-web-api/src/queries/autodiscovery.rs` — split at concern boundary

At 1,846 lines, `autodiscovery.rs` contains two unrelated concerns: ignore-rule management
(~170 lines) and discovery-result processing (~1,676 lines). These should be
`queries/autodiscovery_ignore_rules.rs` and `queries/autodiscovery_processing.rs` within the
same module.

---

### Dependency Graph Analysis

The dependency graph is a valid DAG with no circular dependencies. Key observations:

1. **Controller → web-api (upward dependency):** The most significant structural issue. The
   controller binary (`core/`) depends on a UI-layer crate (`ui/web-api`). This means any
   change to web-api triggers controller recompilation. The resolution is to extract `AppState`
   construction and settings into a shared crate, making web-api depend on it rather than the
   reverse.

2. **Graph depth is shallow:** The longest dependency chain is approximately:
   `types` → `wire` → `service-sdk` → `agent-core` → `agent`. This 5-hop depth is healthy
   and means most leaf changes compile quickly.

3. **`uptrakit-plugin-infrastructure-registry` is broadly imported:** 6+ crates depend on it
   directly. The registry is a legitimate hub crate, but any change to it (e.g., adding a
   new dispatch method) triggers recompilation of all consumers. Feature flags
   (`daemon`, `ssh`) help limit this.

4. **`uptrakit-directories` is not in `[workspace.dependencies]`:** It is referenced via
   `path = ...` in 7 crates. Moving it to `[workspace.dependencies]` would standardize the
   reference pattern and make it eligible for workspace-level feature management. (The same
   applies to several other path-only crates: `uptrakit-nats`, `uptrakit-scheduler-engine`,
   `uptrakit-notification-channels`.)

---

### Overall Assessment

The workspace crate organization is generally healthy. The four-domain layout (`core/`,
`shared/`, `ui/`, `plugins/`) enforces a clean dependency gradient for most crates. The main
structural debt is the `controller → web-api` upward dependency, which is a known issue and the
root cause of several other concerns (settings logic in the UI layer, `AppState` with 26 public
fields). Resolving this one dependency inversion would enable the most impactful follow-on
improvements: a web-api split, PKI extraction, and a cleaner `AppState` API surface.

The highest-value, lowest-risk improvement is the `uptrakit-shared-wire` module extraction:
splitting `src/lib.rs` into domain-specific sub-modules with `lib.rs` as a re-export façade.
This is a pure file reorganization with no dependency changes and no impact on consumers, and
it directly addresses the single largest navigation pain point in the codebase.

---

## Test Coverage Analysis

- **Analysis date**: 2026-03-05
- **Tool**: `cargo-llvm-cov` (all features, workspace, unit + integration tests)
- **Branch**: docs/test-coverage

### Overall Coverage

| Metric | Covered | Total | Percentage |
| --- | --- | --- | --- |
| Lines | 60,518 | 92,310 | 65.6% |
| Functions | 6,921 | 10,235 | 67.6% |
| Regions | 89,839 | 133,607 | 67.2% |

### Per-Crate Coverage (sorted by uncovered lines)

| Crate | Lines | Covered | Uncovered | Coverage |
| --- | ---: | ---: | ---: | ---: |
| web-api | 18,914 | 9,090 | 9,824 | 48.1% |
| cli | 8,502 | 4,170 | 4,332 | 49.0% |
| web-api-queries | 7,786 | 4,319 | 3,467 | 55.5% |
| controller | 5,897 | 2,835 | 3,062 | 48.1% |
| agent-ssh | 6,091 | 3,510 | 2,581 | 57.6% |
| openapi-client | 3,268 | 1,622 | 1,646 | 49.6% |
| service-sdk | 2,815 | 1,644 | 1,171 | 58.4% |
| agent-core | 1,694 | 852 | 842 | 50.3% |
| mqtt | 2,179 | 1,516 | 663 | 69.6% |
| web-api-auth | 3,575 | 2,977 | 598 | 83.3% |
| scheduler-engine | 2,600 | 2,046 | 554 | 78.7% |
| shared-db | 1,532 | 1,057 | 475 | 69.0% |
| docker plugin | 1,840 | 1,463 | 377 | 79.5% |
| wire | 3,375 | 3,085 | 290 | 91.4% |
| web-api-types | 6,196 | 6,003 | 193 | 96.9% |
| scheduler | 274 | 83 | 191 | 30.3% |
| agent | 373 | 202 | 171 | 54.2% |
| github plugin | 891 | 745 | 146 | 83.6% |
| homebrew plugin | 1,092 | 981 | 111 | 89.8% |
| PHS plugin | 1,408 | 1,299 | 109 | 92.3% |
| nats | 375 | 267 | 108 | 71.2% |
| types | 2,010 | 1,911 | 99 | 95.1% |
| crypto | 1,056 | 958 | 98 | 90.7% |
| forgejo plugin | 707 | 617 | 90 | 87.3% |
| notification-channels | 625 | 541 | 84 | 86.6% |
| gitlab plugin | 639 | 564 | 75 | 88.3% |
| apt plugin | 982 | 908 | 74 | 92.5% |
| npm plugin | 782 | 715 | 67 | 91.4% |
| directories | 621 | 565 | 56 | 91.0% |
| command | 1,020 | 982 | 38 | 96.3% |
| plugin-core | 646 | 619 | 27 | 95.8% |
| audit-log | 149 | 125 | 24 | 83.9% |
| shell plugin | 257 | 245 | 12 | 95.3% |
| update-hooks | 475 | 469 | 6 | 98.7% |
| backoff | 55 | 55 | 0 | 100.0% |
| macros | 26 | 26 | 0 | 100.0% |

### Files With 0% Coverage (>50 lines)

These files contain zero test-exercised code paths:

| File | Lines | Category |
| --- | ---: | --- |
| `service_ws/handler/updates.rs` | 665 | WebSocket update handling |
| `controller/src/main.rs` | 497 | Controller binary entry |
| `service_ws/handler/messages.rs` | 412 | WebSocket message handling |
| `agent-ssh/src/commands/host.rs` | 393 | SSH host management CLI |
| `agent-core/src/client.rs` | 372 | Agent client logic (inner helpers now partially tested) |
| `service-sdk/src/ws.rs` | 308 | WebSocket SDK |
| `service_ws/handler/mqtt.rs` | 248 | MQTT trigger via WS |
| `mqtt_client_store.rs` | 202 | MQTT client store |
| `commands/sync.rs` | ~520 | Host sync (sudoers, PVE node name, PVE privilege verification) |
| `queries/audit_logs.rs` | 149 | Audit log queries |
| `service-sdk/src/connection.rs` | 130 | Service connection |
| `service_ws/handler/renewal.rs` | 119 | Certificate renewal handler |
| `queries/scheduled_tasks.rs` | 110 | Scheduler task queries |
| `agent/src/main.rs` | 89 | Agent binary entry |
| `nats/src/connection.rs` | 86 | NATS connection |
| `docker/src/docker_proxy.rs` | 83 | Docker-over-SSH proxy |
| `controller/src/server.rs` | 79 | Controller server setup |
| `openapi-client/src/notifications.rs` | 76 | Notification API client |
| `mqtt/src/main.rs` | 66 | MQTT binary entry |
| `routes/ocsp.rs` | 63 | OCSP responder route |
| `agent/src/client.rs` | 62 | Agent client |
| `service_ws/handler/discovery.rs` | 60 | Discovery handler |
| `openapi-client/src/autodiscovery.rs` | 54 | Autodiscovery API client |

### Security-Critical Files Below 70% Coverage

| File | Coverage | Risk |
| --- | ---: | --- |
| `controller/src/crl_manager.rs` | 28.9% | CRL generation and distribution |
| `controller/src/mtls_acceptor.rs` | 0.0% | mTLS connection acceptance |
| `openapi-client/src/enrollment_tokens.rs` | 49.2% | Enrollment token operations |
| `openapi-client/src/system_enrollment_tokens.rs` | 45.6% | System enrollment tokens |
| `auth/authentication.rs` | 51.7% | OIDC user resolution |
| `settings_store.rs` | 35.3% | Settings persistence |
| `queries/enrollment_tokens.rs` | 62.4% | Enrollment token queries |
| `queries/system_enrollment_tokens.rs` | ~60% | System enrollment token queries (6 unit tests added) |
| `ocsp.rs` | 48.6% | OCSP response building |
| `routes/ocsp.rs` | 0.0% | OCSP route handler |
| `routes/oidc_auth.rs` | 8.8% | OIDC auth flow |
| `routes/device_auth.rs` | 9.7% | Device auth flow |
| `routes/api_tokens.rs` | 25.0% | API token CRUD |
| `routes/settings_auth.rs` | 27.4% | Auth settings |
| `routes/system_enrollment_tokens.rs` | 31.2% | System enrollment token routes |

### Priority Test Recommendations

#### Tier 1: Security-Critical (test first)

1. **`resolve_oidc_user` email verification** (`authentication.rs`): The `email_verified = None`
   and `email_verified = Some(false)` paths must return `EmailNotVerified`. A bug here enables
   account takeover via an IdP that omits or falsifies the claim.

2. **`validate_update_ownership`** (`handler/updates.rs`): Cross-tenant isolation gate. Must
   reject update records belonging to unlinked hosts.

3. **MQTT trigger tenant assignment** (`handler/mqtt.rs`): `handle_mqtt_trigger_update` and
   `handle_mqtt_trigger_host_package_update` must reject operations for unassigned tenants.

4. ~~**`find_active_system_tokens` compound filter** (`system_enrollment_tokens.rs`): All three
   conditions (not revoked, not expired, uses remaining) must be ANDed correctly. A logic bug
   would allow enrollment with exhausted or revoked tokens.~~

   > **Fixed:** 6 unit tests added covering expired, revoked, exhausted, unlimited,
   > partially-used, and null-expiry token cases.

#### Tier 2: Data Integrity (prevent silent corruption)

1. **`handle_update_result` version propagation** (`handler/updates.rs`):
   `installed_version` must be written to `host_software_item` on `Completed`
   status, not on `Failed`.
1. ~~**`deactivate_system_service` transaction** (`system_services.rs`): Must
   atomically mark `deactivated_at` and bulk-revoke all certificates.~~

   > **Fixed:** 6 unit tests added covering atomic deactivation, idempotency,
   > approve/reject state transitions, and `Some(0)` clearing columns to `NULL`.
1. **`sync_oidc_roles` atomicity** (`authentication.rs`): Must delete all existing
   roles before inserting mapped ones; partial application mixes old and new.
1. **`deliver_pending_updates` batch deduplication** (`handler/updates.rs`): Must
   skip non-first pending updates per `(batch_id, host_id)` on reconnect.

#### Tier 3: Business Rule Correctness

1. **Discovery allowlist three-level precedence** (`handler/discovery.rs`):
   Host-specific overrides tenant-wide, which overrides all-allowed default.
1. **`deactivate_missing_host_packages` ignore-set** (`host_packages.rs`): Ignored
   packages must never be deactivated even when absent from discovery snapshot.
1. **`handle_graceful_shutdown` sequencing** (`agent-core/client.rs`): Must send
   `UpdateResult` before `Disconnecting`; timeout must send `Failed` result.
   `batch_host_package_update_inner` and `run_check_versions` inner helpers now have
   unit tests; the `ControllerConnection`-dependent shutdown path remains untested.
1. **Cron normalization in `update_scheduled_task`** (`scheduled_tasks.rs`):
   5-field expressions must be prefixed with `0` for seconds field; invalid cron
   must return error.

#### Tier 4: Error Resilience

1. **Allowlist `load_*_set` fail-open** (`discovery_allowlist.rs`): Must return
   empty set on DB error so discovery is not blocked.
1. **`handle_discover_software` isolation** (`agent-core/client.rs`): One failing
   plugin must not prevent other plugins from running.
1. **OIDC callback error redirects** (`oidc_auth.rs`): All `OidcUserResolution`
   error variants must redirect to `/login?error=...`, not return 500.

### Well-Tested Areas (>90% coverage, >100 lines)

These files demonstrate excellent test discipline:

- `wire/src/` (2,410 lines, 99.0%) - wire protocol
- `crypto/src/lib.rs` (1,056 lines, 90.7%) - cryptographic operations
- `phs/discovery.rs` (970 lines, 97.0%) - PHS discovery logic
- `registry/src/registry.rs` (943 lines, 97.3%) - plugin registry
- `apt/src/plugin.rs` (872 lines, 91.7%) - APT plugin
- `mqtt/ha_discovery.rs` (857 lines, 99.9%) - HA discovery topics
- `controller/reencrypt.rs` (767 lines, 92.2%) - re-encryption
- `web-api-types/src/lib.rs` (767 lines, 100.0%) - API types
- `npm/src/plugin.rs` (732 lines, 90.8%) - npm plugin
- `agent-ssh/host_ops.rs` (679 lines, 95.1%) - SSH host operations
- `command/src/command.rs` (478 lines, 95.6%) - command executor
- `update-hooks/src/lib.rs` (475 lines, 98.7%) - update hooks

---

## Cross-Cutting Test Quality Issues

*(2026-03-06 parallel review: tests agent)*

**[HIGH]** `thiserror` Display format tests violating testing philosophy -- Per
`docs/development/testing.md`, tests that verify `thiserror` `#[error("...")]` Display output
are upstream testing and should be removed. At least 22 such tests exist across plugin error
modules, all constructing an error variant and asserting `to_string()` matches the format
string. This tests `thiserror`'s formatting behavior, not application logic. Affected files:
  - `crates/plugins/releases/docker/src/error.rs:60-138` (10 tests)
  - `crates/plugins/releases/github/src/error.rs:46-68` (3 tests)
  - `crates/plugins/releases/gitlab/src/error.rs:46-64` (3 tests)
  - `crates/plugins/releases/forgejo/src/error.rs:46-64` (3 tests)
  - `crates/plugins/infrastructure/proxmox/src/error.rs:44-54` (2 tests)
  - Additional parse error Display tests in `web-api-types/src/notifications.rs:531-533,597-599,663-665`, `registration.rs:183`, `types/src/plugin_types.rs:545`, `plugin_role.rs:271`, `update_status.rs:134`

Note: Tests for custom `Display` implementations that delegate to hand-written `as_str()` match
arms (e.g., `PluginType::display()`, `AlertSeverity::display_all_variants()`) are internal logic
tests and are correctly included.

**[HIGH]** DB row backdating violates documented testing rules -- `docs/development/testing.md`
states: "Do not backdate database rows directly (fragile: column rename silently breaks the test
and does not exercise the production code path)." Multiple test modules use direct DB backdating:
  - `crates/ui/web-api-auth/src/auth/oidc_state.rs:628-639,669-679,702-712,838,916` (at least 7 instances) -- `Expr::value(OffsetDateTime::now_utc() - time::Duration::hours(1))` to backdate `expires_at`
  - `crates/ui/web-api-auth/src/auth/device_flow.rs:236-255` -- `expire_flow()` test helper backdates `expires_at`

These stores (`OidcFlowStore`, `AccountLinkStore`, `OidcTokenExchangeStore`, `DeviceFlowStore`)
use `OffsetDateTime::now_utc()` in production code without clock injection. The correct fix per
the documented `with_clock` pattern (already implemented in `RateLimitStore`) is to add
`with_clock` constructors and advance the injected clock in tests.

**[MEDIUM]** 7 query modules totaling 2,659 lines have zero test coverage --
`notifications.rs` (521 lines), `host_packages.rs` (793 lines), `services.rs` (412 lines),
`plugin_configs.rs` (356 lines), `enrollment_tokens.rs` (206 lines), `scheduled_tasks.rs` (170
lines), and `audit_logs.rs` (201 lines) in `crates/ui/web-api-queries/src/queries/` have no
`#[cfg(test)]` module. Some may be exercised indirectly by integration tests, but
`host_packages`, `audit_logs`, and `scheduled_tasks` have no corresponding integration test
files either.

**[MEDIUM]** `crates/ui/web-api-auth/src/settings_store.rs` (580 lines) has zero test coverage
despite containing 7 `OffsetDateTime::now_utc()` calls and handling encrypted setting storage.

---

## Cross-Cutting Consistency Issues

*(2026-03-06 parallel review: consistency agent)*

**[LOW]** Error variant naming inconsistency for DB-wrapping variants across query error types:
  - `ServiceQueryError::Db(sea_orm::DbErr)` at `services.rs:44`
  - `SoftwareItemQueryError::Db(sea_orm::DbErr)` at `software_items.rs:61`
  - `TriggerUpdateError::Database(sea_orm::DbErr)` at `update_triggers.rs:76`
  - `PluginConfigError::Db(sea_orm::DbErr)` in `plugin_configs.rs`

All should use one name (`Db` or `Database`) consistently. This is cosmetic but breaks
`grep`-based consistency checks.

**[LOW]** `Option` serialization inconsistency across response types --
`ServiceResponse` uses `#[serde(skip_serializing_if = "Option::is_none")]` on optional fields
(omitting keys when `None`), while `HostResponse` does NOT use `skip_serializing_if` on any
`Option` fields (serializing as `null`). Within `SoftwareItemResponse`, some `Option` fields
use `skip_serializing_if` and others do not. This means different response types have different
JSON contracts for `None` values, which could confuse API consumers.

**[LOW]** `hosts.rs` query module (`web-api-queries/src/queries/hosts.rs:101`) returns
`Result<..., sea_orm::DbErr>` directly, deviating from the pattern used by every other query
module which defines a domain-specific error enum wrapping `sea_orm::DbErr` in a
`rootcause::Report`. The lack of a typed error prevents future extension (e.g., adding a
`DuplicateHostname` variant) without a breaking change.

---

## Cross-Cutting Maintainability Issues

*(2026-03-06 parallel review: maintainability agent)*

**[MEDIUM]** No migration registration check -- There is no automated check that the migration
vector in `crates/shared/db/src/migration/mod.rs` matches the file list in the directory. A
future contributor could add a migration file but forget to register it. A compile-time or
test-time assertion that all `m20*_*.rs` files in the directory appear in the
`Migrator::migrations()` vector would prevent silent omission. Additionally, the migration vector
does not follow chronological file-name order (documented and intentional for FK dependency
reasons), but a comment at the top of the `migrations()` function would help prevent
well-intentioned "cleanup" by new contributors.

**[LOW]** `crates/shared/agent-core/src/version_check.rs` -- Detect/fetch retry logic duplicated.
`detect_installed` (lines 421-481) and `fetch_latest` (lines 488-545) share nearly identical
retry-with-backoff logic (~50 lines each). A shared generic retry helper would eliminate
approximately 100 lines of duplicated control flow. Similarly, batch group processing (detect
lines 157-233, fetch lines 281-376) follows the same structural pattern and could use a generic
`run_batch_group` function.

---

## Cross-Cutting Database Issues

*(2026-03-06 parallel review: database agent)*

**[MEDIUM]** `crates/ui/web-api-queries/src/queries/host_packages.rs:645-776` --
`promote_host_package` performs cross-function mutations without a wrapping transaction. It calls
`si_queries::create_software_item` (own transaction), then `si_queries::assign_hosts` (another
transaction), then `HostSoftwareItem::update_many` (line 739, no transaction). If the version
copy at step 5 fails after assign_hosts succeeds, the software item exists without version data.
These should be wrapped in a single encompassing transaction.

**[LOW]** Multiple internal helper functions accept raw `DatabaseConnection` instead of
`TenantDb` and rely on upstream callers to have already filtered by tenant. Defense-in-depth gaps:
  - `software_items.rs:237,255,269,284` -- `load_item_hosts` queries `Host::find()` without tenant filter
  - `software_items.rs:824` -- `assign_hosts` host existence check lacks tenant filter (user-provided `host_id`)
  - `host_packages.rs:435,485` -- `find_or_create_host_package` omits `tenant_id` filter on `HostPackage::find()`
These are mitigated by upstream tenant scoping but violate the defense-in-depth convention.

---

## Review — 2026-03-10

- **Reviewer**: AI code review (architecture|quality|maintainability)
- **Scope**: Workspace-level findings from architecture, maintainability, and code quality passes

### Architecture

#### Strengths

- **(A5, confirmed)** Dependency boundary discipline is clean. No circular dependencies.
  `shared/wire`, `service-sdk`, and `shared-types` have zero reverse dependencies on `web-api`
  or `controller`. Plugin crates depend on `infrastructure-core` and `shared-types` but not
  `web-api`. This was noted in the 2026-03-06 parallel review and is confirmed unchanged.

- **(A6, confirmed)** `BackgroundTasks` and startup signal handling correctly use a
  `shutdown_token` (`CancellationToken`) propagated to all background tasks. Confirmed by the
  architecture review pass.

#### Issues

**[HIGH]** (A1) `uptrakit-web-api` (`app_state.rs`) — `AppState` is a god object with 30+
publicly accessible fields. All route handlers have full access to CA key store, credential
sources, SMTP settings, and other sensitive subsystems regardless of whether the handler
requires them. This is a reinforcement of the existing `AppState.pub` concern documented in
the 2026-03-06 parallel review (`[LOW] AppStateBuilder has no compile-time safety for its 32
Option fields`). Recommendation: introduce domain-scoped sub-state accessor methods
(`AppState::auth()`, `AppState::pki()`, etc.) and mark sensitive fields `pub(crate)` so only
the controller startup path can construct them directly.

### Maintainability

#### Strengths

- **(M6, confirmed)** All 45 crates use `workspace = true` for every third-party dependency
  with zero inline version declarations in member `Cargo.toml` files. Workspace
  `[dependencies]` is the authoritative version source. Already noted in the dependency graph
  analysis section; confirmed by the 2026-03-10 maintainability pass.

- **(M7, confirmed)** `resolver = "3"` is correctly declared. All crates are on edition 2024.
  Consistent with the crate-structure analysis above.

- **(M8, confirmed)** The dependency graph is acyclic. No circular crate dependencies. Already
  established in the 2026-03-06 crate structure analysis.

- **(M9, confirmed)** Feature flag propagation chains are well-structured for
  `db-sqlite`/`db-postgres`, notification plugins (`webhook`, `telegram`, `email`), OIDC, and
  `zeroconf`. No orphaned or contradicted flags found.

#### Issues

**[HIGH]** (M1) Workspace `Cargo.toml` — Notification plugin sub-crates (`webhook`,
`telegram`, `email`, `registry`) reference `uptrakit-notification-plugin-core` via
`path = "../core"` instead of `{ workspace = true }`. This bypasses workspace version control:
if the core crate version is ever bumped in `[workspace.dependencies]`, these four crates will
silently reference a different version than the rest of the workspace. Recommendation: change
all four path references to `uptrakit-notification-plugin-core = { workspace = true }`.

**[MEDIUM]** (M2) Workspace `Cargo.toml:42-43` — `sea-orm` and `sea-orm-migration` are pinned
to `2.0.0-rc.35` (pre-release). This is reinforced by the existing `[LOW]` note in the
controller review (`SeaORM pinned at 2.0.0-rc.35`). Add an inline `# TODO: upgrade to stable
once SeaORM 2.0 GA ships` comment on both lines so the pin intent is visible in `cargo deny`
audits and does not appear as an oversight.

**[MEDIUM]** (M3) Workspace — Multiple duplicate dependency versions are present: `base64`
(0.21 + 0.22), `darling` (0.20 + 0.21), `der` (0.7 + 0.8), `getrandom` (0.2 + 0.3 + 0.4,
triple split). The triple-split `getrandom` is the most risky because it activates three
distinct `getrandom` initialization paths at link time. Recommendation: run
`cargo update --aggressive` periodically to collapse minor-version splits; add
`cargo deny [bans]` `skip-tree` entries with an expiry comment for the `getrandom` triple-split
so it is visible and dated.

**[LOW]** (M4) All 45 member crates — `edition = "2024"` is repeated verbatim in every member
`Cargo.toml` and is not inherited from `[workspace.package]`. Recommendation: add
`edition = "2024"` to `[workspace.package]` and replace each member's `edition = "2024"` with
`edition.workspace = true`. Pure mechanical change with no semantic effect; reduces the
surface where a future edition change must be applied.

**[LOW]** (M5) Workspace `Cargo.toml` — Binary/entry-point crates (`controller`, `scheduler`,
`agent`, `mqtt`, `agent-ssh`) are intentionally absent from `[workspace.dependencies]` (they
are never imported by other crates), but this convention is undocumented. Add a brief comment
in `[workspace.dependencies]` such as `# Binary crates are excluded — they are never depended
upon and are not listed here` so the omission reads as intentional rather than an oversight.

### Code Quality

#### Strengths

- **(Q3, confirmed)** Atomic database semantics applied consistently across `uptrakit-controller`
  and `crates/ui/web-api-queries`. `pki.rs` wraps all multi-step CA operations in explicit
  `db.begin()`/`txn.commit()` blocks. `merge_service` uses `lock_exclusive()` to prevent TOCTOU
  races. This pattern matches the `Multi-Statement DB Cleanup` coding standard documented in the
  project memory.
