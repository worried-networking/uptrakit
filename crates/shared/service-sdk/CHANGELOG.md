# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.1](https://github.com/worried-networking/uptrakit/releases/tag/uptrakit-service-sdk-v0.0.1) - 2026-04-21

### Added

- port service providers and cli to surface runtime
- add surface wire protocol and service proxy
- *(service-sdk)* add shutdown signal wrappers and exports
- *(service-sdk)* add poll-based SignalWatcher shutdown API
- *(service-sdk)* add TracingBuilder, init_cli_tracing, init_test_tracing
- *(wire,sdk)* add generic service config store messages and ServiceConfigProxy
- *(wire)* send report page limits in service settings
- *(service-sdk)* add mDNS zero-configuration discovery
- *(service-sdk)* add send_auto_paginate for transparent pagination
- *(service-sdk)* persist tenant_id in service.json
- *(service-sdk)* add generic `decrypt_sensitive_params<T>` for ECIES sealed-box decryption
- *(service-sdk)* add ServiceExtensionProxy for service-initiated extension invocations
- *(crypto)* add ECIES sealed-box encryption using P-256 + AES-256-GCM
- *(service-sdk)* add spans to event loop and connection methods
- *(wire)* add TraceContext type for distributed tracing
- *(service-sdk)* add extension request handling to ServiceHandler
- *(db,sdk)* add service_app_name to enrollment and DB entities
- *(service-sdk)* add ShutdownCause enum and update on_shutdown signature
- *(service-sdk)* scope -v verbosity to uptrakit crates only
- *(logging)* add verbosity flags and structured log instrumentation
- add mock feature to openapi-client and CLI integration tests
- *(agent-ssh)* add CLI host management subcommands
- *(cli)* add unified --version build metadata across binaries

### Fixed

- *(ci)* resolve all backend-lint, frontend, semantic-boundary, markdown, and edition CI failures
- *(services)* make surface registration best effort
- resolve remaining task 8 verification blockers
- tighten surface validation and proxy cleanup semantics
- complete transport close-policy overrides and tests
- *(service-sdk)* harden recv error classification pipeline
- *(tracing-init)* warn on invalid RUST_LOG directives; update docs
- *(integration)* align enrollment flow with current services
- stop event loop from blocking on recv when service events are pending
- *(service-sdk)* replace cfg(not(feature)) with unconditional fallback in resolve_connection
- *(service-sdk)* reset reconnect backoff after successful connection
- *(service-sdk)* share SignalWatcher across reconnect iterations
- *(service-sdk)* reconnect on transient network errors instead of crashing
- *(errors)* remove #[from] on variants that have paired impl_report_conversion!
- *(agent-ssh,service-sdk)* defer ExtensionRegister until after capability negotiation
- *(service-sdk,web-api)* add timeout guards to prevent shutdown hangs
- *(service-sdk)* prevent WebSocket write freeze and Ctrl+C unresponsiveness
- *(security)* use TofuVerifier for TOFU CA fetch instead of danger_accept_invalid_certs (ATK-04)
- *(wire)* add Other(String) catch-all to ErrorCode/EnrollmentStatus, mark payload structs #[non_exhaustive]
- *(service-sdk)* use latest in-memory CA cert on every reconnect
- *(service-sdk)* treat empty ca.pem as missing; validate non-empty CA on all write paths
- *(plugins)* add HTTP client timeouts to service-sdk, github, and phs plugins
- *(service-sdk)* retry enrollment on transient network errors
- *(service-sdk,controller)* use Constrained(0) in test CA helpers
- *(service-sdk,web-api)* enforce service_id filter on bearer-token WS lookup
- *(wire)* add Unknown catch-all variant for forward-compatible message deserialization
- remove unapproved .expect() uses and Report::new() violations
- *(wire)* add required protocol_version field to wire envelopes
- *(service-sdk)* replace tick().await with interval_at in ping timer setup
- *(service-sdk)* store enrollment secret as SecretString
- frontend accessibility, security, and UX improvements with expanded tests
- resolve remaining codereview issues with ping interval, retry, and auto-refresh
- resolve top 5 codereview issues across codebase
- resolve top 5 code review findings across directories, openapi-client, CLI, web-api-types, and shared-db
- resolve top 5 code review findings across 8 crates
- resolve top 5 code review findings across 6 crates
- resolve top 5 code review findings across 8 crates
- *(security)* resolve SEC-01, DIR-01, DB-01 from code review

### Other

