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
irreversibly-bound key (DB pool URL, master key, log path, embedded-service topology, NATS URL, TLS
trust domain), the controller calls `exec()` to replace the process image instead. Changing the HTTPS
or PKI listener address does **not** reexec — it is rejected at validate; see [Listener Address and
Zeroconf Changes](#listener-address-and-zeroconf-changes).

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

Requires the `system.config-state:read` action. The response includes:

- `coordinator_state` — one of `idle`, `reloading`, or `degraded`
- `file_digest` — SHA-256 of the last successfully loaded TOML
- `last_reload` — timestamp and summary of the most recent reload attempt
- `recent_events` — ordered list of recent reload lifecycle events

### Dashboard

Navigate to **Settings → Instance Configuration**. The tab shows the config file path, current
coordinator state, and a scrollable list of recent events. A **Clear Degraded** button is visible
when the coordinator is in the `degraded` state (requires `system.config-state:manage`).

## Failure Matrix

| Failure                                    | Operator-visible signal                                                                                                                                                                                                                                                                 | Recovery                                                                                                                                |
| ------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| TOML parse error                           | `ConfigReloadFailed { Validate }` audit; `system_alerts` Warning; log line; Dashboard "validation failed" badge on Instance Configuration tab                                                                                                                                           | Edit the TOML; reload (`SIGHUP` or save file).                                                                                          |
| Cross-section invariant fails              | Same as parse error                                                                                                                                                                                                                                                                     | Same.                                                                                                                                   |
| Subsystem `validate()` fails               | Same as parse error, with `subsystem` set                                                                                                                                                                                                                                               | Same.                                                                                                                                   |
| Subsystem `apply()` fails                  | `ConfigReloadFailed { Apply }`, `system_alerts` Error, partial revert audit chain                                                                                                                                                                                                       | Investigate logs; revert TOML or DB if needed.                                                                                          |
| Watchdog timeout / `health_check` fails    | `ConfigReloadFailed { Watchdog }`, revert audit, `system_alerts` Error                                                                                                                                                                                                                  | Investigate; the subsystem is back on old config.                                                                                       |
| `revert()` itself fails                    | Coordinator enters **Degraded** state. Further reloads refused. `system_alerts` Critical. `GET /api/v1/instance/config-state` reports `coordinator_state: degraded` with failing subsystems.                                                                                            | Operator investigates; calls `POST /api/v1/instance/config-reload/clear-degraded` once subsystem health restored, or restarts manually. |
| Reexec validation passes but child crashes | systemd restart loop; binary keeps booting until it succeeds or stays down. **No in-product audit row for the post-exec crash** — the parent that would emit it no longer exists. Operator relies on system logs (`journalctl`) and on the absence of new `ConfigReloadApplied` events. | Fix TOML out-of-band (revert from VCS, edit on another node, etc.); init system picks up working config.                                |
| Concurrent edits (two Operators)           | 409 Conflict on the second writer                                                                                                                                                                                                                                                       | Re-fetch, re-apply, retry.                                                                                                              |

## Reexec Semantics

Reexec occurs when a config change includes an irreversibly-bound key. The controller calls `exec()`
to replace the running process image with the updated configuration:

- **Listening sockets are preserved** via `listenfd` / `sd_notify` — no port re-binding.
- **Accepted TCP connections are reset.** Clients reconnect via their existing retry loops.
- The new process image re-reads the updated TOML from disk.

**Irreversibly-bound keys** (additions require an ADR amendment):

- `db.url`
- `master_key`
- `log.path`
- `embedded_services` topology (enabling/disabling Agent, Agent-SSH, MQTT, or Scheduler)
- `nats.url`
- `tls.trust_domain`

## Listener Address and Zeroconf Changes

The HTTPS listener address (`network.addr`), the PKI advertisement address (`network.pki_addr`), and
`[zeroconf]` config are **not** reexec-forcing and **not** hot-reloadable. A reload attempt that changes
any of them fails loudly at validate:

- `network.addr` change: `"listener address change requires a full controller restart (network https addr)"`
- `network.pki_addr` change: `"listener address change requires a full controller restart (network pki_addr)"`
- `[zeroconf]` change: `"zeroconf config change requires restart"`

Re-binding a listening socket mid-process has no FD-inheritance path in the current reexec
implementation, so these fields are validate-reject gates rather than reexec triggers. To change a
listener address or zeroconf config, edit `controller.toml` and restart the controller (`systemctl
restart uptrakit-controller`, not `reload`).

## TLS Certificate and Key Hot-Reload

`tls.cert_path` and `tls.key_path` are hot-reloadable **via file reload only** — there is no DB-backed
settings route for these fields. On a reload:

1. `validate()` loads the candidate certificate/key pair from disk and confirms the key matches the
   certificate (`keys_match()`) before accepting the change.
2. `apply()` independently re-loads and re-verifies the pair, then atomically swaps it into the served
   TLS resolver — the next handshake serves the new leaf with no restart and no dropped connections.
3. If the post-apply health check fails, the reloadable reverts to the previously served leaf.

`tls.trust_domain` is excluded from this hot-swap path — see the reexec list above.

## Recovery from Degraded State

1. Call `GET /api/v1/instance/config-state` and inspect `degraded.reason` and
   `degraded.failed_subsystems`.
2. Fix the underlying issue (bad config value, temporary resource unavailability, etc.).
3. Clear the degraded flag:
   - **Dashboard:** Settings → Instance Configuration → **Clear Degraded** button
     (requires `system.config-state:manage`).
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
