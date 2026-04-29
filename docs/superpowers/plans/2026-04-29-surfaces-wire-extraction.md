# Surfaces Wire Extraction Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove `uptrakit-wire` as a dependency from plugin crates by moving `ConfigTestKind`
to `uptrakit-shared-types` and giving `infrastructure/core` a direct `uptrakit-surfaces` dep.

**Architecture:** `ConfigTestKind` moves to a new `config_test_kind.rs` file in shared-types
(one-file-per-type convention). Wire gains a `pub use uptrakit_shared_types::ConfigTestKind`
re-export so all 21 existing wire dependents are unaffected. `infrastructure/core` drops
`uptrakit-wire`, adds `uptrakit-surfaces`.

**Tech Stack:** Rust, Cargo workspaces, `cargo xtask sync-sdk`,
`cargo xtask sync-openapi-client`, Sentrux (`.sentrux/rules.toml`)

---

## Task 1: Add `ConfigTestKind` to `uptrakit-shared-types`

**Files:**

- Create: `crates/shared/types/src/config_test_kind.rs`
- Modify: `crates/shared/types/src/lib.rs`

- [ ] **Step 1: Write the new file with tests**

Create `crates/shared/types/src/config_test_kind.rs`:

```rust
use serde::{Deserialize, Serialize};

// WIRE TYPE — used in TestPluginConfigPayload (uptrakit-wire); must follow the
// Other(String) catch-all pattern before new variants are added (see coding-standards.md).
/// The kind of configuration test to perform on the agent.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigTestKind {
    /// Execute `detect_installed_version()` and return output + detected version.
    VersionDetection,
    /// Validate update_command syntax (sh -n check, do NOT execute).
    UpdateCommandValidation,
    /// Execute pre-update hook with mock context.
    PreUpdateHook,
    /// Execute post-update hook with mock context.
    PostUpdateHook,
    /// Test connectivity for controller-side plugins (`fetch_releases`).
    Connectivity,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_detection_roundtrip() {
        let json = serde_json::to_string(&ConfigTestKind::VersionDetection).unwrap();
        assert_eq!(json, r#""version_detection""#);
        let back: ConfigTestKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ConfigTestKind::VersionDetection);
    }

    #[test]
    fn update_command_validation_roundtrip() {
        let json = serde_json::to_string(&ConfigTestKind::UpdateCommandValidation).unwrap();
        assert_eq!(json, r#""update_command_validation""#);
        let back: ConfigTestKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ConfigTestKind::UpdateCommandValidation);
    }

    #[test]
    fn pre_update_hook_roundtrip() {
        let json = serde_json::to_string(&ConfigTestKind::PreUpdateHook).unwrap();
        assert_eq!(json, r#""pre_update_hook""#);
        let back: ConfigTestKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ConfigTestKind::PreUpdateHook);
    }

    #[test]
    fn post_update_hook_roundtrip() {
        let json = serde_json::to_string(&ConfigTestKind::PostUpdateHook).unwrap();
        assert_eq!(json, r#""post_update_hook""#);
        let back: ConfigTestKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ConfigTestKind::PostUpdateHook);
    }

    #[test]
    fn connectivity_roundtrip() {
        let json = serde_json::to_string(&ConfigTestKind::Connectivity).unwrap();
        assert_eq!(json, r#""connectivity""#);
        let back: ConfigTestKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ConfigTestKind::Connectivity);
    }
}
```

- [ ] **Step 2: Register the module in `crates/shared/types/src/lib.rs`**

Add after line `pub mod command_validation;` (alphabetical order: `config` comes after `command`):

```rust
mod config_test_kind;
```

Add to the `pub use` block, after `pub use batch_status::{BatchStatus, ParseBatchStatusError};`:

```rust
pub use config_test_kind::ConfigTestKind;
```

- [ ] **Step 3: Run the new tests to verify they pass**

```sh
cargo test -p uptrakit-shared-types config_test_kind
```

Expected: 5 tests pass, 0 fail.

- [ ] **Step 4: Commit**

```sh
git add crates/shared/types/src/config_test_kind.rs crates/shared/types/src/lib.rs
git commit -m "feat(shared-types): add ConfigTestKind"
```

---

## Task 2: Wire — replace inline `ConfigTestKind` with `pub use`

**Files:**

- Modify: `crates/shared/wire/src/payloads.rs`
- Modify: `crates/shared/wire/src/lib.rs`

- [ ] **Step 1: In `payloads.rs`, replace the inline enum definition**

Find and remove this block (lines 1640–1657):

```rust
// ── Config test payloads ─────────────────────────────────────────────────────

/// The kind of configuration test to perform on the agent.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigTestKind {
    /// Execute `detect_installed_version()` and return output + detected version.
    VersionDetection,
    /// Validate update_command syntax (sh -n check, do NOT execute).
    UpdateCommandValidation,
    /// Execute pre-update hook with mock context.
    PreUpdateHook,
    /// Execute post-update hook with mock context.
    PostUpdateHook,
    /// Test connectivity for controller-side plugins (`fetch_releases`).
    Connectivity,
}
```

Replace with:

```rust
// ── Config test payloads ─────────────────────────────────────────────────────

// Must be `pub use`, not bare `use` — wire's lib.rs does `pub use payloads::*`,
// which only re-exports *pub* items. A private import here would silently drop
// ConfigTestKind from the wire public API and break all 21 non-plugin dependents.
pub use uptrakit_shared_types::ConfigTestKind;
```

- [ ] **Step 2: In `wire/src/lib.rs`, add belt-and-suspenders explicit re-export**

Find the existing shared-types re-export block:

```rust
// Re-export shared types used directly in wire protocol messages.
pub use uptrakit_shared_types::{
    AttestationStatus, DiscoveredSoftware, DiscoveryTarget, HookShell, OutputStreamType,
    PluginRole, PluginTypeId, ReleaseAsset, ReleaseInfo, UpdateCategory, plugin_ids,
};
```

Add `ConfigTestKind` to the list (alphabetical: after `AttestationStatus`):

```rust
// Re-export shared types used directly in wire protocol messages.
pub use uptrakit_shared_types::{
    AttestationStatus, ConfigTestKind, DiscoveredSoftware, DiscoveryTarget, HookShell,
    OutputStreamType, PluginRole, PluginTypeId, ReleaseAsset, ReleaseInfo, UpdateCategory,
    plugin_ids,
};
```

Both paths (`uptrakit_wire::payloads::ConfigTestKind` via glob and `uptrakit_wire::ConfigTestKind`
via this line) resolve to the same type — no ambiguity for downstream callers.

`wire/src/wire_validate_impls.rs` imports via `use crate::*` and does not branch on
`test_kind` — no changes needed there.

- [ ] **Step 3: Verify wire compiles and existing tests still pass**

```sh
cargo test -p uptrakit-wire --all-features
```

Expected: all tests pass. Wire roundtrip tests cover `TestPluginConfigPayload` with
`ConfigTestKind` — these confirm the re-export chain is intact.

- [ ] **Step 4: Commit**

```sh
git add crates/shared/wire/src/payloads.rs crates/shared/wire/src/lib.rs
git commit -m "refactor(wire): move ConfigTestKind to uptrakit-shared-types, re-export"
```

---

## Task 3: Update `infrastructure/core` Cargo.toml

**Files:**

- Modify: `crates/plugins/infrastructure/core/Cargo.toml`

- [ ] **Step 1: Confirm no new wire imports were added since spec was written**

```sh
grep -r 'uptrakit_wire' crates/plugins/infrastructure/core/
```

Expected output (exactly these 7 lines, no more):

```text
src/lib.rs:pub use uptrakit_wire::ConfigTestKind;
src/lib.rs:pub use uptrakit_wire::surfaces;
src/descriptor.rs:use uptrakit_wire::{ConfigTestKind, surfaces};
src/roles.rs:use uptrakit_wire::surfaces::{SurfaceActionRequest, SurfaceActionResponse};
src/plugin_ops.rs:use uptrakit_wire::surfaces;
src/catalog.rs:    fn surface_registrations(&self) -> Vec<uptrakit_wire::surfaces::SurfaceRegistration> {
src/catalog.rs:    use uptrakit_wire::surfaces;
```

If there are additional lines, update Task 4 accordingly before proceeding.

- [ ] **Step 2: Edit `Cargo.toml`**

Remove:

```toml
uptrakit-wire = { workspace = true }
```

Add (in the `[dependencies]` block, alphabetical after `uptrakit-notification-plugin-core`):

```toml
uptrakit-surfaces = { workspace = true }
```

(`uptrakit-surfaces` is already registered in `workspace/Cargo.toml` — no workspace changes needed.)

- [ ] **Step 3: Verify the crate fails to compile as expected (wire dep removed, imports not yet updated)**

```sh
cargo check -p uptrakit-plugin-infrastructure-core --all-features 2>&1 | head -20
```

Expected: compile errors referencing `uptrakit_wire` — confirms the dep was removed. Do not fix yet.

- [ ] **Step 4: Commit**

```sh
git add crates/plugins/infrastructure/core/Cargo.toml
git commit -m "refactor(infra-core): swap uptrakit-wire dep for uptrakit-surfaces"
```

---

## Task 4: Update `infrastructure/core` source files

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/lib.rs`
- Modify: `crates/plugins/infrastructure/core/src/descriptor.rs`
- Modify: `crates/plugins/infrastructure/core/src/roles.rs`
- Modify: `crates/plugins/infrastructure/core/src/plugin_ops.rs`
- Modify: `crates/plugins/infrastructure/core/src/catalog.rs`

Apply all 7 import replacements. Each change is a one-line edit.

- [ ] **Step 1: `src/lib.rs` — two re-export lines**

Replace:

```rust
pub use uptrakit_wire::ConfigTestKind;
pub use uptrakit_wire::surfaces;
```

With:

```rust
pub use uptrakit_shared_types::ConfigTestKind;
pub use uptrakit_surfaces as surfaces;
```

- [ ] **Step 2: `src/descriptor.rs` — combined import**

Replace:

```rust
use uptrakit_wire::{ConfigTestKind, surfaces};
```

With:

```rust
use uptrakit_shared_types::ConfigTestKind;
use uptrakit_surfaces as surfaces;
```

- [ ] **Step 3: `src/roles.rs` — cfg-gated surfaces import**

Replace:

```rust
use uptrakit_wire::surfaces::{SurfaceActionRequest, SurfaceActionResponse};
```

With:

```rust
use uptrakit_surfaces::{SurfaceActionRequest, SurfaceActionResponse};
```

(The surrounding `#[cfg(feature = "agent-infra")]` gate is unchanged.)

