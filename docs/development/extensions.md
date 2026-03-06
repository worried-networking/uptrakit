# UI Extensions

Developer guide for the UI extensions framework. This document covers how to create extensions
(plugin-backed and service-backed), the manifest schema, action protocol, timeout handling,
request/response correlation, multi-instance registration rules, and how to add extension
support to a new service binary.

## Architecture overview

The extension framework uses a hybrid registration model:

- **Compile-time registration** for plugins (via the `PluginOps` trait)
- **Runtime registration** for connected services (via the wire protocol)

The controller maintains an `ExtensionRegistry` that tracks all active extensions and their
providers. When a user invokes an action, the controller proxies the request to the appropriate
service instance over WebSocket using a oneshot-channel correlation pattern.

```text
Plugin                     Controller                    Service
  │                           │                            │
  │  PluginOps::              │                            │
  │  extension_manifests()    │                            │
  │──────────────────────────>│                            │
  │                           │                            │
  │                           │  ExtensionRegister         │
  │                           │<───────────(WS)────────────│
  │                           │                            │
  │                           │  ExtensionRequest          │
  │                           │────────────(WS)───────────>│
  │                           │                            │
  │                           │  ExtensionResponse         │
  │                           │<───────────(WS)────────────│
```

For a detailed architecture breakdown, see [Extensions Architecture](../architecture/extensions.md).

## Extension manifest schema

Every extension is described by an `ExtensionManifest`. The manifest declares where the
extension appears in the UI, what permissions are required, how actions are routed, and
what UI components to render.

### `ExtensionManifest`

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `id` | string | yes | Unique identifier (e.g., `"ssh-agent.host-management"`) |
| `label` | string | yes | Human-readable name displayed in the UI |
| `placement` | `ExtensionPlacement` | yes | Where this extension appears |
| `required_permission` | string | no | Permission gate (empty = any authenticated user) |
| `targeting` | `ExtensionTargeting` | no | `universal` (default) or `targeted` |
| `ui` | `ExtensionUi` | yes | Schema-driven UI definition |

### `ExtensionPlacement`

Internally tagged with `"type"`. Four variants:

| Variant | Fields | Description |
| --- | --- | --- |
| `page` | `nav_section`, `icon` (optional) | Full sidebar page |
| `panel` | `target_page`, `position` | Panel injected into an existing page |
| `context_menu_group` | `target_entity`, `group_label` | Action group in an entity context menu |
| `table_columns` | `target_table`, `columns` | Extra columns added to an existing table |

**`target_entity` values:** `"host"`, `"service"`, `"software_item"`, `"host_package"`.

**`target_table` values:** `"hosts"`, `"services"`, `"software_items"`, `"host_packages"`.

### `PanelPosition`

| Variant | Description |
| --- | --- |
| `tab` (default) | Rendered as a tab alongside existing tabs |
| `below` | Below the main content |
| `above` | Above the main content |
| `Other(String)` | Forward-compatible catch-all |

### `ExtensionTargeting`

| Variant | Description |
| --- | --- |
| `universal` (default) | Any connected instance can handle actions; controller picks one |
| `targeted` | User must select a specific service instance |

### `ExtensionUi`

Internally tagged with `"type"`. Four variants:

| Variant | Fields | Description |
| --- | --- | --- |
| `data_table` | `columns`, `data_action`, `row_actions`, `primary_actions` | Table with data fetching |
| `form` | `fields` | Input form |
| `key_value` | `data_action` | Read-only key-value display |
| `actions` | `actions` | List of actions (used with `context_menu_group` placement) |

### `ActionDef`

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `action_id` | string | yes | Unique within the extension |
| `label` | string | yes | Button/menu item label |
| `ui` | `ActionUi` | no | Optional form or wizard shown before invocation |
| `permission` | string | no | Permission required to invoke |
| `destructive` | bool | no | Show with warning styling (default: `false`) |
| `timeout_seconds` | u32 | no | Override the default 30s timeout |

### `ActionUi`

Internally tagged with `"type"`:

- **`form`** -- a `FormDef` with fields
- **`wizard`** -- multi-step wizard with `steps: Vec<WizardStep>`

### `FormDef` and `FieldDef`

