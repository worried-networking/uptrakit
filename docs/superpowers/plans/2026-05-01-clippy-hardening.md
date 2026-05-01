# Clippy Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add 24 new Clippy lints enforcing panic safety, error visibility, and unsafe hygiene;
migrate all 95 `#[allow(...)]` suppressions to `#[expect(..., reason = "...")]`.

**Architecture:** All changes are in configuration files and source suppressions — no new logic.
Work proceeds in a strict sequence: scaffold → add lints at warn → fix violations → migrate
legacy allows → promote to deny → update docs. Running both
`--no-default-features --features db-sqlite` and `--all-features` Clippy variants after each
task is mandatory because feature-conditional sites fail under only one variant.

**Tech Stack:** Rust 1.95, Clippy 0.1.95, `cargo clippy`, `#[expect(lint, reason = "...")]` (stable since 1.81)

---

## File Map

| File                                                       | Change                                                                                          |
| ---------------------------------------------------------- | ----------------------------------------------------------------------------------------------- |
| `clippy.toml` (new)                                        | test-mode exemptions                                                                            |
| `Cargo.toml` (workspace root, lines 191–196)               | add `unfulfilled_lint_expectations = "deny"`; add 24 lints                                      |
| `crates/shared/github-client/Cargo.toml`                   | add `[lints] workspace = true`                                                                  |
| `crates/ui/mcp/Cargo.toml`                                 | add `[lints] workspace = true`                                                                  |
| `crates/core/controller-runtime/src/lib.rs`                | cfg_attr→expect (lines 10, 173, 276, 323, 376, 779, 792, 794); feature-cond exception (line 92) |
| `crates/plugins/releases/docker/src/plugin.rs`             | cfg_attr→expect (line 137)                                                                      |
| `crates/plugins/releases/docker/src/daemon_client.rs`      | cfg_attr→expect (line 67)                                                                       |
| `crates/plugins/releases/docker/src/docker_client.rs`      | cfg_attr→expect (line 185)                                                                      |
| `crates/shared/service-sdk/src/lifecycle.rs`               | feature-cond exception (lines 56, 132)                                                          |
| `crates/ui/web-api/src/batch_progress_broadcaster.rs`      | feature-cond exception (line 110)                                                               |
| `crates/shared/agent-core/src/client.rs`                   | feature-cond exception (line 366)                                                               |
| `crates/plugins/infrastructure/registry/src/registry.rs`   | feature-cond exception (lines 20, 218)                                                          |
| `crates/plugins/releases/docker/src/update.rs`             | feature-cond exception (line 92)                                                                |
| `crates/ui/web-api/src/routes/settings_global_combined.rs` | feature-cond exception (line 63)                                                                |
| `crates/plugins/infrastructure/core/src/macros.rs`         | feature-cond exception (lines 78, 187, 191)                                                     |
| All files with `#[allow(...)]` (see Task 6)                | →`#[expect(..., reason = "...")]`                                                               |
| `docs/development/coding-standards.md`                     | add Lint Suppression section                                                                    |

---

## Task 1: Scaffold — `clippy.toml`, Workspace Rust Lints, Missing Crate Coverage

**Files:**

- Create: `clippy.toml`
- Modify: `Cargo.toml` (workspace root, `[workspace.lints.rust]` section, line 191)
- Modify: `crates/shared/github-client/Cargo.toml` (add `[lints]`)
- Modify: `crates/ui/mcp/Cargo.toml` (add `[lints]`)

- [ ] **Create `clippy.toml` at workspace root**

```toml
allow-unwrap-in-tests = true
allow-expect-in-tests = true
allow-panic-in-tests = true
allow-dbg-in-tests = true
allow-indexing-slicing-in-tests = true
```

- [ ] **Add `unfulfilled_lint_expectations = "deny"` to workspace rust lints**

In `Cargo.toml`, find `[workspace.lints.rust]` (line 191) and add the new key:

```toml
[workspace.lints.rust]
warnings = "deny"
unreachable_pub = "deny"
unfulfilled_lint_expectations = "deny"
```

- [ ] **Add missing `[lints]` sections to two crates**

In `crates/shared/github-client/Cargo.toml`, append:

```toml
[lints]
workspace = true
```

In `crates/ui/mcp/Cargo.toml`, append:

```toml
[lints]
workspace = true
```

- [ ] **Verify both Clippy variants are still clean**

```bash
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
```

Expected: zero warnings, zero errors (no new lints yet).

- [ ] **Commit**

```bash
git add clippy.toml Cargo.toml crates/shared/github-client/Cargo.toml crates/ui/mcp/Cargo.toml
git commit -m "chore(clippy): scaffold test-mode exemptions and workspace lint coverage"
```

---

## Task 2: Add New Lints at `"warn"`

**Files:**

- Modify: `Cargo.toml` (workspace root, `[workspace.lints.clippy]` section, line 195)

- [ ] **Add all 24 new lints at `"warn"` level**

In `Cargo.toml`, update `[workspace.lints.clippy]`:

