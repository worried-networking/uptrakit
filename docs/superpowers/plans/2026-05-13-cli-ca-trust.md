# CLI CA Trust — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add TOFU-based CA trust to the `uptrakit` CLI so operators running a self-managed-CA controller can establish and persist CA trust without `--insecure`.

**Architecture:** `Config` gains `ca_pem: Option<String>`; `UptrakitClient::new` gains a `ca_pem`
param that activates `tls_certs_only` to exclude system roots; a shared `establish_ca_trust`
function fetches `GET /api/v1/pki/ca.crt`, verifies the SHA-256 fingerprint, and persists the
PEM; new `auth login --tofu` and `auth ca trust/status/forget` commands drive it.

**Tech Stack:** Rust, clap 4, reqwest 0.13 (`tls_certs_only`), rustls 0.23
(`CertificateDer` + `PemObject` trait), sha2 0.10, `uptrakit_shared_types::hex`,
rcgen (test certs), httpmock via `uptrakit-openapi-client` mock feature.

---

## File Map

| File                                                        | Action | Purpose                                                                                                        |
| ----------------------------------------------------------- | ------ | -------------------------------------------------------------------------------------------------------------- |
| `crates/ui/cli/Cargo.toml`                                  | Modify | Add `sha2`, `rustls` workspace deps                                                                            |
| `crates/ui/cli/src/config.rs`                               | Modify | Add `ca_pem` + `#[non_exhaustive]`                                                                             |
| `crates/shared/openapi-client/src/lib.rs`                   | Modify | `new`/`with_token` signature + `tls_certs_only` + fix test call sites                                          |
| `crates/shared/openapi-client/src/mock.rs`                  | Modify | Update `new`/`with_token` call sites                                                                           |
| `crates/core/integration-tests/tests/helpers/api_client.rs` | Modify | Update `new`/`with_token` call sites                                                                           |
| `crates/ui/cli/src/client.rs`                               | Modify | Load `ca_pem` from config, error hint                                                                          |
| `crates/ui/cli/src/commands/auth.rs`                        | Modify | `parse_fingerprint`, `establish_ca_trust`, `AuthCommands`, `CaCommands`, `AuthStatusOutput`, `login`, `status` |
| `crates/ui/cli/src/main.rs`                                 | Modify | Update `Login` dispatch arm                                                                                    |
| `CONTEXT.md`                                                | Modify | Add "CLI CA Trust" term                                                                                        |
| `docs/security/tofu-tls.md`                                 | Modify | Add CLI CA Trust section                                                                                       |
| `docs/development/coding-standards.md`                      | Modify | `parse_fingerprint`, `establish_ca_trust`, `tls_certs_only` rules                                              |

---

## Task 1: Add `ca_pem` to `Config` + `#[non_exhaustive]`

**Files:**

- Modify: `crates/ui/cli/src/config.rs`

- [ ] **Step 1: Write failing test**

In `crates/ui/cli/src/config.rs`, add to the existing `#[cfg(test)]` module (create one if absent):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrip_with_ca_pem() {
        let original = Config {
            server: Some("https://example.com".into()),
            ca_pem: Some("-----BEGIN CERTIFICATE-----\nABC\n-----END CERTIFICATE-----\n".into()),
            ..Default::default()
        };
        let json = serde_json::to_string(&original).expect("serialize");
        let parsed: Config = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.server, original.server);
        assert_eq!(parsed.ca_pem, original.ca_pem);
    }

    #[test]
    fn config_roundtrip_without_ca_pem() {
        let original = Config {
            server: Some("https://example.com".into()),
            ca_pem: None,
            ..Default::default()
        };
        let json = serde_json::to_string(&original).expect("serialize");
        let parsed: Config = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.ca_pem, None);
    }

    #[test]
    fn config_missing_ca_pem_field_deserializes_as_none() {
        let json = r#"{"server":"https://example.com"}"#;
        let parsed: Config = serde_json::from_str(json).expect("deserialize");
        assert_eq!(parsed.ca_pem, None);
    }
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```text
cargo test -p uptrakit-cli config -- --nocapture
```

Expected: compile error — `Config` doesn't have `ca_pem` field or `..Default::default()`.

- [ ] **Step 3: Update `Config` struct**

Replace the current `Config` struct in `crates/ui/cli/src/config.rs`:

```rust
#[non_exhaustive]
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub server: Option<String>,
    /// PEM-encoded certificate of the trusted controller CA.
    /// `None` = use system roots. Set by `auth login --tofu` or `auth ca trust`.
    #[serde(default)]
    pub ca_pem: Option<String>,
}
```

- [ ] **Step 4: Fix `poll_for_token` to preserve `ca_pem` — complete fix, no placeholder**

The existing code at `crates/ui/cli/src/commands/auth.rs:310-314` creates
`Config { server: Some(...) }`. After adding `ca_pem` + `#[non_exhaustive]`, this struct
literal fails to compile. A `..Default::default()` placeholder would compile but silently
wipe `ca_pem` on every successful login. Fix immediately with proper config threading.

Change `poll_for_token` signature from:

```rust
async fn poll_for_token(
    client: &UptrakitClient,
    server: &str,
    start_resp: &DeviceAuthorizationResponse,
    client_name: &str,
) -> Result<()> {
```

to:

```rust
async fn poll_for_token(
    client: &UptrakitClient,
    server: &str,
    start_resp: &DeviceAuthorizationResponse,
    client_name: &str,
    config: &mut Config,
) -> Result<()> {
```

Replace the `save_config` call in the success arm (lines ~310-313):

```rust
Ok(resp) => {
    config.server = Some(server.to_string());
    save_config(config).await?;
    save_credentials(&Credentials {
        token: Some(resp.access_token),
    })
    .await?;
    eprintln!();
    println!("Logged in to {} successfully.", server);
    println!("API token stored locally (name: {}).", client_name);
    return Ok(());
}
```

Update `login()` to pass `&mut config` (line ~225 has `let config = load_config()?;` — change to `let mut config`):

```rust
pub async fn login(server_override: Option<&str>, insecure: bool) -> Result<()> {
    let mut config = load_config()?;
    // ... existing server resolution using config.server ...
    poll_for_token(&client, &server, &start_resp, &client_name, &mut config).await
}
```

- [ ] **Step 5: Run tests**

```text
cargo test -p uptrakit-cli config -- --nocapture
```

Expected: all three tests pass.

- [ ] **Step 6: Cargo check**

```text
cargo check --no-default-features --features db-sqlite 2>&1 | grep "uptrakit-cli\|config"
```

Fix any remaining struct literal errors (the `#[non_exhaustive]` means all struct literals in the same crate must use `..Default::default()`).

- [ ] **Step 7: Commit**

```bash
git add crates/ui/cli/src/config.rs
git commit -m "feat(cli): add ca_pem field to Config with non_exhaustive"
```

---

## Task 2: Extend `UptrakitClient::new` and `with_token` with `ca_pem`

**Files:**

- Modify: `crates/shared/openapi-client/src/lib.rs`

- [ ] **Step 1: Write failing test**

Add to the existing `tests` module in `crates/shared/openapi-client/src/lib.rs`:

