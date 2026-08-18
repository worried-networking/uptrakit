# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.6](https://github.com/worried-networking/uptrakit/compare/uptrakit-scheduler-v0.0.5...uptrakit-scheduler-v0.0.6) - 2026-08-18

### Other

- update Cargo.lock dependencies

## [0.0.5](https://github.com/worried-networking/uptrakit/compare/uptrakit-scheduler-v0.0.4...uptrakit-scheduler-v0.0.5) - 2026-08-13

### Added

- *(reencrypt)* lazy v3 encryption of plugin config stores with plaintext-residue counters
- *(db)* [**breaking**] encrypt plugin_configs.config at rest via compile-time AAD newtype
- *(scheduler-runtime)* heartbeat-backed claim leases via live-task set
- *(db)* host_software_items discovery provenance + hysteresis columns

### Fixed

- *(build-info)* align --version with binary crate via build_info! macro
- *(scheduler-runtime)* purge expired sessions in AuthCleanupExecutor
- *(scheduler-runtime)* ownership-scoped release_claim with lost-claim detection

### Other

- *(crypto)* correct the AAD registry entry count and clarify decode paths
- *(crypto)* assert ciphertext in the aliased-read acceptance test
- *(crypto)* real-key acceptance for encrypted plugin config columns
- *(db)* convert all bare begin() call sites to begin_immediate()
- *(deps)* update Rust dependencies, bump rand to 0.10 and syn to 3
- *(plugins)* [**breaking**] plugin type IDs adopt dot-separated kebab-case grammar
- *(scheduler-runtime)* drive heartbeat fatal-path and beat-failure through real extracted fns
- *(adr)* discovery-based uninstall reconciliation lifecycle

## [0.0.4](https://github.com/worried-networking/uptrakit/compare/uptrakit-scheduler-v0.0.3...uptrakit-scheduler-v0.0.4) - 2026-06-04

### Added

- *(scheduler)* wire global_provider_lookup to FetchReleasesExecutor via ReleaseFetchContext
- *(audit-v2)* thread correlation_id through batch dispatch / scheduler
- *(2fa)* settings_auth two_factor_required + MFA challenge cleanup
- *(scheduler-runtime)* declare engine modules in lib.rs
- *(scheduler-runtime)* copy engine source files (pre-integration)
- *(audit)* emit semantic mutation audit events

### Fixed

- *(infra)* address verify-pass findings in agent-skills-a-infra
- *(audit-v2)* add correlation_id: None to existing AuditEventPayload literals
- *(audit-v2)* quality gate fixes
- *(scheduler-runtime)* add new entity fields to auth_cleanup ActiveModel literals
- *(scheduler-runtime)* fix standards violations in engine-sourced files
- *(scheduler-runtime)* switch engine imports to crate-local paths
- *(release-plz)* pin explicit versions, drop workspace.package.version, silence debug logs
- *(clippy)* remediate new lint violations (panic, silent-failure, unsafe)
- *(ci)* resolve all backend-lint, frontend, semantic-boundary, markdown, and edition CI failures
- *(audit)* type runtime audit emitter actions

### Other

- update Cargo.toml dependencies
- release
- *(pki,ocsp,identity)* migrate x509-parser introspection to x509-cert
- *(scheduler-runtime)* rename StandaloneSchedulerHandler to SchedulerHandler
- *(scheduler-runtime)* absorb engine deps ahead of merge
- *(scheduler)* migrate StandaloneSchedulerHandler to dyn ServiceTransport
- release
- *(release-plz)* unblock PR creation via git_only baseline
- *(workspace)* rename uptrakit-internal-wire to uptrakit-wire
- *(release)* per-binary releases and publish hygiene
- isolate plugin boundaries in track a
- Gate scheduler runtime standalone surface
- extract scheduler runtime

## [0.0.3](https://github.com/worried-networking/uptrakit/compare/uptrakit-scheduler-v0.0.2...uptrakit-scheduler-v0.0.3) - 2026-05-18

### Added

- *(scheduler)* wire global_provider_lookup to FetchReleasesExecutor via ReleaseFetchContext
- *(audit-v2)* thread correlation_id through batch dispatch / scheduler
- *(2fa)* settings_auth two_factor_required + MFA challenge cleanup
- *(scheduler-runtime)* declare engine modules in lib.rs
- *(scheduler-runtime)* copy engine source files (pre-integration)

