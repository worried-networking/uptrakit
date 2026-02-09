# Web API Code Review Findings

**Review date:** 2026-02-08
**Branch:** `refactor/codereview-webapi`
**Scope:** `uptrakit-web-api`, `uptrakit-internal-wire`, `uptrakit-web-api-types` — architecture, security & safety, high availability

---

## Summary

| Severity | Count | Fixed |
|----------|-------|-------|
| Critical | 7 | 7 |
| High | 11 | 11 |
| Medium | 17 | 2 |
| Low | 10 | 0 |

---

## CRITICAL

### C1. CA Private Key Material Exposed via `Debug` and Cloned Into Every Handler — FIXED

**Files:** `src/lib.rs:52-76`, `src/ocsp.rs:84-93`, `src/routes/ocsp.rs:43`

`CaSnapshotData` and `TrustedCaSnapshot` derive both `Clone` and `Debug` while containing `key_pem`/`active_key_pem`/`previous_key_pem` fields. Every handler that calls `state.ca_snapshot.borrow().clone()` — including **unauthenticated** OCSP/CA endpoints — creates full heap copies of all CA private keys. Any `tracing::debug!("{:?}", ...)`, panic backtrace, or core dump will dump private keys into logs.

There is also no zeroization of key material on drop (`src/ocsp.rs:368-394`, `src/routes/server_cert.rs:134`). Residual keys persist in freed heap memory.

**Recommendation:** (a) Replace `#[derive(Debug)]` with a manual impl that redacts key fields. (b) Move private keys into a separate non-`Clone` `Arc<RwLock<KeyStore>>` accessed only by the OCSP signer and cert signer. (c) Use `zeroize::Zeroizing<String>` for all key material.

**Fix:** `CaSnapshotData` split into `CaPublicSnapshot` (watch channel, Clone+Debug, no keys) and `CaKeyStore` (`Arc<RwLock>`, non-Clone, custom Debug that redacts all keys). All key material wrapped in `zeroize::Zeroizing<String>`. Only OCSP responder, CRL manager, cert signer, and server cert renewal access the key store. 15 files updated across web-api and controller crates.

### C2. Deactivated Users Can Authenticate via OIDC LinkedUser Path — FIXED

**Files:** `src/auth/authentication.rs:104-113`, `src/routes/oidc_auth.rs:428-439`

`resolve_oidc_user` returns `LinkedUser(user_id)` when an existing OIDC link is found without checking `is_active`. The OIDC callback handler processes `LinkedUser` directly — syncs roles and creates a session — with no deactivation check. The deactivation check at line 124 only triggers in the email-lookup path (step 2).

**Recommendation:** Add `is_active` check before returning `LinkedUser`, or check it in the callback handler before creating a session.

**Fix:** `resolve_oidc_user` now loads the linked user and checks `is_active` before returning `LinkedUser`, returning `Deactivated` for inactive users and falling through to email lookup for orphaned links. Defense-in-depth check also added in the OIDC callback handler's `LinkedUser` arm.

### C3. No Refresh Token Rotation on Use — FIXED

**File:** `src/routes/auth.rs:382-443`

The `refresh` endpoint issues a new access token but does **not** rotate the refresh token. The same refresh token remains valid for its full 7-day lifetime. A stolen refresh token provides persistent access for the entire window.

**Recommendation:** Issue a new refresh token on each refresh, invalidating the old one atomically.

**Fix:** Added `rotate_refresh_token` method to `SessionService` that atomically revokes the old session and creates a new one with a fresh token. The refresh handler now returns both a new access token and the rotated refresh token. Frontend stores the rotated token. 5 new tests cover rotation, revocation, and replay detection.

### C4. Rate Limiter TOCTOU — Check and Increment Are Not Atomic — FIXED

**File:** `src/auth/rate_limit.rs:62-99`