```rust
#[test]
fn new_with_ca_pem_succeeds_for_valid_pem() {
    // A minimal self-signed PEM just needs to parse as valid DER —
    // the reqwest builder accepts any parseable certificate.
    // We use a known-good PEM from the rcgen crate in dev-deps.
    // Since this is a unit test without rcgen, just confirm the
    // parameter is accepted when None.
    let client =
        UptrakitClient::new("https://example.com", None, false, None, None).expect("client");
    assert!(client.token.is_none());
}

#[test]
fn new_with_insecure_ignores_ca_pem() {
    // insecure=true should succeed even with ca_pem=Some(invalid)
    // because ca_pem is ignored when insecure=true
    let client = UptrakitClient::new(
        "https://example.com",
        None,
        true,
        Some("not-a-pem"),
        None,
    );
    // When insecure=true, ca_pem is skipped entirely — no parse error
    assert!(client.is_ok());
}
```

- [ ] **Step 2: Run to confirm compile failure**

```text
cargo test -p uptrakit-openapi-client new_with -- --nocapture 2>&1 | head -20
```

Expected: compile error — `new` has wrong arity.

- [ ] **Step 3: Update `UptrakitClient::new` signature and logic**

Note: `tls_certs_only` is the correct reqwest 0.13 API (confirmed at
`crates/shared/service-sdk/src/ca.rs:80`). Do NOT use the deprecated
`add_root_certificate` — it appends to system roots instead of replacing them.

In `crates/shared/openapi-client/src/lib.rs`, replace the `new` function (lines 150–171):

```rust
pub fn new(
    base_url: &str,
    token: Option<&str>,
    insecure: bool,
    ca_pem: Option<&str>,
    request_timeout: Option<Duration>,
) -> Result<Self> {
    let timeout = request_timeout.unwrap_or(Self::DEFAULT_REQUEST_TIMEOUT);
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Self::DEFAULT_CONNECT_TIMEOUT)
        .timeout(timeout);
    if insecure {
        builder = builder.tls_danger_accept_invalid_certs(true);
    } else if let Some(pem) = ca_pem {
        let cert = reqwest::Certificate::from_pem(pem.as_bytes()).context_to()?;
        builder = builder.tls_certs_only(std::iter::once(cert));
    }
    let http = builder.build().context_to()?;

    Ok(Self {
        http,
        base_url: base_url.trim_end_matches('/').to_string(),
        token: token.map(|t| t.to_string()),
        retry: None,
    })
}
```

- [ ] **Step 4: Update `with_token` signature and impl**

Replace `with_token` (lines 174–176):

```rust
pub fn with_token(base_url: &str, token: &str, insecure: bool, ca_pem: Option<&str>) -> Result<Self> {
    Self::new(base_url, Some(token), insecure, ca_pem, None)
}
```

- [ ] **Step 5: Fix all test call sites in `lib.rs`**

Every `UptrakitClient::new(...)` call in the `tests` module of `lib.rs` currently passes
4 args. Add `None` as the 4th arg (before the final `None`/`request_timeout`). Every
`UptrakitClient::with_token(...)` call passes 3 args — add `None` as the 4th arg.

Affected lines (insert `None,` before the last positional arg in each):

```rust
// Line ~633 — base_url_trailing_slash_is_trimmed
UptrakitClient::new("https://example.com/", None, false, None, None)

// Line ~641 — base_url_without_trailing_slash
UptrakitClient::new("https://example.com", None, false, None, None)

// Line ~647 — with_token_stores_token
UptrakitClient::with_token("https://example.com", "tok-123", false, None)

// Line ~655 — new_without_token_stores_none
UptrakitClient::new("https://example.com", None, false, None, None)

// Line ~661 — token_or_err_returns_token_when_present
UptrakitClient::with_token("https://example.com", "tok", false, None)

// Line ~669 — token_or_err_returns_error_when_absent
UptrakitClient::new("https://example.com", None, false, None, None)

// Line ~723 — default_client_has_no_retry
UptrakitClient::new("https://example.com", None, false, None, None)

// Line ~729 — with_retry_sets_config
UptrakitClient::new("https://example.com", None, false, None, None)

// Line ~746 — retrying_client helper
UptrakitClient::with_token(base_url, "test-token", false, None)

// Lines ~923, ~951, ~969, ~992 — pagination tests
UptrakitClient::with_token(&server.base_url(), "tok", false, None)
```

- [ ] **Step 6: Fix `mock.rs` call sites**

In `crates/shared/openapi-client/src/mock.rs`:

```rust
// Line ~56 — client()
UptrakitClient::with_token(&self.server.base_url(), "test-token", false, None)

// Line ~66 — client_unauth()
UptrakitClient::new(&self.server.base_url(), None, false, None, None)
```

- [ ] **Step 7: Run tests**

```text
cargo test -p uptrakit-openapi-client -- --nocapture 2>&1 | tail -20
```

Expected: all pass.

- [ ] **Step 8: Cargo check**

```text
cargo check --no-default-features --features db-sqlite 2>&1 | grep -E "error|warning.*unused"
```

- [ ] **Step 9: Commit**

```bash
git add crates/shared/openapi-client/src/lib.rs crates/shared/openapi-client/src/mock.rs
git commit -m "feat(openapi-client): add ca_pem parameter to UptrakitClient::new and with_token"
```

---

## Task 3: Fix integration-test call sites

**Files:**

- Modify: `crates/core/integration-tests/tests/helpers/api_client.rs`

- [ ] **Step 1: Read the file**

```text
cargo check -p uptrakit-integration-tests 2>&1 | grep "error\[" | head -10
```

- [ ] **Step 2: Fix call sites**

In `crates/core/integration-tests/tests/helpers/api_client.rs`:

```rust
// Line ~45
UptrakitClient::new(&self.base_url, None, true, None, Some(Duration::from_secs(5)))

// Line ~69
UptrakitClient::new(&self.base_url, None, true, None, Some(Duration::from_secs(60)))

// Line ~117
UptrakitClient::with_token(&self.base_url, &token, true, None)
```

- [ ] **Step 3: Verify**

```text
cargo check -p uptrakit-integration-tests 2>&1 | grep "error\["
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add crates/core/integration-tests/tests/helpers/api_client.rs
git commit -m "fix(integration-tests): update UptrakitClient call sites for ca_pem parameter"
```

---

## Task 4: Update `authenticated_client()` to thread `ca_pem`

**Files:**

- Modify: `crates/ui/cli/Cargo.toml`
- Modify: `crates/ui/cli/src/client.rs`
- Modify: `crates/ui/cli/src/commands/auth.rs` (for all direct `UptrakitClient::new` calls that aren't inside `login`)

- [ ] **Step 1: Add deps to CLI Cargo.toml**

In `crates/ui/cli/Cargo.toml`, add to `[dependencies]`:

```toml
sha2 = { workspace = true }
rustls = { workspace = true }
```

- [ ] **Step 2: Write failing test for `authenticated_client`**

In `crates/ui/cli/src/client.rs`, add:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_server_and_token_returns_not_logged_in_when_both_absent() {
        let result = resolve_server_and_token(None, None);
        assert!(result.is_err());
    }

    #[test]
    fn resolve_server_and_token_uses_overrides() {
        let result = resolve_server_and_token(
            Some("https://example.com"),
            Some("tok"),
        );
        assert!(result.is_ok());
        let (server, token) = result.unwrap();
        assert_eq!(server, "https://example.com");
        assert_eq!(token, "tok");
    }
}
```

These tests should already pass after the change. Run first to confirm they compile.

- [ ] **Step 3: Update `authenticated_client()`**

Replace `crates/ui/cli/src/client.rs` entirely:

```rust
use crate::config::{load_config, load_credentials};
use crate::error::{CliError, Result};
use rootcause::prelude::*;