```toml
[workspace.lints.clippy]
all = "deny"

# Panic prevention
unwrap_used = "warn"
expect_used = "warn"
get_unwrap = "warn"
unwrap_in_result = "warn"
panic = "warn"
todo = "warn"
unimplemented = "warn"
unreachable = "warn"
indexing_slicing = "warn"
string_slice = "warn"

# Silent failure prevention
map_err_ignore = "warn"
let_underscore_future = "warn"
let_underscore_must_use = "warn"
unused_result_ok = "warn"
assertions_on_result_states = "warn"

# Async correctness
large_futures = "warn"

# Memory and unsafe hygiene
mem_forget = "warn"
undocumented_unsafe_blocks = "warn"
multiple_unsafe_ops_per_block = "warn"

# Numeric correctness
float_cmp = "warn"

# Suppression hygiene
allow_attributes = "warn"
allow_attributes_without_reason = "warn"

# Hygiene
dbg_macro = "warn"
rc_mutex = "warn"
```

- [ ] **Enumerate all violations to fix in Tasks 3–6**

Run both variants and save output:

```bash
cargo clippy --all-targets --no-default-features --features db-sqlite 2>&1 | grep "^warning" > /tmp/clippy-sqlite.txt
cargo clippy --all-targets --all-features 2>&1 | grep "^warning" > /tmp/clippy-all.txt
wc -l /tmp/clippy-sqlite.txt /tmp/clippy-all.txt
```

Expected: the `--all-features` run shows ~245 warnings (95 `#[allow]` sites × 2 lints = 190, plus ~55
new lint warnings). The `--no-default-features --features db-sqlite` run will show materially fewer —
feature-gated crates are not compiled, so fewer allow sites are visible. Both counts are correct; do
not expect them to match. Use these files to drive Tasks 3–6.

- [ ] **Commit**

```bash
git add Cargo.toml
git commit -m "chore(clippy): add 24 new lints at warn for remediation pass"
```

---

## Task 3: Fix New Lint Violations

This task fixes violations from the 22 newly-enabled non-suppression lints.
`allow_attributes` and `allow_attributes_without_reason` violations are handled in Tasks 4–6.

Run to see only the violations you need to fix here (excluding suppression-hygiene warnings):

```bash
cargo clippy --all-targets --all-features 2>&1 | grep "^warning\[clippy" | grep -v "allow_attributes"
```

**Files:** Wherever violations appear — check the output above. Known violations from measurement:

- `crates/ui/web-api/src/routes/surfaces.rs:723` — `unwrap_used` (existing allow already
  present, will be converted in Task 6)
- `frontend/build.rs` — `expect_used` (~6 violations)
- `crates/shared/tracing-init/src/lib.rs` — `expect_used` (2 violations)
- `crates/shared/types/src/ssrf.rs` — `expect_used` (1 violation)
- `crates/shared/surfaces/src/surface.rs` — `expect_used` (1 violation)
- Various crates — `indexing_slicing` (5 violations), `string_slice` (5 violations)
- `map_err_ignore` (4 violations), `undocumented_unsafe_blocks` (2–4 violations)
- `panic` (1 violation)

For each violation, either:

**Option A — Fix the code** (preferred). Examples:

```rust
// unwrap_used: replace with ? propagation
let val = some_result.unwrap();
// becomes:
let val = some_result?;
// Note: ? only works in functions returning Result/Option. In closures with a
// non-Result return type use unwrap_or_else or restructure into a named fn.

// expect_used in build.rs: change fn main() signature
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // replace .expect("msg") with ?
}

// indexing_slicing: replace arr[i] with get
let item = arr[i];
// becomes:
let item = arr.get(i).ok_or_else(|| report!(MyError::IndexOutOfBounds(i)))?;

// string_slice: replace &s[a..b] with char-boundary-safe split
let chunk = &s[start..end];
// becomes (when boundary already validated):
// use #[expect] with documented reason (see Option B)

// map_err_ignore: .map_err(|_| Foo) → .map_err(|e| { tracing::warn!(...); Foo })
// or preserve source: .context_to::<MyError>()?

// undocumented_unsafe_blocks: add // SAFETY: comment
unsafe {
    ptr.write(val);
}
// becomes:
// SAFETY: ptr is non-null and properly aligned; caller guarantees exclusive access
unsafe {
    ptr.write(val);
}
```

**Option B — Suppress with `#[expect]`** (when the code is correct but the lint fires):

```rust
#[expect(clippy::indexing_slicing, reason = "index is bounded by the preceding len() check")]
let item = arr[idx];

#[expect(clippy::string_slice, reason = "boundary validated by the preceding char_indices() scan")]
let chunk = &s[start..end];
```

**Important — `unreachable!()` in test code:** `clippy.toml` has no `allow-unreachable-in-tests` key.
`unreachable!()` inside `#[cfg(test)]` blocks and `#[test]` functions is NOT auto-exempted.
Any `unreachable!()` in test code must be suppressed with a per-site `#[expect]` OR the call
removed. Similarly, never place `#[expect(clippy::unwrap_used)]` (or `expect_used`, `panic`,
`indexing_slicing`) inside test code — `clippy.toml` already suppresses those lints in tests,
so the `#[expect]` would be unfulfilled and cause a compile error under `unfulfilled_lint_expectations = "deny"`.

- [ ] **Fix or suppress every violation shown by `cargo clippy --all-targets --all-features`** (excluding `allow_attributes` warnings)

- [ ] **Verify both variants are clean of non-suppression violations**

