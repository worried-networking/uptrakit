# CLI CA Trust — Design

Status: Draft
Author: Andrey Yantsen
Date: 2026-05-13

## 1. Goal

Give the `uptrakit` CLI a Trust-On-First-Use mechanism for self-managed controller CAs.
Operators running a controller with an internally-generated (self-signed) CA can
establish and persist trust in that CA without using `--insecure` on every command.

After this work:

- `uptrakit auth login --tofu` bootstraps trust interactively during login.
- `uptrakit auth ca trust` establishes or updates trust independently of authentication.
- Stored CA PEM is used as the sole TLS trust anchor for all subsequent CLI connections.
- CA rotation is detected and surfaced with a clear recovery path.
- `--insecure` is preserved for development use; it overrides stored CA trust entirely.

## 2. Background

The CLI currently passes `insecure: bool` to `UptrakitClient::new`, which maps to
`reqwest::ClientBuilder::tls_danger_accept_invalid_certs(true)`. This disables all TLS
verification and is the only supported path for controllers with self-managed CAs.

The Controller exposes an unauthenticated `GET /api/v1/pki/ca.crt` endpoint that returns
the active CA certificate in PEM format. This endpoint is the bootstrap path used by
Services during Enrollment; the CLI can use it for the same purpose.

The existing TOFU modes (`system`, `pin-fingerprint`, `pin-spki`, `insecure-tofu`) are
defined in `docs/security/tofu-tls.md` and scoped exclusively to Services (Agent,
Agent-SSH, MQTT, Scheduler). The CLI mechanism is analogous but distinct — see
§9 (Terminology).

## 3. Scope

### In scope

- `Config` struct gains `ca_pem: Option<String>` field.
- `UptrakitClient::new` gains `ca_pem: Option<&str>` parameter; when `Some`, reqwest uses
  the provided PEM as the sole custom root (system roots excluded).
- `--tofu[=<FINGERPRINT>]` flag on `auth login`.
- New `auth ca` subcommand tree: `trust`, `status`, `forget`.
- `auth status` output gains `ca_fingerprint: Option<String>`.
- `parse_fingerprint` function accepting plain hex or `sha256:` prefix.
- `establish_ca_trust` shared function used by both entry points.
- Documentation: `CONTEXT.md`, `docs/security/tofu-tls.md`.

### Out of scope

- `auth logout` server-side token revocation (deferred to a follow-up spec).
- SPKI pinning for the CLI (deferred; would be `--tofu-spki` if needed).
- Non-managed (external-CA) controller detection — the CLI does not attempt to auto-detect
  whether TOFU is necessary; it is always opt-in.
- Root/intermediate CA split (deferred per ADR-0013).

## 4. Data model

### `Config`

```rust
#[non_exhaustive]
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Config {
    pub server: Option<String>,
    /// PEM-encoded certificate of the trusted controller CA.
    /// `None` = use system roots. Set by `auth login --tofu` or `auth ca trust`.
    pub ca_pem: Option<String>,
}
```

`ca_pem` is stored in `config.json` alongside `server`. The file is already written with
`write_secure_file_str` (mode 0600), so the CA PEM receives the same protection as the
stored API token. No separate file is needed.

`Config` carries `#[non_exhaustive]` per the workspace rule for extensible public structs.
Any destructuring of `Config` in the codebase must include `..`; constructors must use
`Config { server: .., ca_pem: .., ..Default::default() }` or the `Default` impl.

### Fingerprint format

SHA-256 of the CA certificate's DER bytes, encoded as 64-character lowercase hex without
separators. Example: `e3676c6137dada24f41974e2fb62546dadc2c6d6b831e4bb2635393218c64ce4`.

This matches the format produced by `controller-runtime/src/pki.rs::ca_fingerprint` and
displayed in the Dashboard's Global Settings page. Operators can copy-paste directly.

## 5. `UptrakitClient` changes

```rust
pub fn new(
    base_url: &str,
    token: Option<&str>,
    insecure: bool,
    ca_pem: Option<&str>,
    request_timeout: Option<Duration>,
) -> Result<Self>
```

Priority order (highest first):

