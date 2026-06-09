# Publishable Crate Squat Chain Break — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Drop the single `[dev-dependencies] uptrakit-audit-log` entry in `wire` that forces five workspace-internal crates (audit-log,
audit-log-derive, shared-db, crypto, tenant-db) onto crates.io as squats, lock those five with `publish = false`, and add `cargo_metadata`-driven
guardrail tests in the two publishable crates (`service-sdk`, `openapi-client`) that fail if the chain ever re-forms.

**Architecture:** Wire-side cut. Replace the two test-fixture `AuditActionType::*` references in `wire/src/tests.rs` with a synthetic module-level
constant (decouples wire tests from audit-log catalog renames). Belt-and-suspenders `publish = false` on the five freed crates defends against direct
`cargo publish` bypass of release-plz. Guardrail tests use `cargo_metadata::MetadataCommand` typed API to walk the root-reachable resolve closure (BFS
from `metadata.resolve.root`) and panic with the full dep chain on any banned name.

**Tech Stack:** Rust 2024 edition (workspace `resolver = "3"`), Cargo workspaces, `cargo_metadata` 0.23.1 (latest stable verified via
`cargo search cargo_metadata --limit 1` at plan-write time), release-plz, cargo-deny + clippy + cargo-fmt + markdownlint as enforced gates.

**Snapshot binding** (`.superpowers/standards-snapshot.md`):

- "all new dependencies registered in [workspace.dependencies] first; crates reference via workspace = true" — `/AGENTS.md#dependency-registration` →
  Task 1.
- "use Conventional Commits; types: feat, fix, docs, refactor, perf, test, build, ci, chore" + "scopes match crate/component names" + "multi-scope
  commits omit scope in first line" — `/docs/development/commit-messages.md` → Tasks 2, 8.
- "tests never sleep on real wall-clock time" — `/docs/development/testing.md` → Tasks 5, 6 (tests perform no time ops; rule does not apply but is
  consciously not violated).
- "clippy.toml allow-unwrap-in-tests=true" — `/clippy.toml` → Tasks 5, 6 (test code uses `.expect()`/`.unwrap()` per this exemption).
- Quality gates — `/docs/development/quality-gates.md` → Task 8.

---

## Tasks

### Task 1: Register `cargo_metadata` in workspace dependencies

**Files:**

- Modify: `Cargo.toml`

- [ ] **Step 1: Edit `Cargo.toml` to add the workspace dep**

Insert this line into `[workspace.dependencies]` (the section starts at line 26 in the current `Cargo.toml`). Place it on a new line
**immediately after the `tempfile = "3"` line at line 93** so it groups with the other test/dev-only deps (rather than mixing into the runtime-stack
group at the top of the section). The line to add:

```toml
cargo_metadata = "0.23.1"
```

After the edit, the relevant region of `[workspace.dependencies]` reads:

```toml
bollard = { version = "0.20", default-features = false, features = ["http", "pipe"] }
tempfile = "3"
cargo_metadata = "0.23.1"
tokio-tungstenite = { version = "0.29", default-features = false, features = ["connect"] }
```

(Line numbers shift by 1 from the next existing entry onward after insertion.)

- [ ] **Step 2: Verify the dep registers cleanly**

Run: `cargo metadata --format-version=1 --no-deps 2>&1 | head -5` from the workspace root.

Expected: command succeeds (no manifest parse errors). The new entry is not yet referenced by any crate, so this is a manifest-level sanity check
only. Do not pass `--offline` here — the cargo index may not yet contain `cargo_metadata 0.23.1` on a cold checkout, and `cargo metadata` will fetch
the index on first run.

- [ ] **Step 3: No commit yet**

This task lands in the cross-cutting commit at the end of Task 8 along with Tasks 3, 4, 5, 6, 7. Do not commit in isolation.

---

### Task 2: Wire-side cut — drop the dev-dep, synthetic constant, substitute call sites

**Files:**

- Modify: `crates/shared/wire/Cargo.toml`
- Modify: `crates/shared/wire/src/tests.rs:1-20` (imports), `:229` (first call site), `:255` (second call site)

- [ ] **Step 1: Drop `uptrakit-audit-log` from `wire`'s dev-deps**

Edit `crates/shared/wire/Cargo.toml`. Locate the `[dev-dependencies]` block (currently 3 entries):