```bash
cargo clippy --all-targets --no-default-features --features db-sqlite 2>&1 | grep "^warning\[clippy" | grep -v "allow_attributes"
cargo clippy --all-targets --all-features 2>&1 | grep "^warning\[clippy" | grep -v "allow_attributes"
```

Expected: zero lines.

- [ ] **Commit**

```bash
git add -p   # stage each changed file
git commit -m "fix(clippy): remediate new lint violations (panic, silent-failure, unsafe)"
```

---

## Task 4: Migrate `cfg_attr` Allow Sites

`#[cfg_attr(cond, allow(lint))]` must become `#[cfg_attr(cond, expect(lint, reason = "..."))]`. Sites span
`crates/core/controller-runtime/src/lib.rs` (8 sites) and the docker plugin crate (3 sites).

**Files:**

- Modify: `crates/core/controller-runtime/src/lib.rs` (lines 10, 173, 276, 323, 376, 779, 792, 794)
- Modify: `crates/plugins/releases/docker/src/plugin.rs` (line 137)
- Modify: `crates/plugins/releases/docker/src/daemon_client.rs` (line 67)
- Modify: `crates/plugins/releases/docker/src/docker_client.rs` (line 185)

- [ ] **Convert lines 10–18** (`embedded` module — multi-line `dead_code`)

```rust
// Before (lines 10–18)
#[cfg_attr(
    not(any(
        feature = "embedded-scheduler",
        feature = "embedded-agent",
        feature = "embedded-ssh-agent",
        feature = "embedded-mqtt"
    )),
    allow(dead_code) // Infrastructure types used by follow-up service embeddings.
)]
mod embedded;

// After
#[cfg_attr(
    not(any(
        feature = "embedded-scheduler",
        feature = "embedded-agent",
        feature = "embedded-ssh-agent",
        feature = "embedded-mqtt"
    )),
    expect(dead_code, reason = "infrastructure types used only when at least one service is embedded")
)]
mod embedded;
```

- [ ] **Convert lines 173–176** (`controller_installation_id` — multi-line `unused_variables`)

```rust
// Before (lines 173–176)
#[cfg_attr(
    not(any(feature = "embedded-scheduler", feature = "embedded-agent")),
    allow(unused_variables)
)]
let controller_installation_id = startup::init_installation_id(app_dirs.state_dir()).await?;

// After
#[cfg_attr(
    not(any(feature = "embedded-scheduler", feature = "embedded-agent")),
    expect(unused_variables, reason = "only read inside the embedded-scheduler and embedded-agent feature blocks below")
)]
let controller_installation_id = startup::init_installation_id(app_dirs.state_dir()).await?;
```

- [ ] **Convert line 276**

```rust
// Before (line 276)
#[cfg_attr(not(feature = "nats"), allow(unused_mut))]
let mut nats_client = /* ... */;

// After
#[cfg_attr(
    not(feature = "nats"),
    expect(unused_mut, reason = "only mutated inside the nats feature block below")
)]
let mut nats_client = /* ... */;
```

- [ ] **Convert line 323**

```rust
// Before (line 323)
#[cfg_attr(not(feature = "nats"), allow(unused_mut))]

// After
#[cfg_attr(
    not(feature = "nats"),
    expect(unused_mut, reason = "only mutated inside the nats feature block below")
)]
```

- [ ] **Convert line 376**

```rust
// Before (line 376)
#[cfg_attr(not(feature = "nats"), allow(unused_mut))]

// After
#[cfg_attr(
    not(feature = "nats"),
    expect(unused_mut, reason = "only mutated inside the nats feature block below")
)]
```

- [ ] **Convert lines 779–786** (`controller_installation_id` param — multi-line `unused_variables`)

```rust
// Before (lines 779–786)
#[cfg_attr(
    not(any(
        feature = "embedded-scheduler",
        feature = "embedded-agent",
        feature = "embedded-mqtt"
    )),
    allow(unused_variables)
)]
controller_installation_id: uuid::Uuid,

// After
#[cfg_attr(
    not(any(
        feature = "embedded-scheduler",
        feature = "embedded-agent",
        feature = "embedded-mqtt"
    )),
    expect(unused_variables, reason = "only read inside the embedded-scheduler, embedded-agent, and embedded-mqtt feature blocks")
)]
controller_installation_id: uuid::Uuid,
```

- [ ] **Convert line 792**

```rust
// Before (line 792)
#[cfg_attr(not(feature = "embedded-agent"), allow(unused_variables))]

// After
#[cfg_attr(
    not(feature = "embedded-agent"),
    expect(unused_variables, reason = "only read inside the embedded-agent feature block")
)]
```

- [ ] **Convert line 794**

```rust
// Before (line 794)
#[cfg_attr(not(feature = "embedded-agent"), allow(unused_variables))] pid_file: Option</* ... */>

// After
#[cfg_attr(
    not(feature = "embedded-agent"),
    expect(unused_variables, reason = "only read inside the embedded-agent feature block")
)] pid_file: Option</* ... */>
```

- [ ] **Convert `docker/src/plugin.rs` line 137** (inline `unused_variables`, `daemon` feature)

