# Controller Crate Split Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split `crates/core/controller/` into a lib crate (`controller-runtime`) and
two thin binary wrappers (`controller`, `controller-standalone`) so both are installable
via `cargo binstall` as distinct packages.

**Architecture:** `controller-runtime` receives all existing code with `src/main.rs`
renamed to `src/lib.rs` and the entry point exposed as `pub fn run()`. Two new wrapper
crates each contain only a `src/main.rs` calling `uptrakit_controller_runtime::run()`,
with their feature differences expressed through which `controller-runtime` features they
activate in their `Cargo.toml`.

**Tech Stack:** Rust workspace, Cargo features, release-plz, GitHub Actions CI.

---

## Tasks

### Task 1: Rename controller directory to controller-runtime

This is a filesystem rename. The workspace glob `"crates/core/*"` auto-discovers new
paths, so no workspace `Cargo.toml` edits are needed. After the rename, `cargo check`
will fail because the package name still says `uptrakit-controller` — that is expected
and fixed in Task 2.

**Files:**

- Rename: `crates/core/controller/` → `crates/core/controller-runtime/`

- [ ] **Step 1: Rename the directory**

```bash
git mv crates/core/controller crates/core/controller-runtime
```

- [ ] **Step 2: Verify git tracked the rename**

```bash
git status
```

Expected: `renamed: crates/core/controller/... -> crates/core/controller-runtime/...`
for every file.

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "refactor: rename controller/ to controller-runtime/ (step 1 of split)"
```

---

### Task 2: Convert controller-runtime to a library crate

Rename `src/main.rs` to `src/lib.rs`, wrap the entry point in `pub fn run()`, and
update `Cargo.toml` to declare the package as `uptrakit-controller-runtime` with
`publish = false`.

**Files:**

- Rename: `crates/core/controller-runtime/src/main.rs`
  → `crates/core/controller-runtime/src/lib.rs`
- Modify: `crates/core/controller-runtime/Cargo.toml`
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Rename main.rs to lib.rs**

```bash
git mv crates/core/controller-runtime/src/main.rs \
       crates/core/controller-runtime/src/lib.rs
```

- [ ] **Step 2: Wrap entry point in pub fn run()**

Open `crates/core/controller-runtime/src/lib.rs`. Find `async fn main()` at the bottom
of the file (it is the tokio entry point). Replace it with a public sync wrapper:

```rust
#[doc(hidden)]
pub fn run() {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to build tokio runtime")
        .block_on(async_main());
}
```

Where `async_main` is the renamed body of the old `async fn main()`. If `main()` was
annotated with `#[tokio::main]`, strip that attribute and rename the function to
`async fn async_main()`. Do not change any logic — only the function signature and name.

- [ ] **Step 3: Update package name in Cargo.toml**

In `crates/core/controller-runtime/Cargo.toml`, change:

```toml
[package]
name = "uptrakit-controller-runtime"
description = "Uptrakit controller runtime — shared implementation for controller binaries"
publish = false
```

Remove any `[[bin]]` section if present. Cargo auto-discovers `src/lib.rs` as the lib
root.

- [ ] **Step 4: Add to workspace dependencies**

In `Cargo.toml` (workspace root), add to `[workspace.dependencies]`:

```toml
uptrakit-controller-runtime = { path = "crates/core/controller-runtime", default-features = false }
```

- [ ] **Step 5: Verify compile**

```bash
cargo check -p uptrakit-controller-runtime --all-features
```

Expected: compiles.

- [ ] **Step 6: Commit**

```bash
git add crates/core/controller-runtime/Cargo.toml \
        crates/core/controller-runtime/src/lib.rs \
        Cargo.toml
git commit -m "refactor: convert controller-runtime to lib crate with pub fn run()"
```

---

### Task 3: Rename embed-frontend feature to embedded-frontend

Do this before creating wrapper crates so the wrapper `Cargo.toml` files use the final
name. **Do not commit yet** — the CI workflow update (Task 6) must land in the same
commit as this rename.

**Files:**

- Modify: `crates/core/controller-runtime/Cargo.toml`
- Modify: `crates/core/controller-runtime/src/lib.rs`
- Modify: `crates/core/controller-runtime/src/server.rs`
- Modify: `crates/core/controller-runtime/src/cli.rs`
- Modify: `crates/core/controller-runtime/src/startup/validation.rs`
- Modify: `crates/core/controller-runtime/src/embedded_frontend.rs`
- Modify: `crates/core/controller-runtime/build.rs`

