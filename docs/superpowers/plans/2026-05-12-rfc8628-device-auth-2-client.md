<!-- markdownlint-disable MD013 -->

# RFC 8628 Device Auth — Plan 2 (Client) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate the openapi-client crate, the CLI's `auth login` command, and the frontend `/device` approval page to the new RFC 8628 wire surface landed by Plan 1.

**Architecture:** The hand-written `uptrakit-openapi-client` gains a `post_form_unauth` helper and a typed `ClientError::OAuthError(OAuthErrorResponse)` variant. Legacy `device_auth_start`/`device_auth_poll`/`stream_device_auth` are deleted; new `oauth_device_authorization`/`oauth_token`/`oauth_authorization_server_metadata`/`device_auth_deny`/`device_auth_lookup` are added. The CLI's `auth.rs` swaps to the form-urlencoded path, drops the SSE branch, opens `verification_uri_complete` in the browser, and maps the typed `OAuthErrorCode` onto user-facing messages. The frontend `/device` page renames `?code=` → `?user_code=`, adds a Deny button, and fetches the lookup endpoint on load via native `fetch` (no TS client crate).

**Tech Stack:** Rust + `reqwest` (openapi-client + CLI), Svelte/SvelteKit + native `fetch` (frontend), Playwright (frontend tests).

**Spec:** `docs/superpowers/specs/2026-05-12-rfc8628-device-auth-design.md` (commit `2ab437436`).

**Dependencies:** Plan 1 (`docs/superpowers/plans/2026-05-12-rfc8628-device-auth-1-backend.md`) must be merged first. The new backend routes are the wire that this plan consumes.

---

## File map

### Modify

- `crates/shared/openapi-client/src/lib.rs` — add `post_form_unauth` helper; drop the SSE module reference.
- `crates/shared/openapi-client/src/error.rs` — add `ClientError::OAuthError(OAuthErrorResponse)` variant.
- `crates/shared/openapi-client/src/auth.rs` — delete `device_auth_start`/`device_auth_poll`; add `oauth_device_authorization`/`oauth_token`/`oauth_authorization_server_metadata`/`device_auth_deny`/`device_auth_lookup`; refresh `*_request_serialization` tests.
- `crates/shared/openapi-client/src/paths.rs` (or the existing path-constants file) — add new path constants, remove deleted ones.
- `crates/ui/cli/src/commands/auth.rs` — rewrite `login` against the new wire; delete SSE branch; map `OAuthErrorCode` to user messages.
- `frontend/src/routes/device/+page.svelte` — rename `?code=` → `?user_code=`; add Deny button; fetch lookup on load; render `client_name`.
- `frontend/src/lib/api.ts` (or wherever `approveDeviceAuth` lives) — rename to use the surviving `POST /api/v1/auth/device/approve`; add `denyDeviceAuth`, `lookupDeviceAuth` helpers.
- Frontend Playwright spec for `/device`.

### Delete

- `crates/shared/openapi-client/src/device_auth_stream.rs` (entire file).

### Documentation

- Public-type Rustdoc comments citing RFC sections (incremental across the public surface added in Plan 1).
- README / `docs/**/*.md` grep recheck (verify no stale references to `verification_url`, `?code=`, `/api/v1/auth/device`).
- `docs/development/openapi-client.md` — verify still current; no changes expected unless this plan exposes a stale claim.

---

## Conventions referenced throughout

All citations point to `.superpowers/standards-snapshot.md` unless otherwise noted.

- **Errors:** `rootcause::Report` + `report!()` / `bail!()`. No `.unwrap()` in production CLI/client code.
- **Wire-safe enums:** consume `OAuthErrorCode` exactly as defined by the `wire_safe_enum!` macro — match on typed variants, fall through to `Other(s)` for forward compat. Never re-define the enum.
- **Form bodies:** use `reqwest::RequestBuilder::form(&body)` (sets `Content-Type: application/x-www-form-urlencoded` automatically). Never hand-build URL-encoded strings.
- **CLI UX:** preserve the existing URL-scheme validation, browser-open helpers, and stderr-prompts pattern.
- **Frontend:** native `fetch`; no new generator pipelines. JSON bodies for `/approve`, `/deny`; query string for `/lookup`.

Commit style: Conventional Commits. Scope examples used below: `feat(openapi-client)`, `feat(cli)`, `feat(frontend)`, `test(cli)`, `test(frontend)`.

---

## Task 1: `post_form_unauth` helper + `ClientError::OAuthError`

**Files:**

- Modify: `crates/shared/openapi-client/src/lib.rs`
- Modify: `crates/shared/openapi-client/src/error.rs`