`check_rate_limit` performs a non-atomic read-then-write: SELECT at line 63, check at line 71, UPDATE at line 83. Concurrent requests (same instance or across HA instances) can all read the same `request_count`, pass the check, and increment. The conditional UPDATE does not include `request_count < max_requests` as a WHERE condition. Additionally, the `upsert_new_window` path (line 94-97) resets the counter to 1 when `rows_affected == 0`, providing a second bypass avenue.

**Recommendation:** Replace with a single atomic SQL statement: `INSERT ... ON CONFLICT DO UPDATE SET request_count = CASE WHEN window_start < threshold THEN 1 ELSE request_count + 1 END ... RETURNING request_count`.

**Fix:** Replaced with a single atomic `INSERT ... ON CONFLICT DO UPDATE SET request_count = CASE WHEN ... END` statement using `sea_orm::Statement::from_sql_and_values()`. Supports SQLite, PostgreSQL, and MySQL backends. Removed the separate `upsert_new_window` method. After the atomic upsert, reads back the current count to decide the outcome. Raw SQL is required because SeaORM's `on_conflict` builder doesn't support CASE WHEN expressions; the statement is fully parameterized (no injection risk).

### C5. Shell Injection via Unsanitized Hook Parameters — FIXED

**File:** `src/update_hooks.rs:138-171`

`resolve_systemd_hook` and `resolve_docker_compose_hook` interpolate user-controlled `service_name`, `project_dir`, and `compose_file` directly into shell commands with `format!()`. A `service_name` of `myapp; rm -rf /` produces `systemctl stop myapp; rm -rf /`. Custom commands (`commands` field) also pass through without any validation (line 119-122).

**Recommendation:** (a) Validate predefined hook fields against strict patterns (alphanumeric + hyphen/underscore for service names, validated paths for directories). (b) For custom commands, document the security boundary clearly and consider a separate permission.

**Fix:** Predefined hooks use `HookCommand::Exec` (direct exec, no shell) instead of shell command strings. Input validation at the API boundary via `validate_hooks_in_config()` in `provider_configs.rs` and `software_items.rs` rejects shell metacharacters in service names, paths, and compose files.

### C6. No WebSocket Message Size Limits — FIXED

**File:** `src/routes/service_ws.rs:187`

`WebSocketUpgrade` is called with no configuration — inheriting tungstenite's default 64 MB max message size. Combined with no rate limiting on the WebSocket endpoint, a small number of concurrent large messages can exhaust server memory.

**Recommendation:** Call `.max_message_size(some_reasonable_limit)` on the upgrade (e.g., 256 KB or 1 MB).

**Fix:** `MAX_WS_MESSAGE_SIZE = 1_048_576` (1 MB) applied to `.max_message_size()` on the WebSocket upgrade. The largest legitimate wire message (`ExecuteUpdate` with provider config) is well under 100 KB.

### C7. No Rate Limiting on WebSocket Endpoint + Argon2 Brute-Force Amplification — FIXED

**Files:** `src/routes/service_ws.rs:200-232`, `src/middleware/rate_limit.rs:20-58`

The `/api/v1/ws/service` endpoint is not rate-limited and accepts anonymous connections. The `lookup_by_secret` function (line 200-232) performs an unprotected O(N) argon2 scan against all non-deactivated MQTT services when the fast SHA-256 lookup fails. Each anonymous connection attempt causes significant CPU consumption.

**Recommendation:** Add per-IP connection rate limiting for WebSocket upgrades. For the bearer lookup, index enrollment secrets by SHA-256 prefix to eliminate the O(N) argon2 scan.

**Fix:** Per-IP connection rate limiting added at the top of `service_ws()` (before WS upgrade): `ws_connect:{ip}` key, 30 requests per 60 seconds, fail-closed on DB error. After failed bearer lookup: `ws_auth_fail:{ip}` key, 10 per 300 seconds, also fail-closed. The Argon2 concern is moot — `lookup_by_secret` already uses SHA-256 hashing exclusively.

