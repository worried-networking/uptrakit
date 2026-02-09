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
| **CRITICAL** | 0 | — |
| **HIGH** | 0 | — |
| **MEDIUM** | 7 | Race conditions in leases, N+1 queries, settings desync, CRL gap |
| **LOW** | 16 | Missing validation, minor inconsistencies, defense-in-depth gaps |

---

## 1. Critical Findings

No critical findings remain open.

---

## 2. High Findings

---

## 3. Medium Findings

### ME-7: N+1 query patterns in list endpoints

**Files:** `web-api/src/routes/hosts.rs:87-91`, `software_items.rs:356-374`, `update_history.rs:194-199`

`list_hosts` issues an individual query per host (up to 1000). Same patterns in software items and update history.

### ME-10: Docker Registry auth token URL params not URL-encoded

**File:** `providers/docker-registry/src/auth.rs:69-82`

`service` and `scope` values from the `WWW-Authenticate` header are appended without URL encoding, enabling parameter injection by a malicious registry.

### ME-12: CRL `next_update` creates 24-hour revocation visibility gap

**File:** `controller/src/crl_manager.rs:295`

CRL clients cache until `next_update` (24h). Revoking a certificate has no effect on clients that already cached the old CRL.

### ME-13: MQTT client count limit TOCTOU

**File:** `web-api/src/mqtt_client_store.rs:88-91`

Count-then-insert pattern allows concurrent requests to exceed the configured maximum.

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

> **Open Fix Plans** — The remaining fix plans below cover the unresolved findings. Additionally, 1 issue (ME-17) has a fix plan in the wire protocol CODEREVIEW.

---

### FP-ME4: Rate limit fails closed on database error (TOP 10 — #18)

**Addresses:** ME-4 (Medium — rate limiting completely disabled during DB outage)

**Problem:** The rate limit middleware (`middleware/rate_limit.rs:96-99`) catches database errors and allows the request through (fail-open). During a database outage, all rate limiting is silently disabled, allowing unlimited brute-force on auth endpoints.

**Current code:**
```rust
Err(e) => {
    tracing::error!("rate limit check failed: {e}");
    next.run(req).await  // ← fail-open: allows ALL requests
}
```

**Detailed implementation plan:**

1. **Add an in-memory fallback rate limiter** that activates when the DB check fails. Use a simple `DashMap<String, (u32, Instant)>` in `AppState`:
   ```rust
   pub struct InMemoryRateLimiter {
       buckets: DashMap<String, (u32, Instant)>,
   }

   impl InMemoryRateLimiter {
       pub fn check(&self, key: &str, max: u32, window_secs: u64) -> RateLimitOutcome {
           let now = Instant::now();
           let window = Duration::from_secs(window_secs);
           let mut entry = self.buckets.entry(key.to_string()).or_insert((0, now));
           if now.duration_since(entry.1) > window {
               // Reset window
               *entry = (1, now);
               return RateLimitOutcome::Allowed;
           }
           if entry.0 >= max {
               return RateLimitOutcome::Limited { retry_after_secs: /* ... */ };
           }
           entry.0 += 1;
           RateLimitOutcome::Allowed
       }
   }
   ```

2. **Update the middleware error path:**
   ```rust
   Err(e) => {
       tracing::error!("rate limit DB check failed, using in-memory fallback: {e}");
       match state.fallback_rate_limiter.check(&key, limit.max_requests, limit.window_secs) {
           RateLimitOutcome::Allowed => next.run(req).await,
           RateLimitOutcome::Limited { retry_after_secs } => {
               // Return 429
           }
       }
   }
   ```

3. **Add periodic cleanup** of stale entries (every 5 minutes, remove entries older than 2x window).

4. **The in-memory limiter is per-instance** (not HA-safe), but that's acceptable as a degraded-mode fallback — it still prevents single-instance brute-force, which is better than no limiting at all.

**Files to modify:**
- `crates/ui/web-api/src/middleware/rate_limit.rs` — use fallback on error
- `crates/ui/web-api/src/lib.rs` — add `InMemoryRateLimiter` to `AppState`

**Testing:**
- Unit test: DB error triggers fallback, requests still rate-limited
- Unit test: fallback limiter correctly tracks counts and windows
- Unit test: fallback limiter resets after window expires

---

### FP-ME5: Require permission for device flow approval (TOP 10 — #19)

**Addresses:** ME-5 (Medium — any authenticated user can approve device flows)

**Problem:** The `device_auth_approve()` handler in `routes/device_auth.rs:183-215` only checks that the caller is authenticated. Any user — including those with only `ViewAgents` permission — can approve device enrollment flows. This violates least-privilege.

**Current code:**
```rust
pub async fn device_auth_approve(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthenticatedUser>,
    Json(req): Json<DeviceAuthApproveRequest>,
) -> Response {
    // ❌ No permission check — any authenticated user can approve
    let normalized = req.user_code.replace('-', "").to_uppercase();
    state.device_flow_store.approve(&normalized, auth_user.user_id).await
    // ...
}
```

**Detailed implementation plan:**

1. **Add permission check at the top of the handler:**
   ```rust
   if !auth_user.has_permission(Permission::ManageAgents) {
       return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
   }
   ```

2. **This follows the existing pattern** used by every other management endpoint (e.g., `approve_service`, `create_provider_config`, etc.).

**Files to modify:**
- `crates/ui/web-api/src/routes/device_auth.rs` — add permission check

**Testing:**
- Unit test: user without `ManageAgents` returns 403
- Unit test: user with `ManageAgents` can approve

---

### FP-ME12: Make CRL validity configurable (TOP 10 — #23)

**Addresses:** ME-12 (Medium — 24-hour CRL caching gap for revoked certificates)

**Problem:** `crl_manager.rs:295` hardcodes `next_update: now + Duration::hours(24)`. CRL clients cache the CRL until `next_update`. Revoking a certificate has no effect on clients that already cached the old CRL — they trust the revoked cert for up to 24 hours.

**Current code (`crl_manager.rs:287-295`):**
```rust
let params = CertificateRevocationListParams {
    this_update: now,
    next_update: now + time::Duration::hours(24),  // ← hardcoded
    crl_number: SerialNumber::from(crl_number),
    // ...
};
```

**Detailed implementation plan:**

1. **Add `crl_validity_hours` to `CrlManagerConfig`:**
   ```rust
   pub struct CrlManagerConfig {
       // ... existing fields ...
       pub crl_validity_hours: u16,  // default: 24, minimum: 1
   }
   ```

2. **Use the configurable value in `sign_crl()`:**
   ```rust
   let params = CertificateRevocationListParams {
       this_update: now,
       next_update: now + time::Duration::hours(config.crl_validity_hours as i64),
       // ...
   };
   ```

3. **Expose as a setting** in the settings API so operators can tune it:
   - Short validity (e.g., 1-4 hours) = faster revocation propagation, more frequent CRL fetches
   - Long validity (e.g., 24-48 hours) = fewer fetches, slower revocation

4. **Document the trade-off** in the setting description and SECURITY.md.

5. **As a complementary measure**, ensure the CRL rebuild task runs more frequently than the validity period (e.g., every validity/2 hours) so fresh CRLs are always available before the old one expires.

