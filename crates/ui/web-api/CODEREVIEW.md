# Web API Code Review Findings

**Review date:** 2026-02-08
**Branch:** `refactor/codereview-webapi`
**Scope:** `uptrakit-web-api`, `uptrakit-internal-wire`, `uptrakit-web-api-types` — architecture, security & safety, high availability

---

## Summary

| Severity | Count | Fixed |
|----------|-------|-------|
| Critical | 7 | 3 |
| High | 11 | 0 |
| Medium | 17 | 0 |
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

### C4. Rate Limiter TOCTOU — Check and Increment Are Not Atomic

**File:** `src/auth/rate_limit.rs:62-99`

`check_rate_limit` performs a non-atomic read-then-write: SELECT at line 63, check at line 71, UPDATE at line 83. Concurrent requests (same instance or across HA instances) can all read the same `request_count`, pass the check, and increment. The conditional UPDATE does not include `request_count < max_requests` as a WHERE condition. Additionally, the `upsert_new_window` path (line 94-97) resets the counter to 1 when `rows_affected == 0`, providing a second bypass avenue.

**Recommendation:** Replace with a single atomic SQL statement: `INSERT ... ON CONFLICT DO UPDATE SET request_count = CASE WHEN window_start < threshold THEN 1 ELSE request_count + 1 END ... RETURNING request_count`.

### C5. Shell Injection via Unsanitized Hook Parameters

**File:** `src/update_hooks.rs:138-171`

`resolve_systemd_hook` and `resolve_docker_compose_hook` interpolate user-controlled `service_name`, `project_dir`, and `compose_file` directly into shell commands with `format!()`. A `service_name` of `myapp; rm -rf /` produces `systemctl stop myapp; rm -rf /`. Custom commands (`commands` field) also pass through without any validation (line 119-122).

**Recommendation:** (a) Validate predefined hook fields against strict patterns (alphanumeric + hyphen/underscore for service names, validated paths for directories). (b) For custom commands, document the security boundary clearly and consider a separate permission.

### C6. No WebSocket Message Size Limits

**File:** `src/routes/service_ws.rs:187`

`WebSocketUpgrade` is called with no configuration — inheriting tungstenite's default 64 MB max message size. Combined with no rate limiting on the WebSocket endpoint, a small number of concurrent large messages can exhaust server memory.

**Recommendation:** Call `.max_message_size(some_reasonable_limit)` on the upgrade (e.g., 256 KB or 1 MB).

### C7. No Rate Limiting on WebSocket Endpoint + Argon2 Brute-Force Amplification

**Files:** `src/routes/service_ws.rs:200-232`, `src/middleware/rate_limit.rs:20-58`

The `/api/v1/ws/service` endpoint is not rate-limited and accepts anonymous connections. The `lookup_by_secret` function (line 200-232) performs an unprotected O(N) argon2 scan against all non-deactivated MQTT services when the fast SHA-256 lookup fails. Each anonymous connection attempt causes significant CPU consumption.

**Recommendation:** Add per-IP connection rate limiting for WebSocket upgrades. For the bearer lookup, index enrollment secrets by SHA-256 prefix to eliminate the O(N) argon2 scan.

---

## HIGH

### H1. First-User Registration Race Condition

**File:** `src/routes/auth.rs:86-136`

Non-atomic read-then-act: `User::find().count()` -> `insert()` -> `assign_owner_role()`. Two concurrent requests during initial setup can both observe zero users and both get the `owner` role. Same race exists in the OIDC path (`src/routes/oidc_auth.rs:443-447`).

**Recommendation:** Wrap in a serializable transaction or use `INSERT ... WHERE NOT EXISTS`.

### H2. JWT Access Tokens Cannot Be Revoked (15-Minute Window)

**Files:** `src/middleware/require_auth.rs:126-156`, `src/auth/jwt.rs:109-118`

JWT tokens are validated statelessly — no DB lookup, no revocation check, no `is_active` check. Permissions are baked into the JWT at issuance time and trusted for 15 minutes. After user deactivation, role changes, or logout, access persists.

**Recommendation:** Either add a short-lived token denylist (checked on each request), reduce token lifetime to ~1-2 minutes, or add a DB-backed `is_active` check on critical endpoints.

### H3. JWT Signing Key Divergence in HA Deployments

**File:** `src/auth/jwt.rs:38-73`

Each controller instance generates its own JWT signing key from its local `data_dir`. If instances don't share the same state directory, tokens issued by instance A are rejected by instance B.

**Recommendation:** Document as a deployment requirement: all HA instances must share the same state directory (or use a shared key management system). Consider adding a health check that detects key mismatch.

