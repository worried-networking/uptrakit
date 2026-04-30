# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.2](https://github.com/worried-networking/uptrakit/compare/uptrakit-mqtt-v0.0.1...uptrakit-mqtt-v0.0.2) - 2026-04-30

### Other

- updated the following local packages: uptrakit-service-sdk, uptrakit-wire, uptrakit-mqtt-runtime

## [0.0.1](https://github.com/worried-networking/uptrakit/releases/tag/uptrakit-mqtt-v0.0.1) - 2026-04-27

### Added

- port service providers and cli to surface runtime
- unify mqtt runtime for standalone and embedded hosting
- *(mqtt)* implement workload claim protocol
- *(mqtt)* accumulate multi-page SoftwareStates in TenantManager
- *(mqtt)* add unsubscribe_topic method to MqttHandle
- *(mqtt)* add entity_picture and dual availability to HA discovery
- propagate zeroconf feature to service binaries
- *(mqtt)* enrich HA device block on all entities with OS info
- *(mqtt)* add title field and set name=null for HA host-level update entities
- *(wire)* add host metadata, enriched attributes, and connectivity payloads
- *(mqtt)* nest software items under hosts, use friendly_name for HA
- *(web-api,mqtt)* add spans to notification dispatch and MQTT handlers
- *(db,sdk)* add service_app_name to enrollment and DB entities
- *(wire)* add SystemService capability
- *(mqtt)* add per-host security updates entity and disable host entities by default
- *(mqtt)* publish host package states and handle HA commands
- *(mqtt)* add host package HA discovery topic helpers
- *(mqtt)* publish JSON attributes topic with in_progress flag
- *(mqtt)* include release_url and release_summary in HA discovery config
- *(mqtt)* use app name as HA device, stable entity IDs via default_entity_id
- *(agent,agent-ssh,mqtt)* handle ServerRestarting with graceful disconnect
- *(mqtt)* implement Home Assistant MQTT discovery and update command handling
- *(wire)* add SoftwareStates, MqttTriggerUpdate; extend MqttTenantConfig with HA settings
- *(logging)* add verbosity flags and structured log instrumentation
- add mock feature to openapi-client and CLI integration tests
- *(agent-ssh)* add CLI host management subcommands
- *(cli)* add unified --version build metadata across binaries
- add mqtt client connection status
- *(db,enrollment,cli,controller)* encrypt credentials at rest and harden TOFU
- *(wire)* [**breaking**] add application-level replay protection with message sequence numbers
- [**breaking**] support multiple MQTT clients per tenant
- *(mqtt)* add uptrakit-mqtt binary crate

### Fixed

- *(ci)* resolve all backend-lint, frontend, semantic-boundary, markdown, and edition CI failures
- *(tracing)* enable info logging for all uptrakit crates by default
- *(wire)* restore generic Register message for capability negotiation on first connect
- *(mqtt)* clean up stale HA discovery topics on item/host removal
- *(mqtt)* include entity domain in default_entity_id for HA discovery
- *(mqtt)* use default_entity_id instead of object_id in HA MQTT discovery
- *(mqtt)* correct HA discovery device name and entity IDs
- *(mqtt)* change max_tenants CLI arg to Option<NonZeroU32>
- *(mqtt)* fix HA device naming and enrich device info block
- *(mqtt)* improve host package state/latest_version for Home Assistant
- address safety and correctness issues across crates
- *(mqtt)* correct entity_id prefix and resolve post-rebase test conflicts
- *(mqtt)* eliminate event-loop self-deadlock on full request channel
- *(mqtt)* decouple state publishing from HA discovery
- *(mqtt)* replace unbounded event channel with bounded channel (capacity 512)
- *(mqtt)* replace fixed reconnect delay with exponential backoff
- frontend accessibility, security, and UX improvements with expanded tests
- use virtual time in MQTT shutdown timeout test
- resolve remaining codereview issues with ping interval, retry, and auto-refresh
- resolve top 5 codereview issues across codebase
- resolve top 5 code review findings across mqtt, agent-ssh, web-api-types, shared-types, and command
- resolve top 5 code review findings across 8 crates
- *(wire)* add protocol version fields
- *(web-api,wire,types)* implement code review fix plans #9-#16
- *(mqtt)* redact credentials in debug logs
- *(mqtt)* add bounded shutdown and cancel loops

### Other

