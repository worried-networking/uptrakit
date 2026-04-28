# `uptrakit-service-sdk` Self-Containment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove all 8 workspace path dependencies from `uptrakit-service-sdk` and set
`publish = true` so it can be published to crates.io.

**Architecture:** Four independent streams that must land in order: (A) inline five small utility
crates directly into service-sdk (`backoff`, `build-info`, `directories`, `tracing-init`,
`shared-macros`); (B) gate the crypto dep behind a new `sensitive-params` feature; (C) create an
xtask crate with a `sync-sdk` command that generates `src/generated/` from `uptrakit-wire` and
`uptrakit-surfaces` source using syn path-rewriting, then update all import sites; (D) set
`publish = true` and add CI + pre-commit guards.

**Tech Stack:** Rust, `syn` (AST parsing), `proc-macro2` + `quote` (token emission), `anyhow`
(xtask error handling), `walkdir` (directory traversal), `prettyplease` (rustfmt in-process).

---

## Context

`uptrakit-service-sdk` lives at `crates/shared/service-sdk/`. It currently has these workspace path
deps that prevent crates.io publication:

| Dep | How used | Resolution |
| --- | --- | --- |
| `uptrakit-backoff` | `pub use uptrakit_backoff::Backoff` in `lib.rs` | Inline `Backoff` struct in `src/backoff.rs` |
| `uptrakit-build-info` | `BuildInfo::current()` + `.render_human()` in `main_helper.rs` | Inline minimal `BuildInfo` in `src/build_info.rs` |
| `uptrakit-directories` | `AppDirs`, `create_secure_dir`, `write_secure_file_str`, `DirectoryError`, `Result` | Copy `src/dirs.rs` from the source crate |
| `uptrakit-tracing-init` | `pub use uptrakit_tracing_init::*` in `src/tracing_init.rs` | Copy full source into `src/tracing_init.rs` |
| `uptrakit-shared-macros` | `impl_report_conversion!` in `error.rs` and `shared_types.rs` | Inline macro in private `src/macros.rs` |
| `uptrakit-crypto` | `sealed_box_decrypt_base64` in `src/sensitive_params.rs` | Gate behind `sensitive-params` feature |
| `uptrakit-wire` | Wire types throughout, `ServiceTransport`, `ServiceMessage`, etc. | Generate into `src/generated/wire/` via xtask |
| `uptrakit-shared-types` | `hex::encode` (3 call sites in `ca.rs`), `SecretString` (1 use in `identity.rs`) | Inline `SecretString` in `src/secret_string.rs`; add `hex` crate dep directly |

`uptrakit-wire` depends on `uptrakit-surfaces`. Both have zero workspace path deps (only
third-party crates: `serde`, `uuid`, `time`, etc.). The xtask codegen copies both source trees
into `src/generated/`, rewriting the one cross-reference (`uptrakit_surfaces::` → local module
path) using `syn`.

---

## File Structure

**Create:**

- `xtask/Cargo.toml`
- `xtask/src/main.rs`
- `xtask/src/sync_sdk.rs`
- `crates/shared/service-sdk/src/backoff.rs`
- `crates/shared/service-sdk/src/build_info.rs`
- `crates/shared/service-sdk/src/dirs.rs`
- `crates/shared/service-sdk/src/macros.rs`
- `crates/shared/service-sdk/src/secret_string.rs`
- `crates/shared/service-sdk/src/generated/mod.rs`
- `crates/shared/service-sdk/src/generated/wire/` (multiple files, written by xtask)
- `crates/shared/service-sdk/src/generated/surfaces/` (multiple files, written by xtask)

**Modify:**

- `Cargo.toml` (workspace root) — add `xtask` to `members`
- `.cargo/config.toml` — add `xtask` alias
- `crates/shared/service-sdk/Cargo.toml` — remove 8 path deps, add new direct deps,
  add `sensitive-params` feature, set `publish = true`
