# Settings Runtime Architecture

Settings are stored in the database and reconciled with CLI flags during startup.

## Reconciliation Priority

1. DB value + CLI provided (different) + `--force-settings-override`: CLI wins, DB updated.
1. DB value + CLI provided (different) + no force: DB wins, warning logged.
1. DB value + CLI absent or same: DB value used.
1. No DB value + CLI provided: CLI value saved to DB.
1. No DB value + CLI absent: default saved to DB.

## Settings Categories

| Category | Key Prefix | API | Runtime Change |
| --- | --- | --- | --- |
| Network | `network.*` | `/settings/network` | Mostly runtime-changeable (some bind addresses need restart). |
| MQTT | `mqtt_*` table | `/settings/mqtt` | Runtime-changeable; controller pushes via WebSocket. |
| Registration | `registration.*` | `/settings/registration` | Runtime-changeable. |
| Authentication | `authentication.*` | `/settings/authentication` | Runtime-changeable. |
| Service Certificates | `service_certificates.*` | `/settings/service-certificates` | Runtime-changeable. |
| SMTP | `smtp.*` | `/settings/smtp` | Runtime-changeable; read by dispatcher on each delivery. |
| NATS | `nats.url` | `/settings/nats` | Stored in DB; **requires restart** to change the live connection. |

Not DB-managed: `--data-dir`, `--db-url`, `--tls-cert`, `--tls-key`, `--ca-cert`, `--ca-key`, `--static-dir`, `--oidc-*` bootstrap flags.

## Watch Channels

- `SettingsSnapshot` is published via `tokio::sync::watch`. Readers use synchronous getters (e.g., `settings.registration()`).
- Writers acquire a `Mutex`, modify snapshot, and call `send_modify()` for atomic replacements.
- Version counters (`version`, `global_version`) use `Ordering::Acquire/Release` for cross-instance invalidation.
- Controllers poll `settings_version` table every 30s and reload only when counters differ.

## Security Notes

For security-sensitive changes to settings, consult [docs/security/secure-development.md](../security/secure-development.md) and ensure permission
checks guard the update endpoints.

## DB-Managed Settings - Detailed

Most CLI arguments are reconciled with DB-persisted values at startup. The reconciliation module
(`crates/core/controller/src/reconcile.rs`) implements a generic 5-case priority logic. Settings are stored in the
`setting` DB entity as JSON values.

### Settings reference

| CLI flag | DB key | Default | Runtime-changeable |
| --- | --- | --- | --- |
| `--trusted-proxy` | `network.trusted_proxies` | `[]` | Yes |
| `--real-ip-header` | `network.real_ip_header` | `X-Forwarded-For` | Yes |
| `--san` | `network.extra_sans` | `[]` | Yes |
| `--forwarded-client-cert-info-header` | `network.forwarded_client_cert_info_header` | `null` | Yes |
| `--forwarded-client-cert-pem-header` | `network.forwarded_client_cert_pem_header` | `null` | Yes |
| `--pki-addr` | `network.pki_addr` | `null` | Yes (requires CA rotation) |
| `--https-addr` | `network.https_addr` | `[::]:8443` | No (restart) |

**Not DB-managed** (bootstrap/infrastructure): `--data-dir`, `--db-url`, `--tls-cert`, `--tls-key`, `--ca-cert`,
`--ca-key`, `--static-dir`, `--reuseport`, `--takeover-from`, `--shutdown-timeout-secs`, `--master-key-file`.

### OIDC provider bootstrap

The controller supports bootstrapping an OIDC provider at startup via CLI flags. This solves the chicken-and-egg problem
where configuring OIDC requires ManageSettings permission, but the first user needs to log in via OIDC.

| CLI flag | Default | Description |
| --- | --- | --- |
| `--oidc-issuer-url` | — | OIDC issuer URL; triggers bootstrap when set |
| `--oidc-client-id` | — | Required with `--oidc-issuer-url` |
| `--oidc-client-secret` | — | Required with `--oidc-issuer-url` |
| `--oidc-provider-name` | `SSO` | Display name for the provider |
| `--oidc-provider-slug` | `sso` | URL-safe slug (used for uniqueness check) |
| `--oidc-scopes` | `openid email profile groups` | Space-separated scopes |

