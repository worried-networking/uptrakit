---
title: Logs and Journald
weight: 260
description: Reading Uptrakit logs under systemd, filtering by priority and fields, and switching the output format.
---

# Logs and Journald

When an Uptrakit daemon runs under systemd with its output connected to the journal, it logs in
native journald format: clean messages, real priorities, and structured fields. In a terminal or
with redirected output it prints classic text lines instead. Selection is automatic; see below for
the override.

## Reading logs

```sh
journalctl -u uptrakit                      # clean one-line messages
journalctl -u uptrakit -p warning           # warnings and errors only
journalctl -u uptrakit -p notice            # INFO and up (DEBUG excluded)
journalctl -u uptrakit -o verbose           # full structured view (PRIORITY, TARGET, fields)
journalctl -u uptrakit F_HOST_ID=<uuid>     # filter by a structured field
```

Event fields such as `host_id` are stored as indexed journal fields with an `F` prefix
(`F_HOST_ID`), not inside the message text. Plain `grep host_id=` over `journalctl` output (and
`journalctl -g`, which searches `MESSAGE` only) will no longer match them — use the field-filter
form above.

## Forcing a format

Set `UPTRAKIT_LOG_FORMAT` to `auto` (default), `text`, or `journald`. On a deployed unit, use a
systemd drop-in so the setting survives reinstalls (the installer rewrites
`/etc/systemd/system/uptrakit.service` on every run — direct edits to the unit file are lost):

```sh
mkdir -p /etc/systemd/system/uptrakit.service.d
printf '[Service]\nEnvironment=UPTRAKIT_LOG_FORMAT=text\n' \
  > /etc/systemd/system/uptrakit.service.d/logging.conf
systemctl daemon-reload && systemctl restart uptrakit
```

## Caveats

- `StandardOutput=journal+console`: stdout still identifies as the journal, so journald mode
  activates and the console tee stops showing log lines. Use the `text` override if you need the
  console copy.
- Structured entries store more bytes per event than text lines. At high verbosity this shortens
  the journal's effective retention window — check with `journalctl --disk-usage` and prefer
  moderate verbosity in production.
