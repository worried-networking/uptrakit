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

- Create `crates/shared/types/src/config_test_kind.rs` following the one-file-per-type
  convention (matches `plugin_role.rs`, `plugin_capability.rs`, etc.). Add
  `mod config_test_kind; pub use config_test_kind::ConfigTestKind;` to `types/src/lib.rs`.
  The new file must carry a doc comment on the enum: `// WIRE TYPE — used in
  TestPluginConfigPayload; must follow the Other(String) catch-all pattern before new
  variants are added (see coding-standards.md).` This preserves the wire context that is
  otherwise lost once the definition moves out of payloads.rs.
- `wire/src/payloads.rs`: replace the inline definition with
  **`pub use uptrakit_shared_types::ConfigTestKind;`** (must be `pub use`, not bare `use` —
  wire's `lib.rs` re-exports `payloads` via `pub use payloads::*`, which only carries `pub`
  items; a private `use` would silently drop `ConfigTestKind` from the wire public API and
  break all 21 non-plugin dependents at compile time).
- `wire/src/lib.rs`: add `pub use uptrakit_shared_types::ConfigTestKind;` as an explicit
  belt-and-suspenders re-export. Redundant given the `pub use` in payloads, but clarifies
  intent: wire is a stable public facade for this type. Both paths resolve to the same type
  identity — no ambiguity or duplicate-import warnings for downstream callers.
- `wire/src/wire_validate_impls.rs`: **no changes needed.** It imports via `use crate::*`;
  `ConfigTestKind` arrives in scope through the re-export chain. The `TestPluginConfigPayload`
  validator does not branch on `test_kind`, so no logic is affected.

**`#[non_exhaustive]` ownership note:** all existing match sites are outside wire and
shared-types (in agent-core, web-api, etc.), so the ownership transfer does not change
`#[non_exhaustive]` exhaustiveness requirements at any call site. Wire's `tests.rs` uses
explicit variant arrays, not `match` statements — no test changes needed.

### 2. Update `infrastructure/core` Cargo.toml

- Remove `uptrakit-wire = { workspace = true }`.
- Add `uptrakit-surfaces = { workspace = true }` (`uptrakit-surfaces` is already a workspace
  dep — wire uses it — so no `workspace/Cargo.toml` change needed).
- `uptrakit-shared-types = { workspace = true }` already present; no change.

### 3. Update `infrastructure/core` source files

Five files, seven import sites. Required changes:

| File | Current import | Replacement |
| ---- | -------------- | ----------- |
| `src/lib.rs` | `pub use uptrakit_wire::ConfigTestKind;` | `pub use uptrakit_shared_types::ConfigTestKind;` |
| `src/lib.rs` | `pub use uptrakit_wire::surfaces;` | `pub use uptrakit_surfaces as surfaces;` |
| `src/descriptor.rs` | `use uptrakit_wire::{ConfigTestKind, surfaces};` | `use uptrakit_shared_types::ConfigTestKind; use uptrakit_surfaces as surfaces;` |
| `src/roles.rs` | `use uptrakit_wire::surfaces::{SurfaceActionRequest, SurfaceActionResponse};` | `use uptrakit_surfaces::{SurfaceActionRequest, SurfaceActionResponse};` |
| `src/plugin_ops.rs` | `use uptrakit_wire::surfaces;` | `use uptrakit_surfaces as surfaces;` |
| `src/catalog.rs` | `fn surface_registrations(...) -> Vec<uptrakit_wire::surfaces::SurfaceRegistration>` | `Vec<uptrakit_surfaces::SurfaceRegistration>` |
| `src/catalog.rs` (test mod) | `use uptrakit_wire::surfaces;` | `use uptrakit_surfaces as surfaces;` |

Start with `grep -r 'uptrakit_wire' crates/plugins/infrastructure/core/` to confirm no sites
were added since this spec was written.

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
`ConfigTestKind` will land correctly in the generated `shared_types` module. The `pub use`
in `payloads.rs` ensures the type remains visible in `generated/wire/payloads.rs` through
the existing glob re-export chain.

## Verification

Pure dep-graph refactor with no behavior changes. Quality gates:

```sh
cargo fmt --all
cargo check --all-features
cargo check --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo xtask sync-sdk --check
cargo xtask sync-openapi-client --check
cargo check -p uptrakit-service-sdk --all-features
cargo check -p uptrakit-openapi-client --all-features
cargo deny check
```

## Out of Scope

- Removing wire's `surfaces.rs` re-export barrel (would touch 10+ non-plugin crates for no
  plugin-isolation benefit).
- Creating `uptrakit-extension-framework` (adds indirection with no payoff today; sentrux
  rules are already staged for it when the time comes).
- Adding `Other(String)` catch-all to `ConfigTestKind` — the enum currently has
  `#[non_exhaustive]` but no wire-safe catch-all variant, which is a pre-existing issue.
  Fix separately; not part of this dep-graph refactor. Note: fix before the type gains wider
  direct consumers (plugin crates depending on shared-types directly).