**Bootstrap behavior:**

1. If no provider with matching `(slug, tenant_id)` exists: INSERT with `auto_create_users=true`
1. If a match exists and `--force-settings-override` is set: UPDATE issuer/client_id/client_secret
1. If a match exists without force: skip with info log

The client secret is never logged. The bootstrapped provider is created with `is_active=true` and
`auto_create_users=true`.

When the first user logs in via OIDC (bootstrapped or otherwise), they are automatically promoted to the `owner` role
and initial setup is completed (registration mode set to closed).

### OIDC registration token enforcement

When registration mode is `Invite`, OIDC-based user creation can require a registration token:

- **First user (initial setup)**: Always requires the registration token printed in controller startup logs.
- **Subsequent users**: Requires the token only if `require_token_for_oidc` is enabled in registration settings.

The `needs_token_for_oidc(is_first_user: bool) -> bool` helper on `RegistrationSettings` encapsulates this logic:
returns `true` when mode is `Invite` AND (`is_first_user` OR `require_token_for_oidc`).

**Deferred-action flow** (follows the `AccountLinkStore` pattern):

1. OIDC callback detects a new user would be created AND `needs_token_for_oidc()` returns `true`.
1. OIDC claims are stored in `pending_oidc_registrations` via `OidcRegistrationStore` (10-minute TTL).
1. User is redirected to `/login?registration_token_required=true&registration_code={code}`.
1. Frontend shows a token input form.
1. User submits the token to `POST /api/v1/auth/oidc/complete-registration`.
1. Backend peeks at the pending registration via `get()` (non-destructive), validates the token. If the token is
   invalid, the entry remains in the store so the user can retry with a correct token.
1. On successful validation, the entry is atomically consumed via `take()`, the user is created, roles assigned, and
   session + JWT issued.

**New endpoint:**

| Endpoint | Auth | Purpose |
| --- | --- | --- |
| `POST /api/v1/auth/oidc/complete-registration` | Public | Complete OIDC registration with registration token |

**New setting:**

| DB key | Type | Default | Description |
| --- | --- | --- | --- |
| `registration.require_token_for_oidc` | bool | `false` | When `true` and mode is `Invite`, require registration token for OIDC user creation |

**New store:** `OidcRegistrationStore` in `crates/ui/web-api-auth/src/auth/oidc_state.rs` — follows the same atomic-delete
pattern as `AccountLinkStore`. Methods: `insert()`, `get()` (non-destructive read for pre-validation), `take()` (atomic
consume), `cleanup_expired()`.

**New entity:** `pending_oidc_registration` in `crates/shared/db/src/entity/pending_oidc_registration.rs` — stores
deferred OIDC registration claims (registration_code PK, provider_id, oidc_subject, email, names, mapped_roles JSON,
expires_at).

### Two-table design: global_settings and settings

Settings are stored in two separate tables:

| Table | PK | Purpose |
| --- | --- | --- |
| `global_settings` | `key` | System-wide settings (no `tenant_id`). 13 keys: network, PKI, MQTT limit, multi-tenancy, JWT signing key, master key verification. |
| `settings` | `(tenant_id, key)` | Per-tenant settings (registration, authentication, service certificates, SMTP, etc.). |

The `global_settings` table was introduced to cleanly separate system-wide configuration from
per-tenant data, avoiding the previous design where global settings were stored under the default
tenant's ID in the `settings` table.

### Bulk loading and known-keys registry

At startup, `Settings::load(db, tenant_id)` issues two bulk queries:

1. `load_all_global_settings(db)` — all rows from `global_settings`
2. `load_all_settings(db, tenant_id)` — all rows from `settings` for the given tenant

The two `RawSettings` maps (`HashMap<String, serde_json::Value>`) are merged (global first, then
tenant) and distributed to all sub-loaders. After the bulk load, `warn_unrecognised_keys()` logs a
warning for any DB key not recognised by `SettingKey::from_db_key()`. The `SettingKey` enum
(defined in `crates/ui/web-api-auth/src/setting_key.rs`) is the single source of truth for all known
setting keys. In tests, `SettingKey::iter()` (via `strum::EnumIter`) provides iteration over every
variant.

