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
| `data_table` | `columns`, `data_action`, `row_actions`, `primary_actions`, `context_selector` | Table with data fetching |
| `form` | `fields` | Input form |
| `key_value` | `data_action` | Read-only key-value display |
| `actions` | `actions` | List of actions (used with `context_menu_group` placement) |

#### Context selector

The `data_table` variant accepts an optional `context_selector: Option<Box<ContextSelectorDef>>`.
When set, the user must choose a value from a dropdown **before** table data loads. The selected
value is automatically injected into all action invocations (data load, primary actions, row
actions) under `context_selector.param_key`.

This eliminates the need for a plugin config picker field in every action form. It also blocks
the data-load request from firing until a value is selected, preventing "missing required
parameter" errors when the page first opens with no existing configuration.

`ContextSelectorDef` fields:

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `param_key` | string | yes | Key injected into all action params (e.g., `"plugin_config_id"`) |
| `label` | string | yes | Dropdown label shown in the UI |
| `source` | `ContextSelectorSource` | yes | How to populate the dropdown options |
| `add_action` | `ActionDef` | no | "Add" button shown next to the selector |
| `empty_message` | string | no | Message shown when no options exist |

`ContextSelectorSource` variants:

| Variant | Fields | Description |
| --- | --- | --- |
| `action` | `action_id` | Invokes an extension action; response must be `[{ value, label }]` |
| `plugin_configs` | `plugin_type` | Calls `GET /api/v1/plugin-configs` and filters by `plugin_type`; maps `{ id, name }` |

The `add_action` may carry `api_submit` to route form submission directly to a REST API
endpoint instead of through the extension proxy. After a successful add, the frontend
refreshes the options list and auto-selects the newly created item (if the response includes
the field named by `api_submit.response_id_field`).

### `ActionDef`

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `action_id` | string | yes | Unique within the extension |
| `label` | string | yes | Button/menu item label |
| `ui` | `ActionUi` | no | Optional form or wizard shown before invocation |
| `permission` | string | no | Permission required to invoke |
| `destructive` | bool | no | Show with warning styling (default: `false`) |
| `timeout_seconds` | u32 | no | Override the default 30s timeout |
| `api_submit` | `ApiSubmitDef` | no | Route form submission to a REST API instead of the extension proxy |
| `row_visible_when` | `RowVisibleWhen` | no | Conditional visibility for row actions in a `DataTable` |

#### `RowVisibleWhen` — conditional row action visibility

When set on an `ActionDef` used as a row action in a `DataTable`, the action
button is only rendered in rows where the condition is met.

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `field` | string | yes | Key of the row data field to check |
| `condition` | `"present"` or `"absent"` | yes | `present`: field is non-null; `absent`: field is null or missing |

Example — show "Approve Match" only when a suggestion exists:

```rust
ActionDef::new("approve-match", "Approve Match")
    .with_row_visible_when("suggested_host_id", RowCondition::Present)
```

#### `ApiSubmitDef` — calling existing REST APIs from extension forms

When `api_submit` is set on an `ActionDef`, the frontend bypasses the extension proxy on
form submission and calls the specified REST endpoint directly. This allows extensions to
expose existing API operations (create plugin config, update service settings, etc.) as
first-class action buttons without duplicating logic in an extension handler.

`ApiSubmitDef` fields:

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `method` | string | yes | HTTP method (`"POST"`, `"PUT"`, `"PATCH"`, `"DELETE"`) |
| `path` | string | yes | API path relative to the base URL (e.g., `"/api/v1/plugin-configs"`) |
| `body` | JSON | yes | Body template — string leaves matching `{{field_name}}` are substituted |
| `response_id_field` | string | no | JSON response field containing the new item's ID |
| `response_label_field` | string | no | JSON response field containing the new item's label |

**Body template syntax:**

| Placeholder | Coercion | Result |
| --- | --- | --- |
| `"{{name}}"` | (none) | String value |
| `"{{enabled:bool}}"` | `bool` | `"true"` → `true`, anything else → `false` |
| `"{{count:number}}"` | `number` | String parsed as JSON number |
| `"{{tags:csv_array}}"` | `csv_array` | Comma-split, trimmed, empty-filtered JSON array |

