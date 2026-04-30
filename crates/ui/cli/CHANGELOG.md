# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.2](https://github.com/worried-networking/uptrakit/compare/uptrakit-cli-v0.0.1...uptrakit-cli-v0.0.2) - 2026-04-30

### Added

- *(permissions)* add AccessMcp variant and DB migration
- *(permissions)* wire-safe Other(String) catch-all for unknown variants

## [0.0.1](https://github.com/worried-networking/uptrakit/releases/tag/uptrakit-cli-v0.0.1) - 2026-04-27

### Added

- add actor names to update history responses
- *(audit)* add semantic audit log infrastructure
- *(settings)* add shared GitHub provider settings foundation
- *(update-history)* add shared protection and recovery fields
- port service providers and cli to surface runtime
- *(auth)* enforce OIDC private issuer policy by tenancy mode
- add agent-side host feature probing and expose in REST API
- *(cli)* add plugin-configs test subcommand and OpenAPI client method
- *(cli)* display embedded service state in human output
- *(api)* expose embedded service state and block removal
- *(api)* add ordinal field to plugin assignments and delete endpoint
- *(api)* add plugin_type filter to software item list
- *(cli)* add ssh_private_key field type and wizard action support
- *(display-version)* surface human-readable version labels from plugins
- *(smtp)* add helo_host setting and fix EHLO hostname for RFC 5321
- *(api)* add active_update_history_id to SoftwareItemHostSummary
- *(db)* add output_truncated flag and MySQL LONGTEXT migration for update_history
- *(web-api-types)* add `updatable` filter param to ListSoftwareItemsParams
- *(reset-data)* add backend for tenant data reset endpoint
- *(cli)* add plugin-type-settings subcommand
- *(cli)* add --icon-url and --clear-icon-url flags to software items
- *(api)* add host_id filter to list software items endpoint
- *(update-history)* persist interactive flag and broadcast UpdateStarted event
- *(cli)* add user, role, and access preset CLI commands
- *(cli)* add --interactive flag to update trigger command
- *(api)* add interactive flag to update trigger request
- add host tags query layer, REST API, OpenAPI client, and CLI
- *(cli)* add batch subcommands for all resource types
- *(api)* add POST /services/{id}/update-freeze endpoint (ATK-17)
- *(cli)* add dynamic extension subcommands with manifest-driven args
- *(wire,web-api)* add E2E encryption support for sensitive extension params
- instrument new code from main (extensions, SSE, event system, agent-core)
- *(openapi-client, cli)* SSE-first device auth login with poll fallback
- *(cli)* add extensions list, providers, and invoke commands
- *(cli)* add host-packages promote subcommand
- *(cli)* add system-enrollment-tokens subcommand
- *(openapi-client, cli)* add audit log client methods and audit-logs CLI subcommand
- *(api)* add /api/v1/system-services REST endpoints, openapi-client, and CLI
- *(cli)* expose cert_lifetime_hours in services and settings commands
- *(scheduler-engine,db)* add detect_version task and rename version_check to fetch_releases
- *(ui,docs)* TTL-proportional renewal window — CLI, frontend, docs
- *(cli)* add host-packages command group
- *(web-api-types)* add host package request/response types
- *(cli)* add batch update commands and SSE progress streaming client
- *(api)* surface update_category in API responses and CLI output
- *(cli)* add settings nats subcommand
- *(openapi-client,cli)* SMTP settings client methods and CLI commands
- *(cli)* add notifications command group
- *(cli)* add discovery-allowlist command group
- *(cli)* add --follow flag and history tail subcommand
- *(cli)* add enrollment-tokens command group
- *(api)* add enrollment token CRUD endpoints and remove old single-token API
- *(cli)* display controller_checks_run in check command human output
- *(registry)* register Shell plugin; add GithubReleases identifier validation
- *(api)* rename provider-configs to plugin-configs
- *(autodiscovery)* auto-create Docker provider config for container discoveries
- *(settings)* propagate ha_discovery/ha_discovery_prefix through MQTT client API and CLI
- *(db)* replace initiated_by with actor_type/actor_id; add HA discovery columns
- *(cli)* scope -v verbosity to uptrakit_cli / uptrakit crates only
- *(cli)* show version status in software output; add software update-latest
- CLI ↔ SPA feature parity — new commands, routes, and docs
- *(autodiscovery)* add frontend UI, discovery_state filter, and doc fixes
- *(logging)* add verbosity flags and structured log instrumentation
- *(autodiscovery)* implement software autodiscovery feature
- add mock feature to openapi-client and CLI integration tests
- *(cli)* add settings management CLI commands
- *(cli,web-api)* add service management CLI commands and GET endpoint
- *(openapi-client)* add typed HTTP client crate and migrate CLI
- *(cli,web-api)* add CLI commands and per-item version check endpoints
- *(cli)* add unified --version build metadata across binaries
- *(db,enrollment,cli,controller)* encrypt credentials at rest and harden TOFU
- *(cli)* add --output/-o flag for JSON, YAML, and human output formats
- *(cli)* replace password login with device authorization
- *(cli)* add basic CLI with auth and raw API support