```toml
[dev-dependencies]
serde_yaml_ng = { workspace = true }
tokio = { workspace = true, features = ["macros", "rt", "time", "test-util"] }
uptrakit-audit-log = { workspace = true }
```

Delete the `uptrakit-audit-log` line. Result:

```toml
[dev-dependencies]
serde_yaml_ng = { workspace = true }
tokio = { workspace = true, features = ["macros", "rt", "time", "test-util"] }
```

- [ ] **Step 2: Remove the `uptrakit_audit_log` import from `wire/src/tests.rs`**

Edit `crates/shared/wire/src/tests.rs`. The file currently has at lines 5–14:

```rust
use std::collections::{BTreeSet, HashSet};

use time::UtcDateTime;
use uuid::Uuid;

use uptrakit_audit_log::AuditActionType;

use crate::limits::{MAX_SHORT_STRING_LEN, WireValidate};

use super::*;
```

Delete the `use uptrakit_audit_log::AuditActionType;` line (and the blank line above or below it, whichever keeps the import grouping clean). Result:

```rust
use std::collections::{BTreeSet, HashSet};

use time::UtcDateTime;
use uuid::Uuid;

use crate::limits::{MAX_SHORT_STRING_LEN, WireValidate};

use super::*;
```

- [ ] **Step 3: Add the synthetic `TEST_ACTION_TYPE` constant**

Still in `crates/shared/wire/src/tests.rs`. After the existing `TEST_UUID_*` constants (currently around lines 16–24), insert this constant with the
doc-comment exactly as specified in spec §2:

```rust
/// Synthetic action_type for `AuditEventPayload` serde tests. Not a real audit-log
/// catalog entry — the tests here verify wire serde round-trip shape, not catalog
/// correctness. Using a synthetic value avoids a workspace dep on `uptrakit-audit-log`
/// (which would re-form the crates.io squat chain documented in
/// docs/development/coding-standards.md "Publishable Crate Dependency Hygiene")
/// and avoids silent drift if the real catalog's constant names ever change.
const TEST_ACTION_TYPE: &str = "test.wire.synthetic_action";
```

- [ ] **Step 4: Substitute the first call site (line 229)**

Find this line inside `audit_event_serialization_roundtrip`:

```rust
action_type: AuditActionType::SOFTWARE_UPDATE_STARTED.to_string(),
```

Replace with:

```rust
action_type: TEST_ACTION_TYPE.to_string(),
```

- [ ] **Step 5: Substitute the second call site (line 255)**

Find this line inside `audit_event_payload_round_trips_correlation_id`:

```rust
action_type: AuditActionType::SOFTWARE_UPDATE_FINALIZED.to_string(),
```

Replace with:

```rust
action_type: TEST_ACTION_TYPE.to_string(),
```

- [ ] **Step 6: Verify wire tests still pass with the cut applied**

Run: `cargo test -p uptrakit-wire --all-features 2>&1 | tail -20`

Expected: all wire tests pass, including `audit_event_serialization_roundtrip` and `audit_event_payload_round_trips_correlation_id`. No reference to
`uptrakit_audit_log` is required for compilation or test pass.

- [ ] **Step 7: Verify the dev-dep is gone from cargo tree**

Run: `cargo tree -p uptrakit-wire --edges dev 2>&1 | grep uptrakit-audit-log`

Expected: empty output (the grep finds nothing). Exit code 1 from grep is fine — that confirms the absence.

- [ ] **Step 8: Commit (commit 1 of 2)**

Stage just the two files touched in this task:

```bash
git add crates/shared/wire/Cargo.toml crates/shared/wire/src/tests.rs
git commit -m "$(cat <<'EOF'
refactor(wire): drop uptrakit-audit-log dev-dep; synthetic action_type const

`wire` is a direct dep of the two publishable crates (`uptrakit-service-sdk`,
`uptrakit-openapi-client`). Its `[dev-dependencies]` carried
`uptrakit-audit-log = { workspace = true }` (versioned via workspace inheritance),
which forced `audit-log` onto crates.io and pulled the rest of the
audit-log/shared-db/crypto/tenant-db chain along with it. Cutting that single edge
breaks the chain.

Two test fixtures referenced `AuditActionType::SOFTWARE_UPDATE_*.to_string()` only
as opaque strings for `AuditEventPayload` serde round-trips. Replace with a
synthetic module-level `TEST_ACTION_TYPE` constant — the tests assert shape, not
catalog correctness, and decoupling them removes silent-drift risk if the real
catalog renames a constant.

See docs/superpowers/specs/2026-06-09-publishable-crate-squat-chain-break-design.md.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: Lock the 5 freed crates with `publish = false`

**Files:**

- Modify: `crates/shared/audit-log/Cargo.toml`
- Modify: `crates/shared/audit-log-derive/Cargo.toml`
- Modify: `crates/shared/db/Cargo.toml`
- Modify: `crates/shared/crypto/Cargo.toml`
- Modify: `crates/shared/tenant-db/Cargo.toml`

- [ ] **Step 1: Lock `crates/shared/audit-log/Cargo.toml`**

The file currently starts with this `[package]` block (lines 1–8):

```toml
[package]
name = "uptrakit-audit-log"
description = "Uptrakit shared audit-log: structured controller-side audit events"
edition.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
version = "0.0.1"
```

Insert `publish = false` on a new line immediately after `version = "0.0.1"`. Result:

```toml
[package]
name = "uptrakit-audit-log"
description = "Uptrakit shared audit-log: structured controller-side audit events"
edition.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
version = "0.0.1"
publish = false
```

- [ ] **Step 2: Lock `crates/shared/audit-log-derive/Cargo.toml`**

Same pattern — insert `publish = false` on a new line immediately after `version = "0.0.1"`. The `[package]` block currently ends with:

```toml
version = "0.0.1"
```

Becomes:

```toml
version = "0.0.1"
publish = false
```

- [ ] **Step 3: Lock `crates/shared/db/Cargo.toml` (package name `uptrakit-shared-db`)**

Same pattern. After:

```toml
version = "0.0.1"
```

insert:

```toml
publish = false
```

- [ ] **Step 4: Lock `crates/shared/crypto/Cargo.toml`**

Same pattern. After `version = "0.0.1"` insert `publish = false`.

- [ ] **Step 5: Lock `crates/shared/tenant-db/Cargo.toml`**

Same pattern. After `version = "0.0.1"` insert `publish = false`.

- [ ] **Step 6: Verify all 5 crates still compile**

Run:

```bash
cargo check \
  -p uptrakit-audit-log \
  -p uptrakit-audit-log-derive \
  -p uptrakit-shared-db \
  -p uptrakit-crypto \
  -p uptrakit-tenant-db \
  --all-features 2>&1 | tail -10
```

Expected: clean compile. `publish = false` is metadata-only and has no compile impact.

- [ ] **Step 7: Verify direct `cargo publish` is now refused**

Run: `cargo publish -p uptrakit-shared-db --dry-run --allow-dirty 2>&1 | tail -5`

Expected: error message stating that the `publish` field is set to `false` and the crate cannot be published. This is the belt-and-suspenders effect.

- [ ] **Step 8: No commit yet**

This task lands in the cross-cutting commit at the end of Task 8.

---

### Task 4: Add `cargo_metadata` dev-dep to the two publishable crates

**Files:**

- Modify: `crates/shared/service-sdk/Cargo.toml` (`[dev-dependencies]` block)
- Modify: `crates/shared/openapi-client/Cargo.toml` (`[dev-dependencies]` block)

- [ ] **Step 1: Add dev-dep to `service-sdk/Cargo.toml`**

The file currently has this `[dev-dependencies]` block:

```toml
[dev-dependencies]
nix = { workspace = true, features = ["signal", "process"] }
tokio = { workspace = true, features = ["macros", "rt", "test-util"] }
```

Insert `cargo_metadata = { workspace = true }` as a new line at the top of that block. Result:

```toml
[dev-dependencies]
cargo_metadata = { workspace = true }
nix = { workspace = true, features = ["signal", "process"] }
tokio = { workspace = true, features = ["macros", "rt", "test-util"] }
```

- [ ] **Step 2: Add dev-dep to `openapi-client/Cargo.toml`**

The file currently has this `[dev-dependencies]` block:

```toml
[dev-dependencies]
http = { workspace = true }
httpmock = { workspace = true }
serde_urlencoded = { workspace = true }
tokio = { workspace = true, features = ["rt-multi-thread", "macros", "time"] }
```

Insert `cargo_metadata = { workspace = true }` as a new line at the top. Result:

```toml
[dev-dependencies]
cargo_metadata = { workspace = true }
http = { workspace = true }
httpmock = { workspace = true }
serde_urlencoded = { workspace = true }
tokio = { workspace = true, features = ["rt-multi-thread", "macros", "time"] }
```

- [ ] **Step 3: Verify the dev-deps resolve**

Run: `cargo check -p uptrakit-service-sdk -p uptrakit-openapi-client --tests --all-features 2>&1 | tail -10`

Expected: clean check. The `--tests` flag forces dev-deps to be resolved.

- [ ] **Step 4: No commit yet**

This task lands in the cross-cutting commit at the end of Task 8.

---

### Task 5: Guardrail test in `service-sdk`

**Files:**

- Create: `crates/shared/service-sdk/tests/no_workspace_db_deps.rs`

- [ ] **Step 1: Write the test file with the complete guardrail logic**

Create `crates/shared/service-sdk/tests/no_workspace_db_deps.rs` with this exact content:

```rust
//! Guardrail: assert that none of the five workspace-internal
//! database/encryption crates are reachable from this publishable crate's
//! resolve graph. See
//! docs/development/coding-standards.md "Publishable Crate Dependency Hygiene"
//! and docs/superpowers/specs/2026-06-09-publishable-crate-squat-chain-break-design.md.