- [ ] **Step 4: `src/plugin_ops.rs` — surfaces module import**

Replace:

```rust
use uptrakit_wire::surfaces;
```

With:

```rust
use uptrakit_surfaces as surfaces;
```

- [ ] **Step 5: `src/catalog.rs` — inline path in `impl` block**

Replace:

```rust
fn surface_registrations(&self) -> Vec<uptrakit_wire::surfaces::SurfaceRegistration> {
```

With:

```rust
fn surface_registrations(&self) -> Vec<uptrakit_surfaces::SurfaceRegistration> {
```

- [ ] **Step 6: `src/catalog.rs` — surfaces import in test module**

Replace (inside the `#[cfg(test)]` block near the bottom of the file):

```rust
use uptrakit_wire::surfaces;
```

With:

```rust
use uptrakit_surfaces as surfaces;
```

- [ ] **Step 7: Verify `infrastructure/core` compiles and tests pass**

```sh
cargo test -p uptrakit-plugin-infrastructure-core --all-features
```

Expected: all tests pass, no `uptrakit_wire` references remaining.

Confirm zero remaining references:

```sh
grep -r 'uptrakit_wire' crates/plugins/infrastructure/core/
```

Expected: no output.

- [ ] **Step 8: Commit**

```sh
git add \
  crates/plugins/infrastructure/core/src/lib.rs \
  crates/plugins/infrastructure/core/src/descriptor.rs \
  crates/plugins/infrastructure/core/src/roles.rs \
  crates/plugins/infrastructure/core/src/plugin_ops.rs \
  crates/plugins/infrastructure/core/src/catalog.rs
git commit -m "refactor(infra-core): replace uptrakit_wire imports with uptrakit_surfaces / uptrakit_shared_types"
```

---

## Task 5: Update Sentrux boundary rule

**Files:**

- Modify: `.sentrux/rules.toml`

- [ ] **Step 1: Update the `reason` field**

Find (lines 1538–1541):

```toml
[[boundaries]]
from = "crates/plugins/**"
to = "crates/shared/wire/**"
reason = "Plugins must not depend on the wire protocol — use crates/shared/extension-framework directly"
```

Replace the `reason` value only — leave `from` and `to` unchanged:

```toml
[[boundaries]]
from = "crates/plugins/**"
to = "crates/shared/wire/**"
reason = "Plugins must not depend on the wire protocol — depend on uptrakit-surfaces for surface types and uptrakit-shared-types for plugin descriptor types instead."
```

Do **not** touch any of the `extension-framework` boundary rules — those are valid
preemptive rules for a planned crate.

- [ ] **Step 2: Commit**

```sh
git add .sentrux/rules.toml
git commit -m "chore(sentrux): update plugins->wire boundary rule reason to name real alternatives"
```

---

## Task 6: Regenerate sync artifacts and final verification

**Files:**

- Auto-modified: `crates/shared/service-sdk/src/generated/`
- Auto-modified: `crates/shared/openapi-client/src/generated/`

- [ ] **Step 1: Regenerate service-sdk**

```sh
cargo xtask sync-sdk
```

Expected: writes updated generated files. `ConfigTestKind` now appears in
`generated/shared_types/config_test_kind.rs` (copied from shared-types source).
`generated/wire/payloads.rs` imports it via the rewritten
`crate::generated::shared_types::ConfigTestKind` path.

- [ ] **Step 2: Verify service-sdk is correct**

```sh
cargo xtask sync-sdk --check
```

Expected: "all generated files up to date" — no diff.

- [ ] **Step 3: Regenerate openapi-client**

```sh
cargo xtask sync-openapi-client
```

Expected: same pattern as service-sdk.

- [ ] **Step 4: Verify openapi-client is correct**

```sh
cargo xtask sync-openapi-client --check
```

Expected: "all generated files up to date".

- [ ] **Step 5: Run full quality gates**

```sh
cargo fmt --all
cargo check --all-features
cargo check --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo check -p uptrakit-service-sdk --all-features
cargo check -p uptrakit-openapi-client --all-features
cargo deny check
```

All must pass. If `cargo test --all-features` runs any `#[ignore]` tests by accident,
that is a pre-existing configuration issue — the gates above do not pass `--include-ignored`
so Docker-gated integration tests will not run.

- [ ] **Step 6: Commit generated files**

```sh
git add \
  crates/shared/service-sdk/src/generated/ \
  crates/shared/openapi-client/src/generated/
git commit -m "chore(generated): regenerate sdk and openapi-client after ConfigTestKind move"
```