- `crates/shared/service-sdk/src/lib.rs` — update `Backoff` and `tracing_init` re-exports
- `crates/shared/service-sdk/src/tracing_init.rs` — replace blanket re-export with inline impl
- `crates/shared/service-sdk/src/main_helper.rs` — replace `uptrakit_build_info::BuildInfo` call
- `crates/shared/service-sdk/src/error.rs` — replace macro import
- `crates/shared/service-sdk/src/shared_types.rs` — replace macro + wire imports
- `crates/shared/service-sdk/src/sensitive_params.rs` — gate behind `sensitive-params` feature
- `crates/shared/service-sdk/src/cli.rs` — update `uptrakit_directories::` imports
- `crates/shared/service-sdk/src/identity.rs` — update `uptrakit_directories::` + `SecretString`
- `crates/shared/service-sdk/src/discovery.rs` — update `uptrakit_directories::` import
- `crates/shared/service-sdk/src/ca.rs` — replace `uptrakit_shared_types::hex::encode`
- All files that `use uptrakit_wire::` — switch to `use crate::generated::wire::`
- `.husky/pre-commit` — add `cargo xtask sync-sdk --check`
- `.github/workflows/release-plz.yml` (or dedicated CI yml) — add `cargo xtask sync-sdk --check`

---

### Task 1: Add xtask crate to workspace

**Files:**

- Create: `xtask/Cargo.toml`
- Create: `xtask/src/main.rs`
- Modify: `Cargo.toml` (workspace root)
- Modify: `.cargo/config.toml`

- [ ] **Step 1: Add xtask to workspace members**

In root `Cargo.toml`, find the `members` array and add `"xtask"`:

```toml
members = [
    "crates/core/*",
    "crates/shared/*",
    "crates/ui/*",
    "crates/plugins/*/*",
    "xtask",
]
```

- [ ] **Step 2: Create `xtask/Cargo.toml`**

```toml
[package]
name = "xtask"
version = "0.0.0"
edition.workspace = true
publish = false

[[bin]]
name = "xtask"
path = "src/main.rs"

[dependencies]
anyhow = "1"
walkdir = "2"
syn = { version = "2", features = ["full", "visit-mut"] }
proc-macro2 = "1"
quote = "1"
prettyplease = "0.2"
clap = { version = "4", features = ["derive"] }
```

Note: do NOT use `workspace = true` for deps here — xtask is a standalone dev tool, not part of
the publishable workspace surface.

- [ ] **Step 3: Create `xtask/src/main.rs`**

```rust
mod sync_sdk;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Regenerate service-sdk src/generated/ from wire + surfaces source.
    SyncSdk {
        /// Exit with error if any file would change (for CI / pre-commit).
        #[arg(long)]
        check: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let workspace_root = workspace_root()?;

    match cli.command {
        Command::SyncSdk { check } => sync_sdk::run(&workspace_root, check)?,
    }
    Ok(())
}

fn workspace_root() -> Result<PathBuf> {
    let output = std::process::Command::new("cargo")
        .args(["locate-project", "--workspace", "--message-format=plain"])
        .output()?;
    let manifest = String::from_utf8(output.stdout)?.trim().to_string();
    Ok(PathBuf::from(manifest)
        .parent()
        .expect("Cargo.toml has parent")
        .to_path_buf())
}
```

- [ ] **Step 4: Add xtask alias to `.cargo/config.toml`**

The file currently contains macOS debuginfo flags. Add the alias section at the top:

```toml
[alias]
xtask = "run --package xtask --"

[target.'cfg(target_os = "macos")']
rustflags = ["-C", "split-debuginfo=unpacked"]
```

- [ ] **Step 5: Create minimal `xtask/src/sync_sdk.rs` stub**

```rust
use anyhow::Result;
use std::path::Path;

pub fn run(workspace_root: &Path, check: bool) -> Result<()> {
    println!("sync-sdk: workspace root = {}", workspace_root.display());
    if check {
        println!("sync-sdk: --check mode (no-op for now)");
    }
    Ok(())
}
```

- [ ] **Step 6: Verify xtask compiles**

```bash
cargo build -p xtask
cargo xtask sync-sdk
```

Expected: "sync-sdk: workspace root = /path/to/uptrakit"

- [ ] **Step 7: Commit**

```bash
git add xtask/ Cargo.toml Cargo.lock .cargo/config.toml
git commit -m "feat(xtask): add xtask crate skeleton with sync-sdk stub"
```