use std::collections::{HashMap, HashSet, VecDeque};

use cargo_metadata::{CargoOpt, Metadata, MetadataCommand, PackageId};

const BANNED: &[&str] = &[
    "uptrakit-audit-log",
    "uptrakit-audit-log-derive",
    "uptrakit-shared-db",
    "uptrakit-tenant-db",
    "uptrakit-crypto",
];

fn load_metadata(all_features: bool) -> Metadata {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let mut cmd = MetadataCommand::new();
    cmd.manifest_path(manifest);
    if all_features {
        cmd.features(CargoOpt::AllFeatures);
    }
    cmd.exec().expect("cargo metadata invocation failed")
}

fn assert_no_banned_deps(metadata: &Metadata, label: &str) {
    let host_pkg_name = env!("CARGO_PKG_NAME");

    let id_to_name: HashMap<&PackageId, &str> = metadata
        .packages
        .iter()
        .map(|p| (&p.id, p.name.as_ref()))
        .collect();

    let resolve = metadata
        .resolve
        .as_ref()
        .expect("cargo metadata returned no resolve graph");

    let root: &PackageId = match resolve.root.as_ref() {
        Some(root) => root,
        None => metadata
            .packages
            .iter()
            .find(|p| p.name.as_ref() == host_pkg_name)
            .map(|p| &p.id)
            .unwrap_or_else(|| {
                panic!("host crate `{host_pkg_name}` not found in metadata.packages")
            }),
    };

    let node_children: HashMap<&PackageId, Vec<&PackageId>> = resolve
        .nodes
        .iter()
        .map(|n| (&n.id, n.deps.iter().map(|d| &d.pkg).collect()))
        .collect();

    let mut parents: HashMap<&PackageId, &PackageId> = HashMap::new();
    let mut visited: HashSet<&PackageId> = HashSet::from([root]);
    let mut queue: VecDeque<&PackageId> = VecDeque::from([root]);

    while let Some(current) = queue.pop_front() {
        if let Some(children) = node_children.get(current) {
            for child in children {
                if visited.insert(child) {
                    parents.insert(child, current);
                    queue.push_back(child);
                }
            }
        }
    }

    for id in &visited {
        let name = id_to_name.get(id).copied().unwrap_or("<unknown>");
        if BANNED.contains(&name) {
            let mut chain: Vec<&PackageId> = vec![id];
            let mut cursor: &PackageId = id;
            while let Some(parent) = parents.get(cursor) {
                chain.push(parent);
                cursor = parent;
            }
            chain.reverse();
            let chain_names: Vec<&str> = chain
                .iter()
                .map(|c| id_to_name.get(c).copied().unwrap_or("<unknown>"))
                .collect();
            panic!(
                "[{label}] banned crate `{name}` reachable from `{host_pkg_name}`:\n  chain: {}",
                chain_names.join(" -> ")
            );
        }
    }
}

