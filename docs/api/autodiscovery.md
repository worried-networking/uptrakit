# Autodiscovery API Reference

This document covers all API endpoints related to the autodiscovery feature: triggering discovery
runs and managing software ignore rules.

For a user-facing explanation of how autodiscovery works and the typical workflow,
see [Autodiscovery (End-user Guide)](../end-user/autodiscovery.md).

For the underlying data model, see [Software Item Entity](../architecture/software-item-entity.md)
and [Unified Software Tracking](../architecture/unified-software-tracking.md).

## Endpoints

### `POST /api/v1/hosts/{id}/discover`

Trigger an autodiscovery run for a specific host. The controller dispatches discovery requests to
all connected agents linked to this host. Each agent queries its discovery-capable plugins and
returns results.

**Permission:** `trigger_checks`

**Path parameters:**

| Parameter | Type | Description |
| --- | --- | --- |
| `id` | UUID | Host UUID |

**Request body:** none

**Response `200`:**

```json
{
  "plugins_queued": 2,
  "message": "Discovery triggered for 2 plugin(s)"
}
```

| Field | Type | Description |
| --- | --- | --- |
| `plugins_queued` | integer | Number of plugin configs queued for discovery |
| `message` | string | Human-readable confirmation |

**Error responses:**

| Status | Condition |
| --- | --- |
| `404` | Host not found or not active |

---

### `POST /api/v1/plugin-configs/{id}/discover`

Trigger an autodiscovery run for a specific plugin config across all connected agents.
Returns an error if the plugin type does not support the `DiscoverLocalSoftware` capability.

**Permission:** `trigger_checks`

**Path parameters:**

| Parameter | Type | Description |
| --- | --- | --- |
| `id` | UUID | Plugin config UUID |

**Request body:** none

**Response `200`:**

```json
{
  "plugins_queued": 3,
  "message": "Discovery triggered for 3 agent(s)"
}
```

| Field | Type | Description |
| --- | --- | --- |
| `plugins_queued` | integer | Number of agents queued for discovery |
| `message` | string | Human-readable confirmation |

**Error responses:**

| Status | Condition |
| --- | --- |
| `400` | Plugin type does not support software discovery |
| `404` | Plugin config not found or not active |

---

## Software Ignore Rules

### `GET /api/v1/software-ignores`

List software ignore rules for the current tenant. Ignore rules suppress software items
by name from being created in future discovery runs. Rules can be tenant-wide or
host-specific.

**Permission:** `view_software`

**Query parameters:**

| Parameter | Type | Default | Description |
| --- | --- | --- | --- |
| `page` | integer | `1` | Page number (1-indexed) |
| `per_page` | integer | `20` | Items per page (max 1000) |

**Response `200`:** Paginated list of ignore rules