The form helper mirrors `post_json_unauth` (line 352) but uses `reqwest::RequestBuilder::form(...)` and handles the RFC 8628 §3.5 / RFC 6749 §5.2 400-response shape: on a 400 with a parseable `OAuthErrorResponse` body, return `Err(ClientError::OAuthError(resp))`; on a 200 with the expected JSON, return `Ok(parsed)`; on anything else, fall through to the existing `handle_response` path.

- [ ] **Step 1: Add the typed variant**

In `crates/shared/openapi-client/src/error.rs`, extend `ClientError`:

```rust
use uptrakit_web_api_types::oauth::OAuthErrorResponse;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    // ... existing variants ...

    /// RFC 6749 §5.2 / RFC 8628 §3.5 error response from an OAuth endpoint.
    #[error("OAuth error: {0:?}")]
    OAuthError(OAuthErrorResponse),

    // ... existing variants (Api, RateLimited, NotFound, NotAuthenticated, InvalidMethod) ...
}
```

Make sure the variant ordering keeps `OAuthError` non-`#[from]` (it must be constructed explicitly) and that `OAuthErrorResponse` is reachable — add `use uptrakit_web_api_types::oauth::OAuthErrorResponse;` at the top of the file.

- [ ] **Step 2: Add the `post_form_unauth` helper**

In `crates/shared/openapi-client/src/lib.rs`, immediately after `post_json_unauth` (line 352), add:

```rust
/// POST a form-urlencoded body without authentication (OAuth endpoints).
///
/// On HTTP 200, deserialise the response body as JSON into `T`.
/// On HTTP 400 with a parseable RFC 6749 §5.2 `OAuthErrorResponse` body,
/// return `Err(ClientError::OAuthError(...))`. Any other status falls through
/// to the shared `handle_response` path (which surfaces `RateLimited`,
/// `NotAuthenticated`, `Api { status, message }`, etc.).
async fn post_form_unauth<T: DeserializeOwned, F: Serialize + ?Sized>(
    &self,
    path: &str,
    form: &F,
) -> Result<T> {
    let url = format!("{}{}", self.base_url, path);
    let req = self.http.post(&url).form(form);
    let resp = self.send_with_retry(req).await?;

    let status = resp.status();
    if status.as_u16() == 400 {
        let bytes = resp.bytes().await.context_to()?;
        if let Ok(err_resp) =
            serde_json::from_slice::<uptrakit_web_api_types::oauth::OAuthErrorResponse>(&bytes)
        {
            return Err(rootcause::Report::from(ClientError::OAuthError(err_resp)));
        }
        // Body did not parse as an RFC error envelope — fall back to generic API error.
        let message = String::from_utf8_lossy(&bytes).to_string();
        bail!(ClientError::Api { status, message });
    }

    // For non-400 responses, hand off to the existing handler.
    let resp = reqwest::Response::from(resp); // already a Response; no-op clarifier
    // The actual existing helper takes a `reqwest::Response`. The simplest path
    // is to call `handle_response(resp).await` directly:
    self.handle_response(resp).await
}
```

> **Note.** `send_with_retry` returns a `reqwest::Response`. If the existing
> code uses an intermediate wrapper, adapt the variable types; the helper's
> contract is: form-POST → 200 means deserialise as `T`; 400 means try to
> parse `OAuthErrorResponse`. Read the surrounding helpers (lines 340–510)
> before finalising the body — there are likely shared response-handler
> patterns that this helper should hook into rather than duplicating.

- [ ] **Step 3: Compile**

Run: `cargo check -p uptrakit-openapi-client --all-features`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/shared/openapi-client/src/lib.rs crates/shared/openapi-client/src/error.rs
git commit -m "feat(openapi-client): add post_form_unauth helper and ClientError::OAuthError variant"
```

---

## Task 2: Update path constants

**Files:**

- Modify: `crates/shared/openapi-client/src/paths.rs` (find the `auth` submodule).

- [ ] **Step 1: Replace the device-auth paths**

In `crates/shared/openapi-client/src/paths.rs` under `pub mod auth { ... }`, replace the device-auth path constants:

```rust
pub mod auth {
    // ... unchanged constants for LOGIN, REGISTER, REFRESH, LOGOUT, METHODS, ME, ...

    // Internal UI-only:
    pub const DEVICE_APPROVE: &str = "/api/v1/auth/device/approve";
    pub const DEVICE_DENY: &str = "/api/v1/auth/device/deny";
    pub const DEVICE_LOOKUP: &str = "/api/v1/auth/device/lookup";

