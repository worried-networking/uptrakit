# Track B Plugin Semantic Boundary Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove plugin-specific knowledge from non-plugin production code by moving dashboard-icons onto generic plugin type settings, threading
pre-resolved lifecycle settings through generic dispatch, deleting the bespoke dashboard-icons API surface, and replacing remaining semantic shortcuts
with registry-owned generic queries.

**Architecture:** Keep plugin-specific meaning inside plugin crates plus the registry/catalogue boundary. The application layer preloads generic
type-settings payloads and passes a synchronous lifecycle context into plugin dispatch; lifecycle plugins deserialize and interpret their own
settings. Enforcement lands early through an inventory-backed CI check so later rewrites cannot regress the boundary.

**Tech Stack:** Rust workspace crates (`uptrakit-plugin-infrastructure-core`, `uptrakit-plugin-infrastructure-registry`,
`uptrakit-plugin-enhancement-dashboard-icons`, `uptrakit-web-api`, `uptrakit-web-api-queries`, `uptrakit-shared-types`, `uptrakit-web-api-types`),
Tokio/async-trait, serde/serde_json, axum/utoipa, Svelte settings UI verification, ripgrep-based CI guard.

---

## File Structure

Implementation units for Track B:

- `docs/internal/changes/TASK-0007/ST-0030-track-b-semantic-inventory.md` Responsibility: persisted list of Track B production leak sites, explicit
  allowlist rationale for the temporary CI baseline, and the rewrite/cleanup checklist the CI guard is allowed to tolerate while the branch is in
  flight.
- `ci/check_plugin_semantic_boundary.sh` Responsibility: fail CI when non-plugin production code reintroduces `settings_dashboard_icons`,
  `dashboard_icons.enabled`, `PluginTypeId` semantic helpers, concrete plugin-ID branches/imports, or identity-specific registry helper names outside
  the temporary allowlist.
- `crates/plugins/infrastructure/core/src/roles.rs` Responsibility: define the new pre-resolved lifecycle context type that carries generic
  type-settings payloads into plugin dispatch.
- `crates/plugins/infrastructure/core/src/plugin_ops.rs` Responsibility: change `SoftwareItemLifecycleOps::on_software_item_created(...)` to accept
  lifecycle context and keep the public registry-facing call shape generic.
- `crates/plugins/infrastructure/core/src/catalog.rs` Responsibility: read pre-resolved settings from the lifecycle context, pass them to each
  lifecycle plugin, and merge patches without performing I/O.
- `crates/plugins/infrastructure/core/src/lib.rs` Responsibility: re-export `SoftwareItemLifecycleContext` so downstream crates can import it through
  the public plugin-infrastructure surface.
- `crates/plugins/infrastructure/registry/src/lib.rs` Responsibility: re-export `SoftwareItemLifecycleContext` through the registry crate so web-api
  and query crates do not need a new direct core dependency.
- `crates/plugins/enhancements/dashboard-icons/src/config.rs` Responsibility: change dashboard-icons from a placeholder config into a `TypeSettings`
  implementation with `enabled`.
- `crates/plugins/enhancements/dashboard-icons/src/plugin.rs` Responsibility: read dashboard-icons type settings from lifecycle context, default to
  enabled when settings are absent, and return `None` when explicitly disabled.
- `crates/ui/web-api-queries/src/queries/plugin_type_settings.rs` Responsibility: expose one generic helper that preloads all lifecycle-plugin type
  settings into a `HashMap<PluginTypeId, serde_json::Value>` for a tenant.
- `crates/ui/web-api/src/routes/software_items/mod.rs` Responsibility: preload lifecycle context from generic plugin type settings and pass it into
  `state.plugin_ops.on_software_item_created(...)` instead of calling `is_dashboard_icons_enabled(...)`.
- `crates/ui/web-api/src/routes/service_ws/handler/messages.rs` Responsibility: same lifecycle-context preload for the websocket-created software-item
  path.
- `crates/ui/web-api/src/routes/settings_dashboard_icons.rs` Responsibility: delete this bespoke dashboard-icons settings surface entirely.
- `crates/ui/web-api/src/routes/mod.rs` Responsibility: remove `settings_dashboard_icons` module export.
- `crates/ui/web-api/src/router.rs` Responsibility: remove the bespoke dashboard-icons routes plus `DashboardIconsApiDoc` OpenAPI merge.
- `crates/ui/web-api-auth/src/setting_key.rs` Responsibility: delete `SettingKey::DashboardIconsEnabled`.
- `crates/shared/web-api-types/src/settings_dashboard_icons.rs` Responsibility: delete the dashboard-icons-specific request/response DTOs.
- `crates/shared/web-api-types/src/lib.rs` Responsibility: remove the `settings_dashboard_icons` module export.
- `crates/ui/web-api/db_access_policy.toml` Responsibility: remove the route policy stanza for `settings_dashboard_icons.rs`.
- `crates/shared/types/src/plugin_type_id.rs` Responsibility: delete `PluginTypeId::is_package_manager()` and `PluginTypeId::display_name()` plus
  their tests.
- `crates/plugins/infrastructure/registry/src/registry.rs` Responsibility: back `is_package_manager_plugin(...)` (or its replacement) from descriptor
  data rather than a hardcoded plugin-ID list and add tests that keep the helper domain-generic.
