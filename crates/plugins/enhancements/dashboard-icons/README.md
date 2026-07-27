# uptrakit-plugin-enhancement-dashboard-icons

Enhancement plugin that enriches newly created Software Items with icon URLs from the
[Dashboard Icons](https://github.com/homarr-labs/dashboard-icons) repository, served via jsDelivr.

## Scope

`PluginScope::Instance` — the kill switch is owned by users with `ManageGlobalSettings` and configured via
`/api/v1/instance-plugins`. The pre-existing tenant `type_settings` opt-out remains: when an instance owner has
enabled the plugin, individual tenants can still opt out by setting `{ "enabled": false }` in plugin type settings.

## Leakage vectors checklist

This plugin is `PluginScope::Instance`. The spec's leakage checklist (see
`docs/superpowers/specs/2026-05-10-instance-scoped-plugins-design.md` §6) has been verified for this plugin:

- **HTTP plugin-type/type-settings routes:** gated by `crate::visibility::is_plugin_visible_to_user` in `uptrakit-web-api`.
- **Surfaces registry:** plugin-owned surfaces filtered through the same predicate at request time.
- **AdminEvent SSE:** the plugin does not emit `AdminEvent` — no leakage channel.
- **Agent-side runtime:** the plugin's only role is `SoftwareItemLifecycle` (controller-side); no agent execution path.
- **MQTT topics:** the plugin does not publish to MQTT.
- **OpenAPI schema:** plugin type IDs are not enum members in any utoipa-derived schema.
- **DB tables tenant can read:** `plugin_type_setting` is filtered by the predicate.

### Known acceptable limitations

- **Audit log historical rows:** pre-existing `enhancement.dashboard-icons` audit rows written before the conversion
  remain visible to tenants viewing audit logs. Accepted limitation.
- **Persisted side effects on `software_item.icon_url`:** URLs of the form
  `https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/...` set by prior enrichment remain on existing
  `software_item.icon_url` rows visible to tenants. No provenance column exists on `icon_url`. Accepted limitation;
  documented in the end-user docs.

See ADR `docs/adr/0006-instance-scoped-plugins.md` (created in Plan B, Task 13) for the architectural decision.