**Files to modify:**
- `crates/core/controller/src/crl_manager.rs` — accept configurable validity
- `crates/core/controller/src/main.rs` — pass config value
- `crates/ui/web-api/src/settings.rs` — add `crl_validity_hours` setting

**Testing:**
- Unit test: CRL `next_update` matches configured value
- Unit test: default is 24 hours (backwards compatible)

---

### FP-ME7: Eliminate N+1 query patterns in list endpoints (TOP 10 — #24)

**Addresses:** ME-7 (Medium — up to 2000+ queries per list request)

**Problem:** Three list endpoints issue individual queries per result item:

1. **`list_hosts`** (`hosts.rs:87-91`): `load_host_agents()` per host = 1 + N queries per host
2. **`list_software_items`** (`software_items.rs:356-374`): `find_active_provider_config()` + `count_linked_hosts()` per item = 2N queries
3. **`list_update_history`** (`update_history.rs:194-199`): `resolve_host_name()` + `resolve_software_item_name()` per record = 2N queries

With max pagination of 1000 items, this means up to 2000+ individual DB round-trips per request.

**Detailed implementation plan:**

1. **`list_hosts` — batch-load agents with a single JOIN query:**
   ```rust
   // Instead of: for h in hosts { load_host_agents(h.id) }
   // Do: single query to load all agents for all host IDs

   let host_ids: Vec<Uuid> = hosts.iter().map(|h| h.id).collect();
   let all_agents = Service::find()
       .inner_join(ServiceHost)
       .filter(service_host::Column::HostId.is_in(host_ids))
       .filter(service::Column::TenantId.eq(tenant.tenant_id))
       .filter(service::Column::DeactivatedAt.is_null())
       .all(&state.db).await?;

   // Group by host_id
   let agents_by_host: HashMap<Uuid, Vec<_>> = /* group all_agents by host_id */;

   let items = hosts.into_iter().map(|h| {
       let agents = agents_by_host.get(&h.id).cloned().unwrap_or_default();
       host_to_response(h, agents)
   }).collect();
   ```

2. **`list_software_items` — batch-load provider configs and host counts:**
   ```rust
   // Batch load provider configs
   let config_ids: Vec<Uuid> = items.iter().map(|i| i.provider_config_id).collect();
   let configs: HashMap<Uuid, _> = ProviderConfig::find()
       .filter(provider_config::Column::Id.is_in(config_ids))
       .all(&state.db).await?
       .into_iter().map(|c| (c.id, c)).collect();

   // Batch count hosts per software item using GROUP BY
   let host_counts = /* SELECT software_item_id, COUNT(*) FROM host_software_items
                        WHERE software_item_id IN (?) GROUP BY software_item_id */;
   ```

3. **`list_update_history` — batch-load host names and software item names:**
   ```rust
   let host_ids: Vec<Uuid> = records.iter().map(|r| r.host_id).collect();
   let si_ids: Vec<Uuid> = records.iter().map(|r| r.software_item_id).collect();

   let host_names: HashMap<Uuid, String> = Host::find()
       .filter(host::Column::Id.is_in(host_ids))
       .all(&state.db).await?
       .into_iter().map(|h| (h.id, h.hostname)).collect();

   let si_names: HashMap<Uuid, String> = SoftwareItem::find()
       .filter(software_item::Column::Id.is_in(si_ids))
       .all(&state.db).await?
       .into_iter().map(|s| (s.id, s.name)).collect();
   ```

**Files to modify:**
- `crates/ui/web-api/src/routes/hosts.rs` — batch agent loading
- `crates/ui/web-api/src/routes/software_items.rs` — batch config + host count
- `crates/ui/web-api/src/routes/update_history.rs` — batch name resolution

**Testing:**
- Existing endpoint tests should pass with same results
- Performance test: list with 100 items → verify ≤5 queries (vs 200+ before)

---

### FP-ME13: Fix MQTT client count TOCTOU (TOP 10 — #25)

**Addresses:** ME-13 (Medium — concurrent requests can exceed MQTT client limit)

**Problem:** `create_mqtt_client()` in `mqtt_client_store.rs:88-91` uses a count-then-insert pattern. Two concurrent requests can both read `count = max - 1`, both pass the check, and both insert — exceeding the configured maximum by one (or more with higher concurrency).

**Current code:**
```rust
let count = count_mqtt_clients(db, tenant_id).await?;    // CHECK
if count >= u64::from(max_clients) {
    return Err(report!(MqttClientError::LimitReached(max_clients)));
}
// ... build model ...
model.insert(db).await.context_to()                        // ACT
```

**Detailed implementation plan:**

1. **Wrap check + insert in a serializable transaction:**
   ```rust
   pub async fn create_mqtt_client(params: CreateMqttClientParams<'_>) -> Result<mqtt_client::Model> {
       let txn = params.db.begin_with_config(
           Some(sea_orm::IsolationLevel::Serializable), None
       ).await.context_to()?;

       let count = count_mqtt_clients(&txn, params.tenant_id).await?;
       if count >= u64::from(params.max_clients) {
           return Err(report!(MqttClientError::LimitReached(params.max_clients)));
       }

       let model = mqtt_client::ActiveModel { /* ... */ };
       let result = model.insert(&txn).await.context_to()?;

       txn.commit().await.context_to()?;
       Ok(result)
   }
   ```

2. **Handle serialization conflict:** If two transactions race, the second will fail at commit with a serialization error. Map this to an appropriate error (retry or 409 Conflict).

3. **Alternative (simpler, database-level):** If the `mqtt_leases` table already has a unique constraint on `mqtt_client_id`, a partial unique index on `mqtt_clients(tenant_id)` with a condition `WHERE deactivated_at IS NULL` could enforce limits at the DB level. However, partial unique indexes don't directly enforce count limits — the serializable transaction approach is more reliable.

**Files to modify:**
- `crates/ui/web-api/src/mqtt_client_store.rs` — wrap in serializable transaction

**Testing:**
- Concurrent test: spawn N tasks creating clients simultaneously → total never exceeds limit
- Unit test: count at limit returns `LimitReached` error

---

### FP-ME10: URL-encode Docker Registry auth parameters (#26)

**Addresses:** ME-10 (Medium — parameter injection via malicious registry)

**Problem:** `providers/docker-registry/src/auth.rs:69-82` constructs the token endpoint URL by appending `service` and `scope` values from the `WWW-Authenticate` header using raw string concatenation. These values originate from an untrusted registry response and are not URL-encoded. A malicious registry can include `&`, `=`, or other query-string metacharacters in `scope` (e.g., `repository:lib/nginx:pull&admin=true`) to inject extra parameters into the token request.

**Current code:**
```rust
url.push_str("service=");
url.push_str(svc);        // ← raw, unencoded
// ...
url.push_str("scope=");
url.push_str(sc);          // ← raw, unencoded
```

**Detailed implementation plan:**

1. **URL-encode both parameters** using percent-encoding:
   ```rust
   use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};

   if let Some(svc) = service {
       url.push(if has_query { '&' } else { '?' });
       has_query = true;
       url.push_str("service=");
       url.push_str(&utf8_percent_encode(svc, NON_ALPHANUMERIC).to_string());
   }
   if let Some(sc) = scope {
       url.push(if has_query { '&' } else { '?' });
       url.push_str("scope=");
       url.push_str(&utf8_percent_encode(sc, NON_ALPHANUMERIC).to_string());
   }
   ```