`Settings::load()` returns `(Self, RawSettings, RawSettings, Option<String>)` — the global raw
map, tenant raw map, and optional registration token — so the controller passes the global map to
reconciliation without re-reading.

The `RawSettingsExt` trait (defined in `settings_store.rs`) provides a `get_setting(SettingKey) -> Option<&Value>`
method for typed lookups on `RawSettings`, replacing raw `raw.get("string.key")` calls throughout the codebase.

### Reconciliation logic

`reconcile_setting()` (`crates/core/controller/src/reconcile.rs`) accepts a `SettingKey` and a `&RawSettings` map
(global settings), looking up the key via `key.as_str()` — no per-key DB reads. It writes to the
`global_settings` table via `upsert_global_setting()`. All reconciled settings are global (no `tenant_id`).

For each DB-managed setting at startup:

1. DB has value + CLI provided + differs + `--force-settings-override` → use CLI, update DB
1. DB has value + CLI provided + differs + no force → use DB, log warning
1. DB has value + (CLI absent or same) → use DB
1. No DB value + CLI provided → use CLI, save to DB
1. No DB value + CLI absent → use hardcoded default, save to DB

### In-memory settings

The `Settings` struct (`crates/ui/web-api/src/settings.rs`) holds `NetworkSettings` behind a `RwLock`.
Runtime-changeable fields (proxies, header, SANs) are updated in-memory immediately when changed via the API.
Restart-required fields (addresses) are saved to DB only.

#### Cross-instance cache synchronisation

In multi-instance deployments (multiple controllers sharing one DB behind a load balancer), the in-memory settings cache
is invalidated cross-instance via a **version-gated periodic reload**. The `settings_version` table stores per-tenant
rows with two version counters:

| Column | Type | Purpose |
| --- | --- | --- |
| `tenant_id` | UUID PK (FK → tenants) | Tenant identifier |
| `version` | BIGINT | Per-tenant settings version (bumped on per-tenant setting changes) |
| `global_version` | BIGINT | Global settings version (bumped on ALL rows when a global setting changes) |
| `revocation_version` | BIGINT | Revocation version (bumped on every certificate revocation for cross-instance CRL propagation) |
| `updated_at` | TIMESTAMP | Last update timestamp |

**Write semantics:** Per-tenant writes (`upsert_setting()`, `delete_setting()`) call
`bump_settings_version(db, tenant_id)` — incrementing only the tenant's `version` counter. Global
writes (`upsert_global_setting()`, `delete_global_setting()`) call
`bump_global_settings_version(db)` — incrementing `global_version` on ALL tenant rows. Certificate
revocation sites call `bump_revocation_version()` before the local `Notify`.

**Read semantics:** A background task (every 30s) calls `Settings::check_version_and_reload()`, which reads a single row
and compares both counters with cached `AtomicI64` values. A full reload (`reload_from_db()`) only happens when either
version differs. The CRL manager polls `revocation_version` every 60s to detect cross-instance revocations (see below).

#### Cross-instance CRL propagation

The `CrlManager` uses a version-gated 60-second poll on `revocation_version` to detect certificate revocations made by
other controller instances. Each revocation site bumps `revocation_version` in the database and fires the local
`Notify`. The CRL manager:

- **Local revocation:** Instant rebuild via `revocation_notify` + optimistic cached version bump.
- **Cross-instance revocation:** Detected within 60s via the version poll. If the DB version differs from the cached
  version, CRL is rebuilt.
- **Fallback:** If the version check fails, the CRL rebuild is forced (fail-safe).

**Key files:**

| File | Purpose |
| --- | --- |
| `crates/shared/db/src/entity/global_setting.rs` | SeaORM entity for `global_settings` table |
| `crates/shared/db/src/entity/settings_version.rs` | SeaORM entity for version tracking |
| `crates/shared/db/src/migration/m20260209_000001_initial.rs` | Single consolidated migration (includes settings_version + revocation_version) |
| `crates/shared/db/src/migration/m20260303_000001_global_settings.rs` | Migration: create `global_settings` table, move 13 keys from `settings` |
| `crates/ui/web-api-auth/src/settings_store.rs` | `upsert_global_setting()`, `load_global_setting()`, `bump_settings_version()`, `bump_global_settings_version()`, `get_settings_versions()`, `bump_revocation_version()`, `get_revocation_version()` |
| `crates/ui/web-api/src/settings.rs` | `reload_from_db()`, `check_version_and_reload()` |
| `crates/core/controller/src/crl_manager.rs` | Version-gated CRL rebuild loop |