- rename surface runtime capability internals
- Remove dead MQTT source duplicates
- migrate all binaries to TracingBuilder
- *(service-sdk)* document SDK dispatch boundary and silence handler catch-all logs
- *(codereview)* update core runtime binary review findings
- *(codereview)* full 14-dimension workspace review cycle
- *(mqtt)* own client config through extension settings
- *(codereview)* refresh Rust backend review files
- *(web-api,wire)* rename MQTT-specific identifiers to generic names
- *(wire,sdk)* remove UpdateCapabilities in favour of Register
- *(mqtt)* replace legacy MQTT wire protocol with Service Config Store + extension UI
- *(wire)* unify Register message across all services; all services declare capabilities on connect
- *(wire)* rename Mqtt-prefixed state types to generic names
- *(wire)* rename MqttBridge capability to UpdateTracking
- split mqtt/tenant_manager.rs into client_manager and state_publisher modules
- extract constructors and shared helpers to reduce complexity
- *(types)* accept Into<String> in SecretString and MaskedEmail constructors
- *(codereview)* extend backend review — 14-dimension analysis (2026-03-15)
- *(mqtt)* extract ha_discovery into topic/device/attribute/parser modules
- *(core)* tighten pub to pub(crate) in binary crates
- Merge branch 'worktree-fix/mqtt-deadlock'
- *(codereview)* add 2026-03-10 comprehensive review across all 12 dimensions
- *(codereview)* add 2026-03-10 comprehensive review across all 12 dimensions
- *(service-sdk)* remove tracing-subscriber, move init_tracing to binaries
- *(mqtt)* add comment explaining intentional alpn/client_auth None
- update plugins, agents, scheduler for unified tracking
- *(wire)* fix event loop starvation, unify Duration types, split wire lib.rs into modules
- remove fixed issues from CODEREVIEW.md files
- merge 2026-03-06 parallel review findings into CODEREVIEW.md files
- add descriptions to binary crates and remove mqtt publish=false
- fmt
- *(mqtt)* drop redundant uptrakit_ prefix from HA discovery topic object_id
- *(codereview)* update backend code review findings
- migrate internal path deps to workspace = true
- *(codereview)* add tests, consistency, maintainability, and database dimensions to all per-crate reviews
- *(mqtt)* introduce ReleaseInfo struct to fix too-many-arguments lint
- add `uptrakit_` prefix to the default entity id
- document non_exhaustive, HTTP timeouts, and parking_lot; clarify start_paused rule
- *(types,wire,web-api-types)* add #[non_exhaustive] to public enums and update match arms
- *(codereview)* remove fixed issues from CODEREVIEW files
- *(codereview)* comprehensive backend code review across 6 dimensions
- *(codereview)* remove fixed issues from CODEREVIEW.md files
- apply cargo fmt to all changed files
- *(service-sdk)* deduplicate resolve_shutdown and init_tracing
- *(cargo)* add workspace lints and consolidate inline dependencies
- apply cargo fmt across workspace
- update all references from old service_ws/service_handler files
- update references from deleted handler files to service_handler
- *(services)* remove SERVICE_TYPE constant from all service binaries
- update documentation for plugin architecture
- *(service-sdk)* remove init_tracing; move subscriber init to binaries
- add comprehensive code review findings
- add comprehensive code review findings
- *(wire)* replace protocol_version with capability-based negotiation
- remove obsolete TESTCOV files
- *(codereview)* resolve all remaining open code review findings
- apply cargo fmt formatting across workspace
- convert LoopError from struct to thiserror enum with Report<T>
- simplify ServiceHandler trait with #[async_trait]
- consolidate service event loops into SDK-managed callbacks
- refresh TESTCOV.md with cargo-llvm-cov coverage data after rebase
- add unit tests for next 10 uncovered critical paths
- add test coverage analysis (TESTCOV.md) for 16 crates
- cargo fmt + fix clippy await_holding_lock in shared-db test
- add CODEREVIEW.md for wire, service-sdk, agent, mqtt, agent-ssh crates
- add extensibility-focused code review findings
- *(deps)* minimise unnecessary dependencies and feature-gate OIDC
- *(deps)* replace workspace-wide tokio full with per-crate minimal features
- cargo fmt
- remove CODEREVIEW.md files
- *(mqtt)* remove unnecessary web-api-types dependency
- *(types)* use Uuid instead of String for all entity ID fields
- *(wire)* replace close reason string constants with CloseReason enum
- cargo fmt
- fix top 5 code review issues (ARCH-04, ARCH-02, WAT-03, SEC-03, MQTT-04)
- *(service-sdk)* extract certificate message handlers into CertificateRenewalHandler
- resolve top 5 code review issues across workspace
- *(wire)* decouple host info from enrollment, rename ReportHostInfo to ReportHosts
- *(service-sdk)* add ServiceHandler trait and lifecycle module
- add extensibility-focused code review for all crates
- add CODEREVIEW.md for services, wire, and service-sdk crates
- *(rust)* apply workspace formatting
- unify agent and MQTT service startup flows via shared SDK
- clean up workspace dependencies
- *(wire)* rename agent_ts to service_ts in Ping/Pong payloads
- *(wire)* replace MqttTenantConfig.transport String with typed MqttTransport enum
- *(wire)* replace untyped String fields with typed enums
- *(wire)* change ID fields from String to uuid::Uuid
- add impl_report_conversion! macro and replace verbose ReportConversion impls
- enforce rootcause best practices and migrate errors to thiserror
- *(enrollment)* introduce EnrollmentParams to reduce function arguments
- [**breaking**] replace MQTT Heartbeat message with Ping/Pong
- [**breaking**] unify enrollment, TLS, CA, and CLI into shared enrollment crate
- [**breaking**] enforce rootcause Report<E> error handling across all crates
- *(directories)* replace AppKind enum with plain &str parameter
- [**breaking**] use cross-platform directories with config/state separation
- [**breaking**] unify agents and MQTT services into single Service entity
- *(mqtt)* drop websocket broker transport
- *(mqtt)* replace direct DB access with WebSocket/mTLS controller communication
- update documentation for MQTT service decoupling
- remove old uptrakit-mqtt library crate