2. **Alternatively, use `url::Url` builder** which handles encoding automatically:
   ```rust
   let mut parsed = url::Url::parse(&realm)?;
   if let Some(svc) = service {
       parsed.query_pairs_mut().append_pair("service", svc);
   }
   if let Some(sc) = scope {
       parsed.query_pairs_mut().append_pair("scope", sc);
   }
   let url = parsed.to_string();
   ```

**Files to modify:**
- `crates/providers/docker-registry/src/auth.rs` — use `url::Url` or percent-encoding
- `crates/providers/docker-registry/Cargo.toml` — add `url` or `percent-encoding` dependency

**Testing:**
- Unit test: scope containing `&extra=1` is properly encoded, not split into separate params
- Unit test: standard Docker Hub `service`/`scope` values encode correctly

---

### FP-ME16: Invalidate registration settings cache across HA (#28)

**Addresses:** ME-16 (Medium — stale registration mode on other instances)

**Problem:** `RegistrationSettings` is cached in-memory in each controller instance's `AppState`. When `complete_initial_setup()` or `update()` modifies registration settings, the change is written to the DB and the local in-memory copy is updated, but other instances continue serving stale values. An instance running with `mode = Invite` could allow registrations even after another instance switched to `mode = Closed`.

**Current code pattern:**
```rust
pub async fn complete_initial_setup(&mut self, db: &DatabaseConnection, tenant_id: Uuid) -> Result<()> {
    upsert_setting(db, tenant_id, SettingKey::RegistrationMode, "Closed").await?;
    self.mode = RegistrationMode::Closed;  // ← only updates THIS instance
}
```

**Detailed implementation plan:**

1. **Bump the settings version** when registration settings change. This is the existing HA mechanism — all instances poll `settings_version` and reload when it changes:
   ```rust
   pub async fn complete_initial_setup(&mut self, db: &DatabaseConnection, tenant_id: Uuid) -> Result<()> {
       upsert_setting(db, tenant_id, SettingKey::RegistrationMode, "Closed").await?;
       delete_setting(db, tenant_id, SettingKey::RegistrationTokenHash).await?;
       delete_setting(db, tenant_id, SettingKey::RegistrationRequireTokenForOidc).await?;

       // Bump version so other instances reload
       crate::settings_store::bump_settings_version(db, tenant_id, false).await?;

       // Update local cache
       self.mode = RegistrationMode::Closed;
       self.token_hash = None;
       self.require_token_for_oidc = false;
       Ok(())
   }
   ```

2. **Apply the same pattern to `update()`** — it should also bump the settings version after DB writes.

3. **Verify the settings reload task** (`reload_from_db()`) correctly reloads registration settings. From the ME-8 analysis, `reload_from_db()` does update `registration` — so bumping the version is sufficient to trigger cross-instance refresh.

**Files to modify:**
- `crates/ui/web-api/src/auth/registration.rs` — bump settings version in `complete_initial_setup()` and `update()`

**Testing:**
- Integration test: update registration mode on instance A → instance B sees new mode after version poll

---

### FP-ME18: Optimize update history tenant scoping query (#29)

**Addresses:** ME-18 (Medium — loads all host models just to extract UUIDs)

**Problem:** `tenant_host_ids()` in `update_history.rs:61-70` loads full `Host` models (`SELECT * FROM hosts WHERE tenant_id = ?`) just to extract the `id` column. For tenants with many hosts, this transfers unnecessary data (hostname, IP, timestamps, etc.) and allocates full model objects.

**Current code:**
```rust
async fn tenant_host_ids(db: &DatabaseConnection, tenant_id: Uuid) -> Result<Vec<Uuid>, DbErr> {
    let hosts = Host::find()
        .filter(host::Column::TenantId.eq(tenant_id))
        .all(db).await?;                           // ← loads all columns
    Ok(hosts.into_iter().map(|h| h.id).collect())  // ← only uses id
}
```

**Detailed implementation plan:**

1. **Use `select_only()` + `column()` to fetch only IDs:**
   ```rust
   async fn tenant_host_ids(db: &DatabaseConnection, tenant_id: Uuid) -> Result<Vec<Uuid>, DbErr> {
       let ids: Vec<Uuid> = Host::find()
           .filter(host::Column::TenantId.eq(tenant_id))
           .filter(host::Column::DeactivatedAt.is_null())
           .select_only()
           .column(host::Column::Id)
           .into_values::<Uuid, host::Column>()
           .all(db)
           .await?;
       Ok(ids)
   }
   ```

2. **Consider using a subquery** instead of materializing IDs at all. The calling code uses `host_ids` in an `IN()` clause — this can be a single query:
   ```rust
   // Instead of:
   //   let host_ids = tenant_host_ids(db, tenant_id).await?;
   //   UpdateHistory::find().filter(HostId.is_in(host_ids))
   // Use:
   UpdateHistory::find()
       .filter(update_history::Column::TenantId.eq(tenant_id))
   ```
   (If `update_history` has a `tenant_id` column, use it directly instead of the host-based indirection.)

**Files to modify:**
- `crates/ui/web-api/src/routes/update_history.rs` — use `select_only()` or subquery

**Testing:**
- Existing endpoint tests should return identical results
- Verify SQL query only selects `id` column (check query log)

---

### FP-LO2: Add password maximum length check (#30)

**Addresses:** LO-2 (Low — Argon2 CPU exhaustion via extremely long passwords)

**Problem:** Registration (`auth.rs:49`) validates `password.len() >= 8` but has no maximum. The password is passed to `Argon2::hash_password()` (with OWASP-recommended parameters: 19 MiB memory, 2 iterations). Extremely long passwords (e.g., 1 MB) cause significant CPU and memory consumption. An attacker can send repeated registration or login requests with massive passwords to exhaust server resources.

**Current validation:**
```rust
if req.password.len() < 8 {
    return error_response(StatusCode::BAD_REQUEST, "Password must be at least 8 characters");
}
// ❌ No maximum check → password passed directly to Argon2
```

**Detailed implementation plan:**

1. **Add a maximum length check** (128 characters is generous — NIST SP 800-63B recommends accepting at least 64):
   ```rust
   if req.password.len() < 8 {
       return error_response(StatusCode::BAD_REQUEST, "Password must be at least 8 characters");
   }
   if req.password.len() > 128 {
       return error_response(StatusCode::BAD_REQUEST, "Password must not exceed 128 characters");
   }
   ```

2. **Apply to both registration and login endpoints.** The login path should also reject oversized passwords before hashing:
   ```rust
   // In login handler, before password::verify_password():
   if req.password.len() > 128 {
       return error_response(StatusCode::UNAUTHORIZED, "Invalid email or password");
   }
   ```
   (Use the generic "invalid credentials" message on login to avoid information leakage.)

3. **Apply to password change/reset** endpoints if they exist.

**Files to modify:**
- `crates/ui/web-api/src/routes/auth.rs` — add max check in register + login handlers

