# Instance-Scoped Plugins — Plan A: Backend Foundations

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the descriptor primitives, data model, and catalog/snapshot wiring needed to support Instance-Scoped Plugins, leaving every existing
plugin's behavior unchanged. Dashboard-icons stays `PluginScope::Tenant` until Plan B flips it.

**Architecture:** Add a `PluginScope` enum + `InstanceConfigOps` slot to `PluginDescriptor`; extend the `declare_plugin!` macro with two optional
arms; introduce a new `instance_plugin_setting` SeaORM table; add an `InstancePluginStates` typed wrapper passed as a third argument to
`PluginCatalog::new`; load the snapshot at controller boot via a new `web-api-queries` module; expose the snapshot through `AppState` as
`Arc<ArcSwap<InstancePluginSnapshot>>` for the visibility predicate.

**Tech Stack:** Rust workspace (cargo, workspace lints `warnings = "deny"`), SeaORM, Axum, `arc_swap`, `parking_lot`, `tracing`, `rootcause`,
`utoipa`. Source of truth: spec `docs/superpowers/specs/2026-05-10-instance-scoped-plugins-design.md`. Snapshot: `.superpowers/standards-snapshot.md`.

**Quality gates (run as final task):** `cargo fmt --all`, `cargo check --no-default-features --features db-sqlite`, `cargo check --all-features`,
`cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings`, `cargo clippy --all-targets --all-features -- -D warnings`,
`cargo test --all-features`, `cargo deny check`, `markdownlint --config .markdownlint.json '**/*.md'`.

---

## File structure

| File                                                                                | Status             | Responsibility                                                                                  |
| ----------------------------------------------------------------------------------- | ------------------ | ----------------------------------------------------------------------------------------------- |
| `crates/plugins/infrastructure/core/src/descriptor.rs`                              | modify             | Add `PluginScope`, `InstanceConfigOps`, descriptor field extension                              |
| `crates/plugins/infrastructure/core/src/lib.rs`                                     | modify             | Re-export `PluginScope`, `InstanceConfigOps`                                                    |
| `crates/plugins/infrastructure/core/src/macros.rs`                                  | modify             | Extend `declare_plugin!` grammar                                                                |
| `crates/plugins/infrastructure/core/src/catalog.rs`                                 | modify             | Add `InstancePluginStates`, change `PluginCatalog::new`, gate construction, validate invariants |
| `crates/plugins/infrastructure/core/src/plugin_ops.rs`                              | modify             | Add `instance_enabled()` method to `PluginMetadataOps`                                          |
| `crates/plugins/infrastructure/core/src/error.rs`                                   | modify (if needed) | Add `InvalidDescriptor` variant if not expressible via existing variants                        |
| `crates/plugins/infrastructure/registry/src/test_support.rs`                        | modify             | Update test fixtures to include new descriptor fields                                           |
| `crates/shared/db/src/entity/instance_plugin_setting.rs`                            | create             | SeaORM entity for new table                                                                     |
| `crates/shared/db/src/entity/mod.rs`                                                | modify             | `pub mod instance_plugin_setting;`                                                              |
| `crates/shared/db/src/migration/mYYYYMMDD_NNNNNN_create_instance_plugin_setting.rs` | create             | Table migration                                                                                 |
| `crates/shared/db/src/migration/mod.rs`                                             | modify             | `mod` decl + `Migrator::migrations()` entry                                                     |
| `crates/ui/web-api-queries/src/instance_plugin_settings.rs`                         | create             | Query module + `InstancePluginSnapshot::load_at_boot`                                           |
| `crates/ui/web-api-queries/src/lib.rs`                                              | modify             | `pub mod instance_plugin_settings;`                                                             |
| `crates/ui/web-api/src/visibility.rs`                                               | create             | Single visibility predicate helper                                                              |
| `crates/ui/web-api/src/lib.rs`                                                      | modify             | `pub mod visibility;`                                                                           |
| `crates/ui/web-api/src/app_state.rs`                                                | modify             | Add `Arc<ArcSwap<InstancePluginSnapshot>>` to `AppState` + builder                              |
| Controller boot path (e.g. `crates/core/controller/src/main.rs` or builder)         | modify             | Load snapshot before catalog build; pass into `PluginCatalog::new`                              |

---

## Task 1: Branch and snapshot prep

**Files:** none

- [ ] **Step 1: Confirm working tree clean and on main**

```bash
git status
git log -1 --oneline
```

Expected: clean tree, HEAD at `7647253cd docs(plugins): spec instance-scoped plugins flavor`.

- [ ] **Step 2: Create feature branch**

```bash
git checkout -b feat/instance-scoped-plugins-foundations
```

- [ ] **Step 3: Re-read the spec and snapshot**

```bash
sed -n '1,200p' docs/superpowers/specs/2026-05-10-instance-scoped-plugins-design.md
cat .superpowers/standards-snapshot.md
```

No commit on this task.

---

## Task 2: Add `PluginScope` enum

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/descriptor.rs`
- Modify: `crates/plugins/infrastructure/core/src/lib.rs`

Snapshot rules: `Extensible public enums #[non_exhaustive]`, `Use #[expect(...)] not #[allow(...)]`.

- [ ] **Step 1: Add the `PluginScope` enum to `descriptor.rs`**

Insert above the existing `PluginFamily` definition (around line 21):

```rust
/// Who manages a plugin's enable state and instance-wide configuration.
///
/// Default is [`Self::Tenant`] for every existing plugin. Plugins promoted to
/// [`Self::Instance`] are managed exclusively by Operators with the
/// `ManageGlobalSettings` permission, and are invisible to tenant Operators
/// when disabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PluginScope {
    /// Default — managed per tenant via plugin_configs / plugin_type_settings.
    Tenant,
    /// Managed by instance owners (`ManageGlobalSettings`); when disabled,
    /// tenant Operators see no evidence the plugin exists.
    Instance,
}

impl Default for PluginScope {
    fn default() -> Self {
        Self::Tenant
    }
}

impl std::fmt::Display for PluginScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tenant => write!(f, "tenant"),
            Self::Instance => write!(f, "instance"),
        }
    }
}
```

- [ ] **Step 2: Add unit tests at the bottom of `descriptor.rs`**

Append to the existing `#[cfg(test)] mod` block (or create one if absent):

```rust
#[test]
fn plugin_scope_default_is_tenant() {
    assert_eq!(PluginScope::default(), PluginScope::Tenant);
}

#[test]
fn plugin_scope_display_lowercase() {
    assert_eq!(PluginScope::Tenant.to_string(), "tenant");
    assert_eq!(PluginScope::Instance.to_string(), "instance");
}
```