    // RFC 8628 / RFC 8414:
    pub const OAUTH_DEVICE_AUTHORIZATION: &str = "/api/v1/oauth/device_authorization";
    pub const OAUTH_TOKEN: &str = "/api/v1/oauth/token";
    pub const OAUTH_METADATA: &str = "/.well-known/oauth-authorization-server";
}
```

Delete the old `DEVICE` (start) and `DEVICE_POLL` constants. If a `DEVICE_STREAM` constant exists, delete it too.

- [ ] **Step 2: Compile**

Run: `cargo check -p uptrakit-openapi-client --all-features`
Expected: build fails with errors at every consumer of the deleted constants. Tasks 3, 4 below fix them.

- [ ] **Step 3: Commit (with the compile error)**

```bash
git add crates/shared/openapi-client/src/paths.rs
git commit -m "feat(openapi-client): rename device-auth paths to RFC 8628 names (transient build break)"
```

(The transient break is intentional — the next task removes every reference to the deleted symbols. Keep it on the feature branch only.)

---

## Task 3: Rewrite `openapi-client/src/auth.rs`

**Files:**

- Modify: `crates/shared/openapi-client/src/auth.rs`

Delete `device_auth_start`/`device_auth_poll`; add the five new methods. Keep `device_auth_approve`. The Rustdoc on each new method cites the relevant RFC section.

- [ ] **Step 1: Delete the legacy methods and update imports**

Replace the top-of-file imports:

```rust
use crate::Result;
use crate::UptrakitClient;
use crate::types_impl::auth::{
    AuthResponse, LoginRequest, LogoutRequest, RefreshRequest, RefreshResponse, RegisterRequest,
    UserResponse,
};
use crate::types_impl::device_auth::{
    DeviceAuthApproveRequest, DeviceAuthApproveResponse, DeviceAuthDenyRequest,
    DeviceAuthDenyResponse,
};
use crate::types_impl::oauth::{
    DeviceAuthLookupQuery, DeviceAuthLookupResponse, DeviceAuthorizationRequest,
    DeviceAuthorizationResponse, OAuthAuthorizationServerMetadata, OAuthTokenRequest,
    OAuthTokenResponse,
};
use crate::types_impl::oidc_auth::AuthMethodsResponse;
```

Delete the `device_auth_start` and `device_auth_poll` methods.

- [ ] **Step 2: Add the five new methods**

```rust
impl UptrakitClient {
    /// Start an RFC 8628 device authorization flow.
    ///
    /// Per RFC 8628 §3.1. Returns the device_code, user_code, verification URIs,
    /// expiry, and recommended polling interval. This endpoint does not require
    /// authentication.
    pub async fn oauth_device_authorization(
        &self,
        req: &DeviceAuthorizationRequest,
    ) -> Result<DeviceAuthorizationResponse> {
        self.post_form_unauth(crate::paths::auth::OAUTH_DEVICE_AUTHORIZATION, req)
            .await
    }

    /// Exchange a device_code for an access token.
    ///
    /// Per RFC 6749 §3.2 / RFC 8628 §3.4. Form-urlencoded body. On HTTP 400 the
    /// caller receives `Err(ClientError::OAuthError(OAuthErrorResponse))` with
    /// the typed `OAuthErrorCode`. This endpoint does not require
    /// authentication.
    pub async fn oauth_token(&self, req: &OAuthTokenRequest) -> Result<OAuthTokenResponse> {
        self.post_form_unauth(crate::paths::auth::OAUTH_TOKEN, req)
            .await
    }

    /// Fetch the RFC 8414 §3 authorization server metadata document.
    ///
    /// Public; no authentication required.
    pub async fn oauth_authorization_server_metadata(
        &self,
    ) -> Result<OAuthAuthorizationServerMetadata> {
        self.get_unauth(crate::paths::auth::OAUTH_METADATA).await
    }

    /// Approve a pending device authorization request (UI-internal).
    pub async fn device_auth_approve(
        &self,
        req: &DeviceAuthApproveRequest,
    ) -> Result<DeviceAuthApproveResponse> {
        self.post_json(crate::paths::auth::DEVICE_APPROVE, req)
            .await
    }

    /// Deny a pending device authorization request (UI-internal).
    pub async fn device_auth_deny(
        &self,
        req: &DeviceAuthDenyRequest,
    ) -> Result<DeviceAuthDenyResponse> {
        self.post_json(crate::paths::auth::DEVICE_DENY, req).await
    }

