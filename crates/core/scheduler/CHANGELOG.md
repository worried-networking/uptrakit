# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
