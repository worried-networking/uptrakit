# PVEHS Installer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create three files under `scripts/pvehs/` that implement a Proxmox VE Helper-Scripts
installer for the `uptrakit-controller-standalone` binary.

**Architecture:** A PVEHS-framework CT creation script sources `build.func` by URL and delegates
entirely to framework functions. The install script receives `install.func` via `$FUNCTIONS_FILE_PATH`
and runs flat top-level code for the install path plus an `update_script()` function for updates.
A JSON metadata file describes the app for the PVEHS web UI.

**Tech Stack:** Bash 5, PVEHS framework (`build.func` / `install.func` / `tools.func`),
`shellcheck` for static analysis, `python3` for JSON validation.

---

## File Map

| File | Action | Responsibility |
| ---- | ------ | -------------- |
| `scripts/pvehs/ct/uptrakit.sh` | Create | CT creation — variables + framework delegation |
| `scripts/pvehs/install/uptrakit-install.sh` | Create | Install path + `update_script` function |
| `scripts/pvehs/json/uptrakit.json` | Create | PVEHS web UI metadata |

The deployment doc at `docs/end-user/deployment/proxmox-helper-scripts.md` was already
created during the spec phase.

---

## Task 1: Directory scaffold + JSON metadata

**Files:**

- Create: `scripts/pvehs/ct/.gitkeep` (dir marker, removed after ct script added)
- Create: `scripts/pvehs/install/.gitkeep`
- Create: `scripts/pvehs/json/uptrakit.json`

- [ ] **Step 1: Create directory structure**

```bash
mkdir -p scripts/pvehs/{ct,install,json}
```

- [ ] **Step 2: Write JSON metadata**

Create `scripts/pvehs/json/uptrakit.json`:

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

- [ ] **Step 3: Validate JSON**

Run:

```bash
python3 -m json.tool scripts/pvehs/json/uptrakit.json > /dev/null && echo "OK"
```

Expected: `OK`

- [ ] **Step 4: Verify category 9 against ProxmoxVED**

Fetch a real ProxmoxVED JSON that uses category 9 and confirm it maps to
"Monitoring & Analytics":

```bash
curl -fsSL "https://raw.githubusercontent.com/community-scripts/ProxmoxVED/main/json/uptime-kuma.json" \
  | python3 -m json.tool | grep -A2 categories
```

Expected: `"categories": [9]` (uptime-kuma is a monitoring app; if the number differs,
update `uptrakit.json` accordingly).

- [ ] **Step 5: Commit**

```bash
git add scripts/pvehs/json/uptrakit.json
git commit -m "feat(pvehs): add JSON metadata for uptrakit CT"
```

---

## Task 2: CT creation script

**Files:**

- Create: `scripts/pvehs/ct/uptrakit.sh`

The ct script has no logic of its own — it declares variables and delegates to PVEHS
framework functions. `build.func` is sourced by URL at runtime on the Proxmox host.
`shellcheck` will warn about functions it cannot resolve; suppress with a directive.

- [ ] **Step 1: Write `scripts/pvehs/ct/uptrakit.sh`**

```bash
#!/usr/bin/env bash
# shellcheck source=/dev/null
source <(curl -fsSL https://raw.githubusercontent.com/community-scripts/ProxmoxVE/main/misc/build.func)

# Default Settings
APP="uptrakit"
var_disk="4"
var_cpu="1"
var_ram="1024"
var_os="debian"
var_version="13"
var_unprivileged="${var_unprivileged:-1}"
var_tags="monitoring"

# App Output & Interface
header_info "$APP"
base_settings
while true; do
  if variables_and_settings; then
    break
  fi
done

start
build_container
description
```

- [ ] **Step 2: Make executable**

```bash
chmod +x scripts/pvehs/ct/uptrakit.sh
```

- [ ] **Step 3: Run shellcheck**

```bash
shellcheck --severity=error scripts/pvehs/ct/uptrakit.sh
```

Expected: no errors. Warnings about unresolvable `source` or undefined framework
functions are suppressed by `# shellcheck source=/dev/null` and are expected.
If `shellcheck` is not installed: `brew install shellcheck` or `apt-get install shellcheck`.

