# Proxmox VE Hosts Surface — Restoration Design

**Date:** 2026-04-22

## Overview

The `proxmox.hosts` shared surface was intentionally degraded during the Surfaces migration because it
depended on "context selector/add-action semantics plus row data" not yet available in the shared-surface
slice. The surface currently shows only a callout explaining the limitation and an embedded Add
Configuration form.

This spec restores the page to full functionality by:

1. Introducing a generic `context_selector` extension to `SurfaceDescriptor` — a dropdown rendered above
   the surface content that drives `baseParams` for all child nodes and interactions.
2. Replacing the degraded `proxmox_hosts_selector_boundary_surface` with a full table surface that uses
   this selector.
3. Modifying `handle_list` to make `plugin_config_id` optional so the table shows all clusters' guests
   by default ("All Configurations" selection).

All action handlers (`list`, `discover`, `test-connection`, `match`, `approve-match`, `unmatch`) already
exist and are unchanged except for `handle_list`.

## Background: why it was degraded

`handle_list` requires `plugin_config_id` in params to know which Proxmox cluster to show. The
`SLOT_SURFACE_PAGE` generic surface page passes no `baseParams`, so the table had no source for that
param. The surface was degraded rather than ship a broken table.

## Design decisions

**Q1 — Selector scope: filter table AND gate Discover/Test.**

The context selector drives `effectiveBaseParams` for the entire surface:

- "All Configurations" (default) → no `plugin_config_id` in params → table shows all mappings; Discover
  and Test Connection buttons disabled with tooltip.
- Specific config selected → `plugin_config_id` present in params → table filtered; buttons enabled.

This is the only option that gives coherent UX: the user's cluster selection is visible and affects both
the data view and the action targets at once.

**Q2 — Context selector placement: `SurfaceReadPanel`, not `SurfaceDescriptor` root node.**

The selector is declared in the descriptor (`context_selector` field) but rendered by `SurfaceReadPanel`
as a native Svelte component above the surface content. It manages internal state and merges the selection
into `baseParams` before passing to `SurfaceRenderer`.

Alternatives rejected:

- New `SurfaceNode::Selector` type: requires implementing full node serialisation, renderer branch, and
  interaction-disabled wiring in `SurfaceInteractionButton` — more surface than needed.
- URL-based `baseParams` (page reads `?plugin_config_id=xxx`): doesn't support in-surface state changes
  without navigation; forces an external entry point rather than a self-contained surface.

**Q3 — "All" default with optional `plugin_config_id` in `handle_list`.**

`handle_list` changes `parse_uuid_param(&params, "plugin_config_id")?` to an optional parse. When absent:
query joins `plugin_config` on `proxmox_host_mapping.plugin_config_id` to attach `config_name` and
`plugin_config_id` to every row. When present: existing per-config query (unchanged behaviour).

Row data always includes `plugin_config_id` so row actions (`match`, `approve-match`, `unmatch`) receive
it via `rowParams(row)` — no handler changes required for those.

**Q4 — Discover/Test Connection disabled state via `required_for_interactions`.**

`SurfaceContextSelectorDescriptor` carries a `required_for_interactions: Vec<String>` list. When an
interaction's ID is in this list and `effectiveBaseParams[param_key]` is absent, `SurfaceInteractionButton`
renders the button disabled with a fixed tooltip: "Select a configuration first."

This is a display-only behaviour change in `SurfaceInteractionButton`. No new interaction kind or backend
change needed.

**Q5 — `match` visible_when.**

`match` (Manual Match) is available on every row regardless of whether it already has a matched host —
re-matching an already-matched host replaces the match. `unmatch` is gated behind
`visible_when: { field: "matched_host", condition: "present" }`. `approve-match` is gated behind
`visible_when: { field: "suggested_host_id", condition: "present" }` (unchanged from existing action
definition).

**Q6 — `add-config` removed from this surface.**

"Add Configuration" is a plugin config management concern, handled by Settings → Plugin Configs tab.
Duplicating it here was a stopgap from the degraded surface design. Not restored.

**Q7 — New `ContextSelector` capability.**

A new `Capability::ContextSelector` is added to the enum. Proxmox hosts surface declares it. This
allows the capability gate in `surface_registry.rs` to enforce that the frontend version supports the
feature before activating the surface.