**Testing:**
- Unit test: 129-char password returns 400 on registration
- Unit test: 128-char password succeeds
- Unit test: oversized password on login returns 401

---

### FP-LO10: Add missing indexes on hot query paths (#31)

**Addresses:** LO-10 (Low — missing indexes cause full table scans on frequent queries)

**Problem:** Several frequently-queried columns lack indexes:
- `user_roles`: No index on `(user_id)` — used on every authenticated request to resolve permissions
- `sessions(revoked_at)`: No index — used for active session queries and cleanup
- `service_certificates(ca_fingerprint)`: No index — needed for FK lookups after HI-9 fix

The `user_roles` table has a composite PK `(tenant_id, user_id, role_id)`, which works for lookups by `tenant_id` prefix but is suboptimal for queries filtering only by `user_id`.

**Detailed implementation plan:**

1. **Modify the relevant migration** to add indexes (since existing migrations can be modified per project rules):

   In `m20260129_000003_create_rbac.rs`, add after table creation:
   ```rust
   // Index for permission lookups by user_id (used on every authenticated request)
   manager.create_index(
       Index::create()
           .name("idx_user_roles_user_id")
           .table(UserRoles::Table)
           .col(UserRoles::UserId)
           .to_owned(),
   ).await?;
   ```

   In `m20260130_000009_jwt_refresh_tokens.rs` (or the sessions migration), add:
   ```rust
   // Index for active session queries (revoked_at IS NULL)
   manager.create_index(
       Index::create()
           .name("idx_sessions_revoked_at")
           .table(Sessions::Table)
           .col(Sessions::RevokedAt)
           .to_owned(),
   ).await?;
   ```

   In `m20260129_000008_create_agent_certificates.rs`, add:
   ```rust
   // Index for ca_fingerprint FK lookups (supports HI-9 FK constraint)
   manager.create_index(
       Index::create()
           .name("idx_service_certificates_ca_fingerprint")
           .table(ServiceCertificates::Table)
           .col(ServiceCertificates::CaFingerprint)
           .to_owned(),
   ).await?;
   ```

**Files to modify:**
- `crates/core/controller/src/migration/m20260129_000003_create_rbac.rs`
- `crates/core/controller/src/migration/m20260130_000009_jwt_refresh_tokens.rs`
- `crates/core/controller/src/migration/m20260129_000008_create_agent_certificates.rs`

**Testing:**
- Migration runs cleanly on fresh database
- Verify indexes exist via `PRAGMA index_list` (SQLite) or `\di` (PostgreSQL)

---

### FP-LO4: Add expiration to API tokens (#32)

**Addresses:** LO-4 (Low — API tokens valid indefinitely until manually revoked)

**Problem:** The `api_tokens` table has no `expires_at` column. Tokens remain valid forever unless explicitly revoked. If a token is forgotten or its owner leaves the organization, it provides permanent access.

**Current entity (no `expires_at`):**
```rust
pub struct Model {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub token_hash: String,
    pub created_at: OffsetDateTime,
    pub last_used_at: Option<OffsetDateTime>,
    pub revoked_at: Option<OffsetDateTime>,
    // ❌ no expires_at
}
```

**Detailed implementation plan:**

1. **Add `expires_at` column** to the `api_tokens` table via migration:
   ```rust
   // New migration (or modify existing)
   manager.alter_table(
       Table::alter()
           .table(ApiTokens::Table)
           .add_column(ColumnDef::new(ApiTokens::ExpiresAt).timestamp_with_time_zone().null())
           .to_owned(),
   ).await?;
   ```
   `NULL` means "never expires" (backwards compatible with existing tokens).

2. **Update the entity model:**
   ```rust
   pub expires_at: Option<OffsetDateTime>,
   ```

3. **Update token creation API** to accept optional `expires_in_days`:
   ```rust
   pub struct CreateApiTokenRequest {
       pub name: String,
       pub expires_in_days: Option<u16>,  // None = no expiry
   }
   ```

4. **Check expiry during token verification** in `api_token.rs`:
   ```rust
   if let Some(expires_at) = token_model.expires_at {
       if expires_at <= OffsetDateTime::now_utc() {
           return Err(report!(AuthError::TokenExpired));
       }
   }
   ```

5. **Add `expires_at` to API token list response** so users can see when tokens expire.

**Files to modify:**
- New migration or modify existing `api_tokens` migration
- `crates/shared/db/src/entity/api_token.rs` — add field
- `crates/ui/web-api/src/auth/api_token.rs` — check expiry
- `crates/ui/web-api/src/routes/api_tokens.rs` — accept `expires_in_days`
- `crates/shared/web-api-types/src/api_tokens.rs` — update request/response types

**Testing:**
- Unit test: expired token returns 401
- Unit test: token with `expires_at = None` remains valid
- Unit test: token within expiry window works

---

### FP-LO1: Add email validation on registration (#33)

**Addresses:** LO-1 (Low — no email format validation accepts garbage)

**Problem:** The registration handler (`auth.rs:39-188`) accepts any string as an email — empty strings, strings without `@`, and absurdly long strings. This can cause issues with email-based lookups, OIDC auto-linking, and notification features.

**Current code (no email validation):**
```rust
pub async fn register(State(state): State<Arc<AppState>>, Json(req): Json<RegisterRequest>) -> Response {
    // Only password is validated:
    if req.password.len() < 8 { return error_response(...); }
    // ❌ No email validation
    // ...
    let existing = User::find().filter(user::Column::Email.eq(&req.email)).one(&state.db).await;
}
```

**Detailed implementation plan:**

1. **Add basic email validation** (no need for a heavy regex — basic structural check suffices):
   ```rust
   let email = req.email.trim().to_lowercase();  // normalize
   if email.is_empty() || email.len() > 254 {
       return error_response(StatusCode::BAD_REQUEST, "Invalid email address");
   }
   if !email.contains('@') || email.starts_with('@') || email.ends_with('@') {
       return error_response(StatusCode::BAD_REQUEST, "Invalid email address");
   }
   // Ensure exactly one @ and domain has at least one dot
   let parts: Vec<&str> = email.splitn(2, '@').collect();
   if parts.len() != 2 || parts[0].is_empty() || !parts[1].contains('.') {
       return error_response(StatusCode::BAD_REQUEST, "Invalid email address");
   }
   ```

2. **Normalize email before storage** — trim whitespace and lowercase (also fixes LO-5 partially):
   ```rust
   let email = req.email.trim().to_lowercase();
   ```

3. **Apply the same normalization in the login handler** and everywhere email is used for lookup.

4. **Apply in OIDC flow** — normalize the email from OIDC claims before lookup/storage.

**Files to modify:**
- `crates/ui/web-api/src/routes/auth.rs` — validate + normalize on register and login
- `crates/ui/web-api/src/auth/authentication.rs` — normalize in OIDC resolution

**Testing:**
- Unit test: empty email rejected
- Unit test: email without `@` rejected
- Unit test: email > 254 chars rejected
- Unit test: `User@Example.COM` normalized to `user@example.com`
- Unit test: valid email like `user@example.com` accepted

---

### FP-LO5: Case-insensitive email handling (#34)

**Addresses:** LO-5 (Low — `User@Example.com` and `user@example.com` treated as different users)