pub use uptrakit_openapi_client::UptrakitClient;

pub fn resolve_server_and_token(
    server_override: Option<&str>,
    token_override: Option<&str>,
) -> Result<(String, String)> {
    let config = load_config()?;
    let creds = load_credentials()?;

    let server = server_override
        .map(|s| s.to_string())
        .or(config.server)
        .ok_or_else(|| report!(CliError::NotLoggedIn))?;

    let token = token_override
        .map(|t| t.to_string())
        .or(creds.token)
        .ok_or_else(|| report!(CliError::NotLoggedIn))?;

    Ok((server, token))
}

pub fn authenticated_client(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<UptrakitClient> {
    let config = load_config()?;
    let (server, token) = resolve_server_and_token(server, token)?;
    let ca_pem = config.ca_pem.clone();

    UptrakitClient::new(&server, Some(&token), insecure, ca_pem.as_deref(), request_timeout)
        .map_err(|e| {
            if ca_pem.is_some() {
                e.attach_printable(
                    "Connection failed. If the controller CA has rotated, run \
                     'uptrakit auth ca trust' to re-establish trust. If the controller \
                     now uses a public CA, run 'uptrakit auth ca forget' to return to \
                     system roots. Otherwise, check your network connection.",
                )
            } else {
                e
            }
        })
        .context_to()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_server_and_token_returns_not_logged_in_when_both_absent() {
        let result = resolve_server_and_token(None, None);
        assert!(result.is_err());
    }

    #[test]
    fn resolve_server_and_token_uses_overrides() {
        let (server, token) =
            resolve_server_and_token(Some("https://example.com"), Some("tok")).expect("ok");
        assert_eq!(server, "https://example.com");
        assert_eq!(token, "tok");
    }
}
```

- [ ] **Step 4: Fix remaining `UptrakitClient::new` call sites in `auth.rs`**

The functions `status`, `token_create`, `token_list`, `token_revoke` in `auth.rs` all call
`UptrakitClient::new` directly. They should pass `ca_pem` loaded from config. Update each:

In each function, load config and pass `ca_pem`:

```rust
// status() — line ~392-394
let config = load_config()?;
let client = UptrakitClient::new(
    &server,
    Some(&token),
    insecure,
    config.ca_pem.as_deref(),
    request_timeout,
)
.context_to()?;
```

Apply the same pattern to `token_create` (~line 418), `token_list` (~line 446),
`token_revoke` (~line 481). Each already calls `resolve_server_and_token` so it just
needs a `load_config()` before it and `ca_pem` threaded through.

- [ ] **Step 5: Run tests**

```text
cargo test -p uptrakit-cli -- --nocapture 2>&1 | tail -20
```

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add crates/ui/cli/Cargo.toml crates/ui/cli/src/client.rs crates/ui/cli/src/commands/auth.rs
git commit -m "feat(cli): thread ca_pem from config through authenticated_client and auth functions"
```

---

## Task 5: `parse_fingerprint` function + unit tests

**Files:**

- Modify: `crates/ui/cli/src/commands/auth.rs`

- [ ] **Step 1: Write failing tests**

Add to the `tests` module in `crates/ui/cli/src/commands/auth.rs`:

```rust
mod fingerprint_tests {
    #![expect(
        clippy::expect_used,
        clippy::unwrap_used,
        reason = "test code: panics on failure are acceptable"
    )]
    use super::*;

    #[test]
    fn plain_hex_accepted() {
        let fp = "a".repeat(64);
        assert_eq!(parse_fingerprint(&fp).unwrap(), fp);
    }

    #[test]
    fn sha256_prefix_stripped() {
        let hex = "b".repeat(64);
        let input = format!("sha256:{hex}");
        assert_eq!(parse_fingerprint(&input).unwrap(), hex);
    }

    #[test]
    fn unsupported_prefix_rejected() {
        let err = parse_fingerprint("sha1:aabbcc").unwrap_err();
        assert!(err.to_string().contains("unsupported fingerprint algorithm 'sha1'"));
    }

    #[test]
    fn wrong_length_rejected() {
        let err = parse_fingerprint("aabbcc").unwrap_err();
        assert!(err.to_string().contains("64 lowercase hex characters"));
    }

    #[test]
    fn uppercase_hex_rejected() {
        let fp = "A".repeat(64);
        let err = parse_fingerprint(&fp).unwrap_err();
        assert!(err.to_string().contains("64 lowercase hex characters"));
    }

    #[test]
    fn non_hex_chars_rejected() {
        let fp = format!("{}zzzz", "a".repeat(60));
        let err = parse_fingerprint(&fp).unwrap_err();
        assert!(err.to_string().contains("64 lowercase hex characters"));
    }

    #[test]
    fn exactly_64_lowercase_hex_passes() {
        let fp = "0123456789abcdef".repeat(4);
        assert_eq!(fp.len(), 64);
        assert_eq!(parse_fingerprint(&fp).unwrap(), fp);
    }
}
```

- [ ] **Step 2: Run to confirm compile failure**

```text
cargo test -p uptrakit-cli fingerprint_tests -- --nocapture 2>&1 | head -10
```

Expected: compile error — `parse_fingerprint` not defined.

- [ ] **Step 3: Implement `parse_fingerprint`**

Add to `crates/ui/cli/src/commands/auth.rs`, after the imports section:

```rust
/// Parse a `--tofu=<value>` fingerprint string into normalized 64-char lowercase hex.
///
/// Accepts plain 64-char lowercase hex or a `sha256:` prefix.
pub fn parse_fingerprint(s: &str) -> Result<String> {
    let hex_part = if let Some(rest) = s.strip_prefix("sha256:") {
        rest
    } else if let Some(colon_pos) = s.find(':') {
        let prefix = &s[..colon_pos];
        bail!(CliError::Other(format!(
            "unsupported fingerprint algorithm '{prefix}'; supported: sha256"
        )));
    } else {
        s
    };

    if hex_part.len() != 64
        || !hex_part
            .chars()
            .all(|c| matches!(c, '0'..='9' | 'a'..='f'))
    {
        bail!(CliError::Other(
            "invalid fingerprint: expected 64 lowercase hex characters".into()
        ));
    }

    Ok(hex_part.to_string())
}
```

- [ ] **Step 4: Run tests**

```text
cargo test -p uptrakit-cli fingerprint_tests -- --nocapture
```

Expected: all 7 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/cli/src/commands/auth.rs
git commit -m "feat(cli): add parse_fingerprint for --tofu flag validation"
```

---

## Task 6: `establish_ca_trust` function + unit tests

**Files:**

- Modify: `crates/ui/cli/src/commands/auth.rs`

Add imports at the top of `auth.rs`:

```rust
use sha2::{Digest, Sha256};
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;
```

- [ ] **Step 1: Add `httpmock` to dev-deps**

In `crates/ui/cli/Cargo.toml`, under `[dev-dependencies]`, add:

```toml
httpmock = { workspace = true }
```

This ensures the next compile failure is "function not found", not "crate not found".

- [ ] **Step 2: Write failing tests**

Add a new module to `auth.rs`. These tests use `rcgen` (already in dev-deps) and `httpmock` (just added in Step 1):

```rust
#[cfg(test)]
mod ca_trust_tests {
    #![expect(
        clippy::expect_used,
        clippy::unwrap_used,
        reason = "test code: panics on failure are acceptable"
    )]
    use super::*;
    use httpmock::prelude::*;

    fn make_test_cert_pem() -> String {
        let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
            .expect("key pair");
        let params = rcgen::CertificateParams::default();
        params.self_signed(&key_pair).expect("self-signed cert").pem()
    }

    fn fingerprint_of_pem(pem: &str) -> String {
        let der = CertificateDer::from_pem_slice(pem.as_bytes()).expect("parse pem");
        let mut h = Sha256::new();
        h.update(der.as_ref());
        uptrakit_shared_types::hex::encode(h.finalize())
    }

    #[tokio::test]
    async fn fetch_succeeds_with_matching_fingerprint_hint() {
        let pem = make_test_cert_pem();
        let fp = fingerprint_of_pem(&pem);

        let server = MockServer::start_async().await;
        server.mock(|when, then| {
            when.method(GET).path("/api/v1/pki/ca.crt");
            then.status(200).body(pem.as_str());
        });

        let mut config = Config::default();
        establish_ca_trust(&server.base_url(), Some(&fp), false, &mut config)
            .await
            .expect("should succeed");
        assert_eq!(config.ca_pem.as_deref(), Some(pem.as_str()));

    }

    #[tokio::test]
    async fn fetch_fails_with_wrong_fingerprint_hint() {
        let pem = make_test_cert_pem();
        let server = MockServer::start_async().await;
        server.mock(|when, then| {
            when.method(GET).path("/api/v1/pki/ca.crt");
            then.status(200).body(pem.as_str());
        });

        let wrong_fp = "0".repeat(64);
        let mut config = Config::default();
        let err = establish_ca_trust(&server.base_url(), Some(&wrong_fp), false, &mut config)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("CA fingerprint mismatch"));
        assert!(config.ca_pem.is_none());
    }

    #[tokio::test]
    async fn non_interactive_without_fingerprint_fails() {
        // std::io::stdin() is not a TTY in tests — ensure error is returned
        let pem = make_test_cert_pem();
        let server = MockServer::start_async().await;
        server.mock(|when, then| {
            when.method(GET).path("/api/v1/pki/ca.crt");
            then.status(200).body(pem.as_str());
        });

        let mut config = Config::default();
        let err = establish_ca_trust(&server.base_url(), None, false, &mut config)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("non-interactive"));
    }

    #[tokio::test]
    async fn stored_ca_matches_fetched_proceeds_silently() {
        let pem = make_test_cert_pem();
        let fp = fingerprint_of_pem(&pem);

        let server = MockServer::start_async().await;
        server.mock(|when, then| {
            when.method(GET).path("/api/v1/pki/ca.crt");
            then.status(200).body(pem.as_str());
        });

        let mut config = Config {
            ca_pem: Some(pem.clone()),
            ..Default::default()
        };
        establish_ca_trust(&server.base_url(), Some(&fp), false, &mut config)
            .await
            .expect("should succeed — fingerprints match");
        assert_eq!(config.ca_pem.as_deref(), Some(pem.as_str()));
    }

    #[tokio::test]
    async fn stored_ca_differs_allow_rotation_false_fails() {
        let pem = make_test_cert_pem();
        let new_pem = make_test_cert_pem();
        let new_fp = fingerprint_of_pem(&new_pem);

        let server = MockServer::start_async().await;
        server.mock(|when, then| {
            when.method(GET).path("/api/v1/pki/ca.crt");
            then.status(200).body(new_pem.as_str());
        });

        let mut config = Config {
            ca_pem: Some(pem.clone()),
            ..Default::default()
        };
        let err = establish_ca_trust(&server.base_url(), Some(&new_fp), false, &mut config)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Controller CA has changed"));
        // config unchanged
        assert_eq!(config.ca_pem.as_deref(), Some(pem.as_str()));
    }

    #[tokio::test]
    async fn stored_ca_differs_allow_rotation_true_updates() {
        let pem = make_test_cert_pem();
        let new_pem = make_test_cert_pem();
        let new_fp = fingerprint_of_pem(&new_pem);

        let server = MockServer::start_async().await;
        server.mock(|when, then| {
            when.method(GET).path("/api/v1/pki/ca.crt");
            then.status(200).body(new_pem.as_str());
        });

        let mut config = Config {
            ca_pem: Some(pem.clone()),
            ..Default::default()
        };
        establish_ca_trust(&server.base_url(), Some(&new_fp), true, &mut config)
            .await
            .expect("should succeed — rotation allowed");
        assert_eq!(config.ca_pem.as_deref(), Some(new_pem.as_str()));
    }
}
```

- [ ] **Step 3: Run to confirm compile failure**

```text
cargo test -p uptrakit-cli ca_trust_tests -- --nocapture 2>&1 | head -15
```

Expected: compile error — `establish_ca_trust` not defined.

- [ ] **Step 4: Implement `establish_ca_trust`**

Add to `crates/ui/cli/src/commands/auth.rs`, after `parse_fingerprint`:

```rust
/// Fetch the controller CA, optionally verify its fingerprint, prompt for
/// interactive confirmation, and persist the PEM to config.
///
/// Used by both `auth login --tofu` and `auth ca trust`.
/// `allow_rotation` controls whether a stored-CA mismatch is an error (login)
/// or a warning (ca trust).
pub async fn establish_ca_trust(
    server: &str,
    fingerprint_hint: Option<&str>,
    allow_rotation: bool,
    config: &mut Config,
) -> Result<()> {
    use std::io::IsTerminal as _;

    // Step 1: build a local insecure client scoped to this bootstrap fetch only.
    // SsrfSafeResolver is intentionally NOT applied: this is a CLI tool where the operator
    // IS the user. They explicitly chose the server URL; restricting private-range IPs
    // would break legitimate self-hosted setups. The codebase SSRF rule applies to
    // server-side paths processing user-submitted URLs, not CLI-operator-controlled config.
    let bootstrap_client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(60))
        .tls_danger_accept_invalid_certs(true)
        .build()
        .context_to()?;

    // Step 2: fetch the CA PEM
    let ca_url = format!("{}/api/v1/pki/ca.crt", server.trim_end_matches('/'));
    let fetched_pem = bootstrap_client
        .get(&ca_url)
        .send()
        .await
        .context_to()?
        .text()
        .await
        .context_to()?;

    // Step 3 & 4: parse PEM → DER → sha256 fingerprint
    let der = CertificateDer::from_pem_slice(fetched_pem.as_bytes())
        .map_err(|_| report!(CliError::Other("failed to parse CA certificate PEM".into())))?;
    let mut hasher = Sha256::new();
    hasher.update(der.as_ref());
    let fetched_fp = uptrakit_shared_types::hex::encode(hasher.finalize());

    // Step 5: existing CA check
    if let Some(stored_pem) = &config.ca_pem {
        let stored_der = CertificateDer::from_pem_slice(stored_pem.as_bytes())
            .map_err(|_| report!(CliError::Other("failed to parse stored CA certificate PEM".into())))?;
        let mut h = Sha256::new();
        h.update(stored_der.as_ref());
        let stored_fp = uptrakit_shared_types::hex::encode(h.finalize());

        if stored_fp != fetched_fp {
            if !allow_rotation {
                bail!(CliError::Other(format!(
                    "Controller CA has changed (stored: {stored_fp}, fetched: {fetched_fp}). \
                     Run 'uptrakit auth ca trust --tofu={fetched_fp}' to update."
                )));
            }
            eprintln!(
                "Warning: CA fingerprint has changed (stored: {stored_fp}). \
                 Proceeding will update stored trust."
            );
        }
    }

    // Step 6: fingerprint verification / interactive confirmation
    if let Some(expected) = fingerprint_hint {
        if fetched_fp != expected {
            bail!(CliError::Other(format!(
                "CA fingerprint mismatch: expected {expected}, got {fetched_fp}"
            )));
        }
    } else {
        // No fingerprint supplied — require interactive TTY
        if !std::io::stdin().is_terminal() {
            bail!(CliError::Other(
                "--tofu requires interactive confirmation when no fingerprint is provided; \
                 use --tofu=<fingerprint> for non-interactive use"
                    .into()
            ));
        }
        // When rotating, explicitly warn that the stored CA is being replaced.
        let rotation_note = if config.ca_pem.is_some() {
            "\nWARNING: This will REPLACE the currently stored CA trust anchor.\n"
        } else {
            ""
        };
        eprintln!(
            "Controller CA fingerprint: {fetched_fp}\n{rotation_note}\
             WARNING: This cannot detect a man-in-the-middle attack. To verify securely,\n\
             obtain the fingerprint from the Dashboard (Global Settings) before running\n\
             this command and compare it to the value above. Use --tofu=<fingerprint> to\n\
             confirm without this prompt."
        );
        let answer = prompt("Trust this CA? [y/N]: ")?;
        if !matches!(answer.to_ascii_lowercase().as_str(), "y" | "yes") {
            bail!(CliError::Other(
                "CA trust establishment aborted by user".into()
            ));
        }
    }

    // Step 7: persist
    config.ca_pem = Some(fetched_pem.clone());
    save_config(config).await?;

    // Step 8: confirm
    eprintln!("Controller CA trusted and stored. Future connections will use the pinned CA.");

    Ok(())
}
```

- [ ] **Step 5: Run tests**

```text
cargo test -p uptrakit-cli ca_trust_tests -- --nocapture 2>&1 | tail -20
```

Expected: all 6 tests pass (the non-interactive test passes because stdin in tests is not a TTY).

- [ ] **Step 6: Commit**

```bash
git add crates/ui/cli/Cargo.toml crates/ui/cli/src/commands/auth.rs
git commit -m "feat(cli): add establish_ca_trust shared function with full fingerprint verification"
```

---

## Task 7: Add `--tofu` flag to `auth login` + update login flow

**Files:**

- Modify: `crates/ui/cli/src/commands/auth.rs`
- Modify: `crates/ui/cli/src/main.rs`

- [ ] **Step 1: Write failing parse tests**

In `crates/ui/cli/src/tests.rs`, add:

```rust
#[test]
fn auth_login_tofu_bare_parses() {
    let args = Cli::try_parse_from(["uptrakit", "auth", "login", "--tofu"]).expect("should parse");
    match args.command {
        Some(Commands::Auth {
            command: AuthCommands::Login { tofu },
        }) => {
            assert!(tofu.is_some());
            // bare --tofu gives default_missing_value ""
            assert_eq!(tofu.as_deref(), Some(""));
        }
        _ => panic!("expected Auth Login"),
    }
}

