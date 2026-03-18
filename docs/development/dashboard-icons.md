# Dashboard Icons Plugin

Development guide for the Dashboard Icons enhancement plugin. This document covers the architecture,
crate layout, lifecycle dispatch flow, and integration points.

## Overview

Dashboard Icons is an enhancement plugin that automatically assigns icon URLs to software items by
looking up their names in the [homarr-labs/dashboard-icons](https://github.com/homarr-labs/dashboard-icons)
community repository. When a software item is created (manually or via autodiscovery) and has no
existing `icon_url`, the plugin converts the item name to a slug, checks whether a matching SVG
icon exists in the cached index, and returns a CDN URL pointing to that icon.

The plugin crate lives at `crates/plugins/enhancements/dashboard-icons/`.

## Architecture

The plugin implements the `SoftwareItemLifecycle` role trait, defined in
`uptrakit-plugin-infrastructure-core`. This role trait provides a hook that fires after a software
item is created, allowing enhancement plugins to inspect the item and return a patch.

Key types (all defined in `crates/plugins/infrastructure/core/src/roles.rs`):

- **`SoftwareItemLifecycle`** -- async role trait with `on_software_item_created(&self, event) -> Result<Option<SoftwareItemPatch>>`.
- **`SoftwareItemCreatedEvent`** -- `#[non_exhaustive]` struct carrying the item snapshot (`id`, `tenant_id`, `name`,
  `featured`, `icon_url`). Constructed via `::new()`.
- **`SoftwareItemPatch`** -- `#[non_exhaustive]` struct with `Option<Option<T>>` fields. `Some(Some(url))` = set,
  `Some(None)` = clear, `None` = no change. Constructed via `::new()` / `::default()` with builder methods.
- **`PluginCapability::SoftwareItemLifecycle`** -- capability variant declared by plugins that implement the role trait.

The plugin is declared via `declare_plugin!` with the `SoftwareItemLifecycle` role and provides a
`create_software_item_lifecycle` factory function. The `PluginCatalog` collects
`SoftwareItemLifecycle` implementations directly via `CreateEnhancementFn` during catalog
construction -- no downcast is needed.

## Components

### DashboardIconCache

`crates/plugins/enhancements/dashboard-icons/src/cache.rs`

Holds a `parking_lot::RwLock<HashSet<String>>` of known icon slugs. The cache is populated by
fetching the GitHub Trees API for the `homarr-labs/dashboard-icons` repository at startup and
then refreshed every 6 hours via a background `tokio::spawn` loop.

Refresh flow:

1. `GET https://api.github.com/repos/homarr-labs/dashboard-icons/git/trees/main?recursive=1`
2. Parse the `tree` array from the JSON response.
3. Extract slugs by matching paths of the form `svg/<slug>-light.svg` (light variants only to avoid
   duplicates).
4. Replace the entire `HashSet` under a write lock.

The `lookup(name)` method slugifies the input, checks the set under a read lock, and returns the
CDN URL if a match is found:

```text
https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/svg/{slug}.svg
```

The background refresh loop uses `CancellationToken` for graceful shutdown and logs errors at
`warn` level without crashing.

### DashboardIconsPlugin

`crates/plugins/enhancements/dashboard-icons/src/plugin.rs`

Declared via `declare_plugin!` with `PluginDescriptor` (`plugin_type_id = "enhancement_dashboard_icons"`,
`family: PluginFamily::Enhancement`) and implements the `SoftwareItemLifecycle` role:

```rust
declare_plugin!(DashboardIconsPlugin, DashboardIconsConfig, "enhancement_dashboard_icons", {
    family: PluginFamily::Enhancement,
    // ...
    roles: [SoftwareItemLifecycle],
});
```

The plugin provides a `create_software_item_lifecycle` function that the `PluginCatalog` calls
during construction to obtain an `Arc<dyn SoftwareItemLifecycle>` singleton.

The `on_software_item_created` implementation:

1. If `event.icon_url` is already `Some`, return `Ok(None)` (do not overwrite).
2. Call `cache.lookup(&event.name)`.
3. If a URL is returned, wrap it in `SoftwareItemPatch::new().with_icon_url(Some(url))`.
4. Otherwise return `Ok(None)`.

### Slugify module

`crates/plugins/enhancements/dashboard-icons/src/slugify.rs`

Converts a software item name to the Dashboard Icons slug format:

- Lowercase the input.
- Replace whitespace, underscores, and hyphens with a single hyphen.
- Collapse consecutive hyphens.
- Strip leading and trailing hyphens.
- Remove all characters that are not ASCII alphanumeric or hyphens.

Examples: `"Home Assistant"` becomes `home-assistant`, `"Node.js"` becomes `nodejs`,
`"PostgreSQL 16"` becomes `postgresql-16`.

### Error types

`crates/plugins/enhancements/dashboard-icons/src/error.rs`

`DashboardIconsError` with two variants:

- `IndexFetch(String)` -- HTTP request to fetch the icon index failed.
- `IndexParse(String)` -- failed to parse the icon index response.

Uses `rootcause::Report<DashboardIconsError>` with a local `Result<T>` alias.

## Feature flag

The `dashboard-icons` feature flag propagates through three crates:

```text
controller/Cargo.toml              web-api/Cargo.toml                         plugin-infrastructure-registry/Cargo.toml
  dashboard-icons  ---------->       dashboard-icons  ---------------------->   dashboard-icons
  (also: dep:uptrakit-plugin-                                                   (dep:uptrakit-plugin-enhancement-dashboard-icons)
   enhancement-dashboard-icons)
```

The feature is **enabled by default** in the controller's `default` feature set, alongside
`db-sqlite`, `oidc`, `zeroconf`, `interactive`, `notifications-all`, and others.

## Per-tenant setting

The feature is gated by a per-tenant setting:

| Setting key | `SettingKey` variant | DB key | Default |
| --- | --- | --- | --- |
| Dashboard Icons enabled | `SettingKey::DashboardIconsEnabled` | `dashboard_icons.enabled` | `true` (when unset) |

This is **not** a global setting (`is_global()` returns `false`). Each tenant can explicitly disable
it. The setting is read via `load_setting()` and written via `upsert_setting()` from
`uptrakit-web-api-auth`'s settings store.

### Settings API

| Method | Path | Permission | Description |
| --- | --- | --- | --- |
| `GET` | `/api/v1/settings/dashboard-icons` | `view_settings` | Returns `{ "enabled": bool }` |
| `PUT` | `/api/v1/settings/dashboard-icons` | `manage_global_settings` | Accepts `{ "enabled": bool }`, returns updated state |

Request/response types are in `crates/shared/web-api-types/src/settings_dashboard_icons.rs`.
The `UpdateDashboardIconsSettingsRequest` implements `Validate` (always succeeds since the only
field is a bool).

## Hook points

The lifecycle dispatch fires in two places:

### Manual software item creation

`crates/ui/web-api/src/routes/software_items/mod.rs` -- `fire_software_item_lifecycle()`

After a successful `POST /api/v1/software-items`, the handler calls `fire_software_item_lifecycle()`,
which:

1. Resolves the effective `SettingKey::DashboardIconsEnabled` value for the tenant. Returns `None` if explicitly disabled.
2. Builds a `SoftwareItemCreatedEvent` from the response.
3. Calls `state.plugin_ops.on_software_item_created(&event)`.
4. Returns the merged `SoftwareItemPatch`, which the caller applies to the DB.

### Autodiscovery

`crates/ui/web-api/src/routes/service_ws/handler/messages.rs` -- after `process_discovery_results()`

After discovery results are processed and new software items are persisted:

1. Check the effective `SettingKey::DashboardIconsEnabled` value for the tenant. Return early if explicitly disabled.
2. Call `load_items_needing_enrichment(db, tenant_id)` to find featured, active items with no `icon_url`.
3. For each item, build a `SoftwareItemCreatedEvent` and call `plugin_ops.on_software_item_created()`.
4. Apply each returned `SoftwareItemPatch` to the DB via `apply_software_item_patch()`.

## Lifecycle dispatch flow

```text
Request arrives (POST /software-items or discovery_results)
  |
  v
Check SettingKey::DashboardIconsEnabled for the tenant
  |-- disabled --> skip
  |-- enabled -->
      |
      v
  Build SoftwareItemCreatedEvent { id, tenant_id, name, featured, icon_url }
      |
      v
  plugin_ops.on_software_item_created(&event)
      |
      v
  PluginCatalog iterates lifecycle_plugins
      |-- for each plugin: on_software_item_created()
      |-- merge patches (last writer wins per field)
      |
      v
  Return Option<SoftwareItemPatch>
      |
      v
  Apply patch to DB (update icon_url on software_item row)
```

## Catalog integration

The `PluginCatalog` stores lifecycle plugins in `lifecycle_plugins: Vec<Arc<dyn SoftwareItemLifecycle>>`.
Registration happens automatically during catalog construction: the `declare_plugin!` macro
registers a `CreateEnhancementFn` that the catalog calls to obtain the `SoftwareItemLifecycle`
singleton. No separate builder method is needed -- the plugin is instantiated as part of catalog
setup when the `dashboard-icons` feature is enabled.

The `on_software_item_created()` method on `PluginOps` iterates all lifecycle plugins, collects
patches, and merges them with a last-writer-wins strategy per field. Errors from individual plugins
are logged at `warn` level and do not prevent other plugins from running.

## Key files

| File | Purpose |
| --- | --- |
| `crates/plugins/enhancements/dashboard-icons/src/lib.rs` | Crate root, re-exports `DashboardIconCache` and `DashboardIconsPlugin` |
| `crates/plugins/enhancements/dashboard-icons/src/plugin.rs` | `DashboardIconsPlugin` declared via `declare_plugin!` with `SoftwareItemLifecycle` role |
| `crates/plugins/enhancements/dashboard-icons/src/cache.rs` | `DashboardIconCache` with refresh loop and CDN URL construction |
| `crates/plugins/enhancements/dashboard-icons/src/slugify.rs` | Name-to-slug conversion function |
| `crates/plugins/enhancements/dashboard-icons/src/error.rs` | `DashboardIconsError` enum |
| `crates/plugins/infrastructure/core/src/roles.rs` | `SoftwareItemLifecycle` role trait, `SoftwareItemCreatedEvent`, `SoftwareItemPatch` |
| `crates/plugins/infrastructure/core/src/plugin_ops.rs` | `PluginOps::on_software_item_created()` default impl |
| `crates/plugins/infrastructure/registry/src/registry.rs` | `PluginCatalog` construction, lifecycle dispatch impl |
| `crates/plugins/infrastructure/registry/src/lib.rs` | `PluginOps::on_software_item_created()` override with merge logic |
| `crates/ui/web-api/src/routes/software_items/mod.rs` | `fire_software_item_lifecycle()` for manual creation |
| `crates/ui/web-api/src/routes/service_ws/handler/messages.rs` | Post-autodiscovery enrichment loop |
| `crates/ui/web-api/src/routes/settings_dashboard_icons.rs` | `GET`/`PUT` `/api/v1/settings/dashboard-icons` handlers |
| `crates/shared/web-api-types/src/settings_dashboard_icons.rs` | Request/response types and `Validate` impl |
| `crates/ui/web-api-auth/src/setting_key.rs` | `SettingKey::DashboardIconsEnabled` definition |

## Testing

- **Unit tests** in `plugin.rs`: verifies icon assignment when a match is found, no-op when icon is already set, no-op
  when no match exists. Uses a pre-populated cache (`new_with_slugs` behind `#[cfg(test)]`).
- **Unit tests** in `cache.rs`: verifies `lookup()` for known slugs, unknown slugs, empty names, and names with spaces.
- **Unit tests** in `slugify.rs`: covers simple names, spaces, special characters, underscores, mixed case, leading/trailing
  hyphens, consecutive separators, empty/whitespace input, and numbers.
- **Serde round-trip tests** in `settings_dashboard_icons.rs`: verifies request/response serialization and `Validate` impl.
- No `start_paused = true` needed -- no tests use tokio time APIs.

## Cross-references

- [Plugin System Architecture](plugin-system.md)
- [Plugin Guidelines](plugin-guidelines.md)
- [Dashboard Icons End-User Guide](../end-user/dashboard-icons.md)
- [Autodiscovery](../end-user/autodiscovery.md)
- [Coding Standards](coding-standards.md)
