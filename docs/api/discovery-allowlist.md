# Discovery Allowlist API Reference

This document covers the API endpoints for managing the discovery plugin allowlist. The allowlist
controls which plugin types participate in automatic host discovery.

For a user-facing explanation of how the allowlist fits into the autodiscovery workflow, see
[Autodiscovery: Controlling Which Plugins Run Discovery](../end-user/autodiscovery.md#controlling-which-plugins-run-discovery).

For the autodiscovery trigger endpoints themselves, see [Autodiscovery API Reference](autodiscovery.md).

## Allowlist Semantics

By default, when no allowlist entries exist, all discovery-capable plugin types run on every host.
Once you add at least one entry to the allowlist, only the listed plugin types will be dispatched
during discovery.

The controller resolves which plugins to run for a given host using the following priority order:

1. **Host-specific allowlist** — if the host has any entries in its own allowlist, only those
   plugin types are used. The tenant-wide allowlist is ignored entirely for that host.
2. **Tenant-wide allowlist** — if the tenant has entries but the host has none, the tenant-wide
   list applies to the host.
3. **Unconfigured (default)** — if neither the host nor the tenant has any entries, all
   discovery-capable plugin types run.

This applies to two discovery triggers:

- Automatic discovery when a new host registers.
- Manual host-level discovery via `POST /api/v1/hosts/{id}/discover`.

The allowlist does **not** apply to `POST /api/v1/plugin-configs/{id}/discover`. That endpoint
explicitly targets a specific plugin config, and the user's intent takes precedence over the
allowlist filter.

---

## Endpoints

### `GET /api/v1/discovery-allowlist`

List all tenant-wide discovery allowlist entries.

**Permission:** `view_software`

**Request body:** none

**Response `200`:** Array of `TenantDiscoveryAllowlistEntry`

```json
[
  {
    "id": "019...",
    "plugin_type": "package_manager_homebrew",
    "created_at": "2026-02-27T10:00:00Z"
  },
  {
    "id": "019...",
    "plugin_type": "package_manager_apt",
    "created_at": "2026-02-27T10:01:00Z"
  }
]
```

**`TenantDiscoveryAllowlistEntry` fields:**

| Field | Type | Description |
| --- | --- | --- |
| `id` | UUID | Entry UUID |
| `plugin_type` | string | Plugin type restricted to (e.g. `"package_manager_homebrew"`) |
| `created_at` | ISO 8601 datetime | When the entry was created |

---

### `POST /api/v1/discovery-allowlist`

Add a plugin type to the tenant-wide discovery allowlist.

**Permission:** `manage_software`

**Request body:**

```json
{
  "plugin_type": "package_manager_homebrew"
}
```

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `plugin_type` | string | Yes | Plugin type to add to the allowlist |

**Response `201`:** `TenantDiscoveryAllowlistEntry`

```json
{
  "id": "019...",
  "plugin_type": "package_manager_homebrew",
  "created_at": "2026-02-27T10:00:00Z"
}
```

**Idempotent creation:** if an entry for the given `plugin_type` already exists, the existing
entry is returned with status `201`. No duplicate is created.

**Error responses:**

| Status | Condition |
| --- | --- |
| `400` | `plugin_type` is unknown or the plugin does not have the `DiscoverLocalSoftware` capability |

---

### `DELETE /api/v1/discovery-allowlist/{id}`

Remove an entry from the tenant-wide discovery allowlist.

**Permission:** `manage_software`

**Path parameters:**

| Parameter | Type | Description |
| --- | --- | --- |
| `id` | UUID | Allowlist entry UUID |

**Request body:** none

**Response `204`:** No content

**Error responses:**

| Status | Condition |
| --- | --- |
| `404` | Entry not found |

---

### `GET /api/v1/hosts/{id}/discovery-allowlist`

List all discovery allowlist entries for a specific host.

**Permission:** `view_software`

**Path parameters:**

| Parameter | Type | Description |
| --- | --- | --- |
| `id` | UUID | Host UUID |

**Request body:** none

**Response `200`:** Array of `HostDiscoveryAllowlistEntry`

```json
[
  {
    "id": "019...",
    "host_id": "019...",
    "plugin_type": "package_manager_apt",
    "created_at": "2026-02-27T11:00:00Z"
  }
]
```

**`HostDiscoveryAllowlistEntry` fields:**

| Field | Type | Description |
| --- | --- | --- |
| `id` | UUID | Entry UUID |
| `host_id` | UUID | Host this entry is scoped to |
| `plugin_type` | string | Plugin type restricted to |
| `created_at` | ISO 8601 datetime | When the entry was created |

**Error responses:**

| Status | Condition |
| --- | --- |
| `404` | Host not found or not active |

---

### `POST /api/v1/hosts/{id}/discovery-allowlist`

Add a plugin type to the allowlist for a specific host.

**Permission:** `manage_software`

**Path parameters:**

| Parameter | Type | Description |
| --- | --- | --- |
| `id` | UUID | Host UUID |

**Request body:**

```json
{
  "plugin_type": "package_manager_apt"
}
```

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `plugin_type` | string | Yes | Plugin type to add to the host allowlist |

**Response `201`:** `HostDiscoveryAllowlistEntry`

```json
{
  "id": "019...",
  "host_id": "019...",
  "plugin_type": "package_manager_apt",
  "created_at": "2026-02-27T11:00:00Z"
}
```

**Idempotent creation:** if an entry for the given `(host_id, plugin_type)` pair already exists,
the existing entry is returned with status `201`. No duplicate is created.

**Error responses:**

| Status | Condition |
| --- | --- |
| `400` | `plugin_type` is unknown or the plugin does not have the `DiscoverLocalSoftware` capability |
| `404` | Host not found or not active |

---

### `DELETE /api/v1/hosts/{id}/discovery-allowlist/{entry_id}`

Remove an entry from a host's discovery allowlist.

**Permission:** `manage_software`

**Path parameters:**

| Parameter | Type | Description |
| --- | --- | --- |
| `id` | UUID | Host UUID |
| `entry_id` | UUID | Allowlist entry UUID |

**Request body:** none

**Response `204`:** No content

**Error responses:**

| Status | Condition |
| --- | --- |
| `404` | Host or entry not found |

---

## Valid Plugin Types

Only plugin types that support the `DiscoverLocalSoftware` capability can be added to the
allowlist. Attempting to add a non-discovery plugin type returns a `400` error.

| Plugin type | Description |
| --- | --- |
| `package_manager_apt` | APT package manager (Debian/Ubuntu) |
| `package_manager_homebrew` | Homebrew formulae and casks (macOS/Linux) |
| `releases_docker` | Docker container discovery |
| `discovery_proxmox_helper_scripts` | Proxmox VE helper script applications |

---

## Related Documentation

- [Autodiscovery (End-user Guide)](../end-user/autodiscovery.md) — workflow overview, allowlist
  usage, and host vs. tenant scoping.
- [Autodiscovery API Reference](autodiscovery.md) — trigger endpoints, ignore rules, and bulk
  discard.
- [HTTP Web API](http-web-api.md) — common API patterns, error response format, and
  authentication.