## Goals

1. `proxmox.hosts` surface renders a fully functional host-mappings table (all clusters by default,
   filtered when a config is selected).
2. Config selector drives `baseParams` for the table and for Discover/Test Connection.
3. Discover and Test Connection are disabled with tooltip when "All" is selected; enabled otherwise.
4. Row actions (approve-match, match, unmatch) function without change to their handlers.
5. `context_selector` is a generic surface framework feature usable by future surfaces.

## Non-goals

- Add Configuration on the Proxmox Hosts surface — Settings → Plugin Configs tab.
- Dedicated Proxmox settings surface for config management — separate spec.
- Sorting / filtering on the mappings table — not in scope.
- `list-all-unmatched` action — not surfaced on this page.
- Any change to `handle_match`, `handle_approve_match`, `handle_unmatch`, `handle_discover`,
  `handle_test_connection` logic (only their action definitions gain `form_ui` or `required_for_interactions`
  membership, not their handler logic).

## Scope

### `crates/shared/surfaces/src/surface.rs`

**`#[non_exhaustive]` on existing types:** `SurfaceDescriptor` and `Capability` currently lack
`#[non_exhaustive]`. Per project standards both extensible public structs and extensible public enums carry
it. Add `#[non_exhaustive]` to both as part of this change. External match sites on `Capability` use
iterator-based checks (not match arms), so no wildcard arm changes are needed by callers.

Add to `Capability` enum:

```rust
ContextSelector,
```

Add new struct (with `#[non_exhaustive]`, `Serialize`, `Deserialize`, `Debug`, `Clone`, `PartialEq`, `Eq`):

```rust
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceContextSelectorDescriptor {
    /// Param key injected into `baseParams` when a specific option is selected.
    pub param_key: String,
    /// Label shown above the selector dropdown.
    pub label: String,
    /// Label for the "show all" option (no param injected).
    pub all_option_label: String,
    /// REST API path returning a JSON array or paginated `items` list.
    pub rest_api_path: String,
    /// Field in each item used as the option value.
    pub value_field: String,
    /// Field in each item used as the option label.
    pub label_field: String,
    /// Interaction IDs disabled (with tooltip) when no specific option is selected.
    #[serde(default)]
    pub required_for_interactions: Vec<String>,
}
```