- [ ] **Step 3: Re-export from `lib.rs`**

Find the existing re-export line that ends `... PluginDescriptor, PluginFamily, RoleCreators,` (around line 109) and add `PluginScope` to it. Keep
alphabetical ordering within the group.

- [ ] **Step 4: Build and test**

```bash
cargo test -p uptrakit-plugin-infrastructure-core descriptor::tests::plugin_scope -- --nocapture
```

Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/plugins/infrastructure/core/src/descriptor.rs \
        crates/plugins/infrastructure/core/src/lib.rs
git commit -m "feat(plugins): add PluginScope enum

Tenant (default) or Instance. Wire-safe-enum convention not needed
(build-time descriptor only, no Other(String) catch-all). Marked
#[non_exhaustive] per project rule for extensible enums."
```

---

## Task 3: Add `InstanceConfigOps` slot

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/descriptor.rs`
- Modify: `crates/plugins/infrastructure/core/src/lib.rs`

Snapshot rules: `Extensible public structs #[non_exhaustive]`.

- [ ] **Step 1: Add `InstanceConfigOps` to `descriptor.rs`**

Insert immediately below `TypeSettingsOps` (around line 73):

```rust
/// Operations for the instance-wide configuration blob owned by an
/// Instance-Scoped Plugin (see [`PluginScope::Instance`]). Optional —
/// instance-scoped plugins may have no configurable knobs beyond the
/// enable toggle.
///
/// Marked `#[non_exhaustive]` because plugin crates construct values of this
/// struct as `static` literals; future field additions must not be breaking.
#[non_exhaustive]
pub struct InstanceConfigOps {
    pub form_schema: fn() -> Vec<FormFieldDescriptor>,
    pub sample: fn() -> serde_json::Value,
    pub validate: fn(&serde_json::Value) -> Result<(), PluginConfigValidationError>,
}
```

- [ ] **Step 2: Re-export from `lib.rs`**

Add `InstanceConfigOps` next to existing `TypeSettingsOps` re-export.

- [ ] **Step 3: Build**

```bash
cargo check -p uptrakit-plugin-infrastructure-core --all-features
```

Expected: success (struct unused for now).

- [ ] **Step 4: Commit**

```bash
git add crates/plugins/infrastructure/core/src/descriptor.rs \
        crates/plugins/infrastructure/core/src/lib.rs
git commit -m "feat(plugins): add InstanceConfigOps descriptor slot

Per-plugin static slot for instance-wide config form schema, sample,
and validate fn. #[non_exhaustive] for forward-compat on future fields."
```

---

## Task 4: Atomically extend `PluginDescriptor` + `declare_plugin!` macro + patch all hand-written literals

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/descriptor.rs` (add fields)
- Modify: `crates/plugins/infrastructure/core/src/macros.rs` (helper macros + grammar + DESCRIPTOR literal)
- Modify: `crates/plugins/infrastructure/core/src/catalog.rs` (test fixtures around lines 600–940)
- Modify: `crates/plugins/infrastructure/registry/src/test_support.rs` (helper fixtures)
- Modify: any other workspace file that constructs `PluginDescriptor { ... }` directly (workspace-wide grep — see Step 5)

Snapshot rules: `warnings = "deny"` makes missing fields hard build errors. **Critical ordering invariant:** every step below changes the working tree
but only the final commit is checked by the build. The macro must be extended to emit the new fields **at the same time** the struct gains them and
all hand-written literals are patched — otherwise every `declare_plugin!` invocation would compile-fail. This task lands as a **single atomic commit**
to avoid leaving the workspace broken between commits.

- [ ] **Step 1: Add helper macros mirroring `__default_hr_value!`**

In `crates/plugins/infrastructure/core/src/macros.rs`, alongside the existing `#[doc(hidden)] #[macro_export] macro_rules! __default_hr_value`:

```rust
#[doc(hidden)]
#[macro_export]
macro_rules! __scope_value {
    () => { $crate::PluginScope::Tenant };
    ($s:expr) => { $s };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __instance_config_value {
    () => { None };
    ($v:expr) => { Some($v) };
}
```

These don't affect anything yet — pure macro definitions, no expansion sites yet.

- [ ] **Step 2: Add two optional arms to the `declare_plugin!` grammar (around line 39, near `type_settings: $ts_marker`)**

```rust
            $( type_settings: $ts_marker:tt, )?
            $( scope: $scope:expr, )?                              // NEW
            $( instance_config: $instance_cfg:expr, )?             // NEW
            roles: [ $( $role:ident $( ... )? ),* $(,)? ]
```

(Position between `type_settings` and `roles:` to keep the grammar prefix-then-comma-tail invariant.)

- [ ] **Step 3: Extend the `pub static DESCRIPTOR: $crate::PluginDescriptor = $crate::PluginDescriptor { ... };` literal in the macro (around
      line 176)**

Inside the existing static struct literal, add two new fields adjacent to `capabilities:`:

```rust
        pub static DESCRIPTOR: $crate::PluginDescriptor = $crate::PluginDescriptor {
            type_id: $type_id,
            display_name: $display_name,
            family: $family,
            config_model: $config_model,
            capabilities: $crate::__compute_capabilities!( ... ),
            scope: $crate::__scope_value!( $( $scope )? ),                              // NEW
            instance_config: $crate::__instance_config_value!( $( $instance_cfg )? ),   // NEW
            // ... remaining existing fields ...
        };
```

(Confirm by inspecting `macros.rs` line 176 — this is the same struct literal that already invokes `__compute_capabilities!` and `__default_hr_value!`
for default values.)

- [ ] **Step 4: Add the two new fields to `PluginDescriptor` struct in `descriptor.rs` (around line 459)**

```rust
    // ── Identity (every plugin) ──
    pub type_id: &'static str,
    pub display_name: &'static str,
    pub family: PluginFamily,
    pub config_model: ConfigModel,
    pub capabilities: &'static [PluginCapability],
    pub scope: PluginScope,                                  // NEW
    pub instance_config: Option<&'static InstanceConfigOps>, // NEW
```

After this step the working tree has the macro emitting `scope`/`instance_config` AND the struct accepting them. Macro-generated descriptors compile.
Hand-written literals do NOT yet compile — Step 5 fixes that.

- [ ] **Step 5: Workspace-wide grep for hand-written `PluginDescriptor` literals and patch every one**

```bash
grep -rn "PluginDescriptor {" crates/ --include="*.rs"
```

Expected hits include (but may not be limited to) `crates/plugins/infrastructure/core/src/catalog.rs` (test fixtures around lines 600–940) and
`crates/plugins/infrastructure/registry/src/test_support.rs`. Patch every hit by adding the two new fields adjacent to `capabilities`:

