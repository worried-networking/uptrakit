# Web API Code Review Findings

**Review date:** 2026-02-08
**Branch:** `refactor/codereview-webapi`
**Scope:** `uptrakit-web-api`, `uptrakit-internal-wire`, `uptrakit-web-api-types` — architecture, security & safety, high availability

---

## Summary

| Severity | Count |
|----------|-------|
| Medium | 13 |
| Low | 10 |

---

## MEDIUM

### M1. Rate Limiter Fails Open on DB Errors

**File:** `src/middleware/rate_limit.rs:96-100`

When the DB check fails, requests pass through. An attacker inducing DB pressure disables all rate limiting.

### M2. OIDC AutoLink Bypasses Account Ownership Verification

**File:** `src/auth/authentication.rs:147-159`

When a user has no password and no other OIDC link, `AutoLink` fires without email ownership proof. Risk depends on OIDC provider email verification.

### M4. OCSP 1-Hour Cache Serves Stale Revocation Status

**File:** `src/routes/ocsp.rs:19-26`

`Cache-Control: max-age=3600` means revoked certificates may appear valid for up to 1 hour.

### M5. Missing Rate Limiting on OIDC Exchange/Link/Complete-Registration

**File:** `src/middleware/rate_limit.rs:20-58`

These public, unauthenticated endpoints accept secret tokens but aren't rate-limited. The `complete-registration` endpoint uses non-destructive `get()`, allowing unlimited brute-force of the registration token.

### M7. No Idle Timeout on WebSocket Connections

**Files:** All WS handlers

No `tokio::time::timeout` branch. Half-open TCP connections persist indefinitely.

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

---

## Fix Plans

> Fix plans for open findings, ordered by severity.

---

### Plan 19: M1 — Rate Limiter Fails Open on DB Errors

**Problem:** `rate_limit_auth` middleware (`src/middleware/rate_limit.rs:96-100`) catches DB errors and allows the request through (fail-open). An attacker inducing DB pressure disables all rate limiting.

**Plan:**
1. Add configurable `fail_closed` flag per endpoint category in `EndpointRateLimit`.
2. Set `fail_closed: true` for security-critical endpoints (login, register), `false` for others.
3. Add an in-memory fallback rate limiter using `Mutex<HashMap<String, (u32, Instant)>>` for fail-closed endpoints.
4. Add periodic cleanup for fallback counters.

**Files:** `src/middleware/rate_limit.rs`

---

### Plan 20: M2 — OIDC AutoLink Bypasses Account Ownership Verification

**Problem:** `resolve_oidc_user` returns `AutoLink` when a user has no password and no other active OIDC link, without proof of email ownership. Risk depends on OIDC provider email verification.

**Plan:**
1. Add `email_verified` and `provider_trusts_email` to `OidcUserParams`.
2. Guard `AutoLink` on `email_verified == Some(true)` and `provider_trusts_email == true`.
3. Add `email_verified_trusted` flag to OIDC provider entity/config.
4. Extract `email_verified` from ID token claims.

**Files:** `src/auth/authentication.rs`, `src/routes/oidc_auth.rs`, `src/routes/oidc_providers.rs`, migration, `crates/shared/db/src/entity/oidc_provider.rs`

---

### Plan 22: M4 — OCSP 1-Hour Cache Serves Stale Revocation Status

**Problem:** `Cache-Control: max-age=3600` on OCSP responses means revoked certificates may appear valid for up to 1 hour.

**Plan:**
1. Reduce cache TTL to 5 minutes (`max-age=300`).
2. Extract as `OCSP_CACHE_MAX_AGE_SECS` constant.
3. Set `nextUpdate` in the OCSP response body to match.
4. Consider making it configurable via `SettingKey`.

**Files:** `src/routes/ocsp.rs`, `src/ocsp.rs`

---

### Plan 23: M5 — Missing Rate Limiting on OIDC Exchange/Link/Complete-Registration

**Problem:** OIDC token exchange, account link, and complete-registration endpoints accept secret tokens but aren't rate-limited. `complete-registration` uses non-destructive `get()`.

**Plan:**
1. Add OIDC endpoints to `RATE_LIMITS` (10/60s for exchange/link, 5/60s for complete-registration, all fail-closed).
2. Change `complete-registration` from `get()` to `take()` for one-time-use tokens.

