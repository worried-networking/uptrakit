# Extensions Architecture

This document describes the architecture of the UI extensions framework: the extension registry,
the proxy mechanism, compile-time vs runtime registration, the targeting model, capability
negotiation, and data flow.

## Design goals

- **Generic framework**: no extension-specific code in the controller or frontend. All
  rendering is schema-driven from `ExtensionManifest` data.
- **Hybrid registration**: compile-time for plugins (always available), runtime for services
  (dynamic, lifecycle-aware).
- **Proxy isolation**: the controller mediates all communication between the frontend and
  service instances. Services never communicate directly with the frontend.
- **Forward compatibility**: all manifest types are `#[non_exhaustive]` with `Other(String)`
  catch-all variants where appropriate.

## Extension registry

The `ExtensionRegistry` is the central data structure that tracks all active extensions
and their providers.

```text
ExtensionRegistry
├── plugin_extensions: Vec<ExtensionManifest>       (compile-time, immutable)
├── service_extensions: Mutex<HashMap<String, ExtensionEntry>>  (runtime, keyed by extension_id)
├── service_index: Mutex<HashMap<Uuid, Vec<String>>>            (reverse map: service_id -> extension_ids)
└── encryption_keys: Mutex<HashMap<Uuid, String>>               (per-instance P-256 public keys for ECIES)
```

### `ExtensionEntry`

Each runtime extension entry contains:

| Field | Type | Description |
| --- | --- | --- |
| `manifest` | `ExtensionManifest` | The deduplicated manifest |
| `app_name` | `String` | `service_app_name` of the registering service |
| `providers` | `BTreeSet<Uuid>` | Service IDs currently providing this extension |

### Compile-time registration (plugins and notifications)

Plugins implement `PluginOps::extension_manifests()` which returns a `Vec<ExtensionManifest>`.
The `PluginRegistry` aggregates manifests from all registered plugins (including notification
plugins) at controller startup. Notification plugins define their own extension manifests and
action handlers in per-plugin `extensions.rs` modules.
These are stored in `plugin_extensions` and are always available -- no provider tracking
is needed because plugins run in-process.

### Runtime registration (services)

When a service connects and sends `ServiceMessage::ExtensionRegister`:

1. The WS handler validates the payload via `WireValidate`.
2. The handler reads `service_app_name` from the connection state (set during enrollment).
3. `ExtensionRegistry::register_service()` is called with the service ID, app name, and
   manifests.
4. For each manifest:
   - If the extension ID already exists with a **different** app name: registration is
     rejected with `ErrorCode::BadRequest`.
   - If the extension ID already exists with the **same** app name: the service ID is added
     to the `providers` set (deduplication).
   - If the extension ID is new: a new `ExtensionEntry` is created.
5. The `service_index` reverse map is updated.

### Deregistration on disconnect

When a service disconnects:

1. `ExtensionRegistry::unregister_service()` looks up the service ID in `service_index`.
2. The service ID is removed from each extension's `providers` set.
3. Extensions with empty `providers` are removed entirely.
4. The `service_index` entry is removed.

### `service_app_name` conflict detection

The `service_app_name` field (set once at enrollment from `env!("CARGO_PKG_NAME")`) prevents
different service binaries from registering the same extension ID. This catches configuration
errors where two unrelated services accidentally use the same extension identifier.

Same extension ID from services with the same `service_app_name` is expected and handled
via deduplication. Same extension ID from different app names is always an error.

## Extension proxy (controller-side)

The `ExtensionProxy` handles request/response correlation for **frontend-initiated**
action invocations using oneshot channels.

```text
ExtensionProxy
└── pending: Mutex<HashMap<String, oneshot::Sender<ExtensionResponsePayload>>>
```

### Frontend-to-service invocation flow

```text
Frontend                   Controller                     Service
   │                          │                             │
   │ POST /extensions/{id}/   │                             │
   │   actions/{action_id}    │                             │
   │   ?service_id=...        │                             │
   │─────────────────────────>│                             │
   │                          │  1. Generate UUID v7        │
   │                          │  2. Create oneshot channel   │
   │                          │  3. Store sender in pending  │
   │                          │     map (lock + drop)        │
   │                          │                             │
   │                          │  ExtensionRequest ──WS──>   │
   │                          │                             │
   │                          │  <──WS── ExtensionResponse  │
   │                          │                             │
   │                          │  4. complete() sends via     │
   │                          │     oneshot channel          │
   │                          │  5. Receiver gets response   │
   │<─────────────────────────│                             │
```

### Timeout handling

The proxy wraps the oneshot receiver in `tokio::time::timeout`. On timeout:

1. The pending sender is removed from the map.
2. The REST endpoint returns `504 Gateway Timeout`.
3. If the service responds late, the oneshot sender is already dropped -- the response
   is silently discarded.

### Disconnect handling

If the service disconnects while an action is pending, the oneshot sender remains in the
map until the timeout fires. The proxy does not actively cancel pending requests on
disconnect (the timeout handles cleanup).

## Extension proxy (service-side)

