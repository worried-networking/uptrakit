# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.1](https://github.com/worried-networking/uptrakit/releases/tag/uptrakit-agent-v0.0.1) - 2026-04-27

### Added

- *(audit)* emit semantic mutation audit events
- add agent-side host feature probing and expose in REST API
- *(agent)* handle config test requests on local and SSH agents
- *(agent-ssh)* advertise InteractiveUpdates and handle UpdateStdinData
- *(agent)* advertise InteractiveUpdates and handle UpdateStdinData
- make `interactive` a default feature in binary crates
- propagate zeroconf feature to service binaries
- *(service-sdk)* add send_auto_paginate for transparent pagination
- *(wire)* add agent_host_id to HostInfo; controller uses it as hosts.id
- *(db,sdk)* add service_app_name to enrollment and DB entities
- *(security)* add remote freeze via SetUpdateFreeze wire message (ATK-17 Phase 3)
- *(security)* add agent-side update rate limiting (ATK-17 Phase 2)
- *(agent)* add operator freeze file to halt update execution
- *(agent-core)* handle batch host package updates
- *(agent-ssh)* wrap executors with SudoAwareCommandExecutor and add CLI options
- *(agent-core)* add ConnectionContext for SSH Docker host injection
- *(agent,agent-ssh,mqtt)* handle ServerRestarting with graceful disconnect
- *(logging)* add verbosity flags and structured log instrumentation
- *(autodiscovery)* implement software autodiscovery feature
- *(agent-ssh)* implement version check and update execution over SSH
- *(agent-ssh)* report enrolled SSH hosts to controller on connect
- *(cli)* add unified --version build metadata across binaries
- add Homebrew provider with agent-side latest version checking
- Consolidate LocalProvider/RemoteProvider for all providers
- *(db,enrollment,cli,controller)* encrypt credentials at rest and harden TOFU
- *(wire)* [**breaking**] add application-level replay protection with message sequence numbers
- [**breaking**] support multiple MQTT clients per tenant
- implement graceful shutdown for agent
- *(controller)* implement zero-downtime graceful restart via SO_REUSEPORT
- implement structured update hooks with fail-early shell execution
- implement update communication flow between agent and controller
- add agent version tracking and version check wire protocol
- [**breaking**] add OCSP SHA-1 support, --pki-addr flag, and Nginx OCSP integration test
- [**breaking**] implement CSR-based agent certificate issuance
- add OCSP responder, backend URL setting, and CA certificate extensions
- add Host entity with machine_id-based identity
- *(controller,agent,web)* implement CA key rotation with dual-CA support

### Fixed

- *(ci)* resolve all backend-lint, frontend, semantic-boundary, markdown, and edition CI failures
- *(agent)* add missing uptrakit-audit-log dep to Cargo.toml
- *(tracing)* enable info logging for all uptrakit crates by default
- *(wire)* restore generic Register message for capability negotiation on first connect
- *(wire)* remove obsolete Register sends from agent, agent-ssh, scheduler
- *(agent)* delay initial host reports until settings arrive
- *(agent)* run long operations in background to prevent WS write timeouts
- *(clippy)* resolve pre-existing lint warnings in agent and dispatcher
- *(agent)* emit warning and randomise unknown machine-ID fallback
- *(agent-core)* send UpdateResult via send() to prevent silent drop
- frontend accessibility, security, and UX improvements with expanded tests
- resolve remaining codereview issues with ping interval, retry, and auto-refresh
- resolve top 5 codereview issues across codebase
- resolve top 5 code review findings across 8 crates
- eliminate .expect() calls and improve error chain preservation
- resolve top 5 code review findings across 6 crates
- resolve top 5 code review findings across 8 crates
- resolve top 5 code review findings across 8 crates
- *(controller)* resolve rebase and fix reverse-proxy test AppState init
- *(security)* resolve SEC-01, DIR-01, DB-01 from code review
- *(wire)* add protocol version fields
- *(agent,enrollment,wire)* implement code review fix plans #7-#13
- *(agent)* implement code review fix plans #1-#5
- *(web-api,wire,agent)* eliminate command injection in update hooks
- *(agent)* handle ServerRestarting message during enrollment
- *(agent)* add x509-parser dep for standalone compilation
- fix cert renewal

### Other

