# Kernel Cleanup B: `hook_kernel_cleanup_apt` Plugin + Docs

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Prerequisite:** Plan A (`2026-05-12-kernel-cleanup-a-framework.md`) is fully
landed and tagged at `plan-a-kernel-cleanup-framework`. Do not start Plan B
until Plan A's quality-gate sweep is clean.

**Goal:** Ship the opt-in `hook_kernel_cleanup_apt` Hook-family plugin, register
it in the plugin catalog, ship ADR 0010, the operator-facing plugin doc, and the
cross-distro kernel-housekeeping runbook covering Fedora's `installonly_limit`
knob.

**Architecture:** New crate `crates/plugins/hooks/kernel-cleanup-apt/` mirrors
`hook_systemd` shape — `config.rs` (with `PluginConfig::validate`),
`decisions.rs` (pure parsing/decision functions), `error.rs`
(`rootcause::Report<KernelCleanupAptError>`), `plugin.rs` (`declare_plugin!` +
`LifecycleHook` impl overriding `detect_host_compatibility`). One sudoers entry.
Audit trail via `tracing::info!` / `tracing::warn!` (batch-path output is
discarded per Plan A's documented limitation). No DB writes, no wire-protocol
changes.

**Tech Stack:** Rust edition 2024 (workspace.edition), `async_trait`,
`rootcause::Report<E>` + `thiserror`, `parking_lot` (not used directly here —
plugin is stateless), `serde` + `serde_json` for config. Snapshot rules applied:
`#[non_exhaustive]` on public config struct; `#[expect(lint, reason = "...")]`
over `#[allow]`; no `unwrap()` in production; Conventional Commits.

---

## File Structure

**Create:**

- `crates/plugins/hooks/kernel-cleanup-apt/Cargo.toml`
- `crates/plugins/hooks/kernel-cleanup-apt/README.md` — plugin lifecycle doc per
  `plugin-guidelines.md`.
- `crates/plugins/hooks/kernel-cleanup-apt/src/lib.rs`
- `crates/plugins/hooks/kernel-cleanup-apt/src/config.rs` —
  `KernelCleanupAptConfig` + `PluginConfig` impl.
- `crates/plugins/hooks/kernel-cleanup-apt/src/error.rs` —
  `KernelCleanupAptError`.
- `crates/plugins/hooks/kernel-cleanup-apt/src/decisions.rs` — `KernelEntry`,
  `KernelVariantSet`, `KeepDecision`, `parse_dpkg_kernel_list`,
  `parse_apt_mark_holds`, `compute_keep_and_purge_sets`.
- `crates/plugins/hooks/kernel-cleanup-apt/src/plugin.rs` —
  `KernelCleanupAptHookPlugin` + `declare_plugin!`.
- `docs/end-user/plugins/hook_kernel_cleanup_apt.md`
- `docs/end-user/operations/kernel-housekeeping.md`
- `docs/adr/0010-host-scoped-housekeeping-via-meta-package-hooks.md`

**Modify:**

- `Cargo.toml` (workspace root) — add the new crate to `[workspace.members]` and
  `[workspace.dependencies]`.
- `crates/plugins/infrastructure/registry/Cargo.toml` — add the new crate to
  `[dependencies]`.
- `crates/plugins/infrastructure/registry/src/registry.rs` (lines 55-57) —
  register the new descriptor.
- `docs/development/plugin-guidelines.md` — single-line pointer to the preflight
  idiom precedent.

---

## Task 1: Pre-flight — confirm Plan A landed

**Files:** workspace root.

- [ ] **Step 1: Verify Plan A tag exists**

```bash
git tag --list | grep plan-a-kernel-cleanup-framework
```

Expected: `plan-a-kernel-cleanup-framework`. If absent, stop and confirm Plan A
is fully merged before continuing.

- [ ] **Step 2: Verify Plan A's primitives exist**

```bash
grep -n "batch_id: Option<uuid::Uuid>" crates/plugins/infrastructure/core/src/traits.rs
grep -n "async fn detect_host_compatibility" crates/plugins/infrastructure/core/src/roles.rs
```

Expected: each returns at least one match. If not, Plan A is incomplete.

- [ ] **Step 3: Baseline quality gates**

```bash
cargo check --all-features
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: clean. Anything red is pre-existing and must be reported before
continuing.

---

## Task 2: Scaffold the crate skeleton

**Files:**

- Create: `crates/plugins/hooks/kernel-cleanup-apt/Cargo.toml`
- Create: `crates/plugins/hooks/kernel-cleanup-apt/src/lib.rs`
- Modify: `Cargo.toml` (workspace root — `[workspace.members]` +
  `[workspace.dependencies]`)

- [ ] **Step 1: Create the crate Cargo.toml**

Create `crates/plugins/hooks/kernel-cleanup-apt/Cargo.toml`:

```toml
[package]
name = "uptrakit-plugin-hook-kernel-cleanup-apt"
description = "Uptrakit hook plugin: purge superseded Linux kernel packages after update (Debian/Ubuntu)"
edition.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
version = "0.0.1"

[dependencies]
uptrakit-plugin-infrastructure-core = { workspace = true }
uptrakit-shared-macros = { workspace = true }
uptrakit-shared-types = { workspace = true }
uptrakit-command = { workspace = true }
rootcause       = { workspace = true }
thiserror       = { workspace = true }
async-trait     = { workspace = true }
serde           = { workspace = true }
serde_json      = { workspace = true }
tracing         = { workspace = true }

[dev-dependencies]
tokio = { workspace = true, features = ["macros", "rt"] }

[lints]
workspace = true
```

- [ ] **Step 2: Create lib.rs**

Create `crates/plugins/hooks/kernel-cleanup-apt/src/lib.rs`:

```rust
//! Opt-in Hook-family plugin that purges superseded Linux kernel
//! packages (`linux-image-*`, `linux-image-unsigned-*`,
//! `linux-modules-*`, `linux-modules-extra-*`, `linux-headers-*`)
//! on Debian and Ubuntu hosts after a kernel update.
//!
//! See `README.md` for the full lifecycle and the operator-facing
//! documentation at `docs/end-user/plugins/hook_kernel_cleanup_apt.md`.

pub mod config;
pub mod decisions;
pub mod error;
pub mod plugin;

pub use config::KernelCleanupAptConfig;
pub use error::{KernelCleanupAptError, Result};
pub use plugin::{DESCRIPTOR, KernelCleanupAptHookPlugin};
```

- [ ] **Step 3: Register the crate in the workspace**

Open the workspace `Cargo.toml` and:

1. Add the new path under `[workspace.members]` next to the existing hook
   crates. Keep alphabetical order in the surrounding section.
2. Add the workspace dependency line near the existing hook entries (around line
   155):

```toml
uptrakit-plugin-hook-kernel-cleanup-apt = { path = "crates/plugins/hooks/kernel-cleanup-apt", version = "0.0.1" }
```

- [ ] **Step 4: Verify the workspace builds (modules will be empty stubs until
      later tasks)**

Create empty module files so the build can succeed before logic lands:

```bash
mkdir -p crates/plugins/hooks/kernel-cleanup-apt/src
printf 'pub struct KernelCleanupAptConfig;\n' > crates/plugins/hooks/kernel-cleanup-apt/src/config.rs
printf '\n' > crates/plugins/hooks/kernel-cleanup-apt/src/decisions.rs
printf 'pub type Result<T> = std::result::Result<T, KernelCleanupAptError>;\npub struct KernelCleanupAptError;\n' > crates/plugins/hooks/kernel-cleanup-apt/src/error.rs
printf 'pub struct KernelCleanupAptHookPlugin;\npub const DESCRIPTOR: &str = "stub";\n' > crates/plugins/hooks/kernel-cleanup-apt/src/plugin.rs
```

These stubs will be entirely overwritten in later tasks — they exist only so
`cargo check` succeeds at the end of Task 2. Then:

```bash
cargo check -p uptrakit-plugin-hook-kernel-cleanup-apt
```

Expected: success.

- [ ] **Step 5: Commit the skeleton**

```bash
git add Cargo.toml crates/plugins/hooks/kernel-cleanup-apt/
git commit -m "feat(plugin): scaffold hook_kernel_cleanup_apt crate

Empty module stubs (config, decisions, error, plugin) registered in
the workspace. Subsequent commits replace each stub with the
intended implementation."
```

---

## Task 3: Implement `KernelCleanupAptError` + `Result` alias

**Files:**

- Modify: `crates/plugins/hooks/kernel-cleanup-apt/src/error.rs`

- [ ] **Step 1: Replace the stub with the real error type**

Overwrite `crates/plugins/hooks/kernel-cleanup-apt/src/error.rs`:

```rust
//! Error type for the kernel-cleanup-apt hook plugin.

use rootcause::Report;
use thiserror::Error;
use uptrakit_plugin_infrastructure_core::PluginError;
use uptrakit_shared_macros::impl_report_conversion;

/// Result alias scoped to this crate.
pub type Result<T> = std::result::Result<T, Report<KernelCleanupAptError>>;

#[non_exhaustive]
#[derive(Debug, Error)]
pub enum KernelCleanupAptError {
    /// A shell command exited non-zero.
    #[error("command `{command}` exited {exit_code}")]
    CommandFailed { command: String, exit_code: i32 },

    /// A command's stdout failed to parse.
    #[error("failed to parse output of `{source}`: {detail}")]
    OutputParse { source: String, detail: String },

    /// Plugin configuration is invalid in a runtime-only way
    /// (`PluginConfig::validate` catches static cases ahead of
    /// `new()`; this variant covers conditions detectable only at
    /// hook-invocation time).
    #[error("configuration: {0}")]
    Configuration(String),
}

// Bidirectional Report-to-Report conversion. The macro lives in
// `uptrakit_shared_macros` (NOT in `plugin_infrastructure_core`); it takes
// `$source => $target, $closure` arms — never a `map:` keyword. Precedent:
// `crates/plugins/package-managers/apt/src/error.rs:16-17`.
impl_report_conversion!(
    KernelCleanupAptError => PluginError,
    |e| PluginError::PluginInternal(e.to_string())
);
impl_report_conversion!(
    PluginError => KernelCleanupAptError,
    |e| KernelCleanupAptError::Configuration(e.to_string())
);
```

- [ ] **Step 2: Verify it builds**

```bash
cargo check -p uptrakit-plugin-hook-kernel-cleanup-apt
```

Expected: success.

```bash
cargo clippy -p uptrakit-plugin-hook-kernel-cleanup-apt --all-targets -- -D warnings
```

Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add crates/plugins/hooks/kernel-cleanup-apt/src/error.rs
git commit -m "feat(kernel-cleanup-apt): error type + Result alias

Mirror the rootcause::Report<E> idiom used by other plugins.
Bidirectional conversion to PluginError via impl_report_conversion!
so LifecycleHook::execute_post_hook can return Result<()> directly."
```

---

## Task 4: Implement `KernelCleanupAptConfig` + `PluginConfig::validate`

**Files:**

- Modify: `crates/plugins/hooks/kernel-cleanup-apt/src/config.rs`

- [ ] **Step 1: Write failing tests**

Overwrite `crates/plugins/hooks/kernel-cleanup-apt/src/config.rs`:

```rust
//! Configuration for the kernel-cleanup-apt hook plugin.

use serde::{Deserialize, Serialize};
use uptrakit_plugin_infrastructure_core::{PluginConfig, PluginConfigValidationError};

const fn default_keep_n() -> u8 {
    2
}

const fn default_min_boot_free_kib() -> u32 {
    51_200
}

/// Configuration for `hook_kernel_cleanup_apt`.
///
/// `keep_n` is the total number of installed kernels retained after
/// cleanup (the currently-running and latest-installed kernels are
/// always kept; `keep_n` must be at least 2). Held kernels
/// (`apt-mark hold`) are exempt and do not consume `keep_n` slots.
///
/// `dry_run = true` causes the hook to emit `[post-hook] would
/// purge: <list>` and a structured `tracing::info!` event, then
/// return without invoking `apt-get purge`.
///
/// `min_boot_free_kib` is the minimum free space on `/boot` (KiB)
/// required to proceed. Below the threshold the hook aborts to
/// avoid `update-initramfs` failing mid-purge. Set to `0` to opt
/// out of `/boot` gating entirely (escape hatch for bind-mounted
/// `/boot`, btrfs subvolumes, LVM-on-`/boot`, or tiny VMs that
/// accept the risk).
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelCleanupAptConfig {
    #[serde(default = "default_keep_n")]
    pub keep_n: u8,

    #[serde(default)]
    pub dry_run: bool,

    #[serde(default = "default_min_boot_free_kib")]
    pub min_boot_free_kib: u32,
}

impl Default for KernelCleanupAptConfig {
    fn default() -> Self {
        Self {
            keep_n: default_keep_n(),
            dry_run: false,
            min_boot_free_kib: default_min_boot_free_kib(),
        }
    }
}

impl PluginConfig for KernelCleanupAptConfig {
    fn validate(&self) -> std::result::Result<(), PluginConfigValidationError> {
        if self.keep_n < 2 {
            return Err(PluginConfigValidationError::invalid_field(
                "keep_n",
                "keep_n must be >= 2 to protect running and latest kernels".to_string(),
            ));
        }
        Ok(())
    }

    fn form_schema() -> Vec<uptrakit_plugin_infrastructure_core::form_schema::FormFieldDescriptor> {
        use uptrakit_plugin_infrastructure_core::form_schema::FormFieldDescriptor;
        vec![
            FormFieldDescriptor::new("keep_n", "Kernels to keep")
                .with_help_text(
                    "Total installed kernels to retain. Running and latest are always kept. Minimum 2.",
                ),
            FormFieldDescriptor::new("dry_run", "Dry run")
                .with_help_text(
                    "If on, log the would-be purge without executing apt-get purge.",
                ),
            FormFieldDescriptor::new("min_boot_free_kib", "Minimum /boot free (KiB)")
                .with_help_text(
                    "Minimum free space on /boot required to proceed. 0 disables the check.",
                ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_safe() {
        let cfg = KernelCleanupAptConfig::default();
        assert_eq!(cfg.keep_n, 2);
        assert!(!cfg.dry_run);
        assert_eq!(cfg.min_boot_free_kib, 51_200);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn keep_n_below_two_is_rejected() {
        let cfg = KernelCleanupAptConfig {
            keep_n: 1,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(format!("{err}").contains("keep_n"));
    }

    #[test]
    fn keep_n_zero_is_rejected() {
        let cfg = KernelCleanupAptConfig {
            keep_n: 0,
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn min_boot_free_kib_zero_is_accepted_as_escape_hatch() {
        let cfg = KernelCleanupAptConfig {
            min_boot_free_kib: 0,
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());
    }

    // No serde-roundtrip test — snapshot rule
    // "No testing of upstream crate behavior (serde, thiserror, regex, etc.)"
    // means we trust serde's own roundtrip behaviour. The fields' types
    // and derives are exercised by the other tests above.
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test -p uptrakit-plugin-hook-kernel-cleanup-apt config::tests
```

Expected: all four tests pass.

```bash
cargo clippy -p uptrakit-plugin-hook-kernel-cleanup-apt --all-targets -- -D warnings
```

Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add crates/plugins/hooks/kernel-cleanup-apt/src/config.rs
git commit -m "feat(kernel-cleanup-apt): KernelCleanupAptConfig + PluginConfig

Fields: keep_n (default 2, min 2 via validate), dry_run (default
false), min_boot_free_kib (default 51200, 0 disables /boot gating).
Form schema lands so the Dashboard config editor renders the
right fields with help text."
```

---

## Task 5: Implement `decisions.rs` pure functions (TDD)

**Files:**

- Modify: `crates/plugins/hooks/kernel-cleanup-apt/src/decisions.rs`

- [ ] **Step 1: Write the types + failing tests first**

Overwrite `crates/plugins/hooks/kernel-cleanup-apt/src/decisions.rs`. This file
is large; write the entire intended content in one pass to avoid drift:

````rust
//! Pure parsing + decision functions for the kernel-cleanup-apt hook plugin.
//!
//! All logic that doesn't touch the executor lives here so the
//! interesting paths can be unit-tested without spawning processes.

use std::collections::{BTreeSet, HashSet};

use crate::error::Result;

/// Which companion variants of a given kernel KVER are installed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct KernelVariantSet {
    pub signed: bool,
    pub unsigned: bool,
    pub modules: bool,
    pub modules_extra: bool,
    pub headers: bool,
}

impl KernelVariantSet {
    pub fn has_image(&self) -> bool {
        self.signed || self.unsigned
    }
}

/// One installed concrete kernel, keyed on KVER (uname-r-shaped).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelEntry {
    pub kver: String,
    pub installed_variants: KernelVariantSet,
}

/// Outcome of `compute_keep_and_purge_sets`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeepDecision {
    pub keep: Vec<String>,
    pub purge: Vec<String>,
    pub abort_reason: Option<String>,
}

/// Meta-package names that `parse_dpkg_kernel_list` MUST drop.
///
/// These never carry a `<KVER>` suffix and represent the operator's
/// stable attachment target — touching them would either no-op or
/// break apt's dependency resolution.
const META_PACKAGE_PREFIXES: &[&str] = &[
    "linux-image-generic",
    "linux-image-virtual",
    "linux-image-aws",
    "linux-image-azure",
    "linux-image-gcp",
    "linux-image-oracle",
    "linux-image-amd64",
    "linux-image-arm64",
    "linux-image-686",
    "linux-image-686-pae",
    "linux-image-unsigned-generic",
    "linux-image-unsigned-virtual",
    "linux-image-unsigned-aws",
];

/// Parse the stdout of:
///
/// ```text
/// dpkg-query --show --showformat='${Package}\t${Status}\n' \
///   'linux-image-*' 'linux-image-unsigned-*' 'linux-modules-*' \
///   'linux-headers-*' 'linux-modules-extra-*'
/// ```
///
/// Returns one `KernelEntry` per concrete KVER, with companion
/// packages (`linux-modules-<KVER>`, `linux-modules-extra-<KVER>`,
/// `linux-headers-<KVER>`) folded into the matching entry's
/// `installed_variants`. Meta-package rows and HWE rollup rows are
/// dropped. Rows whose status is not `install ok installed` are
/// dropped.
///
/// # Errors
///
/// Never errors today; returns an empty `Vec` on completely
/// unparseable input. Future tightening may surface
/// `KernelCleanupAptError::OutputParse`.
pub fn parse_dpkg_kernel_list(output: &str) -> Result<Vec<KernelEntry>> {
    use std::collections::BTreeMap;
    let mut by_kver: BTreeMap<String, KernelVariantSet> = BTreeMap::new();

    for raw_line in output.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        // tab-separated: "<Package>\t<Status>"
        let Some((package, status)) = line.split_once('\t') else {
            continue; // malformed row, drop silently
        };
        let package = package.trim();
        let status = status.trim();
        if status != "install ok installed" {
            continue;
        }
        if META_PACKAGE_PREFIXES.iter().any(|m| package == *m) {
            continue;
        }
        if package.starts_with("linux-image-generic-hwe-")
            || package.starts_with("linux-image-unsigned-generic-hwe-")
        {
            continue;
        }

        // Classify the package and extract KVER.
        let (kver_owned, mut update): (String, Box<dyn FnMut(&mut KernelVariantSet)>) =
            if let Some(rest) = package.strip_prefix("linux-image-unsigned-") {
                (rest.to_string(), Box::new(|v| v.unsigned = true))
            } else if let Some(rest) = package.strip_prefix("linux-image-") {
                (rest.to_string(), Box::new(|v| v.signed = true))
            } else if let Some(rest) = package.strip_prefix("linux-modules-extra-") {
                (rest.to_string(), Box::new(|v| v.modules_extra = true))
            } else if let Some(rest) = package.strip_prefix("linux-modules-") {
                (rest.to_string(), Box::new(|v| v.modules = true))
            } else if let Some(rest) = package.strip_prefix("linux-headers-") {
                (rest.to_string(), Box::new(|v| v.headers = true))
            } else {
                continue;
            };

        // KVER must look like a kernel ABI (digits + dots, optional flavor).
        // Reject obvious meta/flavor leftovers ("generic", "amd64", etc.).
        if !kver_looks_concrete(&kver_owned) {
            continue;
        }

        let entry = by_kver.entry(kver_owned).or_default();
        update(entry);
    }

    // Drop entries that have NO image variant — companion-only rows
    // should not surface as standalone KernelEntries.
    let entries = by_kver
        .into_iter()
        .filter_map(|(kver, variants)| {
            if variants.has_image() {
                Some(KernelEntry {
                    kver,
                    installed_variants: variants,
                })
            } else {
                None
            }
        })
        .collect();

    Ok(entries)
}

/// Heuristic: a concrete KVER starts with a digit (e.g.
/// `6.8.0-45-generic`, `5.15.0-67-aws`). Meta-package leftover
/// suffixes like `"generic"` / `"amd64"` start with letters and are
/// filtered out.
fn kver_looks_concrete(kver: &str) -> bool {
    kver.chars().next().is_some_and(|c| c.is_ascii_digit())
}

/// Parse `apt-mark showhold` stdout into a set of held KVERs.
///
/// Only kernel-image package holds count; non-kernel holds are
/// ignored.
///
/// # Errors
///
/// Never errors; malformed input yields an empty set.
pub fn parse_apt_mark_holds(output: &str) -> Result<HashSet<String>> {
    let mut out = HashSet::new();
    for line in output.lines() {
        let pkg = line.trim();
        if pkg.is_empty() {
            continue;
        }
        if let Some(rest) = pkg.strip_prefix("linux-image-unsigned-") {
            if kver_looks_concrete(rest) {
                out.insert(rest.to_string());
            }
        } else if let Some(rest) = pkg.strip_prefix("linux-image-") {
            if kver_looks_concrete(rest) {
                out.insert(rest.to_string());
            }
        }
    }
    Ok(out)
}

/// Decide which KVERs to keep and which to purge.
///
/// Rules:
///
/// 1. If `running_kver` has no matching entry in `entries` (no
///    `linux-image-*` or `linux-image-unsigned-*` package installed
///    for that KVER), set `abort_reason` and return — never proceed.
/// 2. Keep the running kernel and the latest (highest-versioned)
///    installed kernel unconditionally.
/// 3. Keep every held KVER unconditionally; holds do not consume
///    `keep_n` slots.
/// 4. Starting from the latest, walk descending and keep entries
///    until the unique keep set has `keep_n` members (running and
///    latest count toward `keep_n`).
/// 5. Anything not kept is purged.
///
/// # Errors
///
/// Never errors; abort conditions are returned via
/// `KeepDecision.abort_reason`.
pub fn compute_keep_and_purge_sets(
    entries: &[KernelEntry],
    running_kver: &str,
    held: &HashSet<String>,
    keep_n: u8,
) -> Result<KeepDecision> {
    let keep_n = usize::from(keep_n.max(2));

    // sort descending by KVER (version-aware sort)
    let mut sorted: Vec<&KernelEntry> = entries.iter().collect();
    sorted.sort_by(|a, b| version_compare(&b.kver, &a.kver));

    // Abort: running kernel has no matching entry.
    let running_present = sorted.iter().any(|e| e.kver == running_kver);
    if !running_present {
        return Ok(KeepDecision {
            keep: Vec::new(),
            purge: Vec::new(),
            abort_reason: Some(format!(
                "running kernel {running_kver} has no matching installed linux-image package"
            )),
        });
    }

    // Always-kept set.
    let mut keep_set: BTreeSet<String> = BTreeSet::new();
    keep_set.insert(running_kver.to_string());
    if let Some(latest) = sorted.first() {
        keep_set.insert(latest.kver.clone());
    }
    // Held entries are always kept on top of keep_n.
    for kver in held {
        if sorted.iter().any(|e| &e.kver == kver) {
            keep_set.insert(kver.clone());
        }
    }

    // Top up keep_set with the next-newest entries until we reach
    // keep_n. Held kernels already in keep_set do NOT count toward
    // keep_n in the sense that they survive even if we'd otherwise
    // exceed keep_n; we ensure at least keep_n non-held kernels are
    // retained.
    let non_held_keep_count = keep_set.iter().filter(|k| !held.contains(*k)).count();
    let mut needed = keep_n.saturating_sub(non_held_keep_count);
    for entry in &sorted {
        if needed == 0 {
            break;
        }
        if !keep_set.contains(&entry.kver) {
            keep_set.insert(entry.kver.clone());
            needed = needed.saturating_sub(1);
        }
    }

    let purge: Vec<String> = sorted
        .iter()
        .filter(|e| !keep_set.contains(&e.kver))
        .map(|e| e.kver.clone())
        .collect();

    let keep: Vec<String> = keep_set.into_iter().collect();

    Ok(KeepDecision {
        keep,
        purge,
        abort_reason: None,
    })
}

/// Compare two kernel KVERs in version order.
///
/// Accepts both Debian/Ubuntu shape (`6.8.0-45-generic`) and
/// suffix-flavored shape (`5.15.0-67-aws`). Splits on
/// `[.\-_+]`, compares numeric chunks numerically and alphabetic
/// chunks lexicographically.
fn version_compare(a: &str, b: &str) -> std::cmp::Ordering {
    let split = |s: &str| -> Vec<String> {
        s.split(|c: char| !c.is_ascii_alphanumeric())
            .map(str::to_string)
            .collect()
    };
    let av = split(a);
    let bv = split(b);
    for (x, y) in av.iter().zip(bv.iter()) {
        let order = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(xi), Ok(yi)) => xi.cmp(&yi),
            _ => x.cmp(y),
        };
        if order != std::cmp::Ordering::Equal {
            return order;
        }
    }
    av.len().cmp(&bv.len())
}

/// Build the explicit purge argv for one purge-set KVER.
///
/// Returns the package names that should be passed to
/// `apt-get purge --yes`, in canonical order. Skips any variant
/// that is not in `installed_variants` so we never ask apt to
/// purge a non-installed package.
pub fn purge_argv_for_kver(kver: &str, variants: &KernelVariantSet) -> Vec<String> {
    let mut out = Vec::new();
    if variants.signed {
        out.push(format!("linux-image-{kver}"));
    }
    if variants.unsigned {
        out.push(format!("linux-image-unsigned-{kver}"));
    }
    if variants.modules {
        out.push(format!("linux-modules-{kver}"));
    }
    if variants.modules_extra {
        out.push(format!("linux-modules-extra-{kver}"));
    }
    if variants.headers {
        out.push(format!("linux-headers-{kver}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dpkg_line(pkg: &str) -> String {
        format!("{pkg}\tinstall ok installed\n")
    }

    fn dpkg_line_deinstall(pkg: &str) -> String {
        format!("{pkg}\tdeinstall ok config-files\n")
    }

    #[test]
    fn parse_three_concrete_kernels() {
        let mut input = String::new();
        for k in &["6.8.0-45-generic", "6.8.0-50-generic", "6.8.0-40-generic"] {
            input.push_str(&dpkg_line(&format!("linux-image-{k}")));
            input.push_str(&dpkg_line(&format!("linux-modules-{k}")));
            input.push_str(&dpkg_line(&format!("linux-headers-{k}")));
        }
        let entries = parse_dpkg_kernel_list(&input).unwrap();
        assert_eq!(entries.len(), 3);
        assert!(entries.iter().all(|e| e.installed_variants.signed));
        assert!(entries.iter().all(|e| e.installed_variants.modules));
        assert!(entries.iter().all(|e| e.installed_variants.headers));
        assert!(entries.iter().all(|e| !e.installed_variants.unsigned));
    }

    #[test]
    fn parse_drops_meta_packages() {
        let input = dpkg_line("linux-image-amd64")
            + &dpkg_line("linux-image-generic")
            + &dpkg_line("linux-image-virtual")
            + &dpkg_line("linux-image-generic-hwe-22.04")
            + &dpkg_line("linux-image-6.8.0-45-generic");
        let entries = parse_dpkg_kernel_list(&input).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kver, "6.8.0-45-generic");
    }

    #[test]
    fn parse_drops_deinstall_status() {
        let input = dpkg_line("linux-image-6.8.0-50-generic")
            + &dpkg_line_deinstall("linux-image-6.8.0-40-generic");
        let entries = parse_dpkg_kernel_list(&input).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kver, "6.8.0-50-generic");
    }

    #[test]
    fn parse_handles_empty_input() {
        let entries = parse_dpkg_kernel_list("").unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_handles_malformed_lines() {
        let input = "this is not a dpkg row\nlinux-image-6.8.0-45-generic install ok installed\n"
            .to_string()
            + &dpkg_line("linux-image-6.8.0-50-generic");
        let entries = parse_dpkg_kernel_list(&input).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kver, "6.8.0-50-generic");
    }

    #[test]
    fn parse_canonicalises_unsigned_alongside_signed() {
        let kver = "6.8.0-45-generic";
        let input = dpkg_line(&format!("linux-image-{kver}"))
            + &dpkg_line(&format!("linux-image-unsigned-{kver}"))
            + &dpkg_line(&format!("linux-modules-{kver}"));
        let entries = parse_dpkg_kernel_list(&input).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].installed_variants.signed);
        assert!(entries[0].installed_variants.unsigned);
        assert!(entries[0].installed_variants.modules);
    }

    #[test]
    fn parse_canonicalises_unsigned_only_kver() {
        let kver = "6.1.0-30-amd64";
        let input = dpkg_line(&format!("linux-image-unsigned-{kver}"))
            + &dpkg_line(&format!("linux-modules-{kver}"));
        let entries = parse_dpkg_kernel_list(&input).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(!entries[0].installed_variants.signed);
        assert!(entries[0].installed_variants.unsigned);
        assert!(entries[0].installed_variants.modules);
    }

    #[test]
    fn parse_drops_companion_only_rows() {
        // `linux-modules-extra-X` with no matching image package
        // must not surface as a standalone KernelEntry.
        let input = dpkg_line("linux-modules-extra-6.8.0-45-generic");
        let entries = parse_dpkg_kernel_list(&input).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_apt_mark_holds_empty() {
        assert!(parse_apt_mark_holds("").unwrap().is_empty());
    }

    #[test]
    fn parse_apt_mark_holds_single_kernel() {
        let holds = parse_apt_mark_holds("linux-image-6.8.0-45-generic\n").unwrap();
        assert!(holds.contains("6.8.0-45-generic"));
    }

    #[test]
    fn parse_apt_mark_holds_ignores_non_kernel() {
        let holds = parse_apt_mark_holds(
            "nginx\nlinux-image-6.8.0-45-generic\nlinux-image-unsigned-6.1.0-30-amd64\n",
        )
        .unwrap();
        assert_eq!(holds.len(), 2);
        assert!(holds.contains("6.8.0-45-generic"));
        assert!(holds.contains("6.1.0-30-amd64"));
    }

    fn entry(kver: &str) -> KernelEntry {
        KernelEntry {
            kver: kver.to_string(),
            installed_variants: KernelVariantSet {
                signed: true,
                modules: true,
                headers: true,
                ..Default::default()
            },
        }
    }

    #[test]
    fn compute_running_equals_latest_keeps_two() {
        let entries = vec![
            entry("6.8.0-50-generic"),
            entry("6.8.0-45-generic"),
            entry("6.8.0-40-generic"),
        ];
        let decision =
            compute_keep_and_purge_sets(&entries, "6.8.0-50-generic", &HashSet::new(), 2).unwrap();
        assert!(decision.abort_reason.is_none());
        assert!(decision.keep.contains(&"6.8.0-50-generic".to_string()));
        assert!(decision.keep.contains(&"6.8.0-45-generic".to_string()));
        assert!(decision.purge.contains(&"6.8.0-40-generic".to_string()));
        assert_eq!(decision.keep.len(), 2);
        assert_eq!(decision.purge.len(), 1);
    }

    #[test]
    fn compute_running_below_latest_keeps_running_and_latest() {
        let entries = vec![
            entry("6.8.0-50-generic"),
            entry("6.8.0-45-generic"),
            entry("6.8.0-40-generic"),
        ];
        let decision =
            compute_keep_and_purge_sets(&entries, "6.8.0-45-generic", &HashSet::new(), 2).unwrap();
        assert!(decision.abort_reason.is_none());
        assert!(decision.keep.contains(&"6.8.0-50-generic".to_string()));
        assert!(decision.keep.contains(&"6.8.0-45-generic".to_string()));
        assert!(decision.purge.contains(&"6.8.0-40-generic".to_string()));
    }

    #[test]
    fn compute_abort_when_running_unmatched() {
        let entries = vec![entry("6.8.0-50-generic"), entry("6.8.0-45-generic")];
        let decision =
            compute_keep_and_purge_sets(&entries, "5.15.0-customX", &HashSet::new(), 2).unwrap();
        let reason = decision.abort_reason.as_deref().unwrap();
        assert!(reason.contains("5.15.0-customX"));
        assert!(decision.keep.is_empty());
        assert!(decision.purge.is_empty());
    }

    #[test]
    fn compute_two_installed_no_op() {
        let entries = vec![entry("6.8.0-50-generic"), entry("6.8.0-45-generic")];
        let decision =
            compute_keep_and_purge_sets(&entries, "6.8.0-50-generic", &HashSet::new(), 2).unwrap();
        assert!(decision.purge.is_empty());
        assert_eq!(decision.keep.len(), 2);
    }

    #[test]
    fn compute_one_installed_no_op_no_abort() {
        let entries = vec![entry("6.8.0-50-generic")];
        let decision =
            compute_keep_and_purge_sets(&entries, "6.8.0-50-generic", &HashSet::new(), 2).unwrap();
        assert!(decision.abort_reason.is_none());
        assert!(decision.purge.is_empty());
        assert_eq!(decision.keep, vec!["6.8.0-50-generic".to_string()]);
    }

    #[test]
    fn compute_keep_three_with_four_installed() {
        let entries = vec![
            entry("6.8.0-50-generic"),
            entry("6.8.0-45-generic"),
            entry("6.8.0-40-generic"),
            entry("6.8.0-35-generic"),
        ];
        let decision =
            compute_keep_and_purge_sets(&entries, "6.8.0-50-generic", &HashSet::new(), 3).unwrap();
        assert_eq!(decision.keep.len(), 3);
        assert_eq!(decision.purge, vec!["6.8.0-35-generic".to_string()]);
    }

    #[test]
    fn compute_held_oldest_exempt_from_purge() {
        let entries = vec![
            entry("6.8.0-50-generic"),
            entry("6.8.0-45-generic"),
            entry("6.8.0-40-generic"),
        ];
        let mut held = HashSet::new();
        held.insert("6.8.0-40-generic".to_string());
        let decision =
            compute_keep_and_purge_sets(&entries, "6.8.0-50-generic", &held, 2).unwrap();
        assert!(decision.keep.contains(&"6.8.0-40-generic".to_string()));
        assert!(decision.purge.is_empty());
    }

    #[test]
    fn compute_running_unsigned_only_does_not_abort() {
        let mut kver_entry = KernelEntry {
            kver: "6.1.0-30-amd64".to_string(),
            installed_variants: KernelVariantSet::default(),
        };
        kver_entry.installed_variants.unsigned = true;
        kver_entry.installed_variants.modules = true;
        let entries = vec![kver_entry];
        let decision = compute_keep_and_purge_sets(
            &entries,
            "6.1.0-30-amd64",
            &HashSet::new(),
            2,
        )
        .unwrap();
        assert!(decision.abort_reason.is_none());
    }

    #[test]
    fn purge_argv_emits_only_installed_variants() {
        let variants = KernelVariantSet {
            unsigned: true,
            modules: true,
            ..Default::default()
        };
        let argv = purge_argv_for_kver("6.1.0-30-amd64", &variants);
        assert_eq!(
            argv,
            vec![
                "linux-image-unsigned-6.1.0-30-amd64".to_string(),
                "linux-modules-6.1.0-30-amd64".to_string(),
            ]
        );
    }

    #[test]
    fn purge_argv_signed_and_unsigned_together() {
        let variants = KernelVariantSet {
            signed: true,
            unsigned: true,
            modules: true,
            headers: true,
            ..Default::default()
        };
        let argv = purge_argv_for_kver("6.8.0-45-generic", &variants);
        assert!(argv.contains(&"linux-image-6.8.0-45-generic".to_string()));
        assert!(argv.contains(&"linux-image-unsigned-6.8.0-45-generic".to_string()));
        assert!(argv.contains(&"linux-modules-6.8.0-45-generic".to_string()));
        assert!(argv.contains(&"linux-headers-6.8.0-45-generic".to_string()));
        assert!(!argv.iter().any(|s| s.contains("modules-extra")));
    }
}
````

- [ ] **Step 2: Run all decisions tests**

```bash
cargo test -p uptrakit-plugin-hook-kernel-cleanup-apt decisions::tests
```

Expected: all tests pass. If `parse_dpkg_kernel_list` panics on the
malformed-line case, fix it before continuing — the spec requires graceful drop,
not panic.

```bash
cargo clippy -p uptrakit-plugin-hook-kernel-cleanup-apt --all-targets -- -D warnings
```

Expected: clean. If clippy flags the boxed-closure pattern in
`parse_dpkg_kernel_list`, rewrite using `match` arms over a `Variant` enum
rather than silencing the lint.

- [ ] **Step 3: Commit**

```bash
git add crates/plugins/hooks/kernel-cleanup-apt/src/decisions.rs
git commit -m "feat(kernel-cleanup-apt): decisions module (pure functions + tests)

KernelEntry { kver, installed_variants }, KernelVariantSet
(signed/unsigned/modules/modules_extra/headers bits), KeepDecision
{ keep, purge, abort_reason }. Three pure functions
(parse_dpkg_kernel_list, parse_apt_mark_holds,
compute_keep_and_purge_sets) plus purge_argv_for_kver.

Twenty-plus table-driven tests cover:
- meta-package and HWE-rollup filtering
- deinstall/config-files filtering
- malformed-line graceful drop
- signed/unsigned canonicalisation (both, signed-only, unsigned-only)
- companion-only row drops
- abort-when-running-unmatched invariant
- keep_n=2 with running==latest, running<latest, two installed, one
- keep_n=3 with four installed
- held kernels exempt from purge regardless of keep_n
- purge argv emits only installed variants"
```

---

## Task 6: Implement `KernelCleanupAptHookPlugin` and `declare_plugin!`

**Files:**

- Modify: `crates/plugins/hooks/kernel-cleanup-apt/src/plugin.rs`

- [ ] **Step 1: Write the plugin struct + descriptor + LifecycleHook impl**

**Type-conversion discipline note**: `execute_post_hook` returns `Result<()>`
(the framework's `Report<PluginError>`-based alias from
`uptrakit_plugin_infrastructure_core`), while internal helpers return
`Result<T>` (the crate-local `Report<KernelCleanupAptError>` alias from
`crate::error`). The body below uses explicit `match` arms + early
`return Ok(())` everywhere instead of `?` — so cross-error-type conversion never
actually fires in this function. The `impl_report_conversion!` registration from
Task 3 is in place for symmetry with the apt-plugin pattern and for any future
helper that does want to bubble via `?`. Do not "simplify" the match arms into
`?` without re-checking the conversion direction.

Overwrite `crates/plugins/hooks/kernel-cleanup-apt/src/plugin.rs`:

```rust
//! `hook_kernel_cleanup_apt` — opt-in post-update cleanup of
//! superseded Linux kernel packages on Debian/Ubuntu hosts.

use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_command::{CommandExecutor, CommandSpec};
use uptrakit_plugin_infrastructure_core::{
    ConfigModel, ConfigTestKind, HostCompatibility, HostRequirements, HostRuntime, LifecycleHook,
    OsFamily, PluginError, PluginFamily, PreUpdateHookResult, Result, SudoCommandEntry,
    UpdateLifecycleContext, UpdateOutputSender, declare_plugin, host_features,
};

use crate::config::KernelCleanupAptConfig;
use crate::decisions::{
    compute_keep_and_purge_sets, parse_apt_mark_holds, parse_dpkg_kernel_list,
    purge_argv_for_kver,
};
use crate::error::KernelCleanupAptError;

/// `hook_kernel_cleanup_apt` plugin instance.
pub struct KernelCleanupAptHookPlugin {
    config: KernelCleanupAptConfig,
    executor: Arc<dyn CommandExecutor>,
}

impl KernelCleanupAptHookPlugin {
    /// Construct the plugin. `PluginConfig::validate` is invoked by
    /// the registry before this; by the time `new()` runs the
    /// invariants (`keep_n >= 2`) already hold.
    pub fn new(
        config: KernelCleanupAptConfig,
        runtime: Arc<dyn HostRuntime>,
    ) -> std::result::Result<Self, String> {
        let executor = runtime.executor();
        Ok(Self { config, executor })
    }

    /// Sudo commands required by this plugin.
    ///
    /// One entry, suffix-restricted to
    /// `purge --yes linux-image-* linux-modules-* linux-headers-*`.
    /// The actual invocation passes an explicit per-`KVER` list; the
    /// sudoers wildcard is the safety net.
    fn required_sudo_commands(_config: &serde_json::Value) -> Vec<SudoCommandEntry> {
        vec![
            SudoCommandEntry::new("apt-get", "Purge old kernel packages")
                .with_args_suffix("purge --yes linux-image-* linux-modules-* linux-headers-*")
                .with_setenv(),
        ]
    }

    async fn run_quiet(&self, bin: &str, args: &[&str]) -> Result<String> {
        let owned: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        let out = self
            .executor
            .execute_quiet(&CommandSpec::exec(bin, owned))
            .await
            .map_err(|e| {
                // Match the apt plugin's existing pattern at
                // `crates/plugins/package-managers/apt/src/update.rs` —
                // format the underlying error into the message rather
                // than chaining .attach(...). If the actual rootcause
                // API has changed, defer to the latest in-tree usage,
                // not this plan's verbatim suggestion.
                report!(KernelCleanupAptError::CommandFailed {
                    command: format!("{bin} (executor error: {e})"),
                    exit_code: -1,
                })
            })?;
        if out.exit_code != 0 {
            return Err(report!(KernelCleanupAptError::CommandFailed {
                command: bin.to_string(),
                exit_code: out.exit_code,
            }));
        }
        Ok(out.output)
    }

    /// Probe `/boot` free space (KiB). `min_boot_free_kib == 0`
    /// disables the probe entirely; ambiguous output aborts.
    async fn probe_boot_free_kib(&self) -> std::result::Result<Option<u64>, String> {
        let owned: Vec<String> = vec!["--output=avail".to_string(), "/boot".to_string()];
        let out = self
            .executor
            .execute_quiet(&CommandSpec::exec("df", owned))
            .await
            .map_err(|e| format!("df probe failed: {e}"))?;
        if out.exit_code != 0 {
            return Err(format!("df exit {}", out.exit_code));
        }
        // df --output=avail prints a header then a single number.
        let last = out
            .output
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_digit()))
            .last()
            .ok_or_else(|| format!("df output had no numeric line: {:?}", out.output))?;
        let kib: u64 = last
            .parse()
            .map_err(|e| format!("df numeric parse failed: {e}"))?;
        Ok(Some(kib))
    }
}

static REQUIRED_FEATURES: [uptrakit_plugin_infrastructure_core::HostFeature; 1] =
    [host_features::POSIX_SHELL];

declare_plugin!(
    KernelCleanupAptHookPlugin,
    KernelCleanupAptConfig,
    "hook_kernel_cleanup_apt",
    {
        display_name: "Kernel Cleanup (APT)",
        family: PluginFamily::Hook,
        config_model: ConfigModel::PluginConfig,
        host_requirements: HostRequirements::new(&[OsFamily::Linux], &REQUIRED_FEATURES, false),
        config_test: [ConfigTestKind::PostUpdateHook],
        roles: [LifecycleHook],
        sudo: KernelCleanupAptHookPlugin::required_sudo_commands,
    }
);

#[async_trait]
impl LifecycleHook for KernelCleanupAptHookPlugin {
    /// Preflight: `apt-get` must be present.
    async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
        let owned = vec!["apt-get".to_string()];
        let out = self
            .executor
            .execute_quiet(&CommandSpec::exec("which", owned))
            .await
            .map_err(|e| {
                report!(KernelCleanupAptError::CommandFailed {
                    command: format!("which (executor error: {e})"),
                    exit_code: -1,
                })
            })?;
        if out.exit_code == 0 {
            Ok(HostCompatibility::Compatible)
        } else {
            Ok(HostCompatibility::Incompatible(
                "apt-get not found".to_string(),
            ))
        }
    }

    /// No-op pre-hook (this plugin operates strictly post-update).
    async fn execute_pre_hook(
        &self,
        _ctx: &UpdateLifecycleContext,
        _output_tx: &UpdateOutputSender,
    ) -> Result<PreUpdateHookResult> {
        Ok(PreUpdateHookResult::proceed())
    }

    /// Run the cleanup pipeline. Always returns `Ok(())` per the
    /// non-fatal `PostUpdateHook` contract — failures are logged
    /// via `tracing::warn!` with structured fields and emitted as
    /// `[post-hook] ...` lines on `output_tx` (best-effort; batch
    /// path discards them per docs/development/update-hooks.md).
    async fn execute_post_hook(
        &self,
        ctx: &UpdateLifecycleContext,
        output_tx: &UpdateOutputSender,
    ) -> Result<()> {
        let batch_id = ctx.batch_id;
        let plugin_type = "hook_kernel_cleanup_apt";

        tracing::info!(
            plugin_type,
            batch_id = ?batch_id,
            keep_n = self.config.keep_n,
            dry_run = self.config.dry_run,
            min_boot_free_kib = self.config.min_boot_free_kib,
            "kernel cleanup starting"
        );
        send_hook_line(
            output_tx,
            &format!(
                "[post-hook] kernel cleanup starting (keep_n={}, dry_run={})",
                self.config.keep_n, self.config.dry_run
            ),
        )
        .await;

        // Step 1: read uname -r. `utsname.release` is the kernel ABI
        // suffix (e.g. `6.8.0-45-generic`) and matches the dpkg
        // `linux-image-<KVER>` package name exactly. Custom-compiled
        // kernels with `CONFIG_LOCALVERSION_AUTO=y` append `-dirty` or
        // a git suffix; those kernels are never dpkg-installed, so the
        // abort-when-running-unmatched invariant fires correctly without
        // special-casing here.
        let running = match self.run_quiet("uname", &["-r"]).await {
            Ok(s) => s.trim().to_string(),
            Err(e) => {
                let msg = format!("failed to read uname -r: {}", e.current_context());
                tracing::warn!(plugin_type, batch_id = ?batch_id, error = %msg, "kernel cleanup aborted");
                send_hook_line(
                    output_tx,
                    &format!("[post-hook] kernel cleanup aborted: {msg}"),
                )
                .await;
                return Ok(());
            }
        };
        tracing::info!(plugin_type, batch_id = ?batch_id, running_kernel = %running, "running kernel detected");
        send_hook_line(output_tx, &format!("[post-hook] running kernel: {running}")).await;

        // Step 2: list installed kernel-family packages
        let dpkg_args = [
            "--show",
            "--showformat=${Package}\t${Status}\n",
            "linux-image-*",
            "linux-image-unsigned-*",
            "linux-modules-*",
            "linux-headers-*",
            "linux-modules-extra-*",
        ];
        let dpkg_out = match self.run_quiet("dpkg-query", &dpkg_args).await {
            Ok(s) => s,
            Err(e) => {
                let msg = format!("dpkg-query failed: {}", e.current_context());
                tracing::warn!(plugin_type, batch_id = ?batch_id, error = %msg, "kernel cleanup aborted");
                send_hook_line(
                    output_tx,
                    &format!("[post-hook] kernel cleanup aborted: {msg}"),
                )
                .await;
                return Ok(());
            }
        };
        let entries = match parse_dpkg_kernel_list(&dpkg_out) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(plugin_type, batch_id = ?batch_id, error = %e.current_context(), "dpkg parse failed");
                send_hook_line(output_tx, "[post-hook] kernel cleanup aborted: dpkg parse failed").await;
                return Ok(());
            }
        };
        let installed_kvers: Vec<String> = entries.iter().map(|e| e.kver.clone()).collect();
        tracing::info!(plugin_type, batch_id = ?batch_id, installed_kernels = ?installed_kvers, "installed kernels");
        send_hook_line(
            output_tx,
            &format!("[post-hook] installed kernels: {}", installed_kvers.join(", ")),
        )
        .await;

        // Step 3: list holds (best-effort)
        let holds = match self.run_quiet("apt-mark", &["showhold"]).await {
            Ok(s) => parse_apt_mark_holds(&s).unwrap_or_default(),
            Err(e) => {
                tracing::warn!(plugin_type, batch_id = ?batch_id, error = %e.current_context(), "apt-mark showhold failed; treating held set as empty");
                std::collections::HashSet::new()
            }
        };
        let held_list: Vec<String> = holds.iter().cloned().collect();
        tracing::info!(plugin_type, batch_id = ?batch_id, held_kernels = ?held_list, "held kernels");
        send_hook_line(
            output_tx,
            &format!(
                "[post-hook] held (apt-mark hold): {}",
                if held_list.is_empty() { "none".to_string() } else { held_list.join(", ") }
            ),
        )
        .await;

        // Step 4: decide
        let decision =
            match compute_keep_and_purge_sets(&entries, &running, &holds, self.config.keep_n) {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!(plugin_type, batch_id = ?batch_id, error = %e.current_context(), "compute_keep_and_purge_sets failed");
                    send_hook_line(output_tx, "[post-hook] kernel cleanup aborted: internal decision error").await;
                    return Ok(());
                }
            };
        if let Some(reason) = decision.abort_reason.as_deref() {
            tracing::warn!(plugin_type, batch_id = ?batch_id, abort_reason = %reason, "kernel cleanup aborted");
            send_hook_line(
                output_tx,
                &format!("[post-hook] kernel cleanup aborted: {reason}"),
            )
            .await;
            return Ok(());
        }
        tracing::info!(plugin_type, batch_id = ?batch_id, keep_set = ?decision.keep, purge_set = ?decision.purge, "decision");
        send_hook_line(
            output_tx,
            &format!("[post-hook] keep set: {}", decision.keep.join(", ")),
        )
        .await;
        if decision.purge.is_empty() {
            send_hook_line(output_tx, "[post-hook] nothing to purge; kernel cleanup completed").await;
            tracing::info!(plugin_type, batch_id = ?batch_id, "kernel cleanup completed (no-op)");
            return Ok(());
        }

        // Step 5: /boot free space
        let boot_free_before = if self.config.min_boot_free_kib == 0 {
            tracing::info!(plugin_type, batch_id = ?batch_id, "/boot gating disabled (min_boot_free_kib=0)");
            None
        } else {
            match self.probe_boot_free_kib().await {
                Ok(Some(kib)) => {
                    tracing::info!(plugin_type, batch_id = ?batch_id, boot_free_before_kib = kib, "/boot probe");
                    send_hook_line(output_tx, &format!("[post-hook] /boot free before: {kib} KiB")).await;
                    let threshold = u64::from(self.config.min_boot_free_kib);
                    if kib < threshold {
                        let msg = format!(
                            "/boot has only {kib} KiB free (need >= {threshold} KiB); update-initramfs may fail mid-purge"
                        );
                        tracing::warn!(plugin_type, batch_id = ?batch_id, boot_free_kib = kib, threshold_kib = threshold, "kernel cleanup aborted: /boot tight");
                        send_hook_line(
                            output_tx,
                            &format!("[post-hook] kernel cleanup aborted: {msg}"),
                        )
                        .await;
                        return Ok(());
                    }
                    Some(kib)
                }
                Ok(None) | Err(_) => {
                    tracing::warn!(plugin_type, batch_id = ?batch_id, "/boot probe ambiguous; aborting (set min_boot_free_kib=0 to override)");
                    send_hook_line(
                        output_tx,
                        "[post-hook] kernel cleanup aborted: /boot probe ambiguous; set min_boot_free_kib=0 to override",
                    )
                    .await;
                    return Ok(());
                }
            }
        };

        // Step 6: build explicit purge argv
        let mut purge_argv: Vec<String> = Vec::new();
        for kver in &decision.purge {
            if let Some(entry) = entries.iter().find(|e| &e.kver == kver) {
                purge_argv.extend(purge_argv_for_kver(kver, &entry.installed_variants));
            }
        }
        if purge_argv.is_empty() {
            send_hook_line(output_tx, "[post-hook] nothing concrete to purge; kernel cleanup completed").await;
            tracing::info!(plugin_type, batch_id = ?batch_id, "kernel cleanup completed (purge set resolved to empty)");
            return Ok(());
        }

        // Step 7: dry-run branch
        if self.config.dry_run {
            tracing::info!(plugin_type, batch_id = ?batch_id, would_purge = ?purge_argv, "dry run; not invoking apt-get");
            send_hook_line(
                output_tx,
                &format!("[post-hook] would purge: {}", purge_argv.join(" ")),
            )
            .await;
            return Ok(());
        }

        // Step 8: purge
        send_hook_line(
            output_tx,
            &format!("[post-hook] purging: {}", purge_argv.join(" ")),
        )
        .await;
        let mut purge_args: Vec<String> = vec!["purge".to_string(), "--yes".to_string()];
        purge_args.extend(purge_argv.clone());
        let purge_spec = CommandSpec::exec("apt-get", purge_args)
            .with_env("DEBIAN_FRONTEND", "noninteractive")
            .privileged();
        let exit_code = match self.executor.execute(&purge_spec, output_tx).await {
            Ok(out) => out.exit_code,
            Err(e) => {
                tracing::warn!(plugin_type, batch_id = ?batch_id, error = %e, "apt-get purge invocation failed");
                send_hook_line(output_tx, &format!("[post-hook] apt-get purge invocation failed: {e}")).await;
                return Ok(());
            }
        };
        tracing::info!(plugin_type, batch_id = ?batch_id, apt_purge_exit_code = exit_code, purge_count = purge_argv.len(), "apt-get purge complete");
        send_hook_line(output_tx, &format!("[post-hook] apt-get purge exit {exit_code}")).await;

        // Step 9: /boot free space after
        if let Some(before) = boot_free_before {
            if let Ok(Some(after)) = self.probe_boot_free_kib().await {
                tracing::info!(plugin_type, batch_id = ?batch_id, boot_free_before_kib = before, boot_free_after_kib = after, "/boot delta");
                send_hook_line(output_tx, &format!("[post-hook] /boot free after: {after} KiB")).await;
            }
        }

        // Step 10: done
        send_hook_line(output_tx, "[post-hook] kernel cleanup completed").await;
        tracing::info!(plugin_type, batch_id = ?batch_id, "kernel cleanup completed");
        Ok(())
    }
}

async fn send_hook_line(output_tx: &UpdateOutputSender, line: &str) {
    use uptrakit_plugin_infrastructure_core::command::send_output;
    use uptrakit_plugin_infrastructure_core::OutputStreamType;
    // PostHook (not Stdout) is the correct stream classification: this
    // plugin emits exclusively during `execute_post_hook`, matching the
    // way the agent-core dispatcher labels post-hook framework output.
    send_output(output_tx, line, OutputStreamType::PostHook).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_command::LocalCommandExecutor;
    use uptrakit_plugin_infrastructure_core::{
        HostCapabilities, PluginCapability, PluginMeta, StandardHostRuntime,
    };

    fn test_plugin(config: KernelCleanupAptConfig) -> KernelCleanupAptHookPlugin {
        let executor: Arc<dyn CommandExecutor> = Arc::new(LocalCommandExecutor);
        let caps = HostCapabilities::default();
        let runtime: Arc<dyn HostRuntime> =
            Arc::new(StandardHostRuntime::new(executor, caps));
        KernelCleanupAptHookPlugin::new(config, runtime).unwrap()
    }

    #[test]
    fn plugin_type_id() {
        let p = test_plugin(KernelCleanupAptConfig::default());
        assert_eq!(p.plugin_type_id().as_str(), "hook_kernel_cleanup_apt");
    }

    #[test]
    fn descriptor_capabilities() {
        assert!(DESCRIPTOR.capabilities.contains(&PluginCapability::UpdateLifecycle));
        assert!(DESCRIPTOR.capabilities.contains(&PluginCapability::ConfigTest));
    }

    #[test]
    fn descriptor_has_lifecycle_hook_role_and_no_software_roles() {
        assert!(DESCRIPTOR.roles.lifecycle_hook.is_some());
        assert!(DESCRIPTOR.roles.discoverer.is_none());
        assert!(DESCRIPTOR.roles.version_detector.is_none());
        assert!(DESCRIPTOR.roles.update_executor.is_none());
        assert!(DESCRIPTOR.roles.release_fetcher.is_none());
    }

    #[test]
    fn descriptor_has_one_sudo_entry_with_purge_prefix() {
        let cmds = (DESCRIPTOR.sudo.unwrap())(&serde_json::json!({}));
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].command, "apt-get");
        // The suffix is stored on the entry as an opaque string; we
        // re-render to inspect.
        let rendered = format!("{:?}", cmds[0]);
        assert!(rendered.contains("purge --yes linux-image-*"));
    }
}
```

> If `LocalCommandExecutor` is invoked on a host without `apt-get` (most CI
> hosts), `detect_host_compatibility` will return
> `Incompatible("apt-get not found")` — the descriptor tests above intentionally
> do not exercise the live path. Live execution is covered by the manual VM run
> in Task 10.

- [ ] **Step 2: Build + test**

```bash
cargo build -p uptrakit-plugin-hook-kernel-cleanup-apt
cargo test  -p uptrakit-plugin-hook-kernel-cleanup-apt --all-features
```

Expected: all green. If `send_output` is not exposed from
`uptrakit_plugin_infrastructure_core::command`, adjust the import to match the
existing path used by `hook_systemd::plugin::run_systemctl` (the analogous call
site). Fix the import root cause; do not bypass with a `#[allow(unused)]`.

```bash
cargo clippy -p uptrakit-plugin-hook-kernel-cleanup-apt --all-targets -- -D warnings
```

Expected: clean. Any `#[expect]` introduced must carry a `reason = "..."` field
per snapshot rule.

- [ ] **Step 3: Commit**

```bash
git add crates/plugins/hooks/kernel-cleanup-apt/src/plugin.rs
git commit -m "feat(kernel-cleanup-apt): plugin + LifecycleHook impl

declare_plugin! with PluginFamily::Hook, single LifecycleHook role,
ConfigTestKind::PostUpdateHook, one sudoers entry. Overrides
detect_host_compatibility to probe \`which apt-get\`. Cleanup
pipeline (10 steps) is non-fatal, emits structured tracing::info!
/ tracing::warn! events keyed on plugin_type + batch_id for v1
audit trail."
```

---

## Task 7: Register the plugin in the descriptor catalog

**Files:**

- Modify: `crates/plugins/infrastructure/registry/Cargo.toml`
- Modify: `crates/plugins/infrastructure/registry/src/registry.rs` (lines 55-57)

- [ ] **Step 1: Add the crate dependency**

Add to `[dependencies]` in `crates/plugins/infrastructure/registry/Cargo.toml`,
keeping the existing alphabetical ordering near the other hook crates:

```toml
uptrakit-plugin-hook-kernel-cleanup-apt = { workspace = true }
```

- [ ] **Step 2: Register the descriptor**

Open `crates/plugins/infrastructure/registry/src/registry.rs`. Find the existing
block:

```rust
        &uptrakit_plugin_hook_shell::DESCRIPTOR,
        &uptrakit_plugin_hook_systemd::DESCRIPTOR,
```

Insert:

```rust
        &uptrakit_plugin_hook_kernel_cleanup_apt::DESCRIPTOR,
```

Maintain alphabetical order within the hook group: `kernel` < `shell` <
`systemd`, so place the new descriptor reference **before** the
`uptrakit_plugin_hook_shell::DESCRIPTOR` line.

- [ ] **Step 3: Build + test the registry**

```bash
cargo build -p uptrakit-plugin-infrastructure-registry
cargo test  -p uptrakit-plugin-infrastructure-registry --all-features
```

Expected: success. Any test that enumerates known plugin types should now
include `hook_kernel_cleanup_apt`; if such a test fails, update its expected set
rather than excluding the new plugin.

- [ ] **Step 4: Workspace smoke**

```bash
cargo check --all-features
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/plugins/infrastructure/registry/Cargo.toml \
        crates/plugins/infrastructure/registry/src/registry.rs
git commit -m "feat(plugin-registry): register hook_kernel_cleanup_apt descriptor"
```

---

## Task 8: Write `docs/adr/0010-host-scoped-housekeeping-via-meta-package-hooks.md`

**Files:**

- Create: `docs/adr/0010-host-scoped-housekeeping-via-meta-package-hooks.md`

- [ ] **Step 1: Inspect existing ADR format**

```bash
head -60 docs/adr/0006-instance-scoped-plugins.md
```

Adopt the same header convention (status, date, context, decision, alternatives,
consequences).

- [ ] **Step 2: Author the ADR**

Create `docs/adr/0010-host-scoped-housekeeping-via-meta-package-hooks.md` with
this content:

```markdown
# 0010 — Host-Scoped Housekeeping via Meta-Package Hooks

- Status: Accepted
- Date: 2026-05-12
- Spec: `docs/superpowers/specs/2026-05-12-hook-kernel-cleanup-apt-design.md`

## Context

We need a way for hook plugins like `hook_kernel_cleanup_apt` to fire reliably
on every kernel ABI bump on Debian and Ubuntu hosts. The hook assignment model
is per-(host, software_item) via `host_software_item_plugins`. Concrete kernel
packages (`linux-image-6.8.0-45-generic`) are dynamic — each ABI bump creates a
brand-new Software Item the operator cannot have pre-attached to. The natural
Software Item that ticks on every kernel ABI bump is the kernel meta-package
(`linux-image-amd64`, `linux-image-generic`, flavors). Apt's `Depends:` line
bumps the meta-package whenever a new concrete kernel ships.

## Decision

Operators opt in to `hook_kernel_cleanup_apt` by attaching it to the
**existing** meta-package Software Item on each Host. The hook fires post-batch
when the meta-package is in the upgrade batch (i.e., on every kernel ABI bump).
No new schema, no new assignment surface, no virtual Software Items, no
host-scoped hook concept.

We additionally add two non-breaking framework primitives
(`UpdateLifecycleContext.batch_id` and
`LifecycleHook::detect_host_compatibility`) so per-batch dedup and per-host
compatibility filtering are first-class concerns shared by every future Hook
plugin.

## Alternatives considered

1. **Host-scoped hooks** — nullable `software_item_id` on
   `host_software_item_plugins`, or a new `host_hook_assignments` table. Adds
   wire payload field + agent dispatch loop change for one feature; YAGNI.
2. **Virtual "kernel" Software Item** — APT plugin synthesises a single
   canonical kernel item per host. Requires apt plugin discovery + version
   detection changes; cross-distro lift needed for DNF. Cleaner separation but
   disproportionate scope.
3. **Per-concrete-package attachment** — operator must re-attach after every
   kernel bump. Discoverability cliff; silent gaps.
4. **Flag inside `AptConfig.kernel_cleanup`** — conflates Software-family
   (install) with Hook-family (cleanup); sudoers sprawl; hides the lifecycle
   step in the package manager.
5. **Instance-Scoped sweeper Enhancement plugin** — controller- driven scheduled
   sweep with new wire variant. ADR 0006 established Enhancement-family is
   controller-only; agent-side dispatch requires a new wire surface that v1
   cannot justify.
6. **New `HousekeepingHook` lifecycle phase** — speculative generality; one
   concrete use case does not justify a new role
   - DB column + wire field + frontend modal.

## Consequences

### Positive

- Reuses the established per-(host, software_item) seam.
- Mirrors the existing `hook_systemd` / `hook_shell` plugin shape exactly.
- Two framework primitives (`batch_id`, preflight) are reusable by every future
  Hook plugin.
- Zero wire-protocol changes; zero DB migrations.

### Negative

- Operators must know which meta-package to attach to for their host flavor
  (`linux-image-amd64`, `linux-image-generic`, `-virtual`, `-aws`, `-arm64`, HWE
  variants). The plugin operator doc lists the supported matrix.
- Non-flavored / vendor / hand-compiled kernels (Proxmox `pve-kernel-*`,
  Raspberry Pi `linux-image-raspi*`, vendor-OEM `linux-oem-*`,
  `dpkg -i`-installed kernels) are silent no-ops in v1; documented as "not
  supported."
- HWE rollover (e.g. `linux-image-generic` → `linux-image-generic-hwe-XX.YY`)
  requires the operator to reattach the hook. Documented.

### Neutral / accepted

- Batch-path hook output is currently discarded (`client.rs:521`'s
  `_output_rx`). v1 audit trail is structured `tracing::info!` events; a
  follow-up framework spec must redesign batch-path output capture.
```

- [ ] **Step 3: Lint**

```bash
markdownlint --config .markdownlint.json docs/adr/0010-host-scoped-housekeeping-via-meta-package-hooks.md
```

Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add docs/adr/0010-host-scoped-housekeeping-via-meta-package-hooks.md
git commit -m "docs(adr): 0010 — host-scoped housekeeping via meta-package hooks

Records why kernel-cleanup-apt fits the existing per-(host,
software_item) hook seam through meta-package attachment, and the
five alternatives that were rejected (host-scoped hooks, virtual
SI, per-concrete attachment, AptConfig flag, sweeper Enhancement,
new lifecycle phase)."
```

---

## Task 9: Write operator-facing plugin doc + kernel-housekeeping runbook + crate README

**Files:**

- Create: `docs/end-user/plugins/hook_kernel_cleanup_apt.md`
- Create: `docs/end-user/operations/kernel-housekeeping.md`
- Create: `crates/plugins/hooks/kernel-cleanup-apt/README.md`
- Modify: `docs/development/plugin-guidelines.md`

- [ ] **Step 1: Identify a sibling doc to mirror**

```bash
ls docs/end-user/plugins/ | head -20
```

Mirror the structure of any existing plugin doc (e.g. `apt.md`,
`systemd-hook.md` if present).

- [ ] **Step 2: Author `docs/end-user/plugins/hook_kernel_cleanup_apt.md`**

Create `docs/end-user/plugins/hook_kernel_cleanup_apt.md`:

````markdown
# Hook: Kernel Cleanup (APT)

`hook_kernel_cleanup_apt` purges superseded Linux kernel packages on Debian and
Ubuntu Hosts after each kernel update. It is strictly opt-in: a Host's kernel
meta-package Software Item must have the plugin attached before any cleanup
occurs.

> Cross-distro context, audit recipes, and regulated-environment guidance live
> in `docs/end-user/operations/kernel-housekeeping.md`. This page is the plugin
> reference only.

## Supported meta-package matrix

Attach the hook to **the** meta-package Software Item your Host runs:

| Distribution       | Meta-package                              | Notes                          |
| :----------------- | :---------------------------------------- | :----------------------------- |
| Debian amd64       | `linux-image-amd64`                       | Bumps on every kernel ABI tick |
| Debian arm64       | `linux-image-arm64`                       |                                |
| Debian i386        | `linux-image-686` / `linux-image-686-pae` |                                |
| Ubuntu generic     | `linux-image-generic`                     |                                |
| Ubuntu HWE         | `linux-image-generic-hwe-XX.YY`           | Reattach after HWE rollover    |
| Ubuntu virtual     | `linux-image-virtual`                     | Cloud images                   |
| Ubuntu AWS         | `linux-image-aws`                         | EC2 AMIs                       |
| Debian Secure Boot | `linux-image-unsigned-*` family           | Auto-detected alongside signed |

**Not supported in v1:**

- Proxmox `pve-kernel-*`
- Raspberry Pi `linux-image-raspi*`
- Vendor / OEM kernels (`linux-oem-*`)
- ZFS-on-root (`linux-image-zfs`)
- Manually compiled kernels installed via `dpkg -i` outside an apt-managed pool

On these hosts the hook is a silent no-op (the running kernel will not match the
parser's KVER set and cleanup aborts).

## Configuration

| Field               | Type | Default | Description                                                                                                                                                                               |
| :------------------ | :--- | :------ | :---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `keep_n`            | u8   | `2`     | Total installed kernels to retain. Running and latest are always kept. Min `2`.                                                                                                           |
| `dry_run`           | bool | `false` | Log `[post-hook] would purge: ...` without invoking `apt-get purge`.                                                                                                                      |
| `min_boot_free_kib` | u32  | `51200` | Required free space on `/boot` (KiB) to proceed. `0` disables `/boot` gating (escape hatch for bind-mounted `/boot`, btrfs subvolumes, LVM-on-`/boot`, or tiny VMs that accept the risk). |

## Sudoers entry

The agent's sudoers includes one new line when this plugin is assigned to at
least one Software Item on the Host:

```text
uptrakit ALL=(root) NOPASSWD: SETENV: /usr/bin/apt-get purge --yes linux-image-* linux-modules-* linux-headers-*
```
````

The sudoers wildcard `linux-modules-*` covers `linux-modules-extra-*` via greedy
match. The runtime invocation passes an explicit per-`KVER` list (computed from
`dpkg-query`); the wildcard is the safety net.

## Sample output

### Successful purge

```text
[post-hook] kernel cleanup starting (keep_n=2, dry_run=false)
[post-hook] running kernel: 6.8.0-45-generic
[post-hook] installed kernels: 6.8.0-50-generic, 6.8.0-45-generic, 6.8.0-40-generic
[post-hook] held (apt-mark hold): none
[post-hook] keep set: 6.8.0-45-generic, 6.8.0-50-generic
[post-hook] /boot free before: 102400 KiB
[post-hook] purging: linux-image-6.8.0-40-generic linux-modules-6.8.0-40-generic linux-headers-6.8.0-40-generic
...apt-get output...
[post-hook] apt-get purge exit 0
[post-hook] /boot free after: 582400 KiB
[post-hook] kernel cleanup completed
```

### Dry run

```text
[post-hook] kernel cleanup starting (keep_n=2, dry_run=true)
...
[post-hook] would purge: linux-image-6.8.0-40-generic linux-modules-6.8.0-40-generic linux-headers-6.8.0-40-generic
```

### Abort: running kernel not installed

```text
[post-hook] kernel cleanup aborted: running kernel 5.15.0-customX has no matching installed linux-image package
```

### Abort: `/boot` too tight

```text
[post-hook] kernel cleanup aborted: /boot has only 32768 KiB free (need >= 51200 KiB); update-initramfs may fail mid-purge
```

### Abort: `/boot` probe ambiguous

```text
[post-hook] kernel cleanup aborted: /boot probe ambiguous; set min_boot_free_kib=0 to override
```

## HWE rollover playbook

When Ubuntu rolls a Host from `linux-image-generic` (kernel 5.15) to
`linux-image-generic-hwe-22.04` (kernel 6.x), the old meta-package is replaced
and stops ticking on new kernel updates. Reattach the hook to the new
meta-package Software Item:

1. Wait for software discovery to surface the new meta-package on the Host.
2. Detach the hook from the old meta-package's Software Item.
3. Attach the hook to the new meta-package's Software Item with the same config.
4. Optionally remove the old meta-package's Software Item from tracking.

## `apt-mark hold` interaction

Kernels held via `apt-mark hold linux-image-<KVER>` (or
`linux-image-unsigned-<KVER>`) are exempt from cleanup. They are kept on top of
`keep_n`, not in place of it. Use this for kernels you want to keep as a
known-good fallback regardless of how many newer ones are installed.

## dkms / custom-driver caveat

Cleanup purges `linux-headers-<KVER>` for old KVERs. If a third- party dkms
driver is built only against an old kernel header set and you are not yet
running a newer kernel, hold both the old kernel and its headers
(`apt-mark hold linux-image-<KVER> linux-headers-<KVER>`) until the driver is
rebuilt for the new kernel.

## Discovery race caveat

If you attach the hook before software discovery has surfaced the meta-package
Software Item on a Host, no batches will carry the hook assignment. Verify the
meta-package appears in the Host's inventory before relying on cleanup. Most
operators encounter this only once, on first attachment of a fresh Host.

## Audit and observability

See `docs/end-user/operations/kernel-housekeeping.md` for the `journalctl` audit
recipe and regulated-environment guidance.

````text

- [ ] **Step 3: Author `docs/end-user/operations/kernel-housekeeping.md`**

Create `docs/end-user/operations/kernel-housekeeping.md`:

```markdown
# Kernel Housekeeping

This runbook covers the strategies uptrakit supports for cleaning
up old Linux kernel packages after updates, across distributions.

## Debian / Ubuntu — `hook_kernel_cleanup_apt`

Attach the [`hook_kernel_cleanup_apt`](../plugins/hook_kernel_cleanup_apt.md)
plugin to the Host's kernel meta-package Software Item. See the
plugin doc for the full feature reference, supported meta-package
matrix, and HWE rollover playbook.

## Fedora / RHEL / openSUSE — `installonly_limit`

The DNF package manager natively retains the last N kernels via
the `installonly_limit` setting in `/etc/dnf/dnf.conf`. Default
on Fedora is `3`; we recommend `2` to match the
`hook_kernel_cleanup_apt` default `keep_n`.

```ini
# /etc/dnf/dnf.conf
[main]
installonly_limit=2
````

Apply with:

```bash
sudo dnf install -y dnf-plugins-core
sudo dnf clean all
sudo dnf autoremove
```

`installonly_limit` enforces during every `dnf install` / `dnf upgrade`,
including those triggered by uptrakit's apt-like update path. No uptrakit plugin
is required on these hosts in v1; a sibling `hook_kernel_cleanup_dnf` may land
in a future release if operators ask for one.

## Batch-path observability limitation

The `apt-get upgrade` trigger path (a "batch update" in uptrakit) currently does
not capture hook output into `update_history.output`. This is a known framework
limitation (`crates/shared/agent-core/src/client.rs:521` constructs the batch
output channel with `_output_rx` — the receiver is discarded).
`hook_kernel_cleanup_apt` therefore emits its full decision trace via structured
`tracing::info!` / `tracing::warn!` events on the agent's journal as a
workaround.

A follow-up framework spec will redesign batch-path output capture; once it
lands, the structured-tracing crutch becomes redundant.

## `journalctl` audit recipe

Every cleanup decision is recorded with these fields on the agent's journal:

- `plugin_type = "hook_kernel_cleanup_apt"`
- `batch_id` (uuid)
- `running_kernel`
- `installed_kernels`
- `latest_installed` (implicit — sort installed_kernels)
- `held_kernels`
- `keep_set`
- `purge_set`
- `dry_run`
- `abort_reason` (only on abort)
- `apt_purge_exit_code` (only on successful purge)
- `boot_free_before_kib`, `boot_free_after_kib`

Sample queries:

```bash
# Last 24 h of cleanup decisions on this host
journalctl -u uptrakit-agent --since "24h ago" --output=json \
  | jq 'select(.plugin_type == "hook_kernel_cleanup_apt")'

# All purges for a specific batch
journalctl -u uptrakit-agent --output=json \
  | jq 'select(.plugin_type == "hook_kernel_cleanup_apt" and .batch_id == "<UUID>")'

# Abort counts grouped by reason (last 30 days)
journalctl -u uptrakit-agent --since "30d ago" --output=json \
  | jq -r 'select(.plugin_type == "hook_kernel_cleanup_apt" and .abort_reason)
    | .abort_reason' \
  | sort | uniq -c
```

## Regulated environments

`journalctl`'s default `SystemMaxUse` retention rotates on small VMs within
days. For SOC 2 / HIPAA / similar environments where the cleanup-decision audit
must survive:

- Forward `journald` to a durable sink before enabling the plugin. Common sinks:
  rsyslog, vector, fluent-bit, `systemd-journal-upload`.
- Alternatively, defer enablement until the follow-up framework spec redesigns
  batch-path output capture; until then, the decision audit lives only on the
  local agent.

State the v1 trade-off explicitly in your change ticket: the cleanup action is
auditable (journal + subsequent state inspection), but the audit's durability is
operator-managed.

````text

- [ ] **Step 4: Author the crate README**

Create `crates/plugins/hooks/kernel-cleanup-apt/README.md`:

```markdown
# uptrakit-plugin-hook-kernel-cleanup-apt

`hook_kernel_cleanup_apt` is a `LifecycleHook` plugin that purges
superseded Linux kernel packages on Debian and Ubuntu hosts after
a kernel update. Operator-facing docs:
[`docs/end-user/plugins/hook_kernel_cleanup_apt.md`](../../../../docs/end-user/plugins/hook_kernel_cleanup_apt.md).

## Lifecycle

| Stage | What it does | Where |
| :--- | :--- | :--- |
| Installed-version detection | N/A (Hook plugin; does not implement `VersionDetector`) | — |
| Upstream resolution | N/A | — |
| Version comparison | N/A | — |
| Update execution | N/A | — |
| Pre-hook | No-op (returns `proceed()`) | `plugin.rs::execute_pre_hook` |
| Post-hook | 10-step cleanup pipeline (see below) | `plugin.rs::execute_post_hook` |
| Required privileges | One `apt-get purge --yes linux-image-* linux-modules-* linux-headers-*` sudoers entry | `plugin.rs::required_sudo_commands` |
| Failure modes | All non-fatal: `tracing::warn!` + `[post-hook] ...` output line; returns `Ok(())` per the `PostUpdateHook` contract | All `execute_post_hook` branches |
| Required configuration | `keep_n`, `dry_run`, `min_boot_free_kib` | `config.rs::KernelCleanupAptConfig` |

## Post-hook pipeline (summary)

1. Read `uname -r`.
2. List installed kernel-family packages
   (`dpkg-query --show --showformat='${Package}\t${Status}\n'`
   over five patterns).
3. List apt holds (`apt-mark showhold`).
4. Decide keep / purge sets
   (`decisions::compute_keep_and_purge_sets`).
5. Probe `/boot` (gated when `min_boot_free_kib > 0`).
6. Build explicit purge argv from
   `installed_variants`.
7. Dry-run branch (if configured).
8. `apt-get purge --yes <list>` privileged.
9. Probe `/boot` again for the delta.
10. Done.

Every step emits a structured `tracing::info!` (or `warn!` on
failure) keyed on `plugin_type` + `batch_id` for v1 batch-path
audit. See the operator runbook for the journalctl recipe.

## Assumptions

- Host is Debian or Ubuntu with `apt-get`, `dpkg-query`,
  `apt-mark`, `df`, `uname` available.
- Operator has attached the plugin to **the** kernel meta-package
  Software Item appropriate for the Host flavor.
- Software discovery has already surfaced the meta-package on the
  Host (otherwise no batches carry the hook assignment).

## Not supported in v1

- DNF / RPM family (covered by `installonly_limit`, see runbook).
- Proxmox / OEM / Raspberry Pi / manually-compiled kernels.
- Batch-path output persistence to `update_history.output`
  (framework gap, tracked separately).
````

- [ ] **Step 5: One-line pointer in `docs/development/plugin-guidelines.md`**

Locate the section on hook plugins / preflight idioms (search for
`detect_host_compatibility` if it already exists, otherwise append to the
"Update Lifecycle Plugins" subsection). Add:

```markdown
For an example of overriding `LifecycleHook::detect_host_compatibility`, see
`crates/plugins/hooks/kernel-cleanup-apt/src/plugin.rs` — a default-impl
override that probes `which apt-get` and returns
`Incompatible("apt-get not found")` on non-Debian hosts.
```

- [ ] **Step 6: Markdownlint sweep**

```bash
markdownlint --config .markdownlint.json \
  docs/end-user/plugins/hook_kernel_cleanup_apt.md \
  docs/end-user/operations/kernel-housekeeping.md \
  crates/plugins/hooks/kernel-cleanup-apt/README.md \
  docs/development/plugin-guidelines.md \
  docs/adr/0010-host-scoped-housekeeping-via-meta-package-hooks.md
```

Expected: clean. Fix any flagged issues inline (code-fence languages, table
column count, emphasis-as-heading) — do not silence rules.

- [ ] **Step 7: Commit**

```bash
git add docs/end-user/plugins/hook_kernel_cleanup_apt.md \
        docs/end-user/operations/kernel-housekeeping.md \
        crates/plugins/hooks/kernel-cleanup-apt/README.md \
        docs/development/plugin-guidelines.md
git commit -m "docs(kernel-cleanup-apt): operator doc, runbook, crate README

- docs/end-user/plugins/hook_kernel_cleanup_apt.md: full plugin
  reference (supported meta-package matrix, HWE rollover playbook,
  hold/dkms caveats, sample outputs)
- docs/end-user/operations/kernel-housekeeping.md: cross-distro
  runbook (Debian via plugin, Fedora via installonly_limit,
  batch-path observability limitation, journalctl audit recipe,
  regulated-environment guidance)
- crates/plugins/hooks/kernel-cleanup-apt/README.md: developer-
  facing lifecycle summary
- docs/development/plugin-guidelines.md: one-line pointer to the
  detect_host_compatibility precedent."
```

---

## Task 10: Manual Debian VM end-to-end verification

**Files:** none (operational verification).

> This task is run by a developer on a throwaway Debian (bookworm or trixie) or
> Ubuntu (22.04 / 24.04) VM with at least 3 installed kernels. Do not skip it —
> the integration tests intentionally do not exercise the live `apt-get purge`
> path.

- [ ] **Step 1: Provision the VM**

A Debian 12 cloud image or Ubuntu 24.04 fresh VM is fine. Verify:

```bash
uname -r
dpkg -l 'linux-image-*' | awk '/^ii/ { print $2 }'
```

Expected: at least 3 `linux-image-<KVER>` packages installed. If fewer, install
older kernels from the snapshot repository (Debian) or from the launchpad
archive (Ubuntu) to set up the scenario.

- [ ] **Step 2: Enroll the VM with the uptrakit controller**

Follow the standard enrollment flow. After enrollment, confirm the meta-package
(`linux-image-amd64` on Debian or `linux-image-generic` on Ubuntu) appears as a
tracked Software Item on the Host.

- [ ] **Step 3: Attach the hook with `dry_run = true`**

Via the Dashboard or CLI, attach `hook_kernel_cleanup_apt` to the meta-package
Software Item with config:

```json
{ "keep_n": 2, "dry_run": true, "min_boot_free_kib": 51200 }
```

- [ ] **Step 4: Trigger a no-op update or wait for the next batch**

The cleanest path: trigger an update that touches the meta-package even if it's
a no-op reinstall:

```bash
# On the controller, force re-dispatch of the meta-package update.
```

Or simply trigger a re-discovery and then a batch update. Verify in
`update_history` and in the agent's journal:

```bash
journalctl -u uptrakit-agent --since "5m ago" --output=json \
  | jq 'select(.plugin_type == "hook_kernel_cleanup_apt")'
```

Expected: a structured log line per pipeline step with `dry_run = true`,
`would_purge = [...]` listing the oldest KVER's companion packages.

- [ ] **Step 5: Confirm no packages were actually removed**

```bash
dpkg -l 'linux-image-*' | awk '/^ii/ { print $2 }'
```

Expected: identical to step 1.

- [ ] **Step 6: Flip `dry_run = false` and re-trigger**

Update the plugin config to `dry_run = false`. Trigger another batch.

Expected (via journalctl):

- `apt-get purge --yes linux-image-<oldest> linux-modules-<oldest> linux-headers-<oldest>`
  invocation
- `apt_purge_exit_code = 0`
- `/boot` free delta > 0

Confirm:

```bash
dpkg -l 'linux-image-*' | awk '/^ii/ { print $2 }'
```

Expected: the oldest concrete kernel package and its companions are gone.

- [ ] **Step 7: Verify the abort invariant manually**

Reboot into the oldest still-installed kernel (so running != latest):

```bash
sudo grub-reboot "Advanced options for ..." # pick old kernel
sudo reboot
# ...after reboot...
uname -r
```

Trigger another batch. Expected (via journalctl): the running kernel is now the
oldest among installed; cleanup keeps it + latest + (none more, since keep_n=2).
No purge action this round.

- [ ] **Step 8: Verify the `/boot` abort path**

Fill `/boot` with a placeholder file:

```bash
sudo fallocate -l 200M /boot/uptrakit-test-fill.bin
df --output=avail /boot
```

Trigger a batch. Expected:
`[post-hook] kernel cleanup aborted: /boot has only ... KiB free` with a
`tracing::warn!`. Delete the file (`sudo rm /boot/uptrakit-test-fill.bin`)
afterwards.

- [ ] **Step 9: Dispatcher skip-path verification on a non-apt host**

The Plan A trait-level test proves `detect_host_compatibility()` returns
`Incompatible` correctly, but does not cover the dispatcher actually skipping
`execute_post_hook`. Verify end-to-end:

1. Provision a second VM running Fedora (or macOS if convenient) — anywhere
   without `apt-get` on `$PATH`.
2. Enroll it and attach `hook_kernel_cleanup_apt` to any tracked Software Item
   (it can be a non-kernel item; we just need the assignment to exist).
3. Trigger any update on that Host that would normally fire the post-hook.
4. Query the agent journal:

   ```bash
   journalctl -u uptrakit-agent --since "5m ago" --output=json \
     | jq 'select(.message | tostring | contains("post-hook")) | .message'
   ```

5. Expected: one `"post-hook plugin incompatible with host; skipping"` entry
   with `plugin_type = "hook_kernel_cleanup_apt"` and
   `reason = "apt-get not found"`. **No** `[post-hook] kernel cleanup starting`
   or any `tracing` event from inside `execute_post_hook`. The hook body never
   ran.

Detach the hook after verification.

- [ ] **Step 10: Detach + clean up the Debian VM**

Detach the hook from the Debian VM's meta-package Software Item. Verify the
sudoers no longer contains the `apt-get purge` line for this Host (only present
while at least one assignment exists).

- [ ] **Step 11: Record findings**

If anything diverged from expected behaviour, file an issue referencing this
Task 10 step number. Otherwise, append a one- line note to the PR description:
"Manual VM verification passed on <distro/version>, kernels <X, Y, Z>."

---

## Task 11: Final quality-gate sweep

**Files:** none (verification only).

- [ ] **Step 1: Format check**

```bash
cargo fmt --all -- --check
```

Expected: clean.

- [ ] **Step 2: Build matrix**

```bash
cargo check --no-default-features --features db-sqlite
cargo check --all-features
```

Expected: both succeed.

- [ ] **Step 3: Clippy matrix**

```bash
cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: clean. Any `#[expect(lint, reason = "...")]` introduced must have a
reason field per snapshot rule.

- [ ] **Step 4: Test matrix**

```bash
cargo test --all-features
```

Expected: all green.

- [ ] **Step 5: cargo deny**

```bash
cargo deny check
```

Expected: clean.

- [ ] **Step 6: Plugin semantic boundary check**

```bash
python3 ci/check_plugin_semantic_boundary.py
```

Expected: clean.

- [ ] **Step 7: Sentrux**

```bash
sentrux check .
```

Expected: clean.

- [ ] **Step 8: Markdownlint sweep**

```bash
markdownlint --config .markdownlint.json '**/*.md'
```

Expected: clean.

- [ ] **Step 9: Tag the head**

```bash
git tag plan-b-kernel-cleanup-apt
git log --oneline -15
```

Expected: 8 new commits on top of Plan A's tag.

---

## Self-Review

**Spec coverage check:**

- New crate scaffold — Task 2.
- `KernelCleanupAptError` + `Result<T>` + `impl_report_conversion!` — Task 3.
- `KernelCleanupAptConfig` (`#[non_exhaustive]`, `keep_n`, `dry_run`,
  `min_boot_free_kib`) + `PluginConfig::validate` + form schema — Task 4.
- `KernelEntry { kver, installed_variants }`, `KernelVariantSet`,
  `KeepDecision`, three pure parsing/decision fns + `purge_argv_for_kver` + 20+
  table-driven tests including unsigned canonicalisation and held-kernel
  exemption — Task 5.
- `KernelCleanupAptHookPlugin` + `declare_plugin!` (`PluginFamily::Hook`,
  `LifecycleHook` role, single sudoers entry with `&'static str` literal,
  `with_setenv()`) + `detect_host_compatibility` override + 10-step
  `execute_post_hook` pipeline + descriptor tests — Task 6.
- Registry registration — Task 7.
- ADR 0010 — Task 8.
- Operator plugin doc with supported meta-package matrix + HWE rollover
  playbook + hold/dkms/discovery-race caveats — Task 9.
- Cross-distro runbook with journalctl recipe + regulated-environment guidance +
  DNF `installonly_limit` snippet — Task 9.
- Crate README with lifecycle table — Task 9.
- `docs/development/plugin-guidelines.md` pointer — Task 9.
- Manual VM verification (dry-run, real purge, abort-when-running-unmatched,
  `/boot` abort) — Task 10.
- Quality gates — Task 11.

**Placeholder scan:** no
`TBD`/`TODO`/`later`/`similar to`/`appropriate error handling` patterns in any
task. Every code block is concrete.

**Type consistency:** `KernelEntry`, `KernelVariantSet`, `KeepDecision`,
`KernelCleanupAptConfig`, `KernelCleanupAptError`, `KernelCleanupAptHookPlugin`,
`DESCRIPTOR`, `parse_dpkg_kernel_list`, `parse_apt_mark_holds`,
`compute_keep_and_purge_sets`, `purge_argv_for_kver` — names match between
definition (Task 5/6) and uses (Task 6/registry/tests).

**Idiom audit:** every task names the idiomatic primitive explicitly —
`declare_plugin!`, `PluginConfig::validate`,
`SudoCommandEntry::with_args_suffix(&str).with_setenv()`,
`HostCompatibility::Incompatible(String)`,
`report!(KernelCleanupAptError::...)`, `LocalCommandExecutor` + descriptor-only
tests, `impl_report_conversion!`. No task asks the implementer to silence a
lint; clippy hits are routed to the root-cause fix (e.g., rewrite the
boxed-closure pattern in `parse_dpkg_kernel_list` rather than `#[allow]`).

**Documentation tasks:** every spec deliverable in the Documentation table has a
concrete task — Task 8 (ADR), Task 9 (operator doc + runbook + crate README +
plugin-guidelines pointer). Task 8 of Plan A handled `update-hooks.md`.

---

## Execution Handoff

Plan complete. Two execution options:

1. **Subagent-Driven (recommended)** — fresh subagent per task, review between
   tasks, fast iteration. Use `superpowers:subagent-driven-development`.
2. **Inline Execution** — execute tasks in the current session via
   `superpowers:executing-plans`, batch with checkpoints.

Plan A (`2026-05-12-kernel-cleanup-a-framework.md`) lands first and tags
`plan-a-kernel-cleanup-framework`. Plan B starts from that tag.