### Fixed

- migrate SurfaceDescriptor struct literals to builder across all external crates
- make all pre-push quality gates pass
- *(ci)* resolve all backend-lint, frontend, semantic-boundary, markdown, and edition CI failures
- *(cli)* use as_str() for surface interaction label field
- address task 8 follow-up verification blockers
- align software merge contracts
- update installed_display_version: None in all DiscoveredSoftware struct literals
- *(smtp)* restrict helo_host to global settings only
- handle NULL plugin_config_id in display, version check, and update paths
- *(frontend)* display installed_version in host details assigned software list
- *(frontend)* fix duplicate key crash on software detail page for multi-link hosts
- *(cli)* update CLI and OpenAPI client for name-based ignore rules
- *(cli)* fix panic and add seamless plugin extension support
- *(extensions)* include context_selector.add_action in action lookup
- *(cli)* rename NatsCommands::Get to NatsCommands::Show
- *(cli)* fix truncate() to use char boundaries instead of byte indices
- *(cli)* use value_parser for batch status filter to reject invalid values
- *(web-api-types,cli)* fix clippy collapse-if and update CLI for 204 revoke_enrollment_token
- *(cli)* use constant for version_check task type string
- remove unapproved .expect() uses and Report::new() violations
- *(web-api,cli)* use correct http status codes for delete and idempotent create endpoints
- resolve clippy collapsible-if and missing test field
- frontend accessibility, security, and UX improvements with expanded tests
- resolve remaining codereview issues with ping interval, retry, and auto-refresh
- resolve top 5 codereview issues across frontend, CLI, and SSH agent
- resolve top 5 codereview issues across codebase
- resolve top 5 codereview issues across codebase
- resolve top 5 code review findings across shared-db, directories, web-api-types, and web-api
- resolve top 5 code review findings across directories, openapi-client, CLI, web-api-types, and shared-db
- resolve top 5 code review findings across 12 crates and frontend
- resolve top 5 code review findings across 8 crates
- re-export ParseMqttTransportError and use context_to() in CLI
- eliminate .expect() calls and improve error chain preservation
- resolve top 5 code review findings across 6 crates
- resolve top 5 code review findings across 8 crates
- resolve top 5 code review findings across 8 crates
- resolve top 5 code review findings across 8 crates
- resolve top 5 code review findings across 8 crates
- *(cli)* replace hand-rolled date calculation with time crate
- *(cli)* wrap api subcommand output in structured envelope for JSON/YAML

### Other