---

## HIGH

### H1. First-User Registration Race Condition — FIXED

**File:** `src/routes/auth.rs:86-136`

Non-atomic read-then-act: `User::find().count()` -> `insert()` -> `assign_owner_role()`. Two concurrent requests during initial setup can both observe zero users and both get the `owner` role. Same race exists in the OIDC path (`src/routes/oidc_auth.rs:443-447`).

**Recommendation:** Wrap in a serializable transaction or use `INSERT ... WHERE NOT EXISTS`.

**Fix:** Registration wrapped in a database transaction (`auth.rs:78-140`). The user count check and insert are performed atomically within the same transaction, preventing concurrent first-user races.

### H2. JWT Access Tokens Cannot Be Revoked (15-Minute Window) — FIXED

**Files:** `src/middleware/require_auth.rs:126-156`, `src/auth/jwt.rs:109-118`

JWT tokens are validated statelessly — no DB lookup, no revocation check, no `is_active` check. Permissions are baked into the JWT at issuance time and trusted for 15 minutes. After user deactivation, role changes, or logout, access persists.

**Recommendation:** Either add a short-lived token denylist (checked on each request), reduce token lifetime to ~1-2 minutes, or add a DB-backed `is_active` check on critical endpoints.

**Fix:** Added in-memory `TokenDenylist` (`src/auth/token_denylist.rs`) supporting per-JTI and per-user revocation with auto-expiry. The `authenticate_jwt` middleware checks the denylist on every request. On logout, all tokens for the user are denied for the remaining access token lifetime (15 min). Periodic purge task cleans expired entries. Known limitation: denylist is per-instance; cross-instance revocation relies on token expiry (HA DB sync deferred).

### H3. JWT Signing Key Divergence in HA Deployments — FIXED

**File:** `src/auth/jwt.rs:38-73`

Each controller instance generates its own JWT signing key from its local `data_dir`. If instances don't share the same state directory, tokens issued by instance A are rejected by instance B.

**Recommendation:** Document as a deployment requirement: all HA instances must share the same state directory (or use a shared key management system). Consider adding a health check that detects key mismatch.

**Fix:** JWT signing key is now stored in the database settings table (key: `auth.jwt_signing_key`, base64-encoded). All HA instances share the same key via the DB. On startup, the controller migrates any existing file-based key (`jwt_signing.key`) to the DB, then loads or generates the key from the DB with race-safe upsert. The file-based `JwtManager::load_or_generate` is retained for tests only.

### H4. Tenant Isolation Bypass via X-Tenant-Id Header — FIXED

**File:** `src/middleware/tenant_context.rs:29-51`

`TenantContext` reads `X-Tenant-Id` from any request and uses it without authorization checks. Any authenticated user can set `X-Tenant-Id: <any-uuid>` and access another tenant's resources. This affects **all** tenant-scoped endpoints.

**Recommendation:** Either remove the header processing entirely (since multi-tenancy is "future work"), or gate it behind user-tenant authorization.

**Fix:** Removed `X-Tenant-Id` header processing entirely. `TenantContext` always returns `state.default_tenant_id`. Added `x-tenant-id` to `strip_proxy_headers()` as defense-in-depth. TODO comment left for future multi-tenancy requirements (must verify user-tenant access).

### H5. Origin Header Trusted from Untrusted Clients — FIXED

**File:** `src/middleware/resolve_proxy_headers.rs:274-284`

`resolve_external_base_url` gives highest priority to the `Origin` header regardless of trusted-proxy status. An attacker can set `Origin: https://evil.com` and OIDC redirect URLs and device auth verification URLs will point to the attacker's domain. The `strip_proxy_headers` function does NOT strip `Origin`.

**Recommendation:** Only trust `Origin` from trusted proxies, or validate it against a configured allowlist.

