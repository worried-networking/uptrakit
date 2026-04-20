# Surface Runtime Decomposition Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to
> implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split `SurfaceProxy` and the SSH surface runtime into focused modules with typed internal request shapes, and finish converging the
remaining package-manager duplication by moving the APT and snap batch read paths onto a shared core seam without inventing a parallel helper family.

**Architecture:** Keep this track last. First validate whether the remaining APT/snap batch read paths can share the existing core helper surface with
only a small extension in `helpers.rs`; do not introduce a third orchestration family that overlaps with `execute_batch_names_command` /
`execute_batch_versioned_command`. Then split `SurfaceProxy` without changing behavior, then split the SSH runtime into domain modules and typed
request structs. Existing runtime tests stay as behavioral anchors while the code moves behind thinner coordinators. This plan runs after the
plugin-extension typing plan has landed in `crates/ui/web-api/src/surface_proxy.rs`, so any edge-only error mapping added there moves into the new
module layout as part of this split instead of being reintroduced separately.

**Tech Stack:** Rust workspace crates (`uptrakit-web-api`, `uptrakit-agent-ssh`, package-manager plugins, `uptrakit-plugin-infrastructure-core`),
Tokio async tests, package-specific unit tests, cargo package checks

---

## File Structure

### Package-manager orchestration

- Modify:
  [`crates/plugins/infrastructure/core/src/helpers.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/core/src/helpers.rs)
  Responsibility: host only the minimal shared read-path helper changes needed by the remaining APT/snap adopters, alongside the existing
  `batch_detect.rs`, `batch_fetch.rs`, and `batch_update.rs` support modules rather than duplicating them elsewhere.
- Modify:
  [`crates/plugins/package-managers/apt/src/detection.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/package-managers/apt/src/detection.rs)
- Modify:
  [`crates/plugins/package-managers/apt/src/releases.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/package-managers/apt/src/releases.rs)
- Modify:
  [`crates/plugins/package-managers/snap/src/detection.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/package-managers/snap/src/detection.rs)

### Web runtime split