```rust
        capabilities: &[],
        scope: PluginScope::Tenant,
        instance_config: None,
```

The grep is workspace-wide because plugin crates may carry their own integration-test fixtures that hand-build `PluginDescriptor` directly (rather
than via `declare_plugin!`). All hits must be patched in this same task.

- [ ] **Step 6: Build the workspace + run touched-crate tests**

```bash
cargo check --no-default-features --features db-sqlite 2>&1 | tail -20
cargo check --all-features 2>&1 | tail -20
cargo test -p uptrakit-plugin-infrastructure-core --all-features
cargo test -p uptrakit-plugin-infrastructure-registry --all-features
```

Expected: zero errors, all tests green. Every existing `declare_plugin!` invocation expanded with `scope = Tenant`, `instance_config = None`.

- [ ] **Step 7: Add a macro smoke test**

In the same crate, create `crates/plugins/infrastructure/core/tests/declare_plugin_scope.rs`:

```rust
//! Smoke test: declare_plugin! accepts `scope` and `instance_config` arms.
use uptrakit_plugin_infrastructure_core::{
    ConfigModel, InstanceConfigOps, PluginConfigValidationError, PluginFamily, PluginScope,
    declare_plugin,
};

struct DummyPlugin;
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct DummyConfig {}

fn dummy_form() -> Vec<uptrakit_plugin_infrastructure_core::FormFieldDescriptor> {
    Vec::new()
}
fn dummy_sample() -> serde_json::Value {
    serde_json::json!({})
}
fn dummy_validate(_v: &serde_json::Value) -> Result<(), PluginConfigValidationError> {
    Ok(())
}

static OPS: InstanceConfigOps = InstanceConfigOps {
    form_schema: dummy_form,
    sample: dummy_sample,
    validate: dummy_validate,
};

declare_plugin!(DummyPlugin, DummyConfig, "test_dummy_instance", {
    display_name: "Dummy Instance Plugin",
    family: PluginFamily::Enhancement,
    config_model: ConfigModel::None,
    scope: PluginScope::Instance,
    instance_config: &OPS,
    roles: [],
});

#[test]
fn descriptor_uses_instance_scope() {
    assert_eq!(DESCRIPTOR.scope, PluginScope::Instance);
    assert!(DESCRIPTOR.instance_config.is_some());
}
```

- [ ] **Step 8: Run the smoke test**

```bash
cargo test -p uptrakit-plugin-infrastructure-core --test declare_plugin_scope
```

Expected: 1 passed.

- [ ] **Step 9: Atomic commit**

All four files (`descriptor.rs`, `macros.rs`, `catalog.rs`, `test_support.rs`) plus any other workspace-wide hits from Step 5 plus the new test file
must land in a single commit so no intermediate commit leaves the workspace broken:

```bash
git add crates/plugins/infrastructure/core/src/descriptor.rs \
        crates/plugins/infrastructure/core/src/macros.rs \
        crates/plugins/infrastructure/core/src/catalog.rs \
        crates/plugins/infrastructure/registry/src/test_support.rs \
        crates/plugins/infrastructure/core/tests/declare_plugin_scope.rs \
        # any other paths surfaced by Step 5's workspace grep
git commit -m "feat(plugins): scope + instance_config on PluginDescriptor

Atomically: PluginScope helper macros, declare_plugin! grammar arms,
DESCRIPTOR static literal field emission, struct field addition, hand-
written literal patches across the workspace, smoke test. Defaults
preserve every existing plugin's behavior."
```

---

## Task 5: (merged into Task 4 above; intentionally skipped to keep subsequent task numbers stable)

---

## Task 6: Add catalog-build invariant validation

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/error.rs` (if a new variant is needed; `UnsupportedOperation` may suffice)
- Modify: `crates/plugins/infrastructure/core/src/catalog.rs`

Snapshot rules: `Wrap errors in rootcause::Report; report!() / bail!()`.

- [ ] **Step 1: Write a failing test in `catalog.rs` test module**

Add inside the existing `#[cfg(test)] mod tests` block (or a new one). Use a synthetic descriptor literal:

```rust
#[test]
fn tenant_scope_with_instance_config_fails_catalog_build() {
    use crate::{
        ConfigModel, InstanceConfigOps, PluginConfigValidationError, PluginFamily, PluginScope,
    };

    fn form() -> Vec<crate::FormFieldDescriptor> { Vec::new() }
    fn sample() -> serde_json::Value { serde_json::json!({}) }
    fn validate(_v: &serde_json::Value) -> Result<(), PluginConfigValidationError> { Ok(()) }
    static OPS: InstanceConfigOps = InstanceConfigOps { form_schema: form, sample, validate };

    static BAD: PluginDescriptor = PluginDescriptor {
        type_id: "bad_test",
        display_name: "Bad",
        family: PluginFamily::Enhancement,
        config_model: ConfigModel::None,
        capabilities: &[],
        scope: PluginScope::Tenant,
        instance_config: Some(&OPS),
        // ... fill remaining required fields with defaults; copy from another fixture above ...
    };

    let result = PluginCatalog::new(
        vec![&BAD],
        CatalogConfig::default(),
        InstancePluginStates::all_disabled(),
    );
    let err = result.expect_err("expected build failure");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("scope=Tenant") && msg.contains("instance_config"),
        "error message should name the invariant; got: {msg}"
    );
}
```

(Defer until `InstancePluginStates::all_disabled()` is added in Task 8 — for the test to compile, place the test addition after Task 8's commit, OR
temporarily comment the test until Task 8 lands. Recommended: write the test now, defer running it; Task 8 finalizes the signature.)

- [ ] **Step 2: Add the invariant check inside `PluginCatalog::new`**

In `crates/plugins/infrastructure/core/src/catalog.rs::PluginCatalog::new` (currently at line 79), add at the top of the method (after argument
validation but before any role construction loop):

```rust
        for desc in &descriptors {
            if desc.scope == PluginScope::Tenant && desc.instance_config.is_some() {
                return Err(rootcause::report!(PluginError::UnsupportedOperation(format!(
                    "plugin '{}' has scope=Tenant but declares instance_config; instance_config is \
                     only valid for scope=Instance",
                    desc.type_id
                ))));
            }
        }
```

- [ ] **Step 3: Run check**

```bash
cargo check -p uptrakit-plugin-infrastructure-core --all-features
```

Defer test execution to Task 8.

- [ ] **Step 4: Commit**

```bash
git add crates/plugins/infrastructure/core/src/catalog.rs
git commit -m "feat(plugins): validate scope/instance_config invariant at catalog build

scope=Tenant + instance_config=Some is rejected with PluginError::Unsupported-
Operation. Failure aborts controller boot via rootcause::Report."
```

