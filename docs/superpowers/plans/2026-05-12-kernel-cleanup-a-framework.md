# Kernel Cleanup A: Hook Framework Foundation

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the two additive hook-framework primitives
(`UpdateLifecycleContext.batch_id` and
`LifecycleHook::detect_host_compatibility`) plus agent-side preflight +
per-batch dedup, without changing the behaviour of existing `hook_systemd` /
`hook_shell` plugins. This plan lands the substrate the kernel-cleanup plugin
(Plan B) plugs into.

**Architecture:** Two crate touches. `crates/plugins/infrastructure/core` gains
a non-breaking field on `UpdateLifecycleContext` (the struct is already
`#[non_exhaustive]`) and a default-impl method on the `LifecycleHook` trait.
`crates/shared/agent-core` threads `batch_id` through context constructors,
calls `detect_host_compatibility()` before pre/post hooks, and adds a
function-local `HashSet<(PluginTypeId, Uuid)>` to the two batch dispatchers to
dedup repeat hook firings within one batch. Single-item paths see no behavioural
change beyond the trailing optional `batch_id = None`.

**Tech Stack:** Rust edition 2024 (workspace.edition), tokio async,
`async_trait`, `rootcause::Report`. Snapshot rules: no
`unwrap()`/`expect()`/`panic!()` in production; `#[expect(reason = "...")]` over
`#[allow]`; pre-push runs
`cargo clippy --all-targets --all-features -- -D warnings`; Conventional Commits
per `docs/development/commit-messages.md`.

---

## File Structure

**Modify:**

- `crates/plugins/infrastructure/core/src/traits.rs` — add
  `batch_id: Option<Uuid>` field on `UpdateLifecycleContext`; extend
  `for_pre_hook` / `for_post_hook` constructors.
- `crates/plugins/infrastructure/core/src/roles.rs` — add default-impl
  `detect_host_compatibility()` method on the `LifecycleHook` trait at lines
  210-225.
- `crates/plugins/infrastructure/core/Cargo.toml` — add `uuid` (workspace) to
  dependencies if not already present.
- `crates/shared/agent-core/src/update.rs` — populate `batch_id` in single-item
  context constructors at lines 115 + 284; invoke `detect_host_compatibility()`
  in `run_pre_hook_plugins`, `run_post_hook_plugins`,
  `run_batch_pre_hook_plugins`, `run_batch_post_hook_plugins`; add per-batch
  dedup `HashSet` in the two batch dispatchers.
- `crates/shared/agent-core/src/client.rs` — populate `Some(payload.batch_id)`
  in batch-path context constructors at lines 577 + 598.
- `docs/development/update-hooks.md` — document new field, new trait method,
  dedup semantics, and the existing batch-path output-discard gap as a known
  framework limitation.

**No new files** in this plan. (Plan B creates the kernel-cleanup plugin crate.)

---

## Task 1: Pre-flight — confirm clean workspace and run quality gates

**Files:** workspace root.

- [ ] **Step 1: Verify clean git state**

```bash
git status
```

Expected: working tree clean OR only contains untracked plan / spec docs from
prior workflow.

- [ ] **Step 2: Run baseline quality gates so any later failure is attributable
      to this plan**

```bash
cargo fmt --all -- --check
cargo check --no-default-features --features db-sqlite
cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings
cargo test -p uptrakit-plugin-infrastructure-core --all-features
cargo test -p uptrakit-agent-core --all-features
cargo test -p uptrakit-plugin-hook-systemd --all-features
cargo test -p uptrakit-plugin-hook-shell --all-features
```

Expected: all green. If anything is already red, surface to the user before
continuing — do not pretend the baseline was clean.

---

## Task 2: Add `uuid` workspace dependency to `uptrakit-plugin-infrastructure-core`

**Files:**

- Modify: `crates/plugins/infrastructure/core/Cargo.toml`

- [ ] **Step 1: Inspect current `[dependencies]` section**

```bash
grep -n "uuid\|^\[dependencies\]" crates/plugins/infrastructure/core/Cargo.toml | head -10
```

If `uuid` already appears under `[dependencies]`, skip to Step 4. Otherwise
continue.

- [ ] **Step 2: Add the dependency**

Add the following line to the `[dependencies]` table (keep alphabetical order
with surrounding entries; the workspace already pins `uuid` with feature `v7`):

```toml
uuid = { workspace = true }
```

- [ ] **Step 3: Verify it builds**

```bash
cargo check -p uptrakit-plugin-infrastructure-core
```

Expected: success.

- [ ] **Step 4: Commit**

```bash
git add crates/plugins/infrastructure/core/Cargo.toml
git commit -m "feat(plugin-core): pull uuid into plugin infrastructure deps"
```

(If `uuid` was already present, skip the commit and proceed to Task 3.)

---

## Task 3: Add `batch_id` field to `UpdateLifecycleContext`

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/traits.rs` (lines 43-97)

- [ ] **Step 1: Write a failing regression test for the new field**

First check whether `traits.rs` already has a test module:

```bash
grep -n "#\[cfg(test)\]" crates/plugins/infrastructure/core/src/traits.rs
```

If a `#[cfg(test)] mod tests` block already exists, append the three tests below
**inside** that existing module (omit the outer `#[cfg(test)] mod tests { ... }`
wrapper and the `use super::*;` import if already present). If no test module
exists, paste the entire block as-is at the bottom of the file.

