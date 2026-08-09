# Instance Plugins

This page is for instance owners — Operators holding the `system.settings:manage` action. Tenant Operators do not see Instance Plugins in the
Settings tab.

## What is an Instance-Scoped Plugin?

Most plugins are configured per tenant. **Instance-Scoped Plugins** are configured at the instance level: you enable them once for the entire
instance, and tenant Operators see no evidence of the plugin's existence until you do.

The first instance-scoped plugin is **Dashboard Icons**, which fetches icon URLs from the public Dashboard Icons CDN to enrich software items.

## Enabling an instance plugin

1. Open **Settings → Plugin Configs → Instance Plugins** (the section is visible only to instance owners).
2. Locate the plugin row.
3. Click **Enable**.
4. Read the confirmation dialog — it reminds you that a controller restart is required.
5. Confirm.
6. **Restart the controller.** The action does not take effect until then.

While the change is pending, the row shows a **Pending restart** badge.

## Editing instance configuration

Plugins that expose instance-wide configuration knobs (currently none — Dashboard Icons is kill-switch only) show an **Edit Settings** button next to
the toggle. The form schema is supplied by the plugin descriptor and rendered using the same field types as the rest of Plugin Configs.

## Auditing toggles

Two new audit actions are emitted when you change instance plugin state:

- `instance_plugin.toggled` — whenever you enable or disable a plugin. Details include `previous_enabled` and `new_enabled`.
- `instance_plugin.config_upserted` — whenever you save a configuration. Raw config fields are not included in the audit details (only the field
  count).

Both events are visible in the system-level audit log to anyone with the `system.audit:read` action.

## Tenant-side opt-out

Once an instance plugin is enabled, individual tenants may still disable it for their own scope via **Settings → Plugin Configs → Type Defaults**. The
two settings are independent — an instance-disabled plugin is invisible to tenants entirely; an instance-enabled plugin is visible and tenant
Operators choose whether to opt in or out per their own tenant.

## Disabling an instance plugin

Disabling stops new operations from the plugin (no new lifecycle hooks fire, no new background tasks run after the next restart). It does **not**
retroactively undo persisted side effects.

For Dashboard Icons specifically, software items previously enriched with URLs of the form
`https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/...` keep those URLs. There is no provenance column to safely identify which icons came from
this plugin, so a wipe is not offered.

## Troubleshooting

**Q: I enabled the plugin but nothing happens.** A: Did you restart the controller? Check the **Pending restart** badge — if it's still showing, the
controller is still running the old (disabled) catalog.

**Q: A tenant says they don't see the plugin in their Type Defaults section.** A: Confirm the plugin is enabled at the instance level. If it is, check
the controller logs for any error during plugin construction. Disabled instance plugins are intentionally invisible to tenants.

**Q: I want to wipe Dashboard Icons enrichments from existing software items.** A: Not supported in v1 — there is no plugin-origin tracking on
`software_item.icon_url`. Manual cleanup via the Software Items UI is the only option today.

## See also

- ADR `docs/adr/0006-instance-scoped-plugins.md` — architectural context.
- Spec `docs/superpowers/specs/2026-05-10-instance-scoped-plugins-design.md` — the original design document.
- Plugin developer guide `docs/development/plugin-guidelines.md` — for writing new instance-scoped plugins.