---

## Task 7: Add `InstancePluginStates` typed wrapper

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/catalog.rs`
- Modify: `crates/plugins/infrastructure/core/src/lib.rs`

- [ ] **Step 1: Add the wrapper near the top of `catalog.rs`**

```rust
use std::collections::BTreeMap;

/// Per-plugin enable state for `PluginScope::Instance` plugins, snapshotted at
/// controller boot from the `instance_plugin_setting` table. Tenant-scoped
/// plugins are not represented here — they are always considered "instance-
/// enabled" by the catalog.
#[derive(Default, Debug, Clone)]
pub struct InstancePluginStates(BTreeMap<&'static str, bool>);

impl InstancePluginStates {
    /// Build from an iterator of `(type_id, enabled)` pairs.
    ///
    /// `type_id` keys must be `&'static str` because plugin descriptors carry
    /// `&'static str` type ids. The controller boot path may need to leak the
    /// row-string to `'static` (e.g. via `Box::leak`) — but in practice the
    /// caller iterates known descriptors and picks rows that match, so the
    /// `&'static` keys come directly from descriptors.
    pub fn from_pairs<I>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (&'static str, bool)>,
    {
        Self(pairs.into_iter().collect())
    }

    /// Returns `true` if the plugin's row says enabled. Returns `false` for
    /// any plugin not present in the snapshot (no row ⇒ disabled).
    pub fn enabled(&self, type_id: &str) -> bool {
        self.0.get(type_id).copied().unwrap_or(false)
    }

    /// Test/default constructor — every instance-scoped plugin disabled.
    pub fn all_disabled() -> Self {
        Self::default()
    }
}
```

- [ ] **Step 2: Re-export from `lib.rs`**

Add `InstancePluginStates` to the existing re-export group containing `PluginCatalog`.

- [ ] **Step 3: Build**

```bash
cargo check -p uptrakit-plugin-infrastructure-core --all-features
```

- [ ] **Step 4: Commit**

```bash
git add crates/plugins/infrastructure/core/src/catalog.rs \
        crates/plugins/infrastructure/core/src/lib.rs
git commit -m "feat(plugins): add InstancePluginStates typed wrapper

Snapshotted boot state for instance-scoped plugins. BTreeMap-backed,
keyed by &'static str type ids."
```

---

## Task 8: Extend `PluginCatalog::new` signature and gate construction

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/catalog.rs`
- Modify: `crates/plugins/infrastructure/registry/src/registry.rs` (call site)
- Modify: every other call site of `PluginCatalog::new` (find via grep)

Snapshot rules: `tracing::error!()/warn!()/info!() with structured fields`.

- [ ] **Step 1: Locate every call site**

```bash
grep -rn "PluginCatalog::new" crates/ --include="*.rs"
```

Expected: at least the registry crate's `register_all` function and any test fixtures.

- [ ] **Step 2: Add `instance_states: InstancePluginStates` parameter**

Change the signature at `catalog.rs:79`:

```rust
    pub fn new(
        descriptors: Vec<&'static PluginDescriptor>,
        config: CatalogConfig,
        instance_states: InstancePluginStates, // NEW
    ) -> Result<Self> {
```

Store it on `PluginCatalog`:

```rust
pub struct PluginCatalog {
    descriptors: Vec<&'static PluginDescriptor>,
    transports: HashMap<&'static str, Arc<dyn NotificationTransport>>,
    lifecycle_plugins: Vec<Arc<dyn SoftwareItemLifecycle>>,
    // ... existing fields ...
    instance_states: InstancePluginStates,    // NEW
}
```

Initialize the new field in the `Ok(Self { ... })` block at the end of `new`.

- [ ] **Step 3: Add gating to the per-descriptor loop**

Inside the `for desc in &descriptors` loop in `new` (around line 97), at the very top of the loop body — BEFORE the provider-availability check,
BEFORE every `if let Some(create) = desc.roles.<slot>` block — insert:

```rust
            // Instance-scoped plugins skip every singleton construction when
            // disabled. Their descriptor is still recorded above (so the
            // visibility predicate can show them to instance owners), but no
            // role factory is invoked.
            if desc.scope == PluginScope::Instance && !instance_states.enabled(desc.type_id) {
                tracing::info!(
                    plugin = desc.type_id,
                    scope = "instance",
                    enabled = false,
                    "skipping instance-scoped plugin construction (disabled at boot)",
                );
                continue;
            }
```

- [ ] **Step 4: Patch every existing call site**

Update each caller to pass `InstancePluginStates::all_disabled()` (since no instance-scoped plugins exist yet — will be the safe default until Plan B
flips dashboard-icons). Example for the registry:

```rust
let catalog = PluginCatalog::new(descriptors, config, InstancePluginStates::all_disabled())?;
```

- [ ] **Step 5: Run cargo check across the workspace**

```bash
cargo check --no-default-features --features db-sqlite 2>&1 | tail -20
cargo check --all-features 2>&1 | tail -20
```

Expected: zero errors.

- [ ] **Step 6: Run the deferred invariant test from Task 6**

```bash
cargo test -p uptrakit-plugin-infrastructure-core catalog -- tenant_scope_with_instance_config_fails_catalog_build
```

Expected: PASS.

- [ ] **Step 7: Add gating tests**

Append to the same test module:

```rust
#[test]
fn instance_disabled_skips_singleton_construction() {
    // Use the existing recording_lifecycle test fixture descriptor pattern
    // (see catalog.rs:810). Build a PluginDescriptor literal where:
    //   scope: PluginScope::Instance,
    //   instance_config: None,
    //   roles.software_item_lifecycle: Some(create_recording_lifecycle),
    // then call PluginCatalog::new with InstancePluginStates::all_disabled()
    // and assert software_item_lifecycle_plugins() is empty.
    // (Code body to follow the local fixture pattern at lines ~810-820.)
}

#[test]
fn instance_enabled_constructs_singleton() {
    // Same descriptor as above; call PluginCatalog::new with
    // InstancePluginStates::from_pairs([("test_instance_plugin", true)])
    // and assert software_item_lifecycle_plugins() has length 1.
}
```

(Implementer: copy the exact `create_recording_lifecycle` and surrounding fixture wiring from `catalog.rs:810-820` to fill in the test bodies — no
shortcuts.)

- [ ] **Step 8: Run gating tests**

```bash
cargo test -p uptrakit-plugin-infrastructure-core catalog -- instance_disabled_skips instance_enabled_constructs
```

Expected: 2 passed.

- [ ] **Step 9: Commit**

```bash
git add crates/plugins/infrastructure/core/src/catalog.rs \
        crates/plugins/infrastructure/registry/src/registry.rs \
        # any other call sites you patched
git commit -m "feat(plugins): gate instance-scoped plugin construction by snapshot

PluginCatalog::new takes an InstancePluginStates third arg. Instance-
scoped plugins with enabled=false skip all singleton role construction
at boot but stay in the descriptor list (visibility predicate uses it).
Logs structured tracing::info! on skip."
```

---

## Task 9: Add `instance_enabled()` to `PluginMetadataOps`

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/plugin_ops.rs`
- Modify: `crates/plugins/infrastructure/core/src/catalog.rs` (the impl block for `PluginMetadataOps`)
- Modify: `crates/plugins/infrastructure/registry/src/test_support.rs` (any mock impls)

- [ ] **Step 1: Add the method to `PluginMetadataOps` trait (line 83)**

```rust
pub trait PluginMetadataOps: Send + Sync + 'static {
    // ... existing methods ...

    /// Returns `true` if the plugin is "instance-enabled" at the catalog
    /// snapshot taken at controller boot.
    ///
    /// Semantics:
    /// - For `scope == Tenant` plugins: always `true` (no instance-level kill switch exists).
    /// - For `scope == Instance` plugins: the snapshot value loaded at boot.
    ///
    /// This reflects the *running* catalog state, not the live DB row.
    /// Callers that need the live DB value must query
    /// `instance_plugin_setting` directly.
    fn instance_enabled(&self, id: &PluginTypeId) -> bool;
}
```

- [ ] **Step 2: Implement in `PluginCatalog`**

In `catalog.rs`, find the existing `impl PluginMetadataOps for PluginCatalog` block. Add:

```rust
    fn instance_enabled(&self, id: &PluginTypeId) -> bool {
        match self.descriptor_for(id) {
            Some(desc) if desc.scope == PluginScope::Instance => {
                self.instance_states.enabled(id.as_str())
            }
            // Unknown plugin OR Tenant-scoped: tenant-scoped plugins are
            // semantically always "instance-enabled" (no kill switch). For
            // unknown plugins, return false so callers don't accidentally
            // expose unregistered ids.
            Some(_) => true,
            None => false,
        }
    }
```

- [ ] **Step 3: Patch test_support mocks**

In `crates/plugins/infrastructure/registry/src/test_support.rs`, locate any `impl PluginMetadataOps for ...` mock and add:

```rust
    fn instance_enabled(&self, _id: &PluginTypeId) -> bool { true }
```

- [ ] **Step 4: Build + test**

```bash
cargo check --all-features
cargo test -p uptrakit-plugin-infrastructure-core --all-features
```

- [ ] **Step 5: Commit**

```bash
git add crates/plugins/infrastructure/core/src/plugin_ops.rs \
        crates/plugins/infrastructure/core/src/catalog.rs \
        crates/plugins/infrastructure/registry/src/test_support.rs
git commit -m "feat(plugins): expose instance_enabled() on PluginMetadataOps

Reflects the catalog snapshot from controller boot. Tenant-scoped
plugins always return true; Instance-scoped reflect snapshot value;
unknown ids return false."
```

---

## Task 10: Create `instance_plugin_setting` SeaORM entity

**Files:**

- Create: `crates/shared/db/src/entity/instance_plugin_setting.rs`
- Modify: `crates/shared/db/src/entity/mod.rs`

Snapshot rules: SeaORM `column_type = "Json"` on `serde_json::Value` columns; no `Eq` derive.

- [ ] **Step 1: Create the entity file**

```rust
//! `instance_plugin_setting` — per-plugin enable state and instance-wide
//! configuration for Instance-Scoped Plugins. One row per plugin_type_id.
//! Row absence ⇒ plugin defaults to disabled with empty config.
use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "instance_plugin_setting")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub plugin_type_id: String,
    pub enabled: bool,
    #[sea_orm(column_type = "Json")]
    pub config: serde_json::Value,
    pub updated_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