Add to `SurfaceDescriptor`:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub context_selector: Option<SurfaceContextSelectorDescriptor>,
```

### `frontend/src/lib/surfaces/contract.ts`

Add interface:

```typescript
export interface SurfaceContextSelector {
    param_key: string;
    label: string;
    all_option_label: string;
    rest_api_path: string;
    value_field: string;
    label_field: string;
    required_for_interactions: string[];
}
```

Add to `SurfaceDescriptor`:

```typescript
context_selector?: SurfaceContextSelector;
```

Add to `SurfaceCapability` union:

```typescript
| 'context_selector'
```

### `frontend/src/lib/components/surfaces/SurfaceReadPanel.svelte`

When `descriptor.context_selector` is present:

1. On mount, fetch `context_selector.rest_api_path` via the standard API client. Parse response as
   `items[]` (paginated) or as array directly. Populate a local `selectorOptions: { value: string; label:
   string }[]` array.
2. Manage `selectedContextValue = $state<string>("")`.
3. Build providers array by prepending the "All" option:

   ```svelte
   const contextSelectorProviders = $derived([
       { id: '', label: descriptor.context_selector!.all_option_label },
       ...selectorOptions.map((opt) => ({ id: opt.value, label: opt.label }))
   ]);
   ```

   Render `ProviderSelector` (already used in `SurfaceReadPanel` for the targeted-provider dropdown)
   above the surface content, wrapped in `<div class="mb-4 max-w-[280px]">` to match the existing
   targeted-provider selector layout. `ProviderSelector` accepts `{ id, label }` objects — the "All"
   option gets `id: ""`. Pass `label={descriptor.context_selector!.label}`,
   `selectedId={selectedContextValue}`, and
   `onSelect={(id) => { selectedContextValue = id; }}`.

4. Derive `effectiveBaseParams` and update `baseParamsFingerprint` to use it:

   ```svelte
   const effectiveBaseParams = $derived(
       selectedContextValue
           ? { ...baseParams, [descriptor.context_selector!.param_key]: selectedContextValue }
           : { ...baseParams }
   );
   ```

   **Critical:** `baseParamsFingerprint` is currently derived from `baseParams`. Change the source to
   `effectiveBaseParams` so that selector changes propagate into the hydration fingerprint:

   ```svelte
   // Replace:
   // const baseParamsFingerprint = $derived(stableStringify(baseParams));
   const baseParamsFingerprint = $derived(stableStringify(effectiveBaseParams));
   ```

   Without this change, `hydrationFingerprint` will not react to selector changes and the table will
   not re-fetch.

5. Pass `effectiveBaseParams` in place of `baseParams` to `SurfaceRenderer` (and to the existing
   hydration call — `parseRecordFromStableJson(baseParamsFingerprint)` already reads from the
   fingerprint, so updating the fingerprint source in step 4 is sufficient; `requestParams` in the
   hydration effect requires no separate change).
6. No explicit `hydrationRetryNonce` increment needed when `selectedContextValue` changes — changing
   `effectiveBaseParams` changes `baseParamsFingerprint` which is part of `hydrationFingerprint`,
   which already triggers re-hydration via the existing `$effect`.

### `frontend/src/lib/components/surfaces/SurfaceRenderer.svelte`

Add two new props (both optional / defaulted):

- `requiredContextParam: string | undefined`
- `requiredForInteractionIds: string[]` (default `[]`)

When rendering an `action_bar` node, forward both props to `SurfaceActionBar`. No other node branches
need these props.

> **Note:** `SurfaceRenderer` receives `effectiveBaseParams` (renamed from `baseParams` at the call
> site in `SurfaceReadPanel`) via its existing `baseParams` prop. No prop rename is needed in
> `SurfaceRenderer` itself — the caller passes the derived value under the same prop name.

### `frontend/src/lib/components/surfaces/SurfaceActionBar.svelte`

Add two new props:

- `requiredContextParam: string | undefined`
- `requiredForInteractionIds: string[]` (default `[]`)

For each `SurfaceInteractionButton` rendered, pass `requiredContextParam` only if
`interaction.interaction_id` is in `requiredForInteractionIds`; otherwise pass `undefined`.

### `frontend/src/lib/components/surfaces/SurfaceInteractionButton.svelte`

Add disabled guard via a new `requiredContextParam: string | undefined` prop. When the prop is set,
`requiredContextParam` holds the param key (e.g. `"plugin_config_id"`). If `baseParams[requiredContextParam]`
is absent or empty string, render the button disabled.

`Button` does not accept a `title` prop and uses `disabled:pointer-events-none` — native HTML
`title` tooltips are invisible on disabled buttons. Use a wrapper `<span>` to carry the tooltip text
(the span retains pointer events even when the child button does not):

```svelte
{#if requiredContextParam && !baseParams[requiredContextParam]}
  <span title="Select a configuration first">
    <Button variant="primary" {size} disabled>{actionLabel}</Button>
  </span>
{:else}
  <!-- existing Button render unchanged -->
{/if}
```

Threading — two props flow from `SurfaceReadPanel` → `SurfaceRenderer` → `SurfaceActionBar`:

- `requiredContextParam: string | undefined` — the param key (e.g. `"plugin_config_id"`).
- `requiredForInteractionIds: string[]` — the `required_for_interactions` list from the descriptor.

`SurfaceActionBar` decides per button: if `interaction.interaction_id` is in `requiredForInteractionIds`,
pass `requiredContextParam` to that `SurfaceInteractionButton`; otherwise pass `undefined`. Each button
checks: if `requiredContextParam` is set AND `baseParams[requiredContextParam]` is absent or empty string,
render disabled. Buttons not in the list receive `undefined` and behave unchanged.

Components outside the `SurfaceReadPanel` → `SurfaceRenderer` → `SurfaceActionBar` chain (e.g. recursive
section nodes, table row action buttons) do NOT receive these props — `required_for_interactions` applies
only to action bars declared at the top level of the surface.

### `crates/plugins/infrastructure/proxmox/src/plugin.rs`

Replace `proxmox_hosts_selector_boundary_surface()` with `proxmox_hosts_surface()`:

```rust
fn proxmox_hosts_surface() -> surfaces::RegisteredSurface {
    let data_source_id = surfaces::DataSourceId::new("proxmox.hosts.mappings")
        .expect("literal data source id is valid");

    surfaces::RegisteredSurface {
        descriptor: surfaces::SurfaceDescriptor {
            surface_id: surfaces::SurfaceId::new("proxmox.hosts")
                .expect("literal surface id is valid"),
            label: "Proxmox VE Hosts".to_string(),
            priority: 650,
            slot: surfaces::SLOT_SURFACE_PAGE.to_string(),
            scope: surfaces::Scope::Global,
            targeting: surfaces::Targeting::Universal,
            required_permission: Some(Permission::UpdateHosts.to_string()),
            provider_kind: surfaces::ProviderKind::Plugin,
            required_capabilities: surfaces::CapabilitySet::from_capabilities([
                surfaces::Capability::SectionNode,
                surfaces::Capability::ActionBarNode,
                surfaces::Capability::TableNode,
                surfaces::Capability::DataLoad,
                surfaces::Capability::MutationAction,
                surfaces::Capability::ConfirmableAction,
                surfaces::Capability::ProviderQueryDataSource,
                surfaces::Capability::UniversalTargeting,
                surfaces::Capability::ContextSelector,
            ]),
            root_node: surfaces::SurfaceNode::Section {
                title: None,
                children: vec![
                    surfaces::SurfaceNode::ActionBar {
                        action_ids: vec![
                            surfaces::InteractionId::new("discover")
                                .expect("literal interaction id is valid"),
                            surfaces::InteractionId::new("test-connection")
                                .expect("literal interaction id is valid"),
                        ],
                    },
                    surfaces::SurfaceNode::Table {
                        data_source_id: data_source_id.clone(),
                        columns: vec![
                            surfaces::SurfaceTableColumn { key: "proxmox_name".to_string(), label: "Name".to_string() },
                            surfaces::SurfaceTableColumn { key: "config_name".to_string(), label: "Configuration".to_string() },
                            surfaces::SurfaceTableColumn { key: "proxmox_node".to_string(), label: "Node".to_string() },
                            surfaces::SurfaceTableColumn { key: "proxmox_vmid".to_string(), label: "VMID".to_string() },
                            surfaces::SurfaceTableColumn { key: "proxmox_type".to_string(), label: "Type".to_string() },
                            surfaces::SurfaceTableColumn { key: "proxmox_status".to_string(), label: "Status".to_string() },
                            surfaces::SurfaceTableColumn { key: "hostname".to_string(), label: "Hostname".to_string() },
                            surfaces::SurfaceTableColumn { key: "matched_host".to_string(), label: "Matched Host".to_string() },
                            surfaces::SurfaceTableColumn { key: "suggested_host".to_string(), label: "Suggested Match".to_string() },
                        ],
                        row_actions: vec![
                            surfaces::SurfaceTableRowAction {
                                interaction_id: surfaces::InteractionId::new("approve-match")
                                    .expect("literal interaction id is valid"),
                                visible_when: Some(surfaces::SurfaceRowVisibleWhen {
                                    field: "suggested_host_id".to_string(),
                                    condition: surfaces::SurfaceRowCondition::Present,
                                }),
                            },
                            surfaces::SurfaceTableRowAction {
                                interaction_id: surfaces::InteractionId::new("match")
                                    .expect("literal interaction id is valid"),
                                visible_when: None,
                            },
                            surfaces::SurfaceTableRowAction {
                                interaction_id: surfaces::InteractionId::new("unmatch")
                                    .expect("literal interaction id is valid"),
                                visible_when: Some(surfaces::SurfaceRowVisibleWhen {
                                    field: "matched_host".to_string(),
                                    condition: surfaces::SurfaceRowCondition::Present,
                                }),
                            },
                        ],
                    },
                ],
            },
            context_selector: Some(surfaces::SurfaceContextSelectorDescriptor {
                param_key: "plugin_config_id".to_string(),
                label: "Configuration".to_string(),
                all_option_label: "All Configurations".to_string(),
                rest_api_path: "/api/v1/plugin-configs?plugin_type=infrastructure_proxmox".to_string(),
                value_field: "id".to_string(),
                label_field: "name".to_string(),
                required_for_interactions: vec![
                    "discover".to_string(),
                    "test-connection".to_string(),
                ],
            }),
        },
        interactions: vec![
            surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new("list")
                    .expect("literal interaction id is valid"),
                kind: surfaces::InteractionKind::DataLoad,
                label: "List Hosts".to_string(),
                required_permission: Some(Permission::UpdateHosts.to_string()),
                input_schema: None,
                result_schema: Some(surfaces::SchemaContract::Any),
                sensitive_fields: vec![],
                timeout_seconds: None,
                confirmation: None,
                transport: surfaces::InteractionTransport::ControllerLocal,
                workflow_steps: vec![],
                form_ui: None,
            },
            surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new("discover")
                    .expect("literal interaction id is valid"),
                kind: surfaces::InteractionKind::MutationAction,
                label: "Discover".to_string(),
                required_permission: Some(Permission::UpdateHosts.to_string()),
                input_schema: Some(surfaces::SchemaContract::Object),
                result_schema: Some(surfaces::SchemaContract::Any),
                sensitive_fields: vec![],
                timeout_seconds: Some(120),
                confirmation: None,
                transport: surfaces::InteractionTransport::ControllerLocal,
                workflow_steps: vec![],
                form_ui: None,
            },
            surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new("test-connection")
                    .expect("literal interaction id is valid"),
                kind: surfaces::InteractionKind::MutationAction,
                label: "Test Connection".to_string(),
                required_permission: Some(Permission::UpdateHosts.to_string()),
                input_schema: Some(surfaces::SchemaContract::Object),
                result_schema: Some(surfaces::SchemaContract::Any),
                sensitive_fields: vec![],
                timeout_seconds: Some(30),
                confirmation: None,
                transport: surfaces::InteractionTransport::ControllerLocal,
                workflow_steps: vec![],
                form_ui: None,
            },
            surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new("approve-match")
                    .expect("literal interaction id is valid"),
                kind: surfaces::InteractionKind::MutationAction,
                label: "Approve Match".to_string(),
                required_permission: Some(Permission::UpdateHosts.to_string()),
                input_schema: Some(surfaces::SchemaContract::Object),
                result_schema: Some(surfaces::SchemaContract::Any),
                sensitive_fields: vec![],
                timeout_seconds: None,
                confirmation: None,
                transport: surfaces::InteractionTransport::ControllerLocal,
                workflow_steps: vec![],
                form_ui: None,
            },
            surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new("match")
                    .expect("literal interaction id is valid"),
                kind: surfaces::InteractionKind::FormSubmit,
                label: "Manual Match".to_string(),
                required_permission: Some(Permission::UpdateHosts.to_string()),
                input_schema: Some(surfaces::SchemaContract::Object),
                result_schema: Some(surfaces::SchemaContract::Any),
                sensitive_fields: vec![],
                timeout_seconds: None,
                confirmation: None,
                transport: surfaces::InteractionTransport::ControllerLocal,
                workflow_steps: vec![],
                form_ui: Some(surfaces::FormUiDescriptor {
                    fields: vec![
                        surfaces::FormFieldDescriptor {
                            key: "mapping_id".to_string(),
                            label: "Mapping ID".to_string(),
                            field_type: "hidden".to_string(),
                            required: true,
                            placeholder: None,
                            help_text: None,
                            default_value: None,
                            options: vec![],
                            select_source: None,
                            sensitive: false,
                            list: false,
                            visible_when: None,
                        },
                        surfaces::FormFieldDescriptor {
                            key: "host_id".to_string(),
                            label: "Host".to_string(),
                            field_type: "select".to_string(),
                            required: true,
                            placeholder: Some("Select a host".to_string()),
                            help_text: None,
                            default_value: None,
                            options: vec![],
                            select_source: Some(surfaces::FormSelectSource::RestApi {
                                path: "/api/v1/hosts".to_string(),
                                value_field: "id".to_string(),
                                label_field: "friendly_name".to_string(),
                            }),
                            sensitive: false,
                            list: false,
                            visible_when: None,
                        },
                    ],
                    pre_load_interaction_id: None,
                }),
            },
            surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new("unmatch")
                    .expect("literal interaction id is valid"),
                kind: surfaces::InteractionKind::MutationAction,
                label: "Remove Match".to_string(),
                required_permission: Some(Permission::UpdateHosts.to_string()),
                input_schema: Some(surfaces::SchemaContract::Object),
                result_schema: Some(surfaces::SchemaContract::Any),
                sensitive_fields: vec![],
                timeout_seconds: None,
                confirmation: Some(surfaces::InteractionConfirmation {
                    title: "Remove Match".to_string(),
                    message: "Remove the host mapping for".to_string(),
                    confirm_label: Some("Remove".to_string()),
                    cancel_label: None,
                    severity: surfaces::ConfirmationSeverity::Danger,
                }),
                transport: surfaces::InteractionTransport::ControllerLocal,
                workflow_steps: vec![],
                form_ui: None,
            },
        ],
        data_sources: vec![surfaces::DataSourceDescriptor {
            data_source_id,
            kind: surfaces::DataSourceKind::ProviderQuery {
                operation_id: "list".to_string(),
            },
            result_schema: surfaces::SchemaContract::Any,
            pagination: Some(surfaces::DataSourcePagination {
                default_page_size: 50,
                max_page_size: 200,
            }),
            sorting: None,
            filtering: None,
            refresh_policy: surfaces::RefreshPolicy::Manual,
            empty_state: Some(surfaces::DataSourceEmptyState {
                title: "No Proxmox guests found".to_string(),
                description: Some(
                    "Run Discover on a configuration to populate this table.".to_string(),
                ),
            }),
        }],
    }
}
```

Update `proxmox_surface_registrations()` to call `proxmox_hosts_surface()` instead of
`proxmox_hosts_selector_boundary_surface()`.

### `crates/plugins/infrastructure/proxmox/src/surfaces.rs`

**`handle_list`** — make `plugin_config_id` optional:

Change `parse_uuid_param(&params, "plugin_config_id")?` to:

```rust
let plugin_config_id: Option<Uuid> = parse_optional_uuid_param(&params, "plugin_config_id")?;
```

`parse_optional_uuid_param` already exists in `surfaces.rs` at line 1122 — no new helper needed.
Signature: `fn parse_optional_uuid_param(params: &serde_json::Value, key: &str) -> Result<Option<Uuid>, String>`.
The `?` is required.

Change the query to always inner-join `plugin_config` to get the config name:

```rust
let mut base_query = proxmox_host_mapping::Entity::find()
    .inner_join(plugin_config::Entity);  // always join — FK guarantees no orphans