`FormDef` contains `fields: Vec<FieldDef>`. Each `FieldDef`:

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `key` | string | yes | Field key in form submission |
| `label` | string | yes | Display label |
| `field_type` | `FieldType` | no | `text` (default), `password`, `number`, `select`, `textarea`, `toggle`, `hidden` |
| `required` | bool | no | Default `false` |
| `placeholder` | string | no | Input placeholder |
| `help_text` | string | no | Help text below the field |
| `default_value` | JSON value | no | Default value |
| `options` | `Vec<SelectOption>` | no | Options for `select` fields |

### `WizardStep`

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `step_id` | string | yes | Step identifier |
| `label` | string | yes | Step indicator label |
| `form` | `FormDef` | yes | Fields for this step |
| `submit_action` | string | no | Action to submit before proceeding |

## Creating a service-backed extension

Service-backed extensions are registered at runtime when a service connects to the controller
over WebSocket.

### Step 1: Declare the `UiExtensions` capability

Add `Capability::UiExtensions` to your service's capability set during enrollment:

```rust
capabilities: vec![
    Capability::SoftwareDiscovery,
    Capability::UiExtensions,
],
```

### Step 2: Send `ExtensionRegister` after connection

After the service connects and authenticates, send an `ExtensionRegister` message with
your manifests:

```rust
conn.send(ServiceMessage::ExtensionRegister(ExtensionRegisterPayload {
    manifests: vec![
        ExtensionManifest {
            id: "ssh-agent.host-management".to_string(),
            label: "SSH Host Management".to_string(),
            placement: ExtensionPlacement::Page {
                nav_section: "management".to_string(),
                icon: Some("server".to_string()),
            },
            required_permission: "manage_hosts".to_string(),
            targeting: ExtensionTargeting::Targeted,
            ui: ExtensionUi::DataTable {
                columns: vec![TableColumn {
                    key: "hostname".to_string(),
                    label: "Hostname".to_string(),
                    sortable: true,
                }],
                data_action: "list-hosts".to_string(),
                row_actions: vec![],
                primary_actions: vec![],
            },
        },
    ],
})).await?;
```

### Step 3: Handle `ExtensionRequest` messages

Override the `on_extension_request` method in your `ServiceHandler` implementation:

```rust
async fn on_extension_request(
    &mut self,
    request: ExtensionRequestPayload,
    conn: &mut ControllerConnection,
) -> LoopResult<()> {
    let response = match (request.extension_id.as_str(), request.action_id.as_str()) {
        ("ssh-agent.host-management", "list-hosts") => {
            let hosts = self.list_hosts().await?;
            ExtensionResponsePayload {
                request_id: request.request_id,
                success: true,
                data: serde_json::to_value(&hosts)?,
                error: None,
            }
        }
        _ => ExtensionResponsePayload {
            request_id: request.request_id,
            success: false,
            data: serde_json::Value::Null,
            error: Some("Unknown action".to_string()),
        },
    };

    conn.send(ServiceMessage::ExtensionResponse(response)).await?;
    Ok(())
}
```

### Step 4: Add `SERVICE_APP_NAME`

The service SDK automatically derives the service app name from the crate's `Cargo.toml`
via `env!("CARGO_PKG_NAME")`. No manual changes needed per binary.

## Creating a plugin-backed extension

Plugin-backed extensions are registered at compile time via the `PluginOps` trait.

Override the `extension_manifests` method in your plugin's `PluginOps` implementation:

```rust
fn extension_manifests(&self) -> Vec<ExtensionManifest> {
    vec![
        ExtensionManifest {
            id: "proxmox.lxc-panel".to_string(),
            label: "LXC Matching".to_string(),
            placement: ExtensionPlacement::Panel {
                target_page: "hosts".to_string(),
                position: PanelPosition::Below,
            },
            required_permission: String::new(),
            targeting: ExtensionTargeting::Universal,
            ui: ExtensionUi::KeyValue {
                data_action: "get-lxc-info".to_string(),
            },
        },
    ]
}
```

Plugin-backed extensions are loaded into the `ExtensionRegistry` at controller startup
and are always available (no provider tracking needed).

## Action protocol

### Request/response correlation

The controller uses UUID v7 request IDs and oneshot channels for correlation:

1. Frontend sends `POST /api/v1/extensions/{id}/actions/{action_id}` with JSON params.
2. Controller generates a UUID v7 `request_id` and creates a oneshot channel.
3. Controller sends `ControllerMessage::ExtensionRequest` to the target service.
4. Service processes the request and sends `ServiceMessage::ExtensionResponse` with the
   same `request_id`.
5. Controller WS handler calls `ExtensionProxy::complete()`, which sends the response
   through the oneshot channel.