**Fix:** `Origin` header is now only used inside the `if from_trusted_proxy` block. Added `origin` to `strip_proxy_headers()` for non-proxy requests. Tests updated: Origin from untrusted clients falls back to Host header; spoofing test confirms `Origin: https://evil.com` is ignored for non-proxy requests.

### H6. Server Private Key Written Without Restricted Permissions + Non-Atomic Write — FIXED

**File:** `src/routes/server_cert.rs:138-147`

`std::fs::write()` creates files with default umask permissions (typically 0644). The `server.key` file is world-readable. The cert and key are written in two separate calls — a crash between them produces a mismatched pair.

**Recommendation:** Use `OpenOptionsExt` with mode `0o600`. Write to temp files then atomically rename.

**Fix:** Uses `uptrakit_directories::write_secure_file_str()` (0o600 permissions) to write temp files, then `std::fs::rename()` for atomic replacement. Key is written to `server.key.tmp` and cert to `server.crt.tmp` before atomic rename to final paths, preventing mismatched cert/key pairs on crash.

### H7. Connection Registry Overwrites on Concurrent Reconnect — FIXED

**File:** `src/service_connections.rs`

When a service reconnects before cleanup, `register_agent`/`register_mqtt` unconditionally overwrite via `HashMap::insert`. The old handler's cleanup then removes the **new** connection's entry, leaving the service unable to receive push notifications.

**Fix:** `register_agent`/`register_mqtt` now return a `CancellationToken` alongside the push-message receiver. On re-registration (same `service_id`), the old connection's token is cancelled, causing the old WebSocket handler to exit immediately via a `cancel_token.cancelled()` branch in `tokio::select!`. The superseded handler closes the WebSocket with reason "superseded by new connection" and does NOT call `unregister`, preserving the new connection's registry entry.

### H8. Settings Reload Torn Reads — 6 Independent RwLocks Updated Sequentially — FIXED

**File:** `src/settings.rs:232-281`

`reload_from_db` acquires and releases 6 independent `RwLock`s sequentially. Concurrent request handlers can observe a mix of old and new settings.

**Recommendation:** Use a single `RwLock<AllSettings>` struct or a `tokio::sync::watch` channel for atomic swaps.

**Fix:** Replaced 6 independent `RwLock`s with a `tokio::sync::watch` channel holding an atomic `SettingsSnapshot`. All reader methods are now synchronous (no `.await`). Writers use a `tokio::sync::Mutex` to serialize modifications and publish via `send_modify()`. `reload_from_db` builds a complete snapshot and publishes atomically. Version counter loads use `Ordering::Acquire` (was `Relaxed`).

### H9. TOCTOU in `upsert_setting` — Concurrent Upserts Can Conflict — FIXED

**File:** `src/settings_store.rs:50-84`

Read-then-insert pattern without transaction. Two concurrent inserts for the same key produce a unique constraint violation. Same pattern exists in `bump_settings_version` and `bump_revocation_version`.

**Recommendation:** Use SeaORM's `on_conflict` (database-level upsert).

**Fix:** Replaced read-then-write in `upsert_setting` with `INSERT ... ON CONFLICT DO UPDATE` via SeaORM's `on_conflict` builder. Fixed defensive inserts in `bump_settings_version` and `bump_revocation_version` to use `on_conflict` with `do_nothing` via `try_insert()`. All upserts are now single atomic SQL statements.

### H10. Update History Operations Not Checked Against Agent Ownership (IDOR) — FIXED

**File:** `src/routes/agent_ws.rs`

`UpdateStarted`, `UpdateOutput`, and `UpdateResult` look up records by `update_history_id` from the message payload without verifying the record belongs to the current agent's host. A compromised agent can tamper with any update record.

**Fix:** Added `validate_update_ownership()` helper that checks the `update_history` record's `host_id` against the agent's `linked_host_ids` set (loaded at connection start, refreshed on `ReportHostInfo`). All three update message handlers now call this validation before processing. Unauthorized access attempts are logged and rejected.