Example — create a plugin config on form submit:

```rust
ActionDef::new("add-config", "Add Configuration")
    .with_permission(Permission::ManageHosts)
    .with_ui(ActionUi::Form(FormDef::new(vec![
        FieldDef::new("name", "Name").required(),
        FieldDef::new("api_url", "API URL").required(),
    ])))
    .with_api_submit(
        ApiSubmitDef::new(
            "POST",
            "/api/v1/plugin-configs",
            serde_json::json!({
                "name": "{{name}}",
                "plugin_type": "my_plugin",
                "enabled": true,
                "config": { "api_url": "{{api_url}}" }
            }),
        )
        .with_response_id_field("id")
        .with_response_label_field("name"),
    )
```

### `Permission` enum

Use `uptrakit_shared_types::Permission` (not raw strings) when calling `.with_permission()`:

```rust
use uptrakit_shared_types::Permission;

ActionDef::new("discover", "Discover")
    .with_permission(Permission::ManageHosts)
```

`Permission` lives in `uptrakit-shared-types` so it is accessible to plugins (which must not
depend on `uptrakit-web-api-types`) and to `web-api-types` (which re-exports it). The
`.with_permission()` builders accept `impl Into<String>`, and `Permission` implements
`From<Permission> for String`.

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
| `options` | `Vec<SelectOption>` | no | Static options for `select` fields |
| `select_source` | `SelectSource` | no | Dynamic options loaded at form-open time; takes precedence over `options` |
| `sensitive` | bool | no | Field contains sensitive data (encrypted client-side via ECIES) |
| `visible_when` | `VisibleWhen` | no | Conditional visibility based on another field's value |

#### Dynamic select options (`SelectSource`)

For `select` fields whose options depend on live data (e.g., picking an existing host), use
`select_source` instead of static `options`. The frontend loads options when the form modal
opens, before the user interacts with the field.

Two variants are supported:

**`rest_api` — Fetch from a REST endpoint**

| Field | Type | Description |
| --- | --- | --- |
| `type` | `"rest_api"` | Fetch options from an authenticated REST `GET` endpoint |
| `path` | string | API path relative to the controller base URL (e.g., `"/api/v1/hosts"`) |
| `value_field` | string | Field in each response item used as the submitted option value |
| `label_field` | string | Field in each response item used as the human-readable label |

The frontend calls `GET {path}` with the current user's auth token. The response must be either
a JSON array, or an object with an `items` array (paginated response). Each item is mapped to
`{ value: item[value_field], label: item[label_field] }`.

**Example — host picker:**

```rust
FieldDef::new("host_id", "Host")
    .with_type(FieldType::Select)
    .required()
    .with_select_source(SelectSource::RestApi {
        path: "/api/v1/hosts".to_string(),
        value_field: "id".to_string(),
        label_field: "friendly_name".to_string(),
    })
```

**`action` — Fetch from an extension action**

| Field | Type | Description |
| --- | --- | --- |
| `type` | `"action"` | Fetch options by invoking an extension action |
| `action_id` | string | The action ID to invoke |

The frontend calls the specified extension action and expects the response `data` to contain
an `options` array of `{ "value": "...", "label": "..." }` objects.

**Example — PVE host picker:**

```rust
FieldDef::new("pve_host_id", "PVE Host")
    .with_type(FieldType::Select)
    .required()
    .with_select_source(SelectSource::Action {
        action_id: "list-pve-hosts".to_string(),
    })
```

#### Conditional visibility (`VisibleWhen`)

Fields can be conditionally shown or hidden based on the value of another field using the
`visible_when` property. This is useful for tagged enums (e.g., Docker auth type) or sections
that only apply when a toggle is enabled.

| Field | Type | Description |
| --- | --- | --- |
| `field` | string | Key of the controlling field |
| `values` | `Vec<string>` | Field is visible when the controlling field's value is in this list |

**Example — show password field only when auth type is "basic":**