1. `insecure = true` → `tls_danger_accept_invalid_certs(true)`; `ca_pem` ignored.
2. `ca_pem = Some(pem)` → `tls_certs_only(std::iter::once(reqwest::Certificate::from_pem(pem.as_bytes())?))`;
   system roots are NOT added. Note: `add_root_certificate` (deprecated in reqwest 0.13) must not
   be used here — it appends to system roots rather than replacing them.
3. Neither → system roots only (existing behaviour).

`with_token` convenience constructor gains `ca_pem: Option<&str>` and passes it to `new`.
All existing `UptrakitClient::new` and `with_token` call sites must be updated to pass `ca_pem`;
the implementation plan must enumerate every call site in `crates/shared/openapi-client/` and
`crates/ui/cli/`.

`authenticated_client()` in `client.rs` loads `ca_pem` from `Config` and passes it through.
When a connection error occurs and `ca_pem` is stored, the error is wrapped with the hint:
`"Connection failed. If the controller CA has rotated, run 'uptrakit auth ca trust' to
re-establish trust. If the controller now uses a public CA, run 'uptrakit auth ca forget'
to return to system roots. Otherwise, check your network connection."`
This hint is best-effort — reqwest does not expose a structured TLS-verification-failed
variant, so the hint appears on any connection-layer error (including network unreachable)
when a stored CA is present.

## 6. Fingerprint parsing

```rust
/// Parse a fingerprint supplied via `--tofu=<value>`.
///
/// Accepts plain 64-char lowercase hex or a `sha256:` prefix.
/// Returns the normalized 64-char plain hex string.
pub fn parse_fingerprint(s: &str) -> Result<String, CliError>
```

Rules:

- Strip `sha256:` prefix if present. Any other prefix (e.g. `sha1:`, `spki:`, `md5:`) is an
  error: `"unsupported fingerprint algorithm '<prefix>'; supported: sha256"`.
- After prefix handling, validate: exactly 64 characters, all lowercase hex (`[0-9a-f]`).
  On failure: `"invalid fingerprint: expected 64 lowercase hex characters"`.
- Return the normalized 64-char plain hex string.

## 7. `establish_ca_trust` shared function

