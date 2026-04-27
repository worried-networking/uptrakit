# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.1](https://github.com/worried-networking/uptrakit/releases/tag/uptrakit-openapi-client-v0.0.1) - 2026-04-27

### Added

- *(audit)* add semantic audit log infrastructure
- *(settings)* add shared GitHub provider settings foundation
- port service providers and cli to surface runtime
- add software merge contracts
- *(auth)* enforce OIDC private issuer policy by tenancy mode
- *(cli)* add plugin-configs test subcommand and OpenAPI client method
- *(api)* add ordinal field to plugin assignments and delete endpoint
- *(smtp)* add helo_host setting and fix EHLO hostname for RFC 5321
- *(reset-data)* add backend for tenant data reset endpoint
- add plugin type settings REST API and OpenAPI client
- *(queries)* thread icon_url through all software item query paths
- *(openapi-client)* add user management, role, and preset client methods
- *(api)* add interactive flag to update trigger request
- add host tags query layer, REST API, OpenAPI client, and CLI
- *(openapi-client)* add batch action client methods and paths
- *(api)* add POST /services/{id}/update-freeze endpoint (ATK-17)
- *(wire,web-api)* add E2E encryption support for sensitive extension params
- instrument new code from main (extensions, SSE, event system, agent-core)
- *(openapi-client)* add typed admin events SSE client
- *(openapi-client, cli)* SSE-first device auth login with poll fallback
- *(cli)* add extensions list, providers, and invoke commands
- *(web-api)* add extension REST API endpoints and WS handler integration
- *(host-packages)* add promote endpoint to create software items from packages
- *(openapi-client)* add system enrollment token client methods, remove old settings
- *(openapi-client, cli)* add audit log client methods and audit-logs CLI subcommand
- *(api)* add /api/v1/system-services REST endpoints, openapi-client, and CLI
- *(web-api)* add per-service cert lifetime override
- *(openapi-client)* add host package client methods
- *(cli)* add batch update commands and SSE progress streaming client
- *(batch-updates)* add batch progress broadcasting and SSE streaming
- *(batch-updates)* add OpenAPI client methods and batch notification events
- *(openapi-client)* add NATS settings client methods
- *(openapi-client,cli)* SMTP settings client methods and CLI commands
- *(openapi-client)* add notification channel and rule client methods
- *(api)* add GET /api/v1/plugin-types with sample_config per type
- *(openapi-client)* add discovery allowlist client methods
- *(openapi-client)* add SSE parser and update output streaming method
- *(openapi-client)* add enrollment token client methods and mock helpers
- *(api)* add enrollment token CRUD endpoints and remove old single-token API
- *(registry)* register Shell plugin; add GithubReleases identifier validation
- *(api)* rename provider-configs to plugin-configs
- *(settings)* propagate ha_discovery/ha_discovery_prefix through MQTT client API and CLI
- *(autodiscovery)* add frontend UI, discovery_state filter, and doc fixes
- *(autodiscovery)* implement software autodiscovery feature
- add mock feature to openapi-client and CLI integration tests
- *(ui)* add SSH Agent service type to web UI
- *(openapi-client)* expand to full REST API coverage
- *(cli,web-api)* add service management CLI commands and GET endpoint
- *(openapi-client)* add typed HTTP client crate and migrate CLI

### Fixed

- make all pre-push quality gates pass
- *(ci)* resolve all backend-lint, frontend, semantic-boundary, markdown, and edition CI failures
- align software merge contracts
- merge_service txn consistency, openapi-client plugin_type field, clippy collapsible-if
- *(smtp)* restrict helo_host to global settings only
- *(sse)* fix SSE data serialisation + add UpdateTriggered event
- *(cli)* update CLI and OpenAPI client for name-based ignore rules
- *(web-api)* return HTTP 204 from revoke_enrollment_token; add non-exhaustive wildcard arms
- *(web-api,cli)* use correct http status codes for delete and idempotent create endpoints
- frontend accessibility, security, and UX improvements with expanded tests
- resolve remaining codereview issues with ping interval, retry, and auto-refresh
- resolve top 5 codereview issues across codebase
- resolve top 5 codereview issues across codebase
- resolve top 5 code review findings across shared-db, directories, web-api-types, and web-api
- resolve top 5 code review findings across mqtt, agent-ssh, web-api-types, shared-types, and command
- resolve top 5 code review findings across directories, openapi-client, CLI, web-api-types, and shared-db
- resolve top 5 code review findings across 12 crates and frontend
- resolve top 5 code review findings across 8 crates

### Other

- add #[non_exhaustive] to update history response types
- *(release)* independent versioning for cli, openapi-client, service-sdk
- remove legacy extensions http api
- remove dead extension cli and client exports
- Add parallel shared-surface runtime path for Task 3
- replace PluginType enum with PluginTypeId newtype
- *(codereview)* update shared library review findings
- *(codereview)* refresh Rust backend review files
- *(web-api-types,cli,openapi-client,frontend)* remove MQTT settings API surface
- *(wire)* rename MqttBridge capability to UpdateTracking
- *(types)* accept Into<String> in SecretString and MaskedEmail constructors
- *(notifications)* remove dead notification-specific code
- *(api)* rename extra_sans to sans in API types and CLI
- *(codereview)* add 2026-03-10 comprehensive review across all 12 dimensions
- *(codereview)* add 2026-03-10 comprehensive review across all 12 dimensions
- *(cli)* update CLI and openapi-client for unified tracking
- *(api)* update scheduler API to use interval + jitter fields
- add cargo-llvm-cov test coverage analysis to CODEREVIEW.md files
- fmt
- *(settings)* move global settings to /api/v1/global-settings/
- *(codereview)* update backend code review findings
- migrate external deps to workspace = true
- *(codereview)* add tests, consistency, maintainability, and database dimensions to all per-crate reviews
- *(codereview)* comprehensive backend code review across 6 dimensions
- *(codereview)* add missing per-crate CODEREVIEW.md files
- apply cargo fmt to all changed files
- apply cargo fmt and prettier formatting
- *(cargo)* add workspace lints and consolidate inline dependencies
- apply cargo fmt across workspace
- *(types)* rename PluginType variants and string representations
- *(cli)* update OpenAPI client and CLI for capability-based services
- *(cli)* update CLI and openapi-client for role-based plugin assignments
- update documentation for plugin architecture
- rename providers to plugins throughout the codebase
- apply cargo fmt to entire workspace
- *(software-items)* decouple software items from provider configs
- remove obsolete TESTCOV files
- *(codereview)* resolve all remaining open code review findings
- *(codereview)* resolve top 5 open code review findings
- rebase onto main and fix post-rebase build issues
- apply next-5 code review fixes (Issues 6–10)
- format rust sources after rebase
- full mock coverage for openapi-client with typed sections and shared paths
- apply cargo fmt formatting across workspace
- refresh TESTCOV.md with cargo-llvm-cov coverage data after rebase
- add test coverage analysis (TESTCOV.md) for 16 crates
- cargo fmt + fix clippy await_holding_lock in shared-db test
- replace numeric HTTP status codes with StatusCode enum variants
- add extensibility-focused code review findings
- *(codereview)* add code review for controller, web-api, and openapi-client
- add code review results for shared crates
- *(cli)* remove direct reqwest dependency via openapi-client re-export
- cargo fmt
- *(types)* use Uuid instead of String for all entity ID fields