```rust
// Before (line 137)
#[cfg_attr(not(feature = "daemon"), allow(unused_variables))] proxy_handle: OpaqueHandle,

// After
#[cfg_attr(not(feature = "daemon"), expect(unused_variables, reason = "only used inside the daemon feature block"))] proxy_handle: OpaqueHandle,
```

- [ ] **Convert `docker/src/daemon_client.rs` line 67** (`dead_code` when not in test)

```rust
// Before (line 67)
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn effective_dial_stdio_command(&self) -> &'static str {

// After
#[cfg_attr(not(test), expect(dead_code, reason = "called only from tests; not dead in test builds"))]
pub(crate) fn effective_dial_stdio_command(&self) -> &'static str {
```

- [ ] **Convert `docker/src/docker_client.rs` line 185** (`dead_code` gated on `cfg(all(unix, feature = "daemon"))`)

```rust
// Before (line 185)
#[cfg_attr(test, allow(dead_code))]
fn probe_local_socket_path() -> Option<String> {

// After
#[cfg_attr(test, expect(dead_code, reason = "function exists only under unix + daemon; unreachable in test builds"))]
fn probe_local_socket_path() -> Option<String> {
```

- [ ] **Verify both variants clean**

```bash
cargo clippy --all-targets --no-default-features --features db-sqlite 2>&1 | grep -E "controller.runtime|docker" | grep "allow_attributes"
cargo clippy --all-targets --all-features 2>&1 | grep -E "controller.runtime|docker" | grep "allow_attributes"
```

Expected: zero lines for these files.

- [ ] **Commit**

```bash
git add \
  crates/core/controller-runtime/src/lib.rs \
  crates/plugins/releases/docker/src/plugin.rs \
  crates/plugins/releases/docker/src/daemon_client.rs \
  crates/plugins/releases/docker/src/docker_client.rs
git commit -m "fix(clippy): convert cfg_attr allow sites to expect"
```

---

## Task 5: Migrate Feature-Conditional Exception Sites

These sites suppress lints that fire only under certain feature flags. `#[expect]` cannot be
used directly because the lint would not fire under the other feature variant, causing
`unfulfilled_lint_expectations`. The solution is to keep `#[allow]` and wrap it with
`#[expect(clippy::allow_attributes, clippy::allow_attributes_without_reason, reason = "...")]`.

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/macros.rs` (lines 78, 187, 191)
- Modify: `crates/shared/service-sdk/src/lifecycle.rs` (lines 56, 132)
- Modify: `crates/ui/web-api/src/batch_progress_broadcaster.rs` (line 110)
- Modify: `crates/core/controller-runtime/src/lib.rs` (line 92)
- Modify: `crates/shared/agent-core/src/client.rs` (line 366)
- Modify: `crates/plugins/infrastructure/registry/src/registry.rs` (lines 20, 218)
- Modify: `crates/plugins/releases/docker/src/update.rs` (line 92)
- Modify: `crates/ui/web-api/src/routes/settings_global_combined.rs` (line 63)

The wrapping pattern for every site in this task:

```rust
#[expect(
    clippy::allow_attributes,
    clippy::allow_attributes_without_reason,
    reason = "feature-conditional: lint fires only when <FEATURE> is disabled; #[expect] would fail under the variant where it does not fire"
)]
#[allow(LINT_NAME)]
```

- [ ] **Verify `macros.rs` sites actually fire `allow_attributes` before wrapping**

`macro_rules!` attribute tokens may be invisible to `allow_attributes` when the macro is
expanded in other crates. Check **both** variants explicitly:

```bash
cargo clippy --all-targets --all-features 2>&1 | grep "macros.rs" | grep "allow_attributes"
cargo clippy --all-targets --no-default-features --features db-sqlite 2>&1 | grep "macros.rs" | grep "allow_attributes"
```

- If lines appear in **both** variants: apply the wrapper pattern below for each site.
- If lines appear in **only one** variant: the outer `#[expect(allow_attributes, ...)]` wrapper
  would fire `unfulfilled_lint_expectations` under the other variant. These sites **cannot** use
  the wrapper pattern. Leave them as bare `#[allow]` — they will fail Task 7 promotion and must be
  addressed separately (likely by moving the allow to the `cfg_attr` pattern instead).
- If no lines appear in either variant: skip the three `macros.rs` steps entirely — no change needed.

- [ ] **`macros.rs` line 78** — inside `declare_plugin!` macro body

```rust
// Before
#[allow(unused_imports, dead_code)]

// After
#[expect(
    clippy::allow_attributes,
    clippy::allow_attributes_without_reason,
    reason = "inside macro_rules! body — lint fires selectively at call sites by feature; #[expect] fails under variants where it does not fire"
)]
#[allow(unused_imports, dead_code)]
```

- [ ] **`macros.rs` line 187** — inside `declare_plugin!` macro body

```rust
// Before
#[allow(unused)]

// After
#[expect(
    clippy::allow_attributes,
    clippy::allow_attributes_without_reason,
    reason = "inside macro_rules! body — lint fires selectively at call sites by feature; #[expect] fails under variants where it does not fire"
)]
#[allow(unused)]
```

- [ ] **`macros.rs` line 191** — inside `declare_plugin!` macro body

