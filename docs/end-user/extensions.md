# Extensions

Extensions allow plugins and connected services to add custom pages, panels, context menu
actions, and table columns to the Uptrakit web UI. Extensions appear dynamically based on
which services are connected and what capabilities they provide.

## What extensions can add

| Extension type | Where it appears | Example |
| --- | --- | --- |
| Page | Sidebar navigation item, opens a full page | SSH Host Management page |
| Panel | Tab or section on an existing detail page | LXC matching panel on host detail |
| Context menu group | Submenu in an entity's right-click menu | "SSH Agent" actions on host rows |
| Table columns | Extra columns in an existing list table | "SSH Status" column on hosts table |

Extensions only appear when at least one service instance that provides them is connected.
When all providers disconnect, the extension is removed from the UI until a provider
reconnects.

## Viewing extensions in the UI

### Sidebar pages

Extensions with page placement appear as navigation items in the sidebar, grouped by
section. Click the item to open the extension page.

If the extension is **targeted** (tied to a specific service instance) and multiple
providers are connected, a service selector dropdown appears at the top of the page.
Select which instance to interact with before performing actions.

### Panels on existing pages

Extensions can inject panels into existing pages (such as host detail or service detail).
These panels appear as additional tabs, or as sections above or below the main content,
depending on the extension's configuration.

### Context menu actions

Extensions can add action groups to context menus on entity rows (hosts, services, software
items). Right-click a row to see extension-provided actions grouped under a submenu header
(e.g., "SSH Agent"). Selecting an action may open a form or wizard for input before
execution.

### Table columns

Extensions can add columns to existing tables (hosts list, services list). These columns
fetch their data lazily from the providing service and display alongside the built-in
columns.

## Using extension actions

Many extensions expose **actions** -- operations you can invoke from buttons, context menus,
or table rows. Actions may:

- Execute immediately (e.g., "Refresh Status")
- Show a form for input before executing (e.g., "Bootstrap Host" with a username field)
- Show a multi-step wizard for complex operations

After an action completes, the result is displayed in the UI. Destructive actions are shown
with warning styling and may require confirmation.

### Timeouts

Actions have a default timeout of 30 seconds. Some actions (e.g., long-running remote
operations) may have longer timeouts configured by the extension. If an action times out,
you will see a timeout error message.

## Viewing extensions via the CLI

### List all extensions

```sh
uptrakit extensions list
```

Displays a table with extension ID, label, and placement type. Use `--output json` for
machine-readable output:

```sh
uptrakit --output json extensions list
```

### List providers for an extension

```sh
uptrakit extensions providers ssh-agent.host-management
```

Shows all connected service instances that provide the specified extension.

### Invoke an action (raw JSON)

```sh
uptrakit extensions invoke ssh-agent.hosts list-hosts
```

For targeted extensions, specify which service instance to use:

```sh
uptrakit extensions invoke ssh-agent.hosts list-hosts \
  --service-id 019585f4-1234-7000-8000-000000000001
```

Pass parameters as JSON:

```sh
uptrakit extensions invoke ssh-agent.hosts bootstrap \
  --params '{"target": "root@web-01.example.com", "name": "web-01"}'
```

### Dynamic extension subcommands

Extensions can also be invoked with typed arguments auto-generated from the extension's
manifest. This provides `--help`, type validation, and tab completion:

```sh
# List SSH hosts from an agent
uptrakit extensions ssh-agent.hosts --service-id <UUID> list-hosts

# Bootstrap a new host with typed arguments
uptrakit extensions ssh-agent.hosts --service-id <UUID> bootstrap \
  --target root@192.168.1.100 \
  --name my-server \
  --auth-method password

# Remove a host by ID
uptrakit extensions ssh-agent.hosts --service-id <UUID> remove-host <host-id>

# Show available actions and arguments for an extension
uptrakit extensions ssh-agent.hosts --service-id <UUID> --help

# Show form fields for a specific action
uptrakit extensions ssh-agent.hosts --service-id <UUID> bootstrap --help
```

The `--service-id` flag is only required for targeted extensions (like `ssh-agent.hosts`).
Universal extensions omit it.

## Permissions

Extensions declare a `required_permission` that gates visibility and access. If your user
account does not have the required permission, the extension will not appear in the UI or
CLI output.

Individual actions within an extension may also require specific permissions. You will see
a "You do not have permission" error if you attempt to invoke an action without the required
permission.

## Targeted vs universal extensions

| Mode | Behaviour |
| --- | --- |
| Universal | Any connected provider can handle the action. The system picks one automatically. |
| Targeted | You must select a specific service instance. The UI shows a dropdown when multiple providers are available. |

**Example**: An SSH agent extension is typically **targeted** because each agent manages
different hosts. A monitoring dashboard extension might be **universal** because any
instance has the same aggregated view.

## Proxmox VE extensions

The Proxmox VE infrastructure plugin provides two extensions:

### Proxmox VE Hosts (page)

A full-page data table listing all discovered VMs and containers from your Proxmox VE cluster.
Available actions:

- **Discover** -- query the PVE API for VMs/containers and update the list
- **Test Connection** -- verify connectivity to the PVE API
- **Manual Match** -- link a Proxmox guest to an Uptrakit host via dropdown
- **Approve Match** -- accept a system-suggested match (based on machine ID,
  hostname, or IP address)
- **Remove Match** -- unlink a guest from its matched host

Unmatched guests display inline match suggestions with a confidence level (high,
medium, low) based on machine ID, hostname, IP address, and name similarity.

### Proxmox VE Info (panel)

A panel on the host detail page showing the Proxmox node, VMID, type, and status
for hosts linked to a discovered Proxmox guest.

## See also

- [Extensions API Reference](../api/extensions.md)
- [Extensions Security](../security/extensions.md) -- permission model, trust boundaries