**Files:** `src/middleware/rate_limit.rs`, `src/routes/oidc_auth.rs`

---

### Plan 25: M7 — No Idle Timeout on WebSocket Connections

**Problem:** All WS handlers have no `tokio::time::timeout` branch. Half-open TCP connections persist indefinitely.

**Plan:**
1. Add `WS_IDLE_TIMEOUT` constant (300 seconds).
2. Track `last_activity` and check via periodic `tokio::time::interval`.
3. Apply to all four handler loops (agent authenticated/enrolled, MQTT authenticated/enrolled).

**Files:** `src/routes/service_ws.rs`, `src/routes/agent_ws.rs`, `src/routes/mqtt_ws.rs`

---

### Plan 28: M10 — `cert_signed_by_ca` Only Compares DN, Not Cryptographic Signature

**Problem:** `cert_signed_by_ca` (`src/pki_utils.rs:84-98`) only compares issuer DN. A certificate with matching DN but forged signature passes.

**Plan:**
1. Add `cert.verify_signature(Some(ca.public_key()))` call after DN comparison.
2. Treat unsupported algorithms as verification failure.

**Files:** `src/pki_utils.rs`

---

### Plan 29: M11 — No Max Password Length — Argon2 DoS Vector

**Problem:** No upper bound on password length. Multi-MB passwords exhaust Argon2 resources.

**Plan:**
1. Add `MAX_PASSWORD_LENGTH = 1024` constant.
2. Validate in register, login, and password change endpoints.

**Files:** `src/routes/auth.rs`, `src/routes/oidc_auth.rs`

---

### Plan 30: M12 — Email Not Normalized

**Problem:** Case-sensitive email comparison allows duplicate registrations for the same logical email.

**Plan:**
1. Add `normalize_email()` helper (trim + lowercase).
2. Apply at all entry points (register, login, OIDC resolve, OIDC link).
3. Migration to lowercase existing emails.

**Files:** `src/routes/auth.rs`, `src/auth/authentication.rs`, `src/routes/oidc_auth.rs`, migration

---

### Plan 31: M13 — assign_hosts Missing Tenant Check on Host

**Problem:** `assign_hosts` verifies host existence without `tenant_id` filter. Cross-tenant host assignment possible.

**Plan:**
1. Add `host::Column::TenantId.eq(tenant.tenant_id)` filter to host existence check.
2. Audit all `Host::find_by_id` calls for tenant filtering.

**Files:** `src/routes/software_items.rs`

---

### Plan 32: M14 — MQTT Lease Takeover Without Authorization

**Problem:** Any MQTT service can claim arbitrary client IDs and steal leases from other instances.

**Plan:**
1. Add `service_id` column to `mqtt_leases` table.
2. Only allow takeover of stale leases (heartbeat expired).
3. Verify tenant match for MQTT client IDs.

**Files:** `src/mqtt_lease_coordinator.rs`, migration, `crates/shared/db/src/entity/mqtt_lease.rs`

---

### Plan 33: M15 — Stale Lease Cleanup Does Not Notify In-Memory Registry

**Problem:** DB leases cleaned up but in-memory registry still holds references, causing dual assignment.

**Plan:**
1. Return freed `(mqtt_client_id, service_id)` pairs from `cleanup_stale_leases`.
2. Release corresponding entries in the connection registry after cleanup.

**Files:** `src/mqtt_lease_coordinator.rs`, periodic task caller

---

### Plan 34: M16 — OCSP Hardcoded to ECDSA P-256

**Problem:** `sign_response` hardcodes `ECDSA_P256_SHA256_ASN1_SIGNING`. RSA or P-384 CA keys cause OCSP signing failure.

**Plan:**
1. Detect CA key type and select appropriate signing algorithm (P-256, P-384, RSA).
2. Return matching algorithm OID from `sign_with_detected_algorithm`.

**Files:** `src/ocsp.rs`

---

### Plan 35: M17 — Silent Fallback to Weak UUID on CSPRNG Failure

**Problem:** `generate_secure_token().unwrap_or_else(|_| generate_uuid())` silently falls from 256-bit to ~122-bit entropy.