- [ ] **Step 4: Commit**

```bash
git add scripts/pvehs/ct/uptrakit.sh
git commit -m "feat(pvehs): add CT creation script"
```

---

## Task 3: Install script — install path

**Files:**

- Create: `scripts/pvehs/install/uptrakit-install.sh`

The install script runs **inside the new LXC container** after `build_container` creates
it. `install.func` is injected via `$FUNCTIONS_FILE_PATH` — the script does NOT fetch it
by URL. All framework functions (`msg_info`, `msg_ok`, `msg_error`, `$STD`,
`get_latest_gh_tag`, `fetch_and_deploy_gh_release`, `get_system_arch`, `color`,
`verb_ip6`, `catch_errors`, `setting_up_container`, `network_check`, `update_os`,
`motd_ssh`, `customize`, `cleanup_lxc`) come from this injected content.

- [ ] **Step 1: Write `scripts/pvehs/install/uptrakit-install.sh`**

```bash
#!/usr/bin/env bash
# shellcheck source=/dev/null
source /dev/stdin <<<"$FUNCTIONS_FILE_PATH"
color
verb_ip6
catch_errors
setting_up_container
network_check
update_os

msg_info "Installing Dependencies"
$STD apt-get install -y tar openssl
msg_ok "Installed Dependencies"

msg_info "Downloading uptrakit"
RELEASE_TAG=$(get_latest_gh_tag "worried-networking/uptrakit" \
  "uptrakit-controller-standalone-v")
[ -z "$RELEASE_TAG" ] && { msg_error "No uptrakit release found"; exit 1; }
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
msg_ok "Downloaded uptrakit ${RELEASE_TAG}"

msg_info "Creating service user"
$STD addgroup --system uptrakit
$STD adduser --system --home /opt/uptrakit --shell /usr/sbin/nologin \
  --no-create-home --ingroup uptrakit --disabled-login --disabled-password uptrakit
$STD mkdir -p /opt/uptrakit/{config,state}
$STD chown uptrakit:uptrakit /opt/uptrakit/{config,state}
msg_ok "Created service user"

msg_info "Generating master key"
MASTER_KEY=$(openssl rand -hex 32)
printf 'UPTRAKIT_MASTER_KEY=%s\n' "$MASTER_KEY" >/opt/uptrakit/.env
chmod 600 /opt/uptrakit/.env
chown root:root /opt/uptrakit/.env
msg_ok "Generated master key"

msg_info "Creating systemd service"
cat <<'EOF' >/etc/systemd/system/uptrakit.service
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
EOF
$STD systemctl enable --now uptrakit
msg_ok "Created and started systemd service"

msg_info "Fetching registration token"
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
msg_ok "Fetched registration token"

motd_ssh
customize
cleanup_lxc

echo -e "${APP} installation complete!

  Access the web UI: https://$(hostname -I | awk '{print $1}'):8443
  Registration token: ${REGISTRATION_TOKEN}
  Master key stored at: /opt/uptrakit/.env — back this up!
"
```

- [ ] **Step 2: Make executable**

```bash
chmod +x scripts/pvehs/install/uptrakit-install.sh
```

- [ ] **Step 3: Run shellcheck**

```bash
shellcheck --severity=error scripts/pvehs/install/uptrakit-install.sh
```

Expected: no errors. Warnings about `$FUNCTIONS_FILE_PATH` and undefined framework
functions are expected and suppressed by `# shellcheck source=/dev/null`.

- [ ] **Step 4: Commit**

```bash
git add scripts/pvehs/install/uptrakit-install.sh
git commit -m "feat(pvehs): add install script — install path"
```

---

## Task 4: Install script — add `update_script` function

**Files:**

- Modify: `scripts/pvehs/install/uptrakit-install.sh` (append function at end)

`update_script()` is called by the PVEHS framework when the user runs the CT update
mechanism. It uses `check_for_gh_tag` (tag-based) to detect whether a newer
`uptrakit-controller-standalone-v*` tag exists. If yes, it stops the service, replaces
the binary, and restarts. The same `get_latest_gh_tag` call used at install time is
reused here to ensure consistent version resolution.

