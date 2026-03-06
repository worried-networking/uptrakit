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
└── service_index: Mutex<HashMap<Uuid, Vec<String>>>            (reverse map: service_id -> extension_ids)
```

### `ExtensionEntry`

Each runtime extension entry contains:

| Field | Type | Description |
| --- | --- | --- |
| `manifest` | `ExtensionManifest` | The deduplicated manifest |
| `app_name` | `String` | `service_app_name` of the registering service |
| `providers` | `BTreeSet<Uuid>` | Service IDs currently providing this extension |

### Compile-time registration (plugins)

Plugins implement `PluginOps::extension_manifests()` which returns a `Vec<ExtensionManifest>`.
The `PluginRegistry` aggregates manifests from all registered plugins at controller startup.
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

## Extension proxy

The `ExtensionProxy` handles request/response correlation for service-backed action
invocations using oneshot channels.

```text
ExtensionProxy
└── pending: Mutex<HashMap<String, oneshot::Sender<ExtensionResponsePayload>>>
```

### Invocation flow

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
┌──────────────────────────────────────────────────────────────────┐
│                          Controller                              │
│                                                                  │
│  ┌────────────────┐     ┌───────────────────┐                    │
│  │ PluginRegistry │────>│ ExtensionRegistry  │                    │
│  │ (compile-time) │     │                   │                    │
│  └────────────────┘     │ plugin_extensions  │                    │
│                         │ service_extensions │                    │
│  ┌────────────────┐     │ service_index      │                    │
│  │ WS Handler     │────>│                   │                    │
│  │ (register/     │     └───────┬───────────┘                    │
│  │  unregister)   │             │                                │
│  └────────────────┘             │ all_manifests()                │
│                                 │ find_owner()                   │
│  ┌────────────────┐             │ providers()                    │
│  │ REST Handlers  │<────────────┘                                │
│  │ GET /extensions│                                              │
│  │ GET /providers │     ┌───────────────────┐                    │
│  │ POST /actions  │────>│ ExtensionProxy    │                    │
│  └────────────────┘     │ (oneshot channels)│                    │
│                         └───────┬───────────┘                    │
│                                 │                                │
│  ┌────────────────┐             │ invoke()                       │
│  │ WS Handler     │<────────────┘ complete()                     │
│  │ (send/receive) │                                              │
│  └────────────────┘                                              │
│                                                                  │
└──────────────────────────────────────────────────────────────────┘
         │                                         │
    REST API                                  WebSocket
         │                                         │
    ┌────▼────┐                              ┌─────▼─────┐
    │Frontend │                              │ Service   │
    │ (SPA)   │                              │ Instance  │
    └─────────┘                              └───────────┘
```

## Frontend rendering

The frontend uses a schema-driven approach. Extension manifests describe the UI structure
and the frontend renders appropriate components:

| `ExtensionUi` variant | Frontend component | Description |
| --- | --- | --- |
| `data_table` | `SchemaTable.svelte` | Fetches rows via action, renders columns and actions |
| `form` | `SchemaForm.svelte` | Typed inputs from `FieldDef`, client-side validation |
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

| Message | Publishable | Reason |
| --- | --- | --- |
| `ExtensionRegister` | No | Only relevant to the local controller instance |
| `ExtensionRequest` | Yes | No secrets in request payload |
| `ExtensionResponse` | Yes | No secrets in response payload |

## Key files

| File | Purpose |
| --- | --- |
| `crates/shared/wire/src/extension.rs` | Manifest types and wire payloads |
| `crates/shared/wire/src/lib.rs` | Wire message variants, `UiExtensions` capability |
| `crates/ui/web-api/src/extension_registry.rs` | Registry data structure |
| `crates/ui/web-api/src/extension_proxy.rs` | Oneshot-channel proxy |
| `crates/ui/web-api/src/app_state.rs` | `AppState` fields |
| `crates/ui/web-api/src/routes/extensions.rs` | REST handlers |
| `crates/ui/web-api/src/routes/service_ws/handler/mod.rs` | WS message handling |
| `crates/shared/service-sdk/src/lifecycle.rs` | `ServiceHandler` trait |
| `crates/shared/service-sdk/src/event_loop.rs` | Event loop dispatch |
| `crates/plugins/infrastructure/registry/src/lib.rs` | `PluginOps` trait |
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
