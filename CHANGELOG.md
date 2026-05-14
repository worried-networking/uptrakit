# Changelog

All notable changes to this project will be documented in this file.

## Unreleased

### Breaking Changes — Graceful Reload

- CLI surface shrunk to `--config`, `--master-key-from`, `--migrate-and-exit`, `--check-config`,
  `--version`, `--verbose`. All other flags are removed without alias. Operators must produce a TOML
  file (`controller.toml`) before upgrading; see the
  [operator runbook](docs/end-user/operator-runbook-reload.md).
- `--reuseport` / `--takeover-from` / `SIGUSR1` graceful-restart path is removed. Reexec-style
  reload via SIGHUP + TOML edit replaces routine restarts; external load-balancing across two
  controllers covers accepted-connection preservation if required.
- `spawn_settings_reload` 30 s poll task is replaced by the 2 s `ConfigReconciler` task.
- The following `SettingKey` rows are dropped from `global_settings` at boot via migration
  `m20260512_000001_drop_file_keys`: HTTPS / PKI listen addrs, trusted proxies, real-IP / forwarded
  headers, zeroconf, NATS URL, global audit-log filter, global audit-log retention. Per-tenant
  audit-log rows are untouched.
- Reexec via `exec()` preserves listening sockets but resets accepted TCP connections — clients
  reconnect via their existing retry loops.
- All settings mutation endpoints now require `If-Match` (428 on missing, 409 on stale).

### Added

- `GET /api/v1/instance/config-state` (requires `view_instance_config_state`).
- `POST /api/v1/instance/config-reload/clear-degraded` (requires `manage_instance_config_state`).
- "Instance Configuration" tab under Settings (requires `view_instance_config_state`).
- Graceful config reload: SIGHUP, file-watch, and `ConfigReconciler` (2 s DB poll) all trigger
  atomic in-process reload with watchdog-revert fallback.
- `--check-config` flag to validate the TOML file without starting the server.
