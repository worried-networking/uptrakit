# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.1](https://github.com/worried-networking/uptrakit/releases/tag/uptrakit-shared-types-v0.0.1) - 2026-04-21

### Added

- *(settings)* add shared GitHub provider settings foundation
- *(shared-types)* add test-support plugin id constants
- add foundation types for plugin config testing (dry-run)
- *(plugins)* add SoftwareItemLifecycle capability and plugin subtrait
- *(types)* add PreUpdateHook/PostUpdateHook plugin roles and HookSystemd/HookShell plugin types
- *(docker)* extract resolve_image_info helper and populate installed_display_version during discovery
- *(update-queue)* queue single updates when host is busy instead of 409
- add PluginBase trait hierarchy with 10 capability subtraits
- *(types)* add PackageManagerCargo variant to PluginType
- add Snap package manager plugin crate and PluginType variant
- *(types)* add PackageManagerApk variant to PluginType
- register BSD pkg plugin type and wire protocol updates
- *(plugin)* add Pacman package manager plugin
- *(plugin)* add DNF package manager plugin
- *(types)* add AccessPreset enum and Permission::all() method
- *(permissions)* replace 16 coarse permissions with 32 granular ones
- *(types)* add AttestationStatus Other(String) wire-safe catch-all
- *(wire)* unify wire protocol for single tracking model
- *(docker)* use image-level package_identifier with qualifier for per-container tracking
- *(shared-types)* add SSRF-safe DNS resolver for reqwest clients
- *(shared-types)* move Permission enum from web-api-types to shared-types
- *(shared-types)* add InfrastructureProxmox plugin type
- *(shared-types)* add Queued variant to UpdateStatus
- *(types)* add PackageManagerMas to PluginType
- *(types)* add AttestationStatus, extend ReleaseAsset and ReleaseInfo
- *(security)* add command validation for plugin configs (ATK-16 Phase 1)
- *(plugins)* add tracking_system field to DiscoveredSoftware
- *(shared-types)* add TrackingSystem enum
- *(db)* add update_batches table, BatchStatus enum, and batch_id FK on update_history
- add UpdateCategory enum, wire protocol field, and plugin interface
- *(shared-types)* add ReleasesGitlab and ReleasesCodeberg PluginType variants
- *(shared-types)* move PluginCapability to shared-types, remove all_known()
- *(npm)* add uptrakit-plugin-package-manager-npm crate and PluginType variant
- *(registry)* register Shell plugin; add GithubReleases identifier validation
- *(types)* add Shell plugin type
- *(types)* add DiscoveryTarget type for plugin-driven discovery
- *(types)* add PluginRole enum for role-based plugin assignments
- *(types)* add Apt variant to ProviderType
- *(types)* add ProviderType::Other(String) for wire forward-compatibility
- *(autodiscovery)* implement software autodiscovery feature
- add SSH-backed agent skeleton with controller enrollment and local encrypted storage
- Implement TOP3 least-effort fixes from codereview

### Fixed

- *(ci)* resolve all backend-lint, frontend, semantic-boundary, markdown, and edition CI failures
- *(agent-ssh)* detect restricted sudoers with sudo -l
- *(plugins)* replace rustls-platform-verifier with webpki-roots for HTTP clients
- *(types)* remove ZeroizeOnDrop from MaskedEmail
- *(shared-types)* add Other(String) catch-all to BatchStatus and UpdateCategory
- *(shared-types,db,web-api)* move UpdateStatus to shared/types and fix in_progress status bug
- *(web-api-types,wire,types)* replace ALL_EVENT_TYPES const with strum EnumIter across all enums
- resolve top-5 code-review issues (N+1, wildcards, discovery, OIDC)
- frontend accessibility, security, and UX improvements with expanded tests
- resolve top 5 codereview issues across codebase
- resolve top 5 codereview issues across codebase
- resolve top 5 code review findings across shared-db, directories, web-api-types, and web-api
- resolve top 5 code review findings across mqtt, agent-ssh, web-api-types, shared-types, and command
- resolve top 5 code review findings across 12 crates and frontend
- resolve top 5 code review findings across 6 crates
- *(web-api,wire,types)* implement code review fix plans #9-#16