#[test]
fn no_workspace_db_deps() {
    assert_no_banned_deps(&load_metadata(false), "default-features");
    assert_no_banned_deps(&load_metadata(true), "all-features");
}
```

- [ ] **Step 2: Run the test and verify it passes**

Run: `cargo test -p uptrakit-service-sdk --test no_workspace_db_deps --all-features 2>&1 | tail -10`

Expected: `test no_workspace_db_deps ... ok`. Test invokes `cargo metadata` twice (default + all-features), each takes a few seconds cold-cache.

- [ ] **Step 3: Verify clippy is clean on the new test file**

Run: `cargo clippy -p uptrakit-service-sdk --tests --all-features 2>&1 | tail -10`

Expected: no warnings or errors. `.expect()` and `.unwrap()` in test code are permitted by `clippy.toml allow-unwrap-in-tests = true`.

- [ ] **Step 4: Negative-test the guardrail (temporary, MUST revert before next step)**

To prove the guardrail actually fires, temporarily add `uptrakit-audit-log = { workspace = true }` to `crates/shared/service-sdk/Cargo.toml`'s
`[dev-dependencies]`. Re-run the test:

```bash
cargo test -p uptrakit-service-sdk --test no_workspace_db_deps --all-features 2>&1 | tail -20
```

Expected: test panics with a message of the form:

```text
banned crate `uptrakit-audit-log` reachable from `uptrakit-service-sdk`:
  chain: uptrakit-service-sdk -> uptrakit-audit-log
```

**Then revert ONLY the negative-test edit immediately — do not `git restore` the whole file, because Task 4 also touched
`crates/shared/service-sdk/Cargo.toml` (added the `cargo_metadata = { workspace = true }` dev-dep) and those changes are not yet committed. Remove only
the line you just added:**

```bash
sed -i.bak '/^uptrakit-audit-log = { workspace = true }$/d' crates/shared/service-sdk/Cargo.toml && rm crates/shared/service-sdk/Cargo.toml.bak
```

(On macOS, the `-i.bak` form is required by BSD sed; on Linux GNU sed accepts the same form.)

Re-run the test to confirm the guardrail returns to pass:

```bash
cargo test -p uptrakit-service-sdk --test no_workspace_db_deps --all-features 2>&1 | tail -10
```

Expected: `test no_workspace_db_deps ... ok`. Then verify only Task 4's change remains by diffing against `HEAD`:

```bash
git diff crates/shared/service-sdk/Cargo.toml
```

Expected diff: only the single `+cargo_metadata = { workspace = true }` line from Task 4. No `uptrakit-audit-log` entry.

- [ ] **Step 5: No commit yet**

This task lands in the cross-cutting commit at the end of Task 8.

---

### Task 6: Guardrail test in `openapi-client`

**Files:**

- Create: `crates/shared/openapi-client/tests/no_workspace_db_deps.rs`

- [ ] **Step 1: Write the test file**

Create `crates/shared/openapi-client/tests/no_workspace_db_deps.rs` with the exact same content as the service-sdk test from Task 5, Step 1. Duplicate
the file verbatim — per spec §Guardrail item 8, duplication beats a shared helper crate. `env!("CARGO_PKG_NAME")` resolves to
`uptrakit-openapi-client` at compile time in this file, so the same code performs the correct host-crate scoping automatically.

The complete file content (identical to the previous task):

```rust
//! Guardrail: assert that none of the five workspace-internal
//! database/encryption crates are reachable from this publishable crate's
//! resolve graph. See
//! docs/development/coding-standards.md "Publishable Crate Dependency Hygiene"
//! and docs/superpowers/specs/2026-06-09-publishable-crate-squat-chain-break-design.md.

use std::collections::{HashMap, HashSet, VecDeque};

use cargo_metadata::{CargoOpt, Metadata, MetadataCommand, PackageId};

const BANNED: &[&str] = &[
    "uptrakit-audit-log",
    "uptrakit-audit-log-derive",
    "uptrakit-shared-db",
    "uptrakit-tenant-db",
    "uptrakit-crypto",
];

