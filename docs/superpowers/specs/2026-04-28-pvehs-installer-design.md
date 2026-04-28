# PVEHS Installer for uptrakit Controller Standalone — Design

## Goal

Ship three files that conform to the Proxmox VE Helper-Scripts (PVEHS) framework for
submission to the community-scripts/ProxmoxVE repository: a CT creation script, an
install/update script, and a JSON metadata file.

## Directory Layout

Files live under `scripts/pvehs/` in this repo, mirroring the PVEHS directory structure:

```text
scripts/pvehs/
  ct/uptrakit.sh
  install/uptrakit-install.sh
  json/uptrakit.json
```

When submitted to PVEHS these three files drop into `ct/`, `install/`, and `json/` respectively.

## CT Creation Script — `ct/uptrakit.sh`

Sources PVEHS `misc/build.func` from the upstream URL used by all community scripts.
Declares all required variables and delegates entirely to PVEHS framework functions —
no custom logic in this file.

| Variable      | Value              |
| ------------- | ------------------ |
| `APP`         | `uptrakit`         |
| `var_disk`    | `4` (GB)           |
| `var_cpu`     | `1`                |
| `var_ram`     | `1024` (MB)        |
| `var_os`      | `debian`           |
| `var_version` | `13`               |
| `CT_TYPE`     | `0` (unprivileged) |
| `var_tags`    | `monitoring`       |

Calls `build_container` then `description` (app name, port 8443, uptrakit.org) then
`start_script`.

## Install/Update Script — `install/uptrakit-install.sh`

Sources PVEHS `misc/install.func`. Contains two entry points required by the framework.

### `install_script` function

Sequence:

1. `msg_info "Installing Dependencies"` — install `curl`, `tar`, `openssl` via apt; `msg_ok`.
2. `msg_info "Fetching latest release"` — query GitHub releases API at
   `https://api.github.com/repos/worried-networking/uptrakit/releases`, filter by tag
   prefix `uptrakit-controller-standalone-v`, take first match; extract version string;
   detect arch via `uname -m` and map `x86_64` → `x86_64-unknown-linux-gnu` or
   `aarch64` → `aarch64-unknown-linux-gnu`; download and extract `.tar.gz` asset to
   `/usr/local/bin/uptrakit-controller-standalone`; `chmod +x`; write version to
   `/opt/uptrakit/version.txt`; `msg_ok`.
3. `msg_info "Creating service user"` — `useradd -r -s /usr/sbin/nologin -d /opt/uptrakit
   uptrakit`; `mkdir -p /opt/uptrakit/{config,state}`;
   `chown uptrakit:uptrakit /opt/uptrakit/{config,state}`; `msg_ok`.
4. `msg_info "Generating secrets"` — `openssl rand -hex 32` → `UPTRAKIT_MASTER_KEY`;
   write to `/root/uptrakit_master_key` with `chmod 600`; `openssl rand -hex 16` →
   enrollment token; write both to `/etc/uptrakit/env` with `chmod 600`, owned
   `root:root`; `msg_ok`.
5. `msg_info "Creating systemd unit"` — write `/etc/systemd/system/uptrakit.service`
   (see below); `systemctl daemon-reload`; `msg_ok`.
6. `msg_info "Starting uptrakit"` — `systemctl enable --now uptrakit`; `msg_ok`.
7. `msg_info "Fetching registration token"` — poll `journalctl -u uptrakit` up to 30 s
   for the line `one-time registration token`; extract the token value on the following
   line; `msg_ok`.
8. Print completion banner: CT IP, port 8443, registration token, path to master key file.

### `update_script` function

Sequence:

1. Read current version from `/opt/uptrakit/version.txt`.
2. Fetch latest `uptrakit-controller-standalone-v*` tag from GitHub API.
3. If versions match: `msg_ok "Already at latest ($current)"` and exit 0.
4. `msg_info "Updating uptrakit"` — `systemctl stop uptrakit`; download and replace
   binary; update `/opt/uptrakit/version.txt`; `systemctl start uptrakit`; `msg_ok`.

### Systemd unit

```ini
[Unit]
Description=uptrakit Controller
After=network.target

[Service]
Type=simple
User=uptrakit
Group=uptrakit
EnvironmentFile=/etc/uptrakit/env
ExecStart=/usr/local/bin/uptrakit-controller-standalone \
  --config-dir /opt/uptrakit/config \
  --state-dir /opt/uptrakit/state \
  --https-addr [::]:8443 \
  --bootstrap-enrollment-token ${UPTRAKIT_BOOTSTRAP_ENROLLMENT_TOKEN}
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

`UPTRAKIT_MASTER_KEY` and `UPTRAKIT_BOOTSTRAP_ENROLLMENT_TOKEN` are sourced from
`/etc/uptrakit/env`.

## JSON Metadata — `json/uptrakit.json`

Required by the PVEHS web UI:

```json
{
  "name": "uptrakit",
  "slug": "uptrakit",
  "categories": ["monitoring"],
  "date_created": "2026-04-28",
  "type": "ct",
  "updateable": true,
  "privileged": false,
  "interface_port": 8443,
  "documentation": "https://uptrakit.org/docs",
  "website": "https://uptrakit.org",
  "logo": "https://raw.githubusercontent.com/worried-networking/uptrakit/main/frontend/static/favicon.svg",
  "description": "uptrakit is a self-hosted software update manager. Schedule, track, and audit package updates across servers, VMs, and containers from a single web UI.",
  "install_methods": [{"type": "default", "script": "ct/uptrakit.sh"}],
  "resources": {"cpu": 1, "ram": 1024, "hdd": 4, "os": "debian", "version": "13"}
}
```

## Architecture — PVEHS Framework Dependency

Both shell scripts source helpers from the live PVEHS repo at runtime:

- `ct/uptrakit.sh` sources
  `https://raw.githubusercontent.com/community-scripts/ProxmoxVE/main/misc/build.func`
- `install/uptrakit-install.sh` sources
  `https://raw.githubusercontent.com/community-scripts/ProxmoxVE/main/misc/install.func`

This is the standard PVEHS pattern. All display functions (`msg_info`, `msg_ok`,
`msg_error`), CT creation logic, and color codes come from these shared helpers. Our
scripts contain zero duplicated framework code.

## Security Considerations

- Master key never printed to terminal or journal; written to `/root/uptrakit_master_key`
  (root-only, 600).
- `/etc/uptrakit/env` is 600, owned `root:root` — systemd reads it as root before
  dropping privileges to the `uptrakit` user.
- No privileged CT capabilities required.
- Enrollment token is random 16-byte hex; registration token is one-time use.

## Out of Scope

- PostgreSQL support (users needing PG use the lean controller separately).
- Reverse proxy / Let's Encrypt integration.
- Multi-instance / NATS HA mode.
- OIDC bootstrap (configured via UI post-install).