- `crates/ui/web-api-queries/src/queries/update_dispatch.rs` Responsibility: replace the concrete `generic_shell` branch in
  `config_prefers_interactive(...)` with a registry-owned generic predicate.
- `crates/ui/web-api-queries/src/queries/autodiscovery/discovery_items.rs` Responsibility: keep package-manager config creation generic through
  registry answers, not shared-type helper shortcuts.
- `crates/ui/web-api/src/routes/plugin_configs.rs` Responsibility: add tests proving dashboard-icons now exposes type-settings schema/sample through
  the existing generic plugin-types endpoint once the lifecycle caller migration compiles the web-api crate again.
- `frontend/src/routes/settings/PluginConfigsTab.svelte` Responsibility: verify the existing generic type-settings form renders/submits the new
  dashboard-icons boolean field; change only if the current generic rendering path is insufficient.

---

### Task 1: Inventory And Boundary Guard

**Files:**

- Create: `docs/internal/changes/TASK-0007/ST-0030-track-b-semantic-inventory.md`
- Create: `ci/check_plugin_semantic_boundary.sh`
- Modify: `docs/superpowers/plans/2026-04-13-track-b-plugin-semantic-boundary.md`

- [ ] **Step 1: Capture the current Track B production leak inventory**

Create `docs/internal/changes/TASK-0007/ST-0030-track-b-semantic-inventory.md` with these sections and the exact initial entries:

```md
# ST-0030 Track B Semantic Inventory

## Dashboard-icons bespoke surface

- crates/ui/web-api/src/routes/settings_dashboard_icons.rs
- crates/ui/web-api/src/routes/mod.rs
- crates/ui/web-api/src/router.rs
- crates/ui/web-api-auth/src/setting_key.rs
- crates/shared/web-api-types/src/settings_dashboard_icons.rs
- crates/shared/web-api-types/src/lib.rs
- crates/ui/web-api/db_access_policy.toml

## Lifecycle caller pre-checks

- crates/ui/web-api/src/routes/software_items/mod.rs
- crates/ui/web-api/src/routes/service_ws/handler/messages.rs

## Shared semantic shortcuts

- crates/shared/types/src/plugin_type_id.rs

## Known production rewrite sites to verify before branch close

- crates/ui/web-api-queries/src/queries/update_dispatch.rs
- crates/ui/web-api-queries/src/queries/autodiscovery/discovery_items.rs

## Permanent non-production exclusions

- crates/shared/types/src/plugin_type_id.rs (inline `mod tests` uses `plugin_ids::ALL`)
```

- [ ] **Step 2: Write the CI guard with an explicit temporary allowlist**

Create `ci/check_plugin_semantic_boundary.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TARGETS_DASHBOARD=(
  'ui/web-api/src/routes/**/*.rs'
  'ui/web-api/src/router.rs'
  'ui/web-api/src/routes/mod.rs'
  'ui/web-api-auth/src/**/*.rs'
  'shared/web-api-types/src/**/*.rs'
)

TARGETS_HELPERS=(
  'ui/web-api/src/**/*.rs'
  'ui/web-api-queries/src/queries/**/*.rs'
  'shared/types/src/plugin_type_id.rs'
  'plugins/infrastructure/registry/src/**/*.rs'
)

TARGETS_PLUGIN_IDS=(
  'ui/web-api/src/**/*.rs'
  'ui/web-api-queries/src/queries/**/*.rs'
)

PERMANENT_EXCLUSIONS_PLUGIN_IDS=(
  # Inline test modules in production files are allowed to use canonical IDs.
  'shared/types/src/plugin_type_id.rs'
)

ALLOWLIST=()

deny_in() {
  local label="$1"
  local pattern="$2"
  local exclusion_var="$3"
  shift 3
  local files
  local rg_args=()
  local target
  for target in "$@"; do
    rg_args+=(-g "$target")
  done
  if [[ -n "$exclusion_var" ]]; then
    local exclusion_pattern
    eval "for exclusion_pattern in \"\${${exclusion_var}[@]}\"; do rg_args+=(-g \"!\$exclusion_pattern\"); done"
  fi
  for target in "${ALLOWLIST[@]}"; do
    rg_args+=(-g "!$target")
  done
  files="$(rg -n "$pattern" "${rg_args[@]}" crates || true)"
  if [[ -n "$files" ]]; then
    echo "semantic-boundary violation: $label"
    echo "$files"
    exit 1
  fi
}

deny_in "dashboard-icons bespoke surface" 'settings_dashboard_icons|dashboard_icons\.enabled' "" "${TARGETS_DASHBOARD[@]}"
deny_in "PluginTypeId semantic helpers" 'PluginTypeId::is_package_manager|PluginTypeId::display_name|\.is_package_manager\(' "" "${TARGETS_HELPERS[@]}"
deny_in "identity-specific helpers" 'fn is_[a-z0-9_]*dashboard|fn has_[a-z0-9_]*dashboard|is_dashboard_icons|has_dashboard_icons' "" "${TARGETS_HELPERS[@]}"
deny_in "concrete plugin-id imports in non-plugin production code" 'plugin_ids::' "PERMANENT_EXCLUSIONS_PLUGIN_IDS" "${TARGETS_PLUGIN_IDS[@]}"
```

- [ ] **Step 3: Run the guard to prove it fails on the current tree**

Run:

```bash
bash ci/check_plugin_semantic_boundary.sh
```