    /// Look up the `client_name` + `expires_at` for a pending flow (UI-internal).
    pub async fn device_auth_lookup(
        &self,
        query: &DeviceAuthLookupQuery,
    ) -> Result<DeviceAuthLookupResponse> {
        let url = format!(
            "{}{}?user_code={}",
            self.base_url,
            crate::paths::auth::DEVICE_LOOKUP,
            urlencoding::encode(&query.user_code),
        );
        // Use a small inline GET — or, if a helper exists, prefer
        // self.get_with_query(crate::paths::auth::DEVICE_LOOKUP, query).await.
        // Check `lib.rs` for an existing authenticated-GET-with-query helper.
        let req = self
            .http
            .get(&url)
            .bearer_auth(self.token_or_err()?);
        let resp = self.send_with_retry(req).await?;
        self.handle_response(resp).await
    }
}
```

(The `device_auth_lookup` request requires authentication — note the `bearer_auth(self.token_or_err()?)` call, in contrast to the OAuth endpoints which are unauth.)

- [ ] **Step 3: Refresh the serialisation tests**

In the `mod tests` block, delete `device_auth_start_request_serialization` and `device_auth_poll_request_serialization`. Add:

```rust
#[test]
fn device_authorization_request_form_serialization() {
    use serde_urlencoded;
    let req = DeviceAuthorizationRequest {
        client_id: "uptrakit-cli".into(),
        scope: None,
        client_name: Some("cli-host-2026-05-12".into()),
    };
    let encoded = serde_urlencoded::to_string(&req).expect("encode");
    // Form encoding is order-dependent in serde_urlencoded; check substrings.
    assert!(encoded.contains("client_id=uptrakit-cli"));
    assert!(encoded.contains("client_name=cli-host-2026-05-12"));
    assert!(!encoded.contains("scope="), "scope omitted when None");
}

#[test]
fn oauth_token_request_form_serialization() {
    use serde_urlencoded;
    let req = OAuthTokenRequest {
        grant_type: "urn:ietf:params:oauth:grant-type:device_code".into(),
        device_code: Some("abc-123".into()),
        client_id: Some("uptrakit-cli".into()),
    };
    let encoded = serde_urlencoded::to_string(&req).expect("encode");
    assert!(encoded.contains("grant_type=urn"), "grant_type URI preserved verbatim");
    assert!(encoded.contains("device_code=abc-123"));
    assert!(encoded.contains("client_id=uptrakit-cli"));
}

#[test]
fn device_auth_deny_request_serialization() {
    let req = DeviceAuthDenyRequest {
        user_code: "ABCD-EFGH".into(),
    };
    let json = serde_json::to_value(&req).expect("serialize");
    assert_eq!(json["user_code"], "ABCD-EFGH");
}
```

Keep `device_auth_approve_request_serialization`.

- [ ] **Step 4: Compile + test**

Run: `cargo test -p uptrakit-openapi-client --all-features -- auth`
Expected: every new test passes.

- [ ] **Step 5: Commit**

```bash
git add crates/shared/openapi-client/src/auth.rs
git commit -m "feat(openapi-client): swap device_auth_{start,poll} for OAuth 2.0 methods"
```

---

## Task 4: Delete `device_auth_stream.rs`

**Files:**

- Delete: `crates/shared/openapi-client/src/device_auth_stream.rs`
- Modify: `crates/shared/openapi-client/src/lib.rs`

- [ ] **Step 1: Remove the file and the module reference**

```bash
git rm crates/shared/openapi-client/src/device_auth_stream.rs
```

In `crates/shared/openapi-client/src/lib.rs`, delete the `pub mod device_auth_stream;` line (and the corresponding `pub use device_auth_stream::*;` re-export, if any).

If the workspace `Cargo.toml` lists feature flags or dependencies (`futures-util`, `eventsource-stream`, etc.) that were only used by the SSE module, also remove them. Verify with:

```bash
grep -rn "device_auth_stream\|DeviceAuthSseEvent" crates/ --include='*.rs'
```

Expected after cleanup: no matches (except the CLI's `auth.rs`, which Task 5 handles).

- [ ] **Step 2: Compile**

Run: `cargo check -p uptrakit-openapi-client --all-features`
Expected: clean (with the CLI not yet updated, the CLI crate will fail to build — that's fine for this commit which is scoped to openapi-client).

- [ ] **Step 3: Commit**

```bash
git add -A crates/shared/openapi-client/
git commit -m "chore(openapi-client): delete device_auth_stream module (SSE endpoint removed)"
```

---

## Task 5: CLI `auth login` rewrite

**Files:**

- Modify: `crates/ui/cli/src/commands/auth.rs`

Drop the SSE branch entirely. The polling path becomes the only path. The 400-response handler matches on `OAuthErrorCode` for typed branching.

- [ ] **Step 1: Update imports**

Replace the top imports in `crates/ui/cli/src/commands/auth.rs`:

```rust
use crate::client::{UptrakitClient, resolve_server_and_token};
use crate::commands::CliContext;
use crate::config::{Config, Credentials, load_config, save_config, save_credentials};
use crate::error::{CliError, Result};
use crate::output::HumanOutput;
use clap::Subcommand;
use rootcause::prelude::*;
use serde::Serialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uptrakit_openapi_client::ClientError;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::api_tokens::CreateApiTokenRequest;
use uptrakit_openapi_client::types::oauth::{
    DeviceAuthorizationRequest, DeviceAuthorizationResponse, OAuthErrorCode,
    OAuthTokenRequest,
};
```

(Delete: `futures_util::StreamExt`, `uptrakit_openapi_client::device_auth_stream::DeviceAuthSseEvent`, the old `DeviceAuthPollRequest`/`DeviceAuthStartRequest` imports.)

- [ ] **Step 2: Define the client_id constant**

After the existing `chronos_date` and `TokenStatus`/`TokenEntry` types, add:

```rust
const CLI_CLIENT_ID: &str = "uptrakit-cli";

