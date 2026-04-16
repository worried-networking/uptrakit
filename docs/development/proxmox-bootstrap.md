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

| Command | Purpose | Security rationale |
| --- | --- | --- |
| `id -u {username}` | Check if user exists | Avoid duplicate user creation |
| `useradd -m -s /bin/bash {username}` | Create the target user | Dedicated non-root user for Uptrakit SSH access |
| `getent passwd {username}` | Resolve home directory path | Deploy SSH keys to the correct location |
| `mkdir -p {home}/.ssh` | Create SSH directory | Required for `authorized_keys` |
| `chmod 700 {home}/.ssh` | Restrict SSH directory | OpenSSH requires strict permissions |
| `echo '{key}' >> {home}/.ssh/authorized_keys` | Deploy public key with restrictions | Authenticate Uptrakit SSH sessions; `no-pty,no-agent-forwarding,no-X11-forwarding` restrictions limit key to command execution only |
| `chmod 600 {home}/.ssh/authorized_keys` | Restrict key file | OpenSSH requires strict permissions |
| `chown -R {username}:{username} {home}/.ssh` | Fix ownership | User must own their SSH directory |
| `command -v {cmd}` | Resolve absolute paths for sudo commands | Sudoers entries use full paths for security |
| Write `/etc/sudoers.d/uptrakit-{username}` | Grant passwordless sudo for specific commands (or ALL) | Plugins need root for package management (`apt-get install`, etc.) |
| `chmod 440 /etc/sudoers.d/uptrakit-{username}` | Restrict sudoers file | sudo requires strict file permissions |
| `ssh-keygen -lf /etc/ssh/ssh_host_{algo}_key.pub` | Read host key fingerprint | Verify host identity on subsequent direct SSH connections |
| `hostname -I` (LXC) or `qm guest cmd ... network-get-interfaces` (QEMU) | Discover guest IP | Determine the SSH target address for direct connections |

## PVE API token privileges (controller-side discovery)

The Proxmox infrastructure plugin on the controller uses a PVE API token
to discover VMs and containers. Required privileges:

| Privilege | Scope | Purpose |
| --- | --- | --- |
| `Sys.Audit` | `/` | List cluster nodes |
| `VM.Audit` | `/vms` | List VMs/CTs and read their configs |
| `VM.Monitor` (optional) | `/vms` | Query QEMU guest agent for IPs and `machine_id` |

The built-in **PVEAuditor** role covers `Sys.Audit` and `VM.Audit`. Adding
`VM.Monitor` on `/vms` enables IP discovery and `machine_id` collection via the
guest agent, which improves automatic match suggestions.

### Assigning privileges

Without privilege separation (token inherits user permissions):

```sh
pveum acl modify / --users USER@REALM --roles PVEAuditor
```

With privilege separation (token has independent permissions):

```sh
pveum acl modify / --tokens USER@REALM!TOKENID --roles PVEAuditor
# Optional: enable guest agent queries
pveum acl modify /vms --tokens USER@REALM!TOKENID --roles PVEAuditor
```

## machine_id collection

During discovery, the controller attempts to read `/etc/machine-id` from
running QEMU VMs via the guest agent file-read endpoint
(`GET /nodes/{node}/qemu/{vmid}/agent/file-read?file=/etc/machine-id`).
This requires `VM.Monitor` privilege and the QEMU guest agent to be
installed and running inside the VM.

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
