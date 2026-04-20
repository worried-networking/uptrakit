# Plugin Boundary Isolation Track A Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to
> implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove remaining removable direct plugin-crate dependencies from non-plugin crates, retire local `PluginTypeId` helper classification, and
align Sentrux static-boundary rules with the reviewed Track A spec.

**Architecture:** Track A is a static-boundary migration, not a semantic plugin-knowledge cleanup. Non-plugin metadata and descriptor lookups should
go through `uptrakit-plugin-infrastructure-registry`, while the explicit carve-out for `agent-core`, `scheduler-engine`, and `agent-ssh` keeps only
operational `infrastructure-core` protocol usage. Sentrux should encode that carve-out structurally by rewriting both `[[layers]]` and
`[[boundaries]]` around explicit crate-root groups instead of relying on suppressions, and verification should compare plugin-boundary results against
a Track A baseline rather than requiring the whole repo to be globally clean.

**Tech Stack:** Rust workspace crates, Cargo manifests, `serena`-guided code edits, Sentrux architectural rules, ripgrep verification, frontend build
prerequisite for `cargo check --all-features`

---

## File Structure

### Registry surface

- Modify:
  [`crates/plugins/infrastructure/registry/src/registry.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/registry/src/registry.rs)
  Responsibility: add narrow lookup helpers backed by descriptor data, including a registry-owned package-manager classifier that preserves current
  autodiscovery semantics for all package-manager plugins.
- Modify:
  [`crates/plugins/infrastructure/registry/src/lib.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/registry/src/lib.rs)
  Responsibility: re-export new lookup helpers and keep downstream imports on the registry surface.

### UI layer

- Modify: [`crates/ui/web-api/Cargo.toml`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/Cargo.toml) Responsibility: drop the direct
  `uptrakit-notification-plugin-core` dependency.
- Modify:
  [`crates/ui/web-api/src/notifications/message_builder.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/notifications/message_builder.rs)
  Responsibility: import `DeliveryMessage`, `MessageAction`, and `escape_html` from the registry instead of notification-core.
- Modify: [`crates/ui/web-api/src/routes/notifications.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/routes/notifications.rs)
  Responsibility: import `DeliveryMessage` from the registry instead of notification-core.
- Modify: [`crates/ui/web-api-queries/Cargo.toml`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/Cargo.toml) Responsibility:
  replace the direct `uptrakit-plugin-infrastructure-core` dependency with the registry crate.
- Modify:
  [`crates/ui/web-api-queries/src/queries/plugin_configs.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/plugin_configs.rs)
- Modify:
  [`crates/ui/web-api-queries/src/queries/discovery_allowlist.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/discovery_allowlist.rs)
- Modify:
  [`crates/ui/web-api-queries/src/queries/notifications.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/notifications.rs)
- Modify:
  [`crates/ui/web-api-queries/src/queries/software_items/crud.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/software_items/crud.rs)
- Modify:
  [`crates/ui/web-api-queries/src/queries/software_items/host_assignments.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/software_items/host_assignments.rs)
- Modify:
  [`crates/ui/web-api-queries/src/queries/autodiscovery/discovery_items.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/autodiscovery/discovery_items.rs)
  Responsibility: replace the single production `is_package_manager()` call with a registry-backed predicate in Task 4.

### Shared/core carve-out audit

- Modify if needed: [`crates/shared/types/src/plugin_type_id.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/types/src/plugin_type_id.rs)
  Responsibility: retire `is_package_manager()` from production use and mark `display_name()` as dead/transitional debt if it remains.
- Audit: [`crates/shared/agent-core/Cargo.toml`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/Cargo.toml)
- Audit: [`crates/shared/agent-core/src/client.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/src/client.rs)
- Audit: [`crates/shared/agent-core/src/update.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/src/update.rs)
- Audit: [`crates/shared/agent-core/src/version_check.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/src/version_check.rs)
- Audit: [`crates/shared/agent-core/src/config_test.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/src/config_test.rs)
- Audit:
  [`crates/shared/agent-core/src/connection_context.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/src/connection_context.rs)