---

### Task 2: Inline small utility deps

**Files:**

- Create: `crates/shared/service-sdk/src/backoff.rs`
- Create: `crates/shared/service-sdk/src/build_info.rs`
- Create: `crates/shared/service-sdk/src/secret_string.rs`
- Modify: `crates/shared/service-sdk/src/tracing_init.rs`
- Modify: `crates/shared/service-sdk/src/main_helper.rs`
- Modify: `crates/shared/service-sdk/src/lib.rs`
- Modify: `crates/shared/service-sdk/src/identity.rs`
- Modify: `crates/shared/service-sdk/src/ca.rs`

- [ ] **Step 1: Create `src/backoff.rs`**

Copy verbatim from `crates/shared/backoff/src/lib.rs`, keeping all existing code unchanged.
Then check what deps it needs: `rand` and `tracing`. Add them as direct deps in
`crates/shared/service-sdk/Cargo.toml` if not already present (they are already present via
workspace — keep as-is, just add `rand = { workspace = true }` if missing).

Read `crates/shared/backoff/Cargo.toml` to find the exact `rand` version used, then read
`crates/shared/backoff/src/lib.rs` and write the full content to
`crates/shared/service-sdk/src/backoff.rs`.

- [ ] **Step 2: Update `lib.rs` Backoff re-export**

In `crates/shared/service-sdk/src/lib.rs`, find:

```rust
pub use uptrakit_backoff::Backoff;
```

Replace with:

```rust
pub use backoff::Backoff;
```

And add `mod backoff;` in the module declarations block at the top of `lib.rs`.

- [ ] **Step 3: Create `src/build_info.rs`**

This inlines just the parts of `uptrakit-build-info` used in `main_helper.rs` —
`BuildInfo::current()` and `render_human()`. The full upstream source is at
`crates/shared/build-info/src/lib.rs`. Copy it verbatim to
`crates/shared/service-sdk/src/build_info.rs`.

Check `crates/shared/build-info/Cargo.toml` for its deps. It uses only `std` — no third-party
deps required.

- [ ] **Step 4: Update `src/main_helper.rs`**

Find the import:

```rust
use uptrakit_build_info::BuildInfo;
```

(If `uptrakit_build_info` is used unqualified, find the import line.) Replace with:

```rust
use crate::build_info::BuildInfo;
```

Verify `pub fn print_build_info` compiles unchanged.

- [ ] **Step 5: Create `src/secret_string.rs`**

Copy `crates/shared/types/src/secret_string.rs` verbatim into
`crates/shared/service-sdk/src/secret_string.rs`. Remove the `#[cfg(feature = "sea-orm")]` block
entirely (sea-orm is not a dep of service-sdk). Remove the `#[cfg_attr(feature = "openapi", ...)]`
attribute on the struct. Add the required deps to service-sdk's `Cargo.toml`:
`zeroize = { workspace = true }`.

The final struct header:

```rust
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(transparent)]
pub struct SecretString(String);
```

- [ ] **Step 6: Update `src/identity.rs` SecretString import**

Find:

```rust
use uptrakit_shared_types::SecretString;
```

Replace with:

```rust
use crate::secret_string::SecretString;
```

- [ ] **Step 7: Update `src/ca.rs` hex usage**

`uptrakit_shared_types::hex` re-exports the `hex` crate's `encode` function. Replace the 3 call
sites in `ca.rs`:

Find each occurrence of `uptrakit_shared_types::hex::encode(` and replace with `hex::encode(`.

Add `hex = { workspace = true }` to `crates/shared/service-sdk/Cargo.toml` `[dependencies]`
(verify `hex` is in workspace deps; if not, add `hex = "0.4"` to workspace `[dependencies]` first).

Check if `hex` is already in `[workspace.dependencies]` in root `Cargo.toml`. If not, add it.

- [ ] **Step 8: Inline `uptrakit-tracing-init` into `src/tracing_init.rs`**

The current `crates/shared/service-sdk/src/tracing_init.rs` contains only:

```rust
pub use uptrakit_tracing_init::*;
```

