# PVEHS Installer for uptrakit Controller Standalone — Design

## Goal

Ship three files conforming exactly to the Proxmox VE Helper-Scripts (PVEHS) framework for
submission to the community-scripts/ProxmoxVED repository: a CT creation script, an
install/update script, and a JSON metadata file.

## Submission Target

Initial PR goes to **community-scripts/ProxmoxVED** (the development repo). ProxmoxVE only
accepts scripts promoted from ProxmoxVED — direct PRs to ProxmoxVE with new scripts are
closed without review.

## Directory Layout

Files live under `scripts/pvehs/` in this repo, mirroring the ProxmoxVED directory structure:

```text
scripts/pvehs/
  ct/uptrakit.sh
  install/uptrakit-install.sh
  json/uptrakit.json
```

When submitted, all three files go to ProxmoxVED. ProxmoxVE has no `json/` directory.

## CT Creation Script — `ct/uptrakit.sh`

Sources PVEHS `misc/build.func` from the upstream raw URL. Declares all required variables,
delegates entirely to PVEHS framework functions — no custom logic.

| Variable          | Value                        |
| ----------------- | ---------------------------- |
| `APP`             | `uptrakit`                   |
| `var_disk`        | `4` (GB)                     |
| `var_cpu`         | `1`                          |
| `var_ram`         | `1024` (MB)                  |
| `var_os`          | `debian`                     |
| `var_version`     | `13`                         |
| `var_unprivileged`| `${var_unprivileged:-1}`     |
| `var_tags`        | `monitoring`                 |

Call sequence: `start` → `build_container` → `description`.
`description` takes no arguments; it reads `$APP` and CT IP from the environment.

## Install/Update Script — `install/uptrakit-install.sh`

First line: `source /dev/stdin <<<"$FUNCTIONS_FILE_PATH"` — framework injects `install.func`
content into this env var before running the script; it is not fetched by URL.

### Install path (top-level, flat — no wrapper function)

Mandatory preamble (six calls, in order):

```bash
color
verb_ip6
catch_errors
setting_up_container
network_check
update_os
```

Then install sequence:

1. `msg_info "Installing Dependencies"` — `$STD apt-get install -y tar openssl`; `msg_ok`.
   (`curl` is pre-installed by the framework bootstrap — do not include it here.)

2. `msg_info "Downloading uptrakit"` — resolve version, detect arch, deploy binary:

   ```bash
   RELEASE_TAG=$(get_latest_gh_tag "worried-networking/uptrakit" \
     "uptrakit-controller-standalone-v")
   arch=$(get_system_arch)
   case "$arch" in
     amd64) rust_target="x86_64-unknown-linux-gnu" ;;
     arm64) rust_target="aarch64-unknown-linux-gnu" ;;
     *) msg_error "Unsupported architecture: $arch"; exit 1 ;;
   esac
   tmp_dir=$(mktemp -d)
   fetch_and_deploy_gh_release \
     "uptrakit-controller-standalone" \
     "worried-networking/uptrakit" \
     "prebuild" \
     "$RELEASE_TAG" \
     "$tmp_dir" \
     "uptrakit-controller-standalone-*-${rust_target}.tar.gz"
   install -m 755 "$tmp_dir/uptrakit-controller-standalone" /usr/local/bin/
   rm -rf "$tmp_dir"
   ```

   Guard against no releases found:
   `[ -z "$RELEASE_TAG" ] && { msg_error "No uptrakit release found"; exit 1; }`

   `get_latest_gh_tag` filters by prefix `uptrakit-controller-standalone-v`, ensuring
   the correct release is fetched regardless of other release types in the repo. The
   `prebuild` mode extracts the archive to `$tmp_dir`; deploying via `install` keeps
   `/usr/local/bin` clean of extra archive contents (README, LICENSE, checksums).
   Framework caches version automatically in `~/.uptrakit-controller-standalone`.

   Implementation note: verify that `fetch_and_deploy_gh_release` (release-path) and
   `check_for_gh_tag` (tag-path) write and read the same cache file format. If the version
   strings differ in format (e.g. tag includes `uptrakit-controller-standalone-v` prefix),
   the update check will always report a new version. Test before shipping.
   `msg_ok`.