- *(surfaces)* remove extension-era runtime leftovers
- hide dead extension payload exports
- rename surface runtime capability internals
- remove legacy extension wire messages
- drop service sdk extension shim
- isolate plugin boundaries in track a
- *(platform)* add standalone host adapter seam
- *(service-sdk)* add futures_util::poll recv regression
- Add MockTransport regressions and downstream compile gate
- *(service-sdk)* scaffold mock transport test support
- *(service-sdk)* enforce budget first-exhaustion boundary in event loop
- *(service-sdk)* cover budget reset re-enable path
- *(service-sdk)* cover first budget exhaustion in event loop
- *(service-sdk)* type-erase connection read stream for tests
- enforce default_resolve_shutdown shutdown contract
- *(service-sdk)* make on_shutdown contract normative
- *(service-sdk)* verify close_policy delegates to close_reason_to_policy
- *(service-sdk)* assert reconnect policy for all close reasons
- *(service-sdk)* assert direct cert-expired IO classification
- *(service-sdk)* add recv/transient classification regressions
- *(service-sdk)* remove private rustdoc links in recv docs
- *(service-sdk)* document transport and error-layer contract
- *(service-sdk)* complete transport error contract matrix
- *(service-sdk)* cover phase-1 transport error contract
- *(service-sdk)* extract recv frame classification helper
- extract TracingBuilder into uptrakit-tracing-init crate
- *(service-sdk)* document SDK dispatch boundary and silence handler catch-all logs
- *(codereview)* update shared library review findings
- *(codereview)* refresh Rust backend review files
- *(wire,agent-core)* introduce ServiceTransport trait for transport abstraction
- *(wire,sdk)* remove UpdateCapabilities in favour of Register
- extract helpers to reduce cyclomatic complexity (fixes 2,4,5)
- *(service-sdk)* extract shared types to break lifecycle/event_loop cycle
- *(types)* accept Into<String> in SecretString and MaskedEmail constructors
- *(codereview)* extend backend review — 14-dimension analysis (2026-03-15)
- *(service-sdk)* extract controller message dispatch from run_event_loop
- *(deps)* update Rust dependencies
- *(shared)* tighten pub to pub(crate) in internal shared crates
- *(service-sdk)* non-blocking write path via split WS stream
- *(codereview)* mark fixed issues as resolved in CODEREVIEW files
- *(codereview)* add 2026-03-10 comprehensive review across all 12 dimensions
- *(codereview)* add 2026-03-10 comprehensive review across all 12 dimensions
- *(service-sdk)* remove tracing-subscriber, move init_tracing to binaries
- update plugins, agents, scheduler for unified tracking
- update CODEREVIEW and documentation for wire refactor
- *(wire)* fix event loop starvation, unify Duration types, split wire lib.rs into modules
- strike through fixed CODEREVIEW entries from top-5 fix session
- merge 2026-03-06 parallel review findings into CODEREVIEW.md files
- *(tracing)* use registry-based subscriber in all binaries
- add cargo-llvm-cov test coverage analysis to CODEREVIEW.md files
- fmt
- *(codereview)* remove fixed issues and add constant-time comparison guidance
- *(codereview)* update backend code review findings
- migrate internal path deps to workspace = true
- *(codereview)* add tests, consistency, maintainability, and database dimensions to all per-crate reviews
- *(codereview)* remove fixed issues, update summaries
- *(service-sdk)* extract Backoff into new uptrakit-backoff crate
- *(codereview)* remove fixed issues from all CODEREVIEW.md files
- document non_exhaustive, HTTP timeouts, and parking_lot; clarify start_paused rule
- *(codereview)* comprehensive backend code review across 6 dimensions
- apply cargo fmt to all changed files
- *(service-sdk)* deduplicate resolve_shutdown and init_tracing
- *(cargo)* add workspace lints and consolidate inline dependencies
- apply cargo fmt across workspace
- update all documentation for capability-based service identity
- *(wire)* replace service_type with capabilities in EnrollPayload
- update documentation for plugin architecture
- *(service-sdk)* remove init_tracing; move subscriber init to binaries
- remove all fixed findings from CODEREVIEW.md files
- apply cargo fmt to entire workspace
- add comprehensive code review findings
- add comprehensive code review findings
- *(wire)* replace protocol_version with capability-based negotiation
- remove obsolete TESTCOV files
- *(codereview)* resolve all remaining open code review findings
- *(codereview)* resolve top 5 open code review findings (round 3)
- replace pause() calls with start_paused = true in all time-dependent tests
- apply cargo fmt formatting across workspace
- convert LoopError from struct to thiserror enum with Report<T>
- simplify ServiceHandler trait with #[async_trait]
- consolidate service event loops into SDK-managed callbacks
- refresh TESTCOV.md with cargo-llvm-cov coverage data after rebase
- add unit tests for next 10 uncovered critical paths
- add unit tests for top 10 uncovered critical paths
- add test coverage analysis (TESTCOV.md) for 16 crates
- cargo fmt + fix clippy await_holding_lock in shared-db test
- add CODEREVIEW.md for wire, service-sdk, agent, mqtt, agent-ssh crates
- add extensibility-focused code review findings
- *(deps)* replace workspace-wide tokio full with per-crate minimal features
- remove CODEREVIEW.md files
- *(types)* use Uuid instead of String for all entity ID fields
- fix top 5 code review issues (SDK-04, CMD-04, PCORE-02, DB-02, controller decomposition)
- *(wire)* replace close reason string constants with CloseReason enum
- cargo fmt
- mark XC-06, PREG-05, WAT-02, WAT-08, WAT-09, DB-05, SDK-03 as FIXED
- *(service-sdk)* restructure EnrollmentError into domain sub-enums (SDK-03)
- *(service-sdk)* extract certificate message handlers into CertificateRenewalHandler
- resolve top 5 code review issues across workspace
- *(wire)* decouple host info from enrollment, rename ReportHostInfo to ReportHosts
- *(service-sdk)* add ServiceHandler trait and lifecycle module
- add extensibility-focused code review for all crates
- add CODEREVIEW.md for services, wire, and service-sdk crates
- replace Result<T, String> in crypto.rs and migrate bail!() codebase-wide
- *(rust)* apply workspace formatting
- unify agent and MQTT service startup flows via shared SDK