**Problem:** All email lookups use case-sensitive `Column::Email.eq(email)`. Database collation may or may not be case-sensitive depending on the backend (SQLite default is case-sensitive for non-ASCII). Users can register `Admin@Company.com` and `admin@company.com` as separate accounts due to the unique constraint allowing different cases.

**Current code (case-sensitive):**
```rust
// Registration:
User::find().filter(user::Column::Email.eq(&req.email)).one(db).await
// Login:
User::find().filter(user::Column::Email.eq(&req.email)).one(db).await
// OIDC:
User::find().filter(user::Column::Email.eq(email)).one(db).await
```

**Detailed implementation plan:**

1. **Normalize to lowercase at all entry points** (complements FP-LO1):
   - Registration: `let email = req.email.trim().to_lowercase();`
   - Login: `let email = req.email.trim().to_lowercase();`
   - OIDC callback: `let email = claims.email().map(|e| e.to_lowercase())`

2. **Add a migration to lowercase all existing emails:**
   ```rust
   // For SQLite:
   db.execute_unprepared("UPDATE users SET email = LOWER(TRIM(email))").await?;
   // For PostgreSQL:
   db.execute_unprepared("UPDATE users SET email = LOWER(TRIM(email))").await?;
   ```

3. **Combined with FP-LO1**, this ensures all emails are stored and compared in lowercase, making the existing case-sensitive unique constraint function correctly.

**Files to modify:**
- `crates/ui/web-api/src/routes/auth.rs` — normalize email in register + login
- `crates/ui/web-api/src/auth/authentication.rs` — normalize email in OIDC flow
- New migration to lowercase existing data

**Testing:**
- Unit test: registering `User@Example.com` stores as `user@example.com`
- Unit test: login with `USER@EXAMPLE.COM` finds `user@example.com`
- Unit test: OIDC auto-link with different case still finds existing user

---

### FP-LO11: Add unique constraint on available versions (#35)

**Addresses:** LO-11 (Low — duplicate version strings per software item)

**Problem:** The `available_versions` table (migration 015) has no unique constraint on `(software_item_id, version)`. Multiple rows with the same version can be inserted for the same software item, causing duplicate entries in version listings and ambiguity when resolving "latest version."

**Current table:** Has PK on `id`, FK on `software_item_id`, index on `software_item_id`, CHECK constraint on `version OR release_date` — but NO uniqueness on `(software_item_id, version)`.

**Detailed implementation plan:**

1. **Modify migration 015** to add a unique index:
   ```rust
   manager.create_index(
       Index::create()
           .name("uq_available_versions_item_version")
           .table(AvailableVersions::Table)
           .col(AvailableVersions::SoftwareItemId)
           .col(AvailableVersions::Version)
           .unique()
           .to_owned(),
   ).await?;
   ```

2. **Update the version upsert logic** (if any) to use `INSERT ... ON CONFLICT` instead of blind insert. When a provider fetches releases and a version already exists, it should update `release_date`/`release_notes`/`extra` rather than creating a duplicate.

3. **Handle the `NULL` version case:** Since `version` is nullable (the CHECK allows `release_date` instead), the unique index should only apply when `version IS NOT NULL`. Use a partial/conditional unique index:
   ```sql
   -- SQLite: CREATE UNIQUE INDEX ... WHERE version IS NOT NULL
   -- PostgreSQL: same syntax
   ```
   In SeaORM, this may require a raw SQL migration statement.

**Files to modify:**
- `crates/core/controller/src/migration/m20260201_000015_create_software_items.rs` — add unique index
- Version store/provider logic — use upsert instead of insert

**Testing:**
- Migration runs cleanly
- Inserting duplicate `(software_item_id, version)` fails with constraint error
- Upsert correctly updates existing version metadata

---

### FP-LO3: Add API token name length validation (#36)

**Addresses:** LO-3 (Low — unbounded token name)

**Problem:** `CreateApiTokenRequest` accepts a plain `String` for `name` with no length validation at any layer — request struct, route handler, service, or database schema. An attacker or misbehaving client can submit an arbitrarily long name, wasting storage and potentially causing display issues.

**Current code path:**
- `web-api-types/src/api_tokens.rs:5-7` — `CreateApiTokenRequest { name: String }` with no constraints
- `routes/api_tokens.rs:36` — passes `req.name` directly to service
- `auth/api_token.rs:51` — `Set(name.to_string())` with no validation
- Migration 010, line 22 — `string(Name)` unbounded column

**Detailed implementation plan:**

1. **Add validation in the route handler** (`routes/api_tokens.rs`):
   ```rust
   pub async fn create_api_token(
       State(state): State<Arc<AppState>>,
       axum::Extension(auth_user): axum::Extension<AuthenticatedUser>,
       Json(req): Json<CreateApiTokenRequest>,
   ) -> Response {
       let name = req.name.trim();
       if name.is_empty() || name.len() > 255 {
           return error_response(
               StatusCode::BAD_REQUEST,
               "Token name must be between 1 and 255 characters",
           );
       }
       // ... proceed with trimmed name
   }
   ```

2. **Constrain the database column** in migration 010:
   ```rust
   .col(ColumnDef::new(ApiTokens::Name).string_len(255).not_null())
   ```

3. **Add the same validation to token rename** if such an endpoint exists.

**Files to modify:**
- `crates/ui/web-api/src/routes/api_tokens.rs` — add length check before service call
- `crates/core/controller/src/migration/m20260131_000010_create_api_tokens.rs` — constrain column to 255 chars

**Testing:**
- Empty name → 400
- 256-char name → 400
- 1-char and 255-char names → 201
- Whitespace-only name → 400

---

### FP-LO6: Add JWT audience and issuer claim validation (#37)

**Addresses:** LO-6 (Low — no `aud`/`iss` in JWT)

**Problem:** `AccessTokenClaims` has no `aud` (audience) or `iss` (issuer) fields. `Validation::default()` only checks signature and expiration. In a multi-service deployment sharing signing keys, tokens from one service could be accepted by another.

**Current code:**
- `auth/jwt.rs:16-26` — struct missing `aud`/`iss` fields
- `auth/jwt.rs:94-102` — token created without audience/issuer
- `auth/jwt.rs:113` — `Validation::default()` with no audience/issuer checks

**Detailed implementation plan:**

1. **Define constants** in `jwt.rs`:
   ```rust
   const JWT_ISSUER: &str = "uptrakit-controller";
   const JWT_AUDIENCE: &str = "uptrakit-api";
   ```

2. **Add fields to `AccessTokenClaims`:**
   ```rust
   pub struct AccessTokenClaims {
       pub sub: String,
       pub jti: String,
       pub iss: String,
       pub aud: String,
       pub permissions: Vec<Permission>,
       pub auth_method: String,
       #[serde(skip_serializing_if = "Option::is_none")]
       pub oidc_provider_id: Option<String>,
       pub iat: i64,
       pub exp: i64,
   }
   ```

3. **Populate on creation** in `create_access_token()`:
   ```rust
   let claims = AccessTokenClaims {
       iss: JWT_ISSUER.to_string(),
       aud: JWT_AUDIENCE.to_string(),
       // ... existing fields
   };
   ```