```rust
// Before
#[allow(unused_mut)]

// After
#[expect(
    clippy::allow_attributes,
    clippy::allow_attributes_without_reason,
    reason = "inside macro_rules! body — lint fires selectively at call sites by feature; #[expect] fails under variants where it does not fire"
)]
#[allow(unused_mut)]
```

- [ ] **`lifecycle.rs` line 56** — `unused_variables`, feature-gated path

```rust
// Before
#[allow(unused_variables)]

// After
#[expect(
    clippy::allow_attributes,
    clippy::allow_attributes_without_reason,
    reason = "feature-conditional: variable is used only inside the zeroconf feature block; #[expect] fails when zeroconf is enabled"
)]
#[allow(unused_variables)]
```

- [ ] **`lifecycle.rs` line 132** — `unreachable_code` when `zeroconf` active

```rust
// Before
#[allow(unreachable_code)]

// After
#[expect(
    clippy::allow_attributes,
    clippy::allow_attributes_without_reason,
    reason = "feature-conditional: code is unreachable only when zeroconf is disabled; #[expect] fails when zeroconf is enabled"
)]
#[allow(unreachable_code)]
```

- [ ] **`batch_progress_broadcaster.rs` line 110** — `unreachable_code` when `nats` active

```rust
// Before
#[allow(unreachable_code)]

// After
#[expect(
    clippy::allow_attributes,
    clippy::allow_attributes_without_reason,
    reason = "feature-conditional: code is unreachable only when nats is disabled; #[expect] fails when nats is enabled"
)]
#[allow(unreachable_code)]
```

- [ ] **`controller-runtime/src/lib.rs` line 92** — `unused_mut` when `journald` active

```rust
// Before
#[allow(unused_mut)] // mutated inside #[cfg(feature = "journald")] block

// After
#[expect(
    clippy::allow_attributes,
    clippy::allow_attributes_without_reason,
    reason = "feature-conditional: variable is mutated inside the journald feature block; #[expect] fails when journald is enabled"
)]
#[allow(unused_mut)]
```

- [ ] **`agent-core/src/client.rs` line 366** — `unused_assignments, unused_mut` (interactive)

Read lines 360–380 to confirm the feature name, then apply:

```rust
#[expect(
    clippy::allow_attributes,
    clippy::allow_attributes_without_reason,
    reason = "feature-conditional: variable is assigned inside the interactive feature block; #[expect] fails when interactive is enabled"
)]
#[allow(unused_assignments, unused_mut)]
```

- [ ] **`registry/src/registry.rs` line 20** — `unused_mut`

Read lines 15–30 to confirm the feature condition, then apply:

```rust
#[expect(
    clippy::allow_attributes,
    clippy::allow_attributes_without_reason,
    reason = "feature-conditional: variable is mutated only when plugin features are enabled; #[expect] fails under all-features"
)]
#[allow(unused_mut)]
```

- [ ] **`registry/src/registry.rs` line 218** — `unused_mut`

Same pattern as line 20.

- [ ] **`docker/src/update.rs` line 92** — `unused_variables` (daemon feature)

```rust
#[expect(
    clippy::allow_attributes,
    clippy::allow_attributes_without_reason,
    reason = "feature-conditional: variable is used inside the daemon feature block; #[expect] fails when daemon is enabled"
)]
#[allow(unused_variables)]
```

- [ ] **`settings_global_combined.rs` line 63** — `unused_assignments, unused_mut` (nats)

```rust
#[expect(
    clippy::allow_attributes,
    clippy::allow_attributes_without_reason,
    reason = "feature-conditional: variable is assigned inside the nats feature block; #[expect] fails when nats is enabled"
)]
#[allow(unused_assignments, unused_mut)]
```

- [ ] **Verify both variants show no `allow_attributes` warnings for these files**

```bash
cargo clippy --all-targets --no-default-features --features db-sqlite 2>&1 | grep "allow_attributes" | grep -E "macros|lifecycle|batch_progress|controller.runtime|agent.core|registry|docker.*update|settings_global"
cargo clippy --all-targets --all-features 2>&1 | grep "allow_attributes" | grep -E "macros|lifecycle|batch_progress|controller.runtime|agent.core|registry|docker.*update|settings_global"
```

Expected: zero lines.

- [ ] **Commit**

```bash
git add \
  crates/plugins/infrastructure/core/src/macros.rs \
  crates/shared/service-sdk/src/lifecycle.rs \
  crates/ui/web-api/src/batch_progress_broadcaster.rs \
  crates/core/controller-runtime/src/lib.rs \
  crates/shared/agent-core/src/client.rs \
  crates/plugins/infrastructure/registry/src/registry.rs \
  crates/plugins/releases/docker/src/update.rs \
  crates/ui/web-api/src/routes/settings_global_combined.rs
git commit -m "fix(clippy): wrap feature-conditional allow sites with expect(allow_attributes)"
```

---

## Task 6: Migrate Standard `#[allow]` Sites to `#[expect]`

All remaining `#[allow]` sites receive a straightforward conversion:

```rust
// Before
#[allow(clippy::too_many_arguments)]

// After
#[expect(clippy::too_many_arguments, reason = "mirrors the N fields of <RecordType>")]
```

Work through the sites below in sub-batches, committing after each group. Run both Clippy variants after each sub-batch.

### 6a: `clippy::too_many_arguments` (25 sites)