**Version cache note:** `fetch_and_deploy_gh_release` writes the resolved version to
`~/.uptrakit-controller-standalone`. `check_for_gh_tag` reads from the same file.
Both use the full tag string (e.g. `uptrakit-controller-standalone-v0.1.0`) — formats
align. If `check_for_gh_tag` always reports an update on first post-install run, the
cache was not written by `fetch_and_deploy_gh_release`; investigate the framework's
`CACHED_VERSION` logic in `tools.func` and adjust accordingly.

- [ ] **Step 1: Append `update_script` to `scripts/pvehs/install/uptrakit-install.sh`**

Add this block at the very end of the file (after the completion banner echo):

```bash
update_script() {
  if ! check_for_gh_tag "uptrakit-controller-standalone" \
    "worried-networking/uptrakit" "uptrakit-controller-standalone-v"; then
    exit 0
  fi
  msg_info "Updating uptrakit"
  $STD systemctl stop uptrakit
  RELEASE_TAG=$(get_latest_gh_tag "worried-networking/uptrakit" \
    "uptrakit-controller-standalone-v")
  [ -z "$RELEASE_TAG" ] && { msg_error "No uptrakit release found"; exit 1; }
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
    "prebuild" "$RELEASE_TAG" "$tmp_dir" \
    "uptrakit-controller-standalone-*-${rust_target}.tar.gz"
  install -m 755 "$tmp_dir/uptrakit-controller-standalone" /usr/local/bin/
  rm -rf "$tmp_dir"
  $STD systemctl start uptrakit
  msg_ok "Updated uptrakit to ${RELEASE_TAG}"
}
```

- [ ] **Step 2: Run shellcheck on the full file**

```bash
shellcheck --severity=error scripts/pvehs/install/uptrakit-install.sh
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add scripts/pvehs/install/uptrakit-install.sh
git commit -m "feat(pvehs): add update_script function to install script"
```

---

## Task 5: Final validation checklist

**Files:** none modified — verification only.

These steps confirm the scripts are well-formed and catch any remaining issues before
submitting to ProxmoxVED.

- [ ] **Step 1: Verify all three files exist and are executable (where applicable)**

```bash
ls -la scripts/pvehs/ct/uptrakit.sh \
       scripts/pvehs/install/uptrakit-install.sh \
       scripts/pvehs/json/uptrakit.json
```

Expected: `ct/uptrakit.sh` and `install/uptrakit-install.sh` are `-rwxr-xr-x`.
`uptrakit.json` is a regular file.

- [ ] **Step 2: shellcheck both scripts at warning level**

```bash
shellcheck --severity=warning scripts/pvehs/ct/uptrakit.sh
shellcheck --severity=warning scripts/pvehs/install/uptrakit-install.sh
```

Fix any warnings that are not framework-related (undefined functions from
`$FUNCTIONS_FILE_PATH` injection are expected and OK).

- [ ] **Step 3: Validate JSON is well-formed**

```bash
python3 -m json.tool scripts/pvehs/json/uptrakit.json
```

Expected: pretty-printed JSON with no error.

- [ ] **Step 4: Confirm asset naming matches the glob in the install script**

Check the latest release assets in the repo:

```bash
curl -fsSL "https://api.github.com/repos/worried-networking/uptrakit/releases" \
  | python3 -c "
import json, sys
releases = json.load(sys.stdin)
for r in releases:
    if r['tag_name'].startswith('uptrakit-controller-standalone-v'):
        for a in r['assets']:
            print(a['name'])
        break
" 2>/dev/null || echo "No releases yet — verify manually when first release is tagged"
```

Expected: asset names like
`uptrakit-controller-standalone-0.1.0-x86_64-unknown-linux-gnu.tar.gz`.
Confirm the glob `uptrakit-controller-standalone-*-x86_64-unknown-linux-gnu.tar.gz`
matches at least one asset. If assets use a different naming pattern, update the
glob in both `install_script` and `update_script` accordingly.

- [ ] **Step 5: Commit final state**

```bash
git add scripts/pvehs/
git commit -m "feat(pvehs): complete PVEHS installer scripts for uptrakit-controller-standalone"
```