Replace the entire file with the content of
`crates/shared/tracing-init/src/lib.rs` (449 lines) **excluding** the `#[cfg(test)] mod tests`
block at the end (lines 249–449). The public API (`TracingBuilder`, `BoxedLayer`,
`init_cli_tracing`, `init_test_tracing`) must be preserved exactly.

Update the imports at the top of the file — `uptrakit-tracing-init` uses
`tracing_subscriber`. Verify `tracing-subscriber` is in service-sdk's `Cargo.toml`. If not, add:

```toml
tracing-subscriber = { workspace = true, features = ["env-filter", "fmt", "registry"] }
```

The `cli` and `test-support` feature gates (`#[cfg(feature = "cli")]`,
`#[cfg(feature = "test-support")]`) in the inlined code must match service-sdk's existing feature
names exactly (they already match — `cli` and `test-support` are service-sdk features).

- [ ] **Step 9: Create `src/macros.rs`**

The `impl_report_conversion!` macro is used in `error.rs` and `shared_types.rs`. Copy the macro
definition from `crates/shared/macros/src/lib.rs` into a new private file:

```rust
// src/macros.rs — private macro, not part of public API

macro_rules! impl_report_conversion {
    // Copy the full macro_rules! body verbatim from
    // crates/shared/macros/src/lib.rs — the impl_report_conversion! definition only,
    // not wire_safe_enum!
    ($($tt:tt)*) => { ... };
}
```

Add `#[macro_use] mod macros;` to `lib.rs` before the other `mod` declarations so the macro is
available throughout the crate.

- [ ] **Step 10: Update `error.rs` and `shared_types.rs` macro imports**

In `crates/shared/service-sdk/src/error.rs`, remove:

```rust
use uptrakit_shared_macros::impl_report_conversion;
```

The macro is now in scope crate-wide via `#[macro_use] mod macros`. No import needed.

Same change in `crates/shared/service-sdk/src/shared_types.rs`.

- [ ] **Step 11: Remove inlined deps from `Cargo.toml` and run `cargo check`**

Remove from `[dependencies]` in `crates/shared/service-sdk/Cargo.toml`:

```toml
uptrakit-backoff = { workspace = true }       # remove
uptrakit-tracing-init = { workspace = true }  # remove
uptrakit-shared-macros = { workspace = true } # remove
uptrakit-build-info = { workspace = true }    # remove
```

Keep for now (will remove later): `uptrakit-crypto`, `uptrakit-directories`,
`uptrakit-wire`, `uptrakit-shared-types`.

Add any missing direct deps discovered above (`rand`, `zeroize`, `tracing-subscriber`, `hex`).

```bash
cargo check -p uptrakit-service-sdk --all-features
```

Fix any compilation errors. Do not proceed until this passes.

- [ ] **Step 12: Inline `uptrakit-directories` into `src/dirs.rs`**

Copy `crates/shared/directories/src/lib.rs` verbatim to
`crates/shared/service-sdk/src/dirs.rs`. Check `crates/shared/directories/Cargo.toml` for its
deps. It uses `directories` crate and `rootcause`. Add to service-sdk `Cargo.toml`:

```toml
directories = { workspace = true }
```

Verify `directories` (the third-party platform dirs crate) is in `[workspace.dependencies]` in
root `Cargo.toml`. If missing, add it.

- [ ] **Step 13: Update all `uptrakit_directories::` import sites**

Files that use `uptrakit_directories::`:

- `src/cli.rs` — `use uptrakit_directories::AppDirs`
- `src/identity.rs` — `uptrakit_directories::create_secure_dir`, `write_secure_file_str`,
  `DirectoryError`
- `src/discovery.rs` — `uptrakit_directories::write_secure_file_str`
- `src/error.rs` — `uptrakit_directories::DirectoryError` in error enum

In each file, replace `uptrakit_directories::` with `crate::dirs::`.
For explicit `use` statements, change `use uptrakit_directories::X` to `use crate::dirs::X`.

- [ ] **Step 14: Remove `uptrakit-directories` dep; `cargo check`**

Remove from `crates/shared/service-sdk/Cargo.toml`:

```toml
uptrakit-directories = { workspace = true }  # remove
```

