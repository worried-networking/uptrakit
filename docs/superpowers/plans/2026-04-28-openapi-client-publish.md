# `uptrakit-openapi-client` Self-Containment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove all 3 workspace path dependencies from `uptrakit-openapi-client` and set
`publish = true` so it can be published to crates.io. Static snapshot approach — no sync
machinery. Will be replaced by a progenitor-based rewrite later.

**Architecture:** (A) Inline both shared macros into `src/macros.rs`; remove
`uptrakit-shared-macros`. (B) Extend the xtask (from the service-sdk plan) with a
`sync-openapi-client` command that copies `uptrakit-web-api-types` (and its internal deps:
wire, surfaces, shared-types) as a static snapshot under `src/generated/`; rewrite all
internal crate paths using syn. (C) Update the 38 endpoint modules to use new import paths;
remove `uptrakit-shared-types` and `uptrakit-web-api-types` from `Cargo.toml`; set
`publish = true`.

**Prerequisite:** The service-sdk plan (`2026-04-28-service-sdk-publish.md`) Task 1 must be
complete — the `xtask` crate must exist with `collect_module`, `write_output`, and `run_check`
defined in `sync_sdk.rs`.

**Tech Stack:** Rust, `syn` (AST path rewriting), `prettyplease`, `walkdir`, `anyhow` (all
already in xtask from service-sdk plan).

---

## Context

`uptrakit-openapi-client` lives at `crates/shared/openapi-client/`. The client is hand-written:
38 typed `impl UptrakitClient` endpoint modules, each importing directly from
`uptrakit_web_api_types::some_module::SomeType`. There is no progenitor or other codegen
involved — this is a deliberate hand-authored client.

Current workspace path deps to eliminate:

| Dep | How used | Resolution |
| --- | --- | --- |
| `uptrakit-shared-macros` | `impl_report_conversion!` in `error.rs`; `wire_safe_enum!` in `permissions.rs` | Inline both macros in `src/macros.rs` |
| `uptrakit-shared-types` | Many types re-exported from web-api-types lib.rs; `DeviceAuthStatus` in lib.rs | Included in the web-api-types copy (web-api-types re-exports them; after codegen they resolve via `crate::generated::shared_types::`) |
| `uptrakit-web-api-types` | `pub use uptrakit_web_api_types as types` in lib.rs; all 38 endpoint modules | Copy source snapshot into `src/generated/types/` via xtask |

`uptrakit-web-api-types` itself has 3 internal deps: `uptrakit-wire`, `uptrakit-shared-types`,
`uptrakit-shared-macros`. The xtask copies all four into `src/generated/` with syn path rewrites.

**This is a temporary snapshot.** When the progenitor rewrite lands, `src/generated/` is
replaced wholesale and the xtask command is removed or repurposed.

---

## File Structure

**Create:**

- `crates/shared/openapi-client/src/macros.rs` — both `impl_report_conversion!` and
  `wire_safe_enum!`
- `xtask/src/sync_openapi_client.rs` — new xtask subcommand
- `crates/shared/openapi-client/src/generated/` — written by xtask:
  - `generated/mod.rs`
  - `generated/surfaces/` — surfaces source copy
  - `generated/wire/` — wire source copy
  - `generated/shared_types/` — shared-types source copy
  - `generated/types/` — web-api-types source copy

**Modify:**

- `xtask/src/sync_sdk.rs` — make `collect_module`, `write_output`, `run_check` `pub(crate)`
- `xtask/src/main.rs` — add `SyncOpenapiClient` subcommand
- `crates/shared/openapi-client/Cargo.toml` — remove 3 path deps; add `openapi = []` feature;
  add missing direct deps; set `publish = true`
- `crates/shared/openapi-client/src/lib.rs` — update `types` re-export; add `mod generated`;
  update `DeviceAuthStatus` re-export; update `fetch_all_pages` pagination import
- `crates/shared/openapi-client/src/error.rs` — remove shared-macros import
- All 38 endpoint modules — `use uptrakit_web_api_types::` → `use crate::generated::types::`

---

### Task 1: Inline macros; add `openapi` feature

**Files:**

- Create: `crates/shared/openapi-client/src/macros.rs`
- Modify: `crates/shared/openapi-client/src/lib.rs`
- Modify: `crates/shared/openapi-client/src/error.rs`
- Modify: `crates/shared/openapi-client/Cargo.toml`

- [ ] **Step 1: Find all shared-macros usages**

```bash
grep -rn "uptrakit_shared_macros" crates/shared/openapi-client/src/
```

Expected: `error.rs` uses `impl_report_conversion!`; one other file uses `wire_safe_enum!`.
Note the exact file names.

- [ ] **Step 2: Create `src/macros.rs` with both macros**

