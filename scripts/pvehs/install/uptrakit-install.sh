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