### Settings API endpoints

| Endpoint | Permission | Purpose |
| --- | --- | --- |
| `GET /api/v1/settings/network` | ManageGlobalSettings | Read network settings |
| `PUT /api/v1/settings/network` | ManageGlobalSettings | Update network settings (includes `pki_addr`) |
| `GET /api/v1/settings/mqtt` | ViewSettings | List all MQTT client configurations |
| `POST /api/v1/settings/mqtt` | ManageSettings | Create MQTT client configuration (checks per-tenant limit) |
| `GET /api/v1/settings/mqtt/limit` | ViewSettings | Get max MQTT clients per tenant limit |
| `PUT /api/v1/settings/mqtt/limit` | ManageGlobalSettings | Update max MQTT clients per tenant limit |
| `GET /api/v1/settings/mqtt/{id}` | ViewSettings | Get a specific MQTT client configuration |
| `PUT /api/v1/settings/mqtt/{id}` | ManageSettings | Update MQTT client configuration |
| `DELETE /api/v1/settings/mqtt/{id}` | ManageSettings | Delete MQTT client configuration |
| `GET /api/v1/settings/smtp` | ViewSettings | Read global SMTP settings |
| `PUT /api/v1/settings/smtp` | ManageSettings | Update global SMTP settings |
| `POST /api/v1/settings/rotate-ca` | ManageGlobalSettings | Trigger immediate CA rotation |
| `POST /api/v1/settings/renew-server-certificate` | ManageGlobalSettings | Renew server TLS certificate |
| `GET /api/v1/system/alerts` | ManageGlobalSettings | Get system alerts (CA/cert status) |

### PKI API endpoints (unauthenticated)

| Endpoint | Purpose |
| --- | --- |
| `GET /api/v1/pki/ca.crt` | Download CA certificate bundle |
| `GET /api/v1/pki/ca.crl` | Download CRL (combined PEM) |
| `POST /api/v1/pki/ocsp` | OCSP responder (RFC 6960, `application/ocsp-request` body) |
| `GET /api/v1/pki/ocsp/{encoded}` | OCSP responder (base64-encoded request in URL path) |

MQTT password is never exposed in API responses; a `has_password: bool` field indicates whether one is set.

### MQTT client configuration

MQTT settings are stored in a dedicated `mqtt_clients` table (multiple rows per tenant, up to `MqttMaxClientsPerTenant`
limit) rather than in the key-value `settings` table. The table stores connection components; the URL is a computed
presentation field.

**Table schema (`mqtt_clients`):**

| Column | Type | Default | Notes |
| --- | --- | --- | --- |
| `id` | UUID PK | `Uuid::now_v7()` | |
| `tenant_id` | UUID FK → tenants | | Non-unique index (multiple clients per tenant) |
| `enabled` | bool | `true` | |
| `transport` | text | `tcp` | `tcp`, `tls` |
| `host` | text | | Broker hostname |
| `port` | integer | 1883 | |
| `client_id` | text | `uptrakit-controller` | |
| `username` | text? | | |
| `password` | text? | | |
| `topic_prefix` | text | `uptrakit` | |
| `created_at` | timestamptz | | |
| `updated_at` | timestamptz | | |

**Table schema (`mqtt_leases`):**

| Column | Type | Notes |
| --- | --- | --- |
| `id` | UUID PK | `Uuid::now_v7()` |
| `tenant_id` | UUID FK → tenants | |
| `mqtt_client_id` | UUID FK → mqtt_clients (ON DELETE CASCADE) | UNIQUE (one lease per MQTT client config) |
| `instance_id` | text | MQTT service instance identifier |
| `acquired_at` | timestamptz | |
| `last_heartbeat_at` | timestamptz | |