3. `msg_info "Creating service user"` —

   ```bash
   $STD addgroup --system uptrakit
   $STD adduser --system --home /opt/uptrakit --shell /usr/sbin/nologin \
     --no-create-home --ingroup uptrakit --disabled-login --disabled-password uptrakit
   $STD mkdir -p /opt/uptrakit/{config,state}
   $STD chown uptrakit:uptrakit /opt/uptrakit/{config,state}
   ```

   `msg_ok`.

4. `msg_info "Generating master key"` —

   ```bash
   MASTER_KEY=$(openssl rand -hex 32)
   echo "$MASTER_KEY" > /root/uptrakit_master_key
   chmod 600 /root/uptrakit_master_key
   printf 'UPTRAKIT_MASTER_KEY=%s\n' "$MASTER_KEY" > /opt/uptrakit/.env
   chmod 600 /opt/uptrakit/.env
   chown root:root /opt/uptrakit/.env
   ```

   `/opt/uptrakit/.env` dir already exists from step 3. File contains exactly one line:
   `UPTRAKIT_MASTER_KEY=<64-char hex>`.
   `msg_ok`.

5. `msg_info "Creating systemd service"` — write `/etc/systemd/system/uptrakit.service`
   (see below); `$STD systemctl enable --now uptrakit`; `msg_ok`.
   No `daemon-reload` — new unit files are read without it.

6. `msg_info "Fetching registration token"` — verify service started, then poll journal:

   ```bash
   for j in 1 2 3 4 5; do
     systemctl is-active --quiet uptrakit && break
     sleep 2
   done
   if ! systemctl is-active --quiet uptrakit; then
     msg_error "uptrakit failed to start"
     journalctl -u uptrakit --no-pager -n 20
     exit 1
   fi
   REGISTRATION_TOKEN=""
   for i in $(seq 1 30); do
     REGISTRATION_TOKEN=$(journalctl -u uptrakit --no-pager -o cat -n 100 \
       | grep -A1 "one-time registration token" \
       | tail -1 | tr -d ' ')
     echo "$REGISTRATION_TOKEN" | grep -qE '^[A-Za-z0-9_-]+$' || REGISTRATION_TOKEN=""
     [ -n "$REGISTRATION_TOKEN" ] && break
     sleep 2
   done
   if [ -z "$REGISTRATION_TOKEN" ]; then
     msg_error "Registration token not found after 60s"
     exit 1
   fi
   ```

   `-o cat` strips systemd journal metadata (timestamp, unit name, PID) from each line
   before grepping, so the token is extracted cleanly. `-n 100` provides buffer against
   verbose startup logging. Token format is validated as alphanumeric before accepting it.
   Total poll window: up to 60 s. `msg_ok`.

Mandatory footer (last three calls):

```bash
motd_ssh
customize
cleanup_lxc
```

Print completion banner after footer: CT IP, port 8443, registration token value,
path to master key file (`/root/uptrakit_master_key`) with explicit instruction to
back it up (data is unrecoverable without it).
Note: agents can be enrolled via the web UI without a pre-generated enrollment token.

### `update_script` function

Uses framework tag-aware release check. The repo publishes multiple release types
(`uptrakit-controller-v*`, `uptrakit-agent-v*`, etc.) so `/releases/latest` may not
return a controller-standalone release. Use `check_for_gh_tag` with prefix to filter:

```bash
update_script() {
  if ! check_for_gh_tag "uptrakit-controller-standalone" \
      "worried-networking/uptrakit" "uptrakit-controller-standalone-v"; then
    exit 0
  fi
  msg_info "Updating uptrakit"
  $STD systemctl stop uptrakit
  arch=$(get_system_arch)
  case "$arch" in
    amd64) rust_target="x86_64-unknown-linux-gnu" ;;
    arm64) rust_target="aarch64-unknown-linux-gnu" ;;
    *) msg_error "Unsupported architecture: $arch"; exit 1 ;;
  esac
  tmp_dir=$(mktemp -d)
  fetch_and_deploy_gh_release \
    "uptrakit-controller-standalone" \
    "worried-networking/uptrakit" \
    "prebuild" "latest" "$tmp_dir" \
    "uptrakit-controller-standalone-*-${rust_target}.tar.gz"
  install -m 755 "$tmp_dir/uptrakit-controller-standalone" /usr/local/bin/
  rm -rf "$tmp_dir"
  $STD systemctl start uptrakit
  msg_ok "Updated uptrakit"
}
```

### Systemd unit