### Fixed

- *(cli)* update stale --tofu references to --tofu-insecure in agent/agent-ssh/scheduler tests
- *(scheduler,web-api-queries)* remove dep on deleted scheduler-engine crate
- *(infra)* address verify-pass findings in agent-skills-a-infra
- *(audit-v2)* add correlation_id: None to existing AuditEventPayload literals
- *(audit-v2)* quality gate fixes
- *(scheduler-runtime)* add new entity fields to auth_cleanup ActiveModel literals
- *(scheduler-runtime)* fix standards violations in engine-sourced files
- *(scheduler-runtime)* switch engine imports to crate-local paths

### Other

- *(scheduler-runtime)* rename StandaloneSchedulerHandler to SchedulerHandler
- *(pki,ocsp,identity)* migrate x509-parser introspection to x509-cert
- *(scheduler-runtime)* absorb engine deps ahead of merge
- *(scheduler)* migrate StandaloneSchedulerHandler to dyn ServiceTransport

## [0.0.2](https://github.com/worried-networking/uptrakit/compare/uptrakit-scheduler-v0.0.1...uptrakit-scheduler-v0.0.2) - 2026-05-05

### Added

- *(scheduler)* AwaitingRestartExecutor — timeout enforcement and verification dispatch
- *(scheduler)* TickExecutor trait, register_tick_executor, signal_host_progression
- *(scheduler)* expire email_change_requests in AuthCleanupExecutor
- *(audit)* emit semantic mutation audit events
- *(global-github-provider)* add global runtime and tiny client integration
- *(scheduler-engine)* split run() into drain + abort cancellation tokens
- *(scheduler-engine)* paginated software-states loader + TenantDb refactor
- *(update-hooks)* add resolve_effective_config for 3-layer config merge
- *(db)* add plugin_type_settings table and update host_software_item_plugins schema
- *(queries)* thread icon_url through all software item query paths
- *(infra-core)* add #[non_exhaustive] to public plugin structs with constructors
- *(wire)* add host metadata, enriched attributes, and connectivity payloads
- *(wire)* add friendly_name to MqttSoftwareStateHostEntry and MqttHostPackageHostState
- *(docker)* use image-level package_identifier with qualifier for per-container tracking
- *(logging)* add debug/info/trace logs to scheduler, version check, and extension dispatch
- *(scheduler)* add #[instrument] spans to task executors
- *(db,sdk)* add service_app_name to enrollment and DB entities
- *(scheduler)* add DiscoverHostPackagesExecutor for periodic host-package rediscovery
- *(scheduler)* add AuditLogCleanupExecutor for retention cleanup
- *(web-api)* query and push host package states to MQTT
- *(scheduler-engine,db)* add detect_version task and rename version_check to fetch_releases
- *(scheduler)* TTL-proportional service cert renewal window
- *(web-api,scheduler)* integrate host packages into version check flow
- *(wire)* add batch host package update messages and version check routing fields
- *(db)* add update_category column to host_software_items and update_history
- *(scheduler-engine)* add CrlRenewalExecutor and signal_crl_renewal
- *(mqtt)* add update_in_progress field to wire protocol
- *(mqtt)* load and propagate release metadata in software-states queries
- *(scheduler)* add per-task execution timeout and cancellation awareness
- create uptrakit-scheduler-engine shared crate

### Fixed