if let Some(pcid) = plugin_config_id {
    base_query = base_query.filter(proxmox_host_mapping::Column::PluginConfigId.eq(pcid));
}
// existing tenant filter unchanged
```

**SeaORM projection note:** SeaORM's `.all(db)` on a joined query only populates fields present in
`proxmox_host_mapping::Model`. `config_name` is not a column on `proxmox_host_mapping`. Use
`.column_as(plugin_config::Column::Name, "config_name")` with a custom `DerivePartialModel` or
`FromQueryResult` DTO to capture the joined field. Follow the pattern used elsewhere in the codebase for
joined projections.

Row serialisation must include two new fields:

```rust
"plugin_config_id": m.plugin_config_id.to_string(),   // already on model
"config_name": row_dto.config_name,                    // from DTO / custom select
```

`plugin_config_id` is already on the model (FK column); `config_name` requires the DTO approach above.

**Cross-tenant note:** `handle_list` already returns all-tenant data when `tenant_id` is `None` — this is
existing behaviour and intentional for global admins. The "All Configurations" path does not change this
policy.

**Action definitions** (`surface_actions()`) — no change. `surface_actions()` defines the legacy
action library used by the old `/surface-actions` dispatch path. The new `proxmox_hosts_surface()`
registration declares `InteractionDescriptor` objects inline in the `RegisteredSurface` struct — these
take precedence for shared-surface dispatch. `list` is absent from `surface_actions()` intentionally; it
is handled by `handle_action_inner` (existing arm `("proxmox.hosts", "list")`) and only exposed via the
new surface registration's `InteractionDescriptor`.

### Tests

**`crates/plugins/infrastructure/proxmox/src/plugin.rs`**

Replace test `proxmox_hosts_surface_makes_selector_boundary_explicit` with
`proxmox_hosts_surface_has_full_table_layout`:

```rust
#[test]
fn proxmox_hosts_surface_has_full_table_layout() {
    let registrations = (DESCRIPTOR.surface_registration_ops.as_ref().unwrap().registrations)();
    let hosts = registrations
        .iter()
        .flat_map(|r| r.surfaces.iter())
        .find(|s| s.descriptor.surface_id.as_str() == "proxmox.hosts")
        .expect("proxmox.hosts surface must be registered");

    // context_selector present with correct param key
    let selector = hosts.descriptor.context_selector.as_ref()
        .expect("proxmox.hosts must declare a context_selector");
    assert_eq!(selector.param_key, "plugin_config_id");
    assert!(selector.required_for_interactions.contains(&"discover".to_string()));
    assert!(selector.required_for_interactions.contains(&"test-connection".to_string()));

    // root is a section containing action bar + table
    let children = match &hosts.descriptor.root_node {
        surfaces::SurfaceNode::Section { children, .. } => children,
        other => panic!("expected section root, got {other:?}"),
    };
    assert!(children.iter().any(|n| matches!(n, surfaces::SurfaceNode::ActionBar { .. })));
    let table = children.iter().find_map(|n| match n {
        surfaces::SurfaceNode::Table { row_actions, .. } => Some(row_actions),
        _ => None,
    }).expect("root section must contain a table node");

    // row actions
    let row_action_ids: Vec<&str> = table.iter()
        .map(|ra| ra.interaction_id.as_str())
        .collect();
    assert!(row_action_ids.contains(&"approve-match"));
    assert!(row_action_ids.contains(&"match"));
    assert!(row_action_ids.contains(&"unmatch"));

    // unmatch has danger confirmation
    let unmatch_interaction = hosts.interactions.iter()
        .find(|i| i.interaction_id.as_str() == "unmatch")
        .expect("unmatch interaction must be declared");
    assert!(matches!(
        unmatch_interaction.confirmation.as_ref().map(|c| &c.severity),
        Some(surfaces::ConfirmationSeverity::Danger)
    ));

    // list interaction present (data load)
    assert!(hosts.interactions.iter().any(|i| i.interaction_id.as_str() == "list"));
    // one paginated data source
    assert_eq!(hosts.data_sources.len(), 1);
    assert!(hosts.data_sources[0].pagination.is_some());
}
```

**`crates/plugins/infrastructure/proxmox/src/surfaces.rs`** — new `handle_list` unit tests:

```rust
#[tokio::test]
async fn handle_list_without_plugin_config_id_returns_all_tenant_mappings() {
    // setup: insert two plugin_config rows + two proxmox_host_mappings under same tenant_id
    // call: handle_list(db, Some(tenant_id), serde_json::json!({}))
    // assert: both rows returned; each has "plugin_config_id" and "config_name" fields
}