### H11. Unbounded Update Output Accumulation — FIXED

**File:** `src/routes/agent_ws.rs`

Each `UpdateOutput` message appends to the existing output with `format!("{}{}\n", record.output, payload.output)` — loading the entire existing output from DB, appending, and writing back. No size limit. Quadratic memory growth.

**Fix:** Added `MAX_UPDATE_OUTPUT_BYTES` (1 MB) constant. Output accumulation stops once the cap is reached; further `UpdateOutput` messages are silently dropped with a debug log. The `UpdateResult` handler also caps its final output append against the same limit.

---

## MEDIUM

### M1. Rate Limiter Fails Open on DB Errors

**File:** `src/middleware/rate_limit.rs:96-100`

When the DB check fails, requests pass through. An attacker inducing DB pressure disables all rate limiting.

### M2. OIDC AutoLink Bypasses Account Ownership Verification

**File:** `src/auth/authentication.rs:147-159`

When a user has no password and no other OIDC link, `AutoLink` fires without email ownership proof. Risk depends on OIDC provider email verification.

### M3. OIDC Role Sync Non-Atomic (Delete-Then-Insert Without Transaction)

**File:** `src/auth/authentication.rs:264-282`

Crash between delete and insert leaves user with zero roles. Concurrent logins can interfere.

### M4. OCSP 1-Hour Cache Serves Stale Revocation Status

**File:** `src/routes/ocsp.rs:19-26`

`Cache-Control: max-age=3600` means revoked certificates may appear valid for up to 1 hour.

### M5. Missing Rate Limiting on OIDC Exchange/Link/Complete-Registration

**File:** `src/middleware/rate_limit.rs:20-58`

These public, unauthenticated endpoints accept secret tokens but aren't rate-limited. The `complete-registration` endpoint uses non-destructive `get()`, allowing unlimited brute-force of the registration token.

### M6. MQTT Password Stored in Plaintext

**Files:** `src/mqtt_client_store.rs:104`, `src/mqtt_lease_coordinator.rs:491`

Unlike enrollment tokens (hashed), MQTT broker credentials are stored and transmitted in plaintext.

### M7. No Idle Timeout on WebSocket Connections

**Files:** All WS handlers

No `tokio::time::timeout` branch. Half-open TCP connections persist indefinitely.

### M8. Broadcast Holds Read Lock During Async Sends — FIXED

**File:** `src/service_connections.rs`

A slow consumer blocks all registry writes during broadcast.

**Fix:** `send()`, `broadcast()`, and `broadcast_by_type()` now snapshot senders under the lock and release it before performing async sends. This prevents a slow consumer from blocking connection management operations.

### M9. Relaxed Memory Ordering on Settings Version Counters — FIXED (via H8)

**File:** `src/settings.rs:294-295`

`Ordering::Relaxed` loads can return stale values on ARM/multi-core, causing missed reloads. Two separate non-atomic loads can also produce inconsistent version pairs.

**Fix:** Addressed as part of H8. Version counter loads now use `Ordering::Acquire` (matching `Release` stores). The watch channel pattern eliminates inconsistent version pairs.

### M10. `cert_signed_by_ca` Only Compares DN, Not Cryptographic Signature

**File:** `src/pki_utils.rs:84-98`

A certificate with a matching issuer DN but forged signature passes the check.

### M11. No Max Password Length — Argon2 DoS Vector

**File:** `src/routes/auth.rs:48-54`

No upper bound on password length. Multi-MB passwords exhaust resources.

### M12. Email Not Normalized

**File:** `src/routes/auth.rs:39-82`

Case-sensitive email comparison allows duplicate registrations for the same logical email.

### M13. assign_hosts Missing Tenant Check on Host

**File:** `src/routes/software_items.rs:666-669`

Host existence check has no `tenant_id` filter, unlike `trigger_update` which correctly filters.