- [ ] **Step 1: Rename feature in Cargo.toml**

In `crates/core/controller-runtime/Cargo.toml`:

```toml
# Change in the default array:
default = [
  "db-sqlite", "oidc", "zeroconf", "interactive", "notifications-all",
  "embedded-scheduler", "embedded-mqtt", "embedded-frontend",
  "reset-data", "dashboard-icons"
]

# Change the feature declaration:
embedded-frontend = ["dep:rust-embed"]
```

- [ ] **Step 2: Update lib.rs cfg attribute**

In `crates/core/controller-runtime/src/lib.rs`, line with `mod embedded_frontend`:

```rust
#[cfg(feature = "embedded-frontend")]
mod embedded_frontend;
```

- [ ] **Step 3: Update server.rs cfg attribute**

In `crates/core/controller-runtime/src/server.rs`, find `#[cfg(feature = "embed-frontend")]`
and change to `#[cfg(feature = "embedded-frontend")]`.

- [ ] **Step 4: Update cli.rs**

In `crates/core/controller-runtime/src/cli.rs`, update doc comments (lines 177–179)
from `embed-frontend` to `embedded-frontend` and change any
`#[cfg(feature = "embed-frontend")]` to `#[cfg(feature = "embedded-frontend")]`.

- [ ] **Step 5: Update startup/validation.rs**

In `crates/core/controller-runtime/src/startup/validation.rs`:

```rust
// Line 16 comment:
// Without an explicit path: auto-detect only when embedded-frontend is not compiled in.

// Line 17:
let static_dir = if args.static_dir.is_some() || !cfg!(feature = "embedded-frontend") {

// Line 96 doc comment:
/// embedded frontend assets even when the `embedded-frontend` feature is active.
```

- [ ] **Step 6: Update embedded_frontend.rs module doc**

In `crates/core/controller-runtime/src/embedded_frontend.rs`, line 3:

```rust
//! Only compiled when the `embedded-frontend` Cargo feature is enabled.
```

- [ ] **Step 7: Update build.rs env var**

In `crates/core/controller-runtime/build.rs`, line 12:

```rust
if std::env::var("CARGO_FEATURE_EMBEDDED_FRONTEND").is_ok() {
```

Also update the panic message string from `embed-frontend` to `embedded-frontend`.

- [ ] **Step 8: Verify no remaining embed-frontend references in runtime**

```bash
grep -r "embed-frontend" crates/core/controller-runtime/
```

Expected: no output.

- [ ] **Step 9: Verify compile (skip embedded-frontend — no frontend build here)**

```bash
cargo check -p uptrakit-controller-runtime --no-default-features \
  --features db-sqlite,oidc,embedded-scheduler,embedded-mqtt
```

Expected: compiles.

**Do not commit yet — continue to Task 6 then commit atomically with CI changes.**

---

### Task 4: Create controller wrapper crate

**Files:**

- Create: `crates/core/controller/Cargo.toml`
- Create: `crates/core/controller/src/main.rs`
- Create: `crates/core/controller/CHANGELOG.md`

- [ ] **Step 1: Create directory**

```bash
mkdir -p crates/core/controller/src
```

- [ ] **Step 2: Create Cargo.toml**

Create `crates/core/controller/Cargo.toml`:

```toml
[package]
name = "uptrakit-controller"
description = "Uptrakit controller — self-hosted update management server (lean)"
edition.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
version.workspace = true

[features]
# Forwarding features — allow `cargo build -p uptrakit-controller --features nats`
# without silently building without the feature (Cargo ignores unknown --features).
nats = ["uptrakit-controller-runtime/nats"]
journald = ["uptrakit-controller-runtime/journald"]
swagger-ui = ["uptrakit-controller-runtime/swagger-ui"]
db-postgres = ["uptrakit-controller-runtime/db-postgres"]
db-all = ["uptrakit-controller-runtime/db-all"]

[dependencies]
uptrakit-controller-runtime = { workspace = true, features = [
  "embedded-frontend",
  "db-sqlite",
  "oidc",
  "zeroconf",
  "interactive",
  "notifications-all",
  "reset-data",
  "dashboard-icons",
] }

[lints]
workspace = true
```

