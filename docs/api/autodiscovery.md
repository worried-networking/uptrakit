# Autodiscovery API Reference

This document covers all API endpoints related to the autodiscovery feature: approving discovered
software items, triggering discovery runs, bulk-discarding pending items, and managing ignore rules.

For a user-facing explanation of how autodiscovery works and the typical review workflow,
see [Autodiscovery (End-user Guide)](../end-user/autodiscovery.md).

For the underlying data model, see [Software Item Entity](../architecture/software-item-entity.md).

## Software Item Discovery State

The `discovery_state` field on `SoftwareItemResponse` describes how the item was created
and whether it has been reviewed.

| Value | Meaning | `enabled` value |
| --- | --- | --- |
| `null` | Created manually by a user | `true` |
| `"pending"` | Discovered by an agent, awaiting review | `false` |
| `"approved"` | Discovered item approved by a user | `true` |

`enabled` is always `false` while `discovery_state` is `"pending"`. Approving a pending item sets
both `discovery_state` to `"approved"` and `enabled` to `true`, activating version tracking.

---

## Endpoints

### `POST /api/v1/software-items/{id}/approve`

Approve a pending discovered software item. Sets `discovery_state` to `"approved"` and `enabled`
to `true`, activating version tracking for the item.

**Permission:** `manage_software`

**Path parameters:**

| Parameter | Type | Description |
| --- | --- | --- |
| `id` | UUID | Software item UUID |

**Request body:** none

**Response `200`:** `SoftwareItemResponse`

```json
{
  "id": "019...",
  "name": "git",
  "provider_types": ["homebrew"],
  "enabled": true,
  "discovery_state": "approved",
  "host_count": 1,
  "created_at": "2026-02-23T10:00:00Z",
  "updated_at": "2026-02-23T10:05:00Z"
}
```

**Error responses:**

| Status | Condition |
| --- | --- |
| `404` | Software item not found or not active |
| `409` | Item is not in `pending` state (already approved or manually created) |

---

### `POST /api/v1/hosts/{id}/discover`

Trigger an autodiscovery run for a specific host. The controller dispatches discovery requests to
all connected agents linked to this host. Each agent queries its discovery-capable providers and
returns results.

**Permission:** `manage_software`

**Path parameters:**

| Parameter | Type | Description |
| --- | --- | --- |
| `id` | UUID | Host UUID |

**Request body:** none

**Response `200`:**

```json
{
  "providers_queued": 2,
  "message": "Discovery triggered for 2 provider(s)"
}
```

| Field | Type | Description |
| --- | --- | --- |
| `providers_queued` | integer | Number of provider configs queued for discovery |
| `message` | string | Human-readable confirmation |

**Error responses:**

| Status | Condition |
| --- | --- |
| `404` | Host not found or not active |

---

### `DELETE /api/v1/hosts/{id}/discovered`

Bulk-discard all pending discovered items for a host. Soft-deletes every software item with
`discovery_state = "pending"` that is assigned to this host. No ignore rules are created,
so discarded packages can be re-discovered in future runs.

**Permission:** `manage_software`

**Path parameters:**

| Parameter | Type | Description |
| --- | --- | --- |
| `id` | UUID | Host UUID |

**Query parameters:**

| Parameter | Type | Required | Description |
| --- | --- | --- | --- |
| `provider_config_id` | UUID | No | Limit discard to pending items from a specific provider config |

**Request body:** none

**Response `200`:**

```json
{
  "discarded_count": 14
}
```

| Field | Type | Description |
| --- | --- | --- |
| `discarded_count` | integer | Number of items soft-deleted |

**Error responses:**

| Status | Condition |
| --- | --- |
| `404` | Host not found or not active |

---

### `POST /api/v1/provider-configs/{id}/discover`

Trigger an autodiscovery run for a specific provider config across all connected agents.
Returns an error if the provider type does not support the `DiscoverLocalSoftware` capability.

**Permission:** `manage_software`

**Path parameters:**

| Parameter | Type | Description |
| --- | --- | --- |
| `id` | UUID | Provider config UUID |

**Request body:** none

**Response `200`:**

```json
{
  "providers_queued": 3,
  "message": "Discovery triggered for 3 agent(s)"
}
```

| Field | Type | Description |
| --- | --- | --- |
| `providers_queued` | integer | Number of agents queued for discovery |
| `message` | string | Human-readable confirmation |

**Error responses:**

| Status | Condition |
| --- | --- |
| `400` | Provider type does not support software discovery |
| `404` | Provider config not found or not active |

---

### `DELETE /api/v1/provider-configs/{id}/discovered`

Bulk-discard all pending discovered items for a provider config across all hosts. Soft-deletes
every software item with `discovery_state = "pending"` linked to this provider config. No ignore
rules are created.

**Permission:** `manage_software`

**Path parameters:**

| Parameter | Type | Description |
| --- | --- | --- |
| `id` | UUID | Provider config UUID |

**Request body:** none

**Response `200`:**

```json
{
  "discarded_count": 7
}
```

| Field | Type | Description |
| --- | --- | --- |
| `discarded_count` | integer | Number of items soft-deleted |

**Error responses:**

| Status | Condition |
| --- | --- |
| `404` | Provider config not found or not active |

---

### `GET /api/v1/autodiscovery/ignores`

List autodiscovery ignore rules for the current tenant. Ignore rules suppress specific packages
from being created as pending items in future discovery runs.

**Permission:** `view_software`

**Query parameters:**