The function takes an explicit `config: &mut Config` parameter so callers can pass an
already-loaded config and tests can inject it without filesystem I/O.

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
) -> Result<String>
```

**Steps:**

1. Build a local insecure `reqwest::Client` with `connect_timeout(10s)` and `timeout(60s)`
   (required by workspace HTTP client policy). No token needed. This client is scoped to
   the bootstrap fetch only and is never stored, returned, or reused after step 2.
2. `GET /api/v1/pki/ca.crt` → fetch CA PEM string.
3. Parse PEM → DER: import `rustls::pki_types::CertificateDer` and the
   `rustls::pki_types::pem::PemObject` trait, then call
   `CertificateDer::from_pem_slice(pem.as_bytes())` (the trait must be in scope for
   the method to resolve). Add `rustls = { workspace = true }` to `uptrakit-cli`
   `[dependencies]`. Only the first PEM block is parsed; this matches the server's
   `ca_fingerprint` implementation and the Dashboard display.
4. Compute `fetched_fp = sha256_hex(der)` using workspace `sha2` + `hex` crates (add both
   to `uptrakit-cli` `[dependencies]` if not already present).
5. **Existing CA check:** if `config.ca_pem` is `Some`, compute its fingerprint `stored_fp`.
   If `stored_fp != fetched_fp`:
   - `allow_rotation = false` (login path): fail with:
     `"Controller CA has changed (stored: {stored_fp}, fetched: {fetched_fp}). Run 'uptrakit auth ca trust --tofu=<fetched_fp>' to update."`.
     Note: `auth login --tofu=<new_fp>` also hits this branch and fails even when the
     supplied fingerprint matches the fetched CA. This is deliberate — `auth login` is
     not a rotation recovery tool. Rotation always requires `auth ca trust`. This
     two-step requirement is documented in the CA rotation runbook (§10).
   - `allow_rotation = true` (ca trust path): print warning to stderr:
     `"Warning: CA fingerprint has changed (stored: {stored_fp}). Proceeding will update stored trust."`
6. **Fingerprint verification:**
   - `fingerprint_hint = Some(expected)`: compare `fetched_fp == expected`; on mismatch fail:
     `"CA fingerprint mismatch: expected {expected}, got {fetched_fp}"`.
   - `fingerprint_hint = None`: check `std::io::stdin().is_terminal()` (stable since Rust 1.70
     via `std::io::IsTerminal`; no external crate needed); if no TTY fail:
     `"--tofu requires interactive confirmation when no fingerprint is provided; use --tofu=<fingerprint> for non-interactive use"`.
     If TTY: print the following to stderr, then prompt `"Trust this CA? [y/N]: "`:

     ```text
     Controller CA fingerprint: {fetched_fp}

     WARNING: This cannot detect a man-in-the-middle attack. To verify securely,
     obtain the fingerprint from the Dashboard (Global Settings) before running
     this command and compare it to the value above. Use --tofu=<fingerprint> to
     confirm without this prompt.
     ```

     Accept `y` or `yes` (case-insensitive); anything else aborts without saving.

7. Persist: `config.ca_pem = Some(fetched_pem.clone()); save_config(config).await?`.
8. Print to stderr: `"Controller CA trusted and stored. Future connections will use the pinned CA."`.
9. Return `fetched_pem`.

## 8. CLI commands

### `auth login --tofu`

```text
--tofu[=<FINGERPRINT>]
```

Clap configuration: `num_args(0..=1)`, `require_equals(true)`, `value_name = "FINGERPRINT"`,
conflicts with `--insecure`.

Inserted into the existing `auth login` flow **before** the OAuth device authorization
request. Sequence:

1. Resolve server URL (existing prompt logic).
2. If `--tofu` present: call `establish_ca_trust(server, fingerprint_hint, allow_rotation=false)`.
3. Build client using now-stored `ca_pem` (or stored from a prior run).
4. Continue with OAuth device flow (unchanged).

### `auth ca` subcommand tree

New `AuthCommands::Ca { command: CaCommands }` dispatch arm.

```text
uptrakit auth ca trust [--tofu[=<FINGERPRINT>]]
uptrakit auth ca status
uptrakit auth ca forget
```

**`auth ca trust`**

- Requires `server` in config or `--server` flag; fails with:
  `"no server URL configured; run 'uptrakit auth login' first or supply --server"`.
- `--tofu[=<FINGERPRINT>]` has same semantics as on `auth login`. Without the flag,
  interactive prompt is always shown (TTY required).
- Calls `establish_ca_trust(server, fingerprint_hint, allow_rotation=true)`.
- Does not perform or require authentication — updates only `ca_pem` in config.

**`auth ca status`**

Prints the stored CA fingerprint (plain hex) or `"No CA trust configured (using system roots)"`.
Also reachable via `auth status` (see §8.3).

**`auth ca forget`**

Clears `ca_pem` from config (sets to `None`) and saves. Prints:
`"Stored CA trust removed. System roots will be used for future connections."`.
No server call needed.

### `auth status` changes

`AuthStatusOutput` gains:

```rust
pub ca_fingerprint: Option<String>,
```

`None` when no CA is stored or `--insecure` is active. Populated with the 64-char plain hex
fingerprint of the stored CA when present. Displayed in human output as:

```text
CA trust:    e3676c6137dada24f41974e2fb62546dadc2c6d6b831e4bb2635393218c64ce4
```

or

```text
CA trust:    system roots
```

### `--insecure` + `--tofu` mutual exclusion

Both flags present on the same invocation → clap `conflicts_with` error:
`"--insecure and --tofu are mutually exclusive"`.

## 9. Terminology

`CONTEXT.md` gains a new term:

**CLI CA Trust:**
The mechanism by which the `uptrakit` CLI Operator tool establishes and persists trust
in a Controller's self-managed CA certificate on first connection. The CLI fetches the CA
from `GET /api/v1/pki/ca.crt`, verifies the SHA-256 fingerprint interactively or against
a supplied value, and stores the PEM in `config.json` for use as the sole TLS trust anchor
on future connections. Distinct from TOFU mode, which is scoped to Services.
_Avoid_: TOFU mode (reserved for Service bootstrap flags).

## 10. CA rotation recovery

When a managed CA rotates, the stored `ca_pem` no longer matches the controller's active
CA. The next CLI command that requires a TLS connection will fail with the error hint from §5.

**Managed CA rotation** (same operator, new keypair):

```sh
# Interactive — fingerprint visible in Dashboard > Global Settings
uptrakit auth ca trust

