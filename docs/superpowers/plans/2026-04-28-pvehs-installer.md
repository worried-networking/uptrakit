# PVEHS Installer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create three files under `scripts/pvehs/` that implement a Proxmox VE Helper-Scripts
installer for the `uptrakit-controller-standalone` binary.

**Architecture:** A PVEHS-framework CT creation script sources `build.func` by URL, declares
variables, defines `update_script()`, and delegates to `start`/`build_container`/`description`.
The install script receives `install.func` via `$FUNCTIONS_FILE_PATH` and runs flat top-level
install code only — no functions. JSON metadata describes the app for the PVEHS web UI.

**Tech Stack:** Bash 5, PVEHS framework (`build.func` / `install.func` / `tools.func`),
`shellcheck` for static analysis, `python3` for JSON validation.

---

## File Map

| File | Action | Responsibility |
| ---- | ------ | -------------- |
| `scripts/pvehs/ct/uptrakit.sh` | Create | CT creation, `update_script()`, framework delegation |
| `scripts/pvehs/install/uptrakit-install.sh` | Create | Flat install path only — no functions |
| `scripts/pvehs/json/uptrakit.json` | Create | PVEHS web UI metadata |

`update_script()` lives in `ct/uptrakit.sh` — that is where the PVEHS framework calls it from.
The install script is flat top-level code only; defining functions in it has no effect on updates.

The deployment doc at `docs/end-user/deployment/proxmox-helper-scripts.md` was created
during the spec phase. Task 4 verifies it exists.

---

## Task 1: Directory scaffold + JSON metadata

**Files:**

- Create: `scripts/pvehs/json/uptrakit.json`

- [ ] **Step 1: Create directory structure**

```bash
mkdir -p scripts/pvehs/{ct,install,json}
```

No `.gitkeep` files — the scripts are added in the same task as the directories.

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

```bash
python3 -m json.tool scripts/pvehs/json/uptrakit.json > /dev/null && echo "OK"
```

Expected: `OK`

- [ ] **Step 4: Verify category 9 maps to "Monitoring & Analytics"**

```bash
curl -fsSL \
  "https://raw.githubusercontent.com/community-scripts/ProxmoxVED/main/json/uptime-kuma.json" \
  | python3 -c "import json,sys; d=json.load(sys.stdin); print(d['categories'])"
```

Expected: `[9]`. If the number differs, update `categories` in `uptrakit.json` accordingly.

- [ ] **Step 5: Commit**

```bash
git add scripts/pvehs/json/uptrakit.json
git commit -m "feat(pvehs): add JSON metadata for uptrakit CT"
```

---

## Task 2: CT creation script (includes `update_script`)

**Files:**

- Create: `scripts/pvehs/ct/uptrakit.sh`

The ct script runs on the **Proxmox VE host** (not inside the CT). `build.func` is sourced by
URL at runtime. `update_script()` must be defined here — the PVEHS framework calls it from this
file's context when the user updates the CT. The install script is flat code only and never
receives an `update_script` call.

`check_for_gh_tag` is used instead of the more common `check_for_gh_release` because the
uptrakit repo publishes multiple release types (`uptrakit-controller-v*`, `uptrakit-agent-v*`,
etc.) and `/releases/latest` may return the wrong one. `check_for_gh_tag` filters by the
`uptrakit-controller-standalone-v` prefix to target only standalone releases.

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
variables
color
catch_errors

function update_script() {
  header_info
  if ! check_for_gh_tag "uptrakit-controller-standalone" \
    "worried-networking/uptrakit" "uptrakit-controller-standalone-v"; then
    msg_ok "No update required"
    exit 0
  fi
  msg_info "Updating uptrakit"
  systemctl stop uptrakit
  arch=$(get_system_arch)
  case "$arch" in
    amd64) rust_target="x86_64-unknown-linux-gnu" ;;
    arm64) rust_target="aarch64-unknown-linux-gnu" ;;
    *) msg_error "Unsupported architecture: $arch"; exit 1 ;;
  esac
  tmp_dir=$(mktemp -d) || { msg_error "Failed to create temp dir"; exit 1; }
  fetch_and_deploy_gh_release \
    "uptrakit-controller-standalone" \
    "worried-networking/uptrakit" \
    "prebuild" "latest" "$tmp_dir" \
    "uptrakit-controller-standalone-*-${rust_target}.tar.gz"
  install -m 755 "$tmp_dir/uptrakit-controller-standalone" /usr/local/bin/
  rm -rf "$tmp_dir"
  # Note: update_script runs inside the CT — start() detects no pveversion, skips install path.
  systemctl start uptrakit
  msg_ok "Updated uptrakit"
  exit
}

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

