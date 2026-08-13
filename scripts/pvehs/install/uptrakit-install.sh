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
$STD apt-get install -y tar openssl sudo
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
# Controller creates XDG dirs (.config, .local/state) under $HOME on first run,
# so the whole home tree must be uptrakit-owned, not just the explicit subdirs.
$STD chown -R uptrakit:uptrakit /opt/uptrakit
msg_ok "Created service user"

msg_info "Generating master key"
openssl rand -hex 32 >/opt/uptrakit/master.key
chmod 600 /opt/uptrakit/master.key
chown uptrakit:uptrakit /opt/uptrakit/master.key
msg_ok "Generated master key"

msg_info "Installing sudoers drop-in for uptrakit user"
cat >/etc/sudoers.d/uptrakit-uptrakit.tmp <<'SUDOERS'
# Managed by Uptrakit - DO NOT EDIT MANUALLY
# Regenerate: uptrakit-agent-ssh host sync <host>
# /bin/install: Install downloaded GitHub release assets to the target path
uptrakit ALL=(root) NOPASSWD: /bin/install
# /bin/systemctl stop *: Stop services before GitHub release asset installation
uptrakit ALL=(root) NOPASSWD: /bin/systemctl stop *
# /bin/systemctl start *: Start services after GitHub release asset installation
uptrakit ALL=(root) NOPASSWD: /bin/systemctl start *
# /usr/local/bin/uptrakit-phs-version: Reads /root/.<slug> for PHS version detection; the helper script validates the slug argument to prevent path traversal
uptrakit ALL=(root) NOPASSWD: /usr/local/bin/uptrakit-phs-version
# /usr/bin/update: Runs /usr/bin/update with PHS_SILENT=1 and TERM=xterm for PHS container updates over a PTY; SETENV: is required so the agent can pass the env vars inline in the sudo call
uptrakit ALL=(root) NOPASSWD: SETENV: /usr/bin/update
# /bin/apt-get update *: Package index refresh requires root privileges
uptrakit ALL=(root) NOPASSWD: SETENV: /bin/apt-get update *
# /bin/apt-get install *: Package installation requires root privileges
uptrakit ALL=(root) NOPASSWD: SETENV: /bin/apt-get install *
# /bin/apt-get -o Dir::Etc::Preferences=/tmp/uptrakit-apt-batch.pref upgrade *: Batch package upgrade (pinned versions) requires root privileges
uptrakit ALL=(root) NOPASSWD: SETENV: /bin/apt-get -o Dir\:\:Etc\:\:Preferences\=/tmp/uptrakit-apt-batch.pref upgrade *
SUDOERS
chmod 0440 /etc/sudoers.d/uptrakit-uptrakit.tmp
if visudo -cf /etc/sudoers.d/uptrakit-uptrakit.tmp >/dev/null; then
  mv /etc/sudoers.d/uptrakit-uptrakit.tmp /etc/sudoers.d/uptrakit-uptrakit
  msg_ok "Installed sudoers drop-in for uptrakit user"
else
  rm -f /etc/sudoers.d/uptrakit-uptrakit.tmp
  msg_error "sudoers drop-in failed visudo validation"
  exit 1
fi

msg_info "Writing controller config"
# Resolve container's primary IPv4 for the PKI advertise URL. Operators on
# DHCP setups should switch this to a stable hostname after install.
CTRL_IP="$(hostname -I | awk '{print $1}')"
cat >/opt/uptrakit/config/controller.toml <<EOF
master_key = "file:/opt/uptrakit/master.key"

[db]
url = "sqlite:///opt/uptrakit/state/controller.db?mode=rwc"

[network]
addr = "[::]:8443"
# PKI HTTP listener for certificate issuance. \`http://\` form is required —
# bare host:port advertises but does not bind. Edit this to use a stable
# DNS name once DNS is configured for the controller.
pki_addr = "http://${CTRL_IP}:8080"
EOF
chown uptrakit:uptrakit /opt/uptrakit/config/controller.toml
chmod 640 /opt/uptrakit/config/controller.toml
msg_ok "Wrote controller config"

msg_info "Creating systemd service"
cat <<'EOF' >/etc/systemd/system/uptrakit.service
[Unit]
Description=uptrakit Controller
After=network.target

[Service]
Type=simple
User=uptrakit
Group=uptrakit
ExecStart=/usr/local/bin/uptrakit-controller-standalone --config /opt/uptrakit/config/controller.toml
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF
$STD systemctl enable --now uptrakit
msg_ok "Created and started systemd service"

msg_info "Fetching registration token"
for _ in 1 2 3 4 5; do
  systemctl is-active --quiet uptrakit && break
  sleep 2
done
if ! systemctl is-active --quiet uptrakit; then
  msg_error "uptrakit failed to start"
  journalctl -u uptrakit --no-pager -n 20
  exit 1
fi
REGISTRATION_TOKEN=""
for _ in {1..30}; do
  # `|| true` is load-bearing: build.func runs under `set -Eeuo pipefail`, so a
  # grep miss (journal not flushed yet) would abort the install instead of
  # letting this poll loop retry.
  REGISTRATION_TOKEN=$(journalctl -u uptrakit --no-pager -o cat \
    | grep -A1 "one-time registration token" \
    | tail -1 | tr -d ' ' || true)
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

# Override /usr/bin/update written by upstream customize() — its hardcoded
# URL points at community-scripts/ProxmoxVE which doesn't host our script.
cat >/usr/bin/update <<'UPDATEEOF'
#!/usr/bin/env bash
bash -c "$(curl -fsSL https://raw.githubusercontent.com/worried-networking/uptrakit/main/scripts/pvehs/ct/uptrakit.sh)"
UPDATEEOF
chmod +x /usr/bin/update

cleanup_lxc

msg_ok "Completed Successfully!\n"
# Upstream install.func exports no `IP`; under `set -u` referencing it aborts the
# install after every step already succeeded. Reuse the address resolved above.
echo -e "${GN}${APP} is running at https://${CTRL_IP}:8443${CL}"
echo -e "${GN}PKI HTTP endpoint: http://${CTRL_IP}:8080${CL}"
echo -e "${YW}Registration token:${CL} ${REGISTRATION_TOKEN}"
echo -e "${YW}Master key:${CL} /opt/uptrakit/master.key — back this up!"
