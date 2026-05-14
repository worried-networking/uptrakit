---
title: Operator Runbook — Graceful Reload
weight: 115
description: How to trigger, monitor, and recover from graceful config reloads in the Uptrakit Controller.
---

# Operator Runbook — Graceful Reload

This runbook covers the three reload triggers, how to read reload state from the API and Dashboard,
and how to recover from each failure mode.

## Triggering a Reload

### SIGHUP

```sh
# Using the PID directly
kill -HUP <pid>

# Using systemd
systemctl reload uptrakit-controller
```

The controller re-reads `controller.toml` and applies changes in-process. If the change includes an
irreversibly-bound key (listen address, DB pool URL, TLS trust domain), the controller calls `exec()`
to replace the process image instead.

### File Edit + File-Watch

Edit `controller.toml` and save. The built-in file-watcher detects the change within 500 ms (debounce)
and triggers an automatic reload — no manual signal required.

### Dashboard / API Mutation

Mutate a setting via the Dashboard or API (requires the `If-Match` header). The `ConfigReconciler`
detects the incremented `settings_version` within 2 s and triggers a reload.

## Reading Reload State

### API

```http
GET /api/v1/instance/config-state
```

Requires the `view_instance_config_state` permission. The response includes:

- `coordinator_state` — one of `idle`, `reloading`, or `degraded`
- `file_digest` — SHA-256 of the last successfully loaded TOML
- `last_reload` — timestamp and summary of the most recent reload attempt
- `recent_events` — ordered list of recent reload lifecycle events

### Dashboard

Navigate to **Settings → Instance Configuration**. The tab shows the config file path, current
coordinator state, and a scrollable list of recent events. A **Clear Degraded** button is visible
when the coordinator is in the `degraded` state (requires `manage_instance_config_state`).

## Failure Matrix

| Phase        | What went wrong                            | Coordinator state           | Recovery                                         |
| ------------ | ------------------------------------------ | --------------------------- | ------------------------------------------------ |
| Validate     | Config file has invalid values             | `idle` (no apply attempted) | Fix TOML; reload again                           |
| Apply        | A subsystem rejected the new config        | `degraded` (all reverted)   | Fix config; `POST clear-degraded`; reload        |
| Health check | A subsystem failed post-apply health check | `degraded` (all reverted)   | Fix the subsystem; `POST clear-degraded`; reload |
| Reexec       | `exec()` failed (missing binary, bad path) | Process dead / crash loop   | Fix binary path; check systemd status            |

## Reexec Semantics

Reexec occurs when a config change includes an irreversibly-bound key. The controller calls `exec()`
to replace the running process image with the updated configuration:

- **Listening sockets are preserved** via `listenfd` / `sd_notify` — no port re-binding.
- **Accepted TCP connections are reset.** Clients reconnect via their existing retry loops.
- The new process image re-reads the updated TOML from disk.

**Irreversibly-bound keys** (additions require an ADR amendment):

- `network.https_addr`
- `network.pki_addr`
- `db.url`
- `tls.trust_domain`

## Recovery from Degraded State

1. Call `GET /api/v1/instance/config-state` and inspect `degraded.reason` and
   `degraded.failed_subsystems`.
2. Fix the underlying issue (bad config value, temporary resource unavailability, etc.).
3. Clear the degraded flag:
   - **Dashboard:** Settings → Instance Configuration → **Clear Degraded** button
     (requires `manage_instance_config_state`).
   - **API:** `POST /api/v1/instance/config-reload/clear-degraded`.
4. Re-trigger a reload (SIGHUP, file-edit, or Dashboard mutation) to apply the corrected config.

## Recovery from a Stuck Reexec Crash Loop

1. Validate the config file without starting the server:

   ```sh
   uptrakit-controller --check-config --config /etc/uptrakit/controller.toml
   ```

2. Check systemd for the exit reason:

   ```sh
   systemctl status uptrakit-controller
   journalctl -u uptrakit-controller -n 50
   ```

3. Revert `controller.toml` to the last known-good version. If the file is tracked in git, use
   `git log -- /etc/uptrakit/controller.toml` to find the previous revision, then
   `git show <hash>:path/to/controller.toml > /etc/uptrakit/controller.toml`.

4. Once the controller starts cleanly, verify state:

   ```http
   GET /api/v1/instance/config-state
   ```

## See Also

- [ADR 0008 — Graceful Reload Architecture](../adr/0008-graceful-reload-architecture.md)
- [CLI Usage Guide](cli-usage.md)