**Global setting (`MqttMaxClientsPerTenant`):**

| DB key | Type | Default | Description |
| --- | --- | --- | --- |
| `mqtt.max_clients_per_tenant` | u16 | 10 | Maximum number of MQTT client configurations per tenant |

**MQTT URL scheme:**

| URL example | Transport | Default port |
| --- | --- | --- |
| `mqtt://broker:1883` | tcp | 1883 |
| `mqtts://broker:8883` | tls | 8883 |

The API accepts either a `url` field (parsed into components) or individual `transport`/`host`/`port` fields. The
response always includes the computed `url`.

### SMTP settings

Global SMTP settings control how the email notification channel connects to an outgoing mail server. These
settings are shared across all email notification channels for the tenant. Per-channel config stores only
recipient addresses (see [Notifications Development](../development/notifications.md)).

**Settings stored in the `settings` key-value table:**

| DB key | Type | Default | Description |
| --- | --- | --- | --- |
| `smtp.host` | string? | `null` | SMTP server hostname |
| `smtp.port` | u16? | `null` (effective default: 587) | SMTP server port |
| `smtp.username` | string? | `null` | SMTP auth username |
| `smtp.password` | string? | `null` (encrypted) | SMTP auth password (AES-256-GCM) |
| `smtp.from_address` | string? | `null` | Sender email address |
| `smtp.from_name` | string? | `null` | Sender display name |
| `smtp.tls_mode` | string | `"starttls"` | TLS mode: `starttls`, `tls`, or `none` |

Email delivery is disabled (notifications skipped with a warning) until both `smtp.host` and
`smtp.from_address` are configured (`SmtpSettingsSnapshot::is_configured()`).

**`GET /api/v1/settings/smtp` — Response (`SmtpSettingsResponse`):**

```json
{
  "host": "smtp.example.com",
  "port": 587,
  "username": "sender@example.com",
  "has_password": true,
  "from_address": "noreply@example.com",
  "from_name": "Uptrakit",
  "tls_mode": "starttls"
}
```

`has_password: true` indicates a password is stored. The actual password is never returned.

**`PUT /api/v1/settings/smtp` — Request (`UpdateSmtpSettingsRequest`):**

All fields are optional. Omitting a field leaves the current value unchanged. Explicitly setting a nullable
field to `null` clears the stored value.

```json
{
  "host": "smtp.example.com",
  "port": 587,
  "username": "sender@example.com",
  "password": "secret",
  "from_address": "noreply@example.com",
  "from_name": "Uptrakit",
  "tls_mode": "starttls"
}
```

To clear the password: `"password": null`. To clear the username: `"username": null`.

**Validation rules:**

- `host`: non-empty string if provided
- `port`: 1–65535 if provided
- `from_address`: must contain `@` if provided
- `tls_mode`: must be `"starttls"`, `"tls"`, or `"none"` if provided