#[tokio::test]
async fn handle_list_with_plugin_config_id_filters_to_that_config() {
    // setup: two plugin_config rows; two mappings (one per config)
    // call: handle_list(db, Some(tenant_id), serde_json::json!({ "plugin_config_id": config1_id.to_string() }))
    // assert: only row matching config1_id returned
}
```

`handle_list` returns `Result<serde_json::Value, String>`. `parse_optional_uuid_param` returns
`Result<Option<Uuid>, String>`, so `?` propagation is compatible.

**`frontend/src/lib/components/surfaces/SurfaceReadPanel.test.ts`** — new tests:

- Context selector rendered when `descriptor.context_selector` present; options fetched from `rest_api_path`.
- "All" option is first; selecting it leaves `effectiveBaseParams` equal to `baseParams`.
- Selecting a specific option merges `{ [param_key]: selectedId }` into `effectiveBaseParams` (verify via
  stub on `SurfaceRenderer` capturing received `baseParams` prop).

**`frontend/src/lib/components/surfaces/SurfaceInteractionButton.test.ts`** — new tests:

- Button disabled + `title="Select a configuration first"` when `requiredContextParam` set and
  `baseParams[requiredContextParam]` absent.
- Button enabled when `baseParams[requiredContextParam]` present.

## Data flow

```text
/surfaces/proxmox.hosts
  └── SurfaceReadPanel (baseParams = {})
        ├── [context_selector dropdown]  ← fetches /api/v1/plugin-configs?plugin_type=...
        │     effectiveBaseParams = {} | { plugin_config_id: "..." }
        └── SurfaceRenderer
              ├── SurfaceActionBar
              │     ├── SurfaceInteractionButton [discover]   ← disabled when no plugin_config_id
              │     └── SurfaceInteractionButton [test-connection] ← same
              └── SurfaceTable (data_source: proxmox.hosts.mappings)
                    loadPage() → invokeSurfaceInteraction("proxmox.hosts", "list",
                        { params: { ...effectiveBaseParams, page, per_page } })
                    row actions → invokeSurfaceInteraction("proxmox.hosts", actionId,
                        { params: { ...effectiveBaseParams, ...row } })