### Other

- *(surfaces)* remove extension-era runtime leftovers
- enforce plugin semantic boundary
- isolate plugin boundaries in track a
- *(types)* replace HostFeature enum with forward-compatible Cow newtype
- replace PluginType enum with PluginTypeId newtype
- *(types)* add foundation types for plugin framework redesign
- *(codereview)* update shared library review findings
- *(core)* remove shared MQTT-specific config ownership
- *(codereview)* refresh Rust backend review files
- shared HTTP client builder, Validated<T> extractor, PackageIdentifierRules
- remove old hook system
- *(types)* accept Into<String> in SecretString and MaskedEmail constructors
- stop auto-creating plugin_configs for package managers
- *(codereview)* add 2026-03-10 comprehensive review across all 12 dimensions
- *(codereview)* add 2026-03-10 comprehensive review across all 12 dimensions
- remove fixed issues from all CODEREVIEW.md files
- remove fixed issues from CODEREVIEW.md files
- *(shared-types)* extract is_private_ip from is_private_host
- merge 2026-03-06 parallel review findings into CODEREVIEW.md files
- *(deps)* add husky-rs for automatic git hook installation
- fmt
- *(types)* consolidate is_private_host into shared network module
- remove resolved issues from CODEREVIEW.md files
- *(codereview)* update backend code review findings
- *(codereview)* add tests, consistency, maintainability, and database dimensions to all per-crate reviews
- *(types)* add ZeroizeOnDrop to MaskedEmail
- *(shared-types)* rename ReleasesCodeberg → ReleasesForgejo
- document non_exhaustive, HTTP timeouts, and parking_lot; clarify start_paused rule
- *(types,wire,web-api-types)* add #[non_exhaustive] to public enums and update match arms
- *(codereview)* comprehensive backend code review across 6 dimensions
- *(cargo)* add workspace lints and consolidate inline dependencies
- apply cargo fmt across workspace
- *(types)* rename PluginType variants and string representations
- *(types)* remove ServiceType enum entirely
- update documentation for plugin architecture
- rename providers to plugins throughout the codebase
- *(types)* rename ProviderType::DockerRegistry to Docker
- apply cargo fmt to entire workspace
- remove obsolete TESTCOV files
- *(codereview)* resolve all remaining open code review findings
- *(codereview)* resolve top 5 open code review findings
- apply next-5 code review fixes (Issues 6–10)
- apply cargo fmt formatting across workspace
- refresh TESTCOV.md with cargo-llvm-cov coverage data after rebase
- add test coverage analysis (TESTCOV.md) for 16 crates
- update CODEREVIEW files and documentation for resolved issues
- add extensibility-focused code review findings
- add code review results for shared crates
- remove CODEREVIEW.md files
- cargo fmt
- *(types)* replace ad-hoc string parsers with FromStr
- fix top 5 code review issues (DB-05, OCSP, HA-04, CLI-04, session txn)
- mark XC-06, PREG-05, WAT-02, WAT-08, WAT-09, DB-05, SDK-03 as FIXED
- *(shared-types,web-api)* add DeviceAuthStatus enum, replace String across DB/API/logic (WAT-02, DB-05)
- add #[non_exhaustive] to public enums (XC-06, WIRE-05, TYP-03)
- mark resolved code review findings as FIXED
- *(shared-types,registry)* add FromStr to ProviderType, replace serde hack in registry
- resolve top 5 code review issues across workspace
- add extensibility-focused code review for all crates
- add code review reports for 7 shared crates
- move ProviderType, ReleaseAsset, ReleaseInfo to shared-types
- drop hex, urlencoding and open crates