```rust
FieldDef::new("auth_password", "Password")
    .with_type(FieldType::Password)
    .sensitive()
    .with_visible_when("auth_type", &["basic"])
```

The frontend hides the field (and omits its value from submission) when the controlling field's
current value does not match any entry in `values`. Both `SchemaForm.svelte` and the plugin
config form implement this logic.

### Sensitive fields and E2E encryption

Fields marked `sensitive: true` contain credentials that must not be visible to the controller.
The client encrypts these fields using the ECIES sealed-box scheme (P-256 + AES-256-GCM) with
the target service's public key, and sends the ciphertext in
`ExtensionRequestPayload.sensitive_params` instead of `params`.

The service provides its encryption key via `ExtensionRegisterPayload.encryption_public_key`
(base64-encoded uncompressed P-256 public key, 65 bytes). The controller surfaces this key
in the `GET /api/v1/extensions/{id}/providers` response via `ExtensionProviderInfo.encryption_public_key`.

The controller passes the encrypted `sensitive_params` through opaquely — it cannot decrypt.
Only the target service instance can decrypt using its mTLS private key.

#### Frontend implementation

The frontend implements the matching ECIES sealed-box algorithm in `sealedBoxEncrypt` (in
`frontend/src/lib/api.ts`) using the Web Crypto API. The algorithm mirrors the Rust
`sealed_box_encrypt_base64` implementation exactly:

1. Import recipient's P-256 public key from the `encryption_public_key` field in the
   provider list response.
2. Generate an ephemeral P-256 keypair.
3. ECDH: derive the 32-byte X-coordinate shared secret (`crypto.subtle.deriveBits`).
4. Key derivation: SHA-256 of the shared secret → AES-256 key.
5. AES-256-GCM encrypt with a random 12-byte nonce; the ephemeral public key bytes are
   the AAD (binds the ciphertext to this specific exchange).
6. Output: `[ephemeral pubkey (65 B)] [nonce (12 B)] [ciphertext + GCM tag (N+16 B)]`,
   base64-encoded (standard, non-URL-safe).

When `ActionButton` invokes an action, it inspects `action.ui.fields` to identify fields
with `sensitive: true`, separates them from regular params, and passes them as
`sensitiveParams` to `invokeExtensionAction`. The function encrypts them with
`sealedBoxEncrypt` and includes the ciphertext as `sensitive_params` in the request body.
If sensitive params are present but no encryption key is available (untargeted or no
`encryption_public_key`), the invocation fails with an error rather than leaking credentials.

For targeted extensions, `ServiceSelector` tracks the selected service's encryption key
(bound via `selectedEncryptionKey`) and propagates it through `SchemaTable` and `ActionButton`.

See [Extensions Security](../security/extensions.md) for the full trust model.

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
        // Use serde_json::from_value(json!({...})) for #[non_exhaustive] types
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
    encryption_public_key: None, // Set to base64-encoded P-256 public key for ECIES
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
They differ from service-backed extensions in two ways:

1. **Registration**: manifests are returned by `PluginOps::extension_manifests()` and
   loaded into the `ExtensionRegistry` at controller startup (always available, no
   provider tracking needed).
2. **Action dispatch**: invocations are handled in-process by
   `PluginOps::handle_extension_action()` instead of being proxied over WebSocket.

### Step 1: Define extension manifests

Use the builder constructors on `ExtensionManifest` and related types (required because
the types are `#[non_exhaustive]`):

```rust
pub fn extension_manifests() -> Vec<ExtensionManifest> {
    vec![
        ExtensionManifest::new(
            "myplugin.hosts",
            "My Plugin Hosts",
            ExtensionPlacement::Page {
                nav_section: "infrastructure".to_string(),
                icon: Some("server".to_string()),
            },
            ExtensionUi::DataTable {
                columns: vec![TableColumn::new("name", "Name").sortable()],
                data_action: "list".to_string(),
                row_actions: vec![],
                primary_actions: vec![
                    ActionDef::new("discover", "Discover")
                        .with_permission(Permission::ManageHosts)
                        .with_timeout(120),
                ],
                context_selector: None,
            },
        )
        .with_permission("manage_hosts"),
    ]
}
```