- *(release-plz)* pin explicit versions, drop workspace.package.version, silence debug logs
- *(clippy)* remediate new lint violations (panic, silent-failure, unsafe)
- *(self-update)* thread reuseport_configured, fix ignore reason, fix clippy
- *(scheduler)* parking_lot in SpyNotifier; dedup owner_map; add NULL-timeout test
- *(scheduler)* skip tick executors on drain/abort; explicit NoopSchedulerNotifier signal_host_progression
- *(db)* add awaiting_restart_timeout/since to all ActiveModel literal sites in tests
- *(ci)* resolve all backend-lint, frontend, semantic-boundary, markdown, and edition CI failures
- *(dispatch)* apply GitHub provider defaults across runtime paths
- *(service-hosts)* block deactivated host links from background flows
- add missing `is_embedded` fields to test fixtures and fix clippy
- *(scheduler)* group Phase A fetch_releases by (plugin_config_id, assignment_config)
- *(mqtt)* unify MQTT software states query path to fix incorrect version state
- *(scheduler/web-api)* always populate host_software_item_id in version check assignments
- *(scheduler)* fix fetch_releases skipping package managers with null plugin_config_id
- *(mqtt)* filter non-featured items from individual MQTT entities in scheduler-engine
- *(scheduler)* increase stale lease cleanup threshold to 5 minutes
- *(scheduler)* rename DiscoverHostPackages to DiscoverSoftware to match DB value
- *(errors)* remove #[from] on variants that have paired impl_report_conversion!
- *(cli)* rename NatsCommands::Get to NatsCommands::Show
- *(shared-types)* add Other(String) catch-all to BatchStatus and UpdateCategory
- *(scheduler-engine)* populate host_packages.latest_version via fetch_releases Phase B
- *(scheduler-engine)* detect double executor registration with debug_assert
- *(scheduler)* add rolling-upgrade safety to ScheduledTaskType
- *(scheduler)* release task claim before breaking on cancellation
- *(scheduler-engine)* remove real sleep from scheduler cancellation test

### Other

- update Cargo.lock dependencies
- *(release-plz)* unblock PR creation via git_only baseline
- *(workspace)* rename uptrakit-internal-wire to uptrakit-wire
- *(surfaces)* remove extension-era runtime leftovers
- Implement Track C semantic boundary gate
- isolate plugin boundaries in track a
- *(scheduler-engine)* cover controller-side mock fetch failures
- replace PluginType enum with PluginTypeId newtype
- migrate all call sites to catalog/descriptor model
- *(codereview)* update database and scheduler-engine review findings
- *(codereview)* refresh Rust backend review files
- *(web-api-types,cli,openapi-client,frontend)* remove MQTT settings API surface
- add embedded services architecture documentation
- *(scheduler-engine)* replace AtomicBool with closure for yield logic
- update documentation for MQTT decoupling refactor
- *(scheduler)* replace push_software_states with signal_software_states_changed
- consolidate load_software_states_for_tenant into scheduler-engine
- remove old hook system
- *(codereview)* extend backend review — 14-dimension analysis (2026-03-15)
- update registry and consumers to use PluginBase + subtrait accessors
- *(codereview)* add 2026-03-10 comprehensive review across all 12 dimensions
- *(codereview)* add 2026-03-10 comprehensive review across all 12 dimensions
- update plugins, agents, scheduler for unified tracking
- *(scheduler)* replace cron expressions with interval + jitter
- remove fixed issues from CODEREVIEW.md files
- *(scheduler-engine)* improve Duration consistency and doc comments
- strike through fixed CODEREVIEW entries from top-5 fix session
- merge 2026-03-06 parallel review findings into CODEREVIEW.md files
- add cargo-llvm-cov test coverage analysis to CODEREVIEW.md files
- fmt
- *(scheduler-engine)* make scheduler tenant-agnostic
- *(codereview)* remove fixed issues and update summaries
- *(scheduler)* categorize tasks as internal vs external
- eliminate raw SQL from tests and migrations
- fix top-5 code-review issues and repair reencrypt tests
- *(scheduler-engine)* use batch_fetch_releases in Phase A version check
- *(scheduler-engine)* parallelize fetch_releases phase A and run phases concurrently
- *(scheduler-engine)* extract shared agent-assignment query helpers
- *(codereview)* update backend code review findings
- migrate internal path deps to workspace = true
- *(codereview)* add tests, consistency, maintainability, and database dimensions to all per-crate reviews
- *(codereview)* remove fixed issues, update summaries
- *(scheduler-engine)* parallelize poll_cycle task execution with JoinSet
- *(scheduler-engine)* remove db from SchedulerNotifier::push_software_states_for_tenant
- remove fixed issues from CODEREVIEW.md files
- *(plugins)* make all plugin new() async, remove block_on() panic
- *(codereview)* comprehensive backend code review across 6 dimensions
- *(codereview)* add missing per-crate CODEREVIEW.md files
- apply cargo fmt to all changed files
- *(shared-db)* remove dead code, crypto shim, and convenience re-exports