6. Controller returns the response to the frontend.

### Timeout handling

Actions have a default timeout of 30 seconds. Extensions can override this per-action
via `ActionDef.timeout_seconds`.

Timeout behaviour:

- On timeout, the pending oneshot sender is removed from the proxy map.
- The REST endpoint returns `504 Gateway Timeout`.
- If the service responds after the timeout, the late response is silently dropped.

### Error responses

| HTTP Status | Condition |
| --- | --- |
| 200 | Action succeeded (`success: true`) |
| 400 | Missing `service_id` for targeted extension, or invalid params |
| 404 | Extension or action not found |
| 422 | Action failed (`success: false`, includes error message) |
| 501 | Plugin-backed action invocation (not yet implemented) |
| 503 | Target service disconnected |
| 504 | Action timed out |

## Multi-instance registration rules

1. **Same extension ID from same `service_app_name`**: Allowed. Multiple instances of the
   same service binary can register the same extension. The registry deduplicates the
   manifest and tracks all service IDs as providers.

2. **Same extension ID from different `service_app_name`**: Rejected. The second registration
   receives `ErrorCode::BadRequest` with message "Extension '{id}' is already registered by
   a different service application".

3. **On disconnect**: The service is removed from the provider set. If no providers remain,
   the extension is removed from the registry entirely.

## Adding extension support to a new service binary

1. Add `Capability::UiExtensions` to the service's capability set.
2. Build your `Vec<ExtensionManifest>` and send `ExtensionRegister` on connect.
3. Implement `on_extension_request` in your `ServiceHandler` to handle action invocations.
4. The `SERVICE_APP_NAME` is derived automatically from `env!("CARGO_PKG_NAME")`.

No changes to the controller are needed -- the framework is fully generic.

## Wire protocol limits

All extension payloads are validated via `WireValidate` after deserialization:

| Limit | Value | Description |
| --- | --- | --- |
| `MAX_EXTENSION_MANIFESTS` | 50 | Manifests per `ExtensionRegister` message |
| `MAX_EXTENSION_COLUMNS` | 50 | Columns per table or `DataTable` UI |
| `MAX_EXTENSION_ACTIONS` | 50 | Actions per extension UI |
| `MAX_EXTENSION_FIELDS` | 100 | Fields per form |
| `MAX_EXTENSION_WIZARD_STEPS` | 20 | Steps per wizard |
| `MAX_EXTENSION_SELECT_OPTIONS` | 200 | Options per select field |
| `MAX_EXTENSION_PARAMS_LEN` | 64 KB | Action params JSON size |
| `MAX_EXTENSION_RESPONSE_LEN` | 1 MB | Action response JSON size |

String lengths use the standard wire limits (`MAX_SHORT_STRING_LEN` = 1024,
`MAX_MEDIUM_STRING_LEN` = 4096).

## Key files

| File | Purpose |
| --- | --- |
| `crates/shared/wire/src/extension.rs` | Extension manifest types and wire payloads |
| `crates/shared/wire/src/limits.rs` | Wire validation limits |
| `crates/shared/wire/src/wire_validate_impls.rs` | `WireValidate` implementations |
| `crates/ui/web-api/src/extension_registry.rs` | Extension registry (provider tracking) |
| `crates/ui/web-api/src/extension_proxy.rs` | Request/response proxy (oneshot channels) |
| `crates/ui/web-api/src/routes/extensions.rs` | REST API route handlers |
| `crates/shared/web-api-types/src/extensions.rs` | REST API request/response types |
| `crates/shared/openapi-client/src/extensions.rs` | OpenAPI client methods |
| `crates/shared/service-sdk/src/lifecycle.rs` | `ServiceHandler::on_extension_request` |
| `crates/shared/service-sdk/src/event_loop.rs` | `ExtensionRequest` dispatch |
| `crates/plugins/infrastructure/registry/src/lib.rs` | `PluginOps::extension_manifests` |
| `crates/ui/cli/src/commands/extensions.rs` | CLI `extensions` subcommand |

## Cross-references

- [Extensions Architecture](../architecture/extensions.md)
- [Extensions Security](../security/extensions.md)
- [Extensions API Reference](../api/extensions.md)
- [Extensions End-User Guide](../end-user/extensions.md)
- [Plugin Guidelines](plugin-guidelines.md)
- [Coding Standards](coding-standards.md)
- [Error Handling](error-handling.md)
- [Testing](testing.md)
