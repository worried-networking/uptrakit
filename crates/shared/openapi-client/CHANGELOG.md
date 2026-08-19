# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.5](https://github.com/worried-networking/uptrakit/compare/uptrakit-openapi-client-v0.0.4...uptrakit-openapi-client-v0.0.5) - 2026-08-19

### Added

- *(settings)* validate oauth.canonical_host as a bare host
- *(plugins/uv)* implement discovery and version detection

### Fixed

- *(oauth)* reject backslash in canonical_host shape gate
- *(plugins/uv)* use dotted-kebab type id and fail loud on parse drift

## [0.0.4](https://github.com/worried-networking/uptrakit/compare/uptrakit-openapi-client-v0.0.3...uptrakit-openapi-client-v0.0.4) - 2026-08-18

### Fixed

- *(web-api)* make plugin-config If-Match satisfiable via ETag layer

### Other

- *(deps)* bump sha2 to 0.11 and hmac to 0.13

## [0.0.3](https://github.com/worried-networking/uptrakit/compare/uptrakit-openapi-client-v0.0.2...uptrakit-openapi-client-v0.0.3) - 2026-08-13

### Added

- *(web-api)* [**breaking**] retire the wire-level Permission vocabulary
- *(web-api)* [**breaking**] remove the access-preset endpoints and consumers
- *(web-api)* add the access catalog endpoint
- *(web-api)* [**breaking**] role CRUD on access:manage with reshaped RoleResponse
- *(web-api)* grant update/delete with lockout guard and system-plane fine check
- *(web-api)* grant create/list/get endpoints with audit and invalidation
- *(openapi-client)* OAuth clients/consents methods
- *(cli)* method-aware surface interaction dispatch + typed client methods
- *(surfaces)* [**breaking**] read model at GET /surfaces/{surface_id}; register surface routes in OpenAPI
- *(web-api)* add sealed RoutingEnvelope projection on Unvalidated bodies
- *(auth)* [**breaking**] add actions and authority fields to UserResponse
- *(web-api-types)* Validate impls for remaining unvalidated mutation requests
- *(web-api-types)* Validate impls for software-item mutation requests
- *(web-api-types)* Validate impls for OIDC update/exchange/registration requests
- *(web-api-types)* grant management request/response types
- *(web-api-types)* typed OAuth client/consent response rows
- *(web-api)* [**breaking**] method-mapped surface interaction routes with 405/Allow, HEAD, item segment, GET coercion
- *(web-api-types)* openapi schema derives for surface DTOs
- *(web-api-types)* add UpdateStatus::Interrupted variant
- *(web-api)* emit access.denied audit Events for qualifying denials
- *(web-api)* [**breaking**] users:manage/access:manage split with engine-backed lockout guard
- *(shared-types)* add system.access:manage catalog action
- *(surfaces)* [**breaking**] required_permission becomes required_action, typed and engine-enforced
- *(types)* iterate DenyReason via strum::EnumIter in label test
- *(types)* DenyReason::as_str + Display for deny labels
- *(types)* access decision types (TargetRef, Decision, DenyReason, Visibility)
- *(types)* grant patterns with wildcard matching and write-time validity
- *(types)* Action with parse-time matrix rejection, serde, and typed catalog constants
- *(types)* access catalog macro, Resource, and built-in action matrix
- *(types)* access Selector, SelectorSupport, and bounded-size constants
- *(types)* access module scaffold with Verb and kebab-segment grammar
- *(wire)* manual wire-format JsonSchema impls for custom-serde enums
- *(wire)* additive schema feature with JsonSchema derives across wire/shared-types/surfaces
- *(types)* add terminal UpdateStatus::Interrupted (outcome unknown)
- *(web-api)* typed-slot dispatch for InstalledVersionEnricher
- *(plugin-types)* add EnrichInstalledVersion capability
- *(surfaces)* admission rejects provider_invocable under a gated descriptor
- *(surfaces)* method on SurfaceActionRequest; proxy stamps effective method; per-field body validation
- *(surfaces)* (id,method) uniqueness, kind/method matrix, params rules, method-aware reference resolution
- *(surfaces)* ActionRef two-form reader + method disambiguation on reference nodes
- *(surfaces)* ParamFieldDescriptor + params declarations with wire bounds
- *(surfaces)* add InteractionHttpMethod + http_method on InteractionDescriptor
- *(surfaces)* add provider_invocable opt-in to InteractionDescriptor

