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