fn load_metadata(all_features: bool) -> Metadata {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let mut cmd = MetadataCommand::new();
    cmd.manifest_path(manifest);
    if all_features {
        cmd.features(CargoOpt::AllFeatures);
    }
    cmd.exec().expect("cargo metadata invocation failed")
}

fn assert_no_banned_deps(metadata: &Metadata, label: &str) {
    let host_pkg_name = env!("CARGO_PKG_NAME");

    let id_to_name: HashMap<&PackageId, &str> = metadata
        .packages
        .iter()
        .map(|p| (&p.id, p.name.as_ref()))
        .collect();

    let resolve = metadata
        .resolve
        .as_ref()
        .expect("cargo metadata returned no resolve graph");

    let root: &PackageId = match resolve.root.as_ref() {
        Some(root) => root,
        None => metadata
            .packages
            .iter()
            .find(|p| p.name.as_ref() == host_pkg_name)
            .map(|p| &p.id)
            .unwrap_or_else(|| {
                panic!("host crate `{host_pkg_name}` not found in metadata.packages")
            }),
    };

    let node_children: HashMap<&PackageId, Vec<&PackageId>> = resolve
        .nodes
        .iter()
        .map(|n| (&n.id, n.deps.iter().map(|d| &d.pkg).collect()))
        .collect();

    let mut parents: HashMap<&PackageId, &PackageId> = HashMap::new();
    let mut visited: HashSet<&PackageId> = HashSet::from([root]);
    let mut queue: VecDeque<&PackageId> = VecDeque::from([root]);

    while let Some(current) = queue.pop_front() {
        if let Some(children) = node_children.get(current) {
            for child in children {
                if visited.insert(child) {
                    parents.insert(child, current);
                    queue.push_back(child);
                }
            }
        }
    }

    for id in &visited {
        let name = id_to_name.get(id).copied().unwrap_or("<unknown>");
        if BANNED.contains(&name) {
            let mut chain: Vec<&PackageId> = vec![id];
            let mut cursor: &PackageId = id;
            while let Some(parent) = parents.get(cursor) {
                chain.push(parent);
                cursor = parent;
            }
            chain.reverse();
            let chain_names: Vec<&str> = chain
                .iter()
                .map(|c| id_to_name.get(c).copied().unwrap_or("<unknown>"))
                .collect();
            panic!(
                "[{label}] banned crate `{name}` reachable from `{host_pkg_name}`:\n  chain: {}",
                chain_names.join(" -> ")
            );
        }
    }
}

#[test]
fn no_workspace_db_deps() {
    assert_no_banned_deps(&load_metadata(false), "default-features");
    assert_no_banned_deps(&load_metadata(true), "all-features");
}
```

- [ ] **Step 2: Run the test and verify it passes**

Run: `cargo test -p uptrakit-openapi-client --test no_workspace_db_deps --all-features 2>&1 | tail -10`

Expected: `test no_workspace_db_deps ... ok`.

- [ ] **Step 3: Verify clippy is clean**

Run: `cargo clippy -p uptrakit-openapi-client --tests --all-features 2>&1 | tail -10`

Expected: no warnings or errors.

- [ ] **Step 4: No separate negative-test required**

The test code is byte-for-byte identical to Task 5's `service-sdk` test (only `env!("CARGO_PKG_NAME")` differs at compile time, which is implicit).
Task 5 Step 4's negative-test already validated the guardrail fires on a synthetic regression; the same code in this crate inherits that proof. Do
not re-run a negative-test here.

- [ ] **Step 5: No commit yet**

This task lands in the cross-cutting commit at the end of Task 8.

---

### Task 7: Add "Publishable Crate Dependency Hygiene" section to coding-standards.md

**Files:**

- Modify: `docs/development/coding-standards.md` (append new section at end of file, after the existing "Service Binary/Runtime Boundary" section
  which ends near line 2145)

- [ ] **Step 1: Append the new section at the end of the file**

Open `docs/development/coding-standards.md`. The file's last existing section is "Service Binary/Runtime Boundary" and the file currently ends around
line 2145 with an Embedded service construction example.

Append (after a blank line following the existing last line) this section, exactly:

```markdown
## Publishable Crate Dependency Hygiene

Two crates in this workspace are published to crates.io:

- `uptrakit-service-sdk`
- `uptrakit-openapi-client`