4. **Validate on decode** in `decode_access_token()`:
   ```rust
   let mut validation = Validation::default();
   validation.set_issuer(&[JWT_ISSUER]);
   validation.set_audience(&[JWT_AUDIENCE]);
   ```

5. **Backwards compatibility:** Existing tokens lack `iss`/`aud`. Since access tokens expire in 15 minutes (`ACCESS_TOKEN_EXPIRY_SECS = 900`), all tokens will rotate naturally. No migration needed — just deploy and wait 15 minutes.

**Files to modify:**
- `crates/ui/web-api/src/auth/jwt.rs` — add fields, constants, validation

**Testing:**
- Newly created tokens contain `iss` and `aud` claims
- Token with wrong issuer → rejected
- Token with wrong audience → rejected
- Token with correct claims → accepted

---

### FP-LO7: Throttle `last_used_at` writes for API tokens (#38)

**Addresses:** LO-7 (Low — DB write on every API token use)

**Problem:** Every authenticated API request using a token triggers a synchronous `UPDATE` to `last_used_at` in `verify_token()` (line 121-126 of `api_token.rs`). Under heavy API token usage, this creates write amplification with one DB write per request.

**Current code:** `auth/api_token.rs:121-126` — `model.update(&self.db).await` on every call to `verify_token()`.

**Detailed implementation plan:**

1. **Add time-based throttle** in `verify_token()` — only update if the last recorded timestamp is older than a threshold (e.g., 5 minutes):
   ```rust
   pub async fn verify_token(&self, plaintext: &str) -> Result<(uuid::Uuid, uuid::Uuid)> {
       let token_hash = hash_token(plaintext);

       let token = ApiToken::find()
           .filter(api_token::Column::TokenHash.eq(token_hash))
           .one(&self.db)
           .await
           .context_to()?
           .ok_or_else(|| report!(AuthError::ApiTokenNotFound))?;

       if token.revoked_at.is_some() {
           return Err(report!(AuthError::ApiTokenRevoked));
       }

       let token_id = token.id;
       let user_id = token.user_id;

       // Only update last_used_at if stale (>5 minutes old)
       let now = OffsetDateTime::now_utc();
       let should_update = token
           .last_used_at
           .is_none_or(|last| (now - last).whole_minutes() >= 5);

       if should_update {
           let mut model: api_token::ActiveModel = token.into();
           model.last_used_at = Set(Some(now));
           model.update(&self.db).await.context_to()?;
       }

       Ok((user_id, token_id))
   }
   ```

2. This reduces writes from once-per-request to at most once-per-5-minutes per token, while still providing useful "last used" information.

**Files to modify:**
- `crates/ui/web-api/src/auth/api_token.rs` — add staleness check around the `last_used_at` update

**Testing:**
- First use of a token → `last_used_at` is set
- Second use within 5 minutes → `last_used_at` unchanged
- Use after 5 minutes → `last_used_at` updated
- Revoked token → still rejected regardless of throttle

---

### FP-LO8: Clean up revoked sessions and schedule cleanup (#39)

**Addresses:** LO-8 (Low — revoked sessions accumulate)

**Problem:** `cleanup_expired_sessions()` in `session.rs:125-136` only deletes sessions where `expires_at < now()`. Revoked sessions (where `revoked_at IS NOT NULL`) are never cleaned up. Additionally, this function is **never called** in production — it's dead code (only used in tests).

**Current code:**
- `session.rs:125-136` — cleanup only checks `expires_at`
- No caller anywhere in production code

**Detailed implementation plan:**

1. **Extend cleanup to include revoked sessions:**
   ```rust
   pub async fn cleanup_expired_sessions(&self) -> Result<u64> {
       let now = OffsetDateTime::now_utc();

       let result = Session::delete_many()
           .filter(
               Condition::any()
                   .add(session::Column::ExpiresAt.lt(now))
                   .add(session::Column::RevokedAt.is_not_null()),
           )
           .exec(&self.db)
           .await
           .context_to()?;

       Ok(result.rows_affected)
   }
   ```

2. **Schedule periodic cleanup** — add a background task in the controller startup (similar to `event_poller`). In `main.rs` or the app initialization:
   ```rust
   // Spawn session cleanup task
   tokio::spawn(async move {
       let session_service = SessionService::new(db.clone());
       let mut interval = tokio::time::interval(Duration::from_secs(3600)); // 1 hour
       loop {
           interval.tick().await;
           match session_service.cleanup_expired_sessions().await {
               Ok(count) if count > 0 => {
                   tracing::info!(deleted = count, "cleaned up expired/revoked sessions");
               }
               Err(e) => {
                   tracing::warn!(error = %e, "session cleanup failed");
               }
               _ => {}
           }
       }
   });
   ```

**Files to modify:**
- `crates/ui/web-api/src/auth/session.rs` — extend filter to include `revoked_at IS NOT NULL`
- Controller startup code — spawn periodic cleanup task

**Testing:**
- Expired sessions → deleted
- Revoked-but-not-expired sessions → deleted
- Active (not expired, not revoked) sessions → preserved
- Cleanup task runs on schedule

---

### FP-LO9: Standardize soft-delete column naming (#40)

**Addresses:** LO-9 (Low — inconsistent naming: `deactivated_at`, `deleted_at`, `revoked_at`)

**Problem:** Three different column names for the soft-delete concept across 12 entities:
- `deactivated_at` — tenant, user, service, host, software_item, provider_config, ca_certificate (7 entities)
- `revoked_at` — api_token, session, service_certificate (3 entities)
- `deleted_at` — oidc_provider (1 entity)

**Analysis:** Upon closer examination, these names are **semantically distinct** and mostly intentional:
- `deactivated_at` — entity is disabled but may be reactivated (tenants, users, services, hosts)
- `revoked_at` — cryptographic/security revocation, permanent (tokens, sessions, certificates)
- `deleted_at` — the outlier; OIDC provider uses `deleted_at` where `deactivated_at` would be consistent

**Detailed implementation plan:**

1. **Rename only the true outlier:** `oidc_providers.deleted_at` → `oidc_providers.deactivated_at`. This aligns the OIDC provider entity with the dominant pattern for "disabled but recoverable" entities.

2. **Add a migration** to rename the column:
   ```rust
   // In a new migration or modify m20260129_000004_create_oidc.rs
   manager.alter_table(
       Table::alter()
           .table(OidcProviders::Table)
           .rename_column(
               Alias::new("deleted_at"),
               Alias::new("deactivated_at"),
           )
           .to_owned(),
   ).await?;
   ```

3. **Update entity definition** (`db/src/entity/oidc_provider.rs`):
   ```rust
   // Change:
   pub deleted_at: Option<OffsetDateTime>,
   // To:
   pub deactivated_at: Option<OffsetDateTime>,
   ```

4. **Keep `revoked_at` as-is** for tokens, sessions, and certificates — the semantic distinction between "deactivated" (can reactivate) and "revoked" (permanent invalidation) is valuable and correct.

5. **Update all queries** that filter on `oidc_providers.deleted_at` to use `deactivated_at`.