**Plan:**
1. Replace all 4 `unwrap_or_else` fallbacks with explicit error handling (return 500).
2. Log CSPRNG failure at ERROR level.

**Files:** `src/routes/oidc_auth.rs`

---

### Plan 36: L1 — User Enumeration via Differential Login Responses

**Problem:** "User is deactivated" (403) vs "Invalid credentials" (401) reveals account status.

**Plan:**
1. Unify all login failure responses to `401 "Invalid credentials"`.
2. Add constant-time dummy hash for non-existent users.

**Files:** `src/routes/auth.rs`

---

### Plan 37: L2 — Logout Is Unauthenticated

**Problem:** Anyone with a refresh token can revoke it without proving identity.

**Plan:**
1. Move logout route into `auth_routes` (require_auth middleware).
2. Add `AuthenticatedUser` extraction and ownership verification.
3. Add `verify_refresh_token_owner` to `SessionService`.

**Files:** `src/routes/auth.rs`, `src/auth/session.rs`, router setup

---

### Plan 38: L3 — Bearer Token Prefix Is Case-Sensitive

**Problem:** `strip_prefix("Bearer ")` rejects `bearer ` per RFC 6750 case-insensitivity.

**Plan:**
1. Use `eq_ignore_ascii_case("bearer ")` for prefix matching.
2. Apply to both `require_auth.rs` and `service_ws.rs` bearer extraction.

**Files:** `src/middleware/require_auth.rs`, `src/routes/service_ws.rs`

---

### Plan 39: L4 — Error Messages Contain Trailing Newlines

**Problem:** Several error strings in `require_auth.rs` contain `\n`, inconsistent with codebase.

**Plan:** Remove trailing newlines from all 5 error message strings.

**Files:** `src/middleware/require_auth.rs`

---

### Plan 40: L5 — API Token `last_used_at` Written on Every Request

**Problem:** Every API token request triggers a DB write. Write amplification under heavy usage.

**Plan:**
1. Add 5-minute debounce: only update if `last_used_at` is stale (>300 seconds old).
2. Extract `API_TOKEN_LAST_USED_DEBOUNCE_SECS` constant.

**Files:** `src/auth/api_token.rs`

---

### Plan 41: L6 — Health Endpoint Returns Static "ok"

**Problem:** No DB connectivity or CA availability check.

**Plan:**
1. Add `State(state)` extraction and DB ping (`SELECT 1` with 3s timeout).
2. Check CA snapshot availability.
3. Return 503 with reason when unhealthy.

**Files:** `src/routes/health.rs`

---

### Plan 42: L7 — Device Flow `client_name` Not Length-Validated

**Problem:** Unbounded string from unauthenticated endpoint stored in DB.

**Plan:**
1. Add `MAX_CLIENT_NAME_LENGTH = 256` validation.
2. Trim whitespace and reject empty strings after trimming.

**Files:** `src/routes/device_auth.rs`

---

### Plan 43: L8 — MQTT Enrolled Loop Has No Push Channel

**Problem:** MQTT service not registered in connection registry during pending-approval state. Approval notifications can't be pushed.

**Plan:**
1. Register MQTT service in connection registry during enrollment.
2. Add `tokio::select!` with push channel for `Approved`/`Rejected` messages.
3. Optionally retain Ping-based polling as fallback.

**Files:** `src/routes/mqtt_ws.rs`

---

### Plan 44: L9 — Deactivation Doesn't Update `status` Column

**Problem:** `deactivated_at` is set but `status` remains "approved", causing audit trail inconsistency.

**Plan:**
1. Set `status = Deactivated` in `deactivate_service`.
2. Apply same fix to `merge_service` source deactivation path.

**Files:** `src/routes/services.rs`

---

### Plan 45: L10 — N+1 Queries in List Endpoints

**Problem:** Per-record DB lookups in `update_history`, `hosts`, and `software_items` list endpoints. Up to 2000+ queries per request with max pagination.

**Plan:**
1. Batch-load host names and software item names for update history (2 queries vs 2N).
2. Batch-load agent links and models for hosts with JOIN (2 queries vs 3N).
3. Batch-load provider configs and host counts with GROUP BY for software items (2 queries vs 2N).

**Files:** `src/routes/update_history.rs`, `src/routes/hosts.rs`, `src/routes/software_items.rs`