```ini
[Unit]
Description=uptrakit Controller
After=network.target

[Service]
Type=simple
User=uptrakit
Group=uptrakit
EnvironmentFile=/opt/uptrakit/.env
ExecStart=/usr/local/bin/uptrakit-controller-standalone \
  --config-dir /opt/uptrakit/config \
  --state-dir /opt/uptrakit/state \
  --https-addr [::]:8443
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

`UPTRAKIT_MASTER_KEY` from `EnvironmentFile` is passed directly into the process environment
by systemd — no `PassEnvironment` directive required.

## JSON Metadata — `json/uptrakit.json`

```json
{
  "name": "uptrakit",
  "slug": "uptrakit",
  "categories": [9],
  "date_created": "2026-04-28",
  "type": "ct",
  "updateable": true,
  "privileged": false,
  "interface_port": 8443,
  "documentation": "https://uptrakit.org/docs",
  "website": "https://uptrakit.org",
  "logo": "https://raw.githubusercontent.com/worried-networking/uptrakit/main/frontend/static/favicon.svg",
  "description": "uptrakit is a self-hosted software update manager. Schedule, track, and audit package updates across servers, VMs, and containers from a single web UI.",
  "install_methods": [
    {
      "type": "default",
      "script": "ct/uptrakit.sh",
      "config_path": "/opt/uptrakit/.env",
      "resources": {
        "cpu": 1,
        "ram": 1024,
        "hdd": 4,
        "os": "Debian",
        "version": "13"
      }
    }
  ],
  "default_credentials": {
    "username": null,
    "password": null
  },
  "notes": []
}
```

Category `9` = "Monitoring & Analytics" in the PVEHS schema.

Logo note: PVEHS maintainers may replace the raw GitHub URL with a `cdn.jsdelivr.net/gh/selfhst/icons`
entry once uptrakit is catalogued there. The favicon.svg is a valid temporary placeholder.

## Architecture — PVEHS Framework Dependency

`ct/uptrakit.sh` sources `build.func` by URL (standard ct pattern). `install/uptrakit-install.sh`
receives `install.func` via `$FUNCTIONS_FILE_PATH` env var injected by `build_container`.
Framework provides: `fetch_and_deploy_gh_release`, `check_for_gh_tag`, `get_system_arch`,
`msg_info`, `msg_ok`, `msg_error`, `$STD`, and all preamble/footer helpers.
Zero duplicated framework code in our scripts.

## Asset Naming

Released binary archives follow the pattern:
`uptrakit-controller-standalone-{version}-{rust-target}.tar.gz`

Each archive contains a single binary `uptrakit-controller-standalone` at the archive root.
Rust targets in releases: `x86_64-unknown-linux-gnu` (amd64), `aarch64-unknown-linux-gnu` (arm64).
The install script maps `get_system_arch` output (`amd64`/`arm64`) to these target strings via
a `case` block before constructing the asset glob.

## Security Considerations

- Master key never printed to terminal or journal; written only to `/root/uptrakit_master_key`
  (root-only, 600).
- `/opt/uptrakit/.env` is 600, owned `root:root`. Systemd reads it as root before dropping
  privileges to the `uptrakit` user.
- No privileged CT capabilities required.
- No default credentials — first-run registration token is one-time-use and printed once at
  install completion.
- Service runs as dedicated `uptrakit` system user with no shell and no home directory write
  access outside `/opt/uptrakit/`.

## Implementation Notes

- **Version cache format**: verify that `fetch_and_deploy_gh_release` (releases API) and
  `check_for_gh_tag` (tags API) write/read the same cache file format. If the uptrakit
  repo ever publishes pre-release tags (`uptrakit-controller-standalone-v1.2.4-rc1`)
  without a corresponding GitHub Release, the update check may fire on every run. CI must
  ensure every git tag has a matching GitHub Release.
- **JSON category `9`**: verify against an existing ProxmoxVED JSON entry that category 9
  maps to "Monitoring & Analytics" before submitting the PR.
- **Submission timing**: submit to ProxmoxVED only after `uptrakit.org` is publicly live.
  Reviewers verify project legitimacy; an unreachable website causes immediate rejection.
- **Deployment doc**: `docs/end-user/deployment/proxmox-helper-scripts.md` must be created
  as part of this implementation (draft included with this spec).

## Out of Scope

- PostgreSQL support (users needing PG use the lean controller separately).
- Reverse proxy / Let's Encrypt integration.
- Multi-instance / NATS HA mode.
- OIDC bootstrap (configured via UI post-install).
- Bootstrap enrollment token (users create agent tokens via web UI).