### Step 2: Implement action handling

Add an action handler function that dispatches by `(extension_id, action_id)`:

```rust
pub async fn handle_action(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    extension_id: &str,
    action_id: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    match (extension_id, action_id) {
        ("myplugin.hosts", "list") => handle_list(db, tenant_id, params).await,
        ("myplugin.hosts", "discover") => handle_discover(db, tenant_id, params).await,
        _ => Err(format!("unknown action '{action_id}' for extension '{extension_id}'")),
    }
}
```

### Step 3: Wire into the plugin registry

In the `PluginOps` implementation for `PluginRegistry`:

- Return manifests from `extension_manifests()`.
- Route actions in `handle_extension_action()` based on extension ID prefix.

The route handler passes an `ExtensionActionContext` (DB connection, tenant ID) to
`handle_extension_action()`. `Ok(Value)` maps to HTTP 200; `Err(String)` maps to HTTP 422.

See the [Proxmox VE plugin](proxmox-plugin.md) for a complete working example.

## Service-initiated extension invocation

Services can invoke controller-side plugin actions via `ServiceExtensionProxy`. This
enables cross-plugin coordination without direct crate dependencies.

### Setup

Add `ServiceExtensionProxy` to your handler struct and wire the response callback:

```rust
use uptrakit_service_sdk::ServiceExtensionProxy;

struct MyHandler {
    extension_proxy: Arc<ServiceExtensionProxy>,
    bg_tx: mpsc::Sender<ServiceMessage>,
    // ...
}

impl ServiceHandler for MyHandler {
    fn on_extension_response(&mut self, response: ExtensionResponsePayload) {
        let request_id = response.request_id.clone();
        self.extension_proxy.complete(&request_id, response);
    }
}
```

### Invoking a controller-side action

Use the `invoke()` → send → `wait()` pattern:

```rust
let pending = proxy.invoke("proxmox.hosts", "list-all-unmatched", json!({}));

// Send the request through the background channel
bg_tx.send(pending.message.clone()).await?;

// Wait for the response with a timeout
let response = pending.wait(&proxy, Duration::from_secs(15)).await?;
```

The `PendingExtensionRequest` contains the `ServiceMessage::ExtensionRequest` to send
and a oneshot receiver for the response. The `bg_tx` channel flows through
`poll_service_event` → `on_service_event` → `conn.send()`.

### Graceful degradation

If the target plugin is not installed or the action fails, the controller returns an
error response. Services should handle this gracefully — for example, by returning an
empty options list for a dropdown source action.

### Wire messages

| Direction | Message | Purpose |
| --- | --- | --- |
| Service → Controller | `ServiceMessage::ExtensionRequest` | Service requests a controller-side plugin action |
| Controller → Service | `ControllerMessage::ExtensionResponse` | Controller returns the plugin action result |

Both messages reuse the existing `ExtensionRequestPayload` and `ExtensionResponsePayload`
types. They are **not** NATS-publishable (session-targeted).

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
| 422 | Plugin-backed action failed (includes error message) |
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

For a complete implementation example, see the SSH agent's `extension.rs` module
(`crates/core/agent-ssh/src/extension.rs`) which implements the `ssh-agent.hosts` extension
with list, bootstrap, and remove actions including ECIES E2E encryption for sensitive parameters.

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

## CLI integration

The CLI supports both static commands (`extensions list`, `extensions invoke`)
and **dynamic manifest-driven invocation** (`extensions <id> <action> [--args]`).

### Dynamic command building

When the user runs `extensions <extension_id> <action>`, the CLI:

1. Fetches the extension list from the server via `list_extensions()`.
2. Finds the matching manifest and its resolved action catalogue.
3. Builds a `clap::Command` dynamically from the manifest's UI definition.
4. Parses the remaining args against the generated command.
5. Dispatches the action (see below).

### Context selector injection