Their transitive dep trees (including `[dev-dependencies]` of any crate they reach) must NOT contain any of:

- `uptrakit-audit-log`
- `uptrakit-audit-log-derive`
- `uptrakit-shared-db`
- `uptrakit-tenant-db`
- `uptrakit-crypto`

These five crates are workspace-internal database and encryption plumbing. They have no external consumers and must not be republished to crates.io.

### Why this matters

`cargo publish` (and crates.io's manifest validator) check every named dep entry in the published manifest — including `[dev-dependencies]` that carry
a `version` field, and optional deps — against the registry. A dev-dep on `uptrakit-audit-log` from any crate that the publishable crates transitively
reach is enough to force `audit-log` onto crates.io, and `audit-log` in turn forces `shared-db`, which forces `crypto` and `tenant-db`. The chain is
load-bearing on every edge: cutting any link breaks all of it.

### Enforcement

Two integration tests guard this rule:

- `crates/shared/service-sdk/tests/no_workspace_db_deps.rs`
- `crates/shared/openapi-client/tests/no_workspace_db_deps.rs`

Each test walks the resolved cargo metadata graph (default features and `--all-features`) and panics if any banned name appears, naming the dep chain
back to the publishable crate.

### Why these five and not other internal crates?

Most workspace-internal crates (`uptrakit-build-info`, every plugin, every runtime, etc.) inherit `publish = true` from Cargo's defaults but are kept
off crates.io by `release-plz.toml` declaring `release = false`. That is sufficient because release-plz is the only mechanism that publishes from this
workspace. These five crates additionally carry the belt-and-suspenders `publish = false` in their own `Cargo.toml` because they are the unique
failure case where the squat chain demonstrably reformed once before; locking them in their manifests defends against a contributor running
`cargo publish -p uptrakit-shared-db` directly (bypassing release-plz) and resurrecting the chain.

If you find yourself wanting to add one of these crates to anything in the service-sdk or openapi-client subtree (including dev-deps), stop and think
about what you're actually testing. The wire-side fix for the historical version of this rule replaced two `AuditActionType::*` constants with a
synthetic `TEST_ACTION_TYPE` constant in `crates/shared/wire/src/tests.rs` — the test was asserting serde round-trip shape, not catalog correctness,
so the constant binding added no coverage.
```

- [ ] **Step 2: Verify markdownlint passes**

Run: `npx markdownlint --config .markdownlint.json docs/development/coding-standards.md 2>&1`

Expected: empty output (no lint errors). Snapshot rule: `line_length 150`.

- [ ] **Step 3: No commit yet**

This task lands in the cross-cutting commit at the end of Task 8.

---

### Task 8: Run full workspace quality gates and create commit 2

**Files:**

- No file changes; this is verification + commit.

- [ ] **Step 1: `cargo fmt --all`**

Run: `cargo fmt --all -- --check 2>&1 | tail -10`

Expected: empty output. If any file needs formatting, run `cargo fmt --all` (without `--check`) and stage the formatting fix as part of commit 2.

- [ ] **Step 2: `cargo check` — db-sqlite features**

Run: `cargo check --no-default-features --features db-sqlite 2>&1 | tail -15`

Expected: clean compile (only the standard "Finished" line).

- [ ] **Step 3: `cargo check --all-features`**

Run: `cargo check --all-features 2>&1 | tail -15`

Expected: clean compile.

- [ ] **Step 4: `cargo clippy` — db-sqlite features**

Run: `cargo clippy --all-targets --no-default-features --features db-sqlite 2>&1 | tail -15`

Expected: clean (warnings = deny per snapshot).

- [ ] **Step 5: `cargo clippy --all-targets --all-features`**

Run: `cargo clippy --all-targets --all-features 2>&1 | tail -15`

Expected: clean.

- [ ] **Step 6: `cargo test --all-features`**

Run: `cargo test --all-features 2>&1 | tail -30`

Expected: all tests pass workspace-wide, including the two new `no_workspace_db_deps` integration tests and the existing wire round-trip tests.

- [ ] **Step 7: `cargo deny check`**

Run: `cargo deny check 2>&1 | tail -10`

Expected: clean.

- [ ] **Step 8: `markdownlint`**

Run: `npx markdownlint --config .markdownlint.json '**/*.md' 2>&1 | head -20`

Expected: empty output.

- [ ] **Step 9: Spot-check — wire dev-dep tree is clean**

Run: `cargo tree -p uptrakit-wire --edges dev 2>&1 | grep uptrakit-audit-log && echo FOUND || echo CLEAN`

Expected: `CLEAN`.

- [ ] **Step 10: Stage all the cross-cutting changes and commit (commit 2 of 2)**

Stage the workspace dep, the 5 publish locks, the 2 dev-deps, the 2 new test files, and the docs section:

```bash
git add Cargo.toml \
        crates/shared/audit-log/Cargo.toml \
        crates/shared/audit-log-derive/Cargo.toml \
        crates/shared/db/Cargo.toml \
        crates/shared/crypto/Cargo.toml \
        crates/shared/tenant-db/Cargo.toml \
        crates/shared/service-sdk/Cargo.toml \
        crates/shared/service-sdk/tests/no_workspace_db_deps.rs \
        crates/shared/openapi-client/Cargo.toml \
        crates/shared/openapi-client/tests/no_workspace_db_deps.rs \
        docs/development/coding-standards.md
```

Then commit:

```bash
git commit -m "$(cat <<'EOF'
chore: lock internal crates with publish=false; add publishable-dep guards

- Register `cargo_metadata = "0.23.1"` in `[workspace.dependencies]`.
- Lock `uptrakit-audit-log`, `uptrakit-audit-log-derive`, `uptrakit-shared-db`,
  `uptrakit-crypto`, `uptrakit-tenant-db` with `publish = false` in their
  `[package]` blocks. Belt-and-suspenders against direct `cargo publish` bypass
  of release-plz, which already declares `release = false` for all five.
- Add `cargo_metadata` integration test in `uptrakit-service-sdk` and
  `uptrakit-openapi-client` (`tests/no_workspace_db_deps.rs`). Walks the
  root-reachable resolve closure (default + `--all-features`) and panics with
  the full dep chain on any banned name. Guards against future regression of
  the squat chain broken by the preceding `refactor(wire)` commit.
- Document the rule and the enforcement in
  `docs/development/coding-standards.md` § "Publishable Crate Dependency Hygiene".

See docs/superpowers/specs/2026-06-09-publishable-crate-squat-chain-break-design.md.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 11: Confirm commit landed**

Run: `git log --oneline -3`

Expected: the top two commits are the two created by this plan — `chore: lock internal crates with publish=false; add publishable-dep guards` at
position 1 and `refactor(wire): drop uptrakit-audit-log dev-dep; synthetic action_type const` at position 2. Position 3 will be whatever commit was
HEAD at the start of this plan's execution (do not pin to a specific SHA — the plan may run on top of a branch with arbitrary intervening commits).

---

## Spec coverage check

| Spec section                                   | Covered by                                                        |
| ---------------------------------------------- | ----------------------------------------------------------------- |
| Problem (chain identification)                 | Task 2 (cut) + Task 8 spot-check                                  |
| §Solution                                      | Tasks 2–7                                                         |
| §File-by-file §1 (wire Cargo.toml)             | Task 2 Step 1                                                     |
| §File-by-file §2 (wire tests.rs)               | Task 2 Steps 2–5                                                  |
| §File-by-file §3 (5 publish locks)             | Task 3                                                            |
| §File-by-file §4 (workspace dep)               | Task 1                                                            |
| §File-by-file §5 (dev-deps + 2 new test files) | Tasks 4, 5, 6                                                     |
| §File-by-file §6 (coding-standards.md section) | Task 7                                                            |
| §Guardrail test design (all 8 items)           | Tasks 5, 6 — code matches spec items 1–8 exactly                  |
| §Documentation deliverables                    | Task 7 (standards section) + Task 2 Step 3 (constant doc-comment) |
| §Verification (8 gates + 2 criteria)           | Task 8 Steps 1–9                                                  |
| §Deferred / Out of scope                       | Not implemented (explicitly out of scope — no task)               |
| §Commits (2 conventional commits)              | Task 2 Step 8 (commit 1), Task 8 Step 10 (commit 2)               |
| §Rejected alternatives                         | Not implemented (informational only)                              |
| §Risks                                         | Not implemented (informational only)                              |