Expected: FAIL with matches in the bespoke dashboard-icons surface and the current production rewrite sites recorded in
`ST-0030-track-b-semantic-inventory.md`.

- [ ] **Step 4: Add the temporary inventory-backed baseline so the guard can land early**

Update `ci/check_plugin_semantic_boundary.sh` so the deny checks ignore only the files recorded in the inventory while this branch is in progress:

```bash
ALLOWLIST=(
  # Temporary production allowlist entries removed as the branch lands.
  'ui/web-api/src/routes/settings_dashboard_icons.rs'
  'ui/web-api/src/routes/mod.rs'
  'ui/web-api/src/router.rs'
  'ui/web-api-auth/src/setting_key.rs'
  'shared/web-api-types/src/settings_dashboard_icons.rs'
  'shared/web-api-types/src/lib.rs'
  'ui/web-api/db_access_policy.toml'
  'ui/web-api/src/routes/software_items/mod.rs'
  'ui/web-api/src/routes/service_ws/handler/messages.rs'
  'ui/web-api-queries/src/queries/update_dispatch.rs'
  'ui/web-api-queries/src/queries/autodiscovery/discovery_items.rs'
)
```

Use `rg -g '!path'` exclusions for each allowlisted path instead of suppressing entire directory trees.

- [ ] **Step 5: Run the guard again and verify the baseline passes**

Run:

```bash
bash ci/check_plugin_semantic_boundary.sh
```

Expected: PASS with no output.

- [ ] **Step 6: Commit the inventory and guard scaffold**

```bash
git add docs/internal/changes/TASK-0007/ST-0030-track-b-semantic-inventory.md ci/check_plugin_semantic_boundary.sh
git commit -m "chore: add track b semantic boundary inventory"
```

### Task 2: Add The Pre-Resolved Lifecycle Context Seam

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/roles.rs`
- Modify: `crates/plugins/infrastructure/core/src/plugin_ops.rs`
- Modify: `crates/plugins/infrastructure/core/src/catalog.rs`
- Modify: `crates/plugins/infrastructure/core/src/lib.rs`
- Modify: `crates/plugins/infrastructure/registry/src/lib.rs`
- Modify: `crates/plugins/enhancements/dashboard-icons/src/plugin.rs`
- Test: `crates/plugins/infrastructure/core/src/catalog.rs`
- Test: `crates/plugins/enhancements/dashboard-icons/src/plugin.rs`

- [ ] **Step 1: Write failing lifecycle-context tests**

Add one new test in `crates/plugins/infrastructure/core/src/catalog.rs` and one new test in
`crates/plugins/enhancements/dashboard-icons/src/plugin.rs`:

```rust
#[tokio::test]
async fn lifecycle_context_is_forwarded_to_plugins() {
    let mut settings = HashMap::new();
    settings.insert(
        PluginTypeId::from_static("enhancement_dashboard_icons"),
        serde_json::json!({ "enabled": false }),
    );
    let ctx = SoftwareItemLifecycleContext::new(settings);

    let patch = catalog.on_software_item_created(&event, &ctx).await;
    assert!(patch.is_none());
}
```

```rust
#[tokio::test]
async fn explicit_disabled_setting_returns_none() {
    let plugin = make_plugin();
    let event = event("Actual Budget");
    let ctx = SoftwareItemLifecycleContext::new(HashMap::from([(
        PluginTypeId::from_static("enhancement_dashboard_icons"),
        serde_json::json!({ "enabled": false }),
    )]));

    let patch = plugin.on_software_item_created(&event, &ctx).await.unwrap();
    assert!(patch.is_none());
}
```

- [ ] **Step 2: Run the new tests and confirm they fail**

Run:

```bash
cargo test -p uptrakit-plugin-infrastructure-core lifecycle_context_is_forwarded_to_plugins -- --exact
cargo test -p uptrakit-plugin-enhancement-dashboard-icons explicit_disabled_setting_returns_none -- --exact
```

Expected: FAIL because `SoftwareItemLifecycleContext` does not exist and `on_software_item_created(...)` still takes only `event`.

- [ ] **Step 3: Add the lifecycle context type and signature changes**

In `crates/plugins/infrastructure/core/src/roles.rs`, add:

```rust
#[derive(Debug, Clone, Default)]
pub struct SoftwareItemLifecycleContext {
    type_settings: HashMap<PluginTypeId, serde_json::Value>,
}

impl SoftwareItemLifecycleContext {
    pub fn new(type_settings: HashMap<PluginTypeId, serde_json::Value>) -> Self {
        Self { type_settings }
    }