Create `crates/shared/openapi-client/src/macros.rs`. Copy the following from
`crates/shared/macros/src/lib.rs`:

1. The entire `macro_rules! impl_report_conversion { ... }` block (lines 95–148 verbatim).
2. The entire `macro_rules! wire_safe_enum { ... }` block (lines 195–284 verbatim).

Do not include any doc comments, module-level comments, or `#[macro_export]` attributes —
only the two `macro_rules!` blocks themselves.

- [ ] **Step 3: Add `#[macro_use] mod macros;` to `lib.rs`**

In `crates/shared/openapi-client/src/lib.rs`, add as the very first line:

```rust
#[macro_use]
mod macros;
```

- [ ] **Step 4: Remove shared-macros imports from call sites**

In each file that has `use uptrakit_shared_macros::...;` (found in Step 1), remove that
`use` statement. The macros are now in scope crate-wide via `#[macro_use]`.

- [ ] **Step 5: Add `openapi = []` feature and remove `uptrakit-shared-macros` dep**

In `crates/shared/openapi-client/Cargo.toml`:

Add to `[features]`:

```toml
openapi = []
```

This makes `cfg(feature = "openapi")` a known (but false by default) feature, suppressing
`unexpected_cfgs` lint warnings from the copied web-api-types source files which carry
`#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]` attributes.

Remove from `[dependencies]`:

```toml
uptrakit-shared-macros = { workspace = true }  # remove
```

- [ ] **Step 6: Verify**

```bash
cargo check -p uptrakit-openapi-client
```

Expected: clean. If `macro not found` errors appear, verify `#[macro_use]` placement.

- [ ] **Step 7: Commit**

```bash
git add crates/shared/openapi-client/
git commit -m "feat(openapi-client): inline impl_report_conversion! and wire_safe_enum! macros"
```

---

### Task 2: Add `sync-openapi-client` to xtask

**Files:**

- Modify: `xtask/src/sync_sdk.rs`
- Create: `xtask/src/sync_openapi_client.rs`
- Modify: `xtask/src/main.rs`

**Background:** `sync-openapi-client` copies four internal crate source trees into
`openapi-client/src/generated/` with path rewrites:

| Source | Destination | Path rewrites |
| --- | --- | --- |
| `crates/shared/surfaces/src/` | `generated/surfaces/` | `crate::` → `crate::generated::surfaces::` |
| `crates/shared/wire/src/` | `generated/wire/` | `crate::` → `crate::generated::wire::` (first); `uptrakit_surfaces::` → `crate::generated::surfaces::`; `uptrakit_shared_types::` → `crate::generated::shared_types::` |
| `crates/shared/types/src/` | `generated/shared_types/` | `crate::` → `crate::generated::shared_types::` |
| `crates/shared/web-api-types/src/` | `generated/types/` | `crate::` → `crate::generated::types::` (first); `uptrakit_wire::` → `crate::generated::wire::`; `uptrakit_shared_types::` → `crate::generated::shared_types::`; `uptrakit_surfaces::` → `crate::generated::surfaces::` |

After syn rewriting, two string-literal fixes are applied (serde `default` attribute paths
cannot be fixed by syn):

- `"crate::default_enabled"` → `"crate::generated::types::default_enabled"`
- `"crate::default_featured"` → `"crate::generated::types::default_featured"`

And `use uptrakit_shared_macros::...` import lines are stripped (macro is in scope via
`#[macro_use] mod macros` in the openapi-client root).

- [ ] **Step 1: Make sync_sdk helpers `pub(crate)`**

In `xtask/src/sync_sdk.rs`, change the visibility of:

```rust
pub(crate) fn collect_module(...) -> Result<()> { ... }
pub(crate) fn write_output(...) -> Result<()> { ... }
pub(crate) fn run_check(...) -> Result<()> { ... }
```

(They are currently `fn` or private. Add `pub(crate)` to all three.)

- [ ] **Step 2: Create `xtask/src/sync_openapi_client.rs`**