- Audit: [`crates/shared/scheduler-engine/Cargo.toml`](/Users/andreyyantsen/Development/uptrakit/crates/shared/scheduler-engine/Cargo.toml)
- Audit:
  [`crates/shared/scheduler-engine/src/executors/fetch_releases.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/scheduler-engine/src/executors/fetch_releases.rs)
- Modify: [`crates/core/controller/Cargo.toml`](/Users/andreyyantsen/Development/uptrakit/crates/core/controller/Cargo.toml)
- Modify: [`crates/core/controller/src/main.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/controller/src/main.rs)
- Audit: [`crates/core/agent-ssh/Cargo.toml`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/Cargo.toml)
- Audit: [`crates/core/agent-ssh/src/runtime_support.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/runtime_support.rs)
- Audit: [`crates/core/agent-ssh/src/extension.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/extension.rs)
- Audit:
  [`crates/core/agent-ssh/src/commands/bootstrap.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/commands/bootstrap.rs)
- Audit: [`crates/core/agent-ssh/src/commands/sync.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/commands/sync.rs)
- Audit:
  [`crates/core/agent-ssh/src/commands/bootstrap_proxmox.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/commands/bootstrap_proxmox.rs)
- Audit:
  [`crates/core/agent-ssh/src/operations/bootstrap.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/operations/bootstrap.rs)
- Audit: [`crates/core/agent-ssh/src/operations/sync.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/operations/sync.rs)
- Audit:
  [`crates/core/agent-ssh/src/operations/bootstrap_proxmox.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/operations/bootstrap_proxmox.rs)
  Responsibility: keep only allowlisted operational `infrastructure-core` symbol families in carve-out crates; migrate removable metadata/helper
  imports where found.

### Sentrux and docs

- Modify: [`.sentrux/rules.toml`](/Users/andreyyantsen/Development/uptrakit/.sentrux/rules.toml) Responsibility: replace broad plugin boundaries with
  explicit non-carve-out crate-root rules and add missing `hooks`, `enhancements`, and `discovery` family coverage.
- Audit/modify if needed: [`docs/development/plugin-system.md`](/Users/andreyyantsen/Development/uptrakit/docs/development/plugin-system.md)
- Audit/modify if needed: [`docs/development/plugin-guidelines.md`](/Users/andreyyantsen/Development/uptrakit/docs/development/plugin-guidelines.md)
  Responsibility: remove statements that imply direct non-registry plugin imports are acceptable in non-plugin crates.

### Verification commands

- `cargo test -p uptrakit-plugin-infrastructure-registry`
- `cargo check -p uptrakit-web-api`
- `cargo check -p uptrakit-web-api-queries`
- `cargo check -p uptrakit-controller --no-default-features --features db-sqlite`
- `cargo check --no-default-features --features db-sqlite`
- `cargo clippy --all-targets --no-default-features --features db-sqlite`
- `cd frontend && npm ci && npm run build && cd .. && cargo check --all-features`
- `cargo clippy --all-targets --all-features`
- `cargo test --all-features`
- `sentrux check .`
- `rg -n 'uptrakit_(plugin_|notification_plugin_core)' crates/ui crates/core crates/shared --glob '*.rs'`
- `rg -n 'uptrakit-plugin-|uptrakit-notification-plugin-core' crates/ui crates/core crates/shared --glob 'Cargo.toml'`
- `rg -F -n '.is_package_manager(' crates/ui crates/core crates/shared`
- `rg -F -n '.display_name(' crates/ui crates/core crates/shared | rg -v 'plugin_ops.display_name'`
- `rg -F -n -e 'starts_with("package_manager_")' -e 'package_manager_'` `-e 'plugin_ids::' -e 'releases_' -e 'notifications_' -e 'hooks_'`
  `-e 'enhancements_' crates/ui crates/core crates/shared`

### Task 1: Add Registry Lookup Helpers And Coverage

**Files:**

- Modify:
  [`crates/plugins/infrastructure/registry/src/registry.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/registry/src/registry.rs)
- Modify:
  [`crates/plugins/infrastructure/registry/src/lib.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/registry/src/lib.rs)
