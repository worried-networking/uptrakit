# Controller System Code Review

Reviewed: 2026-02-08
Scope: Full controller stack — `uptrakit-controller`, `uptrakit-web-api`, shared crates (`db`, `wire`, `web-api-types`, `enrollment`), and provider crates.
Commit: `b855b6f`

> This review covers architecture, security & safety, high availability, data integrity, and best practices. Findings from the wire-protocol-specific review (`crates/shared/wire/CODEREVIEW.md`) are referenced but not duplicated — only new or broader findings appear here.

---

## Table of Contents

1. [Findings Summary](#findings-summary)
2. [Critical Findings](#1-critical-findings)
3. [High Findings](#2-high-findings)
4. [Medium Findings](#3-medium-findings)
5. [Low Findings](#4-low-findings)
6. [Positive Observations](#5-positive-observations)
7. [Fix Plans](#fix-plans)

---

## Findings Summary

| Severity | Count | Key Themes |
|----------|-------|------------|
| **CRITICAL** | 6 | First-user race, MQTT creds at rest, CA key exposure surface, tenant header bypass, TOFU verifier, logout authz bypass |
| **HIGH** | 12 | Command injection in hooks, merge without transaction, missing update ownership validation, missing cert lifetime cap, missing connection limits, DB schema gaps |
| **MEDIUM** | 18 | Race conditions in rate limiter / settings / leases, missing refresh token rotation, N+1 queries, SSRF vectors, settings desync, CRL gap |
| **LOW** | 16 | Missing validation, minor inconsistencies, defense-in-depth gaps |

---

## 1. Critical Findings

### CR-1: Race condition in first-user owner role assignment

**Files:** `web-api/src/routes/auth.rs:86-136`, `web-api/src/routes/oidc_auth.rs:443-477`

The "first user" check uses `User::find().count()` followed by role assignment without any transactional isolation. Two concurrent registration requests can both observe `count == 0` and both receive the `owner` role, granting full administrative access to both accounts.

**Impact:** An attacker can race the first user registration to obtain owner privileges.

**Fix plan:** [FP-CR1](#fp-cr1-atomic-first-user-registration)

### CR-2: MQTT passwords stored and transmitted in plaintext

**Files:** `db/src/entity/mqtt_client.rs:16`, `web-api/src/routes/mqtt_ws.rs:491`, `notification_service.rs:71`

MQTT broker passwords are:
- Stored as `Option<String>` in the database with no encryption at rest
- Transmitted to MQTT service instances in plaintext JSON over WebSocket
- Written to the `controller_events` outbox table as serialized JSON (overlaps with wire CODEREVIEW A1)

**Impact:** Database compromise exposes all MQTT broker credentials. Wire-level logging or proxy inspection reveals passwords.

**Fix plan:** [FP-CR2](#fp-cr2-encrypt-sensitive-credentials-at-rest)

### CR-3: CA private keys in shared `CaSnapshotData` structure

**Files:** `controller/src/pki.rs:216-220`, `web-api/src/lib.rs:50-55`

The `CaSnapshotData` contains both metadata (fingerprints, cert PEMs) and secrets (`active_key_pem`, `previous_key_pem`). This snapshot is broadcast via a `watch` channel to every component that holds a receiver, including `AppState` which is `Arc`-shared with all HTTP handlers. Additionally, CA private keys are stored unencrypted in the `ca_certificates` DB table.

**Impact:** If any API handler accidentally serializes the snapshot, CA private keys leak. Database compromise yields all historical CA private keys.

**Fix plan:** [FP-CR3](#fp-cr3-separate-ca-signing-material-from-metadata)

### CR-4: Unauthenticated tenant context switching via header

**File:** `web-api/src/middleware/tenant_context.rs:29-51`

The `TenantContext` extractor reads `X-Tenant-Id` from request headers with NO authorization check. Any client (authenticated or not) can set this header to any UUID. If multi-tenancy is ever enabled, this is a complete tenant isolation bypass.

**Impact:** Currently limited (single-tenant mode), but becomes critical if multi-tenancy is activated.

**Fix plan:** [FP-CR4](#fp-cr4-restrict-tenant-context-header)

### CR-5: TOFU TLS verifier accepts all certificates and signatures

**File:** `enrollment/src/tls.rs:115-174`

The `AcceptAnyCert` verifier unconditionally returns success for `verify_server_cert`, `verify_tls12_signature`, AND `verify_tls13_signature`. This means it bypasses not just certificate chain validation but also handshake signature verification, enabling trivial MITM during TOFU enrollment.

**Impact:** Active MITM during `--tofu` enrollment can inject a rogue CA, compromising all subsequent mTLS connections.

**Mitigating factors:** `--tofu` is opt-in, conflicts with `--ca-cert` and `--pki-addr`, and is skipped if CA is already cached.

**Fix plan:** [FP-CR5](#fp-cr5-strengthen-tofu-with-fingerprint-pinning)

### CR-6: Logout endpoint accepts unauthenticated requests

**Files:** `web-api/src/lib.rs:437`, `web-api/src/routes/auth.rs:307-321`

The `logout` route is registered outside the `auth_routes` group (which has `require_auth` middleware). Any unauthenticated caller can revoke any valid refresh token by providing its value.

**Impact:** Design violation — defense in depth requires the caller to prove ownership before revocation. Practical exploitation requires guessing a 256-bit token.

**Fix plan:** [FP-CR6](#fp-cr6-require-auth-on-logout)

---

## 2. High Findings

### HI-1: Command injection in update hooks via admin-configurable parameters

**File:** `web-api/src/update_hooks.rs:138-172`

`resolve_systemd_hook()` directly interpolates `service_name` into a shell command (`systemctl {action} {service_name}`). Similarly, `resolve_docker_compose_hook()` interpolates `project_dir` and `compose_file`. A compromised admin account can set `service_name` to `nginx; rm -rf /`, achieving RCE on agents.

**Impact:** Stored command injection executed on remote agents. Requires admin privileges but has unlimited blast radius.

**Fix plan:** [FP-HI1](#fp-hi1-sanitize-hook-parameters)

### HI-2: Service merge operation not wrapped in a transaction

**File:** `web-api/src/routes/services.rs:455-635`

The merge operation performs sequential DB operations (deactivate source, revoke certs, update target, copy host links) without a transaction. A mid-operation failure leaves the system in an inconsistent, potentially irrecoverable state.

**Impact:** Partial failure leaves orphaned services and irrecoverable state.

**Fix plan:** [FP-HI2](#fp-hi2-wrap-merge-in-transaction)

### HI-3: No authorization check on update status messages from agents

**File:** `web-api/src/routes/agent_ws.rs:306-389`

`UpdateStarted`, `UpdateOutput`, and `UpdateResult` handlers look up `update_history` records by ID without verifying the record belongs to a host linked to the authenticated agent. A compromised agent can manipulate status and output of updates belonging to other agents.

**Impact:** Cross-agent update record tampering via UUID guessing (v7 UUIDs are time-ordered).

> Also identified in wire CODEREVIEW as D2.

**Fix plan:** See wire CODEREVIEW FP-17.

### HI-4: No maximum certificate lifetime enforcement in cert signer

**File:** `controller/src/cert_signer.rs:28-29`

`sign_agent_csr()` accepts an arbitrary `time::Duration` with no upper bound. An extremely large duration would produce a certificate valid for decades.

**Fix plan:** [FP-HI4](#fp-hi4-cap-certificate-lifetime)

### HI-5: No maximum connection limit for WebSocket

**Files:** `web-api/src/service_connections.rs:55-90`, `web-api/src/routes/service_ws.rs:150-188`

The `ServiceConnectionRegistry` accepts unlimited registrations. Combined with the lack of rate limiting on anonymous WebSocket upgrades, this creates an unbounded memory exhaustion vector.

> Also identified in wire CODEREVIEW as S2.

**Fix plan:** See wire CODEREVIEW FP-1, extended with a registry connection cap.

### HI-6: MQTT lease race condition — no DB-level locking

**File:** `web-api/src/mqtt_lease_coordinator.rs:82-155`

Lease assignment uses application-level filtering without `SELECT FOR UPDATE` or unique constraint enforcement. Two controllers can simultaneously assign the same MQTT client to different services.

> Also identified in wire CODEREVIEW as H5.

**Fix plan:** See wire CODEREVIEW FP-8.

### HI-7: EventPoller cursor gap can cause missed events

**File:** `web-api/src/event_poller.rs:37-38`

The poller initializes `last_seen_id` to the current max ID. Events written between initialization and the first poll, or during a crash/restart window, may be permanently skipped.

> Also identified in wire CODEREVIEW as H2 and H3.

**Fix plan:** See wire CODEREVIEW FP-3, FP-12.

### HI-8: `service_certificates.service_id` FK has no ON DELETE action

**File:** `controller/src/migration/m20260129_000008_create_agent_certificates.rs:43-48`

No `on_delete()` is specified, defaulting to `RESTRICT`. This is inconsistent with `service_hosts` (CASCADE) — deleting a service cascades host links but is blocked by certificates, creating a half-deleted state.

**Fix plan:** [FP-HI8](#fp-hi8-fix-fk-cascade-inconsistencies)

### HI-9: `service_certificates.ca_fingerprint` has no FK to `ca_certificates`

**Files:** `migration/m20260129_000008` (service_certificates), `migration/m20260207_000025` (ca_certificates)

The `ca_fingerprint` composite PK column has no foreign key to `ca_certificates.fingerprint`. Migration 008 runs before 025, so the FK couldn't be defined at creation time — but no subsequent migration adds it.

**Fix plan:** [FP-HI8](#fp-hi8-fix-fk-cascade-inconsistencies) (combined)

### HI-10: `load_host_agents` helper lacks tenant scoping

**File:** `web-api/src/routes/hosts.rs:301-339`

Queries agents by `host_id` without tenant filtering. In multi-tenant mode, agents from another tenant could leak into host responses.

**Fix plan:** [FP-HI10](#fp-hi10-add-tenant-filter-to-host-agents)

### HI-11: No rate limiting on token refresh endpoint

**File:** `web-api/src/routes/auth.rs:382-443`

The refresh endpoint is public and has no dedicated rate limiting. An attacker with a stolen refresh token could rapidly generate access tokens.

**Fix plan:** [FP-HI11](#fp-hi11-rate-limit-refresh-endpoint)

### HI-12: OIDC client_secret stored in plaintext

**File:** `db/src/entity/oidc_provider.rs:71`, `controller/src/main.rs:388`

The OIDC client secret is stored as a bare string in the database. Database compromise exposes all OIDC client secrets.

**Fix plan:** [FP-CR2](#fp-cr2-encrypt-sensitive-credentials-at-rest) (combined)

---

## 3. Medium Findings

### ME-1: No refresh token rotation on use

**File:** `web-api/src/auth/session.rs:62-92`

When a refresh token is used, it is NOT rotated. If stolen, it remains usable for its full 7-day lifetime even after the legitimate user refreshes.

**Fix plan:** [FP-ME1](#fp-me1-implement-refresh-token-rotation)

### ME-2: OIDC auto-link without email verification

**File:** `web-api/src/auth/authentication.rs:156-159`

The `AutoLink` branch auto-links OIDC identity to an existing user based solely on email match. If the OIDC provider doesn't verify email addresses, this enables account takeover.

**Fix plan:** [FP-ME2](#fp-me2-check-email-verified-before-auto-link)

### ME-3: TOCTOU race in rate limit check

**File:** `web-api/src/auth/rate_limit.rs:62-106`

The read-then-update pattern allows concurrent requests to both pass the limit check before either increments the counter.

### ME-4: Rate limit fails open on database error

**File:** `web-api/src/middleware/rate_limit.rs:96-99`

Database outage completely disables rate limiting, allowing unrestricted brute-force attacks on auth endpoints.

### ME-5: Device flow approval lacks rate limiting and permission check

**Files:** `web-api/src/routes/device_auth.rs:183-215`, `middleware/rate_limit.rs:20-58`

The `/api/v1/auth/device/approve` endpoint is not rate-limited (user code brute-force risk) and any authenticated user can approve device flows regardless of permissions.

### ME-6: JWT-authenticated requests don't check user active status

**File:** `web-api/src/middleware/require_auth.rs:126-156`

The JWT path trusts claims without a database check. A deactivated user's JWT remains valid for up to 15 minutes. The API token path correctly checks `is_active`.

### ME-7: N+1 query patterns in list endpoints

**Files:** `web-api/src/routes/hosts.rs:87-91`, `software_items.rs:356-374`, `update_history.rs:194-199`

`list_hosts` issues an individual query per host (up to 1000). Same patterns in software items and update history.

### ME-8: Non-atomic settings reload across HA instances

**File:** `web-api/src/settings.rs:232-281`

`reload_from_db()` acquires/releases locks on each setting individually. Readers can observe partially-updated state between lock releases.

### ME-9: SSRF vector in GitHub provider via `api_base_url`

**File:** `providers/github/src/config.rs:17-19`

User-provided `api_base_url` is directly used in API requests without scheme validation or private IP blocking. Combined with unvalidated `owner`/`repo`, this enables SSRF.

### ME-10: Docker Registry auth token URL params not URL-encoded

**File:** `providers/docker-registry/src/auth.rs:69-82`

`service` and `scope` values from the `WWW-Authenticate` header are appended without URL encoding, enabling parameter injection by a malicious registry.

### ME-11: OIDC redirect URL constructed from client-supplied headers

**File:** `web-api/src/routes/oidc_auth.rs:1182-1196`

When `ExternalBaseUrl` is not configured, the `Origin` or `Host` header is used to construct OIDC redirect URLs. An attacker can redirect OIDC callbacks to their domain.

### ME-12: CRL `next_update` creates 24-hour revocation visibility gap

**File:** `controller/src/crl_manager.rs:295`

CRL clients cache until `next_update` (24h). Revoking a certificate has no effect on clients that already cached the old CRL.

### ME-13: MQTT client count limit TOCTOU

**File:** `web-api/src/mqtt_client_store.rs:88-91`

Count-then-insert pattern allows concurrent requests to exceed the configured maximum.

### ME-14: Settings version bump race in concurrent writes

**File:** `web-api/src/settings_store.rs:127-172`

Two concurrent first-time writes for a new tenant can both attempt to insert into `settings_version`, causing a unique constraint violation.

### ME-15: Server key file written without restricted permissions

**Files:** `controller/src/pki.rs:713-714`, `web-api/src/routes/server_cert.rs:137-140`

`std::fs::write()` uses the default umask. The private key file could be world-readable on permissive systems.

### ME-16: Registration settings desync across HA instances

**File:** `web-api/src/auth/registration.rs:158-261`

In-memory `RegistrationSettings` updated after DB writes. Other controller instances may serve stale registration mode (e.g., `Invite` instead of `Closed`).

### ME-17: Enrollment secret not zeroized in memory

**File:** `enrollment/src/identity.rs:318-337`

The `enrollment_secret` is stored as a plain `String` in `ServiceState` without `Zeroize`. Debug-derived `EnrolledPayload` can print secrets.

> Also identified in wire CODEREVIEW as S7.

### ME-18: Update history tenant scoping loads all host models

**File:** `web-api/src/routes/update_history.rs:61-70`

Loads all `Host` models for a tenant just to extract UUIDs, then passes all IDs to an `IN()` clause. Wasteful for tenants with many hosts.

---

## 4. Low Findings

### LO-1: No email format validation in registration

**File:** `web-api/src/routes/auth.rs:39-188`

Only password length is validated. Empty strings, strings without `@`, and absurdly long strings are accepted as emails.

### LO-2: No password maximum length check

**File:** `web-api/src/routes/auth.rs:49`

Minimum 8 characters but no maximum. Extremely long passwords sent to Argon2 can cause CPU exhaustion.

### LO-3: API token name has no length validation

**File:** `web-api/src/routes/api_tokens.rs:29-51`

Unbounded name string could waste database storage.

### LO-4: API tokens have no expiration

**File:** `db/src/entity/api_token.rs`

No `expires_at` field. Tokens remain valid indefinitely until manually revoked.

### LO-5: Case-sensitive email comparison

**File:** `web-api/src/auth/authentication.rs:116-120`

Database collation may treat `user@Example.com` and `user@example.com` as different users.

### LO-6: No JWT audience/issuer claim validation

**File:** `web-api/src/auth/jwt.rs:16-26`

`AccessTokenClaims` has no `aud` or `iss` fields. Tokens from one service could be replayed against another in a multi-service architecture.

### LO-7: `last_used_at` write on every API token use

**File:** `web-api/src/auth/api_token.rs:121-126`

Every authenticated API request triggers a DB write. Write amplification under heavy API token usage.

### LO-8: Revoked sessions not cleaned up

**File:** `web-api/src/auth/session.rs:94-112`

`cleanup_expired_sessions` only deletes expired sessions, not revoked ones. Revoked sessions accumulate.

### LO-9: Soft-delete column naming inconsistency

**Files:** Multiple entities

Three different names for the same concept: `deactivated_at` (most entities), `deleted_at` (`oidc_providers`), `revoked_at` (`sessions`, `api_tokens`).

### LO-10: Missing indexes on hot query paths

**Files:** Multiple migrations

- `user_roles`: No index on `(user_id)` or `(role_id, tenant_id)` for role lookups
- `sessions`: No index on `revoked_at` for active session queries
- `service_certificates`: No FK index to `ca_certificates`

### LO-11: `available_versions` allows duplicate version strings per software item

**File:** `migration/m20260201_000015`

No unique constraint on `(software_item_id, version)`. Multiple rows with the same version can be inserted.

### LO-12: `update_history.status` stored as free-form string

**File:** `migration/m20260203_000018:29`

No CHECK constraint ensuring only valid status values (`pending`, `in_progress`, `completed`, `failed`).

### LO-13: Owner role assignment failure silently continues

**File:** `web-api/src/routes/auth.rs:117-136`

If assigning owner role fails, the first user is created without admin access with no recovery path.

### LO-14: PKI HTTP listener hardcoded to `0.0.0.0`

**File:** `controller/src/main.rs:1070`

Binds on all interfaces. In multi-NIC environments this exposes PKI endpoints to untrusted networks.

### LO-15: `controller_events` table has no partition strategy

**File:** `migration/m20260207_000024`

Outbox table grows unbounded. No `expires_at` column; relies on application-level cleanup.

### LO-16: Migration 011 down path is lossy

**File:** `migration/m20260131_000011`

Rolling back destroys user-role assignments for users migrated from admin to owner.

---

## 5. Positive Observations

These aspects demonstrate strong engineering practices:

1. **Consistent permission checks:** Every authenticated endpoint checks specific permissions. No endpoint was found missing its check.

2. **No SQL injection vectors:** All queries use SeaORM's parameterized query builder. Zero raw SQL.

3. **Proper tenant scoping:** All entity lookups in route handlers filter by `tenant_id` consistently.

4. **Strong password hashing:** Argon2id with OWASP-recommended parameters (19 MiB, 2 iterations).

5. **CSRF/PKCE/Nonce in OIDC:** All three protections are correctly implemented with one-time-use server-side state.

6. **HA-safe atomic patterns:** Device flow approve/consume, OIDC flow take, and CA rotation CAS all use atomic DB operations.

7. **Secret masking in API responses:** Provider config secrets are masked in GET responses and restored on PUT.

8. **Replay protection on wire protocol:** Strict monotonic sequence validation with comprehensive test coverage.

9. **Pagination with hard cap:** Max 1000 items per page, enforced in shared types.

10. **No unsafe code:** The entire Rust codebase uses only safe abstractions.

11. **Error messages don't leak internals:** 500 errors return generic messages. Stack traces go to tracing only.

12. **Graceful shutdown orchestration:** Scattered restart notifications, cancellation tokens, drain timeouts.

13. **Soft-delete everywhere:** Destructive operations use soft-delete, enabling audit trails.

14. **Header stripping for non-proxy clients:** Certificate-related headers stripped when not from trusted proxy.

15. **Trusted proxy validation:** Client IP forwarded headers only trusted from configured proxy IPs.

---

## Fix Plans

### FP-CR1: Atomic first-user registration

**Addresses:** CR-1

**Problem:** Two concurrent registrations can both become owner.

**Plan:**
1. Wrap the entire first-user check + creation + role assignment in a serializable transaction.
2. Add a unique constraint or advisory lock on "initial setup completed" to prevent races.
3. Use `SELECT ... FOR UPDATE` on the user count or a dedicated "setup_completed" flag in settings.
4. As a simpler alternative: use the existing `registration.mode` setting — set it to `Closed` inside the same transaction that creates the first user. The second concurrent request will see `Closed` and fail.

**Files:** `web-api/src/routes/auth.rs`, `web-api/src/routes/oidc_auth.rs`

---

### FP-CR2: Encrypt sensitive credentials at rest

**Addresses:** CR-2, HI-12

**Problem:** MQTT passwords, OIDC client secrets, and CA private keys are stored as plaintext in the database.

**Plan:**
1. Introduce an application-level encryption layer using AES-256-GCM with a master key.
2. The master key is loaded from an environment variable or file at startup (not stored in DB).
3. Add `encrypt_secret()` / `decrypt_secret()` helpers that produce `ENC:base64(nonce+ciphertext+tag)` strings.
4. Apply to: `mqtt_clients.password`, `oidc_providers.client_secret`, `ca_certificates.key_pem`.
5. Migration: write a migration that encrypts existing plaintext values in-place.
6. Ensure the encrypted values never appear in logs, API responses, or outbox events.

**Alternative (simpler for MVP):** Use a dedicated secrets table with the encrypted value and a key version, allowing key rotation.

**Files:** New `web-api/src/crypto.rs`, migrations, entity files, stores that read/write secrets.

---

### FP-CR3: Separate CA signing material from metadata

**Addresses:** CR-3

**Problem:** `CaSnapshotData` mixes metadata with private keys, shared broadly.

**Plan:**
1. Split into two structs:
   - `CaMetadata` — fingerprints, cert PEMs, bundle, pki_addr (safe to share widely)
   - `CaSigningMaterial` — key PEMs (only where signing is needed)
2. `AppState` holds `watch::Receiver<CaMetadata>` (all handlers).
3. `CertSigner` holds `watch::Receiver<CaSigningMaterial>` (only the signer).
4. The controller's `main.rs` sends to both channels on CA init/rotation.

**Files:** `web-api/src/lib.rs`, `controller/src/pki.rs`, `controller/src/cert_signer.rs`, `controller/src/crl_manager.rs`

---

### FP-CR4: Restrict tenant context header

**Addresses:** CR-4

**Problem:** `X-Tenant-Id` header accepted from any client without authorization.

**Plan:**
1. **Immediate (single-tenant mode):** Ignore the `X-Tenant-Id` header entirely. Always use `default_tenant_id`. Log a warning if the header is present.
2. **Future (multi-tenant mode):** Validate that the authenticated user has access to the requested tenant before accepting the header. Unauthenticated endpoints always use the default tenant.

**Files:** `web-api/src/middleware/tenant_context.rs`

---

### FP-CR5: Strengthen TOFU with fingerprint pinning

**Addresses:** CR-5

**Problem:** TOFU mode accepts any certificate without even verifying signatures.

**Plan:**
1. Replace the full bypass with a fingerprint-based approach:
   - On first connection, accept the server cert, compute its SHA-256 fingerprint, display it to the user, and save it.
   - On subsequent connections, verify the fingerprint matches.
2. Alternatively, at minimum verify TLS handshake signatures (keep cert verification off but ensure the handshake is cryptographically valid).
3. Add a prominent runtime warning when `--tofu` is used.
4. Consider requiring interactive confirmation (`--tofu` prints fingerprint, user must confirm).

**Files:** `enrollment/src/tls.rs`, `enrollment/src/ca.rs`

---

### FP-CR6: Require auth on logout

**Addresses:** CR-6

**Problem:** Logout is outside the `require_auth` middleware group.

**Plan:**
1. Move the `logout` route into the `auth_routes` group (which has `require_auth` middleware).
2. Validate that the refresh token being revoked belongs to the authenticated user's session.
3. Alternatively, keep logout public but require the refresh token to match a session owned by the user identified by the accompanying access token.

**Files:** `web-api/src/lib.rs` (router), `web-api/src/routes/auth.rs`

---

### FP-HI1: Sanitize hook parameters

**Addresses:** HI-1

**Problem:** Shell command injection via `service_name`, `project_dir`, `compose_file`.

**Plan:**
1. Add a validation function for hook parameters that rejects shell metacharacters: `;`, `&`, `|`, `$`, `` ` ``, `(`, `)`, `{`, `}`, `\n`, `\r`, etc.
2. Apply validation at the API layer (when creating/updating provider configs and software items).
3. Use `shell-words::quote()` or manual quoting when constructing commands.
4. Consider switching from string interpolation to argument arrays passed to `Command::new()` directly, bypassing the shell entirely.

**Files:** `web-api/src/update_hooks.rs`, `web-api/src/routes/provider_configs.rs`, `web-api/src/routes/software_items.rs`

---

### FP-HI2: Wrap merge in transaction

**Addresses:** HI-2

**Problem:** Service merge performs multiple DB operations without a transaction.

**Plan:**
1. Open a `db.begin()` transaction at the start of `merge_service()`.
2. Execute all operations (deactivate source, revoke certs, update target, copy host links) within the transaction.
3. Only `txn.commit()` at the end.
4. On any error, the transaction auto-rolls back, leaving the database in its original state.

**Files:** `web-api/src/routes/services.rs`

---

### FP-HI4: Cap certificate lifetime

**Addresses:** HI-4

**Problem:** No maximum enforcement on agent certificate lifetime.

**Plan:**
1. Add a `MAX_AGENT_CERT_LIFETIME_DAYS` constant (e.g., 730 days / 2 years).
2. In `sign_agent_csr()`, clamp the requested lifetime to the maximum.
3. Log a warning if the requested lifetime exceeds the cap.

**Files:** `controller/src/cert_signer.rs`

---

### FP-HI8: Fix FK cascade inconsistencies

**Addresses:** HI-8, HI-9

**Problem:** Missing and inconsistent foreign key behaviors.

**Plan:**
1. Add a migration that:
   - Adds `ON DELETE CASCADE` to `service_certificates.service_id -> services` (or `RESTRICT` with documentation — but it must be explicit).
   - Adds FK from `service_certificates.ca_fingerprint` to `ca_certificates.fingerprint`.
   - Normalizes `mqtt_leases.tenant_id` to `RESTRICT` (matching all other tenant FKs).
2. Add a unique constraint on `mqtt_leases.mqtt_client_id` to prevent duplicate lease assignments.

**Files:** New migration file.

---

### FP-HI10: Add tenant filter to host agents

**Addresses:** HI-10

**Problem:** `load_host_agents` queries agents without tenant scoping.

**Plan:**
1. Accept `tenant_id` as a parameter in `load_host_agents()`.
2. Join through `service_hosts` to `services` and filter `services.tenant_id = ?`.

**Files:** `web-api/src/routes/hosts.rs`

---

### FP-HI11: Rate limit refresh endpoint

**Addresses:** HI-11

**Problem:** Token refresh has no rate limiting.

**Plan:**
1. Add `/api/v1/auth/refresh` to the `RATE_LIMITS` HashMap in `middleware/rate_limit.rs` with a limit of 10 req/min/IP.
2. Also add `/api/v1/auth/device/approve` with 5 req/min/IP to address ME-5.

**Files:** `web-api/src/middleware/rate_limit.rs`

---

### FP-ME1: Implement refresh token rotation

**Addresses:** ME-1

**Problem:** Stolen refresh tokens remain usable for 7 days.

**Plan:**
1. On each token refresh: revoke the old session, create a new session with a new refresh token.
2. Return the new refresh token in the response.
3. If a revoked refresh token is used (replay detection), revoke ALL sessions for that user as a safety measure.

**Files:** `web-api/src/auth/session.rs`, `web-api/src/routes/auth.rs`

---

### FP-ME2: Check email_verified before auto-link

**Addresses:** ME-2

**Problem:** OIDC auto-link trusts email without verification.

**Plan:**
1. In the `AutoLink` branch, check the `email_verified` claim from the OIDC provider's userinfo/ID token.
2. If `email_verified` is `false` or absent, do NOT auto-link. Instead, require explicit account linking through the existing `AccountLinkStore` flow.

**Files:** `web-api/src/auth/authentication.rs`

---

## Cross-References to Wire Protocol CODEREVIEW

The following findings from `crates/shared/wire/CODEREVIEW.md` are confirmed and not duplicated here:

| Wire ID | This Review | Status |
|---------|-------------|--------|
| A1 (MQTT password in outbox) | CR-2 | Extended to cover at-rest encryption |
| S1 (Argon2 brute-force) | — | Confirmed, fix in wire FP-1 |
| S2 (Anonymous timeout) | HI-5 | Extended with connection cap |
| S4 (Unbounded output) | HI-3 | Extended with ownership validation |
| H2 (Cursor advancement) | HI-7 | Confirmed, fix in wire FP-3 |
| H5 (Lease TOCTOU) | HI-6 | Confirmed, fix in wire FP-8 |
| D1 (Connection dedup) | — | Confirmed, fix in wire FP-16 |
| D2 (Update ownership) | HI-3 | Confirmed, fix in wire FP-17 |
| D3 (Register order) | — | Confirmed, fix in wire FP-18 |
| D4 (Broadcast lock) | — | Confirmed, fix in wire FP-19 |
| D5 (Approval timeout) | — | Confirmed, fix in wire FP-20 |

---

## Architecture Observations

### Strengths

1. **HA readiness is strong.** CA rotation CAS, version-gated CRL polling, settings version checking, and the notification outbox all demonstrate thoughtful multi-instance design. The database is correctly treated as the single source of truth.

2. **Clean separation of concerns.** The controller (orchestration), web-api (routes/middleware), wire (protocol), enrollment (shared identity), and db (entities) crates have clear boundaries.

3. **Comprehensive test infrastructure.** Docker-based reverse proxy integration tests covering Nginx, Traefik, Caddy, HAProxy, and Envoy with real mTLS, CRL, and OCSP scenarios.

4. **Zero-downtime restart.** The `SO_REUSEPORT` + `SIGUSR1` takeover mechanism with scattered restart notifications is well-engineered.

### Areas for Improvement

1. **Key material surface area.** `CaSnapshotData` conflates metadata with secrets. Split recommended (FP-CR3).

2. **main.rs monolith.** The `run()` function is 1100+ lines. Extract into `init_pki()`, `init_oidc()`, `start_background_tasks()`, `graceful_shutdown()`.

3. **Multi-tenancy readiness.** Several tables lack `tenant_id` (`users`, `sessions`, `roles`, `permissions`), and junction tables (`service_hosts`, `host_software_items`) have no cross-tenant protection. The current single-tenant mode masks these gaps.

4. **Missing observability.** No request ID correlation, no distributed tracing, no metrics endpoint.

5. **Inconsistent shutdown.** The CRL manager is `abort()`-ed while all other tasks use `CancellationToken`. The PKI HTTP server has no graceful shutdown.
