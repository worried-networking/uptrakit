# Settings Runtime Architecture

Settings are stored in the database and reconciled with CLI flags during startup.

## Reconciliation Priority

1. DB value + CLI provided (different) + `--force-settings-override`: CLI wins, DB updated.
2. DB value + CLI provided (different) + no force: DB wins, warning logged.
3. DB value + CLI absent or same: DB value used.
4. No DB value + CLI provided: CLI value saved to DB.
5. No DB value + CLI absent: default saved to DB.

## Settings Categories

| Category | Key Prefix | API | Runtime Change |
| --- | --- | --- | --- |
| Network | `network.*` | `/settings/network` | Mostly runtime-changeable (some bind addresses need restart). |
| MQTT | `mqtt_*` table | `/settings/mqtt` | Runtime-changeable; controller pushes via WebSocket. |
| Registration | `registration.*` | `/settings/registration` | Runtime-changeable. |
| Authentication | `authentication.*` | `/settings/authentication` | Runtime-changeable. |
| Service Certificates | `service_certificates.*` | `/settings/service-certificates` | Runtime-changeable. |

Not DB-managed: `--data-dir`, `--db-url`, `--tls-cert`, `--tls-key`, `--ca-cert`, `--ca-key`, `--static-dir`, `--oidc-*` bootstrap flags.

## Watch Channels

- `SettingsSnapshot` is published via `tokio::sync::watch`. Readers use synchronous getters (e.g., `settings.registration()`).
- Writers acquire a `Mutex`, modify snapshot, and call `send_modify()` for atomic replacements.
- Version counters (`version`, `global_version`) use `Ordering::Acquire/Release` for cross-instance invalidation.
- Controllers poll `settings_version` table every 30s and reload only when counters differ.

## Security Notes

For security-sensitive changes to settings, consult [docs/security/secure-development.md](../security/secure-development.md) and ensure permission checks guard the update endpoints.