- [ ] **Step 3: Create src/main.rs**

Create `crates/core/controller/src/main.rs`:

```rust
fn main() {
    uptrakit_controller_runtime::run();
}
```

- [ ] **Step 4: Create stub CHANGELOG.md**

Create `crates/core/controller/CHANGELOG.md`:

```markdown
# Changelog

See [controller-runtime/CHANGELOG.md](../controller-runtime/CHANGELOG.md)
for the full release history.
```

- [ ] **Step 5: Verify compile**

```bash
cargo check -p uptrakit-controller
```

Expected: compiles.

- [ ] **Step 6: Commit**

```bash
git add crates/core/controller/
git commit -m "feat: add uptrakit-controller thin binary wrapper"
```

---

### Task 5: Create controller-standalone wrapper crate

**Files:**

- Create: `crates/core/controller-standalone/Cargo.toml`
- Create: `crates/core/controller-standalone/src/main.rs`
- Create: `crates/core/controller-standalone/CHANGELOG.md`

- [ ] **Step 1: Create directory**

```bash
mkdir -p crates/core/controller-standalone/src
```

- [ ] **Step 2: Create Cargo.toml**

Create `crates/core/controller-standalone/Cargo.toml`:

```toml
[package]
name = "uptrakit-controller-standalone"
description = "Uptrakit controller — self-hosted update management server (all-in-one)"
edition.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
version.workspace = true

[features]
# Forwarding features — see controller/Cargo.toml for rationale.
nats = ["uptrakit-controller-runtime/nats"]
journald = ["uptrakit-controller-runtime/journald"]
swagger-ui = ["uptrakit-controller-runtime/swagger-ui"]
db-postgres = ["uptrakit-controller-runtime/db-postgres"]
db-all = ["uptrakit-controller-runtime/db-all"]

[dependencies]
uptrakit-controller-runtime = { workspace = true, features = [
  "embedded-frontend",
  "db-sqlite",
  "oidc",
  "zeroconf",
  "interactive",
  "notifications-all",
  "reset-data",
  "dashboard-icons",
  "embedded-scheduler",
  "embedded-mqtt",
  "embedded-agent",
  "embedded-ssh-agent",
] }

[lints]
workspace = true
```

- [ ] **Step 3: Create src/main.rs**

Create `crates/core/controller-standalone/src/main.rs`:

```rust
fn main() {
    uptrakit_controller_runtime::run();
}
```

- [ ] **Step 4: Create stub CHANGELOG.md**

Create `crates/core/controller-standalone/CHANGELOG.md`:

```markdown
# Changelog

See [controller-runtime/CHANGELOG.md](../controller-runtime/CHANGELOG.md)
for the full release history.
```

- [ ] **Step 5: Verify both wrappers and runtime compile together**

```bash
cargo check \
  -p uptrakit-controller \
  -p uptrakit-controller-standalone \
  -p uptrakit-controller-runtime
```

Expected: compiles.

- [ ] **Step 6: Commit**

```bash
git add crates/core/controller-standalone/
git commit -m "feat: add uptrakit-controller-standalone thin binary wrapper"
```

---

### Task 6: Update release-plz.toml and CI workflows (atomic with feature rename)

This task commits the Task 3 feature rename changes together with CI workflow updates.
Both must land in the same commit: if the rename lands before CI is updated, workflows
silently build binaries without the frontend.

**Files:**

- Modify: `release-plz.toml`
- Modify: `.github/workflows/release-plz.yml`
- Modify: `.github/workflows/docker.yml`

- [ ] **Step 1: Update release-plz.toml**

Replace the `[[package]] name = "uptrakit-controller"` block with three blocks:

```toml
[[package]]
name = "uptrakit-controller-runtime"
changelog_update = true
changelog_include = ["frontend"]

[[package]]
name = "uptrakit-controller"
git_release_enable = true
git_tag_enable = true
git_tag_name = "uptrakit-controller-v{{ version }}"
git_release_name = "uptrakit-controller v{{ version }}"
changelog_path = "crates/core/controller-runtime/CHANGELOG.md"
changelog_update = false

[[package]]
name = "uptrakit-controller-standalone"
git_release_enable = true
git_tag_enable = true
git_tag_name = "uptrakit-controller-standalone-v{{ version }}"
git_release_name = "uptrakit-controller-standalone v{{ version }}"
changelog_path = "crates/core/controller-runtime/CHANGELOG.md"
changelog_update = false
```

