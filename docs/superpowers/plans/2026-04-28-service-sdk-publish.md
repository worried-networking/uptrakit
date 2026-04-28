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
| `uptrakit-shared-types` | `hex::encode` (3 call sites in `ca.rs`), `SecretString` (1 use in `identity.rs`); also referenced throughout wire source (payloads, pagination, etc.) | Included in codegen: copy source into `src/generated/shared_types/`; add `hex` dep directly for `ca.rs` |

`uptrakit-wire` depends on `uptrakit-surfaces` and `uptrakit-shared-types`. All three have
zero workspace path deps themselves (only third-party crates: `serde`, `uuid`, `time`, etc.).
The xtask codegen copies all three source trees into `src/generated/`, rewriting cross-references
(`uptrakit_surfaces::` and `uptrakit_shared_types::` → local module paths) using `syn`.

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

In root `Cargo.toml`, find the `members` array and add `"xtask"`. The current complete list is:

```toml
members = [
    "crates/core/*",
    "crates/shared/*",
    "crates/ui/*",
    "crates/plugins/*/*",
    "crates/plugins/hooks/*",
    "crates/plugins/notifications/*",
    "frontend",
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

And add `pub mod backoff;` in the module declarations block at the top of `lib.rs`.
(Must be `pub mod` — `Backoff` is part of the public API via the re-export.)

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

- [ ] **Step 5: Update `src/ca.rs` hex usage**

`uptrakit_shared_types::hex` re-exports the `hex` crate's `encode` function. Replace the 3 call
sites in `ca.rs`:

Find each occurrence of `uptrakit_shared_types::hex::encode(` and replace with `hex::encode(`.

Add `hex = { workspace = true }` to `crates/shared/service-sdk/Cargo.toml` `[dependencies]`
(verify `hex` is in workspace deps; if not, add `hex = "0.4"` to workspace `[dependencies]` first).

Check if `hex` is already in `[workspace.dependencies]` in root `Cargo.toml`. If not, add it.

- [ ] **Step 6: Inline `uptrakit-tracing-init` into `src/tracing_init.rs`**

The current `crates/shared/service-sdk/src/tracing_init.rs` contains only:

```rust
pub use uptrakit_tracing_init::*;
```

Read `crates/shared/tracing-init/src/lib.rs` first. Replace the entire `src/tracing_init.rs`
file with that content **excluding** the `#[cfg(test)] mod tests` block at the end. The public
API (`TracingBuilder`, `BoxedLayer`,
`init_cli_tracing`, `init_test_tracing`) must be preserved exactly.

Update the imports at the top of the file — `uptrakit-tracing-init` uses
`tracing_subscriber`. Verify `tracing-subscriber` is in service-sdk's `Cargo.toml`. If not, add:

```toml
tracing-subscriber = { workspace = true, features = ["env-filter", "fmt", "registry"] }
```

The `cli` and `test-support` feature gates (`#[cfg(feature = "cli")]`,
`#[cfg(feature = "test-support")]`) in the inlined code must match service-sdk's existing feature
names exactly (they already match — `cli` and `test-support` are service-sdk features).

- [ ] **Step 7: Create `src/macros.rs`**

The `impl_report_conversion!` macro is used in `error.rs` and `shared_types.rs`. Create
`crates/shared/service-sdk/src/macros.rs` containing exactly the three-arm
`macro_rules! impl_report_conversion! { ... }` block found at lines 95–148 of
`crates/shared/macros/src/lib.rs`. Copy those 54 lines verbatim. Do NOT include
`wire_safe_enum!` or the doc comments above line 95.

The result must look like:

```rust
macro_rules! impl_report_conversion {
    // Single: simple variant mapping
    ($source:ty => $target:ident :: $variant:ident) => {
        impl<T> rootcause::ReportConversion<$source, rootcause::prelude::markers::Mutable, T>
            for $target
        where
            $target: rootcause::prelude::markers::ObjectMarkerFor<T>,
        {
            fn convert_report(
                report: rootcause::prelude::Report<
                    $source,
                    rootcause::prelude::markers::Mutable,
                    T,
                >,
            ) -> rootcause::prelude::Report<
                Self,
                rootcause::prelude::markers::Mutable,
                T,
            > {
                report.context_transform($target::$variant)
            }
        }
    };

    // Single: closure-based transform
    ($source:ty => $target:ident, $closure:expr) => {
        impl<T> rootcause::ReportConversion<$source, rootcause::prelude::markers::Mutable, T>
            for $target
        where
            $target: rootcause::prelude::markers::ObjectMarkerFor<T>,
        {
            fn convert_report(
                report: rootcause::prelude::Report<
                    $source,
                    rootcause::prelude::markers::Mutable,
                    T,
                >,
            ) -> rootcause::prelude::Report<
                Self,
                rootcause::prelude::markers::Mutable,
                T,
            > {
                report.context_transform($closure)
            }
        }
    };

    // Multiple: trailing comma support
    ($($source:ty => $target:ident :: $variant:ident),+ $(,)?) => {
        $(
            $crate::impl_report_conversion!($source => $target::$variant);
        )+
    };
}
```

Add `#[macro_use] mod macros;` to `lib.rs` before the other `mod` declarations so the macro is
available throughout the crate.

- [ ] **Step 8: Update `error.rs` and `shared_types.rs` macro imports**

In `crates/shared/service-sdk/src/error.rs`, remove:

```rust
use uptrakit_shared_macros::impl_report_conversion;
```

The macro is now in scope crate-wide via `#[macro_use] mod macros`. No import needed.

Same change in `crates/shared/service-sdk/src/shared_types.rs`.

- [ ] **Step 9: Remove inlined deps from `Cargo.toml` and run `cargo check`**

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

- [ ] **Step 10: Inline `uptrakit-directories` into `src/dirs.rs`**

Copy `crates/shared/directories/src/lib.rs` verbatim to
`crates/shared/service-sdk/src/dirs.rs`. Check `crates/shared/directories/Cargo.toml` for its
deps. It uses `directories` crate and `rootcause`. Add to service-sdk `Cargo.toml`:

```toml
directories = { workspace = true }
```

Verify `directories` (the third-party platform dirs crate) is in `[workspace.dependencies]` in
root `Cargo.toml`. If missing, add it.

- [ ] **Step 11: Update all `uptrakit_directories::` import sites**

Files that use `uptrakit_directories::`:

- `src/cli.rs` — `use uptrakit_directories::AppDirs`
- `src/identity.rs` — `uptrakit_directories::create_secure_dir`, `write_secure_file_str`,
  `DirectoryError`
- `src/discovery.rs` — `uptrakit_directories::write_secure_file_str`
- `src/error.rs` — `uptrakit_directories::DirectoryError` in error enum

In each file, replace `uptrakit_directories::` with `crate::dirs::`.
For explicit `use` statements, change `use uptrakit_directories::X` to `use crate::dirs::X`.

- [ ] **Step 12: Remove `uptrakit-directories` dep; `cargo check`**

Remove from `crates/shared/service-sdk/Cargo.toml`:

```toml
uptrakit-directories = { workspace = true }  # remove
```

```bash
cargo check -p uptrakit-service-sdk --all-features
```

- [ ] **Step 13: Commit**

```bash
git add crates/shared/service-sdk/ xtask/ Cargo.toml Cargo.lock .cargo/config.toml
git commit -m "feat(service-sdk): inline backoff, build-info, directories, tracing-init, shared-macros"
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

Add `aws-lc-rs` as an optional dep, plus the other deps the inlined ECIES code needs.
`sha2` is already in service-sdk's `[dependencies]`. The others need to be added:

```toml
# Optional (sensitive-params feature only)
aws-lc-rs = { workspace = true, optional = true }

# Direct (also needed by backoff inline and SecretString inline in Task 2)
rand = { workspace = true }
zeroize = { workspace = true }
base64 = { workspace = true }
```

Verify all four are in `[workspace.dependencies]` in root `Cargo.toml`. If any are missing,
add them (e.g., `aws-lc-rs = { version = "1", default-features = false }`).
`rand`, `zeroize`, and `base64` are already workspace deps (used widely in other crates).

Note: `aws-lc-rs` requires a C toolchain and NASM on some platforms. Transitive via `rustls`
in most builds; the explicit optional dep does not change default build requirements.

- [ ] **Step 2: Gate `src/sensitive_params.rs` behind the feature; inline ECIES**

`crates/shared/service-sdk/src/sensitive_params.rs` currently has one import:
`use uptrakit_crypto::ecies::sealed_box_decrypt_base64;`

Replace the entire file with the following. The two helper functions are copied from
`crates/shared/crypto/src/ecies.rs` (`sealed_box_decrypt` lines 127–177 and
`sealed_box_decrypt_base64` lines 206–221) with `crate::CryptoError` / `crate::Result` replaced
by `Result<_, String>` since `decrypt_sensitive_params` already converts errors to `String`.

```rust
//! ECIES-sealed sensitive parameter decryption for surface actions.

use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use aws_lc_rs::agreement::{self, PrivateKey};
use base64::Engine as _;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

const P256_UNCOMPRESSED_PUBLIC_KEY_LEN: usize = 65;
const NONCE_LEN: usize = 12;
const MIN_SEALED_LEN: usize = P256_UNCOMPRESSED_PUBLIC_KEY_LEN + NONCE_LEN + 16;

fn sealed_box_decrypt(sealed: &[u8], private_key_pkcs8_der: &[u8]) -> Result<Vec<u8>, String> {
    if sealed.len() < MIN_SEALED_LEN {
        return Err("ciphertext too short".to_string());
    }

    let ephemeral_public_bytes = &sealed[..P256_UNCOMPRESSED_PUBLIC_KEY_LEN];
    let nonce_bytes: [u8; NONCE_LEN] = sealed
        [P256_UNCOMPRESSED_PUBLIC_KEY_LEN..P256_UNCOMPRESSED_PUBLIC_KEY_LEN + NONCE_LEN]
        .try_into()
        .map_err(|_| "invalid nonce length".to_string())?;
    let ciphertext_and_tag = &sealed[P256_UNCOMPRESSED_PUBLIC_KEY_LEN + NONCE_LEN..];

    let private_key =
        PrivateKey::from_private_key_der(&agreement::ECDH_P256, private_key_pkcs8_der)
            .map_err(|e| format!("parse private key: {e}"))?;
    let peer_public = aws_lc_rs::agreement::UnparsedPublicKey::new(
        &agreement::ECDH_P256,
        ephemeral_public_bytes,
    );
    let shared_secret: Zeroizing<[u8; 32]> = agreement::agree(
        &private_key,
        peer_public,
        "ECDH agreement failed".to_string(),
        |secret| {
            let mut key = Zeroizing::new([0u8; 32]);
            let hash = Sha256::digest(secret);
            key.copy_from_slice(&hash);
            Ok(key)
        },
    )?;

    let unbound = UnboundKey::new(&AES_256_GCM, shared_secret.as_slice())
        .map_err(|e| format!("AES key: {e}"))?;
    let aes_key = LessSafeKey::new(unbound);
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);
    let mut buf = ciphertext_and_tag.to_vec();
    let plaintext = aes_key
        .open_in_place(nonce, Aad::from(ephemeral_public_bytes), &mut buf)
        .map_err(|_| "wrong key or tampered sealed box".to_string())?;

    Ok(plaintext.to_vec())
}

fn sealed_box_decrypt_base64(
    sealed_base64: &str,
    private_key_pkcs8_der: &[u8],
) -> Result<String, String> {
    let sealed = base64::engine::general_purpose::STANDARD
        .decode(sealed_base64)
        .map_err(|e| format!("base64 decode sealed box: {e}"))?;
    let plaintext = sealed_box_decrypt(&sealed, private_key_pkcs8_der)?;
    String::from_utf8(plaintext).map_err(|e| format!("invalid UTF-8: {e}"))
}

/// Decrypt and deserialize ECIES-sealed sensitive parameters.
pub fn decrypt_sensitive_params<T: DeserializeOwned>(
    sealed_base64: Option<&str>,
    private_key_der: Option<&[u8]>,
) -> Result<Option<T>, String> {
    let sealed_b64 = match sealed_base64 {
        Some(s) if !s.is_empty() => s,
        _ => return Ok(None),
    };
    let private_key = private_key_der
        .ok_or_else(|| "sensitive params received but no private key available".to_string())?;
    let json_str = sealed_box_decrypt_base64(sealed_b64, private_key)
        .map_err(|e| format!("failed to decrypt sensitive params: {e}"))?;
    let params: T = serde_json::from_str(&json_str)
        .map_err(|e| format!("failed to parse sensitive params JSON: {e}"))?;
    Ok(Some(params))
}
```

Note: `aws_lc_rs::agreement::agree` is generic over `E`. Passing `"ECDH agreement failed".to_string()`
fixes `E = String`, consistent with the `Result<_, String>` return type throughout this function.

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
    let shared_types_src = workspace_root.join("crates/shared/types/src");
    let sdk_generated = workspace_root
        .join("crates/shared/service-sdk/src/generated");

    // Collect output files: path → content
    let mut output: Vec<(PathBuf, String)> = Vec::new();

    // 1. Surfaces — no path rewriting needed (no internal deps)
    collect_module(
        &surfaces_src,
        &sdk_generated.join("surfaces"),
        &[],
        &mut output,
    )?;

    // 2. Shared-types — rewrite self-references only (no internal path deps)
    collect_module(
        &shared_types_src,
        &sdk_generated.join("shared_types"),
        &[(&["crate"], &["crate", "generated", "shared_types"])],
        &mut output,
    )?;

    // 3. Wire — rewrite uptrakit_surfaces and uptrakit_shared_types
    //    (wire has workspace path deps on both)
    collect_module(
        &wire_src,
        &sdk_generated.join("wire"),
        &[
            (&["uptrakit_surfaces"], &["crate", "generated", "surfaces"]),
            (&["uptrakit_shared_types"], &["crate", "generated", "shared_types"]),
        ],
        &mut output,
    )?;

    // 4. Top-level generated/mod.rs
    let mod_rs = sdk_generated.join("mod.rs");
    let mod_content = "pub mod shared_types;\npub mod surfaces;\npub mod wire;\n".to_string();
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

- [ ] **Step 3: Add `--commit` convenience flag to `sync_sdk.rs`**

Spec requires `cargo xtask sync-sdk --commit` to regenerate and commit in one shot.
Add `commit: bool` arg to the `SyncSdk` variant in `xtask/src/main.rs`:

```rust
SyncSdk {
    #[arg(long)]
    check: bool,
    #[arg(long)]
    commit: bool,
},
```

In `xtask/src/sync_sdk.rs`, add a `commit` parameter to `run`:

```rust
pub fn run(workspace_root: &Path, check: bool, commit: bool) -> Result<()> {
    // ... existing codegen ...

    if check {
        run_check(&output)
    } else {
        write_output(&sdk_generated, &output)?;
        if commit {
            let status = std::process::Command::new("git")
                .args(["add", "crates/shared/service-sdk/src/generated/"])
                .status()?;
            anyhow::ensure!(status.success(), "git add failed");
            let status = std::process::Command::new("git")
                .args(["commit", "-m", "chore(generated): regenerate service-sdk wire/surfaces types"])
                .status()?;
            anyhow::ensure!(status.success(), "git commit failed");
        }
        Ok(())
    }
}
```

Update the `match` arm in `main.rs` to pass `commit`.

- [ ] **Step 4: Commit**

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

Also update `src/identity.rs`: find `use uptrakit_shared_types::SecretString` and replace with:

```rust
use crate::generated::shared_types::SecretString;
```

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
git commit -m "feat(service-sdk): generate wire/surfaces/shared_types via xtask; remove all workspace path deps"
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

- [ ] **Step 1: Update `.husky/pre-commit` to run `sync-sdk` and abort if files changed**

Per spec: the hook runs `cargo xtask sync-sdk` (regenerates in-place), then checks if any
generated files are now dirty and aborts the commit if so.

Read the existing `.husky/pre-commit` to find the correct insertion point (after the
markdownlint check, before `exit 0`). Add:

```bash
echo "[pre-commit] Checking service-sdk generated types..."
cargo xtask sync-sdk
if ! git diff --quiet crates/shared/service-sdk/src/generated/; then
  echo ""
  echo "Wire/surface types changed — service-sdk generated types updated:"
  git diff --name-only crates/shared/service-sdk/src/generated/
  echo ""
  echo "Commit aborted. Review changes, then:"
  echo "  git add crates/shared/service-sdk/src/generated/ && git commit"
  echo "  — or —"
  echo "  cargo xtask sync-sdk --commit"
  exit 1
fi
```

Note: `--commit` is referenced above; add it as a convenience alias in Task 4 (see Step 3 there)
if not already done, or add a note that it is not yet implemented and the manual `git add` path
is the current method.

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