### M14. MQTT Lease Takeover Without Authorization

**File:** `src/mqtt_lease_coordinator.rs:421-444`

Any MQTT service can claim arbitrary client IDs and steal leases.

### M15. Stale Lease Cleanup Does Not Notify In-Memory Registry

**File:** `src/mqtt_lease_coordinator.rs:323-335`

DB leases are cleaned up but the in-memory registry still holds references, causing dual assignment.

### M16. OCSP Hardcoded to ECDSA P-256

**File:** `src/ocsp.rs:331-335`

If the CA uses RSA or P-384, OCSP signing fails entirely.

### M17. Silent Fallback to Weak UUID on CSPRNG Failure

**File:** `src/routes/oidc_auth.rs:379-380, 515-516, 1159`

`generate_secure_token().unwrap_or_else(|_| generate_uuid())` falls from 256-bit to ~62-bit entropy without any alarm.

---

## LOW

### L1. User Enumeration via Differential Login Responses

**File:** `src/routes/auth.rs:221-223`

"User is deactivated" (403) vs "Invalid credentials" (401) reveals account status.

### L2. Logout Is Unauthenticated

**File:** `src/routes/auth.rs:307-321`

Anyone with a refresh token can revoke it without proving identity.

### L3. Bearer Token Prefix Is Case-Sensitive

**File:** `src/middleware/require_auth.rs:158-165`

`strip_prefix("Bearer ")` rejects `bearer ` per RFC 6750 case-insensitivity.

### L4. Error Messages Contain Trailing Newlines

**File:** `src/middleware/require_auth.rs:100,107,110,133,136`

Inconsistent with rest of codebase.

### L5. API Token `last_used_at` Written on Every Request

**File:** `src/auth/api_token.rs:121-126`

Unnecessary write load under high API token usage.

### L6. Health Endpoint Returns Static "ok"

**File:** `src/routes/health.rs:1-5`

No DB connectivity or CA availability check.

### L7. Device Flow `client_name` Not Length-Validated

**File:** `src/routes/device_auth.rs:35-37`

Unbounded string stored in DB from unauthenticated endpoint.

### L8. MQTT Enrolled Loop Has No Push Channel

**File:** `src/routes/mqtt_ws.rs:353-511`

MQTT service doesn't register in connection registry during pending-approval state, so approval notifications can't be pushed.

### L9. Deactivation Doesn't Update `status` Column

**File:** `src/routes/services.rs:389-391`

`deactivated_at` is set but `status` remains "approved", causing audit trail inconsistency.

### L10. N+1 Queries in List Endpoints

**Files:** `src/routes/update_history.rs:61-70`, `src/routes/hosts.rs:88-91`, `src/routes/software_items.rs:357-373`

Per-record DB lookups for denormalized names.

---

## Positive Findings

- **Password hashing:** Argon2id with OWASP parameters (19 MiB, 2 iterations)
- **Token storage:** All tokens stored as SHA-256 hashes, never plaintext
- **OIDC security:** PKCE with S256, CSRF state tokens, nonce validation, database-backed stores (HA-safe)
- **Sequence number validation:** Strict monotonic replay protection on the wire protocol
- **Proxy header stripping:** `X-Forwarded-*` and cert headers stripped from non-proxy clients
- **SeaORM parameterized queries:** Parameterized queries throughout, preventing injection (rate limiter uses raw SQL via `Statement::from_sql_and_values()` for atomic upsert — fully parameterized)
- **Permission model:** Consistent `has_permission()` checks on all protected endpoints
- **Rate limit architecture:** Database-backed sliding window (correct for HA)
- **Wire protocol design:** Strongly typed enums, comprehensive test coverage (230+ tests), AsyncAPI documentation
- **Soft deletion:** Consistent `deactivated_at` pattern prevents accidental data loss
- **Certificate revocation on deactivation:** Properly revokes certs and bumps revocation version