- [ ] **Step 2: Update release-plz.yml — Build controller step (around line 204)**

Replace the "Build controller" step:

```yaml
      # --- controller (lean: no embedded services, with frontend) ---
      - name: Build controller
        if: ${{ needs.release-plz.outputs.released == 'true' }}
        env:
          UPTRAKIT_RELEASE_NAME: uptrakit-controller
        run: |
          ${{ steps.cargo-cmd.outputs.cmd }} build --release \
            --target ${{ matrix.target }} \
            -p uptrakit-controller \
            --features db-all,nats,notifications-all,zeroconf,interactive,reset-data,dashboard-icons
          cp target/${{ matrix.target }}/release/uptrakit-controller \
            uptrakit-controller-${{ matrix.target }}
```

- [ ] **Step 3: Update release-plz.yml — Build controller-standalone step (around line 215)**

Replace the "Build controller-standalone" step:

```yaml
      # --- controller-standalone (all-in-one: embedded scheduler + mqtt + frontend) ---
      - name: Build controller-standalone
        if: ${{ needs.release-plz.outputs.released == 'true' }}
        env:
          UPTRAKIT_RELEASE_NAME: uptrakit-controller-standalone
        run: |
          ${{ steps.cargo-cmd.outputs.cmd }} build --release \
            --target ${{ matrix.target }} \
            -p uptrakit-controller-standalone \
            --features db-all,nats,notifications-all,zeroconf,interactive,reset-data,dashboard-icons
          cp target/${{ matrix.target }}/release/uptrakit-controller-standalone \
            uptrakit-controller-standalone-${{ matrix.target }}
```

- [ ] **Step 4: Update release-plz.yml — upload_if_released for standalone (line 278)**

```bash
# Before:
upload_if_released "uptrakit-controller"  "uptrakit-controller-standalone-${TARGET}"

# After:
upload_if_released "uptrakit-controller-standalone"  "uptrakit-controller-standalone-${TARGET}"
```

- [ ] **Step 5: Update release-plz.yml — check any binary released filter (around line 62)**

Add `uptrakit-controller-standalone` to the `IN(...)` list:

```bash
count=$(echo "$RELEASES" | jq '[.[] | select(.package_name | IN(
  "uptrakit-controller","uptrakit-controller-standalone",
  "uptrakit-agent","uptrakit-agent-ssh",
  "uptrakit-mqtt","uptrakit-scheduler","uptrakit-cli"
))] | length')
```

- [ ] **Step 6: Update release-plz.yml — frontend bump step (around line 107)**

```bash
VERSION=$(echo "$PR_JSON" | jq -r \
  '(.releases // [])[] | select(.package_name | IN(
    "uptrakit-controller","uptrakit-controller-standalone"
  )) | .version' | head -1)
```

- [ ] **Step 7: Update release-plz.yml — Extract controller release step (around line 47)**

```bash
tag=$(echo "$RELEASES" | jq -r \
  '(.[] | select(.package_name | IN(
    "uptrakit-controller","uptrakit-controller-standalone"
  )) | .tag) // empty' | head -1)
version=$(echo "$RELEASES" | jq -r \
  '(.[] | select(.package_name | IN(
    "uptrakit-controller","uptrakit-controller-standalone"
  )) | .version) // empty' | head -1)
```

- [ ] **Step 8: Update docker.yml — replace embed-frontend and use standalone**

In `.github/workflows/docker.yml`, on each of the three lines containing
`embed-frontend`:

- Replace `embed-frontend` with `embedded-frontend`
- Replace `-p uptrakit-controller` with `-p uptrakit-controller-standalone`
  (Docker image wants all embedded services)
- Remove `--no-default-features` and the explicit `--features` flags where they were
  used to select the standalone feature set; the `controller-standalone` package
  activates the right features by default

Lines 51, 57, 203: apply the changes above.

- [ ] **Step 9: Verify no remaining embed-frontend in CI or runtime**

```bash
grep -r "embed-frontend" crates/core/controller-runtime/ .github/
```

Expected: no output.

- [ ] **Step 10: Commit atomically (Task 3 rename + CI changes)**

