# Proxmox Bootstrap Privileges

This document describes the privilege chain required for the Proxmox VE
bootstrap flow (the `bootstrap-proxmox-guest` shared surface action) and the
controller-side discovery via the PVE REST API.

## PVE node privileges (SSH user)

The `bootstrap-proxmox-guest` action connects to an already-bootstrapped PVE
node via SSH and executes commands inside guests via `pct exec` (LXC) or
`qm guest exec` (QEMU). This requires **root access** on the PVE node
because `pct` and `qm` are privileged commands.

## Commands executed inside the guest

All commands below run as root inside the guest via `pct exec` or
`qm guest exec`. Each command, its purpose, and security rationale:

| Command                                                                 | Purpose                                                | Security rationale                                                                                                                  |
| ----------------------------------------------------------------------- | ------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------- |
| `id -u {username}`                                                      | Check if user exists                                   | Avoid duplicate user creation                                                                                                       |
| `useradd --create-home --shell /bin/sh {username}`                      | Create the target user                                 | Dedicated non-root user for Uptrakit SSH access                                                                                     |
| `getent passwd {username}`                                              | Resolve home directory path                            | Deploy SSH keys to the correct location                                                                                             |
| `mkdir -p {home}/.ssh`                                                  | Create SSH directory                                   | Required for `authorized_keys`                                                                                                      |
| `chmod 700 {home}/.ssh`                                                 | Restrict SSH directory                                 | OpenSSH requires strict permissions                                                                                                 |
| `echo '{key}' >> {home}/.ssh/authorized_keys`                           | Deploy public key with restrictions                    | Authenticate Uptrakit SSH sessions; `no-pty,no-agent-forwarding,no-X11-forwarding` restrictions limit key to command execution only |
| `chmod 600 {home}/.ssh/authorized_keys`                                 | Restrict key file                                      | OpenSSH requires strict permissions                                                                                                 |
| `chown -R {username}:{username} {home}/.ssh`                            | Fix ownership                                          | User must own their SSH directory                                                                                                   |
| `command -v {cmd}`                                                      | Resolve absolute paths for sudo commands               | Sudoers entries use full paths for security                                                                                         |
| Write `/etc/sudoers.d/uptrakit-{username}`                              | Grant passwordless sudo for specific commands (or ALL) | Plugins need root for package management (`apt-get install`, etc.)                                                                  |
| `chmod 440 /etc/sudoers.d/uptrakit-{username}`                          | Restrict sudoers file                                  | sudo requires strict file permissions                                                                                               |
| `ssh-keygen -lf /etc/ssh/ssh_host_{algo}_key.pub`                       | Read host key fingerprint                              | Verify host identity on subsequent direct SSH connections                                                                           |
| `hostname -I` (LXC) or `qm guest cmd ... network-get-interfaces` (QEMU) | Discover guest IP                                      | Determine the SSH target address for direct connections                                                                             |

## PVE API token privileges (controller-side discovery)

The Proxmox infrastructure plugin on the controller uses a PVE API token
to discover VMs and containers, and (for update protection and scaling)
to manage them. Identity is **cluster-wide, not per-tenant**: a single PVE
user `uptrakit@pve` (`pve_setup::PVE_USER`) is created once per cluster and
never gets a password — it is a token-only identity. Each tenant instead
gets its own `--privsep=1` API token on that shared user, id
`tenant-{tenant_uuid}` (`pve_setup::pve_token_id`), full form
`uptrakit@pve!tenant-{tenant_uuid}` (`pve_setup::pve_full_token_id`). See
[ADR-0044](../adr/0044-shared-pve-user-with-per-tenant-privilege-separated-api-tokens.md)
for the rationale.

Bootstrap idempotently creates/updates three custom PVE roles
(`pveum role add '<role>' -privs '<privs>' 2>/dev/null; pveum role modify
'<role>' -privs '<privs>'`):

| Role                 | Privileges                                                            |
| -------------------- | --------------------------------------------------------------------- |
| `UptrakitAudit`      | `Sys.Audit` `VM.Audit` `VM.GuestAgent.Audit` `VM.GuestAgent.FileRead` |
| `UptrakitProtection` | `VM.Snapshot` `VM.Backup` `Datastore.AllocateSpace`                   |
| `UptrakitScaling`    | `VM.Audit` `VM.Config.CPU` `VM.Config.Memory`                         |

These are granted via four `(path, role)` ACL pairs:

| Path       | Role                 |
| ---------- | -------------------- |
| `/`        | `UptrakitAudit`      |
| `/vms`     | `UptrakitProtection` |
| `/storage` | `UptrakitProtection` |
| `/vms`     | `UptrakitScaling`    |

`/storage` is required alongside `/vms` for `UptrakitProtection` because
vzdump backups targeting PBS/directory storage need `Datastore.AllocateSpace`
on the storage path itself, not just `/vms`.

### Assigning privileges

Each `(path, role)` pair is granted at **both** grant levels — user and
token:

```sh
pveum acl modify '<path>' --users 'uptrakit@pve' --roles '<role>'
pveum acl modify '<path>' --tokens 'uptrakit@pve!tenant-{tenant_uuid}' --roles '<role>'
```

This is required because Proxmox's `--privsep=1` model computes a token's
effective privileges as the **intersection** of the user's ACL grants and
the token's own ACL grants (upstream `pve-access-control`,
`RPCEnvironment.pm` `permissions()`): the user-level grant is the ceiling,
the token-level grant is the selection within it. A user with zero ACLs
zeroes every one of its tokens regardless of what the token itself was
granted.

Note that `pveum` itself is never added to any sudoers allowlist — the
Proxmox plugin's sudo contribution
(`collect_pve_sudo_commands`, see [Sudoers Management](../security/sudoers-management.md))
covers only `pct exec` / `qm guest exec` / `qm guest cmd`. `pveum` can only
succeed when the bootstrap SSH session is already running as root.

## machine_id collection

During discovery, the controller attempts to read `/etc/machine-id` from
running QEMU VMs via the guest agent file-read endpoint
(`GET /nodes/{node}/qemu/{vmid}/agent/file-read?file=/etc/machine-id`).
This requires the `VM.GuestAgent.FileRead` privilege (granted via
`UptrakitAudit` on `/`) and the QEMU guest agent to be installed and running
inside the VM. `VM.GuestAgent.FileRead` and `VM.GuestAgent.Audit` (used for
guest network-interface/IP discovery) were introduced in PVE 9, replacing
the older `VM.Monitor` privilege; the legacy per-tenant identity model
granted the built-in `PVEAuditor` role, which never included `VM.Monitor`
either — `machine_id`/guest-agent collection is new capability introduced by
the shared-user roles, not something the legacy model already had.

LXC containers do not support the guest agent file-read API. Their
`machine_id` is populated after bootstrap when the host reports its
identity via the `ReportHosts` message.

## Match suggestion flow

After discovery, the Proxmox Hosts page computes inline match suggestions
for unmatched guests by comparing:

1. **machine_id** (High confidence) -- exact match
2. **hostname + IP** (High confidence) -- both match
3. **hostname only** (Medium confidence) -- case-insensitive
4. **IP only** (Medium confidence) -- host IP in guest's IP list
5. **Proxmox name** (Low confidence) -- case-insensitive name match

Users review suggestions and approve them with the "Approve Match" action,
or override with manual matching via the "Manual Match" action.

## See also

- [SSH Agent Architecture](../architecture/ssh-agent.md) -- bootstrap flow
  (operations/ directory), PVE detection, shared surface actions
- [SSH Agent Secrets](../security/ssh-agent-secrets.md) -- encryption model,
  sudoers configuration, host key verification
