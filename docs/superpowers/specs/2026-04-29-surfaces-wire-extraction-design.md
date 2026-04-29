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
other plugin crates access `ConfigTestKind` through `infrastructure/core`'s re-export, not
directly from wire.

The Sentrux boundary rule already forbids plugins from depending on wire, but its `reason`
field references a non-existent `extension-framework` crate.

## Dependency Graph

**Before:**

```text
plugins/infrastructure/core → uptrakit-wire
uptrakit-wire               → uptrakit-surfaces
                            → uptrakit-shared-types  (ConfigTestKind defined in wire/payloads.rs)
```

**After:**

```text
plugins/infrastructure/core → uptrakit-surfaces      (direct)
                            → uptrakit-shared-types   (ConfigTestKind, already present)

uptrakit-wire               → uptrakit-surfaces       (unchanged)
                            → uptrakit-shared-types   (imports + re-exports ConfigTestKind)
```

## Changes

### 1. Move `ConfigTestKind` to `uptrakit-shared-types`

`ConfigTestKind` is a plugin descriptor concept (which config-test variants a plugin supports),
not a wire protocol concept. It lives in `wire/src/payloads.rs` today only because the wire
`TestPluginConfigPayload` struct references it.

- Move the enum definition into `uptrakit-shared-types` alongside `PluginTypeId`, `PluginRole`.
- `wire/src/payloads.rs` imports it: `use uptrakit_shared_types::ConfigTestKind;`
- Wire crate root adds `pub use uptrakit_shared_types::ConfigTestKind;` to preserve the
  existing `uptrakit_wire::ConfigTestKind` path for all non-plugin callers.

### 2. Update `infrastructure/core` Cargo.toml

- Remove `uptrakit-wire` dependency.
- Add `uptrakit-surfaces = { workspace = true }` if not already present.
- `uptrakit-shared-types` is already a dependency — no change needed there.

### 3. Update `infrastructure/core` import paths

All `uptrakit_wire::surfaces::` references become `uptrakit_surfaces::`. No other plugin
crate changes are needed — they already access `ConfigTestKind` through `infrastructure/core`.

### 4. Update Sentrux boundary rule

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
`ConfigTestKind` will land correctly in the generated shared_types module.

## Verification

This is a pure dep-graph refactor with no behavior changes. Quality gates:

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
- Creating a new `uptrakit-extension-framework` crate (adds indirection with no payoff today).