**Files to modify:**
- `crates/core/controller/src/migration/m20260129_000004_create_oidc.rs` — rename column
- `crates/shared/db/src/entity/oidc_provider.rs` — rename field
- All code referencing `oidc_provider::Column::DeletedAt` or `.deleted_at`

**Testing:**
- Migration runs cleanly (up and down)
- OIDC provider soft-delete/restore works with new column name
- Existing queries return correct results

---

### FP-LO12: Add CHECK constraint for update history status (#41)

**Addresses:** LO-12 (Low — free-form status string)

**Problem:** The `update_history.status` column is a plain string with no database-level constraint. While Rust code defines an `UpdateStatus` enum (`Pending`, `InProgress`, `Completed`, `Failed`), nothing prevents invalid values at the database level (e.g., via direct SQL or a bug in deserialization).

**Current code:**
- Migration 018, line 29 — `string(UpdateHistory::Status)` with no CHECK
- Entity `update_history.rs` — `UpdateStatus` enum with `DeriveActiveEnum` and 4 values
- The codebase already uses `.check()` in migration 015 for `available_versions`, confirming SeaORM supports it

**Detailed implementation plan:**

1. **Add a CHECK constraint** in migration 018:
   ```rust
   .col(string(UpdateHistory::Status))
   .check(
       Expr::col(UpdateHistory::Status).is_in(["pending", "in_progress", "completed", "failed"])
   )
   ```

2. **Alternative for databases that don't support `CHECK IN`:** Use raw SQL in the migration:
   ```rust
   manager.get_connection().execute_unprepared(
       "ALTER TABLE update_history ADD CONSTRAINT chk_update_history_status \
        CHECK (status IN ('pending', 'in_progress', 'completed', 'failed'))"
   ).await?;
   ```

3. Since we're modifying migrations directly (per project convention), add the CHECK inline in the table creation statement.

**Files to modify:**
- `crates/core/controller/src/migration/m20260203_000018_create_update_history.rs` — add CHECK constraint

**Testing:**
- Migration runs cleanly
- Inserting valid status values succeeds
- Inserting invalid status value fails with constraint violation

---

### FP-LO13: Fail registration on owner role assignment error (#42)

**Addresses:** LO-13 (Low — silent failure leaves first user without admin access)

**Problem:** All three registration paths (password, OIDC callback, OIDC complete-registration) silently ignore errors from `assign_owner_role()`. The user is created (HTTP 201) but has no owner role, leaving the system in a broken state with no admin user and no recovery path.

**Current code:**
- `routes/auth.rs:117-121` — `if let Err(e)` logs and continues
- `routes/oidc_auth.rs:458-463` — same pattern
- `routes/oidc_auth.rs:802-808` — same pattern

**Detailed implementation plan:**

1. **Make owner role assignment a hard error** in all three paths. If assigning the owner role fails during first-user registration, the entire registration should fail and the user creation should be rolled back.

2. **Wrap in a transaction** in `routes/auth.rs`:
   ```rust
   if is_first_user {
       assign_owner_role(&state.db, state.default_tenant_id, user_id)
           .await
           .map_err(|e| {
               tracing::error!("Failed to assign owner role to first user: {:?}", e);
               error_response(
                   StatusCode::INTERNAL_SERVER_ERROR,
                   "Registration failed: could not assign owner role",
               )
           })?;

       state
           .settings
           .registration_write()
           .await
           .complete_initial_setup(&state.db, state.default_tenant_id)
           .await
           .map_err(|e| {
               tracing::error!("Failed to complete initial setup: {:?}", e);
               error_response(
                   StatusCode::INTERNAL_SERVER_ERROR,
                   "Registration failed: could not complete setup",
               )
           })?;
   }
   ```

3. **Apply the same pattern** in both OIDC registration paths (`oidc_auth.rs:458` and `oidc_auth.rs:802`).

4. **Ideally use a database transaction** to ensure atomicity: if role assignment fails, the user row is also rolled back. This prevents orphaned users without roles.

**Files to modify:**
- `crates/ui/web-api/src/routes/auth.rs` — return error instead of logging and continuing
- `crates/ui/web-api/src/routes/oidc_auth.rs` — same fix in both OIDC paths

**Testing:**
- Successful first-user registration → owner role assigned, HTTP 201
- Simulate role assignment failure → HTTP 500, user NOT created (rolled back)
- Subsequent users → owner role logic not triggered

---

### FP-LO14: Make PKI HTTP bind address configurable (#43)

**Addresses:** LO-14 (Low — hardcoded `0.0.0.0` for PKI listener)

**Problem:** The PKI HTTP listener in `main.rs:1070` is hardcoded to `SocketAddr::from(([0, 0, 0, 0], port))`, binding on all IPv4 interfaces. The HTTPS server uses a configurable `https_addr` setting, but no equivalent exists for the PKI HTTP listener. In multi-NIC environments, this exposes PKI endpoints (CRL, OCSP) to untrusted networks.

**Current code:**
- `main.rs:1070` — `SocketAddr::from(([0, 0, 0, 0], port))` hardcoded
- HTTPS uses configurable `NetworkSettings.https_addr` with default `[::]:8443`
- CLI has `--https-addr` but no `--pki-http-addr`

**Detailed implementation plan:**

1. **Add a CLI argument** in `cli.rs`:
   ```rust
   /// Bind address for the PKI HTTP listener (default: same host as --https-addr)
   #[arg(long)]
   pub pki_http_addr: Option<SocketAddr>,
   ```

2. **Derive the default from `https_addr`** — use the same host as the HTTPS listener with the PKI port:
   ```rust
   let pki_http_addr = match cli.pki_http_addr {
       Some(addr) => addr,
       None => {
           let https_addr = network_settings.https_addr;
           SocketAddr::new(https_addr.ip(), port)
       }
   };
   ```

3. **Update `main.rs:1070`** to use the derived address:
   ```rust
   let pki_http_task = if let Some(port) = pki_http_port {
       let addr = pki_http_addr.unwrap_or_else(|| {
           SocketAddr::new(network_settings.https_addr.ip(), port)
       });
       let app_state_for_pki = Arc::clone(&app_state);
       Some(tokio::spawn(server::run_pki_http(addr, app_state_for_pki)))
   } else {
       None
   };
   ```

4. This ensures the PKI HTTP listener defaults to the same interface as the HTTPS listener, rather than unconditionally binding to all interfaces.

**Files to modify:**
- `crates/core/controller/src/cli.rs` — add `--pki-http-addr` argument
- `crates/core/controller/src/main.rs` — use configured address instead of hardcoded `0.0.0.0`

**Testing:**
- Default: PKI HTTP binds to same host as HTTPS listener
- Explicit `--pki-http-addr 127.0.0.1:8080` → binds to loopback only
- PKI endpoints accessible on configured interface, not on others

---

### FP-LO15: Add expiration column and retention policy for controller events (#44)

**Addresses:** LO-15 (Low — no partition/expiration strategy for outbox table)

**Problem:** The `controller_events` table (migration 024) has no `expires_at` column and no database-level retention policy. Currently, cleanup relies solely on a background task in `event_poller.rs` that deletes events older than 1 hour every 5 minutes. If the poller fails or is not running (e.g., single-instance deployment without HA), the table grows unbounded.

