# Extensions API

The extensions API exposes three endpoints for listing active extensions, querying extension
providers, and invoking extension actions. All endpoints require authentication.

## Endpoints

### `GET /api/v1/extensions`

Lists all active extension manifests, filtered by the authenticated user's permissions. Each
manifest includes provider count (number of connected service instances) and targeting mode.

**Required permission:** any authenticated user (per-manifest filtering by `required_permission`)

#### Query parameters

None.

#### Response

```json
[
  {
    "manifest": {
      "id": "ssh-agent.host-management",
      "label": "SSH Host Management",
      "placement": {
        "type": "page",
        "nav_section": "management",
        "icon": "server"
      },
      "required_permission": "manage_hosts",
      "targeting": "targeted",
      "ui": {
        "type": "data_table",
        "columns": [
          { "key": "hostname", "label": "Hostname", "sortable": true }
        ],
        "data_action": "list-hosts"
      }
    },
    "provider_count": 2
  }
]
```

#### Example

```sh
# List all extensions visible to the authenticated user
curl -H "Authorization: Bearer $TOKEN" https://uptrakit.example.com/api/v1/extensions
```

---

### `GET /api/v1/extensions/{extension_id}/providers`

Lists connected service instances that provide the specified extension. Used by the frontend
to show a service selector for `targeted` extensions.

**Required permission:** same as the extension's `required_permission`

#### Path parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `extension_id` | string | Extension identifier (e.g., `ssh-agent.host-management`) |

#### Response

```json
[
  {
    "service_id": "019585f4-1234-7000-8000-000000000001",
    "service_label": "SSH Agent (prod-01)",
    "hostname": "prod-01.example.com"
  },
  {
    "service_id": "019585f4-1234-7000-8000-000000000002",
    "service_label": "SSH Agent (prod-02)",
    "hostname": "prod-02.example.com"
  }
]
```

#### Example

```sh
# List providers for a targeted extension
curl -H "Authorization: Bearer $TOKEN" \
  https://uptrakit.example.com/api/v1/extensions/ssh-agent.host-management/providers
```

---

### `POST /api/v1/extensions/{extension_id}/actions/{action_id}`

Invokes an extension action. For service-backed extensions, the controller proxies the request
to a connected service instance over WebSocket and returns the response.

**Required permission:** the action's `permission` field (or the extension's `required_permission`
if the action has no specific permission)

#### Path parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `extension_id` | string | Extension identifier |
| `action_id` | string | Action identifier within the extension |

#### Query parameters

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `service_id` | UUID | conditional | Target service instance. Required for `targeted` extensions. Optional for `universal` extensions (allows explicit routing). |

#### Request body

JSON object with action parameters. The shape depends on the extension's action definition.

```json
{
  "params": {
    "hostname": "web-01.example.com",
    "username": "root"
  }
}
```

Maximum request body size: 64 KB.

#### Response (success)

Data actions return a paginated response:

```json
{
  "items": [
    { "hostname": "web-01", "status": "connected" },
    { "hostname": "web-02", "status": "disconnected" }
  ],
  "total": 42,
  "page": 1,
  "per_page": 20,
  "total_pages": 3
}
```

The response shape is defined by the extension action and is returned as-is from the service.
All `data_table` data actions must return the paginated format shown above (see
[Pagination in Extensions Development](../development/extensions.md#pagination)).

#### Response (action failure)

HTTP 422 with the error message from the service:

```json
{
  "error": "Failed to connect to host: connection refused"
}
```

#### Routing logic

| Extension targeting | `service_id` param | Behaviour |
| --- | --- | --- |
| `targeted` | missing | `400 Bad Request` ("service_id required for targeted extension") |
| `targeted` | present | Proxy to the specified service instance |
| `universal` | missing | Controller picks any available provider |
| `universal` | present | Proxy to the specified instance (explicit routing) |

#### Example

```sh
# Invoke an action on a targeted extension
curl -X POST \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"params": {"hostname": "web-01"}}' \
  "https://uptrakit.example.com/api/v1/extensions/ssh-agent.host-management/actions/list-hosts?service_id=019585f4-1234-7000-8000-000000000001"

# Invoke an action on a universal extension (auto-routed)
curl -X POST \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"params": {}}' \
  https://uptrakit.example.com/api/v1/extensions/proxmox.lxc-panel/actions/get-lxc-info
```

---

## Error codes

| HTTP Status | Condition |
| --- | --- |
| 200 | Action succeeded |
| 400 | Missing `service_id` for targeted extension, or invalid parameters |
| 403 | Insufficient permissions |
| 404 | Extension or action not found |
| 422 | Action executed but returned an error (includes error message) |
| 501 | Plugin-backed action invocation (not yet implemented) |
| 503 | Target service is disconnected |
| 504 | Action timed out (default: 30s, configurable per action) |

---

## Response schemas

### `ExtensionResponse`

Returned by `GET /api/v1/extensions` (array of these objects):

| Field | Type | Description |
| --- | --- | --- |
| `manifest` | `ExtensionManifest` | Full extension manifest |
| `provider_count` | integer | Number of connected service instances providing this extension |

### `ExtensionProviderInfo`

Returned by `GET /api/v1/extensions/{extension_id}/providers` (array of these objects):

| Field | Type | Description |
| --- | --- | --- |
| `service_id` | UUID | Service instance identifier |
| `service_label` | string | Human-readable service label |
| `hostname` | string or null | Hostname of the service instance |

### `InvokeExtensionActionRequest`

Request body for `POST /api/v1/extensions/{extension_id}/actions/{action_id}`:

| Field | Type | Description |
| --- | --- | --- |
| `params` | JSON object | Action parameters (shape defined by the extension) |

---

## CLI

```sh
# List all extensions
uptrakit extensions list

# List extensions (JSON output)
uptrakit --output json extensions list

# List providers for a specific extension
uptrakit extensions providers ssh-agent.host-management

# Invoke an action
uptrakit extensions invoke ssh-agent.host-management list-hosts

# Invoke with parameters
uptrakit extensions invoke ssh-agent.host-management bootstrap --params '{"hostname": "web-01"}'

# Invoke on a specific service instance
uptrakit extensions invoke ssh-agent.host-management list-hosts --service-id 019585f4-...
```

---

## See also

- [Extensions End-User Guide](../end-user/extensions.md)
- [Extensions Security](../security/extensions.md)
- [Extensions Development](../development/extensions.md)
- [HTTP Web API Overview](http-web-api.md)