| Parameter | Type | Default | Description |
| --- | --- | --- | --- |
| `page` | integer | `1` | Page number (1-indexed) |
| `per_page` | integer | `20` | Items per page (max 1000) |
| `provider_config_id` | UUID | — | Filter results to a specific provider config |

**Response `200`:** Paginated list of ignore rules

```json
{
  "items": [
    {
      "id": "019...",
      "provider_config_id": "019...",
      "provider_config_name": "Homebrew (Formulae)",
      "provider_type": "homebrew",
      "package_identifier": "telnet",
      "created_at": "2026-02-23T10:00:00Z"
    }
  ],
  "total": 1,
  "page": 1,
  "per_page": 20,
  "total_pages": 1
}
```

**Ignore rule fields:**

| Field | Type | Description |
| --- | --- | --- |
| `id` | UUID | Ignore rule UUID |
| `provider_config_id` | UUID | Provider config this rule applies to |
| `provider_config_name` | string | Display name of the provider config |
| `provider_type` | string | Provider type (e.g. `"homebrew"`, `"proxmox_helper_scripts"`) |
| `package_identifier` | string | Package identifier suppressed from discovery |
| `created_at` | ISO 8601 datetime | When the rule was created |

---

### `POST /api/v1/autodiscovery/ignores`

Create an ignore rule to permanently suppress a specific package from future discovery runs.
This endpoint is idempotent: if a rule already exists for the `(provider_config_id,
package_identifier)` pair, the existing rule is returned rather than creating a duplicate.

**Permission:** `manage_software`

**Request body:**

```json
{
  "provider_config_id": "019...",
  "package_identifier": "telnet"
}
```

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `provider_config_id` | UUID | Yes | Provider config to scope the rule to |
| `package_identifier` | string | Yes | Package identifier to suppress (must not be empty) |

**Response `201`:** Ignore rule response (same shape as items returned by
`GET /api/v1/autodiscovery/ignores`)

```json
{
  "id": "019...",
  "provider_config_id": "019...",
  "provider_config_name": "Homebrew (Formulae)",
  "provider_type": "homebrew",
  "package_identifier": "telnet",
  "created_at": "2026-02-23T10:00:00Z"
}
```

**Error responses:**

| Status | Condition |
| --- | --- |
| `400` | `provider_config_id` or `package_identifier` missing or invalid |
| `404` | Provider config not found or not active |

---

### `DELETE /api/v1/autodiscovery/ignores/{id}`

Delete an ignore rule. After deletion, the suppressed package can be re-discovered in future
discovery runs.

**Permission:** `manage_software`

**Path parameters:**

| Parameter | Type | Description |
| --- | --- | --- |
| `id` | UUID | Ignore rule UUID |

**Request body:** none

**Response `204`:** No content

**Error responses:**

| Status | Condition |
| --- | --- |
| `404` | Ignore rule not found |

---

## The `?ignore=true` Query Parameter on `DELETE /api/v1/software-items/{id}/hosts/{host_id}`

The host assignment delete endpoint accepts an optional `ignore` query parameter. When set to
`true`, the operation removes the host assignment **and** atomically creates an ignore rule for the
`(provider_config_id, package_identifier)` pair recorded on that specific host assignment.

```http
DELETE /api/v1/software-items/{id}/hosts/{host_id}?ignore=true
```

**Query parameters:**

| Parameter | Type | Default | Description |
| --- | --- | --- | --- |
| `ignore` | boolean | `false` | When `true`, also create an ignore rule for this host assignment |

The ignore rule is scoped to the `(provider_config_id, package_identifier)` stored on the
`host_software_item` row being deleted, and applies tenant-wide — future discovery runs on any host
will skip that package for that provider config.

### Example: unassign a host and suppress the package from re-discovery

```http
DELETE /api/v1/software-items/019.../hosts/019...?ignore=true
```

This is the recommended workflow for packages you want to stop tracking **and** prevent from
reappearing in future discovery runs. If you only want to unassign the host without suppressing
rediscovery, omit the `?ignore=true` parameter.

**Response:** `204` No content

**Error responses:**

| Status | Condition |
| --- | --- |
| `404` | Software item or host assignment not found |

---

## PHS Discovery and Config Synthesis

When the PHS provider (`proxmox_helper_scripts`) returns discovery results, each item's `extra`
field carries metadata that the controller uses to synthesize a downstream provider config:

| `extra` field | Outcome |
| --- | --- |
| `"github_owner"` + `"github_repo"` | Controller finds or creates a `github_releases` config for `owner/repo` with PHS-specific `detect_installed_version_command` and `install_command`. Software item is linked to this GitHub config. |
| `"apt_package"` | Controller finds or creates a shared `"APT (auto)"` config (`{}`). Software item's `package_identifier` is the Debian package name. |
| Neither | Item is skipped (warned in agent log). |

The PHS provider config itself is never directly linked to `host_software_items` — it is used only
as the discovery trigger. All version tracking and update execution happen through the synthesized
configs.

## Related Documentation

- [Autodiscovery (End-user Guide)](../end-user/autodiscovery.md) — workflow overview and
  user-facing concepts.
- [Software Item Entity](../architecture/software-item-entity.md) — full data model, database
  schema, and existing software item CRUD endpoints.
- [HTTP Web API](http-web-api.md) — common API patterns, pagination, error response format, and
  authentication.
- [Provider Guidelines](../development/provider-guidelines.md) — `DiscoverLocalSoftware` provider
  capability and `discover_software()` method contract.