const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
```

- [ ] **Step 3: Rewrite the `login` function**

Replace the body of `pub async fn login(...)`:

```rust
pub async fn login(server_override: Option<&str>, insecure: bool) -> Result<()> {
    // Resolve server URL (unchanged from previous body).
    let config = load_config()?;
    let server = if let Some(s) = server_override {
        s.to_string()
    } else if let Some(s) = &config.server {
        let input = prompt(&format!("Server URL [{}]: ", s))?;
        if input.is_empty() { s.clone() } else { input }
    } else {
        prompt("Server URL: ")?
    };
    if server.is_empty() {
        bail!(CliError::Other("Server URL is required".into()));
    }

    let host = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let date = chrono_date();
    let client_name = format!("cli-{host}-{date}");

    let client = UptrakitClient::new(&server, None, insecure, None).context_to()?;

    // Start the device authorization flow.
    let start_resp = client
        .oauth_device_authorization(&DeviceAuthorizationRequest {
            client_id: CLI_CLIENT_ID.to_string(),
            scope: None,
            client_name: Some(client_name.clone()),
        })
        .await
        .context_to()?;

    print_browser_instructions(&start_resp, insecure);

    eprintln!("  Waiting for authorization...");

    // Poll the token endpoint at the recommended interval until success or
    // a terminal error.
    poll_for_token(&server, &start_resp, &client_name, insecure).await
}

fn print_browser_instructions(start_resp: &DeviceAuthorizationResponse, insecure: bool) {
    eprintln!();
    eprintln!("  Open this URL in your browser:");
    eprintln!("  {}", start_resp.verification_uri);
    eprintln!();
    eprintln!("  And enter this code: {}", start_resp.user_code);
    eprintln!();

    // Prefer the pre-filled URL but fall back to the plain one if the scheme
    // check rejects it.
    let url_to_open = &start_resp.verification_uri_complete;
    if let Err(e) = validate_url_scheme(url_to_open, insecure) {
        eprintln!("  (URL validation failed: {})", e);
        eprintln!("  Please verify and open the URL above manually.");
        eprintln!();
    } else if let Err(e) = open_url(url_to_open) {
        eprintln!("  (Could not open browser automatically: {})", e);
        eprintln!("  Please open the URL above manually.");
        eprintln!();
    }
}

async fn poll_for_token(
    server: &str,
    start_resp: &DeviceAuthorizationResponse,
    client_name: &str,
    insecure: bool,
) -> Result<()> {
    let client = UptrakitClient::new(server, None, insecure, None).context_to()?;
    let started = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(start_resp.expires_in);
    let mut interval = u64::try_from(start_resp.interval.max(1)).unwrap_or(5);

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;

        if started.elapsed() > timeout {
            bail!(CliError::Other(
                "Authorization request expired, please run again".into()
            ));
        }

        let req = OAuthTokenRequest {
            grant_type: DEVICE_CODE_GRANT.to_string(),
            device_code: Some(start_resp.device_code.clone()),
            client_id: Some(CLI_CLIENT_ID.to_string()),
        };

        match client.oauth_token(&req).await {
            Ok(resp) => {
                save_config(&Config { server: Some(server.to_string()) }).await?;
                save_credentials(&Credentials { token: Some(resp.access_token) }).await?;
                eprintln!();
                println!("Logged in to {} successfully.", server);
                println!("API token stored locally (name: {}).", client_name);
                return Ok(());
            }
            Err(e) => match e.current_context() {
                ClientError::OAuthError(err_resp) => match &err_resp.error {
                    OAuthErrorCode::AuthorizationPending => continue,
                    OAuthErrorCode::SlowDown => {
                        let bumped = err_resp
                            .interval
                            .and_then(|i| u64::try_from(i).ok())
                            .unwrap_or(interval.saturating_add(5));
                        interval = bumped;
                        continue;
                    }
                    OAuthErrorCode::AccessDenied => {
                        bail!(CliError::Other("Authorization denied by Operator.".into()));
                    }
                    OAuthErrorCode::ExpiredToken => {
                        bail!(CliError::Other(
                            "Authorization request expired, please run again.".into()
                        ));
                    }
                    OAuthErrorCode::InvalidGrant
                    | OAuthErrorCode::InvalidClient
                    | OAuthErrorCode::InvalidRequest
                    | OAuthErrorCode::UnsupportedGrantType => {
                        bail!(CliError::Other(format!(
                            "CLI/server version mismatch: {}",
                            err_resp.error.as_str()
                        )));
                    }
                    OAuthErrorCode::Other(s) => {
                        bail!(CliError::Other(format!("Unexpected OAuth error: {s}")));
                    }
                },
                ClientError::RateLimited { retry_after_seconds } => {
                    let delay = retry_after_seconds.unwrap_or(interval);
                    tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                    continue;
                }
                _ => return Err(e.context_to()),
            },
        }
    }
}
```

- [ ] **Step 4: Delete the SSE branch (`try_sse_login`) and the old `poll_for_authorization`**

These are replaced by `poll_for_token`. Delete both functions entirely.

- [ ] **Step 5: Compile + run existing tests**

Run: `cargo check -p uptrakit-cli --all-features && cargo test -p uptrakit-cli --all-features -- auth`
Expected: clean compile; all CLI tests pass. The URL-validation tests and human-output tests in this file should continue to pass unchanged.

- [ ] **Step 6: Commit**

```bash
git add crates/ui/cli/src/commands/auth.rs
git commit -m "feat(cli): rewrite auth login against RFC 8628 token endpoint with typed error matching"
```

---

## Task 6: CLI integration tests for the new error map

**Files:**

- Modify: `crates/ui/cli/src/commands/auth.rs` (test module) or `crates/ui/cli/tests/auth_command.rs`.

Pure-unit tests against the polling state machine where possible. If the existing CLI test harness has no mock server, add a small `wiremock`-driven integration test or skip with an explicit gap-note.

- [ ] **Step 1: Survey what exists**

Run: `find crates/ui/cli/tests -type f -name '*.rs' && grep -rn 'wiremock\|mockito\|httpmock' crates/ui/cli/`

If a mock server is already wired:

- [ ] **Step 2a: Add three tests**

Append to the existing tests file (or a new one) using the project's existing mock-server pattern:

```rust
// auth_command_handles_slow_down — start a flow on the mock, queue two 400
// responses with error="slow_down" + interval=10 followed by a 200 success.
// Drive the CLI; assert exit 0 and that two SlowDown branches were taken.