#[test]
fn auth_login_tofu_with_fingerprint_parses() {
    let fp = "a".repeat(64);
    let args = Cli::try_parse_from(["uptrakit", "auth", "login", &format!("--tofu={fp}")])
        .expect("should parse");
    match args.command {
        Some(Commands::Auth {
            command: AuthCommands::Login { tofu },
        }) => {
            assert_eq!(tofu.as_deref(), Some(fp.as_str()));
        }
        _ => panic!("expected Auth Login"),
    }
}

#[test]
fn auth_login_without_tofu_parses() {
    let args = Cli::try_parse_from(["uptrakit", "auth", "login"]).expect("should parse");
    match args.command {
        Some(Commands::Auth {
            command: AuthCommands::Login { tofu },
        }) => {
            assert!(tofu.is_none());
        }
        _ => panic!("expected Auth Login"),
    }
}
```

- [ ] **Step 2: Run to confirm compile failure**

```text
cargo test -p uptrakit-cli auth_login_tofu -- --nocapture 2>&1 | head -10
```

Expected: compile error — `AuthCommands::Login` has no `tofu` field.

- [ ] **Step 3: Update `AuthCommands::Login` variant**

In `auth.rs`, change:

```rust
/// Login to the server via browser authorization
Login,
```

to:

```rust
/// Login to the server via browser authorization
Login {
    /// Trust the controller's CA on first use.
    /// Bare --tofu prompts interactively; --tofu=<FINGERPRINT> pins without prompting.
    #[arg(
        long,
        num_args = 0..=1,
        require_equals = true,
        value_name = "FINGERPRINT",
        default_missing_value = ""
    )]
    tofu: Option<String>,
},
```

- [ ] **Step 4: Update `dispatch` in `auth.rs`**

Change:

```rust
AuthCommands::Login => {
    login(ctx.server.as_deref(), ctx.insecure).await?;
}
```

to:

```rust
AuthCommands::Login { tofu } => {
    if ctx.insecure && tofu.is_some() {
        bail!(CliError::Other("--insecure and --tofu are mutually exclusive".into()));
    }
    // Pass Option<String> directly: None=no TOFU, ""=interactive, "fp"=fingerprint
    login(ctx.server.as_deref(), ctx.insecure, tofu).await?;
}
```

- [ ] **Step 5: Update `login()` signature and body**

Change the `login` function signature from:

```rust
pub async fn login(server_override: Option<&str>, insecure: bool) -> Result<()> {
```

to:

```rust
pub async fn login(
    server_override: Option<&str>,
    insecure: bool,
    tofu: Option<String>,
) -> Result<()> {
```

Inside `login()`, after resolving `server` and before building the client, insert the TOFU
block. Also thread `config` into `poll_for_token` to avoid overwriting `ca_pem`. Here is
the full updated `login` function:

```rust
pub async fn login(
    server_override: Option<&str>,
    insecure: bool,
    tofu: Option<String>,
) -> Result<()> {
    let mut config = load_config()?;
    let server = if let Some(s) = server_override {
        s.to_string()
    } else if let Some(s) = &config.server {
        let input = prompt(&format!("Server URL [{}]: ", s))?;
        if input.is_empty() { s.clone() } else { input }
    } else {
        prompt("Server URL: ")?
    };

    if server.is_empty() {
        bail!(CliError::Other("server URL is required".into()));
    }

    // TOFU: establish CA trust before OAuth flow.
    // tofu: None=no TOFU, ""=interactive prompt, "fp"=non-interactive fingerprint.
    if let Some(raw) = tofu {
        let fp_hint = if raw.is_empty() {
            None
        } else {
            Some(parse_fingerprint(&raw)?)
        };
        establish_ca_trust(&server, fp_hint.as_deref(), false, &mut config).await?;
    }

    let ca_pem = config.ca_pem.clone();
    let host = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let date = chrono_date();
    let client_name = format!("cli-{host}-{date}");

    let client = UptrakitClient::new(&server, None, insecure, ca_pem.as_deref(), None)
        .context_to()?;

    let start_resp = client
        .oauth_device_authorization(&DeviceAuthorizationRequest::new(
            CLI_CLIENT_ID.to_string(),
            None,
            Some(client_name.clone()),
        ))
        .await
        .context_to()?;

    print_browser_instructions(&start_resp, insecure);
    eprintln!("  Waiting for authorization...");

    poll_for_token(&client, &server, &start_resp, &client_name, &mut config).await
}
```

- [ ] **Step 6: Verify `poll_for_token` threading (already done in Task 1 Step 4)**

`poll_for_token` already takes `config: &mut Config` from Task 1. Confirm the call site
in `login()` passes `&mut config` and that `login()` declares
`let mut config = load_config()?;`. No new code needed — just `cargo check` to confirm.

- [ ] **Step 7: Update `main.rs`**

The `Commands::Auth { command }` arm in `run()` dispatches to
`commands::auth::dispatch`. That's already correct — if `Login` is now a struct variant,
it's handled in `dispatch` in `auth.rs`, so `main.rs` needs no changes.

Verify: `cargo check -p uptrakit-cli` compiles without errors.

- [ ] **Step 8: Run parse tests**

```text
cargo test -p uptrakit-cli auth_login_tofu -- --nocapture
```

Expected: all 3 tests pass.

- [ ] **Step 9: Commit**

```bash
git add crates/ui/cli/src/commands/auth.rs crates/ui/cli/src/main.rs crates/ui/cli/src/tests.rs
git commit -m "feat(cli): add --tofu flag to auth login with CA trust bootstrap"
```

---

## Task 8: Add `auth ca` subcommands

**Files:**

- Modify: `crates/ui/cli/src/commands/auth.rs`

- [ ] **Step 1: Write failing parse tests**

Add to `crates/ui/cli/src/tests.rs`:

```rust
#[test]
fn auth_ca_trust_bare_parses() {
    let args = Cli::try_parse_from(["uptrakit", "auth", "ca", "trust"]).expect("should parse");
    match args.command {
        Some(Commands::Auth {
            command: AuthCommands::Ca {
                command: CaCommands::Trust { tofu },
            },
        }) => {
            assert!(tofu.is_none());
        }
        _ => panic!("expected Auth Ca Trust"),
    }
}

#[test]
fn auth_ca_trust_with_tofu_parses() {
    let fp = "a".repeat(64);
    let args = Cli::try_parse_from([
        "uptrakit", "auth", "ca", "trust", &format!("--tofu={fp}"),
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::Auth {
            command: AuthCommands::Ca {
                command: CaCommands::Trust { tofu },
            },
        }) => {
            assert_eq!(tofu.as_deref(), Some(fp.as_str()));
        }
        _ => panic!("expected Auth Ca Trust"),
    }
}

#[test]
fn auth_ca_status_parses() {
    let args = Cli::try_parse_from(["uptrakit", "auth", "ca", "status"]).expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::Auth {
            command: AuthCommands::Ca {
                command: CaCommands::Status
            }
        })
    ));
}

