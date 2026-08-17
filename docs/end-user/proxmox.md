---
title: Proxmox VE Integration
weight: 190
description: Uptrakit can discover virtual machines and containers from a Proxmox VE cluster and link them to managed hosts.
---

# Proxmox VE Integration

Uptrakit can discover virtual machines and containers from a Proxmox VE
cluster and link them to managed hosts.

See also: [Plugin Configurations](plugin-configs.md),
[Proxmox Plugin Development](https://github.com/worried-networking/uptrakit/tree/main/docs/development/).

## Overview

The Proxmox VE plugin connects to your Proxmox cluster's REST API to discover
QEMU VMs and LXC containers. Once discovered, you can manually link them to
existing Uptrakit-managed hosts to see Proxmox metadata alongside update
information.

Beyond discovery and manual matching, Uptrakit can automatically create a PVE
snapshot or vzdump backup for a matched guest before applying an update
(configurable per software item/host). These protection artifacts are
labeled in the PVE UI with the software name, the version transition (e.g.
`1.0.0 -> 2.0.0`), and the triggering update's ID, so you can identify and
manage them at a glance. Automated rollback from these artifacts is not yet
implemented — a future capability.

## Setup

### Prerequisites

- A Proxmox VE cluster (version 7.x or 8.x)
- An API token with at least read access to nodes and VMs/CTs

### Creating a Proxmox API Token

1. In the Proxmox web UI, go to **Datacenter > Permissions > API Tokens**
1. Click **Add** and create a token for an existing user
1. Note the token ID and secret — you will need the full string in the format
   `USER@REALM!TOKENID=SECRET`

> [!TIP]
> Uptrakit's automatic provisioning (see
> [Automatic Credential Provisioning](#automatic-credential-provisioning) below)
> grants only the minimum privileges each feature needs, via custom roles
> rather than the built-in `PVEAuditor` role. If you are creating a token
> manually instead of letting the SSH agent provision one, grant it
> `Sys.Audit`, `VM.Audit`, `VM.GuestAgent.Audit`, and `VM.GuestAgent.FileRead`
> on `/` for discovery-only access -- the same privilege set as the
> `UptrakitAudit` role Uptrakit creates automatically. Update-protection
> (snapshots/backups) and resource-scaling features need additional
> privileges; see [Automatic Credential Provisioning](#automatic-credential-provisioning)
> for the full list.

### Adding the Plugin Configuration

Create a plugin configuration via the API or CLI:

```bash
uptrakit plugin-configs create \
  --name "My PVE Cluster" \
  --plugin-type infrastructure.proxmox \
  --config '{
    "api_url": "https://pve.local:8006",
    "api_token": "root@pam!uptrakit=your-secret-here",
    "verify_tls": false
  }'
```

#### Configuration Fields

| Field         | Required | Default | Description                                     |
| ------------- | -------- | ------- | ----------------------------------------------- |
| `api_url`     | Yes      | —       | Proxmox VE API URL (must be HTTPS)              |
| `api_token`   | Yes      | —       | API token in `USER@REALM!TOKENID=SECRET` format |
| `verify_tls`  | No       | `true`  | Set to `false` for self-signed certificates     |
| `node_filter` | No       | `[]`    | Restrict discovery to specific node names       |

## Usage

All Proxmox operations are accessed through shared surfaces.

### Testing the Connection

```bash
uptrakit surfaces invoke proxmox.hosts test-connection \
  --params '{"plugin_config_id": "YOUR_PLUGIN_CONFIG_ID"}'
```

### Discovering VMs and Containers

```bash
uptrakit surfaces invoke proxmox.hosts discover \
  --params '{"plugin_config_id": "YOUR_PLUGIN_CONFIG_ID"}'
```

This queries all online nodes (or filtered nodes) and lists their QEMU VMs
and LXC containers. For running QEMU VMs, it also attempts to query the
guest agent for IP address information.

### Listing Discovered Guests

```bash
uptrakit surfaces invoke proxmox.hosts list \
  --params '{"plugin_config_id": "YOUR_PLUGIN_CONFIG_ID"}'
```

### Matching to Uptrakit Hosts

Matching is manual — you explicitly link a discovered Proxmox guest to an
Uptrakit host:

```bash
uptrakit surfaces invoke proxmox.hosts match \
  --params '{"mapping_id": "MAPPING_ID", "host_id": "HOST_ID"}'
```

To remove a match:

```bash
uptrakit surfaces invoke proxmox.hosts unmatch \
  --params '{"mapping_id": "MAPPING_ID"}'
```

### Viewing Proxmox Info for a Host

```bash
uptrakit surfaces invoke proxmox.host-info info \
  --params '{"host_id": "HOST_ID"}'
```

## Node Filtering

To restrict discovery to specific Proxmox nodes, set the `node_filter` field
in your plugin configuration:

```json
{
  "api_url": "https://pve.local:8006",
  "api_token": "root@pam!uptrakit=secret",
  "node_filter": ["pve1", "pve3"]
}
```

Only nodes listed in the filter will be queried. An empty array (the default)
means all online nodes are included.

## Bootstrapping Guests via SSH Agent

When the SSH agent bootstraps a Proxmox VE node (via regular SSH bootstrap), it
automatically detects PVE and provisions API credentials for the tenant. This
enables a second bootstrap mode: bootstrapping guests (LXC containers and QEMU
VMs) directly through the PVE node without needing SSH access to the guest.

### Automatic Credential Provisioning

Uptrakit provisions PVE API access using a single cluster-wide user shared by
every tenant, plus one privilege-separated token per tenant -- not a dedicated
user per tenant:

- One user, `uptrakit@pve`, shared across every tenant on the cluster. It
  never gets a password -- it exists only to hold API tokens.
- One API token per tenant, id `tenant-{tenant_uuid}` (full form
  `uptrakit@pve!tenant-{tenant_uuid}`), created with `--privsep=1`. Because
  the token is privilege-separated, its effective privileges are the
  intersection of the grants on `uptrakit@pve` and the grants on the token
  itself -- the user-level grants are the ceiling, the token-level grants are
  the selection within it. A tenant's token can never exceed what
  `uptrakit@pve` itself is granted, and revoking one tenant's token never
  affects another tenant.
- Three custom roles, created and kept up to date automatically:
  - `UptrakitAudit` -- read-only access (`Sys.Audit`, `VM.Audit`,
    `VM.GuestAgent.Audit`, `VM.GuestAgent.FileRead`), granted on `/`.
  - `UptrakitProtection` -- pre-update snapshot/backup access (`VM.Snapshot`,
    `VM.Backup`, `Datastore.AllocateSpace`), granted on `/vms` and `/storage`.
  - `UptrakitScaling` -- live resource-scaling access (`VM.Audit`,
    `VM.Config.CPU`, `VM.Config.Memory`), granted on `/vms`.

  Each role is granted at both the user level (`uptrakit@pve`) and the token
  level (`uptrakit@pve!tenant-{tenant_uuid}`) -- privilege separation requires
  a grant at both levels before it takes effect for the token.

Multiple tenants can share one `uptrakit@pve` user on the same cluster, each
with its own token; a sync only ever creates, reads, or removes its own
tenant's token, never another tenant's. See
[ADR-0044](../adr/0044-shared-pve-user-with-per-tenant-privilege-separated-api-tokens.md)
for the full rationale.

Reuse is gated by whether the agent already holds an acknowledgment for a
reported plugin configuration, not by whether a token happens to exist on
the cluster: if the cluster already has a token for the same tenant (from
bootstrapping another node in the same cluster) but the agent has no such
acknowledgment yet, the token is regenerated rather than reused.

> [!NOTE]
> Deployments upgrading from an earlier release used a different identity
> model: a separate PVE user per tenant (`uptrakit-{tenant_uuid}@pve`) with a
> single, non-privilege-separated token, using the same custom `Uptrakit*`
> roles described above (or the built-in `PVEAuditor` role on deployments
> predating those roles). See
> [Migrating to the Shared PVE Identity Model](#migrating-to-the-shared-pve-identity-model)
> below for how upgrades move to the model described above.

### How It Works

1. **Bootstrap the PVE node** — Use the regular SSH bootstrap to set up the PVE
   host. The agent detects PVE automatically and provisions (or reuses) the
   API user/token described above.
2. **Bootstrap guests** — Use the "Bootstrap via Proxmox" action in the UI or
   CLI. The agent SSHs to the PVE node and runs commands inside the guest via
   `pct exec` (LXC) or `qm guest exec` (QEMU).

### Bootstrap via Proxmox Action

Available in the SSH Hosts surface page when at least one PVE node has been
bootstrapped.

| Field           | Required | Default    | Description                                                 |
| --------------- | -------- | ---------- | ----------------------------------------------------------- |
| PVE Host        | Yes      | —          | PVE node to use as gateway (select from bootstrapped nodes) |
| Guest VMID      | Yes      | —          | VMID of the target container or VM                          |
| Guest Type      | Yes      | `lxc`      | LXC Container or QEMU VM                                    |
| Host Name       | Yes      | —          | Friendly name for identification                            |
| Target Username | No       | `uptrakit` | User to create/use in the guest                             |
| Allow All       | No       | `false`    | Use NOPASSWD: ALL in sudoers                                |

### What Happens During Guest Bootstrap

1. Connects to the PVE node via SSH
2. Creates the target user inside the guest
3. Deploys an SSH key to the guest's `authorized_keys`
4. Configures sudoers for the target user
5. Retrieves the guest's IP address
6. Verifies direct SSH connectivity to the guest
7. Saves the host entry to the local database

After bootstrap, the guest is managed like any other SSH host — the agent
connects directly to it for version checks and updates.

### Bootstrap via Discovered Guest

The "Bootstrap via Discovered Guest" action matches guests discovered by the
Proxmox plugin to PVE hosts using the stored **PVE node name** and
**plugin config ID**. This requires that PVE hosts have been synced (via the
**Sync Host** action or during initial bootstrap) so the short node name (e.g.
`optiplex2`) is stored in the local database. Without a stored node name,
matching will fail.

If matching fails, use the **Sync Host** row action in the web UI (or run
`uptrakit surfaces ssh-agent.hosts --target-provider-id <PROVIDER_ID> sync <host-id>`)
to populate the node name, then retry.

## Migrating to the Shared PVE Identity Model

Deployments upgrading from an earlier release migrate automatically from the
legacy per-tenant-user model (`uptrakit-{tenant_uuid}@pve` plus a single
non-privilege-separated token) to the shared-user model described in
[Automatic Credential Provisioning](#automatic-credential-provisioning). No
manual credential work is required -- only privileged syncs, as detailed
below.

### Privileged session requirement

Credential provisioning and migration bookkeeping run on every sync of a PVE
host, but `pveum` is never in the agent's sudo allowlist -- it can only
succeed in an SSH session that is already root. Nothing progresses until you
sync (or bootstrap) the PVE host using a privileged (root) SSH session;
syncing as an unprivileged user makes no forward progress.

### Sync twice

Migration is a two-phase, cluster-scoped process:

1. **First privileged sync** — the agent records the legacy user (without
   touching it), creates the new per-tenant token, and proves it works by
   presenting it to `https://localhost:8006/api2/json/version` and requiring
   an HTTP 200 response. If the proof fails, the run reports a failure and
   stops -- the legacy user is never deleted on an unproved token. On
   success, the controller acknowledges the new plugin configuration back to
   the agent.
2. **Second privileged sync** — with that acknowledgment in hand, the agent
   deletes the legacy `uptrakit-{tenant_uuid}@pve` user and promotes the new
   configuration to active use.

Both syncs must use a privileged session for migration to complete.

### Reading the outcome

Each sync's summary reports one of:

- `migration pending` -- normal after the first privileged sync; migration is
  in progress and the next privileged sync should complete it.
- `migration STUCK after N attempts` -- legacy-user removal has failed
  repeatedly (5 or more attempts) and needs attention; confirm the session is
  genuinely privileged and that the legacy user hasn't been altered manually.
- `PVE state read degraded; migration paused this run (...)` -- a transient
  read failure. Migration bookkeeping is skipped for that run only; retry on
  the next sync.

### Aftermath

Once migration completes, the cluster has: the shared `uptrakit@pve` user and
its per-tenant token in place, and the legacy `uptrakit-{tenant_uuid}@pve`
user gone. Nothing renames the old plugin configuration -- migration creates
a **new** plugin configuration named `pve-{cluster_name}` (or
`pve-{node_name}-{first 8 characters of the host ID}` for a standalone node)
and leaves the old, legacy-named `pve-{host_id}` configuration in place,
still enabled, holding a token that no longer works once the legacy user is
deleted. See [Controller-side cleanup](#controller-side-cleanup) below to
remove it.

### Deployments with nothing to migrate

A deployment that never synced a PVE host before this release has no legacy
state to migrate -- its first sync provisions directly under the shared-user
model above.

### Split-agent clusters

Migration is scoped to the PVE cluster, not to any single SSH agent. If
different nodes of the same cluster are managed by different SSH agent
instances, migration still completes per cluster: each node converges to the
new model as its own agent instance syncs it with a privileged session.

### Rollback boundary

Once the legacy user has been deleted (end of the second privileged sync),
rolling back to an older Uptrakit release is not simply reversible: an older
agent does not recognize the shared-user/token model and expects the legacy
user to still exist. Rolling back after migration completes means
re-provisioning PVE credentials from scratch.

### If the plugin config is deleted mid-migration

Avoid deleting a `pve-*` plugin configuration while migration is still
pending. The agent's local migration bookkeeping only tracks whether it has
already received the controller's acknowledgment for a plugin configuration --
it does not re-verify that the configuration still exists on the controller.
Deleting it mid-migration can leave the agent believing migration is on track
while the controller has no matching configuration. Wait for the aftermath
state above before deleting or renaming a `pve-*` configuration.

**If you already deleted it:** re-syncing or re-bootstrapping the same host
does not recover on its own -- the agent's local PVE token is untouched by
the controller-side deletion, so it keeps reusing that token and never
re-reports a plugin configuration to the controller. The confirmed recovery
is to remove the host from Uptrakit entirely
(`uptrakit-agent-ssh host remove`, or the equivalent web UI action -- see
[Host Management -- Removing a host](ssh-agent-host-management.md#removing-a-host))
and then re-bootstrap it: a removed-and-re-added host gets fresh local
tracking state, so the next bootstrap provisions PVE credentials and reports
a plugin configuration from a clean slate.

## Deprovisioning

Use these steps when you stop using Uptrakit's Proxmox VE integration
entirely, or when you drop a single tenant from a shared cluster.

### Removing one tenant

To remove a single tenant's access while leaving other tenants on the same
cluster untouched:

```bash
pveum user token remove 'uptrakit@pve' 'tenant-{tenant_uuid}'
```

This is safe to run with other tenants still active on the cluster -- it
removes only that tenant's token. The shared `uptrakit@pve` user, its roles,
and every other tenant's token are unaffected.

### Removing the last tenant

First confirm no other tenant still has a token on the shared user:

```bash
pveum user token list 'uptrakit@pve'
```

Only once that list is empty is it safe to remove the shared user and its
custom roles:

```bash
pveum user delete 'uptrakit@pve'
pveum role delete 'UptrakitAudit'
pveum role delete 'UptrakitProtection'
pveum role delete 'UptrakitScaling'
```

> [!WARNING]
> Deleting `uptrakit@pve` or any of the three roles while another tenant
> still has a token on that user breaks that tenant's PVE integration --
> always confirm the token list is empty first.

### Cleaning up a never-fully-migrated legacy user

If a deployment never completed the migration described in
[Migrating to the Shared PVE Identity Model](#migrating-to-the-shared-pve-identity-model)
(for example, PVE syncs were never run with a privileged session), a legacy
per-tenant user may still be present:

```bash
pveum user list
```

Look for any remaining `uptrakit-*@pve` entries and remove them:

```bash
pveum user delete 'uptrakit-{tenant_uuid}@pve'
```

### Host-side cleanup

Removing a host from Uptrakit (`uptrakit-agent-ssh host remove`, or deleting
the host in the controller UI) does not remove the managed user, its SSH key,
or its sudoers drop-in from the PVE node itself -- see
[SSH Agent Host Management -- Removing a host](ssh-agent-host-management.md#removing-a-host).
To clean those up on the PVE node, remove the target user's entry from
`~<username>/.ssh/authorized_keys` (or delete the user outright), and delete
its sudoers drop-in at `/etc/sudoers.d/uptrakit-<username>` (default target
username `uptrakit`).

### Controller-side cleanup

Deleting the `pve-*` plugin configuration in the web UI removes only the
controller's record of it -- it does **not** remove anything on the PVE node.
Use the `pveum` commands above to remove the actual PVE-side user, token, and
roles.

After a completed migration (see [Aftermath](#aftermath) above), the old,
legacy-named `pve-{host_id}` plugin configuration is left behind alongside
the new `pve-{cluster_name}` one. It holds a token for the now-deleted
legacy user and no longer works -- delete it from the web UI once you've
confirmed the new configuration is in place and working.

## Security Considerations

- The API token secret is stored encrypted at rest and masked in API responses
- HTTPS is required for the API URL — HTTP connections are rejected
- Private and loopback addresses are allowed since Proxmox is typically
  deployed on-premise
- TLS verification can be disabled for self-signed certificates common in PVE
  installations
