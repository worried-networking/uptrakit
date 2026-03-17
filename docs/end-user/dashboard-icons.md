# Dashboard Icons

Dashboard Icons is an optional enhancement that automatically assigns icons to your software items
using the community-curated [Dashboard Icons](https://github.com/homarr-labs/dashboard-icons)
project. When enabled, newly created and auto-discovered software items receive an SVG icon URL
if a matching icon exists in the collection.

## How It Works

When a software item is created -- either manually through the API or via autodiscovery -- the
controller looks up the item name in an index of available Dashboard Icons. If a match is found,
the item's `icon_url` is set to an SVG hosted on the jsDelivr CDN:

```text
https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/svg/{slug}.svg
```

The name is converted to a slug by lowercasing, replacing spaces and underscores with hyphens,
and removing special characters. For example, "Home Assistant" matches `home-assistant.svg` and
"Nginx" matches `nginx.svg`.

Icons are **not** overwritten if a software item already has an `icon_url` set. Only items without
an existing icon are enriched.

## Enabling Dashboard Icons

This feature is **per-tenant** and **disabled by default**. To enable it:

```sh
curl -X PUT /api/v1/settings/dashboard-icons \
  -H "Authorization: Bearer <TOKEN>" \
  -H "Content-Type: application/json" \
  -d '{"enabled": true}'
```

To check the current status:

```sh
curl /api/v1/settings/dashboard-icons \
  -H "Authorization: Bearer <TOKEN>"
```

Response:

```json
{
  "enabled": true
}
```

Enabling requires the `manage_global_settings` permission. Reading the setting requires
`view_settings`.

## When Icons Are Assigned

Icons are assigned at two points:

| Trigger | Description |
| --- | --- |
| Manual creation | When you create a software item via `POST /api/v1/software-items` without specifying an `icon_url`. |
| Autodiscovery | After discovery results are processed, all featured items that have no icon are checked against the index. |

In both cases, the feature must be enabled for the tenant. Items that already have an `icon_url`
are never modified.

## Icon Index

The controller maintains a cached index of all available icon slugs from the Dashboard Icons
repository. The index is fetched from the GitHub API at startup and refreshed automatically
every 6 hours. No manual cache management is required.

If the GitHub API is temporarily unavailable, the controller continues using the last
successfully fetched index.

## Permissions

| Operation | Required Permission |
| --- | --- |
| View Dashboard Icons setting | `view_settings` |
| Enable or disable Dashboard Icons | `manage_global_settings` |

## Related Documentation

- [Dashboard Icons Development Guide](../development/dashboard-icons.md) -- architecture and implementation details.
- [Autodiscovery](autodiscovery.md) -- how software items are automatically discovered.
- [Manual Software Tracking](manual-software-tracking.md) -- creating software items manually.
