#!/bin/sh
# Managed by Uptrakit — do not edit manually.
# Regenerate: uptrakit-agent-ssh host update-sudoers <host>
#
# Runs the Proxmox Helper Scripts (PHS) update tool in unattended mode.
# Without PHS_SILENT=1 the update script may launch a whiptail dialog that
# blocks indefinitely waiting for interactive input.
#
# The corresponding sudoers rule requires no wildcards:
#
#   uptrakit ALL=(root) NOPASSWD: /usr/local/bin/uptrakit-phs-update
#
# Usage:
#   uptrakit-phs-update
#
# The script takes no arguments — it always runs the full PHS update pass
# for all managed containers on this Proxmox node.

set -eu

exec env PHS_SILENT=1 /usr/bin/update