### Fixed

- report awaiting_restart update-history status instead of pending
- *(validation)* unify invoke envelope type and tighten gate skip paths
- close the peek_envelope doc gap and the untested gate baseline modes
- *(web-api)* make invoke-request canary fixtures validator-hostile
- *(web-api-types)* relax UpdateHostAssignmentRequest to at-most-one plugin source
- *(web-api)* restore access-catalog doc comments and assertion strength
- *(web-api-types)* declare schema feature for wire_safe_enum! macro arm
- *(web-api)* dedupe ReadSurfaceInteractionQuery to web-api-types; drop unnecessary clippy expect
- *(web-api-queries)* tenant-scope read-path host loads
- *(web-api-types)* emit serde wire strings in ToSchema for catch-all enums
- *(wire)* restructure schema_tests for clippy test-lint allowlist, dedupe AttestationStatus wire strings
- *(ci)* recognize prefixed raw strings in the no-orphan-modules sanitizer
- *(surfaces)* guard skew on the real descriptor, align the orphaned prepare gate
- *(surfaces)* bound reference-node http_method wire strings (WireValidate invariant)

### Other

- *(plugins)* [**breaking**] plugin type IDs adopt dot-separated kebab-case grammar
- fix UpdateStatus doc drift left by the DB/API enum de-duplication
- *(auth)* [**breaking**] delete the legacy Permission model and its tables
- *(wire)* UpdateStarted.interactive carries dispatch intent
- *(web-api)* drift-proof remaining query handlers via params(<IntoParamsStruct>)
- *(web-api)* convert list handlers to params(<IntoParamsStruct>) (drift-proof)
- *(controller)* document canonical sha256 config digest + stale-on-error
- *(shared-types)* rename AccessPreset to RoleBundle
- *(types)* clarify CatalogEntry constructor-exception citation
- *(plugins)* [**breaking**] delete legacy surface_actions machinery; surfaces: arm is single-source
- *(agent-core)* deadlock and PTY-targeting regressions with stub lifecycle-hook plugin
- slim root AGENTS.md to invariants + pointers; fix stale facts
- *(web-api-types)* guard PluginRole schema drift; point staleness msg to regen-api.sh
- *(surfaces)* pin BuiltIn-kind admission permissiveness for the ADR-0040 gates
- *(surfaces)* split descriptor-gated provider_invocable admission rule
- *(surfaces)* tighten method-model admission tests (raw-json wire path, reserved-key coverage, message substrings)
- *(surfaces)* [**breaking**] delete dead DirectBuiltInApi transport variant

## [0.0.2](https://github.com/worried-networking/uptrakit/compare/uptrakit-openapi-client-v0.0.1...uptrakit-openapi-client-v0.0.2) - 2026-05-05

### Added

- *(events)* add AdminEvent::SurfacesChanged unit variant
- *(types)* add UpdateStatus::AwaitingRestart variant
- *(permissions)* add AccessMcp variant and DB migration
- *(permissions)* wire-safe Other(String) catch-all for unknown variants
- *(task-18)* wire UptrakitSelfUpdatePlugin into controller-standalone registry
- *(surfaces)* add tab_group concept; split Proxmox update-hooks into two surfaces
- *(surfaces)* add icon field to InteractionDescriptor and validate it
- *(surfaces)* add validate_icon_name and IconNameError
- *(surfaces)* add optional nav_icon field to SurfaceDescriptor with wire validation

### Fixed

- *(clippy)* remediate new lint violations (panic, silent-failure, unsafe)
- add #[non_exhaustive] to interaction enums; add @container/buttons to workflow trigger

### Other

- *(web-api-types)* add AwaitingRestart to as_str_values assertion
- *(surfaces)* complete `# Errors` section on validate_for_provider
- *(surfaces)* derive Copy on FrameworkGenerationRange, remove redundant clone

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