- Test:
  [`crates/plugins/infrastructure/registry/src/registry.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/registry/src/registry.rs)

- [ ] **Step 1: Write the failing registry tests**

```rust
#[test]
fn package_manager_lookup_covers_all_current_package_managers() {
    let package_managers = [
        plugin_ids::PACKAGE_MANAGER_APT,
        plugin_ids::PACKAGE_MANAGER_HOMEBREW,
        plugin_ids::PACKAGE_MANAGER_DNF,
        plugin_ids::PACKAGE_MANAGER_NPM,
        plugin_ids::PACKAGE_MANAGER_MAS,
        plugin_ids::PACKAGE_MANAGER_PACMAN,
        plugin_ids::PACKAGE_MANAGER_PKG,
        plugin_ids::PACKAGE_MANAGER_APK,
        plugin_ids::PACKAGE_MANAGER_SNAP,
        plugin_ids::PACKAGE_MANAGER_CARGO,
    ];
    let github = PluginTypeId::from_static("releases_github");

    for plugin_type in package_managers {
        assert!(is_package_manager_plugin(&plugin_type));
    }
    assert!(!is_package_manager_plugin(&github));
}

#[test]
fn plugin_family_lookup_returns_descriptor_family() {
    let apt = PluginTypeId::from_static("package_manager_apt");
    let proxmox = PluginTypeId::from_static("infrastructure_proxmox");
    let missing = PluginTypeId::new("missing_plugin");

    assert_eq!(plugin_family(&apt), Some(PluginFamily::Software));
    assert_eq!(plugin_family(&proxmox), Some(PluginFamily::Infrastructure));
    assert_eq!(plugin_family(&missing), None);
}
```

- [ ] **Step 2: Run the registry tests to verify they fail**

Run: `cargo test -p uptrakit-plugin-infrastructure-registry plugin_family_lookup_returns_descriptor_family -- --exact`

Expected: FAIL with an unresolved function error for `plugin_family` and `is_package_manager_plugin`.

- [ ] **Step 3: Implement the lookup helpers and re-export them**

```rust
pub fn plugin_family(plugin_type_id: &PluginTypeId) -> Option<PluginFamily> {
    get_descriptor(plugin_type_id.as_str()).map(|d| d.family)
}

pub fn is_package_manager_plugin(plugin_type_id: &PluginTypeId) -> bool {
    const PACKAGE_MANAGER_IDS: &[PluginTypeId] = &[
        plugin_ids::PACKAGE_MANAGER_APT,
        plugin_ids::PACKAGE_MANAGER_HOMEBREW,
        plugin_ids::PACKAGE_MANAGER_DNF,
        plugin_ids::PACKAGE_MANAGER_NPM,
        plugin_ids::PACKAGE_MANAGER_MAS,
        plugin_ids::PACKAGE_MANAGER_PACMAN,
        plugin_ids::PACKAGE_MANAGER_PKG,
        plugin_ids::PACKAGE_MANAGER_APK,
        plugin_ids::PACKAGE_MANAGER_SNAP,
        plugin_ids::PACKAGE_MANAGER_CARGO,
    ];

    PACKAGE_MANAGER_IDS.iter().any(|known| known == plugin_type_id)
}
```

```rust
pub use registry::{
    all_descriptors, all_required_sudo_commands, compatible_sudo_commands_for_host, get_descriptor,
    is_package_manager_plugin, plugin_family,
};
```

- [ ] **Step 4: Run the targeted registry tests**

Run: `cargo test -p uptrakit-plugin-infrastructure-registry`

Expected: PASS, including the new lookup tests and the existing descriptor sanity tests.

- [ ] **Step 5: Commit**

```bash
git add crates/plugins/infrastructure/registry/src/lib.rs crates/plugins/infrastructure/registry/src/registry.rs
git commit -m "refactor: add registry lookup helpers for track a"
```

### Task 2: Migrate Web API Notification-Core Imports To The Registry

**Files:**

- Modify: [`crates/ui/web-api/Cargo.toml`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/Cargo.toml)
- Modify:
  [`crates/ui/web-api/src/notifications/message_builder.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/notifications/message_builder.rs)
- Modify: [`crates/ui/web-api/src/routes/notifications.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/routes/notifications.rs)
- Test:
  [`crates/ui/web-api/src/notifications/message_builder.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/notifications/message_builder.rs)

- [ ] **Step 1: Remove the direct manifest dependency first**

```toml
-uptrakit-notification-plugin-core = { workspace = true }
```

- [ ] **Step 2: Run a package check to verify the direct imports now fail**

Run: `cargo check -p uptrakit-web-api`

Expected: FAIL with unresolved import errors for `uptrakit_notification_plugin_core`.

- [ ] **Step 3: Replace the imports with registry re-exports**

```rust
use uptrakit_plugin_infrastructure_registry::{DeliveryMessage, MessageAction, escape_html};
```

```rust
use uptrakit_plugin_infrastructure_registry::DeliveryMessage;
```

- [ ] **Step 4: Re-run the package check**

Run: `cargo check -p uptrakit-web-api`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/web-api/Cargo.toml crates/ui/web-api/src/notifications/message_builder.rs crates/ui/web-api/src/routes/notifications.rs
git commit -m "refactor: route web api notification types through registry"
```

### Task 3: Migrate Web API Queries Off `infrastructure-core`

**Files:**

- Modify: [`crates/ui/web-api-queries/Cargo.toml`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/Cargo.toml)
- Modify:
  [`crates/ui/web-api-queries/src/queries/plugin_configs.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/plugin_configs.rs)