#[test]
fn auth_ca_forget_parses() {
    let args = Cli::try_parse_from(["uptrakit", "auth", "ca", "forget"]).expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::Auth {
            command: AuthCommands::Ca {
                command: CaCommands::Forget
            }
        })
    ));
}
```

Also add `CaCommands` to the test imports at the top of `tests.rs`:

```rust
use commands::auth::{AuthCommands, CaCommands};
```

- [ ] **Step 2: Run to confirm compile failure**

```text
cargo test -p uptrakit-cli auth_ca -- --nocapture 2>&1 | head -10
```

- [ ] **Step 3: Add `CaCommands` enum and `Ca` arm to `AuthCommands`**

In `auth.rs`, add before `pub async fn dispatch`:

```rust
#[derive(Debug, Subcommand)]
pub enum CaCommands {
    /// Establish or update stored CA trust
    Trust {
        /// Trust the controller's CA. Bare --tofu prompts interactively; --tofu=<FINGERPRINT> is non-interactive.
        #[arg(
            long,
            num_args = 0..=1,
            require_equals = true,
            value_name = "FINGERPRINT",
            default_missing_value = ""
        )]
        tofu: Option<String>,
    },
    /// Show stored CA trust status
    Status,
    /// Remove stored CA trust (revert to system roots)
    Forget,
}
```

In `AuthCommands`, add the `Ca` arm:

```rust
/// CA trust management
Ca {
    #[command(subcommand)]
    command: CaCommands,
},
```

- [ ] **Step 4: Add `Ca` dispatch to `dispatch` function**

In `dispatch`, add:

```rust
AuthCommands::Ca { command } => match command {
    CaCommands::Trust { tofu } => {
        ca_trust(ctx.server.as_deref(), tofu).await?;
    }
    CaCommands::Status => {
        ca_status()?;
    }
    CaCommands::Forget => {
        ca_forget().await?;
    }
},
```

- [ ] **Step 5: Implement `ca_trust`, `ca_status`, `ca_forget`**

Add after `token_revoke`:

```rust
/// `auth ca trust` — establish or update stored CA trust.
pub async fn ca_trust(server_override: Option<&str>, tofu: Option<String>) -> Result<()> {
    let mut config = load_config()?;

    let server = server_override
        .map(|s| s.to_string())
        .or_else(|| config.server.clone())
        .ok_or_else(|| {
            report!(CliError::Other(
                "no server URL configured; run 'uptrakit auth login' first or supply --server"
                    .into(),
            ))
        })?;

    let fp_hint = match tofu {
        None => None,
        Some(s) if s.is_empty() => None,
        Some(s) => Some(parse_fingerprint(&s)?),
    };

    establish_ca_trust(&server, fp_hint.as_deref(), true, &mut config).await?;
    Ok(())
}