# Non-interactive / CI — fingerprint copy-pasted from Dashboard
uptrakit auth ca trust --tofu=e3676c6137dada24f41974e2fb62546dadc2c6d6b831e4bb2635393218c64ce4
```

**Migration to a public CA** (e.g. operator switches to Let's Encrypt):

```sh
# Discard the stored self-signed CA; revert to system roots
uptrakit auth ca forget
```

`auth login --tofu` deliberately fails at rotation even when an explicit fingerprint is
supplied (step 5 of §7). `auth ca trust` is the dedicated recovery command. This two-step
requirement is intentional: it keeps authentication and trust management as separate
operations and prevents silent trust changes during scripted re-login flows.

## 11. Testing

### Unit tests

- `parse_fingerprint`: plain hex accepted, `sha256:` prefix accepted and stripped, unknown
  prefix rejected, wrong length rejected, non-hex chars rejected.
- `establish_ca_trust` (mock server via `wiremock` or existing `TestApp` harness):
  - Fetch succeeds, fingerprint matches hint → saves PEM.
  - Fetch succeeds, fingerprint mismatches hint → error.
  - No hint, no TTY → error.
  - No hint, TTY, user accepts → saves PEM.
  - No hint, TTY, user rejects → aborts without saving.
  - Stored CA matches fetched → proceeds silently.
  - Stored CA differs, `allow_rotation=false` → rotation error.
  - Stored CA differs, `allow_rotation=true` → warning, proceeds.

### Integration tests (`TestApp` harness)

- `auth login --tofu` writes `ca_pem` to config.
- `auth login --tofu=<correct_fp>` writes `ca_pem`.
- `auth login --tofu=<wrong_fp>` fails with mismatch error.
- `auth login --tofu` with stored CA that matches → proceeds silently.
- `auth login --tofu` with stored CA that differs → rotation error (even when fingerprint hint matches new CA).
- `auth login --tofu=<new_fp>` with stored CA that differs → rotation error (hint does not bypass rotation block on login path).
- `auth ca trust` updates stored CA.
- `auth ca trust --tofu=<fp>` non-interactive path.
- `auth ca trust` with no TTY and no fingerprint → error.
- `auth ca forget` clears `ca_pem`.
- `authenticated_client` with stored `ca_pem` builds `tls_certs_only` custom-root client (not system roots).
- `--insecure` + `--tofu` rejected by clap.
- `auth status` JSON output includes `ca_fingerprint` field.

`start_paused = true` is not needed — no `tokio::time` usage in this feature.

## 12. Documentation deliverables

| Document                               | Change                                                                                                                                                                                     |
| -------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `CONTEXT.md`                           | Add "CLI CA Trust" term (§9)                                                                                                                                                               |
| `docs/security/tofu-tls.md`            | Add "CLI CA Trust" section: entry points, fingerprint format, rotation recovery, `--insecure` interaction                                                                                  |
| `docs/development/coding-standards.md` | Document `parse_fingerprint` contract, `establish_ca_trust` shared function location, and the rule to use `tls_certs_only` (not `add_root_certificate`) for pinned-CA-only reqwest clients |

No new ADR required — the decision is not surprising without context, is reversible
(`auth ca forget`), and no genuine alternative was rejected during design.

## 13. Deferred

- `auth logout` server-side API token revocation (Seam 1 per ADR-0009).
- SPKI pinning for the CLI (`--tofu-spki` flag, analogous to Service `--tofu-spki`).
- Root/intermediate CA split (ADR-0013).
- `--tofu-fingerprint` non-interactive alias (plain `--tofu=<fp>` covers this use case).