    pub fn type_settings_for(&self, plugin_type: &PluginTypeId) -> Option<&serde_json::Value> {
        self.type_settings.get(plugin_type)
    }
}
```

In `crates/plugins/infrastructure/core/src/roles.rs`, change the per-plugin trait signature to:

```rust
async fn on_software_item_created(
    &self,
    event: &SoftwareItemCreatedEvent,
    ctx: &SoftwareItemLifecycleContext,
) -> std::result::Result<Option<SoftwareItemPatch>, crate::error::PluginError>;
```

In `crates/plugins/infrastructure/core/src/plugin_ops.rs`, change the registry-facing trait signature to:

```rust
fn on_software_item_created<'a>(
    &'a self,
    event: &'a SoftwareItemCreatedEvent,
    ctx: &'a SoftwareItemLifecycleContext,
) -> Pin<Box<dyn Future<Output = Option<SoftwareItemPatch>> + Send + 'a>>;
```

Then re-export the new context from both public surfaces:

```rust
// crates/plugins/infrastructure/core/src/lib.rs
pub use roles::{SoftwareItemCreatedEvent, SoftwareItemLifecycleContext, SoftwareItemPatch};
```

```rust
// crates/plugins/infrastructure/registry/src/lib.rs
// `SoftwareItemLifecycle` is already re-exported here today; keep that export.
pub use uptrakit_plugin_infrastructure_core::SoftwareItemLifecycleContext;
```

- [ ] **Step 4: Thread the lifecycle context through the catalog**

Update `crates/plugins/infrastructure/core/src/catalog.rs`:

```rust
for plugin in &self.lifecycle_plugins {
    match plugin.on_software_item_created(event, ctx).await {
        Ok(Some(patch)) => {
            let merged = merged.get_or_insert_with(SoftwareItemPatch::new);
            if patch.icon_url.is_some() {
                merged.icon_url = patch.icon_url;
            }
        }
        Ok(None) => {}
        Err(e) => {
            tracing::warn!(
                plugin = %plugin.plugin_type_id(),
                error = %e,
                "software item lifecycle plugin error"
            );
        }
    }
}
```

- [ ] **Step 5: Update dashboard-icons to accept the new context parameter**

In `crates/plugins/enhancements/dashboard-icons/src/plugin.rs`, change the method signature now, but keep the old matching logic until Task 3:

```rust
async fn on_software_item_created(
    &self,
    event: &SoftwareItemCreatedEvent,
    ctx: &SoftwareItemLifecycleContext,
) -> std::result::Result<Option<SoftwareItemPatch>, PluginError> {
    let _ = ctx;
    if event.icon_url.is_some() {
        return Ok(None);
    }
    if let Some(url) = self.cache.lookup(&event.name) {
        Ok(Some(SoftwareItemPatch::new().with_icon_url(Some(url))))
    } else {
        Ok(None)
    }
}
```

- [ ] **Step 6: Run the lifecycle seam tests and full crate tests**

Run:

```bash
cargo test -p uptrakit-plugin-infrastructure-core
cargo test -p uptrakit-plugin-enhancement-dashboard-icons
```

Expected: PASS, including the new context-forwarding tests.

- [ ] **Step 7: Commit the lifecycle seam**

```bash
git add crates/plugins/infrastructure/core/src/roles.rs crates/plugins/infrastructure/core/src/plugin_ops.rs crates/plugins/infrastructure/core/src/catalog.rs crates/plugins/infrastructure/core/src/lib.rs crates/plugins/infrastructure/registry/src/lib.rs crates/plugins/enhancements/dashboard-icons/src/plugin.rs
git commit -m "refactor: add lifecycle settings context"
```

### Task 3: Convert Dashboard-Icons To Generic Type Settings

**Files:**

- Modify: `crates/plugins/enhancements/dashboard-icons/src/config.rs`
- Modify: `crates/plugins/enhancements/dashboard-icons/src/plugin.rs`
- Test: `crates/plugins/enhancements/dashboard-icons/src/plugin.rs`
- Test: `crates/plugins/enhancements/dashboard-icons/src/config.rs`

- [ ] **Step 1: Write the failing type-settings tests**

Add this schema/sample test in `crates/plugins/enhancements/dashboard-icons/src/config.rs`:

```rust
#[test]
fn dashboard_icons_type_settings_schema_exposed() {
    let fields = DashboardIconsConfig::type_settings_form_schema();
    let sample = DashboardIconsConfig::type_settings_sample();

    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].key, "enabled");
    assert_eq!(fields[0].field_type, FieldType::Toggle);
    assert_eq!(sample, serde_json::json!({ "enabled": true }));
}
```

Add this lifecycle-defaulting test in `crates/plugins/enhancements/dashboard-icons/src/plugin.rs`:

```rust
#[tokio::test]
async fn unset_settings_default_to_enabled() {
    let plugin = make_plugin();
    let event = event("Actual Budget");
    let ctx = SoftwareItemLifecycleContext::default();

    let patch = plugin.on_software_item_created(&event, &ctx).await.unwrap();
    assert!(patch.is_some());
}
```

- [ ] **Step 2: Run the tests to confirm the current plugin does not expose type settings**

Run:

```bash
cargo test -p uptrakit-plugin-enhancement-dashboard-icons dashboard_icons_type_settings_schema_exposed -- --exact
cargo test -p uptrakit-plugin-enhancement-dashboard-icons unset_settings_default_to_enabled -- --exact
```

Expected: FAIL because `DashboardIconsConfig` is still a placeholder and does not implement the `TypeSettings` trait.

- [ ] **Step 3: Replace the placeholder config with an `enabled` type setting**

In `crates/plugins/enhancements/dashboard-icons/src/config.rs`, replace the placeholder with:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DashboardIconsConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
}

impl DashboardIconsConfig {
    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }
}

impl TypeSettings for DashboardIconsConfig {
    fn type_settings_form_schema() -> Vec<FieldDef> {
        vec![
            FieldDef::new("enabled", "Enabled")
                .with_type(FieldType::Toggle)
                .with_help_text("Enable dashboard icon enrichment for this tenant."),
        ]
    }

    fn type_settings_sample() -> serde_json::Value {
        serde_json::json!({ "enabled": true })
    }
}
```

- [ ] **Step 4: Read dashboard-icons settings from lifecycle context**