```

## Error handling

- Context selector fetch failure: show empty dropdown with "All Configurations" only; surface still renders
  (table loads all data, action buttons remain disabled). No crash.
- `handle_list` with absent `plugin_config_id`: uses `INNER JOIN plugin_config`. Orphaned mappings
  (config deleted) are excluded — acceptable because the `plugin_config` FK is expected to cascade-delete
  associated mappings. No null-config_name case to handle.
- Unmatch confirmation dialog (danger severity) guards against accidental removal.

## Testing summary

| Layer         | What's tested                                                                         |
| ------------- | ------------------------------------------------------------------------------------- |
| Rust unit     | New surface descriptor (table node, context selector, interactions, data source)      |
| Rust unit     | `handle_list` no `plugin_config_id`: all-tenant rows, `config_name` field present     |
| Rust unit     | `handle_list` with `plugin_config_id`: filters to that config (regression)            |
| Frontend unit | `SurfaceReadPanel` renders context selector; options fetched from REST API            |
| Frontend unit | Selector selection updates `effectiveBaseParams`                                      |
| Frontend unit | `SurfaceInteractionButton` disabled when `requiredContextParam` set, param absent     |
| Frontend e2e  | Default state: table visible (mocked), selector shows "All Configurations"            |
| Frontend e2e  | Config selected: table refetches with `plugin_config_id`; Discover button enabled     |

## Rollout

Single PR: `feat(surfaces): restore proxmox.hosts with context_selector framework extension`

Commit order:

1. `crates/shared/surfaces`: add `ContextSelector` capability, `SurfaceContextSelectorDescriptor` struct,
   and `context_selector` field on `SurfaceDescriptor`.
2. `frontend/src/lib/surfaces/contract.ts`: add `SurfaceContextSelector` interface + field.
3. `frontend/src/lib/components/surfaces/SurfaceReadPanel.svelte`: context selector rendering + state.
4. `frontend/src/lib/components/surfaces/SurfaceRenderer.svelte`: add `requiredContextParam` +
   `requiredForInteractionIds` props; forward to `SurfaceActionBar` only on `action_bar` branch.
   `frontend/src/lib/components/surfaces/SurfaceActionBar.svelte`: accept both props; pass
   `requiredContextParam` per-button based on the ID list.
   `frontend/src/lib/components/surfaces/SurfaceInteractionButton.svelte`: disabled guard via
   `requiredContextParam` prop.
5. `crates/plugins/infrastructure/proxmox/src/surfaces.rs`: `handle_list` optional `plugin_config_id` +
   `parse_optional_uuid_param` helper.
6. `crates/plugins/infrastructure/proxmox/src/plugin.rs`: replace boundary surface with full surface.
7. Tests: Rust unit, frontend unit.
8. Full quality gate.

### Dependencies + ordering

- **No block dependencies:** parallel-safe with all Wave 5 specs (#3c/#3g/#3h/#3j/#4) — no file conflicts.
- **Blocks:** nothing.
- **Risk:** `SurfaceContextSelectorDescriptor` is a new serialised field — old frontend versions
  reading a new descriptor will ignore the unknown field (`#[serde(default)]` on the Option ensures
  backwards compat). New frontend reading an old descriptor (no field) renders no selector, which is safe
  — the table loads all data and action buttons remain enabled (no regression for non-Proxmox surfaces).