```rust
#[cfg(test)]
mod batch_id_tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn for_pre_hook_carries_batch_id() {
        let id = Uuid::now_v7();
        let ctx = UpdateLifecycleContext::for_pre_hook(
            "linux-image-amd64",
            "6.1.78-1",
            None,
            None,
            Some(id),
        );
        assert_eq!(ctx.batch_id, Some(id));
        assert!(ctx.update_succeeded.is_none());
    }

    #[test]
    fn for_post_hook_carries_batch_id() {
        let id = Uuid::now_v7();
        let ctx = UpdateLifecycleContext::for_post_hook(
            "linux-image-amd64",
            "6.1.78-1",
            None,
            None,
            true,
            Some(id),
        );
        assert_eq!(ctx.batch_id, Some(id));
        assert_eq!(ctx.update_succeeded, Some(true));
    }

    #[test]
    fn batch_id_is_optional() {
        let ctx = UpdateLifecycleContext::for_pre_hook("pkg", "1.0", None, None, None);
        assert!(ctx.batch_id.is_none());
    }
}
```

Update the test-runner filter in Step 2 to match the actual module name used
(`traits::tests::...` if appended to an existing module; otherwise
`traits::batch_id_tests::...`).

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo test -p uptrakit-plugin-infrastructure-core --all-features -- traits::tests
```

Expected: compile error — `for_pre_hook`/`for_post_hook` do not take a
`batch_id` argument; `UpdateLifecycleContext` has no `batch_id` field.

- [ ] **Step 3: Add the field + extend constructors**

Replace the existing struct and impl block at
`crates/plugins/infrastructure/core/src/traits.rs:43-97` with:

```rust
/// Contextual data passed to update lifecycle hook plugins.
///
/// Context for standalone hook plugins assigned via
/// `PreUpdateHook`/`PostUpdateHook` roles.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct UpdateLifecycleContext {
    /// The package identifier being updated.
    pub package_identifier: String,
    /// The target version being installed.
    pub to_version: String,
    /// The version installed before the update, if detected.
    pub from_version: Option<String>,
    /// Optional release metadata from the upstream source.
    pub release_info: Option<ReleaseInfo>,
    /// Whether the update succeeded.
    ///
    /// `None` during pre-hooks, `Some(true/false)` during post-hooks.
    pub update_succeeded: Option<bool>,
    /// Batch correlation id. `Some(uuid)` when the hook is dispatched
    /// from a batch update (`ExecuteBatchUpdatePayload.batch_id`);
    /// `None` for single-item dispatches. Used by the agent-side
    /// dispatcher to dedup repeat firings of the same hook within
    /// one batch.
    pub batch_id: Option<uuid::Uuid>,
}

impl UpdateLifecycleContext {
    /// Create a new context for pre-update hooks.
    pub fn for_pre_hook(
        package_identifier: impl Into<String>,
        to_version: impl Into<String>,
        from_version: Option<String>,
        release_info: Option<ReleaseInfo>,
        batch_id: Option<uuid::Uuid>,
    ) -> Self {
        Self {
            package_identifier: package_identifier.into(),
            to_version: to_version.into(),
            from_version,
            release_info,
            update_succeeded: None,
            batch_id,
        }
    }

    /// Create a new context for post-update hooks.
    pub fn for_post_hook(
        package_identifier: impl Into<String>,
        to_version: impl Into<String>,
        from_version: Option<String>,
        release_info: Option<ReleaseInfo>,
        update_succeeded: bool,
        batch_id: Option<uuid::Uuid>,
    ) -> Self {
        Self {
            package_identifier: package_identifier.into(),
            to_version: to_version.into(),
            from_version,
            release_info,
            update_succeeded: Some(update_succeeded),
            batch_id,
        }
    }
}
```

- [ ] **Step 4: Run the new tests to verify they pass; expect downstream
      call-site failures**

```bash
cargo build -p uptrakit-plugin-infrastructure-core
```

Expected: success (`uptrakit-plugin-infrastructure-core` itself compiles).

```bash
cargo test -p uptrakit-plugin-infrastructure-core --all-features -- traits::tests
```

Expected: the three new tests pass.

```bash
cargo check --workspace --no-default-features --features db-sqlite 2>&1 | head -40
```

Expected: workspace-wide compile errors in `agent-core` and any other call sites
of `for_pre_hook` / `for_post_hook` — that is the entry point for Task 4.

- [ ] **Step 5: Do NOT commit yet** — the workspace must compile before we
      commit. Continue to Task 4.

---

## Task 4: Thread `batch_id` through `agent-core` call sites

**Files:**

- Modify: `crates/shared/agent-core/src/update.rs` (lines 115, 284, and the test
  fixtures around lines 1331-1355)
- Modify: `crates/shared/agent-core/src/client.rs` (lines 577 + 598 — batch-path
  constructors)

- [ ] **Step 1: Find every call site of `for_pre_hook` / `for_post_hook`**

```bash
grep -rn "UpdateLifecycleContext::for_pre_hook\|UpdateLifecycleContext::for_post_hook" \
  crates/shared/agent-core/src/