Update `crates/plugins/enhancements/dashboard-icons/src/plugin.rs`:

```rust
let enabled = ctx
    .type_settings_for(&self.plugin_type_id())
    .cloned()
    .map(serde_json::from_value::<DashboardIconsConfig>)
    .transpose()?
    .unwrap_or_default()
    .is_enabled();

if !enabled {
    return Ok(None);
}
```

Keep the existing `icon_url` short-circuit and cache lookup logic unchanged after this block.

- [ ] **Step 5: Defer the `uptrakit-web-api` endpoint assertion until Task 4**

Do not run `uptrakit-web-api` tests in this task. Task 2 changed lifecycle method signatures, so the web-api crate will not compile again until Task 4
migrates the web-api caller sites to the new context-aware dispatch.

- [ ] **Step 6: Run the plugin tests**

Run:

```bash
cargo test -p uptrakit-plugin-enhancement-dashboard-icons
```

Expected: PASS, including explicit `enabled=false`, `enabled=true`, and unset-default behavior.

- [ ] **Step 7: Commit the dashboard-icons type-settings conversion**

```bash
git add crates/plugins/enhancements/dashboard-icons/src/config.rs crates/plugins/enhancements/dashboard-icons/src/plugin.rs
git commit -m "feat: move dashboard icons to generic type settings"
```

### Task 4: Migrate Lifecycle Callers And Delete The Bespoke Dashboard-Icons Surface

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/plugin_type_settings.rs`
- Modify: `crates/ui/web-api/src/routes/software_items/mod.rs`
- Modify: `crates/ui/web-api/src/routes/service_ws/handler/messages.rs`
- Modify: `crates/ui/web-api/src/routes/plugin_configs.rs`
- Delete: `crates/ui/web-api/src/routes/settings_dashboard_icons.rs`
- Modify: `crates/ui/web-api/src/routes/mod.rs`
- Modify: `crates/ui/web-api/src/router.rs`
- Modify: `crates/ui/web-api-auth/src/setting_key.rs`
- Delete: `crates/shared/web-api-types/src/settings_dashboard_icons.rs`
- Modify: `crates/shared/web-api-types/src/lib.rs`
- Modify: `crates/ui/web-api/db_access_policy.toml`

- [ ] **Step 1: Write failing lifecycle-context preload tests**

Add tests in `crates/ui/web-api-queries/src/queries/plugin_type_settings.rs` that exercise the new preload helper directly:

```rust
use uptrakit_plugin_infrastructure_registry::{
    PluginError, PluginMeta, PluginTypeId, SoftwareItemCreatedEvent, SoftwareItemLifecycle,
    SoftwareItemLifecycleContext, SoftwareItemPatch,
};
```

```rust
struct FakeLifecyclePlugin {
    plugin_type: PluginTypeId,
}

impl PluginMeta for FakeLifecyclePlugin {
    fn plugin_type_id(&self) -> PluginTypeId {
        self.plugin_type.clone()
    }
}

#[async_trait::async_trait]
impl SoftwareItemLifecycle for FakeLifecyclePlugin {
    async fn on_software_item_created(
        &self,
        _event: &SoftwareItemCreatedEvent,
        _ctx: &SoftwareItemLifecycleContext,
    ) -> std::result::Result<Option<SoftwareItemPatch>, PluginError> {
        Ok(None)
    }
}

#[tokio::test]
async fn lifecycle_context_loads_existing_type_settings_rows() {
    let plugin_type = "enhancement_dashboard_icons";
    upsert_type_settings(&db, tenant_id, plugin_type, serde_json::json!({ "enabled": false }))
        .await
        .unwrap();

    let plugins: Vec<Arc<dyn SoftwareItemLifecycle>> = vec![Arc::new(FakeLifecyclePlugin {
        plugin_type: PluginTypeId::from_static(plugin_type),
    })];
    let ctx = load_software_item_lifecycle_context(&db, tenant_id, &plugins).await.unwrap();

    assert_eq!(
        ctx.type_settings_for(&PluginTypeId::from_static(plugin_type)),
        Some(&serde_json::json!({ "enabled": false }))
    );
}
```

```rust
#[tokio::test]
async fn lifecycle_context_is_empty_when_no_rows_exist() {
    let plugins: Vec<Arc<dyn SoftwareItemLifecycle>> = vec![Arc::new(FakeLifecyclePlugin {
        plugin_type: PluginTypeId::from_static("enhancement_dashboard_icons"),
    })];
    let ctx = load_software_item_lifecycle_context(&db, tenant_id, &plugins).await.unwrap();

    assert!(
        ctx.type_settings_for(&PluginTypeId::from_static("enhancement_dashboard_icons"))
            .is_none()
    );
}
```

- [ ] **Step 2: Run the preload tests and confirm the helper does not exist yet**

Run:

```bash
cargo test -p uptrakit-web-api-queries lifecycle_context_loads_existing_type_settings_rows -- --exact
cargo test -p uptrakit-web-api-queries lifecycle_context_is_empty_when_no_rows_exist -- --exact
```

Expected: FAIL because there is no generic lifecycle-context loader yet.

- [ ] **Step 3: Add a generic lifecycle-context preload helper**

In `crates/ui/web-api-queries/src/queries/plugin_type_settings.rs`, add:

```rust
pub async fn load_software_item_lifecycle_context(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    plugins: &[Arc<dyn SoftwareItemLifecycle>],
) -> Result<SoftwareItemLifecycleContext> {
    let mut type_settings = HashMap::new();

    for plugin in plugins {
        let plugin_type = plugin.plugin_type_id();
        if let Some(model) = get_type_settings(db, tenant_id, plugin_type.as_str()).await? {
            type_settings.insert(plugin_type, model.config);
        }
    }

    Ok(SoftwareItemLifecycleContext::new(type_settings))
}
```

Import `SoftwareItemLifecycle` and `SoftwareItemLifecycleContext` through `uptrakit_plugin_infrastructure_registry`, not through a new direct
dependency on the core crate. This relies on the existing registry re-export of `SoftwareItemLifecycle` plus the new `SoftwareItemLifecycleContext`
re-export added in Task 2.

- [ ] **Step 4: Replace the dashboard-icons pre-checks with lifecycle context loading**

In both `crates/ui/web-api/src/routes/software_items/mod.rs` and `crates/ui/web-api/src/routes/service_ws/handler/messages.rs`, replace the old
branch:

```rust
let lifecycle_ctx = match pts_queries::load_software_item_lifecycle_context(
    tenant_db.db(),
    tenant_db.tenant_id,
    state.plugin_ops.software_item_lifecycle_plugins(),
)
.await
{
    Ok(ctx) => ctx,
    Err(e) => {
        tracing::warn!(error = %e, tenant_id = %tenant_db.tenant_id, "failed to load lifecycle type settings");
        return None;
    }
};

