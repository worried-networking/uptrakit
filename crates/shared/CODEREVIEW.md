# Code Review: `crates/shared` Umbrella

- Review date: 2026-03-17
- Scope: current-state review for shared crates without their own dedicated `CODEREVIEW.md`

## Covered Crates

- `uptrakit-audit-log`
- `uptrakit-backoff`
- `uptrakit-build-info`
- `uptrakit-directories`
- `uptrakit-extension-framework`
- `uptrakit-shared-macros`
- `uptrakit-config-merge`
- `uptrakit-agent-core` (allocation and idiomatic Rust patterns only; HA findings in
  `crates/core/CODEREVIEW.md`)

## Summary

The shared utility layer remains strong overall. Most of these crates are small, stable, and easy
to reason about. This review cycle added allocation findings in `agent-core`'s hot update path and
confirmed the existing maintainability concern in `extension-framework`. Coding-standards compliance
across all shared crates is clean — no violations were found.

## Strengths

- `backoff`, `build-info`, `macros`, and `config-merge` stay small and focused.
- `directories` still provides good platform-aware permission handling and path validation.
- The shared crates continue to enforce workspace conventions instead of weakening them.
- `service-sdk` exports a minimal, stable surface: no internal types leaked, no `pub use *`.
- All required `#[non_exhaustive]`, `Other(String)` catch-all, and `parking_lot` patterns are
  present and correct across shared types, wire, and web-api-types.

## Active Findings

### [MEDIUM] `uptrakit-extension-framework` is a monolithic single-file schema crate

- Dimension: maintainability, crate structure
- Scope: `crates/shared/extension-framework/src/lib.rs` (1970 lines)
- Why it matters: two distinct domains live in one file: UI definitions (manifests, forms, fields,
  actions) and wire payloads (register/request/response messages). Changes to either domain require
  reasoning about the full 1970-line file. Adding a new extension-form feature risks unintentional
  serialization regressions in the wire domain.
- Recommendation: split into at minimum two internal modules (`ui.rs` and `wire.rs`), or two
  separate crates (`extension-ui` and `extension-wire`). The crate split is trivial effort (no
  circular dependencies) and is the highest-value structural improvement available.

### [MEDIUM] `agent-core` clones large update payloads unnecessarily in the dispatch hot path

- Dimension: allocation, performance
- Scope: `crates/shared/agent-core/src/client.rs:start_update`,
  `crates/shared/agent-core/src/client.rs:batch_update_inner`
- Why it matters: `start_update` clones the entire `ExecuteUpdatePayload` (including nested
  `serde_json::Value` plugin configs) to apply connection-context mutations. `batch_update_inner`
  clones every `package_identifier` and `release_info` twice — once for the correlation HashMap
  and once for `BatchUpdateItem`. For batches of 100 packages, this allocates O(N) large JSON
  values on every dispatch.
- Fix: apply connection-context mutations before constructing the payload, or use `&str` keys in
  the correlation HashMap to avoid double-cloning.

### [MEDIUM] `HashSet` is cloned in full before the early-emptiness check in WS event handlers

- Dimension: allocation, performance
- Scope:
  `crates/ui/web-api/src/routes/service_ws/handler/messages.rs` and related handler files
  (7+ call sites)
- Why it matters: `linked_host_ids.lock().clone()` allocates a full `HashSet<Uuid>` clone before
  checking `if current_ids.is_empty()`. For a typical host count of 5–20, the clone cost is small
  but it fires on every incoming `ReportHosts` or connectivity message in the main WS loop.
- Fix: check emptiness under the lock guard before cloning; only clone when the subsequent
  iteration is actually needed.

### [LOW] `PluginType::From<String>` reimplements the `as_str()` match table

- Dimension: idiomatic Rust, maintainability
- Scope: `crates/shared/types/src/plugin_types.rs`, `From<PluginType> for String` impl
- Why it matters: the `From<PluginType> for String` match arm duplicates the string values already
  present in `as_str()`, creating two sources of truth for the same mapping. A future rename of a
  plugin type string requires updating both locations.
- Fix: implement `From<PluginType> for String` as `pt.as_str().to_string()` to delegate to the
  single source of truth.

### [LOW] `uptrakit-directories` is drifting toward the same monolithic shape

- Dimension: maintainability
- Scope: `crates/shared/directories/src/lib.rs`
- Why it matters: path expansion, permission hardening, validation, and I/O helpers are now packed
  into one large file. Platform-specific path behavior is changed for one call site and
  unintentionally affects another because the implementation surface is no longer small.