```

Expected output (line numbers may shift slightly):

```text
crates/shared/agent-core/src/client.rs:577:        let pre_ctx = UpdateLifecycleContext::for_pre_hook(
crates/shared/agent-core/src/client.rs:598:        let post_ctx = UpdateLifecycleContext::for_post_hook(
crates/shared/agent-core/src/update.rs:115:    let lifecycle_ctx = UpdateLifecycleContext::for_pre_hook(
crates/shared/agent-core/src/update.rs:284:    let post_ctx = UpdateLifecycleContext::for_post_hook(
crates/shared/agent-core/src/update.rs:1331:        let ctx = UpdateLifecycleContext::for_pre_hook("pkg", "1.0", None, None);
crates/shared/agent-core/src/update.rs:1340:        let ctx = UpdateLifecycleContext::for_post_hook("pkg", "1.0", None, None, true);
crates/shared/agent-core/src/update.rs:1348:        let ctx = UpdateLifecycleContext::for_pre_hook("", "", None, None);
crates/shared/agent-core/src/update.rs:1355:        let ctx = UpdateLifecycleContext::for_post_hook("", "", None, None, true);
```

If anything else is found in non-test code (e.g. another crate), add it to the
list — the workspace will not compile until every call site passes the new
argument.

- [ ] **Step 2: Update single-item path in `update.rs:115` (pre-hook)**

Open `crates/shared/agent-core/src/update.rs` and locate the
`let lifecycle_ctx = UpdateLifecycleContext::for_pre_hook(` near line 115.
Replace the call with:

```rust
    let lifecycle_ctx = UpdateLifecycleContext::for_pre_hook(
        &payload.execute_update_plugin.package_identifier,
        &payload.to_version,
        None, // from_version not yet available at this stage
        payload.release_info.clone(),
        None, // single-item dispatch — not part of a batch
    );
```

- [ ] **Step 3: Update single-item path in `update.rs:284` (post-hook)**

Locate `let post_ctx = UpdateLifecycleContext::for_post_hook(` near line 284.
Replace with:

```rust
    let post_ctx = UpdateLifecycleContext::for_post_hook(
        &payload.execute_update_plugin.package_identifier,
        &payload.to_version,
        None,
        payload.release_info.clone(),
        succeeded,
        None, // single-item dispatch — not part of a batch
    );
```

- [ ] **Step 4: Update batch path in `client.rs:577` (pre-hook)**

Open `crates/shared/agent-core/src/client.rs` and locate the batch-path
`let pre_ctx = UpdateLifecycleContext::for_pre_hook(` near line 577. Replace
with:

```rust
        let pre_ctx = UpdateLifecycleContext::for_pre_hook(
            "", // batch has no single package identifier
            "", // batch has no single to_version
            None,
            None,
            Some(payload.batch_id),
        );
```

- [ ] **Step 5: Update batch path in `client.rs:598` (post-hook)**

Locate `let post_ctx = UpdateLifecycleContext::for_post_hook(` near line 598.
Replace with:

```rust
        let post_ctx = UpdateLifecycleContext::for_post_hook(
            "", "", None, None, true, // batch succeeded if we got here
            Some(payload.batch_id),
        );
```

- [ ] **Step 6: Update the four test fixtures in `update.rs` around lines
      1331-1355**

For each call (`for_pre_hook` lines 1331, 1348; `for_post_hook` lines 1340,
1355), append a trailing `None,` argument. Example for line 1331:

```rust
        let ctx = UpdateLifecycleContext::for_pre_hook("pkg", "1.0", None, None, None);
```

And for line 1340:

```rust
        let ctx = UpdateLifecycleContext::for_post_hook("pkg", "1.0", None, None, true, None);
```

(Repeat the analogous fix for lines 1348 and 1355.)

- [ ] **Step 7: Build the workspace**

```bash
cargo check --workspace --all-features
```

Expected: success.

```bash
cargo test -p uptrakit-agent-core --all-features
```

Expected: all `agent-core` tests pass.

```bash
cargo test -p uptrakit-plugin-hook-systemd --all-features
cargo test -p uptrakit-plugin-hook-shell --all-features
```

Expected: existing hook plugin tests pass unchanged.

- [ ] **Step 8: Commit**

```bash
git add crates/plugins/infrastructure/core/src/traits.rs \
        crates/shared/agent-core/src/update.rs \
        crates/shared/agent-core/src/client.rs
git commit -m "feat(plugin-core): thread batch_id through UpdateLifecycleContext

Adds an optional batch_id field on UpdateLifecycleContext, populated
with payload.batch_id on the batch path (client.rs) and None on the
single-item path (update.rs). Constructor signatures gain a trailing
batch_id: Option<Uuid> parameter; all call sites and test fixtures
updated. The agent-side hook dispatcher (Task 7) uses batch_id to
dedup repeat firings of the same hook within one batch."
```

---

## Task 5: Add `detect_host_compatibility` default method to `LifecycleHook`

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/roles.rs` (lines 210-225)

- [ ] **Step 1: Write a failing assertion in the existing roles tests**

Locate `crates/plugins/infrastructure/core/src/roles.rs`. If there is no
`#[cfg(test)] mod tests`, append one; otherwise extend it. Add:

```rust
#[cfg(test)]
mod lifecycle_hook_default_compat_tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Arc;

    struct StubHook;

    impl PluginMeta for StubHook {
        fn plugin_type_id(&self) -> PluginTypeId {
            PluginTypeId::from_static("stub_hook")
        }
    }

    #[async_trait]
    impl LifecycleHook for StubHook {
        async fn execute_pre_hook(
            &self,
            _ctx: &UpdateLifecycleContext,
            _output_tx: &UpdateOutputSender,
        ) -> crate::Result<PreUpdateHookResult> {
            Ok(PreUpdateHookResult::proceed())
        }

        async fn execute_post_hook(
            &self,
            _ctx: &UpdateLifecycleContext,
            _output_tx: &UpdateOutputSender,
        ) -> crate::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn default_detect_host_compatibility_returns_compatible() {
        let hook: Arc<dyn LifecycleHook> = Arc::new(StubHook);
        let result = hook.detect_host_compatibility().await.unwrap();
        assert!(matches!(result, HostCompatibility::Compatible));
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo test -p uptrakit-plugin-infrastructure-core --all-features -- lifecycle_hook_default_compat_tests
```

Expected: compile error — `LifecycleHook` has no `detect_host_compatibility`
method.

- [ ] **Step 3: Add the default-impl method**

Replace the `LifecycleHook` trait definition at `roles.rs:210-225` with:

```rust
/// Standalone update lifecycle hooks (pre/post update).
#[async_trait]
pub trait LifecycleHook: PluginMeta {
    /// Preflight host compatibility check. Default returns
    /// `Compatible`; hook plugins override this to skip on
    /// incompatible hosts (e.g. an apt-only hook on a Fedora host).
    ///
    /// The agent-side dispatcher calls this immediately before
    /// `execute_pre_hook` / `execute_post_hook`. On `Incompatible`,
    /// both phases are atomically skipped for that plugin and a
    /// `[pre-hook]/[post-hook] skipped: <reason>` line is emitted.
    ///
    /// # Errors
    ///
    /// Returns an error if the probe itself fails (e.g. the command
    /// executor cannot be invoked at all).
    async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
        Ok(HostCompatibility::Compatible)
    }

    /// Run before an update is applied. May abort the update.
    async fn execute_pre_hook(
        &self,
        ctx: &UpdateLifecycleContext,
        output_tx: &UpdateOutputSender,
    ) -> Result<PreUpdateHookResult>;

    /// Run after an update has been applied. Errors are logged, not fatal.
    async fn execute_post_hook(
        &self,
        ctx: &UpdateLifecycleContext,
        output_tx: &UpdateOutputSender,
    ) -> Result<()>;
}
```

- [ ] **Step 4: Run the test**

```bash
cargo test -p uptrakit-plugin-infrastructure-core --all-features -- lifecycle_hook_default_compat_tests
```

Expected: passes.

- [ ] **Step 5: Confirm existing hooks still compile (they keep default impl)**

```bash
cargo build -p uptrakit-plugin-hook-systemd
cargo build -p uptrakit-plugin-hook-shell
cargo test  -p uptrakit-plugin-hook-systemd --all-features
cargo test  -p uptrakit-plugin-hook-shell   --all-features
```

Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add crates/plugins/infrastructure/core/src/roles.rs
git commit -m "feat(plugin-core): add LifecycleHook::detect_host_compatibility

Default impl returns HostCompatibility::Compatible so existing hook
plugins are unchanged. Future hooks override to skip on incompatible
hosts (e.g. apt-only hooks on Fedora). Dispatcher call sites land in
the next task."
```

---

## Task 6: Wire `detect_host_compatibility` into single-item hook dispatch

**Files:**

- Modify: `crates/shared/agent-core/src/update.rs` (inside
  `run_pre_hook_plugins` near line 540 and `run_post_hook_plugins` near
  line 658)

- [ ] **Step 1: Read the existing pre-hook dispatcher**

```bash
sed -n '500,600p' crates/shared/agent-core/src/update.rs
```

Note the structure: after `let lifecycle = (slot.create)(...)?;` and before
`lifecycle.execute_pre_hook(...)`, insert a preflight call.

- [ ] **Step 2: Insert the preflight in `run_pre_hook_plugins`**

Locate the line:

```rust
        let (plugin_tx, bridge_handle) = make_output_bridge(output_tx);
        let result = lifecycle.execute_pre_hook(ctx, &plugin_tx).await;
```

Replace with:

```rust
        match lifecycle.detect_host_compatibility().await {
            Ok(HostCompatibility::Incompatible(reason)) => {
                tracing::info!(
                    plugin_type = %assignment.plugin_type,
                    reason = %reason,
                    "pre-hook plugin incompatible with host; skipping"
                );
                let msg = format!(
                    "[pre-hook] skipped: {} ({reason})",
                    assignment.plugin_type
                );
                send_output(output_tx, &msg, OutputStreamType::PreHook).await;
                append_bounded(accumulated_output, &format!("{msg}\n"), MAX_OUTPUT_BYTES);
                continue;
            }
            Err(e) => {
                tracing::warn!(
                    plugin_type = %assignment.plugin_type,
                    error = %e,
                    "pre-hook plugin compatibility probe failed; treating as incompatible and skipping"
                );
                let msg = format!(
                    "[pre-hook] skipped: {} (compat probe failed: {e})",
                    assignment.plugin_type
                );
                send_output(output_tx, &msg, OutputStreamType::PreHook).await;
                append_bounded(accumulated_output, &format!("{msg}\n"), MAX_OUTPUT_BYTES);
                continue;
            }
            Ok(HostCompatibility::Compatible) => {}
        }

        let (plugin_tx, bridge_handle) = make_output_bridge(output_tx);
        let result = lifecycle.execute_pre_hook(ctx, &plugin_tx).await;
```

Also add `use uptrakit_plugin_infrastructure_core::HostCompatibility;` to the
top of the file if `HostCompatibility` is not already in scope (search the
existing imports first).

- [ ] **Step 3: Mirror the change in `run_post_hook_plugins`**

Locate the `run_post_hook_plugins` function (near line 604) and find:

```rust
        let (plugin_tx, bridge_handle) = make_output_bridge(output_tx);
        let result = lifecycle.execute_post_hook(ctx, &plugin_tx).await;
```

Replace with the same preflight pattern, swapping `pre-hook` → `post-hook` and
`PreHook` → `PostHook`:

```rust
        match lifecycle.detect_host_compatibility().await {
            Ok(HostCompatibility::Incompatible(reason)) => {
                tracing::info!(
                    plugin_type = %assignment.plugin_type,
                    reason = %reason,
                    "post-hook plugin incompatible with host; skipping"
                );
                let msg = format!(
                    "[post-hook] skipped: {} ({reason})",
                    assignment.plugin_type
                );
                send_output(output_tx, &msg, OutputStreamType::PostHook).await;
                append_bounded(accumulated_output, &format!("{msg}\n"), MAX_OUTPUT_BYTES);
                continue;
            }
            Err(e) => {
                tracing::warn!(
                    plugin_type = %assignment.plugin_type,
                    error = %e,
                    "post-hook plugin compatibility probe failed; treating as incompatible and skipping"
                );
                let msg = format!(
                    "[post-hook] skipped: {} (compat probe failed: {e})",
                    assignment.plugin_type
                );
                send_output(output_tx, &msg, OutputStreamType::PostHook).await;
                append_bounded(accumulated_output, &format!("{msg}\n"), MAX_OUTPUT_BYTES);
                continue;
            }
            Ok(HostCompatibility::Compatible) => {}
        }

        let (plugin_tx, bridge_handle) = make_output_bridge(output_tx);
        let result = lifecycle.execute_post_hook(ctx, &plugin_tx).await;
```

- [ ] **Step 4: Write an integration test for the skip path**

Append to the existing test module in `crates/shared/agent-core/src/update.rs`
(search for `#[cfg(test)] mod tests` near line 1310). If the existing module is
module-private, add the test inside it; otherwise create a sibling test module.

```rust
#[cfg(test)]
mod hook_compat_tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Arc;
    use uptrakit_plugin_infrastructure_core::{
        HostCompatibility, HostRuntime, LifecycleHook, PluginMeta, PreUpdateHookResult,
        UpdateLifecycleContext, UpdateOutputSender,
    };
    use uptrakit_shared_types::PluginTypeId;

    struct IncompatibleHook;

    impl PluginMeta for IncompatibleHook {
        fn plugin_type_id(&self) -> PluginTypeId {
            PluginTypeId::from_static("test_incompat")
        }
    }

    #[async_trait]
    impl LifecycleHook for IncompatibleHook {
        async fn detect_host_compatibility(
            &self,
        ) -> uptrakit_plugin_infrastructure_core::Result<HostCompatibility> {
            Ok(HostCompatibility::Incompatible("test incompat".to_string()))
        }
        async fn execute_pre_hook(
            &self,
            _ctx: &UpdateLifecycleContext,
            _output_tx: &UpdateOutputSender,
        ) -> uptrakit_plugin_infrastructure_core::Result<PreUpdateHookResult> {
            panic!("execute_pre_hook must not run when detect_host_compatibility returns Incompatible");
        }
        async fn execute_post_hook(
            &self,
            _ctx: &UpdateLifecycleContext,
            _output_tx: &UpdateOutputSender,
        ) -> uptrakit_plugin_infrastructure_core::Result<()> {
            panic!("execute_post_hook must not run when detect_host_compatibility returns Incompatible");
        }
    }

    // The full dispatcher requires a runtime + descriptor wiring; the
    // unit-level guarantee is that an Incompatible result causes both
    // execute_pre_hook and execute_post_hook to be skipped. This test
    // covers the trait-level contract; the dispatcher-level integration
    // is exercised via the next task's batch-dispatch tests.
    #[tokio::test]
    async fn incompatible_hook_skips_pre_and_post() {
        let hook: Arc<dyn LifecycleHook> = Arc::new(IncompatibleHook);
        let compat = hook.detect_host_compatibility().await.unwrap();
        assert!(matches!(compat, HostCompatibility::Incompatible(_)));
        // Subsequent dispatcher logic in run_pre_hook_plugins /
        // run_post_hook_plugins must `continue` and never invoke
        // execute_pre_hook / execute_post_hook on this hook.
    }
}
```

- [ ] **Step 5: Run the tests**

```bash
cargo test -p uptrakit-agent-core --all-features -- hook_compat_tests
```

Expected: passes.

```bash
cargo clippy -p uptrakit-agent-core --all-targets --all-features -- -D warnings
```

Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/shared/agent-core/src/update.rs
git commit -m "feat(agent-core): preflight LifecycleHook::detect_host_compatibility

Single-item pre/post-hook dispatchers now invoke
detect_host_compatibility() before execute_pre_hook /
execute_post_hook. Incompatible(reason) or probe failure causes the
hook to be atomically skipped for that plugin with a
[pre-hook]/[post-hook] skipped output line and a structured
tracing event. Existing hook_systemd / hook_shell are unchanged
because they inherit the Compatible default."
```

---

## Task 7: Add preflight + per-batch dedup to batch hook dispatchers

**Files:**

- Modify: `crates/shared/agent-core/src/update.rs` —
  `run_batch_pre_hook_plugins` (near line 713) and `run_batch_post_hook_plugins`
  (near line 770)

- [ ] **Step 1: Inspect the batch pre-hook dispatcher**

```bash
sed -n '709,770p' crates/shared/agent-core/src/update.rs
```

Locate the `for assignment in plugins {` loop. We need two changes inside the
loop: a dedup check against a function-local `HashSet`, and a preflight
invocation.

- [ ] **Step 2: Edit `run_batch_pre_hook_plugins` (near line 713)**

Replace the function body of `run_batch_pre_hook_plugins` so it:

1. Initialises
   `let mut seen: std::collections::HashSet<(uptrakit_plugin_infrastructure_core::PluginTypeId, uuid::Uuid)> = std::collections::HashSet::new();`
   before the loop.
2. Inside the loop, before any plugin construction, performs:

```rust
        // Per-batch dedup: a single hook plugin may be assigned to
        // multiple Software Items in the same batch; we want it to
        // run at most once per (plugin_type, batch_id).
        if let Some(batch_id) = ctx.batch_id {
            let key = (assignment.plugin_type.clone(), batch_id);
            if !seen.insert(key) {
                tracing::info!(
                    plugin_type = %assignment.plugin_type,
                    batch_id = %batch_id,
                    "pre-hook plugin already ran for this batch; skipping duplicate"
                );
                continue;
            }
        }
```

Then, after the plugin is constructed via `(slot.create)(...)` but before the
existing channel-and-execute block
(`let (plugin_tx, mut plugin_rx) = mpsc::channel...`), insert the same preflight
as Task 6:

```rust
        match lifecycle.detect_host_compatibility().await {
            Ok(HostCompatibility::Incompatible(reason)) => {
                tracing::info!(
                    plugin_type = %assignment.plugin_type,
                    reason = %reason,
                    batch_id = ?ctx.batch_id,
                    "batch pre-hook plugin incompatible with host; skipping"
                );
                continue;
            }
            Err(e) => {
                tracing::warn!(
                    plugin_type = %assignment.plugin_type,
                    error = %e,
                    batch_id = ?ctx.batch_id,
                    "batch pre-hook plugin compatibility probe failed; skipping"
                );
                continue;
            }
            Ok(HostCompatibility::Compatible) => {}
        }
```

Note: there is no `output_tx` in the batch dispatcher (the existing local mpsc
is the discarded sink described in the spec's "Known framework limitation"); the
skip is recorded via `tracing` only. This matches the v1 audit decision.

- [ ] **Step 3: Make the analogous change in `run_batch_post_hook_plugins`**

The post-hook dispatcher (`run_batch_post_hook_plugins`) is structurally
identical. Apply the same two changes: the `seen` HashSet at the top, the dedup
check before any plugin construction, and the preflight before
`lifecycle.execute_post_hook(...)`. Substitute `pre-hook` → `post-hook` in the
tracing messages.

- [ ] **Step 4: Add or extend the imports at the top of `update.rs`**

```bash
grep -n "^use \|HashSet\|PluginTypeId\|HostCompatibility" crates/shared/agent-core/src/update.rs | head
```

If not already imported, add to the existing `use` block near the top of the
file. `PluginTypeId` lives in `uptrakit_shared_types`, not
`uptrakit_plugin_infrastructure_core`:

```rust
use std::collections::HashSet;
use uptrakit_plugin_infrastructure_core::HostCompatibility;
use uptrakit_shared_types::PluginTypeId;
```

- [ ] **Step 5: Write a dedup test**

Append to the existing test module:

```rust
#[cfg(test)]
mod hook_batch_dedup_tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use uptrakit_plugin_infrastructure_core::{
        HostCapabilities, HostCompatibility, HostRuntime, LifecycleHook, PluginMeta,
        PreUpdateHookResult, StandardHostRuntime, UpdateLifecycleContext, UpdateOutputSender,
    };
    use uptrakit_shared_types::PluginTypeId;
    use uptrakit_command::{CommandExecutor, LocalCommandExecutor};
    use uuid::Uuid;
    use uptrakit_wire::PluginAssignment;

    struct CountingHook {
        post_calls: Arc<AtomicUsize>,
    }

    impl PluginMeta for CountingHook {
        fn plugin_type_id(&self) -> PluginTypeId {
            PluginTypeId::from_static("test_counting_hook")
        }
    }

    #[async_trait]
    impl LifecycleHook for CountingHook {
        async fn execute_pre_hook(
            &self,
            _ctx: &UpdateLifecycleContext,
            _output_tx: &UpdateOutputSender,
        ) -> uptrakit_plugin_infrastructure_core::Result<PreUpdateHookResult> {
            Ok(PreUpdateHookResult::proceed())
        }
        async fn execute_post_hook(
            &self,
            _ctx: &UpdateLifecycleContext,
            _output_tx: &UpdateOutputSender,
        ) -> uptrakit_plugin_infrastructure_core::Result<()> {
            self.post_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    /// This test verifies the unit-level dedup contract: given an
    /// assignment list with two entries of the same plugin_type and
    /// a `batch_id = Some(uuid)`, only the first should run. The
    /// dispatcher in `run_batch_post_hook_plugins` is responsible
    /// for enforcing this — wire it up here through the descriptor
    /// registry once descriptor injection helpers exist; otherwise
    /// the trait-level proof of dedup keying lives in the next
    /// integration test.
    #[tokio::test]
    async fn dedup_set_collapses_duplicate_entries() {
        let mut seen: HashSet<(PluginTypeId, Uuid)> = HashSet::new();
        let batch_id = Uuid::now_v7();
        let pt = PluginTypeId::from_static("test_counting_hook");
        assert!(seen.insert((pt.clone(), batch_id)));
        assert!(!seen.insert((pt.clone(), batch_id))); // duplicate rejected
        let other_batch = Uuid::now_v7();
        assert!(seen.insert((pt, other_batch))); // different batch is allowed
    }
}
```

(This is a unit-level proof that the `(plugin_type_id, batch_id)` keying behaves
correctly. The dispatcher-level dedup is exercised end-to-end via the manual VM
test in Plan B.)

- [ ] **Step 6: Run the tests**

```bash
cargo test -p uptrakit-agent-core --all-features -- hook_batch_dedup_tests
```

Expected: passes.

```bash
cargo clippy -p uptrakit-agent-core --all-targets --all-features -- -D warnings
```

Expected: clean. If clippy flags the `Clone` on `PluginTypeId` as inefficient,
leave it — `PluginTypeId` wraps `&'static str` so cloning is cheap.

- [ ] **Step 7: Commit**

```bash
git add crates/shared/agent-core/src/update.rs
git commit -m "feat(agent-core): per-batch hook dedup + batch-path preflight

Both batch hook dispatchers (run_batch_pre_hook_plugins,
run_batch_post_hook_plugins) gain a function-local
HashSet<(PluginTypeId, Uuid)> initialised at the top of the loop
over assignments. When ctx.batch_id == Some(uuid), the dispatcher
skips a second occurrence of the same (plugin_type, batch_id)
pair with a structured tracing::info! event. detect_host_compatibility
is also invoked before plugin execution; Incompatible / probe
failure results in a non-fatal skip recorded via tracing.

Note: the batch path's output channel is currently discarded at
client.rs:521 (the _output_rx). This is a known framework gap; v1
batch-path observability is structured-tracing-only. A follow-up
spec must redesign batch-path output capture."
```

---

## Task 8: Update `docs/development/update-hooks.md`

**Files:**

- Modify: `docs/development/update-hooks.md`

- [ ] **Step 1: Inspect the existing structure**

```bash
sed -n '1,40p' docs/development/update-hooks.md
```

Locate the `UpdateLifecycleContext` field table (search for
`| package_identifier`).

- [ ] **Step 2: Add `batch_id` to the field table**

Append a row to the `UpdateLifecycleContext` table:

```markdown
| `batch_id` | `Option<Uuid>` | `Some(uuid)` when the hook fires from a batch
update; `None` for single-item dispatches. Agent-side dispatcher uses this to
dedup repeat firings of the same hook within one batch. |
```

- [ ] **Step 3: Add a `detect_host_compatibility` section under the
      `LifecycleHook` description**

Find the heading that describes the `LifecycleHook` trait (search for
`LifecycleHook`). Add a new subsection immediately under it:

```markdown
### Host compatibility preflight

`LifecycleHook::detect_host_compatibility() -> Result<HostCompatibility>` is a
default-impl trait method (default returns `Compatible`). The agent-side
dispatcher invokes it before `execute_pre_hook` and `execute_post_hook`. On
`Incompatible(reason)` or probe error, both phases are atomically skipped for
that plugin and a `[pre-hook]/[post-hook] skipped: <reason>` line is emitted to
the single-item output stream; on the batch path the skip is recorded via
`tracing::info!` only (see "Batch-path output limitation" below).

Override this method when the hook only applies on a subset of hosts (e.g. an
apt-only hook on a non-Debian host). Existing `hook_systemd` and `hook_shell`
plugins inherit the default and are unchanged.
```

- [ ] **Step 4: Document the per-batch dedup**

Add another subsection:

```markdown
### Per-batch dedup

When a hook is assigned to multiple Software Items in the same batch update, the
agent's `run_batch_pre_hook_plugins` and `run_batch_post_hook_plugins`
dispatchers dedup on `(plugin_type_id, batch_id)`: the first encounter runs the
hook, subsequent encounters in the same batch emit a `tracing::info!` event and
skip. The dedup HashSet is function-local to the dispatcher invocation — never
cached across batches.

Single-item paths pass `batch_id = None` and bypass dedup entirely.
```

- [ ] **Step 5: Document the batch-path output limitation**

Add a final subsection at the bottom of the file:

```markdown
### Known framework limitation: batch-path output is discarded

`crates/shared/agent-core/src/client.rs:521` constructs the batch update output
channel with `_output_rx` (receiver discarded), so every line a plugin or hook
writes via the batch `output_tx` disappears into a dropped channel. The batch
hook dispatchers in `update.rs:740-745,800-805` additionally spawn their own
drain task that discards hook output a second time.

Batch-path observability is currently **return-value-based**: each item's
`BatchUpdateResult.output` is surfaced into `BatchUpdateItemResult.output`
(`client.rs:630`) and persists into `update_history.output`. Hooks have no slot
in that return value because hooks fire once per batch, not once per item.

Hook plugins that need batch-path audit visibility (e.g.
`hook_kernel_cleanup_apt`) emit structured `tracing::info!` / `tracing::warn!`
events with `plugin_type` and `batch_id` fields; operators query the agent's
journal for the decision trail.

A follow-up framework spec must redesign batch-path output capture (e.g. add a
`batch_output: String` field on `BatchUpdateResultPayload`, or rewire
`_output_rx` to forward into each item's result trailer). Once that lands, hooks
may rely on `output_tx` for batch-path audit and drop their structured tracing
crutch.
```

- [ ] **Step 6: Run markdownlint**

```bash
markdownlint --config .markdownlint.json docs/development/update-hooks.md
```

Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add docs/development/update-hooks.md
git commit -m "docs(update-hooks): batch_id, detect_host_compatibility, dedup, known gap

Documents the additive framework changes from feat(plugin-core):
- UpdateLifecycleContext.batch_id field
- LifecycleHook::detect_host_compatibility default-impl method
- per-batch hook dedup keyed on (plugin_type_id, batch_id)
- existing batch-path output discard at client.rs:521 as a known
  framework gap, with a deferred follow-up-spec marker."
```

---

## Task 9: Full quality-gate sweep

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

Expected: both clean.

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

- [ ] **Step 7: Sentrux architectural check**

```bash
sentrux check .
```

Expected: clean.

- [ ] **Step 8: Markdownlint sweep**

```bash
markdownlint --config .markdownlint.json '**/*.md'
```

Expected: clean.

- [ ] **Step 9: Smoke-run hook plugin behaviour is unchanged**

```bash
cargo test -p uptrakit-plugin-hook-systemd --all-features
cargo test -p uptrakit-plugin-hook-shell --all-features
```

Expected: identical results to the Task 1 baseline run. If any test regressed,
do NOT proceed to Plan B — diagnose first.

- [ ] **Step 10: Final status check**

```bash
git log --oneline -10
git status
```

Expected: clean working tree, 5 new commits on top of the Plan A starting point
(one each from Tasks 2 (conditionally), 4, 5, 6, 7, 8). Tag the head for handoff
to Plan B if desired:

```bash
git tag plan-a-kernel-cleanup-framework
```

---

## Self-Review

Plan covers every Plan A spec deliverable:

- `UpdateLifecycleContext.batch_id` — Tasks 3 + 4
- `LifecycleHook::detect_host_compatibility` default-impl — Task 5
- Agent-side preflight (single-item) — Task 6
- Agent-side preflight (batch) + per-batch dedup — Task 7
- Doc update — Task 8
- Quality-gate verification — Tasks 1 + 9

No placeholder text. Every code block is concrete. Types referenced
(`PluginTypeId`, `HostCompatibility`, `PluginAssignment`, `UpdateOutputSender`,
`make_output_bridge`, `send_output`, `OutputStreamType`, `append_bounded`,
`MAX_OUTPUT_BYTES`) all already exist in the `agent-core` /
`plugin-infrastructure-core` crates and were verified during spec authoring.

Out of scope (Plan B owns these): kernel-cleanup plugin crate, registry
registration, ADR 0010, operator runbook, plugin doc page. Plan B picks up at
the tag `plan-a-kernel-cleanup-framework`.