Extensions with a `context_selector` on their `DataTable` UI (e.g., the
Proxmox plugin's `plugin_config_id` selector) expose the selector's `param_key`
as a global CLI flag. The key is converted to kebab-case for the CLI
(e.g., `plugin_config_id` becomes `--plugin-config-id`). The value is injected
into every action's params automatically.

### `api_submit` dispatch

Actions with an `api_submit` target are designed for direct REST API calls
rather than the extension proxy. The CLI detects `api_submit` on the matched
`ActionDef` and calls `UptrakitClient::raw_request()` with a rendered body
template instead of routing through `invoke_extension_action()`.

The template substitution supports four coercion types:

| Syntax | Effect |
| --- | --- |
| `{{key}}` | String (default) |
| `{{key:bool}}` | `"true"` becomes `true`, anything else `false` |
| `{{key:csv_array}}` | Split on `,`, trim whitespace, drop empties, produce JSON array |
| `{{key:number}}` | Parse as JSON number (`i64` first, then `f64`) |

Non-template strings and non-string JSON leaves pass through unchanged.

### Targeted vs Universal extensions

For `Targeted` extensions, the CLI adds a global `--service-id <UUID>` flag
and validates its presence before dispatch. For `Universal` extensions (including
all plugin-backed extensions), no `--service-id` is needed — the server selects
a provider automatically or handles it directly (for plugins).

## Key files

| File | Purpose |
| --- | --- |
| `crates/shared/extension-framework/src/lib.rs` | Extension manifest types, wire payloads, `ApiSubmitDef`, `ContextSelectorDef` (`uptrakit-extension-framework`) |
| `crates/shared/types/src/permissions.rs` | `Permission` enum (shared across plugins and web API) |
| `crates/shared/wire/src/limits.rs` | Wire validation limits |
| `crates/shared/wire/src/wire_validate_impls.rs` | `WireValidate` implementations |
| `crates/ui/web-api/src/extension_registry.rs` | Extension registry (provider tracking) |
| `crates/ui/web-api/src/extension_proxy.rs` | Request/response proxy (oneshot channels) |
| `crates/ui/web-api/src/routes/extensions.rs` | REST API route handlers |
| `crates/shared/web-api-types/src/extensions.rs` | REST API request/response types |
| `crates/shared/openapi-client/src/extensions.rs` | OpenAPI client methods |
| `crates/shared/service-sdk/src/lifecycle.rs` | `ServiceHandler::on_extension_request` + `on_extension_response` |
| `crates/shared/service-sdk/src/extension_proxy.rs` | `ServiceExtensionProxy` for service-initiated invocations |
| `crates/shared/service-sdk/src/event_loop.rs` | `ExtensionRequest` + `ExtensionResponse` dispatch |
| `crates/plugins/infrastructure/registry/src/lib.rs` | `PluginOps::extension_manifests` |
| `crates/ui/cli/src/commands/extensions.rs` | CLI `extensions` subcommand (static + dynamic) |
| `crates/core/agent-ssh/src/extension.rs` | SSH agent extension implementation (reference) |
| `crates/shared/crypto/src/ecies.rs` | ECIES sealed-box encryption/decryption (Rust, backend) |
| `frontend/src/lib/api.ts` | `sealedBoxEncrypt` — Web Crypto API ECIES (frontend) |
| `frontend/src/lib/components/extensions/ServiceSelector.svelte` | Service selector; exposes `selectedEncryptionKey` bindable |
| `frontend/src/lib/components/extensions/ActionButton.svelte` | Sensitive field separation and encryption before invocation |
| `frontend/src/lib/components/extensions/SchemaTable.svelte` | Propagates `encryptionPublicKey` to child `ActionButton` |

## Cross-references

- [Extensions Architecture](../architecture/extensions.md)
- [Extensions Security](../security/extensions.md)
- [Extensions API Reference](../api/extensions.md)
- [Extensions End-User Guide](../end-user/extensions.md)
- [Plugin Guidelines](plugin-guidelines.md)
- [Coding Standards](coding-standards.md)
- [Error Handling](error-handling.md)
- [Testing](testing.md)