// auth_command_handles_access_denied — return a 400 with error="access_denied"
// on the first poll. Assert the CLI exits non-zero with the message
// "Authorization denied by Operator."

// auth_command_opens_verification_uri_complete_when_present — start a flow on
// the mock; assert the browser-open hook is called with verification_uri_complete
// (use a stubbed `open_url` for the test build).
```

Otherwise:

- [ ] **Step 2b: Document the gap**

If the CLI test harness has no mock-server, add a `// TODO: cover via mock server once the harness lands` comment at the top of `auth.rs` and skip this task. Do not invent a mock harness here — that's its own scope.

- [ ] **Step 3: Run tests**

Run: `cargo test -p uptrakit-cli --all-features -- auth`
Expected: pass.

- [ ] **Step 4: Commit**

```bash
git add crates/ui/cli/
git commit -m "test(cli): cover slow_down / access_denied / verification_uri_complete flows"
```

(If 2b was taken, skip this commit.)

---

## Task 7: Frontend `/device` page rewrite

**Files:**

- Modify: `frontend/src/routes/device/+page.svelte`
- Modify: `frontend/src/lib/api.ts` (or its equivalent — find with `grep -rn approveDeviceAuth frontend/src/lib`)

`?code=` → `?user_code=`. Deny button. Lookup-on-load to display `client_name`.

- [ ] **Step 1: Update the API helpers**

Locate the file exporting `approveDeviceAuth`:

```bash
grep -rn "approveDeviceAuth" frontend/src/lib
```

In that file, keep `approveDeviceAuth(user_code)` (still hits `POST /api/v1/auth/device/approve`). Add two new helpers next to it:

```typescript
export async function denyDeviceAuth(user_code: string): Promise<void> {
  const res = await fetch("/api/v1/auth/device/deny", {
    method: "POST",
    headers: { "content-type": "application/json" },
    credentials: "include",
    body: JSON.stringify({ user_code }),
  });
  if (!res.ok) {
    throw new Error(`Failed to deny device authorization (${res.status})`);
  }
}

export interface DeviceLookup {
  client_name: string | null;
  expires_at: string; // RFC 3339
}

export async function lookupDeviceAuth(
  user_code: string,
): Promise<DeviceLookup> {
  const qs = new URLSearchParams({ user_code });
  const res = await fetch(`/api/v1/auth/device/lookup?${qs.toString()}`, {
    method: "GET",
    credentials: "include",
  });
  if (!res.ok) {
    throw new Error(`Failed to look up device (${res.status})`);
  }
  return res.json();
}
```

- [ ] **Step 2: Rewrite the page**

Replace `frontend/src/routes/device/+page.svelte`:

