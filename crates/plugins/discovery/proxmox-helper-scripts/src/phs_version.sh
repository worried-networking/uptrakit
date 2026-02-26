#!/bin/sh
# Managed by Uptrakit — do not edit manually.
# Regenerate: uptrakit-agent-ssh host update-sudoers <host>
#
# Reads /root/.<slug> for Proxmox Helper Scripts (PHS) version detection.
# PHS scripts execute via `pct exec` as root and write their version files
# under /root/.<slug>.  This helper enforces strict argument validation so
# that the corresponding sudoers rule requires no wildcards:
#
#   uptrakit ALL=(root) NOPASSWD: /usr/local/bin/uptrakit-phs-version
#
# Usage:
#   uptrakit-phs-version <slug>
#
#   <slug>  PHS application slug — must consist entirely of lowercase ASCII
#           letters (a-z), digits (0-9), and hyphens (-).  Must not be empty,
#           start with a hyphen, or end with a hyphen.  This mirrors the PHS
#           slug validation in the Uptrakit plugin crate.

set -eu

if [ "$#" -ne 1 ]; then
    printf 'usage: uptrakit-phs-version <slug>\n' >&2
    exit 1
fi

slug="$1"

# Reject empty slugs and those with leading or trailing hyphens.
case "$slug" in
    '' | -* | *-)
        printf 'uptrakit-phs-version: invalid PHS slug (empty or leading/trailing hyphen): %s\n' \
            "$slug" >&2
        exit 1
        ;;
esac

# Reject slugs containing any character outside [a-z0-9-].
# This prevents path traversal (no '/', no '.', no whitespace, no shell
# metacharacters).  The case pattern [!a-z0-9-] matches any forbidden char.
case "$slug" in
    *[!a-z0-9-]*)
        printf 'uptrakit-phs-version: invalid PHS slug (forbidden characters): %s\n' \
            "$slug" >&2
        exit 1
        ;;
esac

cat -- "/root/.${slug}"
