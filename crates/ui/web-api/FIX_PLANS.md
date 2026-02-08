# Fix Plans for Code Review Findings

These plans address findings from CODEREVIEW.md, ordered by severity.

---

## Plan 1: C1 — CA Private Key Material Exposure

### Problem

`CaSnapshotData` and `TrustedCaSnapshot` (in `src/lib.rs:52-76`) derive `Clone` and `Debug` while holding private key PEM strings. Every handler — including unauthenticated OCSP endpoints — clones the entire snapshot including all CA private keys. The `Debug` impl would dump keys into any log, panic backtrace, or core dump. Key material is also never zeroized.

### Plan

**Step 1 — Separate key material from the public snapshot**

Split `CaSnapshotData` into two structs:

- `CaPublicSnapshot` — everything handlers need (cert PEMs, fingerprints, bundle, hash, CNs, `pki_addr`, `managed`, `active_not_after`). Derives `Clone, Debug`.
- `CaKeyStore` — private key PEMs only. Does NOT derive `Clone` or `Debug`.

Replace the `tokio::sync::watch::Receiver<CaSnapshotData>` in `AppState` with:

```rust
pub ca_snapshot: tokio::sync::watch::Receiver<CaPublicSnapshot>,
pub ca_key_store: Arc<tokio::sync::RwLock<CaKeyStore>>,
```

**Step 2 — Restrict key access to consumers that need it**

Only two code paths need the private keys:

1. **OCSP signer** (`src/ocsp.rs`) — signs OCSP responses with CA keys.
2. **Cert signer** (`src/cert_signer.rs`) — signs agent/server certs.
3. **Server cert renewal** (`src/routes/server_cert.rs`) — generates new server key pair.

All other handlers (CA cert distribution, CRL, settings, WebSocket, etc.) only need `CaPublicSnapshot`.

Update the three consumer sites to acquire `ca_key_store.read().await` only when signing is needed.

**Step 3 — Redact keys in Debug and protect with zeroize**

Add the `zeroize` crate as a dependency. Wrap all key material in `Zeroizing<String>`:

```rust
pub struct CaKeyStore {
    pub active_key_pem: Zeroizing<String>,
    pub previous_key_pem: Option<Zeroizing<String>>,
    pub trusted_ca_keys: Vec<TrustedCaKey>,
}

pub struct TrustedCaKey {
    pub fingerprint: String,
    pub key_pem: Zeroizing<String>,
}
```

Implement `Debug` manually for `CaKeyStore` to print `[REDACTED]`.

**Step 4 — Update all consumers**

Files to modify:

| File | Change |
|------|--------|
| `src/lib.rs` | Split `CaSnapshotData`, update `AppState` |
| `src/ocsp.rs` | Read keys from `ca_key_store` instead of snapshot |
| `src/cert_signer.rs` | Read keys from `ca_key_store` |
| `src/routes/server_cert.rs` | Read keys from `ca_key_store` |
| `src/routes/ca.rs` | Use `CaPublicSnapshot` only (no change needed if it doesn't access keys) |
| `src/routes/ocsp.rs` | Pass `ca_key_store` ref to `build_ocsp_response` |
| `src/routes/settings_ca.rs` | Use `CaPublicSnapshot` only |
| `crates/core/controller/` | Update CA rotation code that creates the snapshot |

**Step 5 — Tests**

- Unit test that `Debug` output of `CaKeyStore` does not contain any PEM markers (`-----BEGIN`).
- Unit test that `CaPublicSnapshot` does not have any field containing `key` in its name.
- Verify all existing tests pass after the refactor.

### Risks

- This is a cross-crate refactor — the controller crate creates the snapshot and the web-api crate consumes it.
- The `tokio::sync::watch` channel for the public snapshot stays as-is; only the key store is new.
- The `RwLock` on `CaKeyStore` adds minor contention for OCSP/cert signing, but these are infrequent operations.

---

## Plan 2: C2 — Deactivated Users via OIDC LinkedUser Path

### Problem

`resolve_oidc_user` (`src/auth/authentication.rs:104-113`) returns `LinkedUser(user_id)` when an existing OIDC link is found, without checking if the user is active. The callback handler (`src/routes/oidc_auth.rs:428-439`) processes `LinkedUser` by syncing roles and creating a session — with no deactivation check.

### Plan

**Step 1 — Add `is_active` check in `resolve_oidc_user`**

After finding the link at line 111, load the user and check `is_active`:

```rust
if let Some(link) = existing_link {
    // Load user to check if they are active
    let user = User::find_by_id(link.user_id)
        .one(db)
        .await
        .context_to()?;

    match user {
        Some(u) if !u.is_active => return Ok(OidcUserResolution::Deactivated),
        Some(_) => return Ok(OidcUserResolution::LinkedUser(link.user_id)),
        None => {
            // Orphaned link — user was deleted; fall through to email lookup
        }
    }
}
```

This is the minimal fix. It adds one DB query per OIDC login with an existing link, which is acceptable since OIDC logins are infrequent.

**Step 2 — Defensive check in the callback handler (belt-and-suspenders)**

In `src/routes/oidc_auth.rs`, inside the `LinkedUser` match arm (line 428), add a user-active check before creating the session:

```rust
OidcUserResolution::LinkedUser(user_id) => {
    // Verify user is still active (defense-in-depth)
    match User::find_by_id(user_id).one(&state.db).await {
        Ok(Some(user)) if !user.is_active => {
            return Redirect::to("/login?error=account_deactivated").into_response();
        }
        Ok(None) => {
            return Redirect::to("/login?error=oidc_internal_error").into_response();
        }
        Err(e) => {
            tracing::error!("Failed to load user for OIDC login: {e:?}");
            return Redirect::to("/login?error=oidc_internal_error").into_response();
        }
        _ => {}
    }
    // ... existing sync_oidc_roles + create_oidc_exchange_and_redirect
}
```

**Step 3 — Tests**

Add a test in `authentication.rs` tests:
- Create a user, create an OIDC link, deactivate the user, call `resolve_oidc_user` — assert it returns `Deactivated`.

Add a test in the OIDC callback handler:
- Mock a deactivated user with an existing OIDC link — assert the callback redirects with `error=account_deactivated`.

### Files to modify

| File | Change |
|------|--------|
| `src/auth/authentication.rs` | Add `is_active` check after finding existing link |
| `src/routes/oidc_auth.rs` | Add defensive user-active check in `LinkedUser` arm |

---

## Plan 3: C3 — No Refresh Token Rotation on Use

### Problem

The `refresh` endpoint (`src/routes/auth.rs:382-443`) issues a new access token but does not rotate the refresh token. The same refresh token remains valid for its full 7-day lifetime. A stolen token gives persistent access.

### Plan

**Step 1 — Add `rotate_refresh_token` method to `SessionService`**

In `src/auth/session.rs`, add a new method that atomically:
1. Revokes the old session (set `revoked_at`).
2. Creates a new session with a new refresh token, inheriting `user_id`, `auth_method`, and `oidc_provider_id` from the old session.

```rust
pub async fn rotate_refresh_token(&self, old_token: &str) -> Result<(VerifiedSession, String)> {
    let token_hash = hash_token(old_token);
    let now = OffsetDateTime::now_utc();

    // Find and verify
    let session = Session::find()
        .filter(session::Column::RefreshTokenHash.eq(token_hash))
        .one(&self.db)
        .await
        .context_to()?
        .ok_or_else(|| report!(AuthError::InvalidRefreshToken))?;

    if session.revoked_at.is_some() {
        return Err(report!(AuthError::RefreshTokenRevoked));
    }
    if now >= session.expires_at {
        return Err(report!(AuthError::RefreshTokenExpired));
    }

    // Revoke old
    let mut old: session::ActiveModel = session.clone().into();
    old.revoked_at = Set(Some(now));
    old.update(&self.db).await.context_to()?;

    // Create new
    let auth_method = AuthMethod::from_session(&session.auth_method, session.oidc_provider_id)
        .unwrap_or(AuthMethod::Password);

    let new_token = generate_secure_token()?;
    let new_hash = hash_token(&new_token);
    let expires_at = now + Duration::days(REFRESH_TOKEN_EXPIRY_DAYS);

    let new_session = session::ActiveModel {
        id: Set(generate_uuid()),
        user_id: Set(session.user_id),
        refresh_token_hash: Set(new_hash),
        auth_method: Set(session.auth_method.clone()),
        oidc_provider_id: Set(session.oidc_provider_id),
        token_type: Set("refresh_token".to_string()),
        created_at: Set(now),
        expires_at: Set(expires_at),
        revoked_at: Set(None),
        user_agent: Set(session.user_agent.clone()),
        ip_address: Set(session.ip_address.clone()),
    };
    new_session.insert(&self.db).await.context_to()?;

    let verified = VerifiedSession {
        user_id: session.user_id,
        auth_method,
    };

    Ok((verified, new_token))
}
```

**Step 2 — Update the `refresh` handler**

In `src/routes/auth.rs`, change the `refresh` function to use `rotate_refresh_token` and return both the new access token and the new refresh token:

```rust
let (verified, new_refresh_token) = match session_service
    .rotate_refresh_token(&req.refresh_token)
    .await
{
    Ok(v) => v,
    Err(_) => {
        return error_response(StatusCode::UNAUTHORIZED, "Invalid or expired refresh token");
    }
};
```

**Step 3 — Update `RefreshResponse` type**

In `crates/shared/web-api-types/src/auth.rs`, add `refresh_token` to `RefreshResponse`:

```rust
pub struct RefreshResponse {
    pub access_token: String,
    pub refresh_token: String,  // NEW
    pub expires_in: u64,
    pub token_type: String,
}
```

**Step 4 — Frontend impact**

The frontend must store the new `refresh_token` from each refresh response. This is a breaking API change — the `RefreshResponse` gains a new required field. Update `frontend/src/lib/api.ts` to store the rotated token.

**Step 5 — Tests**

- Test that after `rotate_refresh_token`, the old token is rejected.
- Test that the new token is valid.
- Test that a revoked token cannot be rotated (replay detection).
- Test concurrent rotation of the same token — only one should succeed.

### Files to modify

| File | Change |
|------|--------|
| `src/auth/session.rs` | Add `rotate_refresh_token` method |
| `src/routes/auth.rs` | Use `rotate_refresh_token` in `refresh` handler |
| `crates/shared/web-api-types/src/auth.rs` | Add `refresh_token` to `RefreshResponse` |
| `frontend/src/lib/api.ts` | Store rotated refresh token |

---

## Plan 4: C4 — Rate Limiter TOCTOU

### Problem

`check_rate_limit` (`src/auth/rate_limit.rs:62-99`) performs a non-atomic read-then-write: SELECT, check count, UPDATE. Concurrent requests can all read the same count, pass the check, and increment past the limit. The `upsert_new_window` path (line 94) also unconditionally resets the counter to 1, providing a second bypass avenue.

### Plan

**Step 1 — Replace with atomic increment-then-check**

Rewrite `check_rate_limit` to use a single atomic upsert that increments the counter and returns the new count. The logic is:

1. **Upsert** with ON CONFLICT: if the row exists and the window is still valid, increment `request_count`; if the window expired, reset to 1 with a new `window_start`.
2. **Read back** the current count.
3. **Compare** count against limit.

Since SeaORM's `on_conflict` doesn't support conditional expressions and `RETURNING`, use a two-step approach that is still race-safe:

```rust
pub async fn check_rate_limit(
    &self,
    key: &str,
    max_requests: i32,
    window_secs: i64,
) -> Result<RateLimitOutcome> {
    let now = OffsetDateTime::now_utc();
    let window = time::Duration::seconds(window_secs);
    let threshold = now - window;
    let expires_at = now + time::Duration::seconds(window_secs * 2);

    // Step 1: Atomic upsert — always increment or reset.
    // If the row doesn't exist: insert with count=1.
    // If the row exists with a valid window: increment.
    // If the row exists with an expired window: reset to count=1.
    //
    // We use INSERT ... ON CONFLICT UPDATE with a conditional expression:
    //   request_count = CASE
    //     WHEN window_start >= threshold THEN request_count + 1
    //     ELSE 1
    //   END
    //
    // This is a single SQL statement — atomic at the DB level.

    let stmt = sea_orm::Statement::from_sql_and_values(
        self.db.get_database_backend(),
        r#"INSERT INTO api_rate_limits ("key", request_count, window_start, expires_at)
           VALUES ($1, 1, $2, $3)
           ON CONFLICT ("key") DO UPDATE SET
             request_count = CASE
               WHEN api_rate_limits.window_start >= $4 THEN api_rate_limits.request_count + 1
               ELSE 1
             END,
             window_start = CASE
               WHEN api_rate_limits.window_start >= $4 THEN api_rate_limits.window_start
               ELSE $2
             END,
             expires_at = $3"#,
        [key.into(), now.into(), expires_at.into(), threshold.into()],
    );

    self.db.execute(stmt).await.context_to()?;

    // Step 2: Read back the current count.
    let row = ApiRateLimit::find_by_id(key)
        .one(&self.db)
        .await
        .context_to()?;

    match row {
        Some(row) if row.request_count > max_requests => {
            let window_end = row.window_start + window;
            let remaining = window_end - now;
            let retry_after = Ord::max(remaining.whole_seconds(), 1) as u64;
            Ok(RateLimitOutcome::Limited { retry_after_secs: retry_after })
        }
        _ => Ok(RateLimitOutcome::Allowed),
    }
}
```

**Note:** The raw SQL is necessary because SeaORM's `on_conflict` doesn't support `CASE WHEN` expressions. This is one of the rare justified uses of raw SQL — it's a single parameterized statement, not dynamic SQL. The AGENTS.md rule says "No raw SQL — use SeaORM", so this needs explicit approval. The alternative is using a serializable transaction with `SELECT FOR UPDATE`, which is more complex and less performant.

**Step 2 — Handle DB-backend differences**

SQLite uses `$1` positional params, PostgreSQL uses `$1` too, MySQL uses `?`. Use `sea_orm::Statement::from_sql_and_values` which handles backend-specific quoting, but the SQL syntax for `ON CONFLICT` / `ON DUPLICATE KEY` differs:

- **SQLite/PostgreSQL:** `INSERT ... ON CONFLICT DO UPDATE`
- **MySQL:** `INSERT ... ON DUPLICATE KEY UPDATE`

Provide two SQL variants selected by `self.db.get_database_backend()`.

**Step 3 — Remove the separate `upsert_new_window` method**

The atomic upsert handles both the initial insert and the window reset, so `upsert_new_window` is no longer needed.

**Step 4 — Tests**

- Existing tests should pass unchanged (the observable behavior is the same for non-concurrent cases).
- Add a concurrent test using `tokio::spawn` with multiple tasks hitting the same key simultaneously — assert the total count never exceeds `max_requests + concurrent_tasks` (bounded overshoot due to read-back race, but the increment itself is atomic).

### Files to modify

| File | Change |
|------|--------|
| `src/auth/rate_limit.rs` | Rewrite `check_rate_limit` with atomic upsert |

---

## Plan 5: C5 — Shell Injection in Hook Parameters

### Problem

`resolve_systemd_hook` and `resolve_docker_compose_hook` (`src/update_hooks.rs:138-171`) interpolate user-controlled `service_name`, `project_dir`, and `compose_file` directly into shell command strings via `format!()`. A malicious `service_name` like `myapp; rm -rf /` produces `systemctl stop myapp; rm -rf /`.

### Plan

**Step 1 — Add validation functions for hook parameters**

In `crates/shared/web-api-types/src/update_hooks.rs`, add validation at the type level:

```rust
/// Validate a systemd service name.
/// Allowed: alphanumeric, hyphen, underscore, dot, @.
/// Max length: 256 characters.
pub fn validate_service_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("service name must not be empty".to_string());
    }
    if name.len() > 256 {
        return Err("service name must not exceed 256 characters".to_string());
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || "-_.@".contains(c)) {
        return Err("service name contains invalid characters (allowed: a-z, A-Z, 0-9, -, _, ., @)".to_string());
    }
    Ok(())
}

/// Validate a filesystem path for hook parameters.
/// Disallows shell metacharacters.
/// Allowed: alphanumeric, /, -, _, ., ~
/// Max length: 4096 characters.
pub fn validate_hook_path(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("path must not be empty".to_string());
    }
    if path.len() > 4096 {
        return Err("path must not exceed 4096 characters".to_string());
    }
    let forbidden = [';', '&', '|', '$', '`', '(', ')', '{', '}', '<', '>', '!', '"', '\'', '\\', '\n', '\r', '\0'];
    if path.chars().any(|c| forbidden.contains(&c)) {
        return Err(format!("path contains forbidden shell metacharacters"));
    }
    Ok(())
}
```

**Step 2 — Validate on deserialization or at the API boundary**

Option A: Add `validate()` methods to `SystemdServiceHook` and `DockerComposeHook` and call them in the provider config validation path (`crates/ui/web-api/src/routes/provider_configs.rs` and `crates/ui/web-api/src/routes/software_items.rs`).

Option B: Use a custom deserializer that rejects invalid values. This is more robust but more complex.

Go with Option A — explicit validation at the API boundary:

```rust
impl SystemdServiceHook {
    pub fn validate(&self) -> Result<(), String> {
        validate_service_name(&self.service_name)
    }
}