- add #[non_exhaustive] to update history response types
- *(release)* independent versioning for cli, openapi-client, service-sdk
- require labels for surface interactions
- align built-in pages with ui design spec
- remove dead extension cli and client exports
- isolate plugin boundaries in track a
- migrate cli and integration-tests to uptrakit-tracing-init
- migrate all binaries to TracingBuilder
- replace PluginType enum with PluginTypeId newtype
- *(codereview)* update CLI and add cross-cutting findings
- *(codereview)* refresh Rust backend review files
- *(web-api-types,cli,openapi-client,frontend)* remove MQTT settings API surface
- *(wire)* rename MqttBridge capability to UpdateTracking
- *(types)* accept Into<String> in SecretString and MaskedEmail constructors
- *(notifications)* remove dead notification-specific code
- *(controller)* split startup into phase-specific modules
- *(cli)* split settings command into per-category modules
- *(cli)* extract CLI parsing tests to separate module
- *(cli)* extract notification sub-dispatchers, remove #[allow(too_many_lines)]
- *(cli)* extract settings sub-dispatchers and remove #[allow(too_many_lines)]
- *(cli)* extract subcommand dispatch from monolithic run()
- *(api)* rename extra_sans to sans in API types and CLI
- *(codereview)* add 2026-03-10 comprehensive review across all 12 dimensions
- *(codereview)* add 2026-03-10 comprehensive review across all 12 dimensions
- *(cli)* update CLI and openapi-client for unified tracking
- *(api)* update scheduler API to use interval + jitter fields
- *(agent-ssh)* fix pre-existing rustfmt formatting
- merge 2026-03-06 parallel review findings into CODEREVIEW.md files
- merge 2026-03-06 parallel review findings into CODEREVIEW.md files
- *(web-api,cli)* resolve actions from shared catalogue
- add cargo-llvm-cov test coverage analysis to CODEREVIEW.md files
- add descriptions to binary crates and remove mqtt publish=false
- fmt
- remove resolved issues from CODEREVIEW.md files
- *(codereview)* remove confirmed-fixed issues from CODEREVIEW.md files
- *(codereview)* update backend code review findings
- migrate internal path deps to workspace = true
- *(codereview)* add tests, consistency, maintainability, and database dimensions to all per-crate reviews
- *(codereview)* remove already-fixed and false-positive items
- *(codereview)* remove fixed issues from all CODEREVIEW.md files
- *(types,wire,web-api-types)* add #[non_exhaustive] to public enums and update match arms
- *(codereview)* comprehensive backend code review across 6 dimensions
- remove fixed and false-positive entries from CODEREVIEW files
- apply cargo fmt to all changed files
- apply cargo fmt and prettier formatting
- apply rustfmt after rebasing onto main
- *(cargo)* add workspace lints and consolidate inline dependencies
- apply cargo fmt across workspace
- *(types)* rename PluginType variants and string representations
- *(cli)* update OpenAPI client and CLI for capability-based services
- *(cli)* update CLI and openapi-client for role-based plugin assignments
- update documentation for plugin architecture
- rename providers to plugins throughout the codebase
- apply cargo fmt to entire workspace
- add comprehensive code review findings
- add comprehensive code review findings
- *(cli)* introduce HumanOutput trait and typed command returns
- *(software-items)* decouple software items from provider configs
- remove obsolete TESTCOV files
- *(codereview)* resolve all remaining open code review findings
- rebase onto main and fix post-rebase build issues
- apply next-5 code review fixes (Issues 6–10)
- apply top-5 code review fixes (Issues 1–5)
- format rust sources after rebase
- full mock coverage for openapi-client with typed sections and shared paths
- remove upstream crate tests, add AsyncAPI spec conformance
- add output and formatting tests for CLI command modules
- refresh TESTCOV.md with cargo-llvm-cov coverage data after rebase
- add test coverage analysis (TESTCOV.md) for 16 crates
- cargo fmt + fix clippy await_holding_lock in shared-db test
- replace numeric HTTP status codes with StatusCode enum variants
- add extensibility-focused code review findings
- add codereview for CLI
- *(deps)* replace workspace-wide tokio full with per-crate minimal features
- *(cli)* remove direct reqwest dependency via openapi-client re-export
- cargo fmt
- remove CODEREVIEW.md files
- *(types)* use Uuid instead of String for all entity ID fields
- *(cli)* rename output binary from uptrakit-cli to uptrakit
- replace Result<T, String> with typed error enums across codebase
- fix top 5 code review issues (DB-05, OCSP, HA-04, CLI-04, session txn)
- mark resolved code review findings as FIXED
- *(cli)* use typed API response structs for token commands
- add extensibility-focused code review for all crates
- replace Result<T, String> in crypto.rs and migrate bail!() codebase-wide
- *(deps)* update Rust dependencies to latest versions
- drop hex, urlencoding and open crates
- add impl_report_conversion! macro and replace verbose ReportConversion impls
- enforce rootcause best practices and migrate errors to thiserror
- update all dependencies to latest versions
- extract HTTP request/response types into shared web-api-types crate
- document no-unwrap patterns in CONTRIBUTING.md and AGENTS.md
- initial commit