```bash
cargo check -p uptrakit-service-sdk --all-features
```

- [ ] **Step 15: Commit**

```bash
git add crates/shared/service-sdk/ xtask/ Cargo.toml Cargo.lock .cargo/config.toml
git commit -m "feat(service-sdk): inline backoff, build-info, directories, tracing-init, shared-macros, SecretString"
```

---

### Task 3: Add `sensitive-params` feature; gate crypto dep

**Files:**

- Modify: `crates/shared/service-sdk/Cargo.toml`
- Modify: `crates/shared/service-sdk/src/sensitive_params.rs`
- Modify: `crates/shared/service-sdk/src/lib.rs`

- [ ] **Step 1: Add `sensitive-params` feature to `Cargo.toml`**

In `crates/shared/service-sdk/Cargo.toml`, update `[features]`:

```toml
[features]
default = ["zeroconf"]
zeroconf = ["dep:mdns-sd"]
cli = []
test-support = []
sensitive-params = ["dep:aws-lc-rs"]
```

Remove the `cli` and `test-support` forwarding to `uptrakit-tracing-init` (that dep is now
gone). The features still exist but no longer need to forward anywhere.

Add `aws-lc-rs` as an optional dep:

```toml
aws-lc-rs = { workspace = true, optional = true }
```

Verify `aws-lc-rs` is in `[workspace.dependencies]` in root `Cargo.toml`. If not, add:

```toml
aws-lc-rs = { version = "1", default-features = false }
```

Note: `aws-lc-rs` requires a C toolchain and NASM on some platforms. This is already a transitive
dep via `rustls` in most builds; adding it as an explicit optional dep does not change the default
build's requirements.

- [ ] **Step 2: Gate `src/sensitive_params.rs` behind the feature**

Read `crates/shared/service-sdk/src/sensitive_params.rs`. The file imports and calls
`uptrakit_crypto::ecies::sealed_box_decrypt_base64`. Replace the crypto dep usage:

The function currently calls `uptrakit_crypto::ecies::sealed_box_decrypt_base64(sealed_b64, private_key)`.

The `sensitive-params` feature adds `aws-lc-rs` directly, but the actual ECIES implementation
from `uptrakit-crypto` must be inlined. Read
`crates/shared/crypto/src/ecies.rs` (or wherever `sealed_box_decrypt_base64` is defined) to get
the implementation.

Add `#[cfg(feature = "sensitive-params")]` to the entire `sensitive_params.rs` module:

```rust
#![cfg(feature = "sensitive-params")]
// ... rest of file unchanged ...
```

Or gate individual items:

```rust
#[cfg(feature = "sensitive-params")]
pub fn decrypt_sensitive_params(...) -> ... { ... }
```

For the ECIES implementation: read `crates/shared/crypto/src/` to understand the exact imports,
then inline the `sealed_box_decrypt_base64` function and its dependencies into
`sensitive_params.rs` under the feature gate. The ECIES implementation uses `aws-lc-rs` directly
(or via `ring`). Check the actual implementation to determine which crypto primitives are needed.

- [ ] **Step 3: Gate the `decrypt_sensitive_params` re-export in `lib.rs`**

In `crates/shared/service-sdk/src/lib.rs`, find:

```rust
pub use sensitive_params::decrypt_sensitive_params;
```

Gate it:

```rust
#[cfg(feature = "sensitive-params")]
pub mod sensitive_params;
#[cfg(feature = "sensitive-params")]
pub use sensitive_params::decrypt_sensitive_params;
```

(If `sensitive_params` is currently an unconditional `pub mod`, wrap the mod declaration too.)

- [ ] **Step 4: Remove `uptrakit-crypto` dep; `cargo check` without the feature**

Remove from `crates/shared/service-sdk/Cargo.toml`:

```toml
uptrakit-crypto = { workspace = true }  # remove
```

```bash
# Default build — sensitive-params off — must pass
cargo check -p uptrakit-service-sdk
# With feature — must also pass
cargo check -p uptrakit-service-sdk --features sensitive-params
```

- [ ] **Step 5: Commit**

```bash
git add crates/shared/service-sdk/
git commit -m "feat(service-sdk): add sensitive-params feature; gate ECIES behind it"
```