- Modify: [`crates/ui/web-api/src/surface_proxy.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/surface_proxy.rs) Responsibility:
  reduce `invoke_inner` to a coordinator and move grouped logic into dedicated modules/helpers.
- Create as needed: `crates/ui/web-api/src/surface_proxy/*.rs` Responsibility: request resolution, validation, gating, local execution, proxied
  execution, timeout/bookkeeping.

### SSH runtime split

- Modify: [`crates/core/agent-ssh/src/surface_runtime.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/surface_runtime.rs)
  Responsibility: reduce the file to a coordinator and move registration, dispatch, bootstrap, sync, param parsing, and controller-proxy helpers into
  submodules.
- Create as needed: `crates/core/agent-ssh/src/surface_runtime/*.rs`

### Verification commands

- `cargo fmt --all`
- `cargo test -p uptrakit-plugin-infrastructure-core`
- `cargo check -p uptrakit-plugin-infrastructure-core`
- `cargo test -p uptrakit-web-api surface_proxy -- --nocapture`
- `cargo check -p uptrakit-web-api`
- `cargo clippy -p uptrakit-web-api --all-targets`
- `cargo check -p uptrakit-agent-ssh`
- `cargo test -p uptrakit-agent-ssh`
- `cargo clippy -p uptrakit-agent-ssh --all-targets`
- `cargo check -p uptrakit-plugin-package-manager-apt`
- `cargo check -p uptrakit-plugin-package-manager-dnf`
- `cargo check -p uptrakit-plugin-package-manager-pkg`
- `cargo check -p uptrakit-plugin-package-manager-snap`

### Task 1: Finish The Remaining Shared Package-Manager Adoptions

**Files:**

- Modify:
  [`crates/plugins/infrastructure/core/src/helpers.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/core/src/helpers.rs)
- Modify:
  [`crates/plugins/package-managers/apt/src/detection.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/package-managers/apt/src/detection.rs)
- Modify:
  [`crates/plugins/package-managers/apt/src/releases.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/package-managers/apt/src/releases.rs)
- Modify:
  [`crates/plugins/package-managers/snap/src/detection.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/package-managers/snap/src/detection.rs)

- [ ] **Step 1: Snapshot the existing helper/adopter baseline**

Run:

```bash
cargo test -p uptrakit-plugin-infrastructure-core
cargo test -p uptrakit-plugin-package-manager-apt
cargo test -p uptrakit-plugin-package-manager-snap
```

Expected: PASS and confirms the current shared update helpers already cover DNF, pkg, and snap-update behavior, so this slice only needs to address
the remaining APT/snap read-path duplication.

- [ ] **Step 2: Move only the remaining duplicate adopters onto the shared seam**

Start from the existing shared helper surface. If the APT/snap read paths need a new shared primitive in `helpers.rs`, keep it as a small extension
that both adopters actually use. Do not introduce a separate `execute_batch_query_command` or `BatchQueryParams` family. If a new primitive proves
necessary, add one focused red test for that exact APT/snap behavior in `helpers.rs` before writing the helper so this task keeps the same red-green
discipline as the rest of the plan family.

Ground rules for this slice:

- APT `batch_detect` and `batch_fetch`, plus snap `batch_detect`, are the only intended adopters in this task.
- DNF, pkg, and snap update paths stay unchanged here because they already use the shared update helpers.
- Homebrew and npm remain out of scope unless implementation proves they fit the same minimal extension with no extra branching.

- [ ] **Step 3: Run the first-slice package checks**

Run:

```bash
cargo test -p uptrakit-plugin-infrastructure-core
cargo check -p uptrakit-plugin-package-manager-apt
cargo check -p uptrakit-plugin-package-manager-snap
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/plugins/infrastructure/core/src/helpers.rs crates/plugins/package-managers/apt/src/detection.rs crates/plugins/package-managers/apt/src/releases.rs crates/plugins/package-managers/snap/src/detection.rs
git commit -m "refactor: finish shared package manager helper adoption"
```

### Task 2: Split `SurfaceProxy` Into Responsibility Modules

**Files:**

- Modify: [`crates/ui/web-api/src/surface_proxy.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/surface_proxy.rs)
- Create as needed under: `crates/ui/web-api/src/surface_proxy/`

- [ ] **Step 1: Snapshot the current `SurfaceProxy` behavior tests**

Run:

```bash
cargo test -p uptrakit-web-api surface_proxy -- --nocapture
```

Expected: PASS and establishes the current behavior baseline across caller origin, idempotency, timeout, local-controller execution, and proxied
paths.

- [ ] **Step 2: Move grouped responsibilities into dedicated modules**

Target layout:

```rust
mod resolution;
mod validation;
mod gating;
mod dispatch;
mod local_exec;
mod proxied_exec;
mod bookkeeping;
```

Keep `invoke_inner` thin:

```rust
async fn invoke_inner(&self, request: SurfaceInvokeRequest) -> Result<serde_json::Value, SurfaceProxyError> {
    let resolved = resolution::resolve_request(self, &request).await?;
    validation::validate_request(&resolved)?;
    gating::enforce_rollout_and_idempotency(self, &resolved).await?;
    dispatch::invoke(self, resolved).await
}
```

- [ ] **Step 3: Re-run the proxy tests**

Run:

```bash
cargo test -p uptrakit-web-api surface_proxy -- --nocapture
```

Expected: PASS with the moved tests still covering caller-origin, schema/idempotency, timeout, and local/proxied routing behavior.

- [ ] **Step 4: Commit**

```bash
git add crates/ui/web-api/src/surface_proxy.rs crates/ui/web-api/src/surface_proxy
git commit -m "refactor: split surface proxy responsibilities"
```

### Task 3: Split The SSH Runtime And Introduce Typed Internal Requests

**Files:**

- Modify: [`crates/core/agent-ssh/src/surface_runtime.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/surface_runtime.rs)
- Create as needed under: `crates/core/agent-ssh/src/surface_runtime/`

- [ ] **Step 1: Snapshot the current SSH runtime package tests**

Run:

```bash
cargo test -p uptrakit-agent-ssh
```

Expected: PASS and establishes baseline dispatch/bootstrap/sync behavior.

- [ ] **Step 2: Create typed request structs and move runtime domains out**

Extract internal request types such as:

```rust
struct BootstrapRequest {
    hostname: String,
    username: String,
    auth: BootstrapAuth,
}

struct SyncHostRequest {
    host_id: Uuid,
    auth_override: Option<AuthOverride>,
}
```

Split the file into:

```rust
mod registration;
mod dispatch;
mod params;
mod bootstrap;
mod sync;
mod proxy;
```

- [ ] **Step 3: Re-run the SSH package tests**

Run:

```bash
cargo test -p uptrakit-agent-ssh
cargo check -p uptrakit-agent-ssh
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/core/agent-ssh/src/surface_runtime.rs crates/core/agent-ssh/src/surface_runtime
git commit -m "refactor: split ssh surface runtime domains"
```

### Task 4: Final Cross-Track Verification

**Files:**

- Modify any moved imports/tests surfaced by package checks.

- [ ] **Step 1: Run the full runtime-decomposition verification set**

Run:

```bash
cargo fmt --all
cargo test -p uptrakit-plugin-infrastructure-core
cargo check -p uptrakit-plugin-infrastructure-core
cargo test -p uptrakit-web-api surface_proxy -- --nocapture
cargo check -p uptrakit-web-api
cargo clippy -p uptrakit-web-api --all-targets
cargo test -p uptrakit-agent-ssh
cargo check -p uptrakit-agent-ssh
cargo clippy -p uptrakit-agent-ssh --all-targets
cargo check -p uptrakit-plugin-package-manager-apt
cargo check -p uptrakit-plugin-package-manager-dnf
cargo check -p uptrakit-plugin-package-manager-pkg
cargo check -p uptrakit-plugin-package-manager-snap
```

Expected: PASS, with the helper tests covering names-only orchestration, version-embedded orchestration, parser-result fan-out, and representative
failure mapping.

- [ ] **Step 2: Commit any last integration cleanups**

```bash
git add crates/plugins/infrastructure/core/src/helpers.rs crates/plugins/package-managers/apt/src/detection.rs crates/plugins/package-managers/apt/src/releases.rs crates/plugins/package-managers/snap/src/detection.rs crates/ui/web-api/src/surface_proxy.rs crates/ui/web-api/src/surface_proxy crates/core/agent-ssh/src/surface_runtime.rs crates/core/agent-ssh/src/surface_runtime
git commit -m "chore: finish runtime decomposition verification"
```

## Self-Review

- Spec coverage: Task 1 covers the shared package-manager helper and first-slice adoption. Task 2 covers `SurfaceProxy`. Task 3 covers the SSH
  runtime. Task 4 closes the test/check matrix.
- Placeholder scan: no unfinished-plan markers remain.
- Type consistency: the plan uses the same module split and verification commands throughout.
