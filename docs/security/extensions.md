# Extension Security

## Overview

The UI extensions framework allows plugins and connected services to contribute UI elements
and expose actions through the controller. This document covers the permission model, input
validation, trust boundaries, timeout enforcement, and conflict detection.

## Permission model

### Manifest-level permissions

Each `ExtensionManifest` declares a `required_permission` field that gates visibility and
access:

- **Non-empty value**: only users with the specified permission can see the extension in
  the UI and invoke its actions.
- **Empty string**: any authenticated user can see and interact with the extension.

The `GET /api/v1/extensions` endpoint filters the returned manifests based on the
authenticated user's permissions. Extensions the user cannot access are never sent to
the frontend.

### Action-level permissions

Individual `ActionDef` entries can declare their own `permission` field. This allows
extensions to expose read-only views to a broad audience while restricting destructive
actions to privileged users.

### Permission enforcement points

| Layer | What is checked |
| --- | --- |
| `GET /api/v1/extensions` | Filters manifests by `required_permission` |
| `GET /api/v1/extensions/{id}/providers` | Checks the extension's `required_permission` |
| `POST /api/v1/extensions/{id}/actions/{action_id}` | Checks both the extension's and the action's permissions |
| Frontend | Hides extensions and actions the user cannot access (defense-in-depth) |

All permission checks happen server-side in the route handlers. Frontend filtering is a
UX convenience, not a security boundary.

## Input validation

### WireValidate limits

All extension wire payloads are validated post-deserialization via the `WireValidate` trait.
This prevents processing-cost attacks within the 1 MB WebSocket frame limit.

| Limit | Value | What it protects |
| --- | --- | --- |
| `MAX_EXTENSION_MANIFESTS` | 50 | Manifests per registration message |
| `MAX_EXTENSION_COLUMNS` | 50 | Columns per table |
| `MAX_EXTENSION_ACTIONS` | 50 | Actions per extension |
| `MAX_EXTENSION_FIELDS` | 100 | Fields per form |
| `MAX_EXTENSION_WIZARD_STEPS` | 20 | Steps per wizard |
| `MAX_EXTENSION_SELECT_OPTIONS` | 200 | Options per select field |
| `MAX_EXTENSION_PARAMS_LEN` | 64 KB | Action request parameters |
| `MAX_EXTENSION_RESPONSE_LEN` | 1 MB | Action response data |

String fields use the standard wire limits (`MAX_SHORT_STRING_LEN` = 1024 bytes,
`MAX_MEDIUM_STRING_LEN` = 4096 bytes).

### REST input validation

The `POST /api/v1/extensions/{id}/actions/{action_id}` endpoint validates:

- The extension ID exists in the registry.
- The action ID exists in the extension's manifest.
- The `service_id` query parameter (when present) is a valid UUID and an actual provider.
- The request body JSON does not exceed `MAX_EXTENSION_PARAMS_LEN`.

## Proxy trust boundaries

### Controller as sole mediator

The controller mediates all communication between the frontend and service instances.
There is no direct frontend-to-service communication path.

```text
Frontend ──REST──> Controller ──WS──> Service
Frontend <──REST── Controller <──WS── Service
```

Key properties:

- **Services cannot push unsolicited data to the frontend.** Services only respond to
  requests proxied by the controller.
- **The frontend cannot address services directly.** All routing goes through the
  extension registry and proxy.
- **The controller validates all inputs** before forwarding to services (permission checks,
  provider validation, payload size limits).

### Service responses are untrusted

The controller returns service response data (`ExtensionResponsePayload.data`) to the
frontend as opaque JSON. The frontend renders this data through schema-driven components
that treat all values as display data, not executable content.

The controller does not interpret or transform the response data beyond checking the
`success` flag and `error` message.

### Registration trust

Extension registration is only accepted from services that declared the `UiExtensions`
capability during enrollment. The capability check prevents services from injecting
extensions without explicit opt-in.

The `service_app_name` conflict check (see below) provides an additional layer of
protection against accidental or malicious extension ID collisions.

## Action timeout enforcement

All action invocations are wrapped in `tokio::time::timeout`:

- **Default timeout**: 30 seconds.
- **Per-action override**: `ActionDef.timeout_seconds` allows extensions to declare longer
  timeouts for operations that legitimately take more time.
- **On timeout**: the pending request is cleaned up, and the REST endpoint returns
  `504 Gateway Timeout`.

Timeouts prevent a misbehaving or unresponsive service from holding controller resources
indefinitely. The oneshot channel pattern ensures exactly one response per request -- late
responses after timeout are silently dropped.

## No direct service-to-frontend communication

The extension framework enforces a strict proxy model:

1. **Frontend to controller**: standard REST API calls.
2. **Controller to service**: `ControllerMessage::ExtensionRequest` over the existing
   WebSocket connection (mTLS-protected).
3. **Service to controller**: `ServiceMessage::ExtensionResponse` over the same WebSocket.
4. **Controller to frontend**: HTTP response to the original REST call.

Services have no knowledge of frontend sessions, user identities, or browser connections.
The controller strips all user context before forwarding requests and adds only the
action parameters.

## `service_app_name` conflict detection

The `service_app_name` field (derived from `env!("CARGO_PKG_NAME")` at compile time)
prevents different service binaries from registering the same extension ID:

| Scenario | Result |
| --- | --- |
| SSH agent A registers `"ssh-host-mgmt"`, SSH agent B registers `"ssh-host-mgmt"` | Allowed (same app name, B added as provider) |
| SSH agent registers `"ssh-host-mgmt"`, MQTT service registers `"ssh-host-mgmt"` | Rejected (`ErrorCode::BadRequest`) |

This prevents:

- **Accidental collisions**: two different services using the same extension ID by mistake.
- **Extension hijacking**: a compromised or misconfigured service registering an extension
  ID that belongs to a different service type.

The conflict check happens at registration time. The rejected service receives an error
message identifying the conflict.

## Key files

| File | Purpose |
| --- | --- |
| `crates/shared/extension-framework/src/lib.rs` | `required_permission` field on manifests and actions (`uptrakit-extension-framework`) |
| `crates/shared/wire/src/limits.rs` | Wire validation limits for extension payloads |
| `crates/shared/wire/src/wire_validate_impls.rs` | `WireValidate` implementations |
| `crates/ui/web-api/src/routes/extensions.rs` | Permission checks in route handlers |
| `crates/ui/web-api/src/extension_registry.rs` | `service_app_name` conflict detection |
| `crates/ui/web-api/src/extension_proxy.rs` | Timeout enforcement via oneshot channels |
| `crates/ui/web-api/src/routes/service_ws/handler/mod.rs` | Capability check on registration |

## See also

- [Extensions Development](../development/extensions.md) -- manifest schema, how to create
  extensions, action protocol, wire limits
- [Extensions Architecture](../architecture/extensions.md) -- registry design, proxy mechanism,
  data flow
- [Extensions API Reference](../api/extensions.md) -- endpoint reference, error codes
- [Auth and Authorization](auth-and-authorization.md) -- authentication methods, RBAC
- [Secure Development](secure-development.md) -- secure coding expectations