## [0.0.1](https://github.com/worried-networking/uptrakit/releases/tag/uptrakit-scheduler-v0.0.1) - 2026-04-27

### Added

- *(scheduler)* graceful drain for external scheduler on ServerRestarting/SIGHUP
- *(web-api, scheduler)* paginated software-states push at MQTT registration
- propagate zeroconf feature to service binaries
- *(db,sdk)* add service_app_name to enrollment and DB entities
- *(scheduler)* add DiscoverHostPackagesExecutor for periodic host-package rediscovery
- *(scheduler)* init DEK ring after receiving master key
- *(scheduler-engine,db)* add detect_version task and rename version_check to fetch_releases
- *(scheduler)* register CrlRenewalExecutor and implement signal_crl_renewal via NATS
- *(scheduler)* add per-task execution timeout and cancellation awareness
- *(scheduler)* create external scheduler binary

### Fixed

- *(ci)* resolve all backend-lint, frontend, semantic-boundary, markdown, and edition CI failures
- escalate DEK/encryption failures from warn to error
- *(tracing)* enable info logging for all uptrakit crates by default
- *(wire)* restore generic Register message for capability negotiation on first connect
- *(wire)* remove obsolete Register sends from agent, agent-ssh, scheduler
- *(mqtt)* unify MQTT software states query path to fix incorrect version state
- *(crypto)* convert DataKeyRing::new panic to Result
- *(scheduler)* rename DiscoverHostPackages to DiscoverSoftware to match DB value
- migrate all call sites to ColumnAadEntry, fix managed_ca_rotation test
- *(scheduler)* add SystemService capability and stable service_id
- *(wire)* add Other(String) catch-all to ErrorCode/EnrollmentStatus, mark payload structs #[non_exhaustive]
- *(scheduler)* update missed TrackingNotifier and Unconstrained in ca_rotation.rs
- *(scheduler)* await JoinHandle in stop_scheduler to prevent race

### Other

- Gate scheduler runtime standalone surface
- extract scheduler runtime
- migrate all binaries to TracingBuilder
- *(service-sdk)* document SDK dispatch boundary and silence handler catch-all logs
- *(db)* remove MySQL/MariaDB feature flags from all crates
- *(codereview)* update core runtime binary review findings
- *(core)* remove shared MQTT-specific config ownership
- *(codereview)* refresh Rust backend review files
- add embedded services architecture documentation
- *(scheduler-engine)* replace AtomicBool with closure for yield logic
- *(wire)* unify Register message across all services; all services declare capabilities on connect
- update documentation for MQTT decoupling refactor
- *(scheduler)* replace push_software_states with signal_software_states_changed
- *(codereview)* extend backend review — 14-dimension analysis (2026-03-15)
- *(core)* tighten pub to pub(crate) in binary crates
- *(codereview)* add 2026-03-10 comprehensive review across all 12 dimensions
- *(codereview)* add 2026-03-10 comprehensive review across all 12 dimensions
- *(service-sdk)* remove tracing-subscriber, move init_tracing to binaries
- *(wire)* fix event loop starvation, unify Duration types, split wire lib.rs into modules
- merge 2026-03-06 parallel review findings into CODEREVIEW.md files
- add cargo-llvm-cov test coverage analysis to CODEREVIEW.md files
- add descriptions to binary crates and remove mqtt publish=false
- fmt
- fmt
- *(scheduler)* remove internal tasks from external scheduler
- *(codereview)* update backend code review findings
- migrate internal path deps to workspace = true
- *(codereview)* add tests, consistency, maintainability, and database dimensions to all per-crate reviews
- *(scheduler-engine)* remove db from SchedulerNotifier::push_software_states_for_tenant
- remove fixed issues from CODEREVIEW.md files
- *(codereview)* remove fixed issues from CODEREVIEW files
- *(codereview)* comprehensive backend code review across 6 dimensions
- *(codereview)* add missing per-crate CODEREVIEW.md files
- apply cargo fmt to all changed files
- *(shared-db)* remove dead code, crypto shim, and convenience re-exports
- *(service-sdk)* deduplicate resolve_shutdown and init_tracing

### Security

- *(crypto)* EncryptedString::new requires master key, no plaintext fallback