**Current code:**
- Migration 024 — table with `id`, `source_controller_id`, `target_service_id`, `target_service_type`, `message_json`, `created_at` — no `expires_at`
- `event_poller.rs:201-221` — `cleanup_old_events()` deletes `created_at < now - 1h`, hardcoded
- Index on `created_at` exists for cleanup queries

**Detailed implementation plan:**

1. **Add an `expires_at` column** to migration 024:
   ```rust
   .col(timestamp(ControllerEvents::ExpiresAt))
   ```
   Set `expires_at = created_at + 1 hour` by default in the outbox writer.

2. **Update `notification_service.rs`** to populate `expires_at` when writing events:
   ```rust
   let now = OffsetDateTime::now_utc();
   let model = controller_event::ActiveModel {
       // ... existing fields
       created_at: Set(now),
       expires_at: Set(now + time::Duration::hours(1)),
   };
   ```

3. **Update cleanup to use `expires_at`** in `event_poller.rs`:
   ```rust
   async fn cleanup_old_events(&self) {
       let now = OffsetDateTime::now_utc();
       match controller_event::Entity::delete_many()
           .filter(controller_event::Column::ExpiresAt.lt(now))
           .exec(&self.db)
           .await
       {
           // ... existing logging
       }
   }
   ```

4. **Replace the `created_at` cleanup index** with an index on `expires_at` for efficient cleanup queries.

5. **Make retention configurable** via a constant or setting:
   ```rust
   const EVENT_RETENTION: time::Duration = time::Duration::hours(1);
   ```

**Files to modify:**
- `crates/core/controller/src/migration/m20260207_000024_create_controller_events.rs` — add `expires_at` column and index
- `crates/shared/db/src/entity/controller_event.rs` — add `expires_at` field
- `crates/ui/web-api/src/notification_service.rs` — set `expires_at` on insert
- `crates/ui/web-api/src/event_poller.rs` — use `expires_at` for cleanup

**Testing:**
- Events inserted with correct `expires_at` (created_at + 1h)
- Cleanup deletes events past their `expires_at`
- Events within retention window are preserved

---

### FP-LO16: Make migration 011 down path non-lossy (#45)

**Addresses:** LO-16 (Low — rolling back migration destroys user-role assignments)

**Problem:** Migration 011 (`m20260131_000011_update_rbac_permissions.rs`) migrates all admin users to the new owner role in `up()`. The `down()` path deletes the owner role, which cascades and destroys all user-role assignments for migrated users. There is no way to restore the original admin assignments after rollback.

**Current code:**
- `up()` lines 155-165 — `UPDATE user_roles SET role_id = owner_role_id WHERE role_id = admin_role_id`
- `down()` lines 172-179 — `DELETE FROM roles WHERE name = 'owner'` (cascades to user_roles)

**Detailed implementation plan:**

1. **In `down()`, reverse the role migration before deleting** — reassign owner users back to admin:
   ```rust
   async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
       // 1. Find the admin and owner role IDs
       let db = manager.get_connection();

       let admin_role = db.query_one(Statement::from_string(
           db.get_database_backend(),
           "SELECT id FROM roles WHERE name = 'admin'".to_string(),
       )).await?;

       let owner_role = db.query_one(Statement::from_string(
           db.get_database_backend(),
           "SELECT id FROM roles WHERE name = 'owner'".to_string(),
       )).await?;

       // 2. Migrate owner users back to admin (reverse of up())
       if let (Some(admin_row), Some(owner_row)) = (admin_role, owner_role) {
           let admin_id: uuid::Uuid = admin_row.try_get_by_index(0)?;
           let owner_id: uuid::Uuid = owner_row.try_get_by_index(0)?;

           manager.exec_stmt(
               Query::update()
                   .table(UserRoles::Table)
                   .value(UserRoles::RoleId, admin_id)
                   .and_where(Expr::col(UserRoles::RoleId).eq(owner_id))
                   .to_owned(),
           ).await?;
       }

       // 3. Now safe to delete owner role (no user_roles point to it)
       manager.exec_stmt(
           Query::delete()
               .from_table(Roles::Table)
               .and_where(Expr::col(Roles::Name).eq("owner"))
               .to_owned(),
       ).await?;

       // 4. Delete user role
       manager.exec_stmt(
           Query::delete()
               .from_table(Roles::Table)
               .and_where(Expr::col(Roles::Name).eq("user"))
               .to_owned(),
       ).await?;

       // 5. Clear new permissions
       let new_perms = [
           "view_settings", "manage_settings", "view_agents",
           "manage_agents", "manage_global_settings",
       ];
       for name in new_perms {
           manager.exec_stmt(
               Query::delete()
                   .from_table(Permissions::Table)
                   .and_where(Expr::col(Permissions::Name).eq(name))
                   .to_owned(),
           ).await?;
       }

       Ok(())
   }
   ```

2. **Key insight:** The reverse migration (`owner → admin`) must happen *before* the owner role is deleted. This preserves user access — users who were previously admin get admin back. Users who were directly assigned owner (if any were created post-migration) also get admin, which is the closest safe fallback.

3. **Note:** The `manage_global_settings` permission (owner-only) is lost on rollback, which is expected since the admin role didn't have it pre-migration. This is acceptable data loss since the permission itself is being removed.

**Files to modify:**
- `crates/core/controller/src/migration/m20260131_000011_update_rbac_permissions.rs` — rewrite `down()` to reverse role assignments before deletion

**Testing:**
- Run `up()` → admin users become owner users
- Run `down()` → owner users become admin users (not orphaned)
- Verify user_roles table has no orphaned entries after rollback

The following findings from `crates/shared/wire/CODEREVIEW.md` are confirmed and not duplicated here:

| Wire ID | This Review | Status |
|---------|-------------|--------|
| A1 (MQTT password in outbox) | CR-2 | Fixed (FP-CR2) |
| S1 (Argon2 brute-force) | — | Confirmed, fix in wire FP-1 |
| S2 (Anonymous timeout) | HI-5 | Fixed (wire FP-1 + connection cap) |
| S4 (Unbounded output) | HI-3 | Fixed (wire FP-17 + ownership validation) |
| H2 (Cursor advancement) | HI-7 | Fixed |
| H5 (Lease TOCTOU) | HI-6 | Fixed |
| D1 (Connection dedup) | — | Confirmed, fix in wire FP-16 |
| D2 (Update ownership) | HI-3 | Fixed (wire FP-17) |
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

1. **Key material surface area.** `CaSnapshotData` conflates metadata with secrets. Split recommended (FP-CR3 — implemented).

2. **main.rs monolith.** The `run()` function is 1100+ lines. Extract into `init_pki()`, `init_oidc()`, `start_background_tasks()`, `graceful_shutdown()`.

3. **Multi-tenancy readiness.** Several tables lack `tenant_id` (`users`, `sessions`, `roles`, `permissions`), and junction tables (`service_hosts`, `host_software_items`) have no cross-tenant protection. The current single-tenant mode masks these gaps.

4. **Missing observability.** No request ID correlation, no distributed tracing, no metrics endpoint.

5. **Inconsistent shutdown.** The CRL manager is `abort()`-ed while all other tasks use `CancellationToken`. The PKI HTTP server has no graceful shutdown.