The `ServiceExtensionProxy` (in `uptrakit-service-sdk`) mirrors the controller-side
`ExtensionProxy` pattern but runs inside a service binary. It enables **services to
invoke controller-side plugin actions** via the WebSocket connection.

```text
ServiceExtensionProxy
└── pending: Mutex<HashMap<String, oneshot::Sender<ExtensionResponsePayload>>>
```

### Service-to-controller invocation flow

```text
Service                    Controller                     Plugin
   │                          │                             │
   │ ServiceMessage::          │                             │
   │   ExtensionRequest       │                             │
   │─────────────(WS)────────>│                             │
   │                          │  1. Extract tenant_id       │
   │                          │  2. Look up plugin by       │
   │                          │     extension_id prefix     │
   │                          │  3. Dispatch to             │
   │                          │     handle_extension_action │
   │                          │                        ────>│
   │                          │                        <────│
   │                          │                             │
   │ ControllerMessage::       │                             │
   │   ExtensionResponse      │                             │
   │<────────────(WS)─────────│                             │
   │                          │                             │
   │ 4. complete() sends via   │                             │
   │    oneshot channel        │                             │
```

### Usage pattern

Services use `ServiceExtensionProxy::invoke()` to create a `PendingExtensionRequest`
containing the `ServiceMessage::ExtensionRequest` to send and a oneshot receiver. The
service sends the message through its `bg_tx` channel (which flows through the event
loop to `conn.send()`), then awaits the receiver with a timeout via `pending.wait()`.

When the controller responds with `ControllerMessage::ExtensionResponse`, the service's
`on_extension_response` handler calls `proxy.complete()` to deliver the response through
the oneshot channel.

### Use case: cross-plugin coordination

This mechanism enables services to query controller-side plugins they do not directly
depend on. For example, the SSH agent uses `ServiceExtensionProxy` to:

1. Invoke `proxmox.hosts/list-all-unmatched` to populate a dropdown of discoverable
   Proxmox guests (the Proxmox plugin runs only on the controller).
2. Invoke `proxmox.hosts/match` to auto-match a guest after successful bootstrap.

If the target plugin is not installed, the controller returns an error response and
the service handles it gracefully (e.g., returning an empty options list).

## Targeting model

### Universal extensions

Any connected instance of the source service type can handle actions. The registry's
`pick_provider()` method selects a provider -- preferring a `preferred` hint if given,
otherwise returning the first available.

The frontend does not show a service selector for universal extensions.

### Targeted extensions

Actions must be routed to a specific service instance. The REST endpoint requires a
`service_id` query parameter. The frontend fetches the provider list via
`GET /api/v1/extensions/{id}/providers` and shows a dropdown selector.

The proxy validates that the specified `service_id` is actually a provider of the
requested extension before forwarding.

## Capability negotiation

The `UiExtensions` capability (wire string: `"ui_extensions"`) gates extension support:

- Services must include `Capability::UiExtensions` in their enrollment payload.
- The controller only processes `ExtensionRegister` and `ExtensionResponse` messages from
  services that declared this capability.
- Services without the capability that send extension messages receive no response (the
  messages are ignored).

## Data flow diagram

```text
┌───────────────────────────────────────────────────────────────────────┐
│                            Controller                                 │
│                                                                       │
│  ┌────────────────┐     ┌───────────────────┐                         │
│  │ PluginRegistry │────>│ ExtensionRegistry  │                         │
│  │ (compile-time) │     │                   │                         │
│  └────────────────┘     │ plugin_extensions  │                         │
│                         │ service_extensions │                         │
│  ┌────────────────┐     │ service_index      │                         │
│  │ WS Handler     │────>│                   │                         │
│  │ (register/     │     └───────┬───────────┘                         │
│  │  unregister)   │             │                                     │
│  └────────────────┘             │ all_manifests()                     │
│                                 │ find_owner()                        │
│  ┌────────────────┐             │ providers()                         │
│  │ REST Handlers  │<────────────┘                                     │
│  │ GET /extensions│                                                   │
│  │ GET /providers │     ┌───────────────────┐                         │
│  │ POST /actions  │────>│ ExtensionProxy    │  (frontend → service)   │
│  └────────────────┘     │ (oneshot channels)│                         │
│                         └───────┬───────────┘                         │
│                                 │ invoke() / complete()               │
│  ┌────────────────┐             │                                     │
│  │ WS Handler     │<────────────┘                                     │
│  │ (send/receive) │────────────────────────────────────┐              │
│  └────────────────┘                                    │              │
│         │                                              │              │
│         │ ServiceMessage::ExtensionRequest              │              │
│         │ (service → controller plugin)                │              │
│         ▼                                              │              │
│  ┌────────────────┐     ┌───────────────────┐          │              │
│  │ Plugin dispatch │────>│ PluginRegistry    │          │              │
│  │ (in WS handler)│     │ handle_extension_ │          │              │
│  └────────────────┘     │ action()          │          │              │
│         │               └───────────────────┘          │              │
│         │ ControllerMessage::ExtensionResponse          │              │
│         └──────────────────────────────────────────────┘              │
│                                                                       │
└───────────────────────────────────────────────────────────────────────┘
         │                                         │
    REST API                                  WebSocket
         │                                    (bidirectional)
    ┌────▼────┐                              ┌─────▼─────┐
    │Frontend │                              │ Service   │
    │ (SPA)   │                              │ Instance  │
    └─────────┘                              └───────────┘
```

