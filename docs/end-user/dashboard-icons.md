---
title: Dashboard Icons
weight: 150
description:
  Dashboard Icons is an instance-scoped enhancement that automatically assigns icons to software items using the community-curated Dashboard Icons
  project.
---

# Dashboard Icons

Dashboard Icons is an optional, **instance-scoped** enhancement that automatically assigns icons to your software items using the community-curated
[Dashboard Icons](https://github.com/homarr-labs/dashboard-icons) project. When enabled, newly created and auto-discovered software items receive an
SVG icon URL if a matching icon exists in the collection.

Following the instance-plugins migration, Dashboard Icons is **disabled by default**. It must be enabled by an instance owner before any tenant can
benefit from icon enrichment.

## How It Works

When a software item is created -- either manually through the API or via autodiscovery -- the controller looks up the item name in an index of
available Dashboard Icons. If a match is found, the item's `icon_url` is set to an SVG hosted on the jsDelivr CDN:

```text
https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/svg/{slug}.svg
```

The name is converted to a slug by lowercasing, replacing spaces and underscores with hyphens, and removing special characters. For example, "Home
Assistant" matches `home-assistant.svg` and "Nginx" matches `nginx.svg`.

Icons are **not** overwritten if a software item already has an `icon_url` set. Only items without an existing icon are enriched.

## Enabling Dashboard Icons

Enabling Dashboard Icons is a two-layer process: the instance owner turns the plugin on for the entire instance, and tenants then choose whether to
opt in or out for their own scope.

### Layer 1 — Instance owner enables the plugin

An instance owner (Operator with `manage_global_settings`) enables Dashboard Icons in the web UI under **Settings → Plugin Configs → Instance
Plugins**. Toggling the plugin on requires a **controller restart** before it takes effect. While the change is pending, the row in the UI shows a
**Pending restart** badge.

Until an instance owner enables Dashboard Icons, the plugin is invisible to tenant Operators — they will not see a corresponding entry under their own
**Type Defaults** section.

### Layer 2 — Tenant opt-out (optional)

Once Dashboard Icons is enabled at the instance level, individual tenants may still opt out for their own scope. This is done in the web UI under
**Settings → Plugin Configs → Type Defaults**: look for the `enhancement.dashboard-icons` row and toggle `enabled` off.

You can also manage the per-tenant override through the generic plugin type settings API:

```sh
curl -X PUT /api/v1/plugin-type-settings/enhancement.dashboard-icons \
  -H "Authorization: Bearer <TOKEN>" \
  -H "Content-Type: application/json" \
  -d '{"config":{"enabled":true}}'
```

To inspect the current tenant override:

```sh
curl /api/v1/plugin-type-settings/enhancement.dashboard-icons \
  -H "Authorization: Bearer <TOKEN>"
```

Response:

```json
{
  "plugin_type": "enhancement.dashboard-icons",
  "config": {
    "enabled": true
  }
}
```

If no row exists for this plugin type, Dashboard Icons falls back to the instance-level setting. To return to that default, delete the tenant
override:

```sh
curl -X DELETE /api/v1/plugin-type-settings/enhancement.dashboard-icons \
  -H "Authorization: Bearer <TOKEN>"
```

## Existing Icons After Migration or Disable

Software items already enriched with URLs of the form `https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/...` retain those URLs after the
instance-plugins migration, and they continue to retain those URLs if an instance owner later disables Dashboard Icons.

Disabling the plugin stops **new** enrichments — it does not retroactively wipe historical `icon_url` values. There is no plugin-origin tracking
column on software items, so a safe automatic wipe is not offered. If you want to clear specific icons, edit the affected software items manually.

## When Icons Are Assigned

Icons are assigned at two points:

| Trigger         | Description                                                                                          |
| --------------- | ---------------------------------------------------------------------------------------------------- |
| Manual creation | When you create a software item via `POST /api/v1/software-items` without specifying an `icon_url`.  |
| Autodiscovery   | After discovery results are processed, featured items without an icon are checked against the index. |

In both cases, the plugin must be enabled both at the instance level and (where present) in the tenant's Type Defaults override. Items that already
have an `icon_url` are never modified.

## Icon Index

The controller maintains a cached index of all available icon slugs from the Dashboard Icons repository. The index is fetched from the GitHub API at
startup and refreshed automatically every 6 hours. No manual cache management is required.

If the GitHub API is temporarily unavailable, the controller continues using the last successfully fetched index.

## Permissions

| Operation                                                   | Required Permission                         |
| ----------------------------------------------------------- | ------------------------------------------- |
| View Dashboard Icons type defaults                          | `view_settings` or `manage_global_settings` |
| Enable or disable Dashboard Icons at the **instance** level | `manage_global_settings`                    |
| Override Dashboard Icons for a tenant via **Type Defaults** | `manage_global_settings`                    |

## Related Documentation

- [Instance Plugins administrator guide](https://github.com/worried-networking/uptrakit/blob/main/docs/admin/instance-plugins.md) -- enabling and
  operating instance-scoped plugins.
- [Dashboard Icons Development Guide](https://github.com/worried-networking/uptrakit/tree/main/docs/development/) -- architecture and implementation
  details.
- [Autodiscovery](autodiscovery.md) -- how software items are automatically discovered.
- [Manual Software Tracking](manual-software-tracking.md) -- creating software items manually.