impl DockerComposeHook {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(ref dir) = self.project_dir {
            validate_hook_path(dir)?;
        }
        if let Some(ref file) = self.compose_file {
            validate_hook_path(file)?;
        }
        Ok(())
    }
}

impl HooksConfig {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(ref hook) = self.pre_update {
            hook.validate()?;
        }
        if let Some(ref hook) = self.post_update {
            hook.validate()?;
        }
        Ok(())
    }
}

impl UpdateHookConfig {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(ref predefined) = self.predefined {
            match predefined {
                PredefinedHook::SystemdService(h) => h.validate()?,
                PredefinedHook::DockerCompose(h) => h.validate()?,
            }
        }
        Ok(())
    }
}
```

**Step 3 — Shell-escape in `resolve_predefined_hook` (defense-in-depth)**

Even after validation, shell-quote the values as a second layer of defense:

```rust
fn shell_quote(s: &str) -> String {
    // Single-quote wrapping: replace any ' with '\''
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn resolve_systemd_hook(hook: &SystemdServiceHook) -> String {
    format!("systemctl {} {}", hook.action.as_str(), shell_quote(&hook.service_name))
}

fn resolve_docker_compose_hook(hook: &DockerComposeHook) -> String {
    let mut parts = Vec::new();

    if let Some(project_dir) = &hook.project_dir {
        parts.push(format!("cd {}", shell_quote(project_dir)));
    }

    let mut compose_cmd = String::from("docker-compose");
    if let Some(compose_file) = &hook.compose_file {
        compose_cmd.push_str(&format!(" -f {}", shell_quote(compose_file)));
    }
    compose_cmd.push(' ');
    compose_cmd.push_str(hook.action.as_str());
    if hook.action == DockerComposeAction::Up {
        compose_cmd.push_str(" -d");
    }

    parts.push(compose_cmd);
    parts.join(" && ")
}
```

**Step 4 — Call validation from API endpoints**

In `provider_configs.rs` (create/update) and `software_items.rs` (create/update), after deserializing the config JSON, validate hooks:

```rust
if let Some(hooks_config) = parsed_config.get("hooks") {
    if let Ok(hooks) = serde_json::from_value::<HooksConfig>(hooks_config.clone()) {
        if let Err(e) = hooks.validate() {
            return error_response(StatusCode::BAD_REQUEST, &format!("Invalid hook config: {e}"));
        }
    }
}
```

**Step 5 — Tests**

- Test that `validate_service_name` rejects `"myapp; rm -rf /"`, `"app$(whoami)"`, `"app\nmalicious"`.
- Test that `validate_service_name` accepts `"my-app_v2"`, `"nginx"`, `"postgresql@14"`.
- Test that `validate_hook_path` rejects `"/opt/app; cat /etc/shadow"`, `"/opt/$(cmd)"`.
- Test that `validate_hook_path` accepts `"/opt/my-app"`, `"/home/user/.config/app"`.
- Test that `shell_quote` correctly handles edge cases.
- Test that existing hooks tests still pass.

### Files to modify

| File | Change |
|------|--------|
| `crates/shared/web-api-types/src/update_hooks.rs` | Add validation functions and `validate()` methods |
| `crates/ui/web-api/src/update_hooks.rs` | Add `shell_quote`, update `resolve_systemd_hook` and `resolve_docker_compose_hook` |
| `crates/ui/web-api/src/routes/provider_configs.rs` | Call hooks validation on create/update |
| `crates/ui/web-api/src/routes/software_items.rs` | Call hooks validation on create/update |

---

## Plan 6: C6 — No WebSocket Message Size Limits

### Problem

`WebSocketUpgrade` in `src/routes/service_ws.rs:187` is called with no configuration — inheriting tungstenite's default 64 MB max message size. Combined with no rate limiting on the WebSocket endpoint, a small number of concurrent large messages can exhaust server memory. The wire protocol messages should never be anywhere close to 64 MB.

### Plan

**Step 1 — Configure message size limits on upgrade**

Axum's `WebSocketUpgrade` exposes `.max_message_size()` and `.max_frame_size()` for configuration. Add a reasonable limit based on wire protocol expectations:

- Agent messages: The largest expected message is `UpdateOutput` which appends log output. Even with generous sizing, 1 MB is more than sufficient.
- MQTT messages: Configuration and lease payloads are small JSON, well under 64 KB.

Set a conservative upper limit of **1 MB** (1,048,576 bytes) for max message size:

```rust
// In service_ws handler (src/routes/service_ws.rs:187)
ws.max_message_size(1_048_576)  // 1 MB
    .max_frame_size(262_144)    // 256 KB per frame
    .on_upgrade(move |socket| handle_connection(socket, state, conn_type, ip))
```

**Step 2 — Extract limit as a constant**

Define the limits as module-level constants for easy tuning:

```rust
/// Maximum WebSocket message size (1 MB). Wire protocol messages should be
/// well under this limit — the largest is UpdateOutput log data.
const WS_MAX_MESSAGE_SIZE: usize = 1_048_576;
/// Maximum WebSocket frame size (256 KB).
const WS_MAX_FRAME_SIZE: usize = 262_144;
```

**Step 3 — Handle oversized messages gracefully**

When a message exceeds the limit, tungstenite closes the connection with a protocol error. The existing disconnect cleanup in `handle_connection` (unregistering the service, releasing MQTT leases) already handles unexpected disconnects, so no additional cleanup is needed.

Add a tracing warning in the connection handler for oversized message disconnects (the error will come through as a `tungstenite::Error::Capacity` variant).

**Step 4 — Tests**

- Unit test that the WebSocket upgrade configures the expected limits (verify by inspection since axum doesn't expose the configured limits).
- Integration test: connect a WebSocket and send a message exceeding 1 MB — verify the connection is closed.

### Files to modify

| File | Change |
|------|--------|
| `src/routes/service_ws.rs` | Add `max_message_size` and `max_frame_size` to `ws.on_upgrade()` |

---

## Plan 7: C7 — No Rate Limiting on WebSocket Endpoint + Argon2 Brute-Force Amplification

### Problem

Two related issues:

1. The `/api/v1/ws/service` endpoint is not rate-limited and accepts anonymous connections. Each connection attempt triggers bearer secret verification which can be expensive.
2. `lookup_by_secret` (`src/routes/service_ws.rs:200-232`) performs an O(N) argon2 scan against all non-deactivated MQTT services when the fast SHA-256 lookup fails. Each anonymous connection attempt with a random bearer token causes significant CPU consumption, amplified by the number of MQTT services.

### Plan

**Step 1 — Add per-IP connection rate limiting for WebSocket upgrades**

Add a rate-limiting check at the top of `service_ws` before the WebSocket upgrade. Use the existing database-backed rate limiter (`auth/rate_limit.rs`) with a dedicated key prefix:

```rust
pub async fn service_ws(
    State(state): State<Arc<AppState>>,
    identity: Option<Extension<ServiceIdentity>>,
    client_ip: Option<Extension<ClientIp>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    // Rate-limit WebSocket connection attempts per IP
    if let Some(Extension(ClientIp(ip))) = &client_ip {
        let key = format!("ws_connect:{ip}");
        match state.rate_limiter.check_rate_limit(&key, 30, 60).await {
            Ok(RateLimitOutcome::Limited { retry_after_secs }) => {
                return (
                    StatusCode::TOO_MANY_REQUESTS,
                    format!("Too many connection attempts. Retry after {retry_after_secs}s"),
                ).into_response();
            }
            Err(e) => {
                // Rate limiter fail-closed for WebSocket (unlike HTTP which fails open)
                tracing::error!(error = %e, "WebSocket rate limiter error — rejecting");
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Service temporarily unavailable",
                ).into_response();
            }
            _ => {}
        }
    }
    // ... existing connection type resolution
```

Rate limit: 30 connection attempts per 60 seconds per IP. This is generous enough for legitimate reconnect scenarios but prevents brute-force.

**Step 2 — Eliminate the O(N) argon2 scan**

The root cause is that MQTT enrollment secrets are hashed with argon2 (variable-time, CPU-intensive) instead of SHA-256 like agent secrets. The fix is to standardize on SHA-256 hashing for enrollment secrets across all service types.

Since there's no backward-compatibility requirement, change MQTT service enrollment to use SHA-256 hashing like agents:

a. In the MQTT service creation path, store `enrollment_secret_hash` as a SHA-256 hash (same as agents).

b. Remove the argon2 fallback from `lookup_by_secret` entirely — all lookups are now O(1) SHA-256:

```rust
async fn lookup_by_secret(
    db: &sea_orm::DatabaseConnection,
    secret: &str,
) -> ServiceWsResult<service_entity::Model> {
    let secret_hash = crate::auth::token::hash_token(secret);
    uptrakit_shared_db::entity::prelude::Service::find()
        .filter(service_entity::Column::EnrollmentSecretHash.eq(&secret_hash))
        .filter(service_entity::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .context_to::<ServiceWsError>()?
        .ok_or_else(|| report!(ServiceWsError::InvalidSecret))
}
```

c. Add a migration to re-hash any existing MQTT enrollment secrets. Since the plaintext secret is not stored, existing MQTT services will need to re-enroll after this change. Document this in release notes as a breaking change.

**Step 3 — Add stricter rate limiting for bearer auth failures**

After a failed bearer lookup, add an additional penalty to the rate limiter (count failed attempts more heavily):

```rust
Err(e) => {
    // Count failed bearer attempts with higher weight
    if let Some(Extension(ClientIp(ip))) = &client_ip {
        let key = format!("ws_bearer_fail:{ip}");
        let _ = state.rate_limiter.check_rate_limit(&key, 5, 300).await;
    }
    // ... existing error response
}
```

This limits to 5 failed bearer attempts per 5 minutes per IP, making brute-force impractical.

**Step 4 — Tests**

- Test that WebSocket connections from the same IP are rejected after 30 attempts in 60 seconds.
- Test that failed bearer lookups trigger the stricter rate limit.
- Test that `lookup_by_secret` works with SHA-256-only hashing.
- Test that mTLS connections bypass the bearer rate limit (they don't use `lookup_by_secret`).

### Files to modify

| File | Change |
|------|--------|
| `src/routes/service_ws.rs` | Add rate limiting, simplify `lookup_by_secret` |
| `src/routes/services.rs` | Change MQTT enrollment secret hashing from argon2 to SHA-256 |
| `src/middleware/rate_limit.rs` | Ensure WebSocket endpoints are covered |

---

## Plan 8: H1 — First-User Registration Race Condition

### Problem

`register` handler (`src/routes/auth.rs:86-136`) performs a non-atomic read-then-act: `User::find().count()` → `insert()` → `assign_owner_role()`. Two concurrent requests during initial setup can both observe zero users and both get the `owner` role. The same race exists in the OIDC path.

### Plan

**Step 1 — Use database-level uniqueness to detect the first user**

Instead of counting users and then inserting, leverage the database to atomically determine the first user. Two approaches:

**Approach A (Preferred): Atomic advisory flag in the settings table**

After user insertion, use an atomic upsert to claim the "first user" flag:

```rust
// After inserting the user successfully:
let claimed_first = crate::settings_store::try_claim_initial_owner(
    &state.db,
    state.default_tenant_id,
    user_id,
).await;

if claimed_first {
    assign_owner_role(&state.db, state.default_tenant_id, user_id).await?;
    state.settings.registration_write().await
        .complete_initial_setup(&state.db, state.default_tenant_id).await?;
}
```

The `try_claim_initial_owner` function uses an atomic INSERT with ON CONFLICT:

```rust
/// Atomically claim the initial-owner flag. Returns `true` if this call
/// was the one that set it (i.e., the caller is the first user).
pub async fn try_claim_initial_owner(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    user_id: Uuid,
) -> Result<bool> {
    // INSERT the "initial_owner" setting with user_id value.
    // ON CONFLICT DO NOTHING — if it already exists, this is a no-op.
    let result = Setting::insert(setting::ActiveModel {
        key: Set("initial_owner".to_string()),
        value: Set(serde_json::json!(user_id.to_string())),
        tenant_id: Set(tenant_id),
        ..Default::default()
    })
    .on_conflict(
        sea_query::OnConflict::columns([setting::Column::Key, setting::Column::TenantId])
            .do_nothing()
            .to_owned(),
    )
    .exec(db)
    .await;

    match result {
        Ok(_) => Ok(true),    // INSERT succeeded — we are the first
        Err(DbErr::RecordNotInserted) => Ok(false),  // ON CONFLICT — someone else was first
        Err(e) => Err(e.into()),
    }
}
```

This is race-safe: only one INSERT can succeed, all concurrent ones get `DO NOTHING`.

**Step 2 — Apply the same fix to the OIDC registration path**

In `src/routes/oidc_auth.rs`, the `CreateUser` handling at lines 443-447 has the same pattern. Replace the `User::find().count()` check with `try_claim_initial_owner`:

```rust
OidcUserResolution::CreateUser { email, first_name, last_name } => {
    // Create user...
    let user_id = generate_uuid();
    // ... insert user ...

    // Atomically check if this is the first user
    if try_claim_initial_owner(&state.db, tenant_id, user_id).await? {
        assign_owner_role(&state.db, tenant_id, user_id).await?;
        // ... complete initial setup ...
    } else {
        assign_user_role(&state.db, tenant_id, user_id).await?;
    }
}
```

**Step 3 — Remove the count-based check**

Delete the `User::find().count()` call from both handlers. The `try_claim_initial_owner` function handles the detection atomically.

**Step 4 — Tests**

- Test that two concurrent registrations result in exactly one owner.
- Test that the second user gets the `user` role, not `owner`.
- Test that `try_claim_initial_owner` returns `true` only once per tenant.
- Test the OIDC path has the same behavior.

### Files to modify

| File | Change |
|------|--------|
| `src/routes/auth.rs` | Replace count-based first-user check with `try_claim_initial_owner` |
| `src/routes/oidc_auth.rs` | Same replacement in OIDC `CreateUser` path |
| `src/settings_store.rs` | Add `try_claim_initial_owner` function |

---

## Plan 9: H4 — Tenant Isolation Bypass via X-Tenant-Id Header

### Problem

`TenantContext` (`src/middleware/tenant_context.rs:29-51`) reads `X-Tenant-Id` from any request and uses it without authorization checks. Any authenticated user can set `X-Tenant-Id: <any-uuid>` to access another tenant's resources. Multi-tenancy is documented as "future work" but the header processing is live in production.

### Plan

**Step 1 — Remove X-Tenant-Id header processing**

Since multi-tenancy is future work, the safest fix is to remove the header processing entirely. The `TenantContext` extractor should always use the default tenant ID:

```rust
impl FromRequestParts<Arc<AppState>> for TenantContext {
    type Rejection = (StatusCode, Json<ErrorResponse>);

    async fn from_request_parts(
        _parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        Ok(TenantContext {
            tenant_id: state.default_tenant_id,
        })
    }
}
```

**Step 2 — Strip the X-Tenant-Id header from untrusted requests**

Add `x-tenant-id` to `strip_proxy_headers` in `resolve_proxy_headers.rs` as a defense-in-depth measure:

```rust
fn strip_proxy_headers(
    headers: &mut HeaderMap,
    info_header: Option<&str>,
    pem_header: Option<&str>,
) {
    if let Some(h) = info_header {
        headers.remove(h);
    }
    if let Some(h) = pem_header {
        headers.remove(h);
    }
    headers.remove("x-forwarded-proto");
    headers.remove("x-forwarded-host");
    headers.remove("x-tenant-id");  // Prevent tenant spoofing
}
```

**Step 3 — Leave a TODO for proper multi-tenancy**

Add a comment in `tenant_context.rs` documenting the expected authorization pattern for when multi-tenancy is implemented:

```rust
// TODO: When multi-tenancy is enabled, re-add X-Tenant-Id header processing
// with these requirements:
// 1. Only accept X-Tenant-Id from authenticated users
// 2. Verify the user has access to the requested tenant via a user_tenant
//    mapping table
// 3. Reject with 403 if the user doesn't have access
// 4. Strip X-Tenant-Id header from non-proxy requests
```

**Step 4 — Tests**

- Test that `TenantContext` always returns `default_tenant_id` regardless of `X-Tenant-Id` header value.
- Test that `strip_proxy_headers` removes `x-tenant-id`.
- Remove or update any existing tests that relied on `X-Tenant-Id` header selection.

### Files to modify

| File | Change |
|------|--------|
| `src/middleware/tenant_context.rs` | Remove `X-Tenant-Id` header processing |
| `src/middleware/resolve_proxy_headers.rs` | Add `x-tenant-id` to stripped headers |

---

## Plan 10: H5 — Origin Header Trusted from Untrusted Clients

### Problem

`resolve_external_base_url` (`src/middleware/resolve_proxy_headers.rs:274-284`) gives highest priority to the `Origin` header regardless of whether the request comes from a trusted proxy. An attacker can set `Origin: https://evil.com` and OIDC redirect URLs and device auth verification URLs will point to the attacker's domain. The `strip_proxy_headers` function does NOT strip `Origin`.

### Plan

**Step 1 — Only trust Origin from trusted proxies**

Modify `resolve_external_base_url` to only use the `Origin` header when the request comes from a trusted proxy. For non-proxy requests, skip straight to the `Host` header:

```rust
fn resolve_external_base_url(headers: &HeaderMap, from_trusted_proxy: bool) -> Option<String> {
    if from_trusted_proxy {
        // 1. Origin header (only from trusted proxy)
        let origin = headers
            .get("origin")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim_end_matches('/'));
        if let Some(o) = origin && !o.is_empty() {
            return Some(o.to_string());
        }

        // 2. X-Forwarded-Proto + X-Forwarded-Host
        let proto = headers
            .get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok());
        let fwd_host = headers
            .get("x-forwarded-host")
            .and_then(|v| v.to_str().ok());

        if let (Some(proto), Some(host)) = (proto, fwd_host) {
            return Some(format!("{}://{}", proto, host.trim_end_matches('/')));
        }

        // 3. X-Forwarded-Proto + Host
        if let Some(proto) = proto
            && let Some(host) = headers.get("host").and_then(|v| v.to_str().ok())
        {
            return Some(format!("{}://{}", proto, host.trim_end_matches('/')));
        }
    }

    // 4. Host with https://
    headers
        .get("host")
        .and_then(|v| v.to_str().ok())
        .map(|h| format!("https://{}", h.trim_end_matches('/')))
}
```

**Step 2 — Strip Origin from non-proxy requests**

Add `Origin` to the headers stripped from non-proxy requests in `strip_proxy_headers`:

```rust
fn strip_proxy_headers(
    headers: &mut HeaderMap,
    info_header: Option<&str>,
    pem_header: Option<&str>,
) {
    if let Some(h) = info_header {
        headers.remove(h);
    }
    if let Some(h) = pem_header {
        headers.remove(h);
    }
    headers.remove("x-forwarded-proto");
    headers.remove("x-forwarded-host");
    headers.remove("origin");
}
```

**Step 3 — Update existing tests**

The existing test `external_base_url_from_origin` passes `from_trusted_proxy: false` and expects Origin to be used. Update this test to reflect the new behavior:

```rust
#[test]
fn external_base_url_origin_only_from_trusted_proxy() {
    let mut headers = HeaderMap::new();
    headers.insert("origin", HeaderValue::from_static("https://app.example.com/"));
    headers.insert("host", HeaderValue::from_static("internal:8443"));

    // Non-proxy: Origin ignored, falls back to Host
    let url = resolve_external_base_url(&headers, false);
    assert_eq!(url, Some("https://internal:8443".to_string()));

    // Trusted proxy: Origin used
    let url = resolve_external_base_url(&headers, true);
    assert_eq!(url, Some("https://app.example.com".to_string()));
}
```

**Step 4 — Add attack scenario test**

```rust
#[test]
fn external_base_url_origin_spoofing_blocked() {
    let mut headers = HeaderMap::new();
    headers.insert("origin", HeaderValue::from_static("https://evil.com"));
    headers.insert("host", HeaderValue::from_static("legit.example.com:8443"));

    // Untrusted client — Origin is ignored
    let url = resolve_external_base_url(&headers, false);
    assert_eq!(url, Some("https://legit.example.com:8443".to_string()));
}
```

### Files to modify

| File | Change |
|------|--------|
| `src/middleware/resolve_proxy_headers.rs` | Move Origin inside `from_trusted_proxy` guard, add to `strip_proxy_headers` |

---

## Plan 11: H2 — JWT Access Tokens Cannot Be Revoked (15-Minute Window)

### Problem

JWT access tokens (`src/middleware/require_auth.rs:126-156`, `src/auth/jwt.rs:109-118`) are validated purely statelessly — no DB lookup, no revocation check, no `is_active` check. Permissions are baked into the JWT at issuance and trusted for the full 15-minute lifetime. After user deactivation, role changes, or logout, access persists until the token expires.

### Plan

**Step 1 — Introduce a short-lived token denylist**

Create a new in-memory denylist backed by a time-based eviction cache. Since JWT tokens are short-lived (15 min), the denylist only needs to hold entries until they would have expired anyway.

Add a new module `src/auth/token_denylist.rs`:

```rust
use std::collections::HashMap;
use std::sync::Arc;
use time::OffsetDateTime;
use tokio::sync::RwLock;
use uuid::Uuid;

/// In-memory denylist for revoked JWT access tokens.
///
/// Entries auto-expire after the JWT's `exp` time. The denylist is periodically
/// purged of expired entries.
pub struct TokenDenylist {
    inner: Arc<RwLock<HashMap<String, i64>>>,  // jti -> exp timestamp
}

impl TokenDenylist {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Add a JWT ID to the denylist. The entry will be kept until `exp_timestamp`.
    pub async fn deny(&self, jti: &str, exp_timestamp: i64) {
        self.inner.write().await.insert(jti.to_string(), exp_timestamp);
    }

    /// Deny all tokens for a given user (by adding user_id to a separate set).
    pub async fn deny_user(&self, user_id: Uuid, until: i64) {
        // Store user-level revocation with a "user:" prefix
        let key = format!("user:{user_id}");
        self.inner.write().await.insert(key, until);
    }

    /// Check if a token's JTI or its user is denied.
    pub async fn is_denied(&self, jti: &str, user_id: &Uuid, iat: i64) -> bool {
        let guard = self.inner.read().await;
        if guard.contains_key(jti) {
            return true;
        }
        // Check user-level revocation: if the user was denied after the token's iat
        let user_key = format!("user:{user_id}");
        if let Some(&until) = guard.get(&user_key) {
            if iat < until {
                return true;
            }
        }
        false
    }

    /// Purge expired entries. Call periodically (e.g., every 5 minutes).
    pub async fn purge_expired(&self) {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        self.inner.write().await.retain(|_, exp| *exp > now);
    }
}
```

**Step 2 — Wire denylist into `AppState`**

Add the denylist to `AppState`:

```rust
pub struct AppState {
    // ... existing fields ...
    pub token_denylist: TokenDenylist,
}
```

**Step 3 — Check denylist in JWT authentication**

In `authenticate_jwt` (`src/middleware/require_auth.rs:126-156`), after decoding the token, check the denylist:

```rust
fn authenticate_jwt(
    state: &AppState,
    token: &str,
) -> std::result::Result<AuthenticatedUser, AuthFailure> {
    let claims = state.jwt.decode_access_token(token)
        .map_err(|_| AuthFailure::Unauthorized("Invalid or expired token"))?;

    let user_id = uuid::Uuid::parse_str(&claims.sub)
        .map_err(|_| AuthFailure::Unauthorized("Invalid token subject"))?;

    // Check token denylist (requires async — see Step 4)
    // This is the revocation check.
}
```

Since `authenticate_jwt` is currently synchronous (it calls `decode_access_token` which is pure crypto), but the denylist is async (`RwLock::read`), we need to make it async. Change `authenticate_jwt` to an `async fn` and update the call site.

**Step 4 — Populate denylist on revocation events**

Add denylist entries when:

1. **User deactivation** (`src/routes/users.rs` — deactivate endpoint): call `deny_user(user_id, now + 900)` to invalidate all tokens issued before now, for the max token lifetime.
2. **Role changes** (`src/routes/users.rs` — role assignment): same as deactivation.
3. **Logout** (`src/routes/auth.rs` — logout endpoint): since we don't have the JWT's `jti` at logout (only the refresh token), use `deny_user(user_id, now + 900)`.
4. **Password change**: `deny_user(user_id, now + 900)`.

**Step 5 — HA synchronization via database**

For multi-instance HA, add a `token_revocations` table that instances poll:

```sql
CREATE TABLE token_revocations (
    id UUID PRIMARY KEY,
    revocation_key TEXT NOT NULL,   -- "jti:<jti>" or "user:<user_id>"
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_token_revocations_expires ON token_revocations (expires_at);
```

Each instance polls this table periodically (every 5-10 seconds) and merges new entries into its local denylist. Old entries are purged based on `expires_at`.

**Step 6 — Start a periodic cleanup task**

In the server startup, spawn a task that runs `token_denylist.purge_expired()` every 5 minutes and polls the `token_revocations` table every 10 seconds for HA sync.

**Step 7 — Tests**

- Test that a denied JTI is rejected by `authenticate_jwt`.
- Test that `deny_user` revokes all tokens for that user issued before the revocation.
- Test that tokens issued *after* the revocation are still valid.
- Test that expired denylist entries are purged.
- Test the HA sync: insert into `token_revocations` table, verify the other "instance" picks it up.

### Files to modify

| File | Change |
|------|--------|
| `src/auth/token_denylist.rs` | New module — in-memory denylist |
| `src/auth/mod.rs` | Add `pub mod token_denylist` |
| `src/lib.rs` | Add `TokenDenylist` to `AppState` |
| `src/middleware/require_auth.rs` | Make `authenticate_jwt` async, add denylist check |
| `src/routes/users.rs` | Call `deny_user` on deactivation and role change |
| `src/routes/auth.rs` | Call `deny_user` on logout |
| Migration | Add `token_revocations` table |

### Risks

- Adds a small performance cost (async `RwLock` read) to every JWT-authenticated request. The denylist is in-memory so this should be sub-microsecond.
- HA polling introduces up to 10 seconds of staleness between instances. This is acceptable given the current 15-minute window.
- The denylist is in-memory, so a server restart clears it — but the DB table ensures entries are re-populated on next poll.

---

## Plan 12: H3 — JWT Signing Key Divergence in HA Deployments

### Problem

Each controller instance generates its own JWT signing key from its local `data_dir` (`src/auth/jwt.rs:38-73`). If instances don't share the same state directory, tokens issued by instance A are rejected by instance B, breaking HA.

### Plan

**Step 1 — Store the JWT signing key in the database**

Instead of using a file-based key, store the JWT signing key in the `settings` table. This ensures all HA instances share the same key.

Add a new function to `settings_store.rs`:

```rust
/// Load or generate the JWT signing key from the database.
///
/// If no key exists, generates a new 64-byte random key and stores it.
/// Uses INSERT ... ON CONFLICT DO NOTHING to handle concurrent initialization.
pub async fn load_or_generate_jwt_key(
    db: &DatabaseConnection,
    tenant_id: Uuid,
) -> Result<Vec<u8>> {
    // Try to load existing key
    let raw = load_all_settings(db, tenant_id).await?;
    if let Some(value) = raw.get_setting(SettingKey::JwtSigningKey) {
        if let Some(b64) = value.as_str() {
            let bytes = base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                b64,
            )
            .map_err(|e| report!(AuthError::Internal(format!("invalid JWT key encoding: {e}"))))?;
            return Ok(bytes);
        }
    }

    // Generate new key
    let mut rng = rand::rng();
    let mut bytes = vec![0u8; 64];
    rng.fill(&mut bytes[..]);

    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);

    // Atomic insert — if another instance already created the key, use theirs
    upsert_setting(db, tenant_id, SettingKey::JwtSigningKey, &serde_json::json!(b64)).await?;

    // Re-read to ensure we have the canonical value (in case of race)
    let raw = load_all_settings(db, tenant_id).await?;
    if let Some(value) = raw.get_setting(SettingKey::JwtSigningKey) {
        if let Some(b64) = value.as_str() {
            let bytes = base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                b64,
            )
            .map_err(|e| report!(AuthError::Internal(format!("invalid JWT key encoding: {e}"))))?;
            return Ok(bytes);
        }
    }

    Ok(bytes)
}
```

**Step 2 — Add `JwtSigningKey` to `SettingKey` enum**

In the settings key enum, add:

```rust
JwtSigningKey => "jwt_signing_key",
```

**Step 3 — Update `JwtManager` initialization**

Change the server startup to load the JWT key from DB instead of file:

```rust
// Before:
let jwt = JwtManager::load_or_generate(&data_dir)?;

// After:
let jwt_secret = settings_store::load_or_generate_jwt_key(&db, tenant_id).await?;
let jwt = JwtManager::from_secret(&jwt_secret);
```

**Step 4 — Migration from file-based key**

For existing deployments, add a one-time migration that:
1. Checks if `jwt_signing.key` exists on disk.
2. If yes and no DB key exists, reads the file key and stores it in DB.
3. Logs a warning about the migration.

```rust
pub async fn migrate_file_jwt_key(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    data_dir: &Path,
) -> Result<()> {
    let key_path = data_dir.join("jwt_signing.key");
    if !key_path.exists() {
        return Ok(());
    }

    // Check if DB already has a key
    let raw = load_all_settings(db, tenant_id).await?;
    if raw.get_setting(SettingKey::JwtSigningKey).is_some() {
        tracing::info!("JWT key already in database, skipping file migration");
        return Ok(());
    }

    let bytes = std::fs::read(&key_path)?;
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
    upsert_setting(db, tenant_id, SettingKey::JwtSigningKey, &serde_json::json!(b64)).await?;
    tracing::info!("migrated JWT signing key from file to database");
    Ok(())
}
```

**Step 5 — Keep `load_or_generate` for backward compatibility in tests**

The file-based `load_or_generate` can remain as a convenience for tests but should not be used in production startup.

**Step 6 — Tests**

- Test that two `JwtManager` instances from the same DB key can cross-validate tokens.
- Test the file-to-DB migration path.
- Test that concurrent `load_or_generate_jwt_key` calls produce the same key.

### Files to modify

| File | Change |
|------|--------|
| `src/settings_store.rs` | Add `load_or_generate_jwt_key`, `migrate_file_jwt_key` |
| `src/lib.rs` or startup code | Load JWT key from DB instead of file |
| `src/auth/jwt.rs` | Keep `from_secret`, deprecate `load_or_generate` for production |
| `SettingKey` enum | Add `JwtSigningKey` variant |

---

## Plan 13: H6 — Server Private Key Written Without Restricted Permissions + Non-Atomic Write

### Problem

`renew_server_certificate_inner` (`src/routes/server_cert.rs:138-147`) uses `std::fs::write()` which creates files with default umask permissions (typically 0644). The `server.key` file is world-readable. The cert and key are also written in two separate calls — a crash between them produces a mismatched pair that would break TLS on restart.

### Plan

**Step 1 — Write to temporary files then atomically rename**

Replace the two `std::fs::write` calls with an atomic write pattern:

```rust
use std::io::Write;

// Write to temp files first
let cert_path = state.pki_path.join("server.crt");
let key_path = state.pki_path.join("server.key");
let cert_tmp = state.pki_path.join("server.crt.tmp");
let key_tmp = state.pki_path.join("server.key.tmp");

// Write key with restricted permissions
write_restricted(&key_tmp, key_pem.as_bytes())?;
// Write cert (less sensitive, but still temp first for atomicity)
write_restricted(&cert_tmp, cert_pem.as_bytes())?;

// Atomic rename — both succeed or neither
std::fs::rename(&key_tmp, &key_path).context_to::<RenewCertError>()?;
std::fs::rename(&cert_tmp, &cert_path).context_to::<RenewCertError>()?;
```

**Step 2 — Create a `write_restricted` helper**

Add a helper that writes a file with `0o600` permissions on Unix:

```rust
/// Write data to a file with restricted permissions (0o600 on Unix).
fn write_restricted(path: &std::path::Path, data: &[u8]) -> RenewCertResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .context_to::<RenewCertError>()?;
        file.write_all(data).context_to::<RenewCertError>()?;
        file.sync_all().context_to::<RenewCertError>()?;
    }

    #[cfg(not(unix))]
    {
        std::fs::write(path, data).context_to::<RenewCertError>()?;
    }

    Ok(())
}
```

**Step 3 — Handle crash between renames**

On Unix, `rename(2)` is atomic per-file, but two separate renames are not jointly atomic. However, the key is renamed first so:
- If the process crashes after key rename but before cert rename, the old cert still pairs with... the new key (mismatch). This is better handled by:

Write both files under a single "generation" directory, then atomically swap a symlink:

```rust
let gen_dir = state.pki_path.join(format!("gen-{}", uuid::Uuid::now_v7()));
std::fs::create_dir_all(&gen_dir).context_to::<RenewCertError>()?;

write_restricted(&gen_dir.join("server.key"), key_pem.as_bytes())?;
write_restricted(&gen_dir.join("server.crt"), cert_pem.as_bytes())?;

// Atomically swap symlink
let current_link = state.pki_path.join("current");
let tmp_link = state.pki_path.join("current.tmp");
std::os::unix::fs::symlink(&gen_dir, &tmp_link).context_to::<RenewCertError>()?;
std::fs::rename(&tmp_link, &current_link).context_to::<RenewCertError>()?;
```

**Alternative (simpler):** Since the TLS config is hot-reloaded in memory immediately after the write, the on-disk files are only used for server restart. A simpler approach is to keep the two-rename pattern but also write a `server.gen` marker file that records the generation. On startup, verify that the cert and key match (the cert's public key matches the key) and if not, re-trigger renewal.

**Recommended approach:** Use the simpler two-rename pattern (Step 1-2) since the hot-reload immediately applies the new config in memory. Add a startup validation check (Step 4) to catch any crash-induced mismatch.

**Step 4 — Startup cert/key consistency check**

In the server startup code, after loading `server.crt` and `server.key`, verify they match:

```rust
/// Verify the cert's public key matches the private key.
fn verify_cert_key_match(cert_pem: &str, key_pem: &str) -> bool {
    // Parse cert's public key, parse private key's public component, compare
    // Use rcgen or x509-parser to extract the public key from both
}
```

If they don't match, log an error and trigger automatic renewal.

**Step 5 — Apply same pattern to initial cert generation**

The initial server cert generation in the controller startup code should also use `write_restricted`.

**Step 6 — Tests**

- Test that `write_restricted` creates files with `0o600` permissions on Unix.
- Test that the atomic rename pattern doesn't leave partial state.
- Test the startup consistency check detects mismatched cert/key pairs.

### Files to modify

| File | Change |
|------|--------|
| `src/routes/server_cert.rs` | Add `write_restricted`, atomic rename pattern |
| Controller startup code | Add cert/key consistency check, use `write_restricted` for initial generation |

---

## Plan 14: H7 — Connection Registry Overwrites on Concurrent Reconnect

### Problem

When a service reconnects before its previous connection's cleanup runs, `register_agent`/`register_mqtt` (`src/service_connections.rs:55-67, 74-91`) unconditionally overwrite via `HashMap::insert`. The old handler's cleanup then calls `unregister` which removes the **new** connection's entry, leaving the service unable to receive push notifications.

### Plan

**Step 1 — Add a generation counter to each connection**

Add a monotonically increasing generation counter to distinguish connections:

```rust
use std::sync::atomic::{AtomicU64, Ordering};

static CONNECTION_GENERATION: AtomicU64 = AtomicU64::new(0);

struct ServiceConnection {
    sender: mpsc::Sender<ControllerMessage>,
    service_type: ServiceType,
    generation: u64,  // NEW
    instance_id: Option<String>,
    max_tenants: Option<u32>,
    assigned_mqtt_clients: HashSet<Uuid>,
    last_heartbeat: Option<Instant>,
}
```

**Step 2 — Return generation from registration**

Change `register_agent` and `register_mqtt` to return the generation along with the receiver:

```rust
pub async fn register_agent(
    &self,
    service_id: Uuid,
) -> (mpsc::Receiver<ControllerMessage>, u64) {
    let generation = CONNECTION_GENERATION.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = mpsc::channel(16);
    let conn = ServiceConnection {
        sender: tx,
        service_type: ServiceType::Agent,
        generation,
        instance_id: None,
        max_tenants: None,
        assigned_mqtt_clients: HashSet::new(),
        last_heartbeat: None,
    };
    self.inner.write().await.insert(service_id, conn);
    (rx, generation)
}
```

Same for `register_mqtt`.

**Step 3 — Guard `unregister` with generation check**

Change `unregister` to accept the generation and only remove if it matches:

```rust
pub async fn unregister(
    &self,
    service_id: &Uuid,
    generation: u64,
) -> Option<HashSet<Uuid>> {
    let mut guard = self.inner.write().await;
    if let Some(conn) = guard.get(service_id) {
        if conn.generation == generation {
            return guard.remove(service_id).map(|c| c.assigned_mqtt_clients);
        }
        // Different generation — newer connection exists, don't remove
        tracing::debug!(
            service_id = %service_id,
            old_gen = generation,
            new_gen = conn.generation,
            "skipping unregister for superseded connection"
        );
        None
    } else {
        None
    }
}
```

**Step 4 — Update all callers**

The WebSocket handlers (`agent_ws.rs`, `mqtt_ws.rs`) need to capture the generation on registration and pass it to `unregister` on disconnect:

```rust
// On connect:
let (push_rx, generation) = state.service_connections.register_agent(service_id).await;

// On disconnect:
let released = state.service_connections.unregister(&service_id, generation).await;
```

**Step 5 — Tests**

- Test that registering the same service_id twice produces different generations.
- Test that `unregister` with the old generation does NOT remove the new connection.
- Test that `unregister` with the current generation removes the connection.
- Test that after reconnect, the old handler's cleanup is a no-op.

### Files to modify

| File | Change |
|------|--------|
| `src/service_connections.rs` | Add generation counter to `ServiceConnection`, guard `unregister` |
| `src/routes/agent_ws.rs` | Capture and pass generation |
| `src/routes/mqtt_ws.rs` | Capture and pass generation |

---

## Plan 15: H8 — Settings Reload Torn Reads (6 Independent RwLocks Updated Sequentially)

### Problem

`reload_from_db` (`src/settings.rs:232-281`) acquires and releases 6 independent `RwLock`s sequentially. A concurrent request handler can observe a mix of old and new settings (e.g., new registration settings but old network settings). The `check_version_and_reload` method also uses `Ordering::Relaxed` for version counter loads, which can return stale values on ARM/multi-core.

### Plan

**Step 1 — Replace individual RwLocks with a single atomic snapshot**

Replace the 6 independent `RwLock`s with a single `tokio::sync::watch` channel that holds an immutable settings snapshot:

```rust
#[derive(Clone)]
pub struct SettingsSnapshot {
    pub registration: RegistrationSettings,
    pub authentication: AuthenticationSettings,
    pub agent_cert_lifetime_days: u16,
    pub renewal_window_hours: u16,
    pub network: NetworkSettings,
    pub mqtt_max_clients_per_tenant: u16,
}

pub struct Settings {
    inner: Arc<Inner>,
}

struct Inner {
    /// Watch channel for atomic settings reads. Readers see a consistent snapshot.
    snapshot_tx: tokio::sync::watch::Sender<SettingsSnapshot>,
    snapshot_rx: tokio::sync::watch::Receiver<SettingsSnapshot>,
    /// Version counters for cross-instance invalidation.
    version: AtomicI64,
    global_version: AtomicI64,
    /// Write mutex — only one reload at a time.
    reload_mutex: tokio::sync::Mutex<()>,
}
```

**Step 2 — Readers subscribe to the watch channel**

Replace all the individual `RwLock`-based readers with watch channel reads:

```rust
impl Settings {
    /// Get a consistent snapshot of all settings.
    pub fn snapshot(&self) -> SettingsSnapshot {
        self.inner.snapshot_rx.borrow().clone()
    }

    pub fn registration(&self) -> RegistrationSettings {
        self.inner.snapshot_rx.borrow().registration.clone()
    }

    pub fn authentication(&self) -> AuthenticationSettings {
        self.inner.snapshot_rx.borrow().authentication.clone()
    }

    pub fn agent_cert_lifetime_days(&self) -> u16 {
        self.inner.snapshot_rx.borrow().agent_cert_lifetime_days
    }

    // ... etc for each setting
}
```

Note these are now synchronous (no `.await`), since `watch::Receiver::borrow()` is sync. This simplifies all call sites.

**Step 3 — Atomic reload via watch channel send**

`reload_from_db` builds the complete new snapshot, then atomically publishes it:

```rust
pub async fn reload_from_db(
    &self,
    db: &DatabaseConnection,
    tenant_id: Uuid,
) -> auth::Result<()> {
    let _guard = self.inner.reload_mutex.lock().await;
    let raw = crate::settings_store::load_all_settings(db, tenant_id).await?;

    let snapshot = SettingsSnapshot {
        registration: RegistrationSettings::from_raw(&raw),
        authentication: AuthenticationSettings::from_raw(&raw),
        agent_cert_lifetime_days: raw.get_setting(SettingKey::AgentCertLifetimeDays)
            .and_then(|v| v.as_u64()?.try_into().ok())
            .unwrap_or(DEFAULT_AGENT_CERT_LIFETIME_DAYS),
        renewal_window_hours: raw.get_setting(SettingKey::AgentCertRenewalWindowHours)
            .and_then(|v| v.as_u64()?.try_into().ok())
            .unwrap_or(DEFAULT_RENEWAL_WINDOW_HOURS),
        network: Self::load_network_settings(&raw),
        mqtt_max_clients_per_tenant: raw.get_setting(SettingKey::MqttMaxClientsPerTenant)
            .and_then(|v| v.as_u64()?.try_into().ok())
            .unwrap_or(DEFAULT_MQTT_MAX_CLIENTS_PER_TENANT),
    };

    // Atomic publish — all readers see the complete new snapshot
    let _ = self.inner.snapshot_tx.send(snapshot);

    // Update version counters with Release ordering
    let (version, global_version) =
        crate::settings_store::get_settings_versions(db, tenant_id).await?;
    self.inner.version.store(version, Ordering::Release);
    self.inner.global_version.store(global_version, Ordering::Release);

    Ok(())
}
```

**Step 4 — Fix memory ordering on version counter loads**

Change `check_version_and_reload` to use `Ordering::Acquire` for loads (matching the `Release` stores):

```rust
let cached_version = self.inner.version.load(Ordering::Acquire);
let cached_global_version = self.inner.global_version.load(Ordering::Acquire);
```

**Step 5 — Handle write-side settings updates**

Several endpoints update individual settings (e.g., `set_trusted_proxies`, `set_agent_cert_lifetime_days`). These need to:
1. Acquire the reload mutex.
2. Get the current snapshot.
3. Clone and modify the relevant field.
4. Publish the new snapshot.

```rust
pub async fn set_agent_cert_lifetime_days(&self, days: u16) {
    let _guard = self.inner.reload_mutex.lock().await;
    let mut snapshot = self.inner.snapshot_rx.borrow().clone();
    snapshot.agent_cert_lifetime_days = days;
    let _ = self.inner.snapshot_tx.send(snapshot);
}
```

Alternatively, for write-side operations that update and persist settings, the caller should persist to DB first, then bump the settings version, which will trigger a reload on all instances (including self).

**Step 6 — Simplify registration_write/authentication_write**

The current `registration_write()` and `authentication_write()` return `RwLockWriteGuard`s which are used for mutation. Replace these with explicit update methods:

```rust
pub async fn update_registration<F>(&self, f: F)
where
    F: FnOnce(&mut RegistrationSettings),
{
    let _guard = self.inner.reload_mutex.lock().await;
    let mut snapshot = self.inner.snapshot_rx.borrow().clone();
    f(&mut snapshot.registration);
    let _ = self.inner.snapshot_tx.send(snapshot);
}
```

Update all call sites that use `registration_write()` and `authentication_write()`.

**Step 7 — Tests**

- Test that `snapshot()` returns a consistent snapshot (all fields from the same generation).
- Test that concurrent reads during a reload all see either the old or new snapshot, never a mix.
- Test that `set_*` methods atomically update the snapshot.
- Test that version counters use correct ordering.

### Files to modify

| File | Change |
|------|--------|
| `src/settings.rs` | Replace 6 `RwLock`s with `watch` channel, add `SettingsSnapshot`, `reload_mutex` |
| All files calling `settings.registration()`, `settings.authentication()`, etc. | Remove `.await` from reads (now sync) |
| Files using `registration_write()` / `authentication_write()` | Switch to `update_registration()` / `update_authentication()` |

### Risks

- This is a large refactor touching many files (every handler that reads settings).
- The main simplification is that reads become synchronous, which actually makes call sites simpler.
- The `reload_mutex` serializes writes but reads are lock-free via the watch channel, so there's no performance regression.

---

## Plan 16: H9 — TOCTOU in `upsert_setting` — Concurrent Upserts Can Conflict

### Problem

`upsert_setting` (`src/settings_store.rs:50-84`) performs a read-then-insert pattern without a transaction: `find_by_id` → `update` or `insert`. Two concurrent inserts for the same key produce a unique constraint violation. The same pattern exists in `bump_settings_version` (lines 159-168) where `rows_affected == 0` triggers a bare `insert` that races with another instance's insert.

### Plan

**Step 1 — Replace read-then-write with SeaORM `on_conflict`**

SeaORM supports `INSERT ... ON CONFLICT DO UPDATE` via `insert` with `on_conflict`. Replace the find-then-branch pattern:

```rust
pub async fn upsert_setting(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    key: SettingKey,
    value: serde_json::Value,
) -> Result<()> {
    let now = OffsetDateTime::now_utc();
    let db_key = key.as_str().to_string();

    let model = setting::ActiveModel {
        tenant_id: Set(tenant_id),
        key: Set(db_key),
        value: Set(value),
        updated_at: Set(now),
    };

    Setting::insert(model)
        .on_conflict(
            sea_query::OnConflict::columns([
                setting::Column::TenantId,
                setting::Column::Key,
            ])
            .update_columns([setting::Column::Value, setting::Column::UpdatedAt])
            .to_owned(),
        )
        .exec(db)
        .await
        .context_to()?;

    // Bump the version counter (non-fatal on failure)
    if let Err(e) = bump_settings_version(db, tenant_id, key.is_global()).await {
        tracing::warn!(error = ?e, key = key.as_str(), "failed to bump settings version counter");
    }

    Ok(())
}
```

This is a single atomic SQL statement — no TOCTOU.

**Step 2 — Fix `bump_settings_version` defensive insert**

The `rows_affected == 0` fallback at lines 159-168 uses a bare `insert` that races. Replace with `on_conflict`:

```rust
pub async fn bump_settings_version(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    is_global: bool,
) -> Result<()> {
    let now = OffsetDateTime::now_utc();

    if is_global {
        // Increment global_version on ALL rows
        SettingsVersion::update_many()
            .col_expr(
                settings_version::Column::GlobalVersion,
                Expr::col(settings_version::Column::GlobalVersion).add(1),
            )
            .col_expr(settings_version::Column::UpdatedAt, Expr::value(now))
            .exec(db)
            .await
            .context_to()?;
    } else {
        // Try to increment version on this tenant's row
        let result = SettingsVersion::update_many()
            .col_expr(
                settings_version::Column::Version,
                Expr::col(settings_version::Column::Version).add(1),
            )
            .col_expr(settings_version::Column::UpdatedAt, Expr::value(now))
            .filter(settings_version::Column::TenantId.eq(tenant_id))
            .exec(db)
            .await
            .context_to()?;

        // Defensive: if the row didn't exist, insert with on_conflict
        if result.rows_affected == 0 {
            let model = settings_version::ActiveModel {
                tenant_id: Set(tenant_id),
                version: Set(1),
                global_version: Set(0),
                revocation_version: Set(0),
                updated_at: Set(now),
            };
            SettingsVersion::insert(model)
                .on_conflict(
                    sea_query::OnConflict::column(settings_version::Column::TenantId)
                        .update_columns([
                            settings_version::Column::Version,
                            settings_version::Column::UpdatedAt,
                        ])
                        .to_owned(),
                )
                .exec(db)
                .await
                .context_to()?;
        }
    }

    Ok(())
}
```

**Step 3 — Apply the same fix to `bump_revocation_version`**

The `rows_affected == 0` path at lines 208-217 has the same race. Replace the bare `insert` with `on_conflict`:

```rust
if result.rows_affected == 0 {
    let model = settings_version::ActiveModel {
        tenant_id: Set(tenant_id),
        version: Set(0),
        global_version: Set(0),
        revocation_version: Set(1),
        updated_at: Set(now),
    };
    SettingsVersion::insert(model)
        .on_conflict(
            sea_query::OnConflict::column(settings_version::Column::TenantId)
                .update_columns([
                    settings_version::Column::RevocationVersion,
                    settings_version::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec(db)
        .await
        .context_to()?;
}
```

**Step 4 — Tests**

- Test that concurrent `upsert_setting` calls for the same key don't produce constraint violations.
- Test that the value from the last writer wins.
- Test that `bump_settings_version` initializes correctly on first call for a new tenant.
- Test that concurrent `bump_settings_version` calls both succeed without error.

### Files to modify

| File | Change |
|------|--------|
| `src/settings_store.rs` | Replace `upsert_setting` with `on_conflict`, fix `bump_settings_version` and `bump_revocation_version` defensive inserts |

---

## Plan 17: H10 — Update History Operations Not Checked Against Agent Ownership (IDOR)

### Problem

`UpdateStarted`, `UpdateOutput`, and `UpdateResult` handlers in `src/routes/agent_ws.rs:306-389` look up `update_history` records by `update_history_id` from the message payload without verifying the record belongs to the authenticated agent. A compromised agent can tamper with any update record by submitting a valid `update_history_id` that belongs to a different agent's host.

### Plan

**Step 1 — Extract host_ids for the current agent once**

At the beginning of `handle_agent_authenticated`, after registering the agent, look up the agent's linked host_ids and keep them for the session:

```rust
// At the top of handle_agent_authenticated, after register_agent:
let agent_host_ids: HashSet<uuid::Uuid> = {
    match uptrakit_shared_db::entity::prelude::ServiceHost::find()
        .filter(uptrakit_shared_db::entity::service_host::Column::ServiceId.eq(agent_id))
        .all(&state.db)
        .await
    {
        Ok(links) => links.into_iter().map(|l| l.host_id).collect(),
        Err(e) => {
            tracing::error!(error = %e, "failed to look up agent hosts");
            HashSet::new()
        }
    }
};
```

Note: This set should be refreshed after `ReportHostInfo` (which can create new host links).

**Step 2 — Add ownership validation to all update history handlers**

For `UpdateStarted`, `UpdateOutput`, and `UpdateResult`, after finding the `update_history` record, verify `record.host_id` is in the agent's set:

```rust
ServiceMessage::UpdateStarted(payload) => {
    if let Ok(Some(record)) = UpdateHistory::find_by_id(payload.update_history_id)
        .one(&state.db)
        .await
    {
        // Verify this update belongs to a host linked to this agent
        if !agent_host_ids.contains(&record.host_id) {
            tracing::warn!(
                %agent_id,
                update_id = %payload.update_history_id,
                host_id = %record.host_id,
                "agent attempted to modify update for unlinked host"
            );
            let err = ControllerMessage::Error(ErrorPayload {
                code: ErrorCode::Forbidden,
                message: "update record does not belong to this agent".to_string(),
            });
            if let Some(json) = serialize_controller_msg(out_seq, err) {
                let _ = sink.send(Message::Text(json.into())).await;
            }
            continue;
        }
        // ... proceed with update
    }
}
```

Apply the same check to `UpdateOutput` and `UpdateResult`.

**Step 3 — Refresh host_ids after ReportHostInfo**

After processing `ReportHostInfo` (which calls `find_or_create_host_and_link`), refresh `agent_host_ids`:

```rust
ServiceMessage::ReportHostInfo(payload) => {
    // ... existing host info processing ...

    // Refresh agent_host_ids after potential new host link
    if let Ok(links) = uptrakit_shared_db::entity::prelude::ServiceHost::find()
        .filter(uptrakit_shared_db::entity::service_host::Column::ServiceId.eq(agent_id))
        .all(&state.db)
        .await
    {
        agent_host_ids = links.into_iter().map(|l| l.host_id).collect();
    }
}
```

**Step 4 — Extract validation into a helper**

To avoid repetition across the three update message types, create a helper:

```rust
async fn validate_update_ownership(
    db: &sea_orm::DatabaseConnection,
    update_id: uuid::Uuid,
    agent_host_ids: &HashSet<uuid::Uuid>,
) -> Option<update_history::Model> {
    match UpdateHistory::find_by_id(update_id).one(db).await {
        Ok(Some(record)) if agent_host_ids.contains(&record.host_id) => Some(record),
        Ok(Some(record)) => {
            tracing::warn!(
                update_id = %update_id,
                host_id = %record.host_id,
                "ownership check failed for update record"
            );
            None
        }
        _ => None,
    }
}
```

**Step 5 — Tests**

- Test that `UpdateStarted` for a record belonging to a different agent's host is rejected.
- Test that `UpdateOutput` and `UpdateResult` for unlinked hosts are rejected.
- Test that after `ReportHostInfo`, the agent can update records for the newly-linked host.
- Test that valid updates still work (no regression).

### Files to modify

| File | Change |
|------|--------|
| `src/routes/agent_ws.rs` | Add `agent_host_ids` lookup, ownership check on all update handlers, refresh after `ReportHostInfo` |

---

## Plan 18: H11 — Unbounded Update Output Accumulation

### Problem

Each `UpdateOutput` message (`src/routes/agent_ws.rs:327-343`) appends to the existing output with `format!("{}{}\n", record.output, payload.output)` — loading the entire existing output from DB, concatenating in memory, and writing back. No size limit exists. This causes:

1. **Quadratic memory growth:** Each append reads the full history, so N messages of M bytes each allocates O(N*M) memory per message, totaling O(N^2 * M).
2. **Unbounded DB storage:** A misbehaving agent can fill the database with arbitrarily large output.

### Plan

**Step 1 — Define a maximum output size constant**

```rust
/// Maximum total output size per update record (10 MB).
const MAX_UPDATE_OUTPUT_SIZE: usize = 10 * 1024 * 1024;
```

**Step 2 — Enforce the limit in UpdateOutput handler**

Before appending, check the current size and truncate or reject:

```rust
ServiceMessage::UpdateOutput(payload) => {
    if let Some(record) = validate_update_ownership(
        &state.db, payload.update_history_id, &agent_host_ids
    ).await {
        let current_len = record.output.len();
        if current_len >= MAX_UPDATE_OUTPUT_SIZE {
            tracing::debug!(
                update_id = %payload.update_history_id,
                current_len,
                "update output limit reached, dropping further output"
            );
            continue;
        }

        // Truncate incoming output if it would exceed the limit
        let available = MAX_UPDATE_OUTPUT_SIZE - current_len;
        let to_append = if payload.output.len() > available {
            let truncated = &payload.output[..available];
            format!("{truncated}\n[output truncated at {MAX_UPDATE_OUTPUT_SIZE} bytes]")
        } else {
            format!("{}\n", payload.output)
        };

        let mut active: update_history::ActiveModel = record.into();
        // Use concat in SQL instead of loading full output into memory
        active.output = Set(format!("{}{to_append}", "")); // see Step 3
        if let Err(e) = active.update(&state.db).await {
            tracing::warn!(error = %e, "failed to append update output");
        }
    }
}
```

**Step 3 — Use SQL-level concatenation to avoid quadratic memory**

Instead of loading the full output, appending in Rust, and writing it back, use a SQL UPDATE with concatenation:

```rust
// Instead of read-modify-write, use a direct SQL UPDATE:
use sea_orm::Statement;

let to_append = format!("{}\n", payload.output);
let backend = state.db.get_database_backend();

let stmt = Statement::from_sql_and_values(
    backend,
    r#"UPDATE update_history
       SET output = output || $1
       WHERE id = $2
         AND length(output) < $3"#,
    [to_append.into(), payload.update_history_id.into(), (MAX_UPDATE_OUTPUT_SIZE as i64).into()],
);

if let Err(e) = state.db.execute(stmt).await {
    tracing::warn!(error = %e, "failed to append update output");
}
```

This avoids loading the full output into memory entirely. The `length(output) < $3` condition enforces the limit at the DB level.

**Note:** The SQL concatenation operator (`||`) works in SQLite, PostgreSQL, and MySQL (with `CONCAT()` for MySQL). Use a backend match:

```rust
let sql = match backend {
    sea_orm::DatabaseBackend::MySql => {
        r#"UPDATE update_history
           SET output = CONCAT(output, ?)
           WHERE id = ?
             AND CHAR_LENGTH(output) < ?"#
    }
    _ => {
        r#"UPDATE update_history
           SET output = output || $1
           WHERE id = $2
             AND length(output) < $3"#
    }
};
```

**Step 4 — Apply the same limit to UpdateResult final output**

The `UpdateResult` handler (lines 362-367) also appends output. Apply the same truncation:

```rust
let final_output = if payload.output.is_empty() {
    record.output.clone()
} else {
    let combined = format!("{}{}", record.output, payload.output);
    if combined.len() > MAX_UPDATE_OUTPUT_SIZE {
        let truncated = &combined[..MAX_UPDATE_OUTPUT_SIZE];
        format!("{truncated}\n[output truncated at {MAX_UPDATE_OUTPUT_SIZE} bytes]")
    } else {
        combined
    }
};
```

**Step 5 — Tests**

- Test that output accumulation stops at `MAX_UPDATE_OUTPUT_SIZE`.
- Test that the truncation marker is appended.
- Test that `UpdateResult` also respects the limit.
- Test that normal-sized outputs are not affected.

### Files to modify

| File | Change |
|------|--------|
| `src/routes/agent_ws.rs` | Add `MAX_UPDATE_OUTPUT_SIZE`, enforce limit in `UpdateOutput` and `UpdateResult`, use SQL-level concatenation |

---

## Plan 19: M1 — Rate Limiter Fails Open on DB Errors

### Problem

`rate_limit_auth` middleware (`src/middleware/rate_limit.rs:96-100`) catches DB errors from `check_rate_limit` and allows the request through (fail-open). An attacker who can induce DB pressure (e.g., by overloading the connection pool) can disable all rate limiting.

### Plan

**Step 1 — Add configurable fail-open/fail-closed behavior per endpoint category**

Not all endpoints should fail-closed — for general API endpoints, fail-open is reasonable to avoid availability issues. But for security-critical endpoints (login, register), fail-closed is safer:

```rust
struct EndpointRateLimit {
    max_requests: i32,
    window_secs: i64,
    fail_closed: bool,  // NEW
}
```

Set `fail_closed: true` for `login` and `register`, `false` for others:

```rust
static RATE_LIMITS: LazyLock<HashMap<&'static str, EndpointRateLimit>> = LazyLock::new(|| {
    HashMap::from([
        (
            "/api/v1/auth/login",
            EndpointRateLimit {
                max_requests: 10,
                window_secs: 60,
                fail_closed: true,
            },
        ),
        (
            "/api/v1/auth/register",
            EndpointRateLimit {
                max_requests: 10,
                window_secs: 60,
                fail_closed: true,
            },
        ),
        (
            "/api/v1/auth/refresh",
            EndpointRateLimit {
                max_requests: 10,
                window_secs: 60,
                fail_closed: false,
            },
        ),
        // ... etc
    ])
});
```

**Step 2 — Update the error handling in the middleware**

```rust
Err(e) => {
    tracing::error!(path, error = %e, "rate limit check failed");
    if limit.fail_closed {
        error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "Service temporarily unavailable, please try again later",
        )
    } else {
        next.run(req).await
    }
}
```

**Step 3 — Add an in-memory fallback rate limiter**

For fail-closed endpoints, add a simple in-memory counter as fallback when the DB is unavailable. This provides basic protection even when the database is down:

```rust
use std::sync::Mutex;
use std::collections::HashMap;
use std::time::Instant;

static FALLBACK_COUNTERS: LazyLock<Mutex<HashMap<String, (u32, Instant)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn fallback_rate_check(key: &str, max: u32, window_secs: u64) -> bool {
    let mut map = FALLBACK_COUNTERS.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    let entry = map.entry(key.to_string()).or_insert((0, now));
    if now.duration_since(entry.1).as_secs() > window_secs {
        *entry = (1, now);
        return true; // allowed
    }
    entry.0 += 1;
    entry.0 <= max
}
```

Update the error path:

```rust
Err(e) => {
    tracing::error!(path, error = %e, "rate limit check failed");
    if limit.fail_closed {
        // Fallback to in-memory rate limiting
        if fallback_rate_check(&key, limit.max_requests as u32, limit.window_secs as u64) {
            next.run(req).await
        } else {
            error_response(
                StatusCode::TOO_MANY_REQUESTS,
                "Too many requests, please try again later",
            )
        }
    } else {
        next.run(req).await
    }
}
```

**Step 4 — Add periodic cleanup for the fallback counters**

Spawn a task that periodically purges expired entries from `FALLBACK_COUNTERS` to prevent memory growth.

**Step 5 — Tests**

- Test that login/register return 503 or 429 when DB is unavailable (not 200).
- Test that refresh/device endpoints still work when DB is unavailable (fail-open).
- Test the in-memory fallback correctly limits requests.
- Test that the fallback counter resets after the window expires.

### Files to modify

| File | Change |
|------|--------|
| `src/middleware/rate_limit.rs` | Add `fail_closed` flag, in-memory fallback limiter, update error handling |

---

## Plan 20: M2 — OIDC AutoLink Bypasses Account Ownership Verification

### Problem

`resolve_oidc_user` (`src/auth/authentication.rs:147-159`) returns `AutoLink` when a user has no password and no other active OIDC link. This automatically links the OIDC identity to the existing user without any proof that the OIDC user owns the email address. The risk depends on whether the OIDC provider verifies emails — if not (or if the provider allows arbitrary email claims), an attacker can claim any passwordless account.

### Plan

**Step 1 — Only auto-link if the OIDC provider is trusted for email verification**

Add an `email_verified_trusted` flag to the OIDC provider configuration:

```rust
// In the oidc_provider entity or config
pub email_verified_trusted: bool,  // Whether this provider's email_verified claim is trustworthy
```

In `resolve_oidc_user`, only auto-link if the provider is trusted AND the `email_verified` claim is true:

```rust
// 2d. Auto-link — only if provider is trusted for email verification
//     and the ID token asserts email_verified = true
return Ok(OidcUserResolution::AutoLink {
    user_id: found_user.id,
});
```

**Step 2 — Pass `email_verified` from the ID token to `resolve_oidc_user`**

Add `email_verified: Option<bool>` to `OidcUserParams`:

```rust
pub struct OidcUserParams<'a> {
    pub db: &'a DatabaseConnection,
    pub tenant_id: uuid::Uuid,
    pub provider_id: uuid::Uuid,
    pub oidc_subject: &'a str,
    pub email: &'a str,
    pub first_name: Option<&'a str>,
    pub last_name: Option<&'a str>,
    pub auto_create: bool,
    pub email_verified: Option<bool>,  // NEW
    pub provider_trusts_email: bool,   // NEW
}
```

**Step 3 — Guard AutoLink with email verification**

```rust
// 2d. Auto-link — only if we trust the email ownership
if params.provider_trusts_email && params.email_verified == Some(true) {
    return Ok(OidcUserResolution::AutoLink {
        user_id: found_user.id,
    });
}

// Otherwise, require explicit account linking via password or another provider
if found_user.password_hash.is_some() {
    return Ok(OidcUserResolution::LinkViaPasswordRequired {
        user_id: found_user.id,
    });
}

// No password, no trusted auto-link — the user cannot link without admin help
return Ok(OidcUserResolution::NotAllowed);
```

**Step 4 — Add `email_verified_trusted` to the OIDC provider entity**

Add a migration to add `email_verified_trusted BOOLEAN NOT NULL DEFAULT TRUE` to the `oidc_providers` table. Default to `true` for backward compatibility with existing providers (most major providers like Google, Microsoft, Okta do verify emails).

Update the provider creation/update API to accept and store this flag.

**Step 5 — Extract `email_verified` from ID token claims**

In the OIDC callback handler (`src/routes/oidc_auth.rs`), extract the `email_verified` claim from the ID token:

```rust
let email_verified = claims
    .get("email_verified")
    .and_then(|v| v.as_bool());
```

Pass it to `OidcUserParams`.

**Step 6 — Tests**

- Test that `AutoLink` is returned when `email_verified = true` and `provider_trusts_email = true`.
- Test that `NotAllowed` is returned when `email_verified = false`.
- Test that `NotAllowed` is returned when `provider_trusts_email = false` even if `email_verified = true`.
- Test that `LinkViaPasswordRequired` is returned for users with passwords regardless of email verification.
- Test backward compatibility: existing providers default to `email_verified_trusted = true`.

### Files to modify

| File | Change |
|------|--------|
| `src/auth/authentication.rs` | Add `email_verified` and `provider_trusts_email` to `OidcUserParams`, guard `AutoLink` |
| `src/routes/oidc_auth.rs` | Extract `email_verified` from ID token, pass to `resolve_oidc_user` |
| `src/routes/oidc_providers.rs` | Add `email_verified_trusted` to provider create/update |
| Migration | Add `email_verified_trusted` column to `oidc_providers` |
| `crates/shared/db/src/entity/oidc_provider.rs` | Add `email_verified_trusted` field |

---

## Plan 21: M3 — OIDC Role Sync Non-Atomic (Delete-Then-Insert Without Transaction)

### Problem

`sync_oidc_roles` (`src/auth/authentication.rs:264-282`) deletes all existing `user_role` rows for the user, then inserts the new mapped roles one-by-one. No transaction wraps these operations. A crash between delete and insert leaves the user with zero roles. Concurrent logins can also interfere.

### Plan

**Step 1 — Wrap delete-then-insert in a transaction**

Use SeaORM's `TransactionTrait` to ensure atomicity:

```rust
pub async fn sync_oidc_roles(
    db: &DatabaseConnection,
    tenant_id: uuid::Uuid,
    user_id: uuid::Uuid,
    provider: &oidc_provider::Model,
    claims: &serde_json::Value,
) -> Result<()> {
    // ... existing claim extraction (lines 220-261) ...

    if local_roles.is_empty() {
        return Ok(());
    }

    // Atomic delete + insert within a transaction
    use sea_orm::TransactionTrait;
    db.transaction::<_, _, crate::auth::AuthError>(|txn| {
        Box::pin(async move {
            // Delete existing roles for this user within the tenant
            UserRole::delete_many()
                .filter(user_role::Column::TenantId.eq(tenant_id))
                .filter(user_role::Column::UserId.eq(user_id))
                .exec(txn)
                .await
                .context_to()?;

            // Insert mapped roles
            let now = OffsetDateTime::now_utc();
            for local_role in &local_roles {
                let user_role_model = user_role::ActiveModel {
                    tenant_id: Set(tenant_id),
                    user_id: Set(user_id),
                    role_id: Set(local_role.id),
                    assigned_at: Set(now),
                };
                user_role_model.insert(txn).await.context_to()?;
            }

            Ok(())
        })
    })
    .await
    .map_err(|e| match e {
        sea_orm::TransactionError::Connection(e) => report!(crate::auth::AuthError::from(e)),
        sea_orm::TransactionError::Transaction(e) => e,
    })?;

    Ok(())
}
```

**Step 2 — Tests**

- Test that roles are correctly replaced after sync.
- Test that a failure during insert rolls back the delete (user keeps old roles).

### Files to modify

| File | Change |
|------|--------|
| `src/auth/authentication.rs` | Wrap `sync_oidc_roles` delete+insert in a transaction |

---

## Plan 22: M4 — OCSP 1-Hour Cache Serves Stale Revocation Status

### Problem

`Cache-Control: max-age=3600` on OCSP responses (`src/routes/ocsp.rs:19-26`) means revoked certificates may appear valid to clients caching the response for up to 1 hour after revocation.

### Plan

**Step 1 — Reduce cache TTL to 5 minutes**

Change the `Cache-Control` header from 1 hour to 5 minutes, balancing revocation freshness against OCSP responder load:

```rust
(header::CACHE_CONTROL, "max-age=300, public"),
```

**Step 2 — Extract cache TTL as a constant**

```rust
/// OCSP response cache duration. Shorter values improve revocation freshness
/// at the cost of higher OCSP responder load.
const OCSP_CACHE_MAX_AGE_SECS: u32 = 300; // 5 minutes
```

Build the header value once:

```rust
let cache_control = format!("max-age={OCSP_CACHE_MAX_AGE_SECS}, public");
```

**Step 3 — Set `nextUpdate` in the OCSP response itself**

The OCSP response body should also include a `nextUpdate` field matching the cache TTL. This tells relying parties when to re-check, regardless of HTTP caching. Update `build_ocsp_response` in `src/ocsp.rs` to set `next_update = this_update + 300 seconds`.

**Step 4 — Consider making it configurable**

Add an optional `SettingKey::OcspCacheMaxAgeSecs` so administrators can tune the tradeoff. Default to 300.

**Step 5 — Tests**

- Test that OCSP responses include `Cache-Control: max-age=300`.
- Test that `nextUpdate` in the OCSP response is `thisUpdate + 300s`.

### Files to modify

| File | Change |
|------|--------|
| `src/routes/ocsp.rs` | Reduce `max-age` from 3600 to 300, extract constant |
| `src/ocsp.rs` | Set `nextUpdate` in OCSP response body |

---

## Plan 23: M5 — Missing Rate Limiting on OIDC Exchange/Link/Complete-Registration

### Problem

The OIDC token exchange (`/api/v1/auth/oidc/exchange`), account link (`/api/v1/auth/oidc/link`), and complete-registration (`/api/v1/auth/oidc/complete-registration`) endpoints accept secret tokens from unauthenticated clients but are not in the `RATE_LIMITS` map in `src/middleware/rate_limit.rs`. The `complete-registration` endpoint uses non-destructive `get()` (not `take()`), allowing unlimited brute-force of the registration token.

### Plan

**Step 1 — Add OIDC endpoints to the rate limit map**

```rust
static RATE_LIMITS: LazyLock<HashMap<&'static str, EndpointRateLimit>> = LazyLock::new(|| {
    HashMap::from([
        // ... existing entries ...
        (
            "/api/v1/auth/oidc/exchange",
            EndpointRateLimit {
                max_requests: 10,
                window_secs: 60,
                fail_closed: true,
            },
        ),
        (
            "/api/v1/auth/oidc/link",
            EndpointRateLimit {
                max_requests: 10,
                window_secs: 60,
                fail_closed: true,
            },
        ),
        (
            "/api/v1/auth/oidc/complete-registration",
            EndpointRateLimit {
                max_requests: 5,
                window_secs: 60,
                fail_closed: true,
            },
        ),
    ])
});
```

**Step 2 — Change `complete-registration` to use destructive `take()`**

The `complete-registration` endpoint should consume the registration token on use. If it currently uses `get()`, change to `take()` so the token is invalidated after first use:

```rust
// Before: non-destructive read allows brute-force
let pending = state.oidc_registration_store.get(&req.code).await?;

// After: destructive take — token is one-time-use
let pending = state.oidc_registration_store.take(&req.code).await?;
```

**Step 3 — Tests**

- Test that OIDC exchange is rate-limited at 10/60s.
- Test that complete-registration is rate-limited at 5/60s.
- Test that complete-registration token is consumed after first use.
- Update the `rate_limited_paths_list` test.

### Files to modify

| File | Change |
|------|--------|
| `src/middleware/rate_limit.rs` | Add OIDC endpoints to `RATE_LIMITS` |
| `src/routes/oidc_auth.rs` | Change `complete-registration` from `get()` to `take()` |

---

## Plan 24: M6 — MQTT Password Stored in Plaintext

### Problem

MQTT broker credentials are stored in plaintext in the `mqtt_clients` table (`src/mqtt_client_store.rs:104`). Unlike enrollment tokens (which are SHA-256 hashed), the `password` field is stored and transmitted as cleartext.

### Plan

**Step 1 — Encrypt MQTT passwords at rest**

Since MQTT passwords need to be recovered (sent to the MQTT service for broker connections), they cannot be one-way hashed. Use symmetric encryption with a server-side key.

Add an encryption helper using `aes-gcm`:

```rust
use aes_gcm::{Aes256Gcm, Key, Nonce};
use aes_gcm::aead::{Aead, KeyInit, OsRng};

pub struct FieldEncryptor {
    cipher: Aes256Gcm,
}

impl FieldEncryptor {
    pub fn new(key: &[u8; 32]) -> Self {
        let key = Key::<Aes256Gcm>::from_slice(key);
        Self {
            cipher: Aes256Gcm::new(key),
        }
    }

    /// Encrypt a plaintext value. Returns base64-encoded "nonce:ciphertext".
    pub fn encrypt(&self, plaintext: &str) -> Result<String, aes_gcm::Error> {
        let nonce_bytes: [u8; 12] = rand::random();
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = self.cipher.encrypt(nonce, plaintext.as_bytes())?;
        let combined = [nonce_bytes.as_slice(), ciphertext.as_slice()].concat();
        Ok(base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &combined,
        ))
    }

    /// Decrypt a base64-encoded "nonce+ciphertext" value.
    pub fn decrypt(&self, encoded: &str) -> Result<String, aes_gcm::Error> {
        let combined = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            encoded,
        )
        .map_err(|_| aes_gcm::Error)?;
        let (nonce_bytes, ciphertext) = combined.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);
        let plaintext = self.cipher.decrypt(nonce, ciphertext)?;
        Ok(String::from_utf8(plaintext).map_err(|_| aes_gcm::Error)?)
    }
}
```

**Step 2 — Store the encryption key in the database**

Similar to the JWT signing key (Plan 12), store a 32-byte encryption key in the `settings` table under `SettingKey::FieldEncryptionKey`. Generate on first use.

**Step 3 — Encrypt on create/update, decrypt on read**

In `create_mqtt_client` and `update_mqtt_client`, encrypt the password before storing:

```rust
password: Set(password.map(|p| encryptor.encrypt(p).expect("encryption"))),
```

When sending credentials to the MQTT service, decrypt:

```rust
let plaintext_password = client.password
    .as_ref()
    .map(|p| encryptor.decrypt(p))
    .transpose()?;
```

**Step 4 — Migration for existing plaintext passwords**

Add a one-time migration that encrypts all existing plaintext passwords in the `mqtt_clients` table.

**Step 5 — Tests**

- Test round-trip encrypt/decrypt produces the original value.
- Test that stored values are not plaintext.
- Test that the MQTT service receives the correct decrypted password.

### Files to modify

| File | Change |
|------|--------|
| New: `src/auth/field_encryption.rs` | `FieldEncryptor` with AES-256-GCM |
| `src/mqtt_client_store.rs` | Encrypt/decrypt password on store/load |
| `src/lib.rs` | Add `FieldEncryptor` to `AppState` |
| `src/settings_store.rs` | Add `FieldEncryptionKey` setting |
| Migration | Encrypt existing plaintext passwords |
| `Cargo.toml` | Add `aes-gcm` dependency |

---

## Plan 25: M7 — No Idle Timeout on WebSocket Connections

### Problem

All WebSocket handlers (`agent_ws.rs`, `mqtt_ws.rs`) have no `tokio::time::timeout` branch. Half-open TCP connections (where the remote end disappears without sending a close frame) persist indefinitely, consuming memory and connection registry slots.

### Plan

**Step 1 — Add an idle timeout branch to the select loop**

In both `handle_agent_authenticated` and the MQTT handler loops, add a timeout branch:

```rust
const WS_IDLE_TIMEOUT: Duration = Duration::from_secs(300); // 5 minutes

loop {
    tokio::select! {
        msg = stream.next() => {
            // ... existing message handling ...
        }
        push = push_rx.recv() => {
            // ... existing push handling ...
        }
        _ = tokio::time::sleep(WS_IDLE_TIMEOUT) => {
            tracing::info!(%service_id, "WebSocket idle timeout — disconnecting");
            let _ = close_with_reason(sink, "idle timeout").await;
            break;
        }
    }
}
```

The timeout resets on every loop iteration (any received message or push), so active connections are not affected.

**Step 2 — Use a separate `Interval` for ping expectations**

Instead of a simple sleep that resets each iteration, track the last activity and check against it:

```rust
let mut last_activity = Instant::now();
let mut idle_check = tokio::time::interval(Duration::from_secs(60));

loop {
    tokio::select! {
        msg = stream.next() => {
            last_activity = Instant::now();
            // ... existing handling ...
        }
        push = push_rx.recv() => {
            // ... existing handling ...
        }
        _ = idle_check.tick() => {
            if last_activity.elapsed() > WS_IDLE_TIMEOUT {
                tracing::info!(%service_id, "WebSocket idle timeout — disconnecting");
                let _ = close_with_reason(sink, "idle timeout").await;
                break;
            }
        }
    }
}
```

**Step 3 — Apply to all handler loops**

Apply to:
- `handle_agent_authenticated` in `agent_ws.rs`
- `run_agent_enrolled_loop` in `agent_ws.rs`
- `handle_mqtt_authenticated` in `mqtt_ws.rs`
- `handle_mqtt_enrolled` in `mqtt_ws.rs`

**Step 4 — Extract the constant**

```rust
/// Idle timeout for WebSocket connections. Connections without activity
/// (messages or pushes) for this duration are terminated.
const WS_IDLE_TIMEOUT: Duration = Duration::from_secs(300);
```

Put it in `service_ws.rs` and re-export for both agent and mqtt modules.

**Step 5 — Tests**

- Test that a connection with no messages is closed after the timeout.
- Test that a connection with periodic pings is NOT closed.
- Test that the connection registry is cleaned up after idle disconnect.

### Files to modify

| File | Change |
|------|--------|
| `src/routes/service_ws.rs` | Add `WS_IDLE_TIMEOUT` constant |
| `src/routes/agent_ws.rs` | Add idle timeout branch to both handler loops |
| `src/routes/mqtt_ws.rs` | Add idle timeout branch to both handler loops |

---

## Plan 26: M8 — Broadcast Holds Read Lock During Async Sends

### Problem

`broadcast` and `broadcast_by_type` (`src/service_connections.rs:125-140`) hold the `RwLock` read guard while iterating and calling `.send().await` on each connection. A slow consumer blocks all registry writes (register/unregister) during the entire broadcast.

### Plan

**Step 1 — Collect senders first, then send without the lock**

Clone the senders while holding the lock, then drop the lock before sending:

```rust
pub async fn broadcast(&self, msg: ControllerMessage) {
    let senders: Vec<mpsc::Sender<ControllerMessage>> = {
        let guard = self.inner.read().await;
        guard.values().map(|conn| conn.sender.clone()).collect()
    };
    // Lock is dropped here — writes are unblocked

    for sender in senders {
        let _ = sender.send(msg.clone()).await;
    }
}

pub async fn broadcast_by_type(&self, service_type: ServiceType, msg: ControllerMessage) {
    let senders: Vec<mpsc::Sender<ControllerMessage>> = {
        let guard = self.inner.read().await;
        guard
            .values()
            .filter(|conn| conn.service_type == service_type)
            .map(|conn| conn.sender.clone())
            .collect()
    };

    for sender in senders {
        let _ = sender.send(msg.clone()).await;
    }
}
```

**Step 2 — Use `try_send` instead of `send` for non-blocking broadcast**

For broadcast, we don't need to wait for each consumer. Use `try_send` which returns immediately:

```rust
for sender in senders {
    if sender.try_send(msg.clone()).is_err() {
        tracing::debug!("broadcast: channel full or closed, skipping");
    }
}
```

This makes broadcast O(N) non-blocking instead of O(N) async-blocking.

**Step 3 — Tests**

- Test that broadcast works even when one consumer is slow.
- Test that `register_agent` is not blocked during broadcast.

### Files to modify

| File | Change |
|------|--------|
| `src/service_connections.rs` | Collect senders before dropping lock, use `try_send` for broadcast |

---

## Plan 27: M9 — Relaxed Memory Ordering on Settings Version Counters

### Problem

`check_version_and_reload` (`src/settings.rs:294-295`) uses `Ordering::Relaxed` for loading version counters. On ARM/multi-core, relaxed loads can return stale values, causing missed reloads. Two separate non-atomic loads can also produce inconsistent version pairs.

### Plan

**Note:** If Plan 15 (H8) is implemented first, this issue is already fixed as part of that plan (the version counters use `Acquire/Release` ordering). This plan is for the case where H8 is not yet implemented.

**Step 1 — Change loads to `Acquire`, stores to `Release`**

```rust
// In check_version_and_reload:
let cached_version = self.inner.version.load(Ordering::Acquire);
let cached_global_version = self.inner.global_version.load(Ordering::Acquire);

// In reload_from_db:
self.inner.version.store(version, Ordering::Release);
self.inner.global_version.store(global_version, Ordering::Release);
```

`Acquire` on load ensures the reader sees all writes that happened before the corresponding `Release` store.

**Step 2 — Consider reading both counters atomically**

The two separate `load()` calls can produce an inconsistent pair (e.g., new version but old global_version). This can cause spurious reloads but not correctness issues (a reload is idempotent). If this is a concern, pack both counters into a single `AtomicU64`:

```rust
/// Pack (version: i32, global_version: i32) into a single u64.
fn pack_versions(version: i64, global_version: i64) -> u64 {
    ((version as u32 as u64) << 32) | (global_version as u32 as u64)
}
fn unpack_versions(packed: u64) -> (i64, i64) {
    ((packed >> 32) as i32 as i64, (packed & 0xFFFF_FFFF) as i32 as i64)
}
```

However, this limits each counter to i32 range. Since version counters are monotonically increasing and rarely exceed ~10^6 in practice, this is acceptable. Alternatively, keep the two separate atomics with `Acquire/Release` — the worst case is a spurious reload which is harmless.

**Recommended:** Just fix the ordering to `Acquire/Release`. The spurious reload from non-atomic pair reads is harmless.

**Step 3 — Tests**

- Verify the ordering constants are correct in the source code.
- No functional tests needed since this is a correctness/memory-model fix.

### Files to modify

| File | Change |
|------|--------|
| `src/settings.rs` | Change `Ordering::Relaxed` to `Ordering::Acquire` on loads, verify `Release` on stores |

---

## Plan 28: M10 — `cert_signed_by_ca` Only Compares DN, Not Cryptographic Signature

### Problem

`cert_signed_by_ca` (`src/pki_utils.rs:84-98`) only compares the certificate's issuer DN against the CA's subject DN. A certificate with a matching issuer DN but a forged signature passes the check. An attacker with access to create certificates (e.g., their own CA with the same DN) can produce certificates that pass this check.

### Plan

**Step 1 — Add cryptographic signature verification**

Use the CA's public key to verify the certificate's signature:

```rust
pub fn cert_signed_by_ca(cert_pem: &str, ca_pem: &str) -> Result<bool> {
    let (_, cert_block) = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes())
        .map_err(|_| report!(PkiUtilError::PemParse))?;
    let cert = cert_block
        .parse_x509()
        .map_err(|_| report!(PkiUtilError::PemParse))?;

    let (_, ca_block) = x509_parser::pem::parse_x509_pem(ca_pem.as_bytes())
        .map_err(|_| report!(PkiUtilError::PemParse))?;
    let ca = ca_block
        .parse_x509()
        .map_err(|_| report!(PkiUtilError::PemParse))?;

    // Check issuer DN matches subject DN (quick reject)
    if cert.issuer() != ca.subject() {
        return Ok(false);
    }

    // Cryptographically verify the certificate's signature using the CA's public key
    match cert.verify_signature(Some(ca.public_key())) {
        Ok(()) => Ok(true),
        Err(_) => Ok(false),
    }
}
```

The `x509_parser` crate's `verify_signature` method performs the actual cryptographic verification.

**Step 2 — Handle algorithm mismatches**

`verify_signature` returns an error if the algorithm is unsupported. Treat unsupported algorithms as verification failure (return `false`) rather than propagating the error:

```rust
match cert.verify_signature(Some(ca.public_key())) {
    Ok(()) => Ok(true),
    Err(x509_parser::error::X509Error::SignatureVerificationError) => Ok(false),
    Err(e) => {
        tracing::warn!(error = %e, "certificate signature verification failed unexpectedly");
        Ok(false)
    }
}
```

**Step 3 — Tests**

- Existing test `cert_signed_by_ca_same_ca` should still pass (valid signature).
- Existing test `cert_signed_by_ca_different_ca` should still pass (DN mismatch).
- Add new test: create two CAs with the same DN but different keys, sign a cert with CA1, verify against CA2 — must return `false` (DN matches but signature doesn't).

```rust
#[test]
fn cert_signed_by_ca_same_dn_different_key() {
    // CA1
    let key1 = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let mut ca1_params = rcgen::CertificateParams::default();
    ca1_params.distinguished_name.push(rcgen::DnType::CommonName, "Same CA");
    ca1_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let _ca1_cert = ca1_params.self_signed(&key1).unwrap();
    let issuer1 = rcgen::Issuer::new(ca1_params, key1);

    // Sign cert with CA1
    let server_key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let server_params = rcgen::CertificateParams::new(vec!["localhost".into()]).unwrap();
    let server_cert = server_params.signed_by(&server_key, &issuer1).unwrap();
    let server_pem = server_cert.pem();

    // CA2 with same DN, different key
    let key2 = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let mut ca2_params = rcgen::CertificateParams::default();
    ca2_params.distinguished_name.push(rcgen::DnType::CommonName, "Same CA");
    ca2_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca2_cert = ca2_params.self_signed(&key2).unwrap();
    let ca2_pem = ca2_cert.pem();

    // DN matches, but signature was made by CA1 — must return false
    assert!(!cert_signed_by_ca(&server_pem, &ca2_pem).unwrap());
}
```

### Files to modify

| File | Change |
|------|--------|
| `src/pki_utils.rs` | Add `verify_signature` call after DN comparison |

---

## Plan 29: M11 — No Max Password Length — Argon2 DoS Vector

### Problem

The `register` handler (`src/routes/auth.rs:48-54`) validates a minimum password length (8 chars) but has no upper bound. An attacker can submit a multi-megabyte password, causing argon2 to consume excessive CPU and memory.

### Plan

**Step 1 — Add a maximum password length constant**

```rust
const MIN_PASSWORD_LENGTH: usize = 8;
const MAX_PASSWORD_LENGTH: usize = 1024;
```

1024 bytes is generous for any reasonable password (including passphrases) while preventing abuse.

**Step 2 — Validate max length in register and login**

In the `register` handler:

```rust
if req.password.len() < MIN_PASSWORD_LENGTH {
    return error_response(StatusCode::BAD_REQUEST, "Password must be at least 8 characters");
}
if req.password.len() > MAX_PASSWORD_LENGTH {
    return error_response(StatusCode::BAD_REQUEST, "Password must not exceed 1024 characters");
}
```

In the `login` handler, add the same max-length check before calling `verify_password`:

```rust
if req.password.len() > MAX_PASSWORD_LENGTH {
    return error_response(StatusCode::UNAUTHORIZED, "Invalid credentials");
}
```

Note: The login response intentionally doesn't reveal the specific validation failure (same generic message as wrong password).

**Step 3 — Also check in password change endpoint**

If there's a password change endpoint, apply the same validation.

**Step 4 — Also check OIDC link endpoint**

The `oidc_link` endpoint accepts a password for account linking. Apply the same limit.

**Step 5 — Tests**

- Test that registration with a 2000-char password is rejected with 400.
- Test that login with a 2000-char password is rejected (not processed by argon2).
- Test that passwords up to 1024 chars are accepted.

### Files to modify

| File | Change |
|------|--------|
| `src/routes/auth.rs` | Add `MAX_PASSWORD_LENGTH` check to `register` and `login` |
| `src/routes/oidc_auth.rs` | Add max-length check to `oidc_link` |

---

## Plan 30: M12 — Email Not Normalized

### Problem

The `register` handler (`src/routes/auth.rs:39-82`) uses case-sensitive email comparison. `User@Example.com` and `user@example.com` are treated as different addresses, allowing duplicate registrations for the same logical email. This can also cause OIDC auto-link to fail when email casing differs between the provider and stored value.

### Plan

**Step 1 — Normalize email on input**

Add an `normalize_email` helper:

```rust
/// Normalize an email address for storage and comparison.
///
/// - Trims whitespace
/// - Lowercases the entire address (RFC 5321 says the local part is
///   case-sensitive in theory, but in practice all major providers
///   treat it as case-insensitive)
fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}
```

**Step 2 — Apply normalization at all entry points**

Normalize emails before any comparison or storage:

In `register`:
```rust
let email = normalize_email(&req.email);
```

In `login`:
```rust
let email = normalize_email(&req.email);
```

In `resolve_oidc_user`:
```rust
let email = normalize_email(email);
```

In `oidc_link`:
```rust
let email = normalize_email(&req.email);
```

**Step 3 — Normalize existing data (migration)**

Add a migration that lowercases all existing `user.email` values:

```sql
UPDATE users SET email = LOWER(TRIM(email));
```

**Step 4 — Add a unique index on normalized email**

Ensure the database enforces uniqueness on the normalized email:

```sql
CREATE UNIQUE INDEX idx_users_email_lower ON users (LOWER(email));
```

Or, since we always store the normalized form, the existing unique index on `email` is sufficient.

**Step 5 — Tests**

- Test that `normalize_email("User@Example.COM ")` returns `"user@example.com"`.
- Test that registering with `User@Example.com` after `user@example.com` is rejected as duplicate.
- Test that OIDC login with `User@example.com` matches stored `user@example.com`.

### Files to modify

| File | Change |
|------|--------|
| `src/routes/auth.rs` | Add `normalize_email`, apply to `register` and `login` |
| `src/auth/authentication.rs` | Normalize email in `resolve_oidc_user` |
| `src/routes/oidc_auth.rs` | Normalize email in `oidc_link` |
| Migration | Lowercase existing emails |