---

### Task 4: Implement `sync-sdk` codegen in xtask

**Files:**

- Modify: `xtask/src/sync_sdk.rs` (full implementation)
- Modify: `xtask/Cargo.toml` (no changes needed if deps added in Task 1)

**Background:** `uptrakit-wire` and `uptrakit-surfaces` each have zero workspace path deps. The
only cross-reference to rewrite is `uptrakit_surfaces::` inside wire source files. The xtask:

1. Reads all `.rs` files from `crates/shared/surfaces/src/` and
   `crates/shared/wire/src/`
2. For surfaces files: no path rewriting needed
3. For wire files: rewrites `uptrakit_surfaces::` → `crate::generated::surfaces::` using `syn`
4. Writes output to `crates/shared/service-sdk/src/generated/surfaces/` and
   `crates/shared/service-sdk/src/generated/wire/`
5. Writes `crates/shared/service-sdk/src/generated/mod.rs`
6. In `--check` mode: compares generated output to committed state; exits non-zero if any file differs

- [ ] **Step 1: Write the path-rewriting `syn` visitor**

Replace the stub `xtask/src/sync_sdk.rs` with:

```rust
use anyhow::{Context, Result, bail};
use proc_macro2::Span;
use quote::ToTokens;
use std::{
    fs,
    path::{Path, PathBuf},
};
use syn::{File, Ident, Path as SynPath, visit_mut::VisitMut};
use walkdir::WalkDir;

pub fn run(workspace_root: &Path, check: bool) -> Result<()> {
    let surfaces_src = workspace_root.join("crates/shared/surfaces/src");
    let wire_src = workspace_root.join("crates/shared/wire/src");
    let sdk_generated = workspace_root
        .join("crates/shared/service-sdk/src/generated");

    // Collect output files: path → content
    let mut output: Vec<(PathBuf, String)> = Vec::new();

    // 1. Surfaces — no path rewriting
    collect_module(
        &surfaces_src,
        &sdk_generated.join("surfaces"),
        &[],
        &mut output,
    )?;

    // 2. Wire — rewrite `uptrakit_surfaces` → `crate::generated::surfaces`
    collect_module(
        &wire_src,
        &sdk_generated.join("wire"),
        &[(&["uptrakit_surfaces"], &["crate", "generated", "surfaces"])],
        &mut output,
    )?;

    // 3. Top-level generated/mod.rs
    let mod_rs = sdk_generated.join("mod.rs");
    let mod_content = "pub mod surfaces;\npub mod wire;\n".to_string();
    output.push((mod_rs, mod_content));

    if check {
        run_check(&output)
    } else {
        write_output(&sdk_generated, &output)
    }
}

/// Walk `src_dir`, parse each .rs file, apply rewrites, collect into `output`.
/// `dst_dir` is the target directory path (used to build output paths).
fn collect_module(
    src_dir: &Path,
    dst_dir: &Path,
    rewrites: &[(&[&str], &[&str])],
    output: &mut Vec<(PathBuf, String)>,
) -> Result<()> {
    for entry in WalkDir::new(src_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "rs"))
    {
        let rel = entry.path().strip_prefix(src_dir)?;
        let dst_path = dst_dir.join(rel);
        let src_content = fs::read_to_string(entry.path())
            .with_context(|| format!("reading {}", entry.path().display()))?;

        let content = rewrite_paths(&src_content, rewrites)
            .with_context(|| format!("rewriting {}", entry.path().display()))?;

        output.push((dst_path, content));
    }
    Ok(())
}

/// Parse `src`, apply all path segment rewrites, return formatted Rust source.
fn rewrite_paths(src: &str, rewrites: &[(&[&str], &[&str])]) -> Result<String> {
    let mut file: File = syn::parse_str(src).context("parsing source file")?;

    for (from, to) in rewrites {
        let mut rewriter = PathRewriter { from, to };
        rewriter.visit_file_mut(&mut file);
    }

    let tokens = file.to_token_stream();
    let formatted = prettyplease::unparse(&syn::parse2(tokens)?);
    Ok(formatted)
}

struct PathRewriter<'a> {
    from: &'a [&'a str],
    to: &'a [&'a str],
}

impl VisitMut for PathRewriter<'_> {
    fn visit_path_mut(&mut self, path: &mut SynPath) {
        // Recurse into nested paths first
        syn::visit_mut::visit_path_mut(self, path);

        let seg_idents: Vec<String> = path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect();

        let n = self.from.len();
        if seg_idents.len() >= n && seg_idents[..n] == *self.from {
            // Collect the tail segments (after the matched prefix)
            let tail: Vec<_> = path.segments.iter().skip(n).cloned().collect();

            path.segments.clear();
            path.leading_colon = None;

            for seg_name in self.to {
                path.segments.push(syn::PathSegment {
                    ident: Ident::new(seg_name, Span::call_site()),
                    arguments: syn::PathArguments::None,
                });
            }
            path.segments.extend(tail);
        }
    }
}

fn write_output(generated_dir: &Path, output: &[(PathBuf, String)]) -> Result<()> {
    for (path, content) in output {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)
            .with_context(|| format!("writing {}", path.display()))?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

fn run_check(output: &[(PathBuf, String)]) -> Result<()> {
    let mut dirty = false;
    for (path, expected) in output {
        let actual = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => {
                eprintln!("MISSING: {}", path.display());
                dirty = true;
                continue;
            }
        };
        if actual != *expected {
            eprintln!("STALE:   {}", path.display());
            dirty = true;
        }
    }
    if dirty {
        bail!(
            "Generated files are stale. Run `cargo xtask sync-sdk` to regenerate, then commit."
        );
    }
    println!("sync-sdk: all generated files up to date");
    Ok(())
}
```