state.plugin_ops.on_software_item_created(&event, &lifecycle_ctx).await
```

and:

```rust
let lifecycle_ctx = match pts_queries::load_software_item_lifecycle_context(
    state.db(),
    tenant_id,
    state.plugin_ops.software_item_lifecycle_plugins(),
)
.await
{
    Ok(ctx) => ctx,
    Err(e) => {
        tracing::warn!(error = %e, %tenant_id, "failed to load lifecycle type settings");
        return;
    }
};

match state.plugin_ops.on_software_item_created(&event, &lifecycle_ctx).await {
```

Delete the `use crate::routes::settings_dashboard_icons::is_dashboard_icons_enabled;` imports entirely. `software_item_lifecycle_plugins()` already
exists on `SoftwareItemLifecycleOps`; do not add a new accessor, just reuse the existing one when building the lifecycle context.

In `crates/ui/web-api/src/routes/plugin_configs.rs`, add the endpoint assertion deferred from Task 3:

```rust
assert_eq!(dashboard_icons.display_name, "Dashboard Icons");
assert_eq!(dashboard_icons.type_settings_form_fields.len(), 1);
assert_eq!(dashboard_icons.type_settings_form_fields[0].key, "enabled");
assert_eq!(dashboard_icons.type_settings_form_fields[0].field_type, FieldType::Toggle);
assert_eq!(dashboard_icons.type_settings_sample, serde_json::json!({ "enabled": true }));
```

- [ ] **Step 5: Delete the bespoke dashboard-icons surface**

Remove these items in one changeset:

```text
Delete crates/ui/web-api/src/routes/settings_dashboard_icons.rs
Delete crates/shared/web-api-types/src/settings_dashboard_icons.rs
Remove pub mod settings_dashboard_icons from crates/ui/web-api/src/routes/mod.rs
Remove pub mod settings_dashboard_icons from crates/shared/web-api-types/src/lib.rs
Remove SettingKey::DashboardIconsEnabled from crates/ui/web-api-auth/src/setting_key.rs
Remove [routes."settings_dashboard_icons.rs"] from crates/ui/web-api/db_access_policy.toml
Remove get_dashboard_icons_settings / update_dashboard_icons_settings and DashboardIconsApiDoc wiring from crates/ui/web-api/src/router.rs
```

- [ ] **Step 6: Run the web-api verification sweep**

Run:

```bash
cargo test -p uptrakit-web-api --features dashboard-icons,db-sqlite
cargo test -p uptrakit-web-api-queries
python3 ci/verify_db_access_policy.py
rg -n 'settings_dashboard_icons|dashboard_icons\.enabled|DashboardIconsEnabled' crates frontend
```

Expected: tests PASS, DB access policy verifier PASS, and the final `rg` returns no production-code matches.

- [ ] **Step 7: Commit the caller migration and endpoint deletion**

```bash
git add crates/ui/web-api-queries/src/queries/plugin_type_settings.rs crates/ui/web-api/src/routes/software_items/mod.rs crates/ui/web-api/src/routes/service_ws/handler/messages.rs crates/ui/web-api/src/routes/plugin_configs.rs crates/ui/web-api/src/routes/mod.rs crates/ui/web-api/src/router.rs crates/ui/web-api-auth/src/setting_key.rs crates/shared/web-api-types/src/lib.rs crates/ui/web-api/db_access_policy.toml
git add -u crates/ui/web-api/src/routes/settings_dashboard_icons.rs crates/shared/web-api-types/src/settings_dashboard_icons.rs
git commit -m "refactor: remove bespoke dashboard icons settings api"
```

### Task 5: Remove Shared Semantic Shortcuts And Rewrite Remaining Production Branches

**Files:**

- Modify: `crates/shared/types/src/plugin_type_id.rs`
- Modify: `crates/plugins/infrastructure/registry/src/registry.rs`
- Modify: `crates/ui/web-api-queries/src/queries/update_dispatch.rs`
- Modify: `crates/ui/web-api-queries/src/queries/autodiscovery/discovery_items.rs`
- Modify: `docs/internal/changes/TASK-0007/ST-0030-track-b-semantic-inventory.md`
- Modify: `ci/check_plugin_semantic_boundary.sh`

- [ ] **Step 1: Write failing helper-removal tests**

Reuse the existing `package_manager_lookup_covers_all_current_package_managers` test in `crates/plugins/infrastructure/registry/src/registry.rs` as
the regression anchor for the current package-manager set, and add a new test for the interactive-dispatch helper:

```rust
#[test]
fn interactive_dispatch_preference_helper_matches_generic_shell_schema() {
    assert!(supports_interactive_dispatch_preference(&plugin_ids::GENERIC_SHELL));
    assert!(!supports_interactive_dispatch_preference(&plugin_ids::RELEASES_DOCKER));
}
```

Add a compile-oriented removal test by deleting the `PluginTypeId` helper tests that mention `is_package_manager()` and `display_name()` and replacing
them with registry lookups in the relevant caller tests.

Before deleting either helper, inventory the remaining callers across non-plugin production code:

```bash
rg -n --glob '!crates/plugins/**' --glob '!docs/**' '\.display_name\(\)|\.is_package_manager\(\)' crates
rg -n --glob '!crates/plugins/**' --glob '!docs/**' 'plugin_ids::' crates
```

Update `docs/internal/changes/TASK-0007/ST-0030-track-b-semantic-inventory.md` with any additional non-test production hits these commands reveal
before proceeding with deletion.

- [ ] **Step 2: Run the focused tests and confirm the current helper is still hardcoded**

Run:

```bash
cargo test -p uptrakit-plugin-infrastructure-registry interactive_dispatch_preference_helper_matches_generic_shell_schema -- --exact
```

Expected: FAIL because `supports_interactive_dispatch_preference(...)` does not exist yet.

- [ ] **Step 3: Replace shared helpers with registry-backed answers**

In `crates/shared/types/src/plugin_type_id.rs`, delete:

```rust
pub fn is_package_manager(&self) -> bool {
    self.0.starts_with("package_manager_")
}

pub fn display_name(&self) -> &str {
    match self.0.as_ref() {
        "releases_github" => "GitHub Releases",
        "releases_gitlab" => "GitLab Releases",
        "releases_forgejo" => "Forgejo Releases",
        "releases_docker" => "Docker",
        "discovery_proxmox_helper_scripts" => "Proxmox Helper Scripts",
        "package_manager_homebrew" => "Homebrew",
        "package_manager_apt" => "APT",
        "package_manager_dnf" => "DNF",
        "package_manager_npm" => "npm",
        "package_manager_mas" => "Mac App Store",
        "package_manager_pacman" => "Pacman",
        "package_manager_pkg" => "FreeBSD pkg",
        "package_manager_apk" => "Alpine APK",
        "package_manager_snap" => "Snap",
        "package_manager_cargo" => "Cargo",
        "generic_shell" => "Shell",
        "hook_shell" => "Shell Hook",
        "hook_systemd" => "Systemd Hook",
        "infrastructure_proxmox" => "Proxmox VE",
        "webhook" => "Webhook",
        "telegram" => "Telegram",
        "email" => "Email",
        "enhancement_dashboard_icons" => "Dashboard Icons",
        other => other,
    }
}
```

In `crates/plugins/infrastructure/registry/src/registry.rs`, keep the existing `is_package_manager_plugin(...)` behavior pinned by tests, and add a
new generic helper for the update-dispatch rewrite:

```rust
pub fn supports_interactive_dispatch_preference(plugin_type_id: &PluginTypeId) -> bool {
    get_descriptor(plugin_type_id.as_str()).is_some_and(|descriptor| {
        (descriptor.config.form_schema)()
            .iter()
            .any(|field| field.key == "prefer_interactive")
    })
}
```

- [ ] **Step 4: Rewrite the remaining production branch sites from the inventory**

In `crates/ui/web-api-queries/src/queries/update_dispatch.rs`, replace the concrete generic-shell check:

```rust
pub(crate) fn config_prefers_interactive(
    plugin_type: &uptrakit_internal_wire::PluginTypeId,
    config: &serde_json::Value,
) -> bool {
    supports_interactive_dispatch_preference(plugin_type)
        && config.get("prefer_interactive").and_then(|v| v.as_bool()).unwrap_or(false)
}
```

In `crates/ui/web-api-queries/src/queries/autodiscovery/discovery_items.rs`, keep the package-manager distinction expressed through the registry
helper only:

```rust
let pc_id = if is_package_manager_plugin(&target.plugin_type) {
    None
} else {
    Some(
        find_or_create_default_plugin_config(
            db,
            tenant_id,
            &target_plugin_type_str,
            &target.plugin_config,
            &target.plugin_config_name,
        )
        .await?,
    )
};
```

Do not reintroduce `plugin_ids::...` imports or prefix checks in this file while making adjacent edits.

- [ ] **Step 5: Tighten the inventory and CI allowlist**

Update `docs/internal/changes/TASK-0007/ST-0030-track-b-semantic-inventory.md` so completed files move to a `Resolved` section, then remove those
paths from `ci/check_plugin_semantic_boundary.sh`:

```md
## Resolved In Branch

- crates/ui/web-api/src/routes/settings_dashboard_icons.rs
- crates/ui/web-api/src/routes/software_items/mod.rs
- crates/ui/web-api/src/routes/service_ws/handler/messages.rs
- crates/shared/types/src/plugin_type_id.rs
```

The CI guard should only keep unresolved paths allowlisted at the end of this task. At the same time, expand the guard target globs from the initial
`web-api`/`web-api-queries` subset to every non-plugin production crate path confirmed by the updated inventory, while keeping the documented
non-production exclusions tight and explicit.

- [ ] **Step 6: Run the registry/query/guard verification**

Run:

```bash
cargo test -p uptrakit-shared-types
cargo test -p uptrakit-plugin-infrastructure-registry
cargo test -p uptrakit-web-api-queries
bash ci/check_plugin_semantic_boundary.sh
```

Expected: PASS, with the guard either fully green or reduced to the final unresolved allowlist that matches the inventory exactly.

- [ ] **Step 7: Commit the helper and branch cleanup**

```bash
git add crates/shared/types/src/plugin_type_id.rs crates/plugins/infrastructure/registry/src/registry.rs crates/ui/web-api-queries/src/queries/update_dispatch.rs crates/ui/web-api-queries/src/queries/autodiscovery/discovery_items.rs docs/internal/changes/TASK-0007/ST-0030-track-b-semantic-inventory.md ci/check_plugin_semantic_boundary.sh
git commit -m "refactor: remove track b semantic shortcuts"
```

### Task 6: Final Verification And Branch Closeout

**Files:**

- Modify: `docs/internal/changes/TASK-0007/ST-0030-track-b-semantic-inventory.md`
- Modify: `ci/check_plugin_semantic_boundary.sh`
- Verify: `frontend/src/routes/settings/PluginConfigsTab.svelte`

- [ ] **Step 1: Remove the temporary production allowlist**

Edit `ci/check_plugin_semantic_boundary.sh` so the final form has no temporary Track B production allowlist entries left. Permanent non-production
exclusions for inline-test containers may remain documented.

```bash
ALLOWLIST=()
PERMANENT_EXCLUSIONS_PLUGIN_IDS=(
  'crates/shared/types/src/plugin_type_id.rs'
)
```

Update the inventory doc to end with:

```md
## Final State

- No temporary Track B production allowlist entries remain.
- Only documented non-production exclusions remain.
- The CI guard is expected to fail on any future reintroduction.
```

- [ ] **Step 2: Run the full verification sweep**

Run:

```bash
cargo fmt --all
cargo test -p uptrakit-plugin-infrastructure-core
cargo test -p uptrakit-plugin-enhancement-dashboard-icons
cargo test -p uptrakit-plugin-infrastructure-registry
cargo test -p uptrakit-web-api --features dashboard-icons,db-sqlite
cargo test -p uptrakit-web-api-queries
cargo test -p uptrakit-shared-types
python3 ci/verify_db_access_policy.py
bash ci/check_plugin_semantic_boundary.sh
```

Expected: every command PASS.

- [ ] **Step 3: Verify the generic settings UI path without bespoke frontend code**

Run:

```bash
cd frontend
npm run check
npm run lint
```

Then manually verify in the settings UI that the dashboard-icons plugin type row exposes an `enabled` toggle through the existing generic
type-settings editor and that saving/deleting the value round-trips through `/plugin-type-settings/{plugin_type}` rather than any bespoke
dashboard-icons endpoint. The generic path is insufficient only if a `FieldType::Toggle` renders incorrectly or the value cannot be saved/deleted
through the existing generic plugin type settings flow without bespoke UI work.

- [ ] **Step 4: Final boundary proof**

Run:

```bash
bash ci/check_plugin_semantic_boundary.sh
rg -n 'settings_dashboard_icons|dashboard_icons\.enabled|DashboardIconsEnabled' crates frontend
```

Expected: the boundary guard passes, and the focused `rg` returns no production-code matches for the deleted dashboard-icons surface.

- [ ] **Step 5: Commit the final guard tightening**

```bash
git add docs/internal/changes/TASK-0007/ST-0030-track-b-semantic-inventory.md ci/check_plugin_semantic_boundary.sh
git commit -m "test: enforce track b semantic boundary"
```

## Self-Review

### Spec Coverage

- Boundary rule: covered by Task 1 guard, Task 5 helper/branch rewrites, and Task 6 final grep/guard tightening.
- Dashboard-icons generic type settings: covered by Task 3.
- Lifecycle pre-resolved context: covered by Task 2 and Task 4.
- Delete bespoke dashboard-icons API surface: covered by Task 4.
- Preserve generic plugin-type settings UI path: covered by Task 4 and Task 6 UI verification.
- Persisted rewrite inventory artifact tied to CI: covered by Task 1, Task 5, and Task 6.

No spec gaps remain in the plan.

### Placeholder Scan

- No `TODO`, `TBD`, `implement later`, or “similar to Task N” placeholders remain.
- Every task names exact files and exact verification commands.
- Every code-changing step includes the concrete symbol or snippet to add/change/delete.

### Type Consistency

- `SoftwareItemLifecycleContext` is introduced in Task 2 and reused consistently in Tasks 3 and 4.
- Dashboard-icons type settings use `DashboardIconsConfig { enabled: Option<bool> }` consistently across config, plugin logic, and UI assertions.
- The CI guard and inventory artifact names remain stable from Task 1 through Task 6.
