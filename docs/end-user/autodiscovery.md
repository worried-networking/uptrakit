# Autodiscovery

Uptrakit can automatically discover software installed on your hosts when an agent connects, and surface those packages for your review. Instead of manually creating a software item for every installed package, autodiscovery lets the agent do the initial inventory work.

Discovered items are held in a "pending" state until you decide what to do with them. You are always in control: Uptrakit never begins tracking or checking for updates on a discovered package without your explicit approval.

## How It Works

When an agent registers a new host (or reconnects with a previously unseen host), the controller sends a discovery request to that agent. The agent queries each of its discovery-capable providers and returns a list of installed packages. The controller then creates software items in a `pending` state for any packages it has not seen before.

Discovery-capable providers currently supported:

| Provider | What it discovers |
| --- | --- |
| Homebrew (Formulae) | Homebrew formula packages installed on the host |
| Homebrew (Casks) | Homebrew cask packages installed on the host |
| Proxmox Helper Scripts | Applications managed by community Proxmox VE helper scripts |

If no provider config exists for a discovery-capable provider when a host registers, Uptrakit creates one automatically (for example, "Homebrew (Formulae)" or "Proxmox Helper Scripts"). This means the feature works out of the box on supported hosts with no manual configuration required.

## Software Item States

Every software item in Uptrakit has a `discovery_state` that describes its origin and review status:

| State | Meaning | Tracking active? |
| --- | --- | --- |
| `null` (manual) | Created manually by a user | Yes |
| `pending` | Discovered by an agent, awaiting your review | No |
| `approved` | Discovered item you have reviewed and approved for tracking | Yes |

Pending items have version tracking disabled (`enabled: false`). Approving an item enables tracking and begins checking for available updates.

## Reviewing Discovered Items

Pending items appear alongside your existing software items. You can identify them by their `discovery_state: "pending"` field. Use the software items list in the Web UI, CLI, or API to see what has been discovered.

For each pending item you have three options:

**Approve** -- Enables the item for version tracking. Uptrakit will begin checking for installed and available versions on the next scheduled check cycle. Use this for packages you want to monitor and update through Uptrakit.

**Delete** -- Soft-deletes the item. The package is removed from your list but can be re-discovered in the future if autodiscovery runs again for that host. Use this when you do not need to track a specific package right now but may want it discovered again later.

**Ignore** -- Permanently suppresses the package from future discovery. Adding a package to the ignore list prevents Uptrakit from ever creating a pending item for it again. Use this for packages you will never want to track.

## The Ignore List

The ignore list lets you permanently suppress specific packages from appearing in future discovery runs. An ignore rule is keyed on a `(provider_config, package_identifier)` pair -- meaning it is scoped to the specific provider and package name, and applies across all hosts.

Once an ignore rule exists, autodiscovery will skip that package entirely when it would otherwise create a pending item for it.

### Adding a package to the ignore list

You can add an item to the ignore list in two ways:

**When deleting a discovered item** -- Pass `?ignore=true` when deleting a software item. This soft-deletes the item and simultaneously creates an ignore rule so it will not be re-discovered.

**Directly via the API** -- Create an ignore rule for any `(provider_config_id, package_identifier)` combination without needing to delete an existing item first. This is useful for pre-suppressing packages you know you will never want before they appear.

### Removing an ignore rule

Delete the ignore rule by its ID. After removal, the next discovery run for the relevant host and provider will be able to create a pending item for that package again.

See the [API reference](../api/autodiscovery.md#get-apiv1autodiscoveryignores) for managing ignore rules via the API.

## Triggering Discovery Manually

Autodiscovery runs automatically when an agent registers a new host. You can also trigger it on demand:

- **Trigger for a specific host** -- runs discovery across all provider configs associated with that host.
- **Trigger for a specific provider config** -- runs discovery for that provider across all connected agents. Returns an error if the provider type does not support discovery.

See [POST /api/v1/hosts/{id}/discover](../api/autodiscovery.md#post-apiv1hostsiddiscover) and [POST /api/v1/provider-configs/{id}/discover](../api/autodiscovery.md#post-apiv1provider-configsiddiscover) in the API reference.

## Bulk Discard

If you want to clear out all pending discovered items at once -- for example after an initial host registration produces a large list you do not want to review individually -- you can bulk-discard them:

- **By host** -- removes all pending items for a specific host. Optionally filter to a single provider config.
- **By provider config** -- removes all pending items across all hosts for a specific provider config.

Bulk discard performs a soft-delete. No ignore rules are created, so discarded packages can be re-discovered in a future run.

See [DELETE /api/v1/hosts/{id}/discovered](../api/autodiscovery.md#delete-apiv1hostsdiscovered) and [DELETE /api/v1/provider-configs/{id}/discovered](../api/autodiscovery.md#delete-apiv1provider-configsiddiscovered) in the API reference.

## Typical Workflow

1. An agent connects and registers a new host.
2. Uptrakit sends a discovery request. The agent queries Homebrew, Proxmox Helper Scripts, or other supported providers.
3. Discovered packages appear in your software items list with `discovery_state: "pending"`.
4. You review the list and choose for each item:
   - Approve the items you want Uptrakit to track and update.
   - Delete items you do not need right now (re-discoverable later).
   - Ignore items you never want to see again.
5. Approved items immediately become active: version checking begins on the next scheduler cycle.

## Related Documentation

- [API Reference: Autodiscovery](../api/autodiscovery.md) -- endpoint details, request/response shapes, and ignore rule management.
- [Software Item Entity](../architecture/software-item-entity.md) -- underlying data model and database schema.
- [System Overview](system-overview.md) -- agent and provider architecture.
- [Update Workflow](update-workflow.md) -- what happens after an item is approved and version tracking begins.
