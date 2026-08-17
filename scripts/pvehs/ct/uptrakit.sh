#!/usr/bin/env bash
# shellcheck source=/dev/null
# shellcheck disable=SC2034

# Upstream build.func hardcodes the application install script URL to
# `community-scripts/ProxmoxVE/main/install/<app>-install.sh`. Our install
# script lives in this fork at `scripts/pvehs/install/`, so we patch the
# URL prefix in build.func before sourcing it. Two occurrences (initial
# install + APT-repair retry) are replaced.
UPTRAKIT_UPSTREAM_INSTALL_PATH='community-scripts/ProxmoxVE/main/install/'
UPTRAKIT_FORK_INSTALL_PATH='worried-networking/uptrakit/main/scripts/pvehs/install/'
UPTRAKIT_BUILD_FUNC="$(curl -fsSL \
  https://raw.githubusercontent.com/community-scripts/ProxmoxVE/main/misc/build.func)"
UPTRAKIT_BUILD_FUNC="${UPTRAKIT_BUILD_FUNC//${UPTRAKIT_UPSTREAM_INSTALL_PATH}/${UPTRAKIT_FORK_INSTALL_PATH}}"
source <(printf '%s' "$UPTRAKIT_BUILD_FUNC")
unset UPTRAKIT_BUILD_FUNC UPTRAKIT_UPSTREAM_INSTALL_PATH UPTRAKIT_FORK_INSTALL_PATH

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

# Non-fatal: every branch is a tested condition or a message, so build.func's
# errexit trap never fires here — an unprovisioned host keeps its existing
# sudoers drop-in and the operator gets a retry command.
function provision_embedded_agent() {
  msg_info "Provisioning host for the embedded agent"
  if ! /usr/local/bin/uptrakit-controller-standalone agent bootstrap-host --help >/dev/null 2>&1; then
    msg_error "Installed binary lacks 'agent bootstrap-host' (pre-rollout release) — run update again after the next release"
  elif timeout 120 /usr/local/bin/uptrakit-controller-standalone agent bootstrap-host --user uptrakit; then
    msg_ok "Provisioned host for the embedded agent"
  else
    msg_error "Host bootstrap failed — run '/usr/local/bin/uptrakit-controller-standalone agent bootstrap-host --user uptrakit' as root to retry"
  fi
}

function update_script() {
  header_info
  if ! check_for_gh_tag "uptrakit-controller-standalone" \
    "worried-networking/uptrakit" "uptrakit-controller-standalone-v"; then
    msg_ok "No update required"
    provision_embedded_agent
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
  RELEASE_TAG=$(get_latest_gh_tag "worried-networking/uptrakit" \
    "uptrakit-controller-standalone-v")
  [ -z "$RELEASE_TAG" ] && { msg_error "No uptrakit release found"; exit 1; }
  fetch_and_deploy_gh_release \
    "uptrakit-controller-standalone" \
    "worried-networking/uptrakit" \
    "prebuild" "$RELEASE_TAG" "$tmp_dir" \
    "uptrakit-controller-standalone-*-${rust_target}.tar.gz"
  install -m 755 "$tmp_dir/uptrakit-controller-standalone" /usr/local/bin/
  # Note: update_script runs inside the CT — start() detects no pveversion, skips install path.
  systemctl start uptrakit
  msg_ok "Updated uptrakit"
  provision_embedded_agent
  exit 0
}

start
build_container
description