- Extract shared agent runtime
- migrate all binaries to TracingBuilder
- *(service-sdk)* document SDK dispatch boundary and silence handler catch-all logs
- *(codereview)* update core runtime binary review findings
- *(codereview)* refresh Rust backend review files
- *(agent-core)* move host_info to shared agent-core crate
- *(wire)* unify Register message across all services; all services declare capabilities on connect
- *(codereview)* extend backend review — 14-dimension analysis (2026-03-15)
- *(core)* tighten pub to pub(crate) in binary crates
- *(codereview)* add 2026-03-10 comprehensive review across all 12 dimensions
- *(codereview)* add 2026-03-10 comprehensive review across all 12 dimensions
- *(service-sdk)* remove tracing-subscriber, move init_tracing to binaries
- update plugins, agents, scheduler for unified tracking
- *(wire)* fix event loop starvation, unify Duration types, split wire lib.rs into modules
- *(agent)* create command executor once, reuse across handlers
- merge 2026-03-06 parallel review findings into CODEREVIEW.md files
- update security architecture, AGENTS and CODEREVIEW files
- add descriptions to binary crates and remove mqtt publish=false
- fmt
- *(codereview)* update backend code review findings
- migrate internal path deps to workspace = true
- *(codereview)* add tests, consistency, maintainability, and database dimensions to all per-crate reviews
- *(codereview)* comprehensive backend code review across 6 dimensions
- apply cargo fmt to all changed files
- *(service-sdk)* deduplicate resolve_shutdown and init_tracing
- *(cargo)* add workspace lints and consolidate inline dependencies
- apply cargo fmt across workspace
- *(deps)* promote plugin-core, plugin-registry, and command to workspace deps
- update all documentation for capability-based service identity
- *(services)* remove SERVICE_TYPE constant from all service binaries
- update documentation for plugin architecture
- *(service-sdk)* remove init_tracing; move subscriber init to binaries
- remove all fixed findings from CODEREVIEW.md files
- apply cargo fmt to entire workspace
- add comprehensive code review findings
- add comprehensive code review findings
- *(wire)* replace protocol_version with capability-based negotiation
- remove obsolete TESTCOV files
- *(codereview)* resolve all remaining open code review findings
- remove upstream crate tests, add AsyncAPI spec conformance
- apply cargo fmt formatting across workspace
- convert LoopError from struct to thiserror enum with Report<T>
- simplify ServiceHandler trait with #[async_trait]
- consolidate service event loops into SDK-managed callbacks
- refresh TESTCOV.md with cargo-llvm-cov coverage data after rebase
- add unit tests for top 10 uncovered critical paths
- add test coverage analysis (TESTCOV.md) for 16 crates
- consolidate UpdateOutputStream into OutputStreamType and remove ShellType alias
- add CODEREVIEW.md for wire, service-sdk, agent, mqtt, agent-ssh crates
- add extensibility-focused code review findings
- *(deps)* replace workspace-wide tokio full with per-crate minimal features
- remove CODEREVIEW.md files
- *(types)* use Uuid instead of String for all entity ID fields
- inject CommandExecutor into providers for transport-agnostic command dispatch
- *(wire)* replace close reason string constants with CloseReason enum
- cargo fmt
- *(service-sdk)* restructure EnrollmentError into domain sub-enums (SDK-03)
- *(service-sdk)* extract certificate message handlers into CertificateRenewalHandler
- resolve top 5 code review issues across workspace
- *(wire)* decouple host info from enrollment, rename ReportHostInfo to ReportHosts
- *(service-sdk)* add ServiceHandler trait and lifecycle module
- add extensibility-focused code review for all crates
- add CODEREVIEW.md for services, wire, and service-sdk crates
- *(rust)* apply workspace formatting
- unify agent and MQTT service startup flows via shared SDK
- simplify provider interface
- *(provider-registry)* merge create_local/remote_provider into create_provider
- drop hex, urlencoding and open crates
- clean up workspace dependencies
- *(agent,provider-core)* extract command execution into uptrakit-command crate
- *(CODEREVIEW)* remove obsolete codereview files
- *(agent,provider-core)* unify update dispatch through provider registry
- *(agent)* add comprehensive code review with 13 findings and fix plans
- *(wire)* rename agent_ts to service_ts in Ping/Pong payloads
- *(wire)* replace untyped String fields with typed enums
- *(wire)* change ID fields from String to uuid::Uuid
- add impl_report_conversion! macro and replace verbose ReportConversion impls
- enforce rootcause best practices and migrate errors to thiserror
- *(enrollment)* introduce EnrollmentParams to reduce function arguments
- *(agent)* introduce AuthenticatedLoopParams to reduce function arguments
- [**breaking**] unify enrollment, TLS, CA, and CLI into shared enrollment crate
- [**breaking**] enforce rootcause Report<E> error handling across all crates
- update all dependencies to latest versions
- *(directories)* replace AppKind enum with plain &str parameter
- [**breaking**] use cross-platform directories with config/state separation
- [**breaking**] unify agents and MQTT services into single Service entity
- move UUIDv7 generation from agent to controller
- remove unnecessary first CSR generation from enrollment
- extract provider matching into registry crate
- *(agent)* apply cargo fmt formatting
- *(agent)* rework CA bootstrap for --pki-addr without --tofu
- remove stupid remark from agent/--pki-addr
- *(reverse-proxy)* add deployment guides and fix clippy warnings
- *(agent)* rename --trust-first-use to --tofu
- clean up workspace dependency placement
- [**breaking**] remove HTTP server, rework agent bootstrap, add CRL endpoint
- *(agent)* replace unused assignment allow with break-value loop
- *(agent)* replace unwrap with match for tracing directive parse
- Merge branch 'feature/cli-basic'
- oidc and settings
- CRL
- agents cert renewal
- cert revokation
- agent WS enrollment
- add mTLS auth
- agent registration
- initial commit