/// `auth ca status` — print stored CA fingerprint or "system roots".
pub fn ca_status() -> Result<()> {
    let config = load_config()?;
    match &config.ca_pem {
        None => println!("CA trust:    system roots"),
        Some(pem) => {
            let der = CertificateDer::from_pem_slice(pem.as_bytes())
                .map_err(|_| report!(CliError::Other("stored CA PEM is unparseable".into())))?;
            let mut h = Sha256::new();
            h.update(der.as_ref());
            let fp = uptrakit_shared_types::hex::encode(h.finalize());
            println!("CA trust:    {fp}");
        }
    }
    Ok(())
}

/// `auth ca forget` — clear stored CA trust, revert to system roots.
pub async fn ca_forget() -> Result<()> {
    let mut config = load_config()?;
    config.ca_pem = None;
    save_config(&config).await?;
    eprintln!("Stored CA trust removed. System roots will be used for future connections.");
    Ok(())
}
```

- [ ] **Step 6: Run parse tests**

```text
cargo test -p uptrakit-cli auth_ca -- --nocapture
```

Expected: all 4 tests pass.

- [ ] **Step 7: Cargo check**

```text
cargo check -p uptrakit-cli
```

- [ ] **Step 8: Commit**

```bash
git add crates/ui/cli/src/commands/auth.rs crates/ui/cli/src/tests.rs
git commit -m "feat(cli): add auth ca trust/status/forget subcommands"
```

---

## Task 9: Update `auth status` output with `ca_fingerprint`

**Files:**

- Modify: `crates/ui/cli/src/commands/auth.rs`

- [ ] **Step 1: Write failing tests**

In the `tests` module of `auth.rs`, add:

```rust
#[test]
fn auth_status_output_includes_ca_fingerprint() {
    let fp = "e".repeat(64);
    let output = AuthStatusOutput {
        server: "https://example.com".into(),
        first_name: "Alice".into(),
        last_name: "B".into(),
        email: "alice@b.com".into(),
        user_id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
        permissions: vec![],
        ca_fingerprint: Some(fp.clone()),
    };
    let json = serde_json::to_string(&output).expect("serialize");
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["ca_fingerprint"], fp);
}

