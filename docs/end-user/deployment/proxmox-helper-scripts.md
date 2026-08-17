---
title: Proxmox VE Helper-Scripts
weight: 70
description: Install the uptrakit controller as an LXC container on Proxmox VE using a single command, with automatic updates via the PVEHS framework.
---

# Proxmox VE Helper-Scripts

Install uptrakit as an LXC container on your Proxmox VE host using the
[Proxmox VE Helper-Scripts](https://github.com/community-scripts/ProxmoxVE) framework.
This is the recommended install method for Proxmox-based home labs and self-hosted
infrastructure.

## What Gets Created

- Unprivileged Debian 13 LXC container
- 1 vCPU, 1 GB RAM, 4 GB disk
- `uptrakit-controller-standalone` binary (all services embedded — no Docker required)
- SQLite database (zero external dependencies)
- Dedicated `uptrakit` system user
- Systemd service with automatic restart

## Prerequisites

- Proxmox VE 8.x or 9.x (verified on 9.2)
- Internet access from the Proxmox host (for CT template download and binary fetch)

## Installation

Run from the **Proxmox VE host shell** (not inside a CT):

```bash
bash -c "$(curl -fsSL https://raw.githubusercontent.com/community-scripts/ProxmoxVE/main/ct/uptrakit.sh)"
```

> [!NOTE]
> The script is pending merge into the official PVEHS repository. Until merged, run it
> directly from the uptrakit repository:
>
> ```bash
> bash -c "$(curl -fsSL https://raw.githubusercontent.com/worried-networking/uptrakit/main/scripts/pvehs/ct/uptrakit.sh)"
> ```

The script creates the LXC container, installs uptrakit, starts the service, and prints
a completion banner with the CT IP address and registration token.

## Post-Installation

### 1. Save the master key

The installer generates an encryption key stored in `/opt/uptrakit/master.key` inside the CT.
This key protects all secrets stored by uptrakit. **Back it up** — data encrypted by this
key is unrecoverable if the CT is destroyed and rebuilt.

```bash
# Inside the CT
cat /opt/uptrakit/master.key
```

Copy the contents of `/opt/uptrakit/master.key` to a password manager or offline backup.

### 2. Log in

Open `https://<CT_IP>:8443` in a browser. Accept the self-signed certificate warning
(the controller issues its own CA on first start). Use the one-time registration token
shown in the install banner to create your admin account.

### 3. Enroll agents

After logging in, enroll agents via the web UI. No pre-generated enrollment token is
required — the controller approves enrollment requests through the UI.

See [SSH Agent Bootstrap](../ssh-agent-bootstrap.md) for enrolling Linux hosts.

## Host Provisioning

A fresh install automatically provisions the CT's local host: the installer calls
`bootstrap-host`, which writes the sudoers drop-in for the `uptrakit` user and installs
any plugin helper scripts (e.g. `/usr/local/bin/uptrakit-phs-version`), deriving the
grant list from the registered plugins rather than a blanket `NOPASSWD: ALL`. See
[Sudoers Management](../../security/sudoers-management.md) for how the drop-in is
generated and validated.

Existing installations self-heal on the next `update` run — the update script re-runs
`bootstrap-host`, so new plugin sudo grants and helper scripts appear without a
reinstall. The command is idempotent: re-running it is always safe.

Bootstrap failure is non-fatal to install or update; the script prints a retry command
and continues. To retry manually as root:

```bash
/usr/local/bin/uptrakit-controller-standalone agent bootstrap-host --user uptrakit
```

On a standalone `uptrakit-agent` host, the equivalent command is:

```bash
uptrakit-agent bootstrap-host --user uptrakit
```

## Updating

### Via PVEHS (recommended)

If you have the PVEHS update script installed on your Proxmox host, uptrakit updates
alongside your other PVEHS-managed containers.

### Manual update

Run the install script again from the Proxmox host shell — it detects an existing
installation and updates the binary in place:

```bash
bash -c "$(curl -fsSL https://raw.githubusercontent.com/worried-networking/uptrakit/main/scripts/pvehs/ct/uptrakit.sh)"
```

The service restarts automatically after the binary is replaced. Existing configuration,
database, and master key are preserved.

## Related Documentation

- [Secrets and encryption](../../security/secrets-and-encryption.md) — master key management and rotation
- [Docker deployment](docker.md) — alternative deployment for non-Proxmox environments
- [Reverse proxy deployment](reverse-proxy.md) — putting uptrakit behind Nginx, Traefik, or Caddy