Reason pattern: `"mirrors the N fields of <struct/record>"` — read the function signature to count parameters and name the record type.

- [ ] **Convert all 25 sites**

| File                                                                       | Line                 |
| -------------------------------------------------------------------------- | -------------------- |
| `crates/core/agent-ssh/src/commands/bootstrap.rs`                          | 366                  |
| `crates/core/agent-ssh/src/operations/bootstrap.rs`                        | 726                  |
| `crates/core/controller-runtime/src/embedded/mod.rs`                       | 116                  |
| `crates/core/controller-runtime/src/embedded/provision.rs`                 | 244                  |
| `crates/core/controller-runtime/src/embedded/provision.rs`                 | 276                  |
| `crates/core/controller-runtime/src/lib.rs`                                | 766                  |
| `crates/core/controller-runtime/src/service_host/embedded_host.rs`         | 79                   |
| `crates/core/mqtt-runtime/src/ha_discovery/device.rs`                      | 84                   |
| `crates/plugins/infrastructure/core/src/agent_infra.rs`                    | 55                   |
| `crates/plugins/infrastructure/proxmox/src/agent/surface_actions.rs`       | 286                  |
| `crates/shared/scheduler-engine/src/executors/awaiting_restart.rs`         | 527                  |
| `crates/shared/service-sdk/src/surface_proxy.rs`                           | 122                  |
| `crates/shared/web-api-types/src/update_history.rs`                        | 161                  |
| `crates/ui/web-api-queries/src/queries/software_items/host_assignments.rs` | 277                  |
| `crates/ui/web-api-queries/src/queries/software_items/mod.rs`              | 171                  |
| `crates/ui/web-api/src/integration_tests/software_items_crud.rs`           | 130                  |
| `crates/ui/web-api/src/routes/oidc_auth.rs`                                | 873, 974, 1188, 1236 |
| `crates/ui/web-api/src/routes/service_ws/handler/mod.rs`                   | 2241, 2335, 2622     |
| `crates/ui/web-api/src/surface_proxy.rs`                                   | 1873                 |
| `crates/ui/web-api/src/surface_proxy/dispatch.rs`                          | 62                   |

For each: read the function signature, count parameters, name the record. Example for `bootstrap.rs:366`:

```rust
#[expect(
    clippy::too_many_arguments,
    reason = "mirrors the 8 fields of SshDeploymentKey — splitting into a builder would obscure the call sites"
)]
```

- [ ] **Verify after 6a**

```bash
cargo clippy --all-targets --all-features 2>&1 | grep "too_many_arguments"
```

Expected: zero.

- [ ] **Commit 6a**

```bash
git add -p
git commit -m "fix(clippy): convert too_many_arguments allow→expect with reason"
```

### 6b: `clippy::type_complexity` (13 sites)

Reason pattern: `"SeaORM join chain; extracting a type alias adds no clarity here"`

- [ ] **Convert all 13 sites**

| File                                                                  | Line          |
| --------------------------------------------------------------------- | ------------- |
| `crates/plugins/infrastructure/core/src/descriptor.rs`                | 341           |
| `crates/ui/web-api-queries/src/queries/autodiscovery/ignore_rules.rs` | 102           |
| `crates/ui/web-api-queries/src/queries/host_tags.rs`                  | 363           |
| `crates/ui/web-api-queries/src/queries/hosts.rs`                      | 398           |
| `crates/ui/web-api-queries/src/queries/plugin_configs.rs`             | 263           |
| `crates/ui/web-api-queries/src/queries/services.rs`                   | 523, 718, 766 |
| `crates/ui/web-api-queries/src/queries/software_items/crud.rs`        | 661, 701      |
| `crates/ui/web-api-queries/src/queries/system_services.rs`            | 366, 409, 453 |

```rust
#[expect(
    clippy::type_complexity,
    reason = "SeaORM join chain; extracting a type alias adds no clarity here"
)]
```

- [ ] **Verify after 6b**

```bash
cargo clippy --all-targets --all-features 2>&1 | grep "type_complexity"
```

Expected: zero.

- [ ] **Commit 6b**

```bash
git add -p
git commit -m "fix(clippy): convert type_complexity allow→expect with reason"
```

### 6c: `dead_code` (29 sites)

Reason: describe why the item is kept. Read each site to determine the reason. Common patterns:

- Feature-gated items: `"used only when <feature> is enabled"`
- Test helpers: `"used by integration tests via mod.rs re-export"`
- Pending wiring: `"infrastructure type reserved for follow-up service embeddings"`

- [ ] **Convert all 29 sites**