```

- [ ] **Step 2: Register in `entity/mod.rs`**

Add `pub mod instance_plugin_setting;` in alphabetical position (between `host_*` and `notification_*`).

- [ ] **Step 3: Build**

```bash
cargo check -p uptrakit-shared-db --all-features
```

- [ ] **Step 4: Commit**

```bash
git add crates/shared/db/src/entity/instance_plugin_setting.rs \
        crates/shared/db/src/entity/mod.rs
git commit -m "feat(db): add instance_plugin_setting entity

SeaORM Model: plugin_type_id PK, enabled, config (JSON), updated_at.
No Eq derive (serde_json::Value is not Eq); no FK (plugin catalog is
static, mirrors plugin_type_setting)."
```

---

## Task 11: Create migration for `instance_plugin_setting`

**Files:**

- Create: `crates/shared/db/src/migration/m20260510_000001_create_instance_plugin_setting.rs` (use today's date — 2026-05-10 — and verify with
  `git log --since='1 month ago' --pretty=format:'%h %s' -- crates/shared/db/src/migration/` that no later prefix already lives on `main`)
- Modify: `crates/shared/db/src/migration/mod.rs`

- [ ] **Step 1: Confirm the date prefix is correct**

```bash
ls crates/shared/db/src/migration/m*.rs | tail -3
```

Latest existing prefix on `main` is `m20260430_000003`. The new file's prefix must be ≥ that. Use today's date unless the working branch has unmerged
migrations adding a later one.

- [ ] **Step 2: Write the migration file**

```rust
use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260510_000001_create_instance_plugin_setting"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(InstancePluginSetting::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(InstancePluginSetting::PluginTypeId)
                            .text()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(InstancePluginSetting::Enabled)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(InstancePluginSetting::Config)
                            .json()
                            .not_null()
                            .default("{}"),
                    )
                    .col(
                        ColumnDef::new(InstancePluginSetting::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(InstancePluginSetting::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum InstancePluginSetting {
    Table,
    PluginTypeId,
    Enabled,
    Config,
    UpdatedAt,
}
```

- [ ] **Step 3: Register in `migration/mod.rs`**

Two edits:

1. Append `mod m20260510_000001_create_instance_plugin_setting;` to the existing `mod` declarations.
2. Append `Box::new(m20260510_000001_create_instance_plugin_setting::Migration)` to the `Migrator::migrations()` `vec!`. Match the existing
   alphabetical/chronological ordering already used in the file.

- [ ] **Step 4: Run migration test**

```bash
cargo test -p uptrakit-shared-db --all-features -- --nocapture migration
```

Expected: every migration applies cleanly, including the new one.

- [ ] **Step 5: Run a smoke test creating a row in tests**

Add a quick integration test (or extend an existing migration smoke test) that performs an insert + select on the new table. Locate existing pattern:

```bash
grep -rn "insert.*plugin_type_setting\|plugin_type_setting::Entity" crates/ui/web-api/src/integration_tests/ | head
```

Pattern these tests after `plugin_type_settings` integration test setups.

- [ ] **Step 6: Commit**

```bash
git add crates/shared/db/src/migration/m20260510_000001_create_instance_plugin_setting.rs \
        crates/shared/db/src/migration/mod.rs
git commit -m "feat(db): migration for instance_plugin_setting table

Both registration sites (mod decl + Migrator::migrations vec) updated.
No seed row — runtime defaults to disabled when row absent."
```

---

## Task 12: Create `instance_plugin_settings` query module

**Files:**

- Create: `crates/ui/web-api-queries/src/instance_plugin_settings.rs`
- Modify: `crates/ui/web-api-queries/src/lib.rs`

Snapshot rules: `BEGIN IMMEDIATE for read-then-write SQLite transactions`; `parking_lot` not needed here (pure DB queries); `rootcause::Report` for
errors.

- [ ] **Step 1: Create the query module**

```rust
//! Queries for `instance_plugin_setting` — Instance-Scoped Plugin enable
//! state and instance-wide configuration. See spec
//! `docs/superpowers/specs/2026-05-10-instance-scoped-plugins-design.md`.
use std::collections::HashMap;

use rootcause::Report;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter,
    Set, SqliteTransactionMode, TransactionOptions, TransactionTrait,
};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::instance_plugin_setting::{
    self, ActiveModel, Column, Entity, Model,
};

use crate::error::QueryError;

pub type Result<T> = std::result::Result<T, Report<QueryError>>;

/// Snapshot of every row in `instance_plugin_setting`, loaded once at
/// controller boot and shared via `Arc<ArcSwap<...>>` in `AppState`.
#[derive(Default, Debug, Clone)]
pub struct InstancePluginSnapshot {
    rows: HashMap<String, InstancePluginRow>,
}

#[derive(Debug, Clone)]
pub struct InstancePluginRow {
    pub enabled: bool,
    pub config: serde_json::Value,
    pub updated_at: OffsetDateTime,
}

impl InstancePluginSnapshot {
    pub fn empty() -> Self { Self::default() }

    pub fn enabled(&self, plugin_type_id: &str) -> bool {
        self.rows.get(plugin_type_id).map(|r| r.enabled).unwrap_or(false)
    }

    pub fn get(&self, plugin_type_id: &str) -> Option<&InstancePluginRow> {
        self.rows.get(plugin_type_id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &InstancePluginRow)> {
        self.rows.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Insert or replace a single row in the in-memory snapshot. Used after
    /// a successful upsert so the returning request reads the new value.
    pub fn upsert(&mut self, plugin_type_id: String, row: InstancePluginRow) {
        self.rows.insert(plugin_type_id, row);
    }
}

/// Load every row from `instance_plugin_setting` in a single query.
pub async fn load_at_boot(db: &DatabaseConnection) -> Result<InstancePluginSnapshot> {
    let rows = Entity::find().all(db).await.map_err(|e| {
        rootcause::report!(QueryError::Db(format!("load_at_boot failed: {e}")))
    })?;
    let snapshot = InstancePluginSnapshot {
        rows: rows
            .into_iter()
            .map(|m| {
                (
                    m.plugin_type_id,
                    InstancePluginRow {
                        enabled: m.enabled,
                        config: m.config,
                        updated_at: m.updated_at,
                    },
                )
            })
            .collect(),
    };
    Ok(snapshot)
}

/// Read-then-write: BEGIN IMMEDIATE per snapshot rule. Returns the previous
/// `enabled` value (None if no prior row) so callers can emit audit details.
pub async fn set_enabled(
    db: &DatabaseConnection,
    plugin_type_id: &str,
    new_enabled: bool,
) -> Result<(Option<bool>, Model)> {
    let txn = db
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
        .map_err(|e| rootcause::report!(QueryError::Db(format!("begin_immediate: {e}"))))?;

    let existing = Entity::find_by_id(plugin_type_id.to_string())
        .one(&txn)
        .await
        .map_err(|e| rootcause::report!(QueryError::Db(format!("find_by_id: {e}"))))?;
    let previous_enabled = existing.as_ref().map(|m| m.enabled);

    let now = OffsetDateTime::now_utc();
    let model = match existing {
        Some(m) => {
            let mut active: ActiveModel = m.into();
            active.enabled = Set(new_enabled);
            active.updated_at = Set(now);
            active.update(&txn).await.map_err(|e| {
                rootcause::report!(QueryError::Db(format!("update: {e}")))
            })?
        }
        None => {
            let active = ActiveModel {
                plugin_type_id: Set(plugin_type_id.to_string()),
                enabled: Set(new_enabled),
                config: Set(serde_json::json!({})),
                updated_at: Set(now),
            };
            active.insert(&txn).await.map_err(|e| {
                rootcause::report!(QueryError::Db(format!("insert: {e}")))
            })?
        }
    };

    txn.commit().await.map_err(|e| {
        rootcause::report!(QueryError::Db(format!("commit: {e}")))
    })?;

    Ok((previous_enabled, model))
}

/// Upsert config — also BEGIN IMMEDIATE since we read enabled to preserve
/// it. (We could write config independently, but the read-then-write keeps
/// the row consistent.)
pub async fn upsert_config(
    db: &DatabaseConnection,
    plugin_type_id: &str,
    new_config: serde_json::Value,
) -> Result<Model> {
    let txn = db
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
        .map_err(|e| rootcause::report!(QueryError::Db(format!("begin_immediate: {e}"))))?;

    let existing = Entity::find_by_id(plugin_type_id.to_string())
        .one(&txn)
        .await
        .map_err(|e| rootcause::report!(QueryError::Db(format!("find_by_id: {e}"))))?;

    let now = OffsetDateTime::now_utc();
    let model = match existing {
        Some(m) => {
            let mut active: ActiveModel = m.into();
            active.config = Set(new_config);
            active.updated_at = Set(now);
            active.update(&txn).await.map_err(|e| {
                rootcause::report!(QueryError::Db(format!("update: {e}")))
            })?
        }
        None => {
            let active = ActiveModel {
                plugin_type_id: Set(plugin_type_id.to_string()),
                enabled: Set(false),
                config: Set(new_config),
                updated_at: Set(now),
            };
            active.insert(&txn).await.map_err(|e| {
                rootcause::report!(QueryError::Db(format!("insert: {e}")))
            })?
        }
    };

    txn.commit().await.map_err(|e| {
        rootcause::report!(QueryError::Db(format!("commit: {e}")))
    })?;

    Ok(model)
}
```

(If `QueryError` doesn't have a generic `Db(String)` variant, add one — match the patterns already used in sibling query modules like
`plugin_type_settings`.)

- [ ] **Step 2: Register in `web-api-queries/src/lib.rs`**

Add `pub mod instance_plugin_settings;` alongside existing module declarations.

- [ ] **Step 3: Build**

```bash
cargo check -p uptrakit-web-api-queries --all-features
```

- [ ] **Step 4: Add unit tests**

Pattern after `plugin_type_settings` integration tests in `web-api/src/integration_tests/`. Cover:

- `load_at_boot` returns empty snapshot when table empty
- `set_enabled` inserts a new row with previous_enabled = None
- `set_enabled` updates an existing row with previous_enabled = Some(old)
- `upsert_config` inserts with default enabled = false
- `upsert_config` preserves enabled when row exists

- [ ] **Step 5: Run tests**

```bash
cargo test -p uptrakit-web-api-queries --all-features instance_plugin_settings
```

Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add crates/ui/web-api-queries/src/instance_plugin_settings.rs \
        crates/ui/web-api-queries/src/lib.rs
git commit -m "feat(web-api-queries): instance_plugin_settings query module

InstancePluginSnapshot + load_at_boot/set_enabled/upsert_config.
BEGIN IMMEDIATE for read-then-write per project rule. Pure rootcause
errors; no parking_lot needed (DB-only)."
```

---

## Task 13: Create visibility predicate helper

**Files:**

- Create: `crates/ui/web-api/src/visibility.rs`
- Modify: `crates/ui/web-api/src/lib.rs`

Snapshot rules: `Extensible public enums #[non_exhaustive]` ⇒ external match sites need wildcard arm with `tracing::warn!` + safe fallback (per
coding-standards.md and CLAUDE memory).

- [ ] **Step 1: Create the file**

```rust
//! Single visibility predicate for plugin descriptors. Centralizes the
//! "is this plugin visible to this user" check so route handlers, the
//! surface registry, and any future filter call into one helper.
use uptrakit_plugin_infrastructure_core::{PluginDescriptor, PluginScope};
use uptrakit_shared_types::Permission;
use uptrakit_web_api_queries::instance_plugin_settings::InstancePluginSnapshot;

use crate::middleware::require_auth::AuthenticatedUser;

/// Returns `true` if the user is allowed to see the plugin in any tenant-
/// facing listing, surface, or detail response.
///
/// - `Tenant`-scoped plugins: always visible.
/// - `Instance`-scoped + enabled: visible to everyone.
/// - `Instance`-scoped + disabled: visible only to users with
///   `ManageGlobalSettings` (instance owners).
///
/// `PluginScope` is `#[non_exhaustive]`; the wildcard arm logs a warning
/// and defaults to visible — the safer side for instance owners
/// (admin debugging) at the cost of a temporary leak should a future
/// scope variant ship before this predicate is updated.
pub fn is_plugin_visible_to_user(
    descriptor: &PluginDescriptor,
    snapshot: &InstancePluginSnapshot,
    user: &AuthenticatedUser,
) -> bool {
    match descriptor.scope {
        PluginScope::Tenant => true,
        PluginScope::Instance => {
            let enabled = snapshot.enabled(descriptor.type_id);
            enabled || user.has_permission(Permission::ManageGlobalSettings)
        }
        _ => {
            tracing::warn!(
                plugin = descriptor.type_id,
                scope = %descriptor.scope,
                "unknown PluginScope variant; defaulting to visible",
            );
            true
        }
    }
}

#[cfg(test)]
mod tests {
    // Implementer note: build a synthetic PluginDescriptor for each scope,
    // and a stub AuthenticatedUser via the existing test_harness builder.
    // Cover:
    // - Tenant-scoped + any user → true
    // - Instance-scoped + enabled + tenant user → true
    // - Instance-scoped + disabled + tenant user → false
    // - Instance-scoped + disabled + ManageGlobalSettings user → true
    // - Instance-scoped + enabled + ManageGlobalSettings user → true
}
```

- [ ] **Step 2: Register in `lib.rs`**

Add `pub mod visibility;`.

- [ ] **Step 3: Implement the unit tests in the file's `mod tests`**

Use the existing `crate::test_harness::fixtures` patterns (see how other modules build `AuthenticatedUser`). Five tests as outlined in the file stub.

- [ ] **Step 4: Run**

```bash
cargo test -p uptrakit-web-api --all-features visibility
```

Expected: 5 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/web-api/src/visibility.rs crates/ui/web-api/src/lib.rs
git commit -m "feat(web-api): visibility predicate for instance-scoped plugins

Single helper used by route handlers and surface registry. Wildcard arm
on #[non_exhaustive] PluginScope logs warn + defaults visible per project
rule. Five-case coverage in unit tests."
```

---

## Task 14: Wire snapshot into `AppState`

**Files:**

- Modify: `crates/ui/web-api/src/app_state.rs`
- Modify: `crates/ui/web-api/Cargo.toml` (add `arc-swap` if not already present)

Snapshot rules: `parking_lot::Mutex` for sync locks; `arc_swap::ArcSwap` for read-mostly snapshots (precedent at
`web-api/global_providers/github.rs`). `tokio::sync::*` forbidden.

- [ ] **Step 1: Confirm `arc-swap` is in workspace deps**

```bash
grep "arc-swap\|arc_swap" Cargo.toml crates/ui/web-api/Cargo.toml
```

If missing, add to `crates/ui/web-api/Cargo.toml` (workspace already pins it for the github provider).

- [ ] **Step 2: Add field to `AppState` (around line 213 in `app_state.rs`)**

```rust
pub struct AppState {
    // ... existing fields ...
    pub instance_plugin_snapshot: Arc<arc_swap::ArcSwap<
        uptrakit_web_api_queries::instance_plugin_settings::InstancePluginSnapshot,
    >>,
}
```

- [ ] **Step 3: Add builder method**

In `AppStateBuilder` (around line 303), add the field + setter:

```rust
    instance_plugin_snapshot: Option<Arc<arc_swap::ArcSwap<
        uptrakit_web_api_queries::instance_plugin_settings::InstancePluginSnapshot,
    >>>,
```

```rust
    pub fn instance_plugin_snapshot(
        mut self,
        v: Arc<arc_swap::ArcSwap<
            uptrakit_web_api_queries::instance_plugin_settings::InstancePluginSnapshot,
        >>,
    ) -> Self {
        self.instance_plugin_snapshot = Some(v);
        self
    }
```

In `build()`, take the field with the existing missing-builder error pattern:

```rust
    instance_plugin_snapshot: self.instance_plugin_snapshot.ok_or(
        AppStateBuildError("instance_plugin_snapshot"),
    )?,
```

- [ ] **Step 4: Update test harness**

`crates/ui/web-api/src/test_harness.rs` (or wherever `TestApp::new` constructs `AppState`) — set the snapshot to an empty
`Arc::new(ArcSwap::from_pointee(InstancePluginSnapshot::empty()))`.

- [ ] **Step 5: Build + run web-api tests**

```bash
cargo check -p uptrakit-web-api --all-features
cargo test -p uptrakit-web-api --all-features
```

Expected: all green; harness picks up the new field.

- [ ] **Step 6: Commit**

```bash
git add crates/ui/web-api/Cargo.toml \
        crates/ui/web-api/src/app_state.rs \
        crates/ui/web-api/src/test_harness.rs
git commit -m "feat(web-api): expose InstancePluginSnapshot via AppState

Stored as Arc<ArcSwap<...>> (read-optimized atomic swap, matches the
existing global_providers/github.rs pattern). tokio::sync::* not used."
```

---

## Task 15: Wire snapshot loading into controller boot

**Files:**

- Modify: controller boot path. Locate via:

  ```bash
  grep -rn "PluginCatalog::new\|AppStateBuilder::new\|InstancePluginStates" crates/core --include="*.rs"
  ```

- [ ] **Step 1: Identify the boot module**

Read the file containing `PluginCatalog::new` in the controller crate. Likely `crates/core/controller/src/main.rs` or a helper module.

- [ ] **Step 2: Insert snapshot load before catalog construction**

Sketch (concrete line numbers depend on the file):

```rust
let instance_plugin_snapshot =
    uptrakit_web_api_queries::instance_plugin_settings::load_at_boot(&db).await?;

// Build a `&'static`-keyed states map by intersecting the snapshot with the
// known descriptor list from the registry.
let descriptors = uptrakit_plugin_infrastructure_registry::register_all_descriptors();
let instance_states = uptrakit_plugin_infrastructure_core::InstancePluginStates::from_pairs(
    descriptors
        .iter()
        .filter(|d| d.scope == uptrakit_plugin_infrastructure_core::PluginScope::Instance)
        .map(|d| (d.type_id, instance_plugin_snapshot.enabled(d.type_id))),
);

let catalog = uptrakit_plugin_infrastructure_registry::build_catalog(
    descriptors,
    catalog_config,
    instance_states,
)?;

let snapshot_handle = std::sync::Arc::new(arc_swap::ArcSwap::from_pointee(instance_plugin_snapshot));

let app_state = AppStateBuilder::new()
    // ... existing setters ...
    .instance_plugin_snapshot(std::sync::Arc::clone(&snapshot_handle))
    .build()?;
```

- [ ] **Step 3: Update `register_all` (or equivalent) in the registry crate**

If the registry exposes a single `build_catalog`-like function, extend its signature to take `InstancePluginStates`. Otherwise extend the boot path
directly.

- [ ] **Step 4: Build the controller binary**

```bash
cargo build -p uptrakit-controller --all-features 2>&1 | tail -20
```

- [ ] **Step 5: Run any controller boot smoke tests**

```bash
cargo test -p uptrakit-controller --all-features
```

- [ ] **Step 6: Commit**

```bash
git add crates/core/controller/src/main.rs \
        crates/plugins/infrastructure/registry/src/registry.rs
git commit -m "feat(controller): load instance plugin snapshot at boot

Order: open DB → run migrations → load InstancePluginSnapshot → build
catalog with InstancePluginStates → start web-api with Arc<ArcSwap<>>
in AppState."
```

---

## Task 16: Quality gates checkpoint

**Files:** none (verification only)

Snapshot rules: every gate in `docs/development/pr-process.md`.

- [ ] **Step 1: Format**

```bash
cargo fmt --all
git diff --quiet || (echo 'fmt produced changes'; git status; exit 1)
```

- [ ] **Step 2: Cargo check both feature combos**

```bash
cargo check --no-default-features --features db-sqlite
cargo check --all-features
```

- [ ] **Step 3: Clippy both feature combos with `-D warnings`**

```bash
cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings
cargo clippy --all-targets --all-features -- -D warnings
```

- [ ] **Step 4: Tests**

```bash
cargo test --all-features
```

- [ ] **Step 5: cargo deny**

```bash
cargo deny check
```

- [ ] **Step 6: Markdownlint (in case any docs touched)**

```bash
markdownlint --config .markdownlint.json '**/*.md'
```

- [ ] **Step 7: Smoke-boot the controller**

```bash
cargo run -p uptrakit-controller -- --master-key-file <test-key> --help
```

Just the `--help` is fine — confirms the binary builds and the snapshot wiring doesn't panic on init.

- [ ] **Step 8: No commit needed**

If everything is green, Plan A is done. Plan B (API + dashboard-icons + audit + ADR) starts from this branch.

---

## Self-review

Plan A vs spec:

- **Spec §2 (PluginScope, InstanceConfigOps, descriptor extension, invariants, struct-literal compat, declare_plugin! grammar):** Tasks 2, 3, 4, 5, 6.
- **Spec §3 (entity, migration, registration):** Tasks 10, 11.
- **Spec §4 (catalog gating, snapshot loading, AppState wiring, instance_enabled on PluginMetadataOps):** Tasks 7, 8, 9, 12, 14, 15.
- **Spec §6 (visibility predicate):** Task 13. Wildcard arm on `#[non_exhaustive]` PluginScope match per coding-standards.md.
- **Quality gates:** Task 16.

Out of scope for Plan A (deferred to B/C):

- Routes, DTOs, audit constants, dashboard-icons descriptor flip — Plan B
- Frontend Instance Plugins section — Plan C
- ADR + plugin-guidelines.md + ARCHITECTURE.md + end-user docs — Plans B/C

Snapshot conformance per task — every task that introduces an enum/struct uses `#[non_exhaustive]`; every error path uses `rootcause::Report` +
`report!`; every concurrency point uses `parking_lot` or `arc_swap` (never `tokio::sync::*`); every SeaORM `Value` column uses `column_type = "Json"`
and skips `Eq`; every match on `#[non_exhaustive]` enum carries a wildcard arm with `tracing::warn!`. No `#[allow(...)]` introduced.

No "silence the lint" tasks. No vague "polish" tasks. No fights with the framework — extends existing patterns (`PluginDescriptor`, `declare_plugin!`,
`PluginMetadataOps`, `AppStateBuilder`, `arc_swap` precedent).