- [ ] **Step 2: Verify xtask compiles**

```bash
cargo build -p xtask
```

Expected: clean build.

- [ ] **Step 3: Commit**

```bash
git add xtask/
git commit -m "feat(xtask): implement sync-sdk codegen with syn path rewriting"
```

---

### Task 5: Run `sync-sdk`, wire up generated types in service-sdk

**Files:**

- Create: `crates/shared/service-sdk/src/generated/` (xtask writes these)
- Modify: `crates/shared/service-sdk/src/lib.rs`
- Modify: `crates/shared/service-sdk/Cargo.toml`
- Modify: all files currently importing from `uptrakit_wire::` or `uptrakit_shared_types::`

- [ ] **Step 1: Run the codegen**

```bash
cargo xtask sync-sdk
```

Expected: files written under `crates/shared/service-sdk/src/generated/`.

- [ ] **Step 2: Add `pub mod generated` to `lib.rs`**

In `crates/shared/service-sdk/src/lib.rs`, add at the top of the `pub mod` declarations:

```rust
pub mod generated;
```

- [ ] **Step 3: Try to compile; collect all `uptrakit_wire::` and `uptrakit_shared_types::` errors**

```bash
cargo check -p uptrakit-service-sdk --all-features 2>&1 | grep "^error"
```

There will be many errors about unresolved `uptrakit_wire::` paths. Collect them all before fixing.

- [ ] **Step 4: Update import sites — `uptrakit_wire::` → `crate::generated::wire::`**

Files that import from `uptrakit_wire`: `ws.rs`, `event_loop.rs`, `connection.rs`, `lifecycle.rs`,
`config_proxy.rs`, `shared_types.rs`, `surface_proxy.rs`, `cert_handler.rs`, `test_support.rs`.

In each file, change every `use uptrakit_wire::` to `use crate::generated::wire::`.
Change every `uptrakit_wire::` qualified path (not in a `use`) to `crate::generated::wire::`.
Change `uptrakit_wire::surfaces::` to `crate::generated::surfaces::`.

This is mechanical. Run `cargo check` after each file to confirm no regressions.

- [ ] **Step 5: Remove `uptrakit-wire` and `uptrakit-shared-types` deps**

Remove from `crates/shared/service-sdk/Cargo.toml`:

```toml
uptrakit-wire = { workspace = true }           # remove
uptrakit-shared-types = { workspace = true }   # remove
```

- [ ] **Step 6: Run full check**