**Security:** The password is encrypted with `uptrakit_crypto::encrypt_str` (AES-256-GCM) before storage.
See [Notifications Security — Email Channel Security](../security/notifications-security.md#email-channel-security).

**CLI:**

```bash
uptrakit settings smtp show
uptrakit settings smtp set --host smtp.example.com --port 587 --from-address noreply@example.com --tls-mode starttls
uptrakit settings smtp set --password mysecret
uptrakit settings smtp set --clear-password
```

### NATS settings (feature: `nats`)

The `nats.url` global setting stores the NATS server URL used by the controller for cross-instance messaging.
This setting is only applicable when the controller is compiled with the `nats` feature.

**Settings stored in the `settings` key-value table (global):**

| DB key | Type | Default | Description |
| --- | --- | --- | --- |
| `nats.url` | string? | `null` | NATS server URL (encrypted with AES-256-GCM). |

The stored URL is encrypted at rest using `uptrakit_crypto::encrypt_str` and decrypted at startup during reconciliation.

**Startup reconciliation with `--nats-url` CLI flag:**

The `nats.url` DB setting is reconciled with the `--nats-url` CLI flag using the standard 5-case priority:

1. DB value + CLI provided (different) + `--force-settings-override`: CLI wins, DB updated and re-encrypted.
1. DB value + CLI provided (different) + no force: DB wins, warning logged. CLI flag is ignored.
1. DB value + CLI absent or same: DB value used as-is.
1. No DB value + CLI provided: CLI value encrypted and saved to DB.
1. No DB value + CLI absent: no URL stored; NATS transport is disabled.

After reconciliation the `ReconciledSettings.nats_url` field holds the raw (decrypted) URL, which is used to
initialise the NATS transport and to populate the `SettingsSnapshot.nats_url` field as a `MaskedUrl` (password
redacted in all serialized/logged output).

**Hot-reload:** Changing the NATS URL via the API updates the DB and the in-memory `SettingsSnapshot`, but does
**not** replace the live NATS connection. The change takes effect on the next controller restart.

**`GET /api/v1/settings/nats` — Response (`NatsSettingsResponse`):**

```json
{
  "url": "nats://user:***@nats.internal:4222",
  "has_url": true
}
```

`url` shows the password component replaced with `***` (via `MaskedUrl`). `has_url: true` indicates a URL is
configured. `url` is omitted from the response when `has_url: false`.

**`PUT /api/v1/settings/nats` — Request (`UpdateNatsSettingsRequest`):**

```json
{ "url": "nats://user:password@nats.internal:4222" }
```

- Omit `url` (or omit the field entirely) to leave the stored value unchanged.
- Set `url` to `null` to clear the stored URL.
- The URL is validated: must start with `nats://` or `nats-tls://`, must not be empty, max 1024 chars.

**Validation rules:**

- `url`: must start with `nats://` or `nats-tls://` if non-null
- Maximum length: 1024 characters

**Security:** The URL may contain embedded credentials (`nats://user:password@host`). It is stored encrypted
at rest with AES-256-GCM and never returned in plaintext via the API — only the masked form is returned.
See [Security — Credential Storage](../security/secure-development.md).

**CLI:**

```bash
uptrakit settings nats get
uptrakit settings nats set --url nats://user:password@host:4222
uptrakit settings nats clear
```

After `set` or `clear`, the CLI prints:

```text
NATS URL updated. The change will take effect after the controller is restarted.
```

## MQTT Service (Standalone Binary)

MQTT is handled by a separate `uptrakit-mqtt` binary (`crates/core/mqtt/`) that connects to the controller via mTLS
WebSocket. The controller pushes tenant assignments and configuration updates; the MQTT service no longer has direct
database access. Multiple instances can run simultaneously with centralized lease coordination managed by the
controller.

**CLI flags (`uptrakit-mqtt`):**

The agent and MQTT service share a common set of CLI flags via `CommonServiceArgs` (defined in
`uptrakit-service-sdk::cli`). Service-specific flags are listed separately.

**Common flags (shared with agent via `CommonServiceArgs`):**

| Flag | Env var | Default | Description |
| --- | --- | --- | --- |
| `--version` | | `false` | Print crate version and build metadata (enabled features, target/cfg/profile) and exit. |
| `--url` | | required for daemon mode | Controller URL (e.g., `https://controller:8443`). Port defaults to 443. Not required for local-only subcommands (e.g., `uptrakit-agent-ssh host`). |
| `--tofu` | | `false` | Trust the controller's TLS certificate on first connection (TOFU) with signature verification via `TofuVerifier`. Conflicts with `--ca-cert` and `--pki-addr`. |
| `--tofu-fingerprint` | | | SHA-256 fingerprint for TOFU verification (hex-encoded, with or without colons). Requires `--tofu`. When set, the fetched CA certificate's fingerprint is compared against this value before trusting it. |
| `--ca-cert` | | | Path to a PEM-encoded CA certificate file |
| `--pki-addr` | | | Optional URL for PKI endpoints (CA certificate, OCSP). Supports `http://` and `https://`. |
| `--config-dir` | `UPTRAKIT_CONFIG_DIR` | platform-specific | Config directory for CA certificate |
| `--state-dir` | `UPTRAKIT_STATE_DIR` | platform-specific | State directory for service identity (service_id, keypair, certificate) |
| `--friendly-name` | | hostname | Human-readable display name |
| `--enrollment-token` | `UPTRAKIT_ENROLLMENT_TOKEN` | | Enrollment token for auto-approval |
| `--force-enroll` | | `false` | Force fresh enrollment, discarding existing state (preserves cached CA certificate) |

**MQTT-specific flags:**

| Flag | Default | Description |
| --- | --- | --- |
| `--max-tenants` | `0` | Max tenants per instance (0 = unlimited) |
| `--ping-interval` | `15` | Ping interval in seconds |

**Connection lifecycle (shared with agent via `uptrakit-service-sdk`):**

1. **CA bootstrap**: Cached CA → `--ca-cert` file → `--pki-addr` fetch → `--tofu` TOFU (with optional
   `--tofu-fingerprint` SHA-256 pinning) → system trust (via `uptrakit_service_sdk::ca::bootstrap_ca`)
1. **Enrollment**: Connect to `/api/v1/ws/service` anonymously, send `Enroll` with
   capabilities/hostname/friendly_name/enrollment_token, receive `Enrolled` with service_id and
   enrollment_secret (saved to state dir) (via `uptrakit_service_sdk::ws::run_enrollment`)
1. **Certificate issuance**: Reconnect with bearer token (enrollment_secret), send CSR, receive signed certificate
   (saved to state dir) (via `uptrakit_service_sdk::ws::resume_enrollment`)
1. **Authenticated operation**: Reconnect with mTLS using shared `ControllerConnection`, send `Register`,
   receive `TenantAssignments`, run MQTT clients. Automatic reconnection with exponential backoff on disconnect,
   and certificate rotation handling (matching agent behavior).

**Instance identification:**

- Each instance generates a unique ID: `{hostname}-{uuid_v7_first_8_chars}`
- The controller manages leases centrally via the `mqtt_leases` table

**Main loop (event-driven, not polling):**

1. Receive `TenantAssignments` → start/update MQTT clients (keyed by `mqtt_client_id`)
1. Receive `TenantConfigUpdated` → hot-reload MQTT client configuration
1. Receive `TenantRevoked` → stop MQTT client (by `mqtt_client_id`)
1. Receive `CaBundleUpdated` → update local CA certificate
1. Receive `RequestCertRenewal` → trigger certificate renewal
1. Receive `ServerRestarting` → prepare for reconnect
1. Send `Ping` periodically (controller uses Ping receipt to update lease heartbeats)

**Wire protocol:**

Agents and MQTT services share a unified wire protocol (`ServiceMessage` / `ControllerMessage`) defined in
`crates/shared/wire/src/messages.rs` (re-exported via `lib.rs`). `ServiceMessage` contains both agent-specific variants (`ReportHosts`,
`VersionCheckResults`, `UpdateStarted`, `UpdateOutput`, `UpdateResult`) and MQTT-specific variants (`Register`,
`ReleaseTenants`), plus shared variants (`Enroll`, `RequestCertificate`, `RenewCertificate`, `Ping`, `Disconnecting`).
`ControllerMessage` is fully shared. The `service_ws/` directory module is the public WebSocket entry point (`service_ws/mod.rs`
handles routing, dispatch, and connection setup; `service_ws/protocol.rs` provides shared
serialization, rate limiting, and utility types; `service_ws/connection.rs` manages
authenticated/enrolled/anonymous connection paths). The `service_ws/handler/` submodule is the
`pub(crate)` internal implementation containing capability-gated message dispatch for all
service types, with handlers organized by concern (`handler/messages.rs`, `handler/mqtt.rs`,
`handler/updates.rs`, `handler/renewal.rs`, `handler/discovery.rs`).

**Enrollment and approval:**

MQTT services use the unified service entity:

- Single `services` table with `capabilities` column (JSON array of capability strings)
- Single `service_certificates` table for all service types
- A single enrollment token is shared by all service types (DB key `service_enrollment.token_hash`
  via `SettingKey::EnrollmentTokenHash`)
- Approval via unified REST API: `POST /api/v1/services/{id}/approve` (permission: `ManageAgents`)
- If a valid enrollment token is provided, the service is auto-approved

**REST API endpoints (unified services API):**

| Method | Path | Permission | Description |
| --- | --- | --- | --- |
| GET | `/api/v1/services?capability=mqtt_bridge&status=...` | ViewAgents | List MQTT services (filter by capability) |
| POST | `/api/v1/services/{id}/approve` | ManageAgents | Approve a pending service |
| POST | `/api/v1/services/{id}/reject` | ManageAgents | Reject a pending service |
| DELETE | `/api/v1/services/{id}` | ManageAgents | Deactivate a service |
| POST | `/api/v1/services/enrollment-token` | ManageAgents | Create enrollment token (shared by all service types) |
| DELETE | `/api/v1/services/enrollment-token` | ManageAgents | Revoke enrollment token |
| GET | `/api/v1/services/enrollment-token/status` | ManageAgents | Check enrollment token status |

**Key files:**

| File | Purpose |
| --- | --- |
| `crates/shared/web-api-types/src/mqtt_transport.rs` | `MqttTransport` enum (Tcp/Tls) |
| `crates/shared/web-api-types/src/mqtt_url.rs` | `MqttUrl` parsing and formatting |
| `crates/shared/web-api-types/src/settings_mqtt.rs` | API request/response types |
| `crates/shared/wire/src/messages.rs` | Unified wire protocol messages (`ServiceMessage` / `ControllerMessage`) |
| `crates/shared/db/src/entity/service.rs` | SeaORM entity for service identity (agents and MQTT) |
| `crates/shared/db/src/entity/service_certificate.rs` | SeaORM entity for service certificates |
| `crates/shared/db/src/entity/mqtt_client.rs` | SeaORM entity for MQTT config |
| `crates/shared/db/src/entity/mqtt_lease.rs` | SeaORM entity for leases (managed by controller) |
| `crates/shared/service-sdk/` | `uptrakit-service-sdk` crate: shared service SDK — enrollment, identity, TLS (`TofuVerifier`), CA bootstrap (with `ca_pem_fingerprint()` and `--tofu-fingerprint` pinning), WebSocket protocol, `ControllerConnection`, `Backoff`, and CLI args |
| `crates/ui/web-api/src/mqtt_client_store.rs` | MQTT client config CRUD store |
| `crates/ui/web-api/src/service_connections.rs` | `ServiceConnectionRegistry` for all connected services (with `CancellationToken`-based connection deduplication) |
| `crates/ui/web-api/src/notification_service.rs` | `NotificationService` — local delivery + optional NATS cross-controller publish |
| `crates/ui/web-api/src/nats_transport.rs` | `NatsTransport` — NATS JetStream transport (feature-gated) |
| `crates/ui/web-api/src/event_delivery.rs` | Shared delivery routing logic |
| `crates/ui/web-api/src/mqtt_lease_coordinator.rs` | Centralized lease management logic |
| `crates/ui/web-api/src/routes/settings_mqtt.rs` | MQTT config API route handlers |
| `crates/ui/web-api/src/routes/service_ws/mod.rs` | Unified WebSocket entry point (`/api/v1/ws/service`), dispatch, and connection setup |
| `crates/ui/web-api/src/routes/service_ws/protocol.rs` | Shared WS protocol utilities (serialization, rate limiting, types) |
| `crates/ui/web-api/src/routes/service_ws/connection.rs` | Connection path handlers (authenticated/enrolled/anonymous) |
| `crates/ui/web-api/src/routes/service_ws/handler/` | Unified capability-gated WebSocket handler (`pub(crate)`) — organized by concern |
| `crates/ui/web-api/src/routes/services.rs` | Unified service management REST endpoints |
| `crates/core/mqtt/src/main.rs` | Entry point, enrollment flow, authenticated main loop |
| `crates/core/mqtt/src/cli.rs` | CLI argument definitions |
| `crates/shared/service-sdk/src/connection.rs` | Shared `ControllerConnection` — authenticated WebSocket client for controller communication (used by both agent and MQTT) |
| `crates/core/mqtt/src/tenant_manager.rs` | Per-MQTT-client lifecycle management (push-based, keyed by `mqtt_client_id`) |
| `crates/core/mqtt/src/mqtt_client.rs` | MQTT broker connection logic |
| `crates/core/mqtt/src/error.rs` | Application error types |
