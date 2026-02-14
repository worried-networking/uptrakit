# Cross-Crate Architecture Review: External Extensibility

**Date:** 2026-02-14
**Scope:** All 19 workspace crates + frontend
**Focus:** External extensibility -- ease of creating new services, API client apps, and providers

This document summarizes cross-cutting findings that span multiple crates. Per-crate reviews are in each
crate's `CODEREVIEW.md` file.

## Dependency Graph

```text
uptrakit-shared-types (leaf -- no internal deps)
    |
    +-- uptrakit-shared-macros (depends on: rootcause)
    |
    +-- uptrakit-internal-wire (depends on: shared-types)
    |       |
    |       +-- uptrakit-web-api-types (depends on: wire)
    |       |
    |       +-- uptrakit-service-sdk (depends on: wire, shared-types, directories)
    |
    +-- uptrakit-shared-db (depends on: shared-types, shared-macros)
    |
    +-- uptrakit-command (no internal deps)
    |
    +-- uptrakit-directories (no internal deps)
    |
    +-- uptrakit-build-info (no internal deps)
    |
    +-- uptrakit-provider-core (depends on: command, shared-types)
    |       |
    |       +-- uptrakit-provider-{github,docker,homebrew,proxmox} (depend on: provider-core only)
    |       |
    |       +-- uptrakit-provider-registry (depends on: provider-core, all 4 providers, shared-types)
    |
    +-- uptrakit-web-api (depends on: shared-db, wire, provider-registry, web-api-types, directories)
    |
    +-- uptrakit-agent (depends on: service-sdk, wire, command, provider-registry, shared-types, build-info)
    |
    +-- uptrakit-mqtt (depends on: service-sdk, wire, web-api-types, build-info)
    |
    +-- uptrakit-controller (depends on: web-api, shared-db, wire, web-api-types, directories, shared-types, build-info)
    |
    +-- uptrakit-cli (depends on: web-api-types, shared-macros, build-info)
```

## Cross-Cutting Findings

### XC-01: Pervasive Type Duplication (Critical)

The most serious architectural issue across the codebase. Four types exist in multiple locations:

| Type | Locations | Impact |
|---|---|---|
| `ServiceType` | wire, web-api-types, shared-db | Adding a new service type requires 3 coordinated changes |
| `MqttTransport` | wire, web-api-types | MQTT service manually maps between the two |
| `ShellType` / `HookShell` | command, wire | Agent must manually map between them |
| Status enums | wire, web-api-types, shared-db | Semantically overlapping but structurally distinct |

**Recommendation:** Move canonical types to `shared-types` with feature-gated derives:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "sea-orm", derive(sea_orm::EnumIter, sea_orm::DeriveActiveEnum))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum ServiceType {
    Agent,
    Mqtt,
}
```

**Related per-crate findings:** [WIRE-04](crates/shared/wire/CODEREVIEW.md#wire-04),
[WAT-07](crates/shared/web-api-types/CODEREVIEW.md),
[CMD-03](crates/shared/command/CODEREVIEW.md),
[MQTT-06](crates/core/mqtt/CODEREVIEW.md#mqtt-06)

### XC-02: Provider System is Closed to External Providers (Critical)

The combination of a closed `ProviderType` enum and a static-dispatch registry makes it impossible for
external developers to add providers without forking two crates:

1. `ProviderType` in `shared-types` is a closed enum with 4 variants (no `#[non_exhaustive]`)
2. `ProviderRegistry` in `provider-registry` is a unit struct with static methods using `match` on `ProviderType`
3. All 4 provider crates are unconditional compile-time dependencies

Adding a new provider requires:
1. Adding a variant to `ProviderType` (breaks all downstream exhaustive matchers)
2. Adding match arms in 4 registry methods
3. Adding the new crate as a dependency in `provider-registry/Cargo.toml`

**Recommendation:** Short-term: add `#[non_exhaustive]` and feature gates. Medium-term: introduce a
`ProviderFactory` trait with runtime registration. Long-term: string-based provider type identification.

**Related per-crate findings:** [TYP-03](crates/shared/types/CODEREVIEW.md),
[PREG-01](crates/providers/registry/CODEREVIEW.md),
[PREG-02](crates/providers/registry/CODEREVIEW.md),
[PREG-03](crates/providers/registry/CODEREVIEW.md)

### XC-03: No Service Lifecycle Abstraction in SDK (Major)

The service-sdk provides all building blocks but no high-level abstraction for the service lifecycle.
~200 lines of enrollment/reconnection boilerplate are duplicated between the agent and MQTT service:

- URL parsing with error wrapping
- Directory resolution
- Identity loading, force-enroll check
- CA bootstrap
- Certificate expiry check and fallback
- Enrollment with backoff loop
- Reconnection loop with backoff

An external developer building a new service would create a third copy of this boilerplate.

**Recommendation:** Introduce a `ServiceHandler` trait and `run_service_lifecycle()` function in the SDK.

**Related per-crate findings:** [SDK-02](crates/shared/service-sdk/CODEREVIEW.md#sdk-02),
[AGENT-02](crates/core/agent/CODEREVIEW.md#agent-02),
[MQTT-05](crates/core/mqtt/CODEREVIEW.md#mqtt-05)

### XC-04: web-api-types Lacks Prelude for API Client Developers (Major)

The crate exposes 28 public modules with zero root-level re-exports. External API client developers must
discover which module contains each type. The CLI crate proves this works but requires verbose imports.

**Recommendation:** Add a `pub mod prelude` or flat re-exports of commonly used types (all response types,
request types, `ErrorResponse`, `PaginatedResponse`, `Permission`).

**Related per-crate findings:** [WAT-06](crates/shared/web-api-types/CODEREVIEW.md)

### XC-05: web-api AppState Difficult to Construct for Embedding (Major)

`AppState` has 21 fields, no builder or factory, and a hard dependency on `axum_server::RustlsConfig`. An
external developer wanting to embed the web-api router must construct all 21 fields manually with no
documented guidance. The only examples are scattered test helpers.

**Recommendation:** Add an `AppStateBuilder` with sensible defaults, make TLS config optional.

**Related per-crate findings:** See web-api CODEREVIEW.md extensibility section.

### XC-06: Closed Enums Without `#[non_exhaustive]` (Minor)

Multiple public enums across the codebase lack `#[non_exhaustive]`:

- `ProviderType` (shared-types)
- `ServiceMessage`, `ControllerMessage` (wire)
- `ServiceType` (wire, web-api-types, db)
- `ProviderCapability` (provider-core)

Adding variants to any of these is a semver-breaking change, even for in-project evolution.

**Recommendation:** Apply `#[non_exhaustive]` to all public enums that may evolve.

### XC-07: Duplicated `strip_tag_prefix` in Provider Crates (Minor)

Identical `strip_tag_prefix` function in `providers/github/src/tag.rs` and
`providers/docker-registry/src/tag.rs`. Should be extracted to `provider-core`.

**Related per-crate findings:** [GH-04](crates/providers/github/CODEREVIEW.md),
[DOCK-02](crates/providers/docker-registry/CODEREVIEW.md)

### XC-08: Undocumented `install_command` / `restart_command` in Provider Configs (Minor)

Both GitHub and Docker providers extract execution commands from raw JSON `provider_config` rather than
from their typed config structs. These fields are not declared in `GitHubConfig` or `DockerRegistryConfig`,
creating an undocumented configuration surface for API consumers.

**Related per-crate findings:** [GH-01](crates/providers/github/CODEREVIEW.md),
[DOCK-01](crates/providers/docker-registry/CODEREVIEW.md),
[PCORE-01](crates/providers/core/CODEREVIEW.md)

## Three Extensibility Scenarios

### Scenario 1: Building a New Service

**Current path:** Study MQTT source code, copy ~200 lines of boilerplate, implement message handling.

**After recommended changes:** Implement `ServiceHandler` trait (3 methods), call `run_service_lifecycle()`.

**Key blockers:** [XC-03](#xc-03-no-service-lifecycle-abstraction-in-sdk-major) (no SDK abstraction),
[XC-01](#xc-01-pervasive-type-duplication-critical) (if new service type needed)

### Scenario 2: Building an API Client

**Current path:** Depend on `web-api-types`, discover types across 28 modules, write verbose imports.

**After recommended changes:** `use uptrakit_web_api_types::prelude::*` for all common types.

**Key blockers:** [XC-04](#xc-04-web-api-types-lacks-prelude-for-api-client-developers-major) (no prelude),
[WAT-08/09](crates/shared/web-api-types/CODEREVIEW.md) (weak typing in some request types)

### Scenario 3: Building a New Provider

**Current path:** Impossible without forking `shared-types` and `provider-registry`.

**After recommended changes:** Implement `Provider` trait + `ProviderFactory` trait, call
`registry.register()`.

**Key blockers:** [XC-02](#xc-02-provider-system-is-closed-to-external-providers-critical) (closed
enum + static registry)

## Summary of All Findings by Severity

| Severity | Count | Key Themes |
|---|---|---|
| Critical | 2 | Type duplication across crates, closed provider system |
| Major | 5 | No service lifecycle abstraction, no prelude, AppState construction, overloaded SDK error types, monolithic controller run() |
| Minor | ~20 | Closed enums without `#[non_exhaustive]`, duplicated code, hardcoded values, inconsistent `FromStr`/`Display` |
| Info | ~15 | Missing derives, documentation gaps, ergonomic suggestions |

## Priority Recommendations

1. **Consolidate duplicated types into `shared-types`** with feature-gated derives. This is the single
   highest-impact change -- it eliminates divergence risk and makes adding new variants a single-crate change.

2. **Add `#[non_exhaustive]` to all public enums** in wire, shared-types, and web-api-types. One-line change
   per enum that enables future evolution.

3. **Add a prelude to `web-api-types`**. Flat re-exports of the 20-30 most commonly used types for API client
   developers.

4. **Introduce a `ServiceHandler` trait in service-sdk**. Default `run_service_lifecycle()` function handles
   enrollment + reconnection + message loop. This eliminates ~200 lines of duplication per service.

5. **Introduce `ProviderFactory` trait for runtime registration**. Make `ProviderRegistry` hold a
   `HashMap<String, Box<dyn ProviderFactory>>` with `register()` method. Built-in providers self-register;
   external crates register their own.

6. **Add `AppStateBuilder` to web-api**. Documented construction with sensible defaults and optional TLS
   config for embedding.

## Per-Crate Review Index

| Crate | File | Key Extensibility Finding |
|---|---|---|
| shared/types | [CODEREVIEW.md](crates/shared/types/CODEREVIEW.md) | Closed `ProviderType` enum (TYP-03) |
| shared/macros | [CODEREVIEW.md](crates/shared/macros/CODEREVIEW.md) | Missing rootcause documentation (MAC-03) |
| shared/wire | [CODEREVIEW.md](crates/shared/wire/CODEREVIEW.md) | Type duplication (WIRE-04), no `#[non_exhaustive]` (WIRE-05) |
| shared/web-api-types | [CODEREVIEW.md](crates/shared/web-api-types/CODEREVIEW.md) | No prelude (WAT-06), type duplication (WAT-07) |
| shared/db | [CODEREVIEW.md](crates/shared/db/CODEREVIEW.md) | Third copy of ServiceType (DB-05) |
| shared/command | [CODEREVIEW.md](crates/shared/command/CODEREVIEW.md) | ShellType duplication (CMD-03) |
| shared/directories | [CODEREVIEW.md](crates/shared/directories/CODEREVIEW.md) | No extensibility issues |
| shared/service-sdk | [CODEREVIEW.md](crates/shared/service-sdk/CODEREVIEW.md) | No ServiceHandler trait (SDK-02) |
| shared/build-info | [CODEREVIEW.md](crates/shared/build-info/CODEREVIEW.md) | Missing Deserialize (BI-03) |
| providers/core | [CODEREVIEW.md](crates/providers/core/CODEREVIEW.md) | Raw JSON in execute_update (PCORE-01) |
| providers/registry | [CODEREVIEW.md](crates/providers/registry/CODEREVIEW.md) | Closed registry (PREG-01) |
| providers/github | [CODEREVIEW.md](crates/providers/github/CODEREVIEW.md) | Undocumented install_command (GH-01) |
| providers/docker-registry | [CODEREVIEW.md](crates/providers/docker-registry/CODEREVIEW.md) | Undocumented restart_command (DOCK-01) |
| providers/homebrew | [CODEREVIEW.md](crates/providers/homebrew/CODEREVIEW.md) | Discarded exit code (BREW-02) |
| providers/proxmox-helper-scripts | [CODEREVIEW.md](crates/providers/proxmox-helper-scripts/CODEREVIEW.md) | Capability contract violation (PHS-01) |
| ui/web-api | [CODEREVIEW.md](crates/ui/web-api/CODEREVIEW.md) | AppState construction barrier |
| ui/cli | [CODEREVIEW.md](crates/ui/cli/CODEREVIEW.md) | Proves web-api-types is sufficient for clients |
| core/agent | [CODEREVIEW.md](crates/core/agent/CODEREVIEW.md) | Enrollment boilerplate (AGENT-02) |
| core/controller | [CODEREVIEW.md](crates/core/controller/CODEREVIEW.md) | Monolithic run() function |
| core/mqtt | [CODEREVIEW.md](crates/core/mqtt/CODEREVIEW.md) | Best service reference; enrollment boilerplate (MQTT-05) |
| frontend | [CODEREVIEW.md](frontend/CODEREVIEW.md) | (Separate frontend review) |