```bash
cargo check -p uptrakit-service-sdk --no-default-features
cargo check -p uptrakit-service-sdk --all-features
cargo test -p uptrakit-service-sdk --all-features
```

All must pass.

- [ ] **Step 7: Commit**

```bash
git add crates/shared/service-sdk/ xtask/ Cargo.toml Cargo.lock
git commit -m "feat(service-sdk): generate wire/surfaces types via xtask; remove all workspace path deps"
```

---

### Task 6: Set `publish = true`; verify `cargo publish --dry-run`

**Files:**

- Modify: `crates/shared/service-sdk/Cargo.toml`

- [ ] **Step 1: Add `publish = true`**

In `crates/shared/service-sdk/Cargo.toml`, add to `[package]`:

```toml
publish = true
```

This overrides the workspace default `publish = false`.

Also add required `description` field (required by crates.io):

```toml
description = "Rust SDK for building services managed by Uptrakit"
```

- [ ] **Step 2: Run `cargo publish --dry-run`**

```bash
cargo publish -p uptrakit-service-sdk --dry-run --allow-dirty
```

Expected: "Packaging uptrakit-service-sdk ... Uploading uptrakit-service-sdk ... (dry run)"
with no errors about missing fields or unpublishable deps.

If any workspace path dep is flagged, it means a dep was missed — find and inline it.

- [ ] **Step 3: Verify no workspace path deps remain**

```bash
cargo metadata --format-version 1 | python3 -c "
import sys, json
data = json.load(sys.stdin)
sdk = next(p for p in data['packages'] if p['name'] == 'uptrakit-service-sdk')
path_deps = [
    d['name'] for d in sdk['dependencies']
    if d.get('path') or (
        d['name'].startswith('uptrakit-') and
        any(p['name'] == d['name'] and 'path' in str(p.get('manifest_path',''))
            for p in data['packages'])
    )
]
print(path_deps or 'NONE')
"
```

Expected: `NONE`.

- [ ] **Step 4: Commit**

```bash
git add crates/shared/service-sdk/Cargo.toml
git commit -m "feat(service-sdk): set publish = true; add crates.io description"
```

---

### Task 7: Pre-commit hook and CI check

**Files:**

- Modify: `.husky/pre-commit`
- Modify: `.github/workflows/release-plz.yml` (or create `.github/workflows/generated-check.yml`)

- [ ] **Step 1: Update `.husky/pre-commit` to run `sync-sdk --check`**

Read the existing `.husky/pre-commit` file to understand its structure. Add a `sync-sdk --check`
call alongside the existing checks:

```bash
echo "[pre-commit] Checking service-sdk generated types are up to date..."
cargo xtask sync-sdk --check || {
  echo ""
  echo "Wire/surface types changed — service-sdk generated types are stale."
  echo "Run: cargo xtask sync-sdk"
  echo "Then: git add crates/shared/service-sdk/src/generated/ && git commit"
  exit 1
}
```

- [ ] **Step 2: Add CI check**

Create `.github/workflows/generated-check.yml`:

```yaml
name: Generated files

on:
  push:
    branches: [main]
    paths:
      - "crates/shared/wire/**"
      - "crates/shared/surfaces/**"
      - "crates/shared/service-sdk/src/generated/**"
      - "xtask/**"
  pull_request:
    paths:
      - "crates/shared/wire/**"
      - "crates/shared/surfaces/**"
      - "crates/shared/service-sdk/src/generated/**"
      - "xtask/**"

jobs:
  check-generated:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: Check service-sdk generated types are up to date
        run: cargo xtask sync-sdk --check
```

- [ ] **Step 3: Verify pre-commit hook runs correctly**

```bash
# Modify a wire source file temporarily to trigger a stale check
echo "// test" >> crates/shared/wire/src/lib.rs
cargo xtask sync-sdk --check
# Expected: exit 1 with "STALE" output
# Revert the change
git checkout crates/shared/wire/src/lib.rs
# Re-run — should pass
cargo xtask sync-sdk --check
```

- [ ] **Step 4: Commit**

```bash
git add .husky/pre-commit .github/workflows/generated-check.yml
git commit -m "ci(generated): add sync-sdk --check to pre-commit hook and CI"
```