```bash
git add \
  crates/core/controller-runtime/Cargo.toml \
  crates/core/controller-runtime/src/lib.rs \
  crates/core/controller-runtime/src/server.rs \
  crates/core/controller-runtime/src/cli.rs \
  crates/core/controller-runtime/src/startup/validation.rs \
  crates/core/controller-runtime/src/embedded_frontend.rs \
  crates/core/controller-runtime/build.rs \
  crates/core/controller/Cargo.toml \
  crates/core/controller-standalone/Cargo.toml \
  release-plz.toml \
  .github/workflows/release-plz.yml \
  .github/workflows/docker.yml
git commit -m "feat: rename embed-frontend to embedded-frontend and update CI for split"
```

---

### Task 7: Full quality gate

**Files:** None modified.

- [ ] **Step 1: Format**

```bash
cargo fmt --all
```

Expected: no changes (fix and re-stage if any).

- [ ] **Step 2: Check no-default-features**

```bash
cargo check --no-default-features --features db-sqlite
```

Expected: compiles.

- [ ] **Step 3: Check all-features**

```bash
cargo check --all-features
```

Expected: compiles.

- [ ] **Step 4: Clippy no-default**

```bash
cargo clippy --all-targets --no-default-features --features db-sqlite
```

Expected: no errors.

- [ ] **Step 5: Clippy all-features**

```bash
cargo clippy --all-targets --all-features
```

Expected: no errors.

- [ ] **Step 6: Tests**

```bash
cargo test --all-features
```

Expected: all pass.

- [ ] **Step 7: Cargo deny**

```bash
cargo deny check
```

Expected: no errors.

- [ ] **Step 8: Confirm both wrappers produce distinct binaries**

```bash
cargo build -p uptrakit-controller -p uptrakit-controller-standalone
ls -la target/debug/uptrakit-controller target/debug/uptrakit-controller-standalone
```

Expected: two separate binary files.

- [ ] **Step 9: Commit fmt fixes if needed**

```bash
git add -A
git commit -m "style: fmt after controller crate split" || echo "nothing to commit"
```

---

### Task 8: release-plz dry-run verification

**Files:** None modified.

- [ ] **Step 1: Install release-plz if not present**

```bash
cargo install release-plz --locked 2>/dev/null || echo "already installed"
```

- [ ] **Step 2: Run dry-run**

```bash
release-plz update --dry-run 2>&1 | head -60
```

Expected: output shows all three crates (`uptrakit-controller-runtime`,
`uptrakit-controller`, `uptrakit-controller-standalone`) processed without errors.

- [ ] **Step 3: Confirm changelog path resolves**

```bash
ls crates/core/controller-runtime/CHANGELOG.md
```

Expected: file exists. This is the workspace-root-relative path that
`changelog_path = "crates/core/controller-runtime/CHANGELOG.md"` in `release-plz.toml`
resolves to.

- [ ] **Step 4: Verify controller-runtime publish = false**

```bash
cargo metadata --no-deps --format-version 1 | \
  jq '.packages[] | select(.name == "uptrakit-controller-runtime") | .publish'
```

Expected: `[]` (empty array means publish = false in Cargo metadata).

---

## Self-Review

### Spec coverage

| Spec requirement | Task |
| --- | --- |
| Rename controller/ to controller-runtime/ | 1 |
| lib crate with pub fn run() | 2 |
| Feature rename embed-frontend → embedded-frontend | 3, 6 |
| controller wrapper crate with forwarding features | 4 |
| controller-standalone wrapper crate with forwarding features | 5 |
| release-plz.toml three-block config | 6 |
| CI build steps updated (no --features on wrappers) | 6 |
| upload_if_released standalone fix (required, not deferred) | 6 |
| select(.package_name) gate updated | 6 |
| Feature rename + CI atomic commit | 6 |
| Full quality gate | 7 |
| release-plz dry-run | 8 |

### Notes

- CHANGELOG.md at `crates/core/controller-runtime/CHANGELOG.md` was already moved in
  Task 1 as part of the directory rename — no separate `git mv` needed.
- The stub `CHANGELOG.md` files in Tasks 4 and 5 just redirect readers; the runtime
  changelog is authoritative.
- Docker uses `controller-standalone` since it wants the all-in-one binary. Verify
  this matches team expectations before merging.
- The "Extract controller release" step uses `head -1` to pick one version when both
  wrappers release simultaneously — safe because they share `version.workspace = true`
  and always have the same version number.