```svelte
<script lang="ts">
    import { page } from '$app/state';
    import { approveDeviceAuth, denyDeviceAuth, lookupDeviceAuth, type DeviceLookup } from '$lib/api';
    import { getLoading, getUser } from '$lib/auth.svelte';
    import { Callout } from '$lib/components/ui';
    import PublicEntryShell from '$lib/components/ui/PublicEntryShell.svelte';
    import Button from '$lib/components/Button.svelte';

    let error = $state('');
    let success = $state(false);
    let denied = $state(false);
    let processing = $state(false);
    let lookup = $state<DeviceLookup | null>(null);

    const DEVICE_CODE_PATTERN = /^[BCDFGHJKLMNPQRSTVWXZ]{4}-[BCDFGHJKLMNPQRSTVWXZ]{4}$/;
    let rawCode = $derived(page.url.searchParams.get('user_code') || '');
    let code = $derived(DEVICE_CODE_PATTERN.test(rawCode) ? rawCode : '');
    let invalidCode = $derived(rawCode !== '' && code === '');
    let isLoggedIn = $derived(!!getUser());

    $effect(() => {
        if (code && isLoggedIn && !lookup && !error) {
            lookupDeviceAuth(code)
                .then((r) => (lookup = r))
                .catch((err) => {
                    error = err instanceof Error ? err.message : 'Lookup failed';
                });
        }
    });

    async function onApprove() {
        if (!code) return;
        error = '';
        processing = true;
        try {
            await approveDeviceAuth(code);
            success = true;
        } catch (err) {
            error = err instanceof Error ? err.message : 'Failed to authorize device';
        } finally {
            processing = false;
        }
    }

    async function onDeny() {
        if (!code) return;
        error = '';
        processing = true;
        try {
            await denyDeviceAuth(code);
            denied = true;
        } catch (err) {
            error = err instanceof Error ? err.message : 'Failed to deny device';
        } finally {
            processing = false;
        }
    }
</script>

<PublicEntryShell
    eyebrow="Device approval"
    title="Authorize Device"
    subtitle="Confirm the code shown in your CLI to finish signing in."
>
    {#if getLoading()}
        <Callout tone="info" message="Loading your session..." />
    {:else if success}
        <Callout
            tone="success"
            title="Device approved"
            message="CLI session approved. You can close this tab."
        />
    {:else if denied}
        <Callout
            tone="warning"
            title="Device denied"
            message="CLI authorization denied. You can close this tab."
        />
    {:else if invalidCode}
        <Callout
            tone="danger"
            title="Invalid code"
            message="Invalid device code format. Please use the link shown in your CLI."
        />
    {:else if !code}
        <Callout
            tone="warning"
            title="Missing code"
            message="No device code provided. Please use the link shown in your CLI."
        />
    {:else if !isLoggedIn}
        <Callout tone="info" message="You need to log in before you can authorize this device." />
        <Button
            variant="primary"
            href="/login?redirect=/device?user_code={encodeURIComponent(code)}"
            class="w-full justify-center"
        >
            Log in
        </Button>
    {:else}
        {#if error}
            <Callout tone="danger" title="Unable to process device request" message={error} />
        {/if}

        {#if lookup?.client_name}
            <Callout
                tone="info"
                title="Approve sign-in"
                message="Approve sign-in from {lookup.client_name}? Confirm the code below matches what is shown in your terminal."
            />
        {:else}
            <Callout
                tone="info"
                message="Your CLI is requesting access. Confirm the code below matches what is shown in your terminal."
            />
        {/if}

        <div
            class="rounded-panel border border-[var(--border-subtle)] bg-[var(--bg-raised)] px-4 py-5 text-center"
            data-ui="device-code"
        >
            <span
                class="font-mono text-device-code font-semibold tracking-device-code text-[var(--text-primary)]"
                >{code}</span
            >
        </div>

        <div class="flex gap-3">
            <Button
                variant="primary"
                type="button"
                class="flex-1 justify-center"
                disabled={processing}
                loading={processing}
                onclick={onApprove}
            >
                Approve
            </Button>
            <Button
                variant="secondary"
                type="button"
                class="flex-1 justify-center"
                disabled={processing}
                onclick={onDeny}
            >
                Deny
            </Button>
        </div>
    {/if}
</PublicEntryShell>
```

- [ ] **Step 3: Confirm + lint**