## Frontend rendering

The frontend uses a schema-driven approach. Extension manifests describe the UI structure
and the frontend renders appropriate components. All extension components use the same
Skeleton UI classes and shared components (e.g. `Pagination`, `Modal`, `ConfirmDialog`) as
built-in pages to ensure a consistent look and feel:

| `ExtensionUi` variant | Frontend component | Description |
| --- | --- | --- |
| `data_table` | `SchemaTable.svelte` | Fetches paginated rows via action, renders columns with `table-wrap`/`table` classes, row actions, and shared `Pagination` component |
| `form` | `SchemaForm.svelte` | Typed inputs from `FieldDef` using Skeleton's `.label`/`.input`/`.select` classes |
| `key_value` | `SchemaKeyValue.svelte` | Read-only key-value display from action response |
| `actions` | `ExtensionContextMenuItems.svelte` | Action buttons/menu items |

Composite components:

- `ExtensionPage.svelte` -- loads manifest, renders the appropriate UI component, shows
  service selector for targeted extensions
- `ExtensionPanel.svelte` -- same but for panel placement (injected into existing pages)
- `SchemaWizard.svelte` -- multi-step form with step indicator and per-step submission
- `ActionButton.svelte` -- renders an action as a button, opens form/wizard modal if needed
- `ServiceSelector.svelte` -- dropdown for selecting a provider instance

### Extension store

A shared reactive store (`extensions.svelte.ts`) loads extensions once on authentication
and provides filtered views:

- `getPageExtensions()` -- extensions with `page` placement
- `getPanelExtensions(targetPage)` -- panels for a specific page
- `getContextMenuExtensions(targetEntity)` -- context menu groups for an entity type
- `getTableExtensions(targetTable)` -- extra columns for a table

### Dynamic navigation

Page extensions are injected into the sidebar navigation dynamically, filtered by user
permissions (same pattern as static nav items).

## NATS publishability

| Message | Direction | Publishable | Reason |
| --- | --- | --- | --- |
| `ExtensionRegister` | Service → Controller | No | Only relevant to the local controller instance |
| `ControllerMessage::ExtensionRequest` | Controller → Service | No | Session-targeted (routed to a specific service's WS) |
| `ServiceMessage::ExtensionResponse` | Service → Controller | No | Session-targeted (response to a specific proxy request) |
| `ServiceMessage::ExtensionRequest` | Service → Controller | No | Session-targeted (service requesting its own controller) |
| `ControllerMessage::ExtensionResponse` | Controller → Service | No | Session-targeted (response to a specific service request) |

All extension request/response messages are session-targeted — they must be delivered
to the specific WebSocket connection that originated or should receive them. They are
**not** safe for NATS publication.

## Key files

| File | Purpose |
| --- | --- |
| `crates/shared/extension-framework/src/lib.rs` | Extension manifest types and wire payloads (`uptrakit-extension-framework`) |
| `crates/shared/wire/src/lib.rs` | Wire message variants, `UiExtensions` capability |
| `crates/ui/web-api/src/extension_registry.rs` | Registry data structure |
| `crates/ui/web-api/src/extension_proxy.rs` | Controller-side oneshot-channel proxy (frontend → service) |
| `crates/shared/service-sdk/src/extension_proxy.rs` | Service-side oneshot-channel proxy (service → controller plugin) |
| `crates/ui/web-api/src/app_state.rs` | `AppState` fields |
| `crates/ui/web-api/src/routes/extensions.rs` | REST handlers |
| `crates/ui/web-api/src/routes/service_ws/handler/mod.rs` | WS message handling (both directions) |
| `crates/shared/service-sdk/src/lifecycle.rs` | `ServiceHandler` trait (`on_extension_request` + `on_extension_response`) |
| `crates/shared/service-sdk/src/event_loop.rs` | Event loop dispatch |
| `crates/plugins/infrastructure/core/src/plugin_ops.rs` | `PluginOps` trait (feature `plugin-ops`) |
| `frontend/src/lib/components/extensions/` | Schema-driven Svelte components |
| `frontend/src/lib/extensions.svelte.ts` | Extension store |

## See also

- [Extensions Development](../development/extensions.md) -- manifest schema, how to create
  extensions, action protocol
- [Extensions Security](../security/extensions.md) -- permission model, trust boundaries,
  input validation
- [Extensions API Reference](../api/extensions.md) -- REST endpoint reference
- [Extensions End-User Guide](../end-user/extensions.md) -- UI walkthrough
- [SSH-Backed Agent Architecture](ssh-agent.md) -- example service that will use extensions