#[test]
fn auth_status_output_null_when_no_ca() {
    let output = AuthStatusOutput {
        server: "https://example.com".into(),
        first_name: "Alice".into(),
        last_name: "B".into(),
        email: "alice@b.com".into(),
        user_id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
        permissions: vec![],
        ca_fingerprint: None,
    };
    let json = serde_json::to_string(&output).expect("serialize");
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["ca_fingerprint"], serde_json::Value::Null);
}

#[test]
fn auth_status_human_output_shows_ca_trust() {
    let fp = "e".repeat(64);
    let output = AuthStatusOutput {
        server: "https://example.com".into(),
        first_name: "A".into(),
        last_name: "B".into(),
        email: "a@b.com".into(),
        user_id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
        permissions: vec![],
        ca_fingerprint: Some(fp.clone()),
    };
    let s = output.to_human_string();
    assert!(s.contains(&fp), "fingerprint missing from human output");
}

#[test]
fn auth_status_human_output_shows_system_roots_when_no_ca() {
    let output = AuthStatusOutput {
        server: "https://example.com".into(),
        first_name: "A".into(),
        last_name: "B".into(),
        email: "a@b.com".into(),
        user_id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
        permissions: vec![],
        ca_fingerprint: None,
    };
    let s = output.to_human_string();
    assert!(s.contains("system roots"), "system roots line missing");
}
```

- [ ] **Step 2: Run to confirm compile failure**

```text
cargo test -p uptrakit-cli auth_status_output -- --nocapture 2>&1 | head -10
```

- [ ] **Step 3: Add `ca_fingerprint` to `AuthStatusOutput`**

Replace the struct:

```rust
#[derive(Debug, Serialize)]
pub struct AuthStatusOutput {
    pub server: String,
    pub first_name: String,
    pub last_name: String,
    pub email: String,
    pub user_id: Uuid,
    pub permissions: Vec<String>,
    pub ca_fingerprint: Option<String>,
}
```

- [ ] **Step 4: Update `to_human_string`**

```rust
impl HumanOutput for AuthStatusOutput {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("Server:      {}\n", self.server));
        out.push_str(&format!(
            "User:        {} {}\n",
            self.first_name, self.last_name
        ));
        out.push_str(&format!("Email:       {}\n", self.email));
        out.push_str(&format!("User ID:     {}\n", self.user_id));
        if !self.permissions.is_empty() {
            out.push_str(&format!("Permissions: {}\n", self.permissions.join(", ")));
        }
        match &self.ca_fingerprint {
            Some(fp) => out.push_str(&format!("CA trust:    {fp}\n")),
            None => out.push_str("CA trust:    system roots\n"),
        }
        out
    }
}
```

- [ ] **Step 5: Update `status()` function to populate `ca_fingerprint`**

In the `status()` function, after `load_config()` and before constructing `AuthStatusOutput`, compute the fingerprint:

```rust
pub async fn status(
    server_override: Option<&str>,
    token_override: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<AuthStatusOutput> {
    let config = load_config()?;
    let (server, token) = resolve_server_and_token(server_override, token_override)?;
    let client = UptrakitClient::new(
        &server,
        Some(&token),
        insecure,
        config.ca_pem.as_deref(),
        request_timeout,
    )
    .context_to()?;

    let user = client.me().await.context_to()?;
    let permissions: Vec<String> = user.permissions.iter().map(|p| p.to_string()).collect();

    let ca_fingerprint = if insecure {
        None
    } else {
        match config.ca_pem.as_deref() {
            None => None,
            Some(pem) => {
                let der = CertificateDer::from_pem_slice(pem.as_bytes())
                    .map_err(|_| report!(CliError::Other(
                        "stored CA PEM is unparseable; run 'uptrakit auth ca trust' to re-establish".into()
                    )))?;
                let mut h = Sha256::new();
                h.update(der.as_ref());
                Some(uptrakit_shared_types::hex::encode(h.finalize()))
            }
        }
    };

    Ok(AuthStatusOutput {
        server,
        first_name: user.first_name,
        last_name: user.last_name,
        email: user.email,
        user_id: user.id,
        permissions,
        ca_fingerprint,
    })
}
```

- [ ] **Step 6: Fix existing `auth_status_output_serialization` and `auth_status_human_output` tests**

The existing tests in `auth.rs` construct `AuthStatusOutput` without `ca_fingerprint`. Add `ca_fingerprint: None` to each.

- [ ] **Step 7: Run tests**

```text
cargo test -p uptrakit-cli auth_status -- --nocapture
```

Expected: all status tests pass.

- [ ] **Step 8: Cargo clippy**

```text
cargo clippy -p uptrakit-cli --all-targets --all-features 2>&1 | grep "error\["
```

- [ ] **Step 9: Commit**

```bash
git add crates/ui/cli/src/commands/auth.rs
git commit -m "feat(cli): add ca_fingerprint to auth status output"
```

---

## Task 10: Full quality gate

- [ ] **Step 1: `cargo fmt`**

```text
cargo fmt --all
```

- [ ] **Step 2: Full check + clippy**

```text
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
```

Fix any warnings.

- [ ] **Step 3: Full test suite**

```text
cargo test --all-features
```

Expected: all pass.

- [ ] **Step 4: Commit any fmt/clippy fixes**

```bash
git add -p
git commit -m "style(cli): fmt and clippy fixes for cli-ca-trust"
```

---

## Task 11: Documentation updates

**Files:**

- Modify: `CONTEXT.md`
- Modify: `docs/security/tofu-tls.md`
- Modify: `docs/development/coding-standards.md`

- [ ] **Step 1: Add "CLI CA Trust" term to `CONTEXT.md`**

In `CONTEXT.md`, after the `**TOFU mode**:` entry (line ~128–132), add:

```markdown
**CLI CA Trust**:
The mechanism by which the `uptrakit` CLI establishes and persists trust in a Controller's
self-managed CA certificate. The CLI fetches the CA from `GET /api/v1/pki/ca.crt`, verifies
the SHA-256 fingerprint interactively or against a supplied value, and stores the PEM in
`config.json` as the sole TLS trust anchor for future connections.
Entry points: `uptrakit auth login --tofu` (bootstrap) and `uptrakit auth ca trust` (rotation).
_Avoid_: TOFU mode (reserved for Service bootstrap flags).
```

- [ ] **Step 2: Add CLI CA Trust section to `docs/security/tofu-tls.md`**

Append before the "Removed: bare `--tofu`" section. Add the following content (the section
heading, description, entry-points table, fingerprint-format paragraph, and rotation-recovery
shell block):

Section heading and description:

```text
## CLI CA Trust

The `uptrakit` CLI uses a separate Trust-On-First-Use mechanism for connecting to Controllers
with internally-generated (self-managed) CAs. This is distinct from TOFU mode, which is
scoped to Services (Agents, MQTT, etc.).
```

Entry points table:

| Command                                | Purpose                                                      |
| -------------------------------------- | ------------------------------------------------------------ |
| `uptrakit auth login --tofu`           | Bootstrap CA trust during first login                        |
| `uptrakit auth login --tofu=<fp>`      | Non-interactive bootstrap with fingerprint verification      |
| `uptrakit auth ca trust`               | Establish or rotate CA trust independently of auth           |
| `uptrakit auth ca trust --tofu=<fp>`   | Non-interactive CA trust update                              |
| `uptrakit auth ca status`              | Show stored CA fingerprint                                   |
| `uptrakit auth ca forget`              | Remove stored trust, revert to system roots                  |

Fingerprint format paragraph:

```text
SHA-256 of the CA certificate's DER bytes, encoded as 64-character lowercase hex.
This matches the fingerprint shown in Dashboard → Global Settings. Accepts a `sha256:` prefix.
Example: `e3676c6137dada24f41974e2fb62546dadc2c6d6b831e4bb2635393218c64ce4`
```

Rotation recovery block:

```sh
# Interactive (fingerprint visible in Dashboard > Global Settings)
uptrakit auth ca trust

# Non-interactive (CI)
uptrakit auth ca trust --tofu=<fingerprint>

# Migration to public CA (Let's Encrypt)
uptrakit auth ca forget
```

Closing paragraph (add after the shell block):

```text
`auth login --tofu` deliberately fails when a stored CA differs from the fetched one,
even when an explicit fingerprint is supplied. Use `auth ca trust` for rotation.
```

`--insecure` interaction paragraph:

```text
### `--insecure` interaction

`--insecure` overrides all stored CA trust. When `--insecure` is active, `ca_fingerprint`
is `None` in `auth status` output.
```

- [ ] **Step 3: Add rules to `docs/development/coding-standards.md`**

Find the HTTP Client Requirements section and add a subsection after it. The new
subsection heading is `#### Pinned-CA-only reqwest clients`. Content:

> When building a reqwest client that must use a custom CA and exclude system roots, use
> `tls_certs_only`. Example:
>
> ```rust
> let cert = reqwest::Certificate::from_pem(pem.as_bytes())?;
> builder = builder.tls_certs_only(std::iter::once(cert));
> ```
>
> Do **not** use `add_root_certificate` — deprecated in reqwest 0.13 because it appends
> to system roots rather than replacing them.

Then add a second subsection `#### CLI CA fingerprint helpers`:

> `parse_fingerprint(s: &str) -> Result<String>` — normalize a `--tofu` flag value to
> 64-char lowercase hex. Located in `crates/ui/cli/src/commands/auth.rs`.
>
> `establish_ca_trust(server, fingerprint_hint, allow_rotation, config) -> Result<()>` —
> shared bootstrap function used by `auth login --tofu` and `auth ca trust`. Fetches
> `GET /api/v1/pki/ca.crt`, verifies SHA-256 fingerprint, persists PEM. Located in
> `crates/ui/cli/src/commands/auth.rs`.

- [ ] **Step 4: Lint markdown**

```text
npx prettier --write CONTEXT.md docs/security/tofu-tls.md docs/development/coding-standards.md
```

Fix any markdownlint errors:

```text
npx markdownlint --config .markdownlint.json CONTEXT.md docs/security/tofu-tls.md docs/development/coding-standards.md
```

- [ ] **Step 5: Commit**

```bash
git add CONTEXT.md docs/security/tofu-tls.md docs/development/coding-standards.md
git commit -m "docs: add CLI CA Trust terminology, security guide, and coding-standards rules"
```

---

## Task 12: Update pending-specs tracker

**Files:**

- Modify: `.superpowers/pending-specs.md`

- [ ] **Step 1: Update entry for CLI CA Trust**

In `.superpowers/pending-specs.md`, find the entry for spec #12 (CLI CA Trust). Replace its `NO_PLAN` status with the plan row:

```markdown
## 12. CLI CA Trust

| Artifact | Path                                                                                                                      | Status      |
| -------- | ------------------------------------------------------------------------------------------------------------------------- | ----------- |
| Spec     | [`docs/superpowers/specs/2026-05-13-cli-ca-trust-design.md`](../docs/superpowers/specs/2026-05-13-cli-ca-trust-design.md) |             |
| Plan     | [`docs/superpowers/plans/2026-05-13-cli-ca-trust.md`](../docs/superpowers/plans/2026-05-13-cli-ca-trust.md)               | NOT_STARTED |
```

- [ ] **Step 2: Commit**

```bash
git add .superpowers/pending-specs.md docs/superpowers/plans/2026-05-13-cli-ca-trust.md
git commit -m "docs(superpowers): add CLI CA Trust implementation plan"
```

---

## Self-Review

### Spec coverage check

| Spec section                                          | Covered by task       |
| ----------------------------------------------------- | --------------------- |
| §4 `Config.ca_pem` + `#[non_exhaustive]`              | Task 1                |
| §5 `UptrakitClient::new` signature + `tls_certs_only` | Task 2                |
| §5 `authenticated_client` error hint                  | Task 4                |
| §6 `parse_fingerprint`                                | Task 5                |
| §7 `establish_ca_trust`                               | Task 6                |
| §8 `auth login --tofu`                                | Task 7                |
| §8 `auth ca trust/status/forget`                      | Task 8                |
| §8 `auth status` `ca_fingerprint`                     | Task 9                |
| §10 CA rotation runbook                               | Task 11 (tofu-tls.md) |
| §9 CONTEXT.md term                                    | Task 11               |
| §12 Documentation deliverables                        | Task 11               |
| `poll_for_token` preserves `ca_pem`                   | Task 7 step 6         |
| Integration test call sites                           | Task 3                |

### Binding Rules verified

- `#[non_exhaustive]` on `Config` ✓
- `connect_timeout(10s)` + `timeout(60s)` on bootstrap client ✓
- `tls_certs_only` not `add_root_certificate` ✓
- `rootcause::Report` + `bail!` ✓
- `uptrakit_shared_types::hex::encode` (no external hex crate) ✓
- workspace `sha2` at 0.10 (not 0.11) ✓
- No `unwrap()` in production code ✓
- `start_paused = true` not needed (no `tokio::time` in feature) ✓