Expected: no errors. Warnings about unresolvable framework functions from the sourced
`build.func` are suppressed by `# shellcheck source=/dev/null`.

- [ ] **Step 4: Commit**

```bash
git add scripts/pvehs/ct/uptrakit.sh
git commit -m "feat(pvehs): add CT creation script with update_script"
```

---

## Task 3: Install script — flat install path

**Files:**

- Create: `scripts/pvehs/install/uptrakit-install.sh`

This script runs **inside the new LXC container** after `build_container` creates it.
`install.func` is injected via `$FUNCTIONS_FILE_PATH` env var — the script does NOT fetch
it by URL. All framework functions (`msg_info`, `msg_ok`, `msg_error`, `$STD`,
`get_latest_gh_tag`, `fetch_and_deploy_gh_release`, `get_system_arch`, `color`,
`verb_ip6`, `catch_errors`, `setting_up_container`, `network_check`, `update_os`,
`motd_ssh`, `customize`, `cleanup_lxc`, `$IP`) come from this injected content.

`$IP` is set by the framework's `setting_up_container`/`network_check` — use it in the
banner instead of `hostname -I`. No `set -euo pipefail` needed — `catch_errors` in the
preamble already sets this up.

- [ ] **Step 1: Write `scripts/pvehs/install/uptrakit-install.sh`**

```bash
#!/usr/bin/env bash
# shellcheck source=/dev/null
# shellcheck disable=SC1091
source /dev/stdin <<<"$FUNCTIONS_FILE_PATH"

APP="uptrakit"

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
tmp_dir=$(mktemp -d) || { msg_error "Failed to create temp dir"; exit 1; }
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
for i in {1..30}; do
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

msg_ok "Completed Successfully!\n"
echo -e "${GN}${APP} is running at https://${IP}:8443${CL}"
echo -e "${YW}Registration token:${CL} ${REGISTRATION_TOKEN}"
echo -e "${YW}Master key:${CL} /opt/uptrakit/.env — back this up!"
```

Note on the systemd unit heredoc: `<<'EOF'` (single-quoted) suppresses shell variable
expansion — intentional, since the unit file has no shell variables. The `\` continuation
on `ExecStart` is passed literally to the file; systemd supports backslash line
continuation in unit files.

- [ ] **Step 2: Make executable**

```bash
chmod +x scripts/pvehs/install/uptrakit-install.sh
```

- [ ] **Step 3: Run shellcheck**

```bash
shellcheck --severity=error scripts/pvehs/install/uptrakit-install.sh
```

Expected: no errors. `# shellcheck disable=SC1091` on the source line suppresses the
"can't follow source" error from the herestring injection pattern.

- [ ] **Step 4: Commit**

```bash
git add scripts/pvehs/install/uptrakit-install.sh
git commit -m "feat(pvehs): add install script"
```

---

## Task 4: Final validation checklist

**Files:** none modified — verification only.

- [ ] **Step 1: Verify all three files exist and are executable where applicable**

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

Fix any warnings that are not framework-related (undefined functions sourced from
`$FUNCTIONS_FILE_PATH` are expected).

- [ ] **Step 3: Validate JSON**

```bash
python3 -m json.tool scripts/pvehs/json/uptrakit.json
```

Expected: pretty-printed JSON, no error.

- [ ] **Step 4: Verify deployment doc exists**

```bash
ls docs/end-user/deployment/proxmox-helper-scripts.md && echo "OK"
```

Expected: `OK`. If missing, it was created during the spec phase — check git log:
`git log --oneline -- docs/end-user/deployment/proxmox-helper-scripts.md`

- [ ] **Step 5: Confirm asset naming matches the glob in the install script**

Run on developer machine (requires internet + python3):

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

Expected output example: `uptrakit-controller-standalone-v0.1.0-x86_64-unknown-linux-gnu.tar.gz`
(note: archive filename may or may not include the `v` prefix from the tag; the glob
`uptrakit-controller-standalone-*-x86_64-unknown-linux-gnu.tar.gz` matches either way).
If the naming pattern differs, update the globs in `ct/uptrakit.sh` (update_script) and
`install/uptrakit-install.sh` accordingly.

- [ ] **Step 6: Final commit**

```bash
git add scripts/pvehs/
git commit -m "feat(pvehs): complete PVEHS installer scripts for uptrakit-controller-standalone"
```