- Modify:
  [`crates/ui/web-api-queries/src/queries/discovery_allowlist.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/discovery_allowlist.rs)
- Modify:
  [`crates/ui/web-api-queries/src/queries/notifications.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/notifications.rs)
- Modify:
  [`crates/ui/web-api-queries/src/queries/software_items/crud.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/software_items/crud.rs)
- Modify:
  [`crates/ui/web-api-queries/src/queries/software_items/host_assignments.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/software_items/host_assignments.rs)
- Test:
  [`crates/ui/web-api-queries/src/queries/plugin_configs.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/plugin_configs.rs)

- [ ] **Step 1: Swap the manifest dependency from core to registry**

```toml
-uptrakit-plugin-infrastructure-core = { workspace = true, features = ["plugin-ops"] }
+uptrakit-plugin-infrastructure-registry = { workspace = true }
```

- [ ] **Step 2: Run a package check to expose remaining direct imports**

Run: `cargo check -p uptrakit-web-api-queries`

Expected: FAIL with unresolved import errors for `PluginConfigOps`, `PluginOps`, `SoftwareItemPatch`, `descriptor::PluginDescriptor`, `RoleKey`, and
any other remaining `uptrakit_plugin_infrastructure_core::...` paths.

- [ ] **Step 3: Replace plugin-ops imports with the registry surface**

```rust
use uptrakit_plugin_infrastructure_registry::{
    NotificationOps, PluginConfigOps, PluginDescriptor, PluginExtensionOps, PluginMetadataOps,
    PluginOps, RoleKey, SoftwareItemLifecycleOps, SoftwareItemPatch,
};
```

Also replace inline fully-qualified references such as:

```rust
-impl uptrakit_plugin_infrastructure_core::PluginMetadataOps for MockPluginOps {
+impl uptrakit_plugin_infrastructure_registry::PluginMetadataOps for MockPluginOps {
```

```rust
-Option<&uptrakit_plugin_infrastructure_core::descriptor::PluginDescriptor>
+Option<&uptrakit_plugin_infrastructure_registry::PluginDescriptor>
```

- [ ] **Step 4: Re-run the package check**

Run: `cargo check -p uptrakit-web-api-queries`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/web-api-queries/Cargo.toml crates/ui/web-api-queries/src/queries/plugin_configs.rs crates/ui/web-api-queries/src/queries/discovery_allowlist.rs crates/ui/web-api-queries/src/queries/notifications.rs crates/ui/web-api-queries/src/queries/software_items/crud.rs crates/ui/web-api-queries/src/queries/software_items/host_assignments.rs
git commit -m "refactor: route web api query plugin ops through registry"
```

### Task 4: Replace `is_package_manager()` With A Registry Predicate And Retire Helpers

**Files:**

- Modify:
  [`crates/ui/web-api-queries/src/queries/autodiscovery/discovery_items.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/autodiscovery/discovery_items.rs)
- Modify: [`crates/shared/types/src/plugin_type_id.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/types/src/plugin_type_id.rs)
- Test:
  [`crates/ui/web-api-queries/src/queries/autodiscovery/discovery_items.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/queries/autodiscovery/discovery_items.rs)
- Test: [`crates/shared/types/src/plugin_type_id.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/types/src/plugin_type_id.rs)

- [ ] **Step 1: Add a real regression test for a package manager without type settings**

```rust
#[tokio::test]
async fn target_based_npm_creates_hsip_without_plugin_config() {
    let db = setup_db().await;
    let tenant_id = Uuid::now_v7();
    let host_id = Uuid::now_v7();
    let now = time::OffsetDateTime::now_utc();

    insert_tenant(&db, tenant_id).await;
    insert_host(&db, host_id, tenant_id).await;

    let mut result = phs_result_with_apt_target("pm2", "PM2", "5.4.2");
    result.discoveries[0].targets[0].plugin_type = plugin_ids::PACKAGE_MANAGER_NPM.clone();

    process_plugin_result(&db, tenant_id, host_id, now, &result, &HashSet::new())
        .await
        .expect("process_plugin_result");

    let configs = PluginConfig::find()
        .filter(plugin_config::Column::TenantId.eq(tenant_id))
        .filter(plugin_config::Column::PluginType.eq("package_manager_npm"))
        .all(&db)
        .await
        .expect("query configs");
    assert!(
        configs.is_empty(),
        "package managers no longer create plugin_configs"
    );

    let plugin_links = HostSoftwareItemPlugin::find()
        .filter(host_software_item_plugin::Column::HostId.eq(host_id))
        .filter(host_software_item_plugin::Column::PackageIdentifier.eq("pm2"))
        .all(&db)
        .await
        .expect("query plugin links");
    assert!(!plugin_links.is_empty(), "expected plugin links for npm target");
    for link in &plugin_links {
        assert!(
            link.plugin_config_id.is_none(),
            "package manager HSIP rows must have plugin_config_id = NULL"
        );
        assert_eq!(link.plugin_type, "package_manager_npm");
    }
}
```

- [ ] **Step 2: Run the new test and the existing APT regression to verify the current behavior**

Run:

```bash
cargo test -p uptrakit-web-api-queries target_based_apt_creates_hsip_without_plugin_config -- --exact
cargo test -p uptrakit-web-api-queries target_based_npm_creates_hsip_without_plugin_config -- --exact
```

Expected: PASS. This locks in the current npm behavior before the helper migration.

- [ ] **Step 3: Replace the production caller in autodiscovery**

```rust
use uptrakit_plugin_infrastructure_registry::is_package_manager_plugin;

-let pc_id = if target.plugin_type.is_package_manager() {
+let pc_id = if is_package_manager_plugin(&target.plugin_type) {
```

- [ ] **Step 4: Run the targeted regression tests**

Run:

```bash
cargo test -p uptrakit-web-api-queries target_based_apt_creates_hsip_without_plugin_config -- --exact
cargo test -p uptrakit-web-api-queries target_based_npm_creates_hsip_without_plugin_config -- --exact
```

Expected: PASS.

- [ ] **Step 5: Deprecate or remove the shared helpers**

```rust
#[deprecated(note = "Use registry is_package_manager_plugin() instead")]
pub fn is_package_manager(&self) -> bool {
    self.0.starts_with("package_manager_")
}

#[deprecated(note = "No Track A replacement; remove callers or add a dedicated registry label lookup later")]
pub fn display_name(&self) -> &str {
    // body unchanged
}
```

Do not replace the existing `display_name()` match arms with a stub. Add only the attribute unless the method is deleted outright.

If `display_name()` has no internal callers after this task, it is acceptable to delete it instead of deprecating it.

- [ ] **Step 6: Verify no production callers remain**

Run:

```bash
rg -F -n '.is_package_manager(' crates/ui crates/core crates/shared
rg -F -n '.display_name(' crates/ui crates/core crates/shared | rg -v 'plugin_ops.display_name'
```

Expected:

- no production `.is_package_manager(` matches
- no production `PluginTypeId::display_name()` callsites after excluding the known `plugin_ops.display_name` path
- any remaining hits must be tests or the deprecated method definitions themselves

- [ ] **Step 7: Commit**

```bash
git add crates/ui/web-api-queries/src/queries/autodiscovery/discovery_items.rs crates/shared/types/src/plugin_type_id.rs
git commit -m "refactor: replace plugin type helpers with registry predicate"
```

### Task 5: Remove Removable Core-Layer Direct Dependency In Controller

**Files:**

- Modify: [`crates/core/controller/Cargo.toml`](/Users/andreyyantsen/Development/uptrakit/crates/core/controller/Cargo.toml)
- Modify: [`crates/core/controller/src/main.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/controller/src/main.rs)
- Test: [`crates/core/controller/src/main.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/controller/src/main.rs)

- [ ] **Step 1: Remove the direct `infrastructure-core` manifest dependency**

```toml
-uptrakit-plugin-infrastructure-core = { workspace = true, features = ["http-client"] }
```

- [ ] **Step 2: Run a package check to verify the helper import now fails**

Run: `cargo check -p uptrakit-controller --no-default-features --features db-sqlite`

Expected: FAIL with unresolved references to `build_plugin_http_client` and `PluginHttpClientConfig`.

- [ ] **Step 3: Use the registry re-export instead**

```rust
use uptrakit_plugin_infrastructure_registry::{PluginHttpClientConfig, build_plugin_http_client};

let http_client = build_plugin_http_client(PluginHttpClientConfig {
    user_agent: "uptrakit-controller",
    redirect_policy: reqwest::redirect::Policy::limited(5),
    ..Default::default()
});
```

- [ ] **Step 4: Re-run the controller package check**

Run: `cargo check -p uptrakit-controller --no-default-features --features db-sqlite`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core/controller/Cargo.toml crates/core/controller/src/main.rs
git commit -m "refactor: route controller plugin http client through registry"
```

### Task 6: Audit The Operational Carve-Out Crates

**Files:**

- Audit: [`crates/shared/agent-core/Cargo.toml`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/Cargo.toml)
- Audit: [`crates/shared/agent-core/src/client.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/src/client.rs)
- Audit: [`crates/shared/agent-core/src/update.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/src/update.rs)
- Audit: [`crates/shared/agent-core/src/version_check.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/src/version_check.rs)
- Audit: [`crates/shared/agent-core/src/config_test.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/src/config_test.rs)
- Audit:
  [`crates/shared/agent-core/src/connection_context.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/src/connection_context.rs)
- Audit: [`crates/shared/scheduler-engine/Cargo.toml`](/Users/andreyyantsen/Development/uptrakit/crates/shared/scheduler-engine/Cargo.toml)
- Audit:
  [`crates/shared/scheduler-engine/src/executors/fetch_releases.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/scheduler-engine/src/executors/fetch_releases.rs)
- Audit: [`crates/core/agent-ssh/Cargo.toml`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/Cargo.toml)
- Audit: [`crates/core/agent-ssh/src/runtime_support.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/runtime_support.rs)
- Audit: [`crates/core/agent-ssh/src/extension.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/extension.rs)
- Audit:
  [`crates/core/agent-ssh/src/commands/bootstrap.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/commands/bootstrap.rs)
- Audit: [`crates/core/agent-ssh/src/commands/sync.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/commands/sync.rs)
- Audit:
  [`crates/core/agent-ssh/src/commands/bootstrap_proxmox.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/commands/bootstrap_proxmox.rs)
- Audit:
  [`crates/core/agent-ssh/src/operations/bootstrap.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/operations/bootstrap.rs)
- Audit: [`crates/core/agent-ssh/src/operations/sync.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/operations/sync.rs)
- Audit:
  [`crates/core/agent-ssh/src/operations/bootstrap_proxmox.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/operations/bootstrap_proxmox.rs)

- [ ] **Step 1: Capture the current direct `infrastructure-core` usages**

Run: `rg -n 'uptrakit_plugin_infrastructure_core' crates/shared/agent-core crates/shared/scheduler-engine crates/core/agent-ssh`

Expected: matches in the carve-out crates only.

- [ ] **Step 2: Compare each match against the allowlisted symbol families**

Use this allowlist:

```text
construct_host_runtime
HostRuntime
HostCapabilities
BatchDetectItem
BatchFetchItem
BatchFetchResult
BatchUpdateItem
PluginCapability
HostCompatibility
UpdateLifecycleContext
PluginError
BootstrapInfraResult
InfraBundle
InfraActionInvoker
InfraPluginContext
GuestBootstrapExecutor
GuestBootstrapParams
GuestBootstrapResult
SudoCommandEntry
```

- [ ] **Step 3: If any match falls outside the allowlist, migrate it immediately**

Example migration pattern for removable metadata access:

```rust
-use uptrakit_plugin_infrastructure_core::PluginConfigOps;
+use uptrakit_plugin_infrastructure_registry::PluginConfigOps;
```

Example migration pattern for inline fully-qualified references:

```rust
-plugin_ops: &dyn uptrakit_plugin_infrastructure_core::PluginOps,
+plugin_ops: &dyn uptrakit_plugin_infrastructure_registry::PluginOps,
```

Example migration pattern for removable utility access:

```rust
-uptrakit_plugin_infrastructure_core::build_plugin_http_client(...)
+uptrakit_plugin_infrastructure_registry::build_plugin_http_client(...)
```

- [ ] **Step 4: Re-run the audit grep and package checks**

Run: `cargo check -p uptrakit-agent-core && cargo check -p uptrakit-scheduler-engine && cargo check -p uptrakit-agent-ssh`

Expected: PASS, with remaining direct `infrastructure-core` references limited to allowlisted operational protocol usage.

- [ ] **Step 5: Commit if any code changed**

```bash
git add crates/shared/agent-core crates/shared/scheduler-engine crates/core/agent-ssh
git commit -m "refactor: prune removable plugin core usage from carve-out crates"
```

If the audit finds no out-of-allowlist usages, skip the commit and carry the audit result into Task 8 verification notes.

### Task 7: Rewrite Sentrux Rules Without Suppressions

**Files:**

- Modify: [`.sentrux/rules.toml`](/Users/andreyyantsen/Development/uptrakit/.sentrux/rules.toml)

- [ ] **Step 1: Snapshot the current plugin-boundary rule behavior**

Run:

```bash
sentrux check . | tee /tmp/track-a-sentrux-before.txt
rg -n 'layer_direction|boundary' /tmp/track-a-sentrux-before.txt
```

Expected: current output includes plugin-boundary `layer_direction` and `boundary` violations, plus unrelated global rule failures.

- [ ] **Step 2: Rewrite both layers and boundaries around explicit crate-root groups**

Do not keep these broad rules:

```toml
[[layers]]
name = "binaries"
paths = ["crates/core/**"]
order = 0

[[layers]]
name = "shared"
paths = ["crates/shared/**"]
order = 3

[[boundaries]]
from = "crates/shared/**"
to = "crates/plugins/**"
reason = "Shared crates must not depend on plugin crates"

[[boundaries]]
from = "crates/core/**"
to = "crates/plugins/infrastructure/core/**"
reason = "Binary crates must not bypass the registry and depend on plugin infrastructure/core directly"
```

Instead, encode the carve-out structurally by:

1. creating an explicit carve-out layer for:

```toml
[[layers]]
name = "plugin_protocol_clients"
paths = [
  "crates/shared/agent-core/**",
  "crates/shared/scheduler-engine/**",
  "crates/core/agent-ssh/**",
]
order = 2
```

1. replacing the broad `shared` and `binaries` layers with explicit non-carve-out root lists.

Use explicit `paths` values for the non-carve-out shared layer:

```toml
"crates/shared/audit-log/**"
"crates/shared/backoff/**"
"crates/shared/build-info/**"
"crates/shared/command/**"
"crates/shared/crypto/**"
"crates/shared/db/**"
"crates/shared/directories/**"
"crates/shared/extension-framework/**"
"crates/shared/macros/**"
"crates/shared/nats/**"
"crates/shared/openapi-client/**"
"crates/shared/service-platform/**"
"crates/shared/service-sdk/**"
"crates/shared/tracing-init/**"
"crates/shared/types/**"
"crates/shared/update-hooks/**"
"crates/shared/web-api-types/**"
"crates/shared/wire/**"
```

Use explicit `paths` values for the non-carve-out core layer:

```toml
"crates/core/agent/**"
"crates/core/agent-runtime/**"
"crates/core/agent-ssh-runtime/**"
"crates/core/controller/**"
"crates/core/integration-tests/**"
"crates/core/mqtt/**"
"crates/core/mqtt-runtime/**"
"crates/core/scheduler/**"
"crates/core/scheduler-runtime/**"
```

Then add exact boundary tuples for the non-carve-out roots to the disallowed plugin families, while leaving
`crates/plugins/infrastructure/registry/**` reachable from non-plugin crates.

At minimum, the new explicit tuples must replace the deleted broad rules for:

```toml
to = "crates/plugins/releases/**"
to = "crates/plugins/package-managers/**"
to = "crates/plugins/notifications/**"
to = "crates/plugins/discovery/**"
to = "crates/plugins/generic/**"
to = "crates/plugins/hooks/**"
to = "crates/plugins/enhancements/**"
to = "crates/plugins/infrastructure/core/**"
to = "crates/plugins/infrastructure/proxmox/**"
```

The shared/core matrix is intentionally mechanical. With 18 non-carve-out shared roots, 9 non-carve-out core roots, and 9 disallowed plugin-family
targets, the final rule set should contain 243 explicit shared/core plugin-boundary tuples before the separate UI-family rules are counted.

Use repeated per-root boundaries for the non-carve-out shared/core roots. A complete tuple should look like:

```toml
[[boundaries]]
from = "crates/shared/audit-log/**"
to = "crates/plugins/infrastructure/core/**"
reason = "Non-carve-out shared crates must not bypass the registry and depend on plugin infrastructure/core directly"

[[boundaries]]
from = "crates/core/controller/**"
to = "crates/plugins/package-managers/**"
reason = "Non-carve-out core crates must not directly import package manager plugin crates — use infrastructure/registry"
```

Do not introduce any new `from` glob that still matches `crates/shared/agent-core/**`, `crates/shared/scheduler-engine/**`, or
`crates/core/agent-ssh/**`.

- [ ] **Step 3: Add the missing UI plugin-family boundaries and remove superseded broad rules**

Add explicit UI boundaries for the missing families. Do not duplicate the per-core `hooks/**` and `enhancements/**` tuples from Step 2; those are
already covered by the 243-entry shared/core matrix.

```toml
[[boundaries]]
from = "crates/ui/**"
to = "crates/plugins/hooks/**"
reason = "UI crates must not directly import hook plugin crates — use infrastructure/registry"

[[boundaries]]
from = "crates/ui/**"
to = "crates/plugins/enhancements/**"
reason = "UI crates must not directly import enhancement plugin crates — use infrastructure/registry"
```

Keep the existing `discovery/**` family in the final rule set.

Also delete the superseded broad plugin-boundary rules that would otherwise continue to match the carve-out crates.

- [ ] **Step 4: Run Sentrux and compare only plugin-boundary results against baseline**

Run:

```bash
sentrux check . | tee /tmp/track-a-sentrux-after.txt
rg -n 'layer_direction|boundary' /tmp/track-a-sentrux-after.txt
```

Expected:

- no inline suppression annotations added to source files or comments
- the plugin-boundary `layer_direction` and `boundary` findings from the baseline are eliminated or reduced to the intentional carve-out
- unrelated `max_*` failures may still remain and do not block Track A

- [ ] **Step 5: Commit**

```bash
git add .sentrux/rules.toml
git commit -m "refactor: align sentrux plugin boundaries with track a"
```

### Task 8: Final Verification And Documentation Audit

**Files:**

- Audit/modify if needed: [`docs/development/plugin-system.md`](/Users/andreyyantsen/Development/uptrakit/docs/development/plugin-system.md)
- Audit/modify if needed: [`docs/development/plugin-guidelines.md`](/Users/andreyyantsen/Development/uptrakit/docs/development/plugin-guidelines.md)

- [ ] **Step 1: Run the source and manifest boundary scans**

Run:

```bash
rg -n 'uptrakit_(plugin_|notification_plugin_core)' crates/ui crates/core crates/shared --glob '*.rs'
rg -n 'uptrakit-plugin-|uptrakit-notification-plugin-core' crates/ui crates/core crates/shared --glob 'Cargo.toml'
rg -F -n '.is_package_manager(' crates/ui crates/core crates/shared
rg -F -n '.display_name(' crates/ui crates/core crates/shared | rg -v 'plugin_ops.display_name'
rg -F -n -e 'starts_with("package_manager_")' -e 'package_manager_' -e 'plugin_ids::' -e 'releases_' -e 'notifications_' -e 'hooks_' -e 'enhancements_' crates/ui crates/core crates/shared
```

Expected:

- source scan only shows `uptrakit_plugin_infrastructure_registry` and allowlisted carve-out `infrastructure-core` references
- manifest scan only shows `uptrakit-plugin-infrastructure-registry` plus `uptrakit-plugin-infrastructure-core` in carve-out manifests; ignore
  `[dev-dependencies]` while triaging results and enforce this only for `[dependencies]`, `[build-dependencies]`, and target-specific non-dev
  dependency tables, matching the Track A spec scope
- helper scans have no production `is_package_manager()` callers and no remaining `PluginTypeId::display_name()` callsites after excluding
  `plugin_ops.display_name`; `plugin_ids::` hits are informational only for Track B and `releases_`/`notifications_`/`hooks_`/`enhancements_` matches
  must be triaged as signals, not treated as standalone failures

- [ ] **Step 2: Run package and workspace verification**

Run:

```bash
cargo check -p uptrakit-web-api
cargo check -p uptrakit-web-api-queries
cargo check -p uptrakit-controller --no-default-features --features db-sqlite
cargo check --no-default-features --features db-sqlite
cargo clippy --all-targets --no-default-features --features db-sqlite
cd frontend && npm ci && npm run build && cd ..
cargo check --all-features
cargo clippy --all-targets --all-features
cargo test --all-features
sentrux check . | tee /tmp/track-a-sentrux-final.txt
rg -n 'layer_direction|boundary' /tmp/track-a-sentrux-final.txt
```

Expected:

- cargo and test commands PASS
- final Sentrux output may still contain unrelated global rule failures
- plugin-boundary `layer_direction` and `boundary` findings are gone except for the documented carve-out

- [ ] **Step 3: Audit and update the docs only if they still describe direct non-registry plugin imports as valid**

Use this replacement direction:

```md
- Non-plugin crates may import plugin crates directly when they need plugin traits.

* Non-plugin crates should import plugin-facing metadata and ops through `uptrakit-plugin-infrastructure-registry`; direct plugin-crate imports are
  reserved for plugin crates and the explicit Track A operational carve-out.
```

- [ ] **Step 4: Commit**

```bash
git add docs/development/plugin-system.md docs/development/plugin-guidelines.md
git commit -m "docs: align plugin boundary guidance with track a"
```

If neither doc changes, skip this commit.

## Self-Review

- Spec coverage: this plan covers the reviewed Track A scope only: registry helper surface, UI/query migrations, helper retirement, controller
  cleanup, carve-out audit, Sentrux rewrite, and final verification/docs.
- Placeholder scan: no `TODO`, `TBD`, or “similar to above” shortcuts remain. The Sentrux task enumerates actual non-carve-out crate roots instead of
  referring to an unspecified exception syntax.
- Type consistency: the plan uses a registry-owned `is_package_manager_plugin()` helper backed by the existing `plugin_ids` package-manager constants
  because `PluginFamily::Software` is too broad and package-manager membership is not currently expressed as a dedicated descriptor field.