| File                                                                           | Line                                                                   | Known context                                                                                                                                                                            |
| ------------------------------------------------------------------------------ | ---------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/core/controller-runtime/src/embedded/mod.rs`                           | 17                                                                     | "infrastructure types for follow-up embeddings"                                                                                                                                          |
| `crates/core/controller-runtime/src/embedded/mod.rs`                           | 52                                                                     | "fields used by agent embedding follow-up"                                                                                                                                               |
| `crates/core/controller-runtime/src/embedded/types.rs`                         | 33                                                                     | "transport methods used by service closures"                                                                                                                                             |
| `crates/core/integration-tests/tests/database_helpers/harness.rs`              | 14                                                                     | read file for context                                                                                                                                                                    |
| `crates/core/integration-tests/tests/database_helpers/mod.rs`                  | 1, 3, 5, 7, 9                                                          | "test helper re-exports used by sibling test files"                                                                                                                                      |
| `crates/plugins/hooks/shell/src/plugin.rs`                                     | 20                                                                     | read for context                                                                                                                                                                         |
| `crates/plugins/infrastructure/core/src/catalog.rs`                            | 344 (**both** `dead_code` + `unreachable_pub`), 392 (`dead_code` only) | line 344 is `#[allow(dead_code, unreachable_pub)]` on `mod tests` — convert to `#[expect(dead_code, unreachable_pub, reason = "test-only module; items not re-exported at crate root")]` |
| `crates/plugins/notifications/webhook/src/plugin.rs`                           | 46                                                                     | read for context                                                                                                                                                                         |
| `crates/plugins/releases/docker/src/plugin.rs`                                 | 105, 166                                                               | read for context                                                                                                                                                                         |
| `crates/shared/db/src/migration/m20260309_000003_unified_software_tracking.rs` | 56, 58, 170                                                            | "migration helper used by SeaORM migration runner"                                                                                                                                       |
| `crates/shared/openapi-client/src/lib.rs`                                      | 370, 389                                                               | read for context                                                                                                                                                                         |
| `crates/ui/web-api/src/global_providers/github.rs`                             | 149, 386, 392, 398, 410, 626                                           | read for context                                                                                                                                                                         |
| `crates/ui/web-api/src/routes/service_ws/protocol.rs`                          | 94                                                                     | "used by tests in mod.rs"                                                                                                                                                                |
| `crates/ui/web-api/src/surface_proxy/controller_local.rs`                      | 67                                                                     | read for context                                                                                                                                                                         |
| `crates/ui/web-api/src/test_harness/mod.rs`                                    | 23                                                                     | "test harness helper"                                                                                                                                                                    |

For each site, read the surrounding ~10 lines to write a specific reason.

- [ ] **Verify after 6c**

```bash
cargo clippy --all-targets --all-features 2>&1 | grep "dead_code"
```

Expected: zero.

- [ ] **Commit 6c**

```bash
git add -p
git commit -m "fix(clippy): convert dead_code allow→expect with reason"
```

### 6d: Remaining `unused_*` and Miscellaneous Sites

- [ ] **Convert `unused_imports` (3 sites in `controller_local.rs`)**

`crates/ui/web-api/src/surface_proxy/controller_local.rs` lines 45, 54, 58.

Read each site before applying. Line 54 is inside a `#[cfg(test)]` block — use a test-specific reason:

Lines 45 and 58 (production code — pending wiring):

```rust
#[expect(
    unused_imports,
    reason = "pending wiring of local_executor.rs into the module tree — remove once local_executor.rs is incorporated"
)]
```

Line 54 (inside `#[cfg(test)]`):

```rust
#[expect(
    unused_imports,
    reason = "test-only import used by local_executor.rs tests — remove once local_executor.rs is incorporated"
)]
```

- [ ] **Convert `unused_variables` (2 sites)**

`crates/core/controller-runtime/src/lib.rs:770` and `crates/ui/web-api/src/event_broadcaster.rs:131`:

Read each site; reason describes which feature or path uses the variable.

- [ ] **Convert `unused_mut` in `agent-ssh/src/client.rs:488`**

Read context; typical reason: `"mut required by the ... API even though this path does not mutate"`

- [ ] **Convert `unused_mut` in `agent-ssh/src/db/migration/mod.rs:27`**

```rust
#[expect(unused_mut, reason = "migrations vec is mutated conditionally by push calls below")]
```

- [ ] **Convert remaining miscellaneous allows**

| File                                                                               | Line    | Lint                              | Suggested reason                                                                                          |
| ---------------------------------------------------------------------------------- | ------- | --------------------------------- | --------------------------------------------------------------------------------------------------------- |
| `crates/core/agent-ssh/src/db/migration/m20260308_000003_ssh_host_uuid_columns.rs` | 263     | `clippy::enum_variant_names`      | `"migration enum variants share prefix by convention"`                                                    |
| `crates/plugins/infrastructure/core/src/descriptor.rs`                             | 443     | `clippy::derivable_impls`         | `"cfg-gated fields prevent automatic derive"`                                                             |
| `crates/plugins/infrastructure/core/src/testing.rs`                                | 62, 134 | `clippy::new_ret_no_self`         | `"builder returns a concrete plugin type, not Self by design"`                                            |
| `crates/shared/openapi-client/src/lib.rs`                                          | 54      | `unused_imports`                  | read for context                                                                                          |
| `crates/ui/web-api-queries/src/queries/autodiscovery/mod.rs`                       | 94      | `unreachable_pub`                 | `"public item needed by integration tests but not re-exported at crate root"`                             |
| `crates/ui/web-api/src/routes/plugin_type_settings.rs`                             | 35      | `clippy::result_large_err`        | `"error type is large due to axum Response; refactoring would require significant handler restructuring"` |
| `crates/ui/web-api/src/routes/surfaces.rs`                                         | 723     | `clippy::unwrap_used`             | read and document the specific invariant                                                                  |
| `crates/core/mqtt-runtime/src/surface_runtime.rs`                                  | 285     | `clippy::items_after_test_module` | `"helper functions defined after test module for code locality"`                                          |