Run: `cd frontend && npm run lint && npm run format:check && npm run check`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/routes/device/+page.svelte frontend/src/lib/
git commit -m "feat(frontend): rename ?code -> ?user_code, add Deny button + lookup context"
```

---

## Task 8: Frontend Playwright tests

**Files:**

- Modify / Create: existing frontend Playwright spec at `frontend/tests/device.spec.ts` (or the closest existing path).

Locate the existing spec first; the project may already have a `/device` test that needs the param rename.

- [ ] **Step 1: Find the existing test**

Run: `find frontend/tests -name '*.spec.*'`
Read whichever file references `/device`.

- [ ] **Step 2: Update for the new param + add the new cases**

Update the URL param from `?code=` to `?user_code=` in every test in the file.

Add three test cases:

- **Happy approval:** navigate to `/device?user_code=ABCD-EFGH`, mock the lookup endpoint to return `{ client_name: 'cli-laptop-2026-05-12', expires_at: <future> }`, assert "Approve sign-in from cli-laptop-2026-05-12?" is visible, click Approve, mock `POST /api/v1/auth/device/approve` 200, assert the success callout.
- **Deny path:** same setup, click Deny, mock `POST /api/v1/auth/device/deny` 200, assert the "Device denied" callout.
- **Invalid code:** navigate to `/device?user_code=GARBAGE`, assert the "Invalid code" callout.

Use the project's existing Playwright mocking patterns; if `page.route()` is the established pattern, use it.

- [ ] **Step 3: Run Playwright tests**

Run: `cd frontend && npm run test`
Expected: green.

- [ ] **Step 4: Commit**

```bash
git add frontend/tests/
git commit -m "test(frontend): rename ?code -> ?user_code and cover Approve/Deny/invalid paths"
```

---

## Task 9: Documentation deliverables

**Files:**

- Search / modify: `README.md`, `docs/**/*.md`.
- Verify: `docs/development/openapi-client.md`.

Per the spec's "Documentation deliverables" section.

- [ ] **Step 1: Grep for stale references**

Run, from the project root:

```bash
grep -rn 'verification_url\|/api/v1/auth/device\|/device?code=' \
    README.md docs/ \
    --include='*.md' --exclude-dir='target' --exclude-dir='node_modules'
```

Expected at the time the spec was written: zero matches outside the spec itself (`docs/superpowers/specs/2026-05-12-rfc8628-device-auth-design.md`) and ADR 0009. If anything else turns up, update each file:

- `verification_url` → `verification_uri` (or `verification_uri_complete` where pre-filling is the intent).
- `/api/v1/auth/device` → `/api/v1/oauth/device_authorization`.
- `/api/v1/auth/device/poll` → `/api/v1/oauth/token`.
- `/device?code=...` → `/device?user_code=...`.

- [ ] **Step 2: Verify `docs/development/openapi-client.md` is still accurate**

Run: `cat docs/development/openapi-client.md | head -80`
Confirm the "Design decisions" section still reads "Hand-written instead of code-generated" and the list of covered endpoints matches the new method names. If the doc enumerates methods, update the list: drop `device_auth_start`/`device_auth_poll`/`stream_device_auth`; add `oauth_device_authorization`/`oauth_token`/`oauth_authorization_server_metadata`/`device_auth_deny`/`device_auth_lookup`.

- [ ] **Step 3: Confirm CONTEXT.md unchanged**

`CONTEXT.md` should not change — RFC 8628 vocab is OAuth standard. Verify with:

```bash
git diff --name-only origin/main -- CONTEXT.md
```

Expected: no diff. If somehow modified, revert.

- [ ] **Step 4: Lint markdown**

Run: `npx prettier --write 'docs/**/*.md' 'README.md' && markdownlint --config .markdownlint.json '**/*.md'`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add README.md docs/
git commit -m "docs: align device-auth references with RFC 8628 endpoints + params"
```

(If nothing changed, this commit is empty — skip it.)

---

## Task 10: Quality gates

**Files:**

- None (verification only).

- [ ] **Step 1: Rust workspace**

Run, sequentially:

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo deny check
```

Expected: every command exits 0 with no warnings.

- [ ] **Step 2: Markdown**

Run: `markdownlint --config .markdownlint.json '**/*.md'`
Expected: clean.

- [ ] **Step 3: Frontend**

Run:

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

Expected: every step green; build artefact produced.

- [ ] **Step 4: End-to-end smoke test (manual)**

Start the controller locally, run `uptrakit-cli auth login` against it, click through the `/device?user_code=...` page (approve once, deny once on a fresh flow). Confirm:

- The browser opens at `…/device?user_code=...` (the `verification_uri_complete`).
- The page renders "Approve sign-in from cli-…" with the `client_name`.
- Clicking Approve completes the flow and prints the success message in the CLI.
- On a fresh flow, clicking Deny completes the flow with "Authorization denied by Operator." in the CLI.

- [ ] **Step 5: Final state check**

Run: `git log --oneline -20`
Expected: a coherent commit graph across Plan 2 Tasks 1–9.

- [ ] **Step 6: Plan-completion note**

Plans 1 + 2 are both complete. Both PRs must land before the next minor release tag (per the spec's "Hard break, single PR" decision — Plan 1's PR can merge first but Plan 2's must follow within the same release boundary so the CLI and controller versions stay in lockstep).