```rust
use anyhow::Result;
use std::{fs, path::Path, path::PathBuf};

use crate::sync_sdk::{collect_module, run_check, write_output};

pub fn run(workspace_root: &Path, check: bool) -> Result<()> {
    let surfaces_src = workspace_root.join("crates/shared/surfaces/src");
    let wire_src = workspace_root.join("crates/shared/wire/src");
    let shared_types_src = workspace_root.join("crates/shared/types/src");
    let web_api_types_src = workspace_root.join("crates/shared/web-api-types/src");
    let out_root = workspace_root
        .join("crates/shared/openapi-client/src/generated");

    let mut output: Vec<(PathBuf, String)> = Vec::new();

    // 1. Surfaces — rewrite self-references only
    collect_module(
        &surfaces_src,
        &out_root.join("surfaces"),
        &[(&["crate"], &["crate", "generated", "surfaces"])],
        &mut output,
    )?;

    // 2. Wire — self-references first, then external deps (order matters: crate:: rule
    //    must run first so it doesn't corrupt paths produced by later rules)
    collect_module(
        &wire_src,
        &out_root.join("wire"),
        &[
            (&["crate"], &["crate", "generated", "wire"]),
            (&["uptrakit_surfaces"], &["crate", "generated", "surfaces"]),
            (&["uptrakit_shared_types"], &["crate", "generated", "shared_types"]),
        ],
        &mut output,
    )?;

    // 3. Shared-types — rewrite self-references only (no internal path deps)
    collect_module(
        &shared_types_src,
        &out_root.join("shared_types"),
        &[(&["crate"], &["crate", "generated", "shared_types"])],
        &mut output,
    )?;

    // 4. Web-api-types — self-references first, then external deps (order matters: crate::
    //    rule must run first so it doesn't corrupt paths produced by external dep rules)
    collect_module(
        &web_api_types_src,
        &out_root.join("types"),
        &[
            (&["crate"], &["crate", "generated", "types"]),
            (&["uptrakit_wire"], &["crate", "generated", "wire"]),
            (&["uptrakit_shared_types"], &["crate", "generated", "shared_types"]),
            (&["uptrakit_surfaces"], &["crate", "generated", "surfaces"]),
        ],
        &mut output,
    )?;

    // 5. Top-level generated/mod.rs
    output.push((
        out_root.join("mod.rs"),
        "pub mod shared_types;\npub mod surfaces;\npub mod types;\npub mod wire;\n".to_string(),
    ));

    // 6. Fix serde default string literals (syn cannot touch string attributes)
    for (_, content) in &mut output {
        *content = content
            .replace(
                "\"crate::default_enabled\"",
                "\"crate::generated::types::default_enabled\"",
            )
            .replace(
                "\"crate::default_featured\"",
                "\"crate::generated::types::default_featured\"",
            );
    }

    // 7. Strip uptrakit_shared_macros use-imports (macro available via #[macro_use])
    for (_, content) in &mut output {
        let filtered: Vec<&str> = content
            .lines()
            .filter(|l| !l.contains("uptrakit_shared_macros"))
            .collect();
        *content = filtered.join("\n");
        if !content.ends_with('\n') {
            content.push('\n');
        }
    }

    if check {
        run_check(&output)
    } else {
        write_output(&out_root, &output)
    }
}
```

- [ ] **Step 3: Add `SyncOpenapiClient` subcommand to `xtask/src/main.rs`**

Add `mod sync_openapi_client;` at the top.

In the `Command` enum:

```rust
/// Copy web-api-types + internal deps into openapi-client src/generated/.
SyncOpenapiClient {
    /// Exit non-zero if any generated file would change (CI / pre-commit).
    #[arg(long)]
    check: bool,
},
```

In the `match`:

```rust
Command::SyncOpenapiClient { check } => {
    sync_openapi_client::run(&workspace_root, check)?
}
```

- [ ] **Step 4: Verify xtask compiles**

```bash
cargo build -p xtask
```

Expected: clean build.

- [ ] **Step 5: Commit**

```bash
git add xtask/
git commit -m "feat(xtask): add sync-openapi-client command (web-api-types snapshot)"
```

---

### Task 3: Run codegen; update imports; remove remaining deps

**Files:**

- Create: `crates/shared/openapi-client/src/generated/` (xtask writes these)
- Modify: `crates/shared/openapi-client/src/lib.rs`
- Modify: `crates/shared/openapi-client/Cargo.toml`
- Modify: all 38 endpoint modules

- [ ] **Step 1: Run the codegen**

```bash
cargo xtask sync-openapi-client
```

Expected: files written under `crates/shared/openapi-client/src/generated/`.

Verify the four subdirectories exist:

```bash
ls crates/shared/openapi-client/src/generated/
# Expected: mod.rs  shared_types/  surfaces/  types/  wire/
```

- [ ] **Step 2: Add `pub mod generated;` to `lib.rs`**

In `crates/shared/openapi-client/src/lib.rs`, add:

```rust
pub mod generated;
```

- [ ] **Step 3: Update the `types` re-export in `lib.rs`**

Find:

```rust
pub use uptrakit_web_api_types as types;
```

Replace with:

```rust
pub use generated::types as types;
```

- [ ] **Step 4: Update the `DeviceAuthStatus` re-export in `lib.rs`**

Find:

```rust
pub use uptrakit_shared_types::DeviceAuthStatus;
```

Replace with:

```rust
pub use generated::shared_types::DeviceAuthStatus;
```