### H4. Tenant Isolation Bypass via X-Tenant-Id Header

**File:** `src/middleware/tenant_context.rs:29-51`

`TenantContext` reads `X-Tenant-Id` from any request and uses it without authorization checks. Any authenticated user can set `X-Tenant-Id: <any-uuid>` and access another tenant's resources. This affects **all** tenant-scoped endpoints.

**Recommendation:** Either remove the header processing entirely (since multi-tenancy is "future work"), or gate it behind user-tenant authorization.

### H5. Origin Header Trusted from Untrusted Clients

**File:** `src/middleware/resolve_proxy_headers.rs:274-284`

`resolve_external_base_url` gives highest priority to the `Origin` header regardless of trusted-proxy status. An attacker can set `Origin: https://evil.com` and OIDC redirect URLs and device auth verification URLs will point to the attacker's domain. The `strip_proxy_headers` function does NOT strip `Origin`.

**Recommendation:** Only trust `Origin` from trusted proxies, or validate it against a configured allowlist.

### H6. Server Private Key Written Without Restricted Permissions + Non-Atomic Write

**File:** `src/routes/server_cert.rs:138-147`

`std::fs::write()` creates files with default umask permissions (typically 0644). The `server.key` file is world-readable. The cert and key are written in two separate calls — a crash between them produces a mismatched pair.

**Recommendation:** Use `OpenOptionsExt` with mode `0o600`. Write to temp files then atomically rename.

### H7. Connection Registry Overwrites on Concurrent Reconnect

**File:** `src/service_connections.rs:55-67, 74-91`

When a service reconnects before cleanup, `register_agent`/`register_mqtt` unconditionally overwrite via `HashMap::insert`. The old handler's cleanup then removes the **new** connection's entry, leaving the service unable to receive push notifications.

**Recommendation:** Use a generation counter or connection epoch. The `unregister` method should verify the connection being removed matches the one that was registered.

### H8. Settings Reload Torn Reads — 6 Independent RwLocks Updated Sequentially

**File:** `src/settings.rs:232-281`

`reload_from_db` acquires and releases 6 independent `RwLock`s sequentially. Concurrent request handlers can observe a mix of old and new settings.

**Recommendation:** Use a single `RwLock<AllSettings>` struct or a `tokio::sync::watch` channel for atomic swaps.

### H9. TOCTOU in `upsert_setting` — Concurrent Upserts Can Conflict

**File:** `src/settings_store.rs:50-84`

Read-then-insert pattern without transaction. Two concurrent inserts for the same key produce a unique constraint violation. Same pattern exists in `bump_settings_version` and `bump_revocation_version`.

**Recommendation:** Use SeaORM's `on_conflict` (database-level upsert).

### H10. Update History Operations Not Checked Against Agent Ownership (IDOR)

**File:** `src/routes/agent_ws.rs:306-389`

`UpdateStarted`, `UpdateOutput`, and `UpdateResult` look up records by `update_history_id` from the message payload without verifying the record belongs to the current agent's host. A compromised agent can tamper with any update record.

**Recommendation:** Validate that the update record's `host_id` belongs to a host linked to the authenticated `service_id`.

### H11. Unbounded Update Output Accumulation

**File:** `src/routes/agent_ws.rs:327-343`

Each `UpdateOutput` message appends to the existing output with `format!("{}{}\n", record.output, payload.output)` — loading the entire existing output from DB, appending, and writing back. No size limit. Quadratic memory growth.

**Recommendation:** Enforce a maximum total size (e.g., 10 MB). Truncate or reject further output messages once the limit is reached.

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

### M8. Broadcast Holds Read Lock During Async Sends

**File:** `src/service_connections.rs:125-130`

A slow consumer blocks all registry writes during broadcast.

### M9. Relaxed Memory Ordering on Settings Version Counters

**File:** `src/settings.rs:294-295`

`Ordering::Relaxed` loads can return stale values on ARM/multi-core, causing missed reloads. Two separate non-atomic loads can also produce inconsistent version pairs.

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
- **SeaORM parameterized queries:** No raw SQL, preventing injection
- **Permission model:** Consistent `has_permission()` checks on all protected endpoints
- **Rate limit architecture:** Database-backed sliding window (correct for HA)
- **Wire protocol design:** Strongly typed enums, comprehensive test coverage (230+ tests), AsyncAPI documentation
- **Soft deletion:** Consistent `deactivated_at` pattern prevents accidental data loss
- **Certificate revocation on deactivation:** Properly revokes certs and bumps revocation version
