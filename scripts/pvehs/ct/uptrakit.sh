#!/usr/bin/env bash
# shellcheck source=/dev/null
# shellcheck disable=SC2034
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
  trap 'rm -rf "$tmp_dir"' EXIT
  fetch_and_deploy_gh_release \
    "uptrakit-controller-standalone" \
    "worried-networking/uptrakit" \
    "prebuild" "latest" "$tmp_dir" \
    "uptrakit-controller-standalone-*-${rust_target}.tar.gz"
  install -m 755 "$tmp_dir/uptrakit-controller-standalone" /usr/local/bin/
  # Note: update_script runs inside the CT — start() detects no pveversion, skips install path.
  systemctl start uptrakit
  msg_ok "Updated uptrakit"
  exit
}

start
build_container
description