`DeviceAuthStatus` lives in shared_types, not re-exported by web-api-types lib.rs, so reference
`generated::shared_types` directly instead of going through `generated::types`.

- [ ] **Step 5: Update `fetch_all_pages` in `lib.rs`**

Find the import inside `fetch_all_pages`:

```rust
use uptrakit_web_api_types::pagination::{MAX_PER_PAGE, PaginatedResponse};
```

Replace with:

```rust
use crate::generated::types::pagination::{MAX_PER_PAGE, PaginatedResponse};
```

- [ ] **Step 6: First compile attempt — collect all remaining errors**

```bash
cargo check -p uptrakit-openapi-client 2>&1 | grep "^error" | head -40
```

There will be errors in the 38 endpoint modules about `uptrakit_web_api_types::` paths.
Note the count. Do NOT fix them yet — collect all errors first.

- [ ] **Step 7: Update all 38 endpoint modules**

In every file under `crates/shared/openapi-client/src/` that contains
`use uptrakit_web_api_types::`, replace:

```rust
use uptrakit_web_api_types::
```

with:

```rust
use crate::generated::types::
```

This is a mechanical, project-wide substitution. Run:

```bash
# macOS
find crates/shared/openapi-client/src -name "*.rs" \
  ! -name "lib.rs" \
  -exec sed -i '' 's/use uptrakit_web_api_types::/use crate::generated::types::/g' {} +
# Verify
grep -r "uptrakit_web_api_types" crates/shared/openapi-client/src/
```

Expected: no remaining occurrences of `uptrakit_web_api_types` in endpoint modules.

- [ ] **Step 8: Add missing direct deps to `Cargo.toml`**

The generated code pulls in types from wire, surfaces, and shared-types. Add any deps not
already present in `crates/shared/openapi-client/Cargo.toml`:

```toml
strum = { workspace = true }
zeroize = { workspace = true }
url = { workspace = true }
time = { workspace = true, features = ["serde"] }
```

Verify each is in `[workspace.dependencies]` in root `Cargo.toml`.

- [ ] **Step 9: Remove `uptrakit-shared-types` and `uptrakit-web-api-types` deps**

In `crates/shared/openapi-client/Cargo.toml`:

```toml
uptrakit-shared-types = { workspace = true }   # remove
uptrakit-web-api-types = { workspace = true }  # remove
```

- [ ] **Step 10: Full compile + test**

```bash
cargo check -p uptrakit-openapi-client
cargo check -p uptrakit-openapi-client --features mock,tracing
cargo test -p uptrakit-openapi-client --features mock
```

If any errors remain:

- `use crate::generated::types::X` not found → the type may come from
  `crate::generated::shared_types::` (e.g. `PluginCapability`, `PluginTypeId`).
  Check the generated `types/mod.rs` — if the type is NOT re-exported there, add
  the re-export: `pub use crate::generated::shared_types::PluginCapability;`
- Missing dep → add to `Cargo.toml`
- Macro expansion error in `wire_safe_enum!` or `impl_report_conversion!` →
  verify `#[macro_use] mod macros;` is the first item in `lib.rs`

Fix any remaining errors and re-run until all three commands pass.

- [ ] **Step 11: Commit**

```bash
git add crates/shared/openapi-client/ xtask/ Cargo.toml Cargo.lock
git commit -m "feat(openapi-client): snapshot web-api-types into generated/; remove workspace path deps"
```

---

### Task 4: Set `publish = true`; verify `cargo publish --dry-run`

**Files:**

- Modify: `crates/shared/openapi-client/Cargo.toml`

- [ ] **Step 1: Add `publish = true`**

In `crates/shared/openapi-client/Cargo.toml`, add to `[package]`:

```toml
publish = true
```

The `description` field is already present (`"Typed HTTP client for the Uptrakit web API"`).

- [ ] **Step 2: Verify no workspace path deps remain**

```bash
cargo metadata --format-version 1 | python3 -c "
import sys, json
data = json.load(sys.stdin)
pkg = next(p for p in data['packages'] if p['name'] == 'uptrakit-openapi-client')
path_deps = [
    d['name'] for d in pkg['dependencies']
    if d['name'].startswith('uptrakit-')
    and any(p['name'] == d['name'] for p in data['packages'])
]
print(path_deps or 'NONE')
"
```

Expected: `NONE`.

- [ ] **Step 3: Run `cargo publish --dry-run`**

```bash
cargo publish -p uptrakit-openapi-client --dry-run --allow-dirty
```

Expected: packaging succeeds with no errors about unpublishable path deps or missing fields.

- [ ] **Step 4: Commit**

```bash
git add crates/shared/openapi-client/Cargo.toml
git commit -m "feat(openapi-client): set publish = true"
```
