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

- Proxmox VE 7.x or 8.x
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

The installer generates an encryption key at `/root/uptrakit_master_key` inside the CT.
This key protects all secrets stored by uptrakit. **Back it up immediately** — data
encrypted by this key is unrecoverable if the key is lost (e.g., if the CT is destroyed
and rebuilt).

```bash
# Inside the CT
cat /root/uptrakit_master_key
```

Copy the 64-character hex string to a password manager or offline backup.

### 2. Log in

Open `https://<CT_IP>:8443` in a browser. Accept the self-signed certificate warning
(the controller issues its own CA on first start). Use the one-time registration token
shown in the install banner to create your admin account.

### 3. Enroll agents

After logging in, enroll agents via the web UI. No pre-generated enrollment token is
required — the controller approves enrollment requests through the UI.

See [SSH Agent Bootstrap](../ssh-agent-bootstrap.md) for enrolling Linux hosts.

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
