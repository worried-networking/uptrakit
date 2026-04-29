# Design: Extract Surfaces from Wire

**Date:** 2026-04-29
**Status:** Approved

## Goal

Remove `uptrakit-wire` as a dependency from all plugin crates. Plugins should depend on
`uptrakit-surfaces` (for surface protocol types) and `uptrakit-shared-types` (for plugin
descriptor types like `ConfigTestKind`) directly. Wire keeps re-exporting both so its 21
other dependents are unaffected.

## Background

`uptrakit-surfaces` already exists as a standalone crate. `wire/src/surfaces.rs` is a 6-line
barrel (`pub use uptrakit_surfaces::*`). The only plugin crate currently depending on wire is
`crates/plugins/infrastructure/core`, which needs surfaces types and `ConfigTestKind`. All
other plugin crates access `ConfigTestKind` through `infrastructure/core`'s re-export
(`pub use uptrakit_wire::ConfigTestKind` in `lib.rs`), not directly from wire.

The Sentrux boundary rule already forbids plugins from depending on wire, but its `reason`
field directs plugins toward `crates/shared/extension-framework` — a planned crate that does
not yet exist.

## Dependency Graph

**Before:**

```text
plugins/infrastructure/core → uptrakit-wire
                              (wire dep needed for surfaces types + ConfigTestKind)
uptrakit-wire               → uptrakit-surfaces
                            → uptrakit-shared-types
                              (ConfigTestKind defined directly in wire/src/payloads.rs)
```

**After:**

```text
plugins/infrastructure/core → uptrakit-surfaces      (direct)
                            → uptrakit-shared-types   (ConfigTestKind moves here; already a dep)

uptrakit-wire               → uptrakit-surfaces       (unchanged)
                            → uptrakit-shared-types   (imports + re-exports ConfigTestKind)
```

## Changes

### 1. Move `ConfigTestKind` to `uptrakit-shared-types`

`ConfigTestKind` is defined in `wire/src/payloads.rs` as the discriminant for
`TestPluginConfigPayload`. It travels over the wire, but it is also used in plugin
descriptors — making it a shared concept that belongs in `uptrakit-shared-types` alongside
`PluginTypeId` and `PluginRole`, accessible to both wire and plugin crates without the plugin
layer needing to depend on wire.

- Move the `ConfigTestKind` enum definition into `uptrakit-shared-types`.
- `wire/src/payloads.rs`: replace inline definition with `use uptrakit_shared_types::ConfigTestKind;`
- `wire/src/lib.rs`: add `pub use uptrakit_shared_types::ConfigTestKind;` to preserve the
  existing `uptrakit_wire::ConfigTestKind` path for all non-plugin callers.

### 2. Update `infrastructure/core` Cargo.toml

- Remove `uptrakit-wire = { workspace = true }`.
- Add `uptrakit-surfaces = { workspace = true }` (`uptrakit-surfaces` is already a workspace
  dep — wire uses it — so no `workspace/Cargo.toml` change needed).
- `uptrakit-shared-types = { workspace = true }` already present; no change.

### 3. Update `infrastructure/core` source files

Five files import from `uptrakit_wire`. Required changes:

| File | Current import | Replacement |
| ---- | -------------- | ----------- |
| `src/lib.rs` | `pub use uptrakit_wire::ConfigTestKind;` | `pub use uptrakit_shared_types::ConfigTestKind;` |
| `src/lib.rs` | `pub use uptrakit_wire::surfaces;` | `pub use uptrakit_surfaces as surfaces;` |
| `src/descriptor.rs` | `use uptrakit_wire::{ConfigTestKind, surfaces};` | `use uptrakit_shared_types::ConfigTestKind; use uptrakit_surfaces as surfaces;` |
| `src/roles.rs` | `use uptrakit_wire::surfaces::{SurfaceActionRequest, SurfaceActionResponse};` | `use uptrakit_surfaces::{SurfaceActionRequest, SurfaceActionResponse};` |
| `src/plugin_ops.rs` | `use uptrakit_wire::surfaces;` | `use uptrakit_surfaces as surfaces;` |
| `src/catalog.rs` | `fn surface_registrations(...) -> Vec<uptrakit_wire::surfaces::SurfaceRegistration>` | `Vec<uptrakit_surfaces::SurfaceRegistration>` |
| `src/catalog.rs` (test mod) | `use uptrakit_wire::surfaces;` | `use uptrakit_surfaces as surfaces;` |

No changes needed to any other plugin crate — they access `ConfigTestKind` and surfaces only
through `infrastructure/core`'s re-exports.

### 4. Update Sentrux boundary rule

Update only the `reason` field of the existing plugins→wire boundary rule. Do not touch the
`extension-framework` boundary rules — those are valid preemptive rules for when that crate
is created.

```toml
[[boundaries]]
from = "crates/plugins/**"
to = "crates/shared/wire/**"
reason = "Plugins must not depend on the wire protocol — depend on uptrakit-surfaces for surface types and uptrakit-shared-types for plugin descriptor types instead."
```

### 5. Regenerate sync artifacts

After the move, re-run:

```sh
cargo xtask sync-sdk --commit
cargo xtask sync-openapi-client --commit
```

No logic changes to the xtask sync code are needed. Both tools already have the
`uptrakit_shared_types → crate::generated::shared_types` path-rewrite rule, so
`ConfigTestKind` will land correctly in the generated `shared_types` module.

## Verification

Pure dep-graph refactor with no behavior changes. Quality gates:

```sh
cargo fmt --all
cargo check --all-features
cargo check --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo xtask sync-sdk --check
cargo xtask sync-openapi-client --check
cargo deny check
```

## Out of Scope

- Removing wire's `surfaces.rs` re-export barrel (would touch 10+ non-plugin crates for no
  plugin-isolation benefit).
- Creating `uptrakit-extension-framework` (adds indirection with no payoff today; sentrux
  rules are already staged for it when the time comes).
- Adding `Other(String)` catch-all to `ConfigTestKind` — the enum currently has
  `#[non_exhaustive]` but no wire-safe catch-all variant, which is a pre-existing issue.
  Fix separately; not part of this dep-graph refactor.