```json
{
  "items": [
    {
      "id": "019...",
      "host_id": null,
      "plugin_config_id": null,
      "name": "telnet",
      "package_identifier": null,
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
| `host_id` | UUID or null | Host UUID for host-specific rules; `null` for tenant-wide rules |
| `plugin_config_id` | UUID or null | Plugin config scope (host-specific rules) |
| `name` | string or null | Software item name to suppress (tenant-wide rules) |
| `package_identifier` | string or null | Package identifier to suppress (host-specific rules) |
| `created_at` | ISO 8601 datetime | When the rule was created |

---

### `POST /api/v1/software-ignores`

Create an ignore rule to suppress a software item from future discovery runs. This endpoint is
idempotent: if a matching rule already exists, the existing rule is returned rather than creating
a duplicate.

Two types of rules are supported:

- **Tenant-wide** (no `host_id`): suppresses software by name across all hosts and plugins.
- **Host-specific** (with `host_id`): suppresses a specific package on a specific host.

**Permission:** `manage_ignores`

**Request body (tenant-wide):**

```json
{
  "name": "telnet"
}
```

**Request body (host-specific):**

```json
{
  "host_id": "019...",
  "plugin_config_id": "019...",
  "package_identifier": "nginx"
}
```

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `name` | string | Conditional | Software item name to suppress (required for tenant-wide rules) |
| `host_id` | UUID | Conditional | Host UUID (required for host-specific rules) |
| `plugin_config_id` | UUID | No | Plugin config scope |
| `package_identifier` | string | No | Package identifier scope |

**Response `201`:** Ignore rule response

**Error responses:**

| Status | Condition |
| --- | --- |
| `400` | Missing required fields for the rule type |

---

### `DELETE /api/v1/software-ignores/{id}`

Delete an ignore rule. After deletion, the suppressed software can be re-discovered in future
discovery runs.

**Permission:** `manage_ignores`

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
software item's name.

```text
DELETE /api/v1/software-items/{id}/hosts/{host_id}?ignore=true
```

**Query parameters:**

| Parameter | Type | Default | Description |
| --- | --- | --- | --- |
| `ignore` | boolean | `false` | When `true`, also create an ignore rule for the software item's name |

The ignore rule is scoped to the software item's name and applies tenant-wide -- future discovery
runs on any host will skip that name across all plugin configs and discovery targets.

### Example: unassign a host and suppress the software from re-discovery

```text
DELETE /api/v1/software-items/019.../hosts/019...?ignore=true
```

This is the recommended workflow for software you want to stop tracking **and** prevent from
reappearing in future discovery runs. If you only want to unassign the host without suppressing
rediscovery, omit the `?ignore=true` parameter.

**Response:** `204` No content

**Error responses:**

| Status | Condition |
| --- | --- |
| `404` | Software item or host assignment not found |

---

## Plugin-Driven Discovery Targets

Discovery results use structured `DiscoveryTarget` values instead of opaque `extra` metadata.
Each `DiscoveredSoftware` item carries a `targets` array that tells the controller exactly which
plugin configs and role assignments to create. The controller processes these generically — no
plugin-specific synthesis logic exists in the web-API layer.

### Processing rules

The controller routes each discovered item through one of two paths:

| Condition | Processing |
| --- | --- |
| `targets` is non-empty | For each target: find-or-create the plugin config matching `(plugin_type, plugin_config)`, then create role assignments per `target.roles`. |
| `targets` is empty, `plugin_config_id` is set | Use the discovering plugin's own config for all three roles (`detect_version`, `fetch_releases`, `execute_update`). |

### PHS discovery targets

The PHS plugin (`discovery_proxmox_helper_scripts`) always emits `DiscoveryTarget` values. It analyzes
each container's CT script and builds targets:

| Script analysis result | `DiscoveryTarget` emitted |
| --- | --- |
| GitHub repository detected | `plugin_type: releases_github`, config with `owner`, `repo`, `detect_installed_version_command`, `install_command`. Name: `"{owner}/{repo}"`. |
| APT package detected | `plugin_type: package_manager_apt`, config: `{}`. Name: `"APT (auto)"`. |
| Neither detected | Item skipped (warning logged on agent). |

The PHS plugin config itself is never directly linked to `host_software_item_plugins` — it is used
only as the discovery trigger. All version tracking and update execution happen through the target
configs.

### Homebrew discovery targets

The Homebrew plugin always emits per-item discovery targets:

| Package type | `DiscoveryTarget` emitted |
| --- | --- |
| Formula | `plugin_type: package_manager_homebrew`, config: `{"package_type": "formula"}`. Name: `"Homebrew (Formulae)"`. |
| Cask | `plugin_type: package_manager_homebrew`, config: `{"package_type": "cask"}`. Name: `"Homebrew (Casks)"`. |

When running with an existing config (pre-created with a specific `package_type`), targets are
empty and the controller uses the config-ID path.

### Role plugin rows created by autodiscovery

When autodiscovery processes a target, it creates `host_software_item_plugins` rows for each role
listed in `target.roles` (typically all three):

| Role | Plugin config | Description |
| --- | --- | --- |
| `detect_version` | Target config (e.g. `releases_github` or `package_manager_apt`) | Detects the installed version on the agent host |
| `fetch_releases` | Target config (same as above) | Fetches the latest available upstream version |
| `execute_update` | Target config (same as above) | Executes the actual software update |

For PHS discoveries with a `releases_github` target config, the `fetch_releases` role
typically runs controller-side (via the scheduler) because the GitHub Releases plugin has the
`ControllerSideFetchReleases` capability. The `execution_site` column defaults to `"auto"`, which
lets the system decide based on plugin capabilities.

## Related Documentation

- [Autodiscovery (End-user Guide)](../end-user/autodiscovery.md) — workflow overview and
  user-facing concepts.
- [Software Item Entity](../architecture/software-item-entity.md) — full data model, database
  schema, and existing software item CRUD endpoints.
- [HTTP Web API](http-web-api.md) — common API patterns, pagination, error response format, and
  authentication.
- [Plugin Guidelines](../development/plugin-guidelines.md) — `DiscoverLocalSoftware` plugin
  capability and `discover_software()` method contract.