- [ ] **Verify after 6d**

```bash
cargo clippy --all-targets --no-default-features --features db-sqlite 2>&1 | grep "allow_attributes"
cargo clippy --all-targets --all-features 2>&1 | grep "allow_attributes"
```

Expected: zero lines (all 95 sites converted or wrapped).

- [ ] **Commit 6d**

```bash
git add -p
git commit -m "fix(clippy): convert remaining allow→expect sites (unused, misc)"
```

---

## Task 7: Promote All New Lints to `"deny"`

**Files:**

- Modify: `Cargo.toml` (workspace root, `[workspace.lints.clippy]`)

- [ ] **Change all 24 new lints from `"warn"` to `"deny"`**

In `Cargo.toml`, update every `= "warn"` entry added in Task 2 to `= "deny"`:

```toml
[workspace.lints.clippy]
all = "deny"

# Panic prevention
unwrap_used = "deny"
expect_used = "deny"
get_unwrap = "deny"
unwrap_in_result = "deny"
panic = "deny"
todo = "deny"
unimplemented = "deny"
unreachable = "deny"
indexing_slicing = "deny"
string_slice = "deny"

# Silent failure prevention
map_err_ignore = "deny"
let_underscore_future = "deny"
let_underscore_must_use = "deny"
unused_result_ok = "deny"
assertions_on_result_states = "deny"

# Async correctness
large_futures = "deny"

# Memory and unsafe hygiene
mem_forget = "deny"
undocumented_unsafe_blocks = "deny"
multiple_unsafe_ops_per_block = "deny"

# Numeric correctness
float_cmp = "deny"

# Suppression hygiene
allow_attributes = "deny"
allow_attributes_without_reason = "deny"

# Hygiene
dbg_macro = "deny"
rc_mutex = "deny"
```

- [ ] **Final verification — both variants must be fully clean**

```bash
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
```

Expected: exit 0, zero warnings, zero errors.

- [ ] **Run tests to confirm no regressions**

```bash
cargo test --all-features
```

Expected: all tests pass.

- [ ] **Commit**

```bash
git add Cargo.toml
git commit -m "chore(clippy): promote all 24 new lints from warn to deny"
```

---

## Task 8: Update Coding Standards

**Files:**

- Modify: `docs/development/coding-standards.md` (after the **Panic Policy** section, after line 42)

- [ ] **Insert new Lint Suppression section**

After the closing ` ``` ` of the Panic Policy code block, insert:

````markdown
## Lint Suppression

Use `#[expect(lint_name, reason = "...")]`, never `#[allow(lint_name)]`. The `reason` field
is mandatory (`allow_attributes_without_reason = "deny"`). When the lint stops firing at a
site, the `#[expect]` becomes a compile error via `unfulfilled_lint_expectations`, so stale
suppressions are caught automatically.

```rust
// ✓ Correct
#[expect(clippy::too_many_arguments, reason = "mirrors the eight DB columns of Update")]
fn create_update_record(…) { … }

// ✗ Wrong — no reason, and will silently persist if the lint is fixed
#[allow(clippy::too_many_arguments)]
fn create_update_record(…) { … }
```

When two lints fire on the same expression, list both in one attribute:

```rust
#[expect(clippy::unwrap_used, clippy::unwrap_in_result, reason = "infallible: regex compiled from a literal")]
let re = Regex::new(PATTERN).unwrap();
```

**Feature-conditional exception.** Sites where a lint fires only under certain feature flags
cannot use `#[expect]` — an unsatisfied `#[expect]` becomes a compile error under the feature
variant where the lint does not fire. These sites keep `#[allow]` with a wrapping `#[expect]`
covering both suppression lints:

```rust
#[expect(
    clippy::allow_attributes,
    clippy::allow_attributes_without_reason,
    reason = "feature-conditional: lint fires only when <feature> is disabled; #[expect] fails under the variant where it does not fire"
)]
#[allow(unused_mut)]
```
````

- [ ] **Verify markdownlint passes**

```bash
npx prettier --write docs/development/coding-standards.md
markdownlint --config .markdownlint.json docs/development/coding-standards.md
```

Expected: no errors.

- [ ] **Commit**

```bash
git add docs/development/coding-standards.md
git commit -m "docs(standards): add Lint Suppression section documenting #[expect] pattern"
```

---

## Self-Review

**Spec coverage:**

| Spec requirement                                       | Task      |
| ------------------------------------------------------ | --------- |
| `clippy.toml` with test exemptions                     | Task 1    |
| `unfulfilled_lint_expectations = "deny"`               | Task 1    |
| Two crates missing `[lints] workspace = true`          | Task 1    |
| 24 new lints at warn                                   | Task 2    |
| Fix new lint violations                                | Task 3    |
| `cfg_attr` allow sites → `cfg_attr` expect             | Task 4    |
| Feature-conditional exception pattern                  | Task 5    |
| Standard `#[allow]` → `#[expect]` migration (95 sites) | Task 6    |
| Promote all lints to deny                              | Task 7    |
| Coding standards update                                | Task 8    |
| Both feature variants verified throughout              | All tasks |

All spec sections are covered. ✓
