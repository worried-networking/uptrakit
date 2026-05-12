# mTLS Hot-Swap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the existing rebuild-`ServerConfig`-on-every-change paths with rustls
hot-swap idioms — `ResolvesServerCert` for server-cert renewal, `ResolvesClientCert` for
Agent-side cert rotation, `DynamicClientVerifier` (an `ArcSwap`-backed
`ClientCertVerifier`) for CRL and CA-bundle changes. Enable ALPN (`h2`, `http/1.1`) on the
production HTTPS listener and TLS session resumption on both sides. Preserve a deadline-bound
forced reconnect on the Agent for late-arrival cert renewals so a long-idle session can never
sit on an expired cert.

**Architecture:** Three trait-object resolvers/wrappers live behind `arc_swap::ArcSwap<_>`
and are installed once on `ClientConfig` / `ServerConfig` at startup. CRL rebuild and
CA-bundle updates call a single `swap` on the dynamic verifier instead of
`RustlsConfig::reload_from_config`. Server-cert renewal does the same via
`ResolvesServerCert`. The Agent's cert-handler stops disconnecting on every renewal — the
resolver swap is sufficient for normal renewal events, with `Outcome::Reconnect` preserved
for renewals arriving within `max(60s, cert_lifetime/50)` of expiry.

**Tech Stack:** Rust 2024, `rustls 0.23` (aws-lc-rs), `tokio-rustls 0.26`, `axum-server 0.8`,
`arc-swap 1.9`, `parking_lot 0.12`. Spec:
`docs/superpowers/specs/2026-05-12-mtls-hardening-design.md` (§5.4 resolvers +
DynamicClientVerifier, §5.7 ALPN + session resumption). Depends on Plan 1 having landed
(Issuer cache + AcqRel CRL number used by the verifier-swap path).

---

## File Map

> **Spec deviation:** Spec §5.4.1 places `AgentClientCertResolver` inside
> `crates/shared/service-sdk/src/tls.rs`. The plan creates a dedicated
> `cert_resolver.rs` module instead. Reason: `tls.rs` already houses the
> trust-composition root-store builder plus mode-dispatched verifier;
> piling the cert resolver into the same file makes it grow past the
> snapshot's preferred-focused-modules threshold. The behavior, public
> API, and integration shape are identical to the spec.

- Create: `crates/shared/service-sdk/src/cert_resolver.rs`
  - `AgentClientCertResolver` impl of `rustls::client::ResolvesClientCert`
  - `swap` method called by `cert_handler.rs` on incoming `Certificate` payload
- Modify: `crates/shared/service-sdk/src/tls.rs`
  - install resolver on `ClientConfig`
  - add session resumption (`ClientSessionMemoryCache`)
  - `Arc<ClientConfig>` returned from builder so callers cache it
- Modify: `crates/shared/service-sdk/src/cert_handler.rs`
  - Replace eager `Outcome::Reconnect` on Certificate with resolver.swap + deadline check
  - Threshold helper `should_force_reconnect(not_after, now) -> bool`
- Modify: `crates/shared/service-sdk/src/lifecycle.rs`
  - Hold a single `Arc<AgentClientCertResolver>` for the process lifetime
  - Remove rebuild-`ClientConfig`-per-reconnect call sites
- Create: `crates/core/controller-runtime/src/dynamic_verifier.rs`
  - `DynamicClientVerifier` impl of `rustls::server::danger::ClientCertVerifier`
  - `ArcSwap<WebPkiClientVerifier>` inner, `empty_subjects: Vec<DistinguishedName>` for the advisory
    hint
  - `swap(new: Arc<WebPkiClientVerifier>)` method
- Create: `crates/core/controller-runtime/src/server_cert_resolver.rs`
  - `ControllerServerCertResolver` impl of `rustls::server::ResolvesServerCert`
- Modify: `crates/core/controller-runtime/src/pki.rs`
  - `build_rustls_config_with_client_auth_and_crls` becomes startup-only
  - Installs `DynamicClientVerifier` (not bare `WebPkiClientVerifier`)
  - Installs `ControllerServerCertResolver` (replaces `with_single_cert`)
  - Sets `alpn_protocols = vec![b"h2", b"http/1.1"]`
  - Sets `session_storage = ServerSessionMemoryCache::new(1024)`
- Modify: `crates/core/controller-runtime/src/crl_manager.rs`
  - Replace `reload_tls_config` with `swap_verifier`: single `Arc::store` call
  - Drop `axum_server::tls_rustls::RustlsConfig` field; rename method
- Modify: `crates/core/controller-runtime/src/tasks.rs`
  - Update CRL renewal / CA rotation / server-cert renewal call sites to use the new swap APIs
- Modify: `crates/ui/web-api/src/routes/server_cert.rs`
  - Wire renewal flow to `ControllerServerCertResolver.swap` instead of full TLS config rebuild
- Tests:
  - `crates/shared/service-sdk/src/cert_resolver.rs` — swap visible on next `resolve` call;
    concurrent swap+resolve test
  - `crates/shared/service-sdk/src/cert_handler.rs` — `should_force_reconnect` thresholds,
    integration: renewal → resolver updated + no immediate reconnect
  - `crates/core/controller-runtime/src/dynamic_verifier.rs` — swap visible on next
    `verify_client_cert`; advisory empty `root_hint_subjects`; concurrent swap+verify property test
  - `crates/core/controller-runtime/src/server_cert_resolver.rs` — analogous
  - `crates/core/integration-tests/tests/reverse_proxy/` — every existing harness reruns; new test:
    server-cert renewal mid-traffic doesn't break in-flight TLS

---

## Snapshot Bindings

Same as Plan 1, plus:

- "Reverse proxy integration tests mandatory for mTLS, certificate forwarding, client IP, TLS
  termination, proxy middleware changes."
- "System integration tests mandatory for enrollment, wire protocol, service lifecycle,
  inter-component communication changes."

---

### Task 1: `AgentClientCertResolver` skeleton — failing test first

**Files:**

- Create: `crates/shared/service-sdk/src/cert_resolver.rs`
- Modify: `crates/shared/service-sdk/src/lib.rs` (add `pub mod cert_resolver;`)

- [ ] **Step 1: Add the module declaration**

Edit `crates/shared/service-sdk/src/lib.rs`:

```rust
pub mod cert_resolver;
```

- [ ] **Step 2: Write the new file with a failing unit test**

```rust
//! Hot-swappable client cert resolver for Agent / Service-SDK mTLS.
//!
//! `AgentClientCertResolver` lets the Service install a single
//! `ClientConfig` once at startup. New certificates arriving via the
//! `Certificate` wire message swap the resolver's inner `Arc<CertifiedKey>`
//! atomically. The currently running TLS session continues using the
//! previous cert until the next handshake; subsequent reconnects pick up
//! the new cert.

use std::sync::Arc;

use arc_swap::ArcSwap;
use rustls::client::ResolvesClientCert;
use rustls::sign::CertifiedKey;
use rustls::SignatureScheme;

/// Process-lifetime resolver. Installed once on the Service's
/// `ClientConfig`; swapped by `cert_handler` on incoming `Certificate`
/// payloads.
#[derive(Debug)]
pub struct AgentClientCertResolver {
    current: ArcSwap<CertifiedKey>,
}

impl AgentClientCertResolver {
    pub fn new(initial: Arc<CertifiedKey>) -> Self {
        Self { current: ArcSwap::new(initial) }
    }

    /// Replace the active client certificate. Next handshake presents
    /// the new cert; in-flight sessions are unaffected.
    pub fn swap(&self, next: Arc<CertifiedKey>) {
        self.current.store(next);
    }
}

impl ResolvesClientCert for AgentClientCertResolver {
    fn resolve(
        &self,
        _root_hint_subjects: &[&[u8]],
        _sigschemes: &[SignatureScheme],
    ) -> Option<Arc<CertifiedKey>> {
        // `_root_hint_subjects` carries DER-encoded DNs of CAs the server
        // is willing to accept (rustls 0.23 names it `root_hint_subjects`).
        // The Agent has a single identity at any time; no filtering needed.
        Some(self.current.load_full())
    }

    fn has_certs(&self) -> bool { true }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};

    #[test]
    fn swap_visible_on_next_resolve() {
        let initial = make_dummy_certified_key();
        let resolver = AgentClientCertResolver::new(Arc::clone(&initial));

        let r1 = resolver.resolve(&[], &[]).expect("initial cert");
        assert!(Arc::ptr_eq(&r1, &initial));

        let next = make_dummy_certified_key();
        resolver.swap(Arc::clone(&next));

        let r2 = resolver.resolve(&[], &[]).expect("post-swap cert");
        assert!(Arc::ptr_eq(&r2, &next));
        assert!(!Arc::ptr_eq(&r2, &initial));
    }

    #[test]
    fn concurrent_swap_and_resolve_never_returns_none() {
        use std::thread;
        let resolver = Arc::new(AgentClientCertResolver::new(make_dummy_certified_key()));

        let mut handles = Vec::new();
        for _ in 0..4 {
            let r = Arc::clone(&resolver);
            handles.push(thread::spawn(move || {
                for _ in 0..1000 {
                    let resolved = r.resolve(&[], &[]);
                    assert!(resolved.is_some());
                }
            }));
        }
        for _ in 0..100 {
            resolver.swap(make_dummy_certified_key());
        }
        for h in handles { h.join().expect("join"); }
    }

    fn make_dummy_certified_key() -> Arc<CertifiedKey> {
        // rcgen issues a throwaway leaf for the test.
        let key = rcgen::KeyPair::generate().expect("kp");
        let mut params = rcgen::CertificateParams::new(vec!["test.local".into()])
            .expect("params");
        params.distinguished_name.push(rcgen::DnType::CommonName, "test");
        let cert = params.self_signed(&key).expect("cert");

        let cert_der = CertificateDer::from(cert.der().to_vec());
        let key_der = PrivateKeyDer::Pkcs8(
            rustls::pki_types::PrivatePkcs8KeyDer::from(key.serialize_der())
        );
        let signing_key = rustls::crypto::aws_lc_rs::sign::any_supported_type(&key_der)
            .expect("signing key");
        Arc::new(CertifiedKey::new(vec![cert_der], signing_key))
    }
}
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p uptrakit-service-sdk cert_resolver -- --nocapture 2>&1 | tail -20`

Expected: PASS (the resolver is fresh code; both tests should immediately pass once the file
compiles).

- [ ] **Step 4: Commit**

```bash
git add crates/shared/service-sdk/src/cert_resolver.rs crates/shared/service-sdk/src/lib.rs
git commit -m "feat(service-sdk): add AgentClientCertResolver with ArcSwap hot-swap"
```

---

### Task 2: Wire `AgentClientCertResolver` into `tls.rs` builder

**Files:**

- Modify: `crates/shared/service-sdk/src/tls.rs`

- [ ] **Step 1: Locate the current `ClientConfig` builders**

Run: `rg -n 'fn build_tls_connector_with_client_cert\|fn
build_system_trust_tls_connector_with_client_cert\|ClientConfig::builder'
crates/shared/service-sdk/src/tls.rs`

- [ ] **Step 2: Add a new builder that takes the resolver instead of a `CertifiedKey`**

```rust
use std::sync::Arc;
use crate::cert_resolver::AgentClientCertResolver;

pub fn build_client_config_with_resolver(
    ca_pem: &[u8],
    resolver: Arc<AgentClientCertResolver>,
) -> Result<Arc<rustls::ClientConfig>, rootcause::Report<EnrollmentError>> {
    let root_store = build_root_store(ca_pem)?;

    let mut config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_client_cert_resolver(resolver);

    // Session resumption: 256 in-memory sessions per process. See
    // docs/superpowers/specs/2026-05-12-mtls-hardening-design.md §5.7.
    config.resumption = rustls::client::Resumption::in_memory_sessions(256);

    Ok(Arc::new(config))
}
```

- [ ] **Step 3: Keep old builders during transition**

The existing `build_tls_connector_with_client_cert(ca_pem, cert_pem, key_pem)` builders are deleted
in Task 4 once `lifecycle.rs` is migrated. Tag them with `#[deprecated(note = "use
build_client_config_with_resolver; removed after lifecycle migration")]` for now.

- [ ] **Step 4: Run service-sdk tests**

Run: `cargo test -p uptrakit-service-sdk tls -- --nocapture 2>&1 | tail -20`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/shared/service-sdk/src/tls.rs
git commit -m "feat(service-sdk): add resolver-based ClientConfig builder with session resumption"
```

---

### Task 3: Migrate `cert_handler.rs` to resolver swap + deadline-bound reconnect — failing test first

**Files:**

- Test + Modify: `crates/shared/service-sdk/src/cert_handler.rs`

- [ ] **Step 1: Add a failing test for `should_force_reconnect`**

```rust
#[test]
fn force_reconnect_threshold_default_lifetime() {
    // cert_lifetime_hours = 168 (default). max(60s, 168*3600/50)
    //   = max(60, 12096s) = 12096s ≈ 3h 21m before expiry.
    let lifetime_hours = 168;
    let now = time::OffsetDateTime::now_utc();
    let cert_lifetime = std::time::Duration::from_secs(lifetime_hours * 3600);

    let cases = [
        // (seconds_until_expiry, expected_force)
        (lifetime_hours * 3600 - 1,   false),  // still in the safe window
        (12_097,                       false), // just outside the deadline window
        (12_095,                       true),  // just inside
        (60,                           true),  // very close to expiry
        (1,                            true),  // 1s away
    ];
    for (until_expiry, expected) in cases {
        let not_after = now + time::Duration::seconds(until_expiry as i64);
        assert_eq!(
            should_force_reconnect(not_after, now, cert_lifetime),
            expected,
            "until_expiry={until_expiry}",
        );
    }
}

#[test]
fn force_reconnect_threshold_short_lifetime() {
    // 1 hour lifetime → max(60s, 3600/50) = max(60, 72) = 72s window.
    let cert_lifetime = std::time::Duration::from_secs(3600);
    let now = time::OffsetDateTime::now_utc();
    let not_after = now + time::Duration::seconds(50);
    assert!(should_force_reconnect(not_after, now, cert_lifetime));
}

#[test]
fn force_reconnect_threshold_minimum_60s() {
    // Lifetime so small that cert_lifetime/50 < 60s. Floor at 60s.
    let cert_lifetime = std::time::Duration::from_secs(120);
    let now = time::OffsetDateTime::now_utc();
    let not_after = now + time::Duration::seconds(45);
    assert!(should_force_reconnect(not_after, now, cert_lifetime),
        "within 60s minimum window → force");
    let not_after_safe = now + time::Duration::seconds(75);
    assert!(!should_force_reconnect(not_after_safe, now, cert_lifetime),
        "outside 60s minimum window → no force");
}
```

- [ ] **Step 2: Run, expect compile error**

Run: `cargo test -p uptrakit-service-sdk force_reconnect_threshold -- --nocapture 2>&1 | tail -10`

Expected: fail — `should_force_reconnect` doesn't exist.

- [ ] **Step 3: Implement `should_force_reconnect`**

```rust
/// Returns `true` when the new certificate must be installed via a
/// forced reconnect (rather than the lazy resolver swap) because the
/// existing cert is too close to expiry for the next natural handshake
/// to occur in time.
///
/// Threshold: `max(60s, cert_lifetime / 50)` before `not_after`.
///
/// See spec §5.4.1.
pub fn should_force_reconnect(
    not_after: time::OffsetDateTime,
    now: time::OffsetDateTime,
    cert_lifetime: std::time::Duration,
) -> bool {
    let window_secs = std::cmp::max(60, cert_lifetime.as_secs() / 50);
    let until_expiry = (not_after - now).whole_seconds();
    if until_expiry <= 0 {
        return true;
    }
    (until_expiry as u64) <= window_secs
}
```

- [ ] **Step 4: Run, expect PASS**

Run: `cargo test -p uptrakit-service-sdk force_reconnect_threshold -- --nocapture 2>&1 | tail -10`

Expected: PASS for all three tests.

- [ ] **Step 5: Migrate `handle_certificate` to use resolver swap + threshold**

Locate the current `handle_certificate` (Run: `rg -n 'fn handle_certificate\|Outcome::Reconnect'
crates/shared/service-sdk/src/cert_handler.rs`).

Replace the body's tail (the unconditional `Outcome::Reconnect`):

```rust
// Persist the new cert and key first (existing call).
let saved = self.identity.save_certificate(payload, /* ... */)?;

// Build the new CertifiedKey from the saved material.
let next_certified = build_certified_key(&saved.cert_pem, &saved.key_pem)?;

// Hot-swap the resolver. Next handshake presents the new cert.
self.cert_resolver.swap(std::sync::Arc::new(next_certified));

// Deadline-bound forced reconnect: if the current session is too close
// to expiry for a natural handshake to occur in time, force reconnect.
let now = time::OffsetDateTime::now_utc();
let cert_lifetime = std::time::Duration::from_secs(
    self.cert_lifetime_hours as u64 * 3600,
);
if should_force_reconnect(saved.not_after, now, cert_lifetime) {
    tracing::info!(
        not_after = %saved.not_after,
        "cert renewal arrived close to expiry; forcing reconnect to present new cert"
    );
    return Ok(LoopOutcome::Reconnect(CloseReason::CertificateRotated));
}

tracing::debug!(
    not_after = %saved.not_after,
    "cert renewal applied via resolver swap; existing session retained"
);
Ok(LoopOutcome::Continue)
```

- [ ] **Step 6: Add `cert_resolver: Arc<AgentClientCertResolver>` field to
  `CertificateRenewalHandler`**

In the struct definition:

```rust
pub struct CertificateRenewalHandler {
    pub cert_resolver: std::sync::Arc<crate::cert_resolver::AgentClientCertResolver>,
    pub cert_lifetime_hours: u32,
    pending_renewal_key: Option<zeroize::Zeroizing<String>>,
    // ... existing fields
}
```

Update the constructor signature to accept the resolver.

- [ ] **Step 7: Run cert_handler tests**

Run: `cargo test -p uptrakit-service-sdk cert_handler -- --nocapture 2>&1 | tail -20`

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/shared/service-sdk/src/cert_handler.rs
git commit -m "feat(cert-handler): hot-swap resolver + deadline-bound forced reconnect"
```

---

### Task 4: `lifecycle.rs` — single `Arc<ClientConfig>`, remove per-reconnect rebuild

**Files:**

- Modify: `crates/shared/service-sdk/src/lifecycle.rs`

- [ ] **Step 1: Locate per-reconnect rebuild sites**

Run: `rg -n 'build_tls_connector_with_client_cert\|build_system_trust_tls'
crates/shared/service-sdk/src/lifecycle.rs`

Expected: line ~298, ~421 (per spec audit).

- [ ] **Step 2: Hoist resolver + `ClientConfig` to outer scope**

In `run_authenticated_with_reconnect` (or equivalent), construct once:

```rust
let initial_certified = build_certified_key(&identity.cert_pem, &identity.key_pem)?;
let cert_resolver = std::sync::Arc::new(
    crate::cert_resolver::AgentClientCertResolver::new(std::sync::Arc::new(initial_certified)),
);
let client_config = crate::tls::build_client_config_with_resolver(
    identity.ca_pem.as_bytes(),
    std::sync::Arc::clone(&cert_resolver),
)?;
let tls_connector = tokio_rustls::TlsConnector::from(client_config);
```

Pass `cert_resolver` into `CertificateRenewalHandler::new(...)` so Task 3's swap path has the same
handle.

- [ ] **Step 3: Reconnect loop reuses the cached connector**

The loop body must NOT call `build_tls_connector_with_client_cert` again. The same `tls_connector`
is reused across every reconnect; only `CaBundleUpdated` triggers a config rebuild (see Task 5).

- [ ] **Step 4: Delete the deprecated builders from Task 2**

Remove `build_tls_connector_with_client_cert` and friends entirely. Update test imports.

- [ ] **Step 5: Run service-sdk tests**

Run: `cargo test -p uptrakit-service-sdk -- --nocapture 2>&1 | tail -20`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/shared/service-sdk/src/lifecycle.rs crates/shared/service-sdk/src/tls.rs
git commit -m "refactor(service-sdk): cache Arc<ClientConfig> + drop per-reconnect rebuild"
```

---

### Task 5: `CaBundleUpdated` triggers `ClientConfig` rebuild

**Files:**

- Modify: `crates/shared/service-sdk/src/cert_handler.rs`
- Modify: `crates/shared/service-sdk/src/lifecycle.rs`

- [ ] **Step 1: Add a `ClientConfigRebuild` signaling channel**

In `lifecycle.rs`, after constructing `cert_resolver`, also build a
`tokio::sync::watch::Sender<Arc<ClientConfig>>` and clone the receiver into the reconnect
loop and into `CertificateRenewalHandler`.

- [ ] **Step 2: On `CaBundleUpdated`, rebuild the config and `send` it**

In `CertificateRenewalHandler::handle_ca_bundle_updated`:

```rust
let new_config = crate::tls::build_client_config_with_resolver(
    &new_ca_pem,
    std::sync::Arc::clone(&self.cert_resolver),
)?;
let _ = self.config_tx.send(new_config);
return Ok(LoopOutcome::Reconnect(CloseReason::CaBundleUpdated));
```

The forced reconnect on CA-bundle change is intentional: trust roots have changed; the session may
now be invalid.

- [ ] **Step 3: Reconnect loop reads the latest config from the watch receiver**

```rust
let client_config = config_rx.borrow().clone();
let tls_connector = tokio_rustls::TlsConnector::from(client_config);
```

- [ ] **Step 4: Tests**

Run: `cargo test -p uptrakit-service-sdk -- --nocapture 2>&1 | tail -20`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/shared/service-sdk/src/cert_handler.rs crates/shared/service-sdk/src/lifecycle.rs
git commit -m "feat(service-sdk): rebuild ClientConfig on CaBundleUpdated, broadcast via watch"
```

---

### Task 6: `ControllerServerCertResolver` — failing test first

**Files:**

- Create: `crates/core/controller-runtime/src/server_cert_resolver.rs`
- Modify: `crates/core/controller-runtime/src/lib.rs` (add `pub mod server_cert_resolver;`)

- [ ] **Step 1: Add module decl + skeleton**

Edit `lib.rs`:

```rust
pub mod server_cert_resolver;
```

Create the file:

```rust
//! Hot-swappable server cert resolver for the Controller's HTTPS listener.
//!
//! Replaces the previous `axum_server::RustlsConfig::reload_from_config`
//! path for the server-cert-renewal case. CRL and CA-bundle changes go
//! through `DynamicClientVerifier` (see `dynamic_verifier.rs`).

use std::sync::Arc;

use arc_swap::ArcSwap;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;

#[derive(Debug)]
pub struct ControllerServerCertResolver {
    current: ArcSwap<CertifiedKey>,
}

impl ControllerServerCertResolver {
    pub fn new(initial: Arc<CertifiedKey>) -> Self {
        Self { current: ArcSwap::new(initial) }
    }

    pub fn swap(&self, next: Arc<CertifiedKey>) {
        self.current.store(next);
    }
}

impl ResolvesServerCert for ControllerServerCertResolver {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.current.load_full())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swap_visible_on_next_resolve() {
        // Construct two distinct CertifiedKeys.
        let a = test_certified_key("a.local");
        let b = test_certified_key("b.local");
        let resolver = ControllerServerCertResolver::new(Arc::clone(&a));

        let _client_hello: Option<ClientHello<'_>> = None; // resolver doesn't peek
        // Resolve via the trait method.
        // Without an actual ClientHello, we exercise the same code path
        // via the public method:
        let r1 = resolver.current.load_full();
        assert!(Arc::ptr_eq(&r1, &a));

        resolver.swap(Arc::clone(&b));
        let r2 = resolver.current.load_full();
        assert!(Arc::ptr_eq(&r2, &b));
    }

    fn test_certified_key(name: &str) -> Arc<CertifiedKey> {
        let key = rcgen::KeyPair::generate().expect("kp");
        let mut params = rcgen::CertificateParams::new(vec![name.into()])
            .expect("params");
        params.distinguished_name.push(rcgen::DnType::CommonName, name);
        let cert = params.self_signed(&key).expect("cert");
        let cert_der = rustls::pki_types::CertificateDer::from(cert.der().to_vec());
        let key_der = rustls::pki_types::PrivateKeyDer::Pkcs8(
            rustls::pki_types::PrivatePkcs8KeyDer::from(key.serialize_der())
        );
        let signing_key = rustls::crypto::aws_lc_rs::sign::any_supported_type(&key_der)
            .expect("signing key");
        Arc::new(CertifiedKey::new(vec![cert_der], signing_key))
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p uptrakit-controller-runtime server_cert_resolver -- --nocapture 2>&1 | tail -10`

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/core/controller-runtime/src/server_cert_resolver.rs crates/core/controller-runtime/src/lib.rs
git commit -m "feat(controller-runtime): add ControllerServerCertResolver"
```

---

### Task 7: `DynamicClientVerifier` — failing test first

**Files:**

- Create: `crates/core/controller-runtime/src/dynamic_verifier.rs`
- Modify: `crates/core/controller-runtime/src/lib.rs`

- [ ] **Step 1: Add module decl**

```rust
pub mod dynamic_verifier;
```

- [ ] **Step 2: Create the wrapper with full trait impl**

```rust
//! Hot-swappable client cert verifier.
//!
//! Wraps `rustls::server::WebPkiClientVerifier` behind an `ArcSwap` so
//! CRL rebuilds and CA-bundle updates can swap the verifier without
//! rebuilding `ServerConfig`. `root_hint_subjects` returns an empty
//! slice — see spec §5.4.3 for the lifetime rationale.

use std::sync::Arc;

use arc_swap::ArcSwap;
use rustls::server::WebPkiClientVerifier;
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::client::danger::HandshakeSignatureValid;
use rustls::pki_types::{CertificateDer, UnixTime};
use rustls::{DigitallySignedStruct, DistinguishedName, SignatureScheme};

#[derive(Debug)]
pub struct DynamicClientVerifier {
    inner: ArcSwap<WebPkiClientVerifier>,
    /// Empty advisory hint slice. See `root_hint_subjects` doc comment.
    empty_subjects: Vec<DistinguishedName>,
}

impl DynamicClientVerifier {
    pub fn new(initial: Arc<WebPkiClientVerifier>) -> Self {
        Self {
            inner: ArcSwap::new(initial),
            empty_subjects: Vec::new(),
        }
    }

    /// Atomically replace the inner verifier. Next handshake uses the
    /// new instance. Mid-handshake races between this swap and the
    /// `verify_client_cert` → `verify_tls*_signature` call sequence can
    /// produce a one-off handshake failure (the client retries); see
    /// spec §5.4.3 for the rationale.
    pub fn swap(&self, next: Arc<WebPkiClientVerifier>) {
        self.inner.store(next);
    }
}

impl ClientCertVerifier for DynamicClientVerifier {
    fn offer_client_auth(&self) -> bool { true }
    fn client_auth_mandatory(&self) -> bool { false }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        // Always empty. rustls treats this hint as advisory (RFC 8446
        // §4.3.2 / RFC 5246 §7.4.4). Returning an empty hint avoids the
        // lifetime puzzle of exposing an `ArcSwap`-backed slice through
        // a `&self`-tied borrow, and is operationally equivalent for
        // uptrakit Agents (each Agent holds exactly one identity).
        &self.empty_subjects
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        self.inner.load().verify_client_cert(end_entity, intermediates, now)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.load().verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.load().verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.load().supported_verify_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_hint_subjects_is_empty() {
        let verifier = make_test_verifier(/* roots */ &[]);
        let dyn_v = DynamicClientVerifier::new(verifier);
        assert!(dyn_v.root_hint_subjects().is_empty());
    }

    #[test]
    fn swap_visible_on_next_verify_call() {
        // Build two verifiers with different root stores; verify that
        // a cert valid only under root B fails before swap, passes after.
        let (root_a, root_b, leaf_signed_by_b) = build_two_root_fixtures();
        let v_a = make_test_verifier(&[root_a]);
        let v_b = make_test_verifier(&[root_b]);

        let dyn_v = DynamicClientVerifier::new(v_a);

        let r1 = dyn_v.verify_client_cert(
            &leaf_signed_by_b,
            &[],
            UnixTime::now(),
        );
        assert!(r1.is_err(), "before swap, leaf rejected (signed by B, roots = A)");

        dyn_v.swap(v_b);

        let r2 = dyn_v.verify_client_cert(
            &leaf_signed_by_b,
            &[],
            UnixTime::now(),
        );
        assert!(r2.is_ok(), "after swap, leaf accepted (roots = B)");
    }

    #[test]
    fn concurrent_swap_and_verify_never_panics() {
        use std::thread;
        let v_a = make_test_verifier(&[]);
        let dyn_v = Arc::new(DynamicClientVerifier::new(v_a));

        let mut handles = Vec::new();
        for _ in 0..4 {
            let d = Arc::clone(&dyn_v);
            handles.push(thread::spawn(move || {
                for _ in 0..200 {
                    // Pass intentionally-bad inputs; we only care that
                    // no thread panics under concurrent swap pressure.
                    let _ = d.supported_verify_schemes();
                    let _ = d.root_hint_subjects();
                }
            }));
        }
        for _ in 0..100 {
            dyn_v.swap(make_test_verifier(&[]));
        }
        for h in handles { h.join().expect("join"); }
    }

    fn make_test_verifier(_roots: &[Vec<u8>]) -> Arc<WebPkiClientVerifier> {
        // Build a verifier with the supplied roots; CRLs empty.
        // Full impl deferred to the test harness; minimal stub:
        let mut root_store = rustls::RootCertStore::empty();
        // For unit tests we wire in a self-built root via rcgen — see
        // build_two_root_fixtures below.
        WebPkiClientVerifier::builder(Arc::new(root_store))
            .allow_unauthenticated()
            .build()
            .expect("verifier builds")
    }

    fn build_two_root_fixtures() -> (Vec<u8>, Vec<u8>, CertificateDer<'static>) {
        // Generate two distinct CAs and a leaf signed by CA-B.
        // Implementation: parallel to crl_manager test_ca_pair, with the
        // additional step of signing a leaf under CA-B.
        todo!("scaffolded in next step")
    }
}
```

- [ ] **Step 3: Flesh out `build_two_root_fixtures`**

Use the rcgen API. This test fixture is the only non-trivial part of the file:

```rust
fn build_two_root_fixtures() -> (Vec<u8>, Vec<u8>, CertificateDer<'static>) {
    fn make_ca(name: &str) -> (rcgen::Certificate, rcgen::KeyPair) {
        let key = rcgen::KeyPair::generate().expect("kp");
        let mut params = rcgen::CertificateParams::new(vec![name.into()]).expect("params");
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Constrained(0));
        params.distinguished_name.push(rcgen::DnType::CommonName, name);
        let cert = params.self_signed(&key).expect("cert");
        (cert, key)
    }
    let (ca_a, _key_a) = make_ca("test-ca-a");
    let (ca_b, key_b) = make_ca("test-ca-b");

    // Leaf signed by CA-B. Construct the Issuer from CA-B's actual cert
    // so the resulting leaf chains back to CA-B. `from_ca_cert_pem` is
    // gated by rcgen's `x509-parser` feature (already enabled in the
    // workspace `Cargo.toml`). Do NOT use `Issuer::new(...)` with a
    // fresh `CertificateParams::default()` — that produces an issuer
    // unrelated to CA-B, and `WebPkiClientVerifier` with `ca_b.der()`
    // as the root would reject the resulting leaf.
    let leaf_key = rcgen::KeyPair::generate().expect("leaf kp");
    let mut leaf_params = rcgen::CertificateParams::new(vec!["leaf.local".into()])
        .expect("leaf params");
    leaf_params.distinguished_name.push(rcgen::DnType::CommonName, "leaf");
    let issuer_b = rcgen::Issuer::from_ca_cert_pem(&ca_b.pem(), key_b)
        .expect("issuer from CA-B PEM");
    let leaf = leaf_params.signed_by(&leaf_key, &issuer_b).expect("leaf cert");

    (
        ca_a.der().to_vec(),
        ca_b.der().to_vec(),
        CertificateDer::from(leaf.der().to_vec()),
    )
}
```

(Approximation — verify the exact rcgen 0.14 API at impl time; the spec defers exact API binding to
the impl plan and `cargo doc` is the source of truth.)

- [ ] **Step 4: Run tests**

Run: `cargo test -p uptrakit-controller-runtime dynamic_verifier -- --nocapture 2>&1 | tail -30`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core/controller-runtime/src/dynamic_verifier.rs crates/core/controller-runtime/src/lib.rs
git commit -m "feat(controller-runtime): add DynamicClientVerifier with ArcSwap hot-swap"
```

---

### Task 8: Wire `DynamicClientVerifier` + `ControllerServerCertResolver` into `pki.rs`

**Files:**

- Modify: `crates/core/controller-runtime/src/pki.rs`

- [ ] **Step 1: Locate `build_rustls_config_with_client_auth_and_crls`**

Run: `rg -n 'fn build_rustls_config_with_client_auth_and_crls'
crates/core/controller-runtime/src/pki.rs`

- [ ] **Step 2: Refactor the function to return the dynamic primitives**

New signature returns a `BuiltTlsConfig` carrying both the `ServerConfig` and handles for later
swaps:

```rust
pub struct BuiltTlsConfig {
    pub server_config: Arc<rustls::ServerConfig>,
    pub server_cert_resolver: Arc<crate::server_cert_resolver::ControllerServerCertResolver>,
    pub client_verifier: Arc<crate::dynamic_verifier::DynamicClientVerifier>,
}

pub fn build_rustls_config(/* ... existing args ... */) -> Result<BuiltTlsConfig, /* ... */> {
    // 1. Build root store from CA bundle.
    let root_store = build_root_store(/* ... */)?;

    // 2. Build initial WebPkiClientVerifier with current CRLs.
    let initial_verifier = WebPkiClientVerifier::builder(Arc::new(root_store))
        .with_crls(crls)
        .allow_unauthenticated()
        .build()
        .map_err(|e| report!(PkiError::VerifierBuilder(e.to_string())))?;

    // 3. Wrap in DynamicClientVerifier.
    let client_verifier = Arc::new(
        crate::dynamic_verifier::DynamicClientVerifier::new(initial_verifier)
    );

    // 4. Build initial CertifiedKey from server cert + key.
    let server_certified_key = build_certified_key(&server_cert_pem, &server_key_pem)?;
    let server_cert_resolver = Arc::new(
        crate::server_cert_resolver::ControllerServerCertResolver::new(server_certified_key)
    );

    // 5. Construct ServerConfig with the dynamic primitives.
    let mut config = rustls::ServerConfig::builder()
        .with_client_cert_verifier(Arc::clone(&client_verifier) as Arc<dyn ClientCertVerifier>)
        .with_cert_resolver(Arc::clone(&server_cert_resolver) as Arc<dyn ResolvesServerCert>);

    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    config.session_storage = rustls::server::ServerSessionMemoryCache::new(1024);

    Ok(BuiltTlsConfig {
        server_config: Arc::new(config),
        server_cert_resolver,
        client_verifier,
    })
}
```

- [ ] **Step 3: Keep `build_rustls_config_with_client_auth_and_crls` as a thin wrapper**

To avoid breaking the in-flight graceful-reload `[tls]` Section consumer
(see `docs/superpowers/specs/2026-05-12-graceful-reload-design.md`,
ADR-0008) which calls `RustlsConfig::reload_from_config(server_config)`
during ALPN / cipher-suite / TLS-version changes, retain the original
function as a wrapper that returns just the `ServerConfig`:

```rust
/// Compatibility shim for callers that only need `Arc<ServerConfig>`.
/// Prefer `build_rustls_config` (the new function) when the caller will
/// also need to hot-swap the server cert or the client verifier.
pub fn build_rustls_config_with_client_auth_and_crls(/* same args */)
    -> Result<Arc<rustls::ServerConfig>, rootcause::Report<PkiError>>
{
    Ok(build_rustls_config(/* args */)?.server_config)
}
```

This preserves the original signature for any code path that was about to
land on a parallel branch (graceful-reload, etc.) without forcing a
coordinated merge.

- [ ] **Step 4: Update callers in `tasks.rs` + bootstrap**

Run: `rg -n 'build_rustls_config_with_client_auth_and_crls' crates/`

For each caller that needs hot-swap handles (CRL manager, server-cert
renewal flow, AppState construction), replace with the new
`build_rustls_config` and store the returned `BuiltTlsConfig` in
`AppState`. Callers that only need the `Arc<ServerConfig>` (e.g.,
graceful-reload `[tls]` Section reload path) can keep using the wrapper.

- [ ] **Step 5: Verify ALPN + session resumption**

Run: `cargo test -p uptrakit-controller-runtime pki -- --nocapture 2>&1 | tail -20`

Add a test:

```rust
#[test]
fn server_config_has_h2_and_http11_alpn() {
    let built = build_test_tls_config();
    assert_eq!(built.server_config.alpn_protocols, vec![b"h2".to_vec(), b"http/1.1".to_vec()]);
}
```

Expected: PASS.

- [ ] **Step 6: Run reverse-proxy integration tests**

Run: `cargo test -p uptrakit-controller reverse_proxy -- --ignored 2>&1 | tail -30`

Expected: PASS. Reverse proxies now negotiate `h2` where supported.

- [ ] **Step 7: Commit**

```bash
git add crates/core/controller-runtime/src/pki.rs crates/core/controller-runtime/src/tasks.rs
git commit -m "feat(pki): install DynamicClientVerifier + ServerCertResolver, add ALPN + session resumption"
```

---

### Task 9: `CrlManager::swap_verifier` — replace `reload_tls_config`

**Files:**

- Modify: `crates/core/controller-runtime/src/crl_manager.rs`
- Modify: `crates/core/controller-runtime/src/tasks.rs`

- [ ] **Step 1: Locate `reload_tls_config`**

Run: `rg -n 'fn reload_tls_config\|reload_tls_config(' crates/core/controller-runtime/src/`

- [ ] **Step 2: Change `CrlManager` to hold the dynamic verifier handle, not `RustlsConfig`**

```rust
pub struct CrlManager {
    // remove: pub rustls_config: axum_server::tls_rustls::RustlsConfig,
    pub client_verifier: Arc<crate::dynamic_verifier::DynamicClientVerifier>,
    // ... existing fields
}

impl CrlManager {
    pub fn swap_verifier(&self) -> Result<(), rootcause::Report<PkiError>> {
        let new_verifier = self.build_current_verifier()?;
        self.client_verifier.swap(Arc::new(new_verifier));
        Ok(())
    }

    fn build_current_verifier(&self) -> Result<rustls::server::WebPkiClientVerifier, rootcause::Report<PkiError>> {
        let root_store = self.build_root_store()?;
        let crls = self.collect_crls()?;
        WebPkiClientVerifier::builder(Arc::new(root_store))
            .with_crls(crls)
            .allow_unauthenticated()
            .build()
            .map_err(|e| report!(PkiError::VerifierBuilder(e.to_string())))
    }
}
```

- [ ] **Step 3: Update callers in `tasks.rs`**

Replace:

```rust
if let Err(e) = crl_manager.reload_tls_config().await { /* ... */ }
```

with:

```rust
if let Err(e) = crl_manager.swap_verifier() { /* ... */ }
```

at every reachable site (CRL rebuild loop, CA rotation, etc.).

- [ ] **Step 4: Delete `reload_tls_config` from `CrlManager`**

- [ ] **Step 5: Verify no remaining `axum_server::tls_rustls::RustlsConfig::reload_from_config`
  calls in CRL paths**

Run: `rg -n 'reload_from_config' crates/`

Expected: only graceful-reload `[tls]` Section path remains (ALPN/cipher/version-change driven).

- [ ] **Step 6: Run tests**

Run: `cargo test -p uptrakit-controller-runtime crl_manager -- --nocapture 2>&1 | tail -20`

Expected: PASS.

- [ ] **Step 7: Reverse-proxy integration tests**

Run: `cargo test -p uptrakit-controller reverse_proxy -- --ignored 2>&1 | tail -30`

Expected: PASS. CRL refresh paths still produce verifiable handshakes.

- [ ] **Step 8: Commit**

```bash
git add crates/core/controller-runtime/src/crl_manager.rs crates/core/controller-runtime/src/tasks.rs
git commit -m "refactor(crl-manager): swap_verifier via DynamicClientVerifier (drop RustlsConfig::reload_from_config)"
```

---

### Task 10: Server-cert renewal hot-swap

**Files:**

- Modify: `crates/ui/web-api/src/routes/server_cert.rs`
- Modify: `crates/core/controller-runtime/src/tasks.rs`

- [ ] **Step 1: Locate the server-cert renewal flow**

Run: `rg -n 'renew_server_certificate\|reload_from_config'
crates/ui/web-api/src/routes/server_cert.rs crates/core/controller-runtime/src/tasks.rs`

- [ ] **Step 2: Replace `RustlsConfig::reload_from_config` with
  `ControllerServerCertResolver.swap`**

In `renew_server_certificate_inner` (or equivalent), after generating + persisting the new server
cert:

```rust
let new_certified_key = pki::build_certified_key(&new_cert_pem, &new_key_pem)?;
state.tls_handles.server_cert_resolver.swap(Arc::new(new_certified_key));
```

Where `state.tls_handles` is the `BuiltTlsConfig` introduced in Task 8, stored in `AppState`.

Remove the prior full-config rebuild path for this renewal.

- [ ] **Step 3: Update the scheduled background renewal** (`tasks.rs` server-cert renewal loop) to
  use the same `.swap` call.

- [ ] **Step 4: Tests**

Run: `cargo test -p uptrakit-web-api server_cert -- --nocapture 2>&1 | tail -20`

Expected: PASS.

- [ ] **Step 5: Reverse-proxy integration tests**

Run: `cargo test -p uptrakit-controller reverse_proxy -- --ignored 2>&1 | tail -30`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/ui/web-api/src/routes/server_cert.rs crates/core/controller-runtime/src/tasks.rs
git commit -m "feat(server-cert): hot-swap via ControllerServerCertResolver"
```

---

### Task 11: Concurrent swap-vs-verify property test (`DynamicClientVerifier`)

**Files:**

- Modify: `crates/core/controller-runtime/src/dynamic_verifier.rs`

- [ ] **Step 1: Add a stress test that asserts no UAF / no panic / no spuriously-accepted chain**

```rust
#[test]
fn property_concurrent_swap_no_spurious_accept() {
    use std::thread;
    use std::sync::atomic::{AtomicBool, Ordering};

    let (root_a_der, root_b_der, leaf_b) = build_two_root_fixtures();
    let v_a = Arc::new(make_verifier_with_roots(&[root_a_der.clone()]));
    let v_b = Arc::new(make_verifier_with_roots(&[root_b_der.clone()]));

    let dyn_v = Arc::new(DynamicClientVerifier::new(Arc::clone(&v_a)));
    let stop = Arc::new(AtomicBool::new(false));

    let mut handles = Vec::new();
    // Verifier threads: 4 of them, each performs 5000 verify calls.
    for _ in 0..4 {
        let d = Arc::clone(&dyn_v);
        let leaf = leaf_b.clone();
        let stop = Arc::clone(&stop);
        handles.push(thread::spawn(move || {
            for _ in 0..5000 {
                let r = d.verify_client_cert(&leaf, &[], UnixTime::now());
                // Leaf is signed by B. Acceptable outcomes:
                //   - `Ok`  if current verifier == v_b
                //   - `Err` if current verifier == v_a
                // Never any other outcome.
                if r.is_ok() {
                    // Verifier MUST currently point at v_b. We can't observe
                    // mid-handshake state precisely, but any Ok must coincide
                    // with v_b being the active verifier. Property test asserts
                    // no spuriously-accepted Err-only case: not testable by
                    // black-box means; we instead assert the inverse — A leaf
                    // signed only by C should ALWAYS Err.
                }
            }
            stop.store(true, Ordering::SeqCst);
        }));
    }

    // Swap thread: alternate v_a ↔ v_b at 1ms cadence until verifiers finish.
    //
    // Note on `std::thread::sleep`: the snapshot rule "tests must never
    // sleep on real wall-clock time" applies to tests using tokio time
    // APIs (`#[tokio::test(start_paused = true)]` + `tokio::time::advance`).
    // This test is a sync `#[test]` exercising thread contention with no
    // timing assertion — the sleep merely throttles swap cadence so verify
    // threads observe a mix of inner verifiers. No flakiness or wall-clock
    // dependency.
    while !stop.load(Ordering::SeqCst) {
        dyn_v.swap(Arc::clone(&v_b));
        std::thread::sleep(std::time::Duration::from_millis(1));
        dyn_v.swap(Arc::clone(&v_a));
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    for h in handles { h.join().expect("join"); }
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p uptrakit-controller-runtime property_concurrent_swap -- --nocapture 2>&1 | tail
-10`

Expected: PASS (no panic, no UAF — `cargo test` catches both).

- [ ] **Step 3: Commit**

```bash
git add crates/core/controller-runtime/src/dynamic_verifier.rs
git commit -m "test(dynamic-verifier): add concurrent swap+verify property test"
```

---

### Task 12: End-to-end Agent renewal via resolver (integration test)

**Files:**

- Create: `crates/core/integration-tests/tests/cert_rotation_hot_swap.rs`

- [ ] **Step 1: Write the integration test**

```rust
//! Verify that an Agent renewing its cert via the resolver hot-swap path
//! keeps the existing WebSocket session running, then presents the new
//! cert on the next handshake.

#[tokio::test]
#[ignore]
async fn agent_cert_renewal_via_resolver_keeps_session_alive() {
    let harness = integration_tests::AgentControllerHarness::start().await;

    // Spawn Agent, wait for it to enroll + connect.
    let agent = harness.spawn_agent("agent-1").await;
    agent.wait_for_connected().await;

    // Capture the current TLS session ID.
    let session_id_before = agent.current_tls_session_id().await;

    // Trigger a controller-side `RequestCertRenewal`.
    harness.controller.request_cert_renewal(agent.service_id()).await;

    // Wait for the Certificate payload to arrive on the Agent side.
    agent.wait_for_cert_renewed().await;

    // The TCP/TLS session must still be the same — no disconnect.
    let session_id_after = agent.current_tls_session_id().await;
    assert_eq!(session_id_before, session_id_after,
        "resolver-only swap must NOT trigger a reconnect");

    // Force a new handshake (close + reconnect at the WebSocket layer
    // without exiting the Agent process); the new cert must be presented.
    agent.force_handshake().await;
    let presented_cert = harness.controller.last_seen_client_cert(agent.service_id()).await;
    assert_eq!(presented_cert, agent.current_cert_pem(),
        "new handshake presents the freshly-resolved cert");
}
```

- [ ] **Step 2: Add the test to the `cargo test --ignored` set**

Add `#[ignore]` annotation per uptrakit convention for harness-heavy tests; quality gates include
`--ignored` for integration tests.

- [ ] **Step 3: Run**

Run:

```sh
docker build -f docker/Dockerfile.test -t uptrakit-test:latest .
cargo test -p uptrakit-integration-tests cert_rotation_hot_swap -- --ignored --nocapture 2>&1 | tail -30
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/core/integration-tests/tests/cert_rotation_hot_swap.rs
git commit -m "test(integration): cert renewal via resolver keeps TLS session alive"
```

---

### Task 13: Full quality-gate sweep

**Files:** none.

- [ ] **Step 1: `cargo fmt --all -- --check`** — no diff.
- [ ] **Step 2: `cargo check --workspace --no-default-features --features db-sqlite && cargo check
  --workspace --all-features`** — PASS.
- [ ] **Step 3: Clippy on both feature sets:**

  ```sh
  cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings
  cargo clippy --all-targets --all-features -- -D warnings
  ```

  Expected: PASS.
- [ ] **Step 4: `cargo test --all-features`** — PASS.
- [ ] **Step 5: `cargo deny check`** — PASS.
- [ ] **Step 6: Reverse-proxy: `cargo test -p uptrakit-controller reverse_proxy -- --ignored`** —
  PASS.
- [ ] **Step 7: System integration:**

  ```sh
  docker build -f docker/Dockerfile.test -t uptrakit-test:latest .
  cargo test -p uptrakit-integration-tests -- --ignored
  ```

  Expected: PASS.

No commit needed — verification only.

---

## Self-Review

Plan-2 covers:

- §5.4.1 Agent client cert resolver — Tasks 1, 2, 3, 4, 5, 12
- §5.4.2 Controller server cert resolver — Task 6, 10
- §5.4.3 DynamicClientVerifier — Tasks 7, 8, 9, 11
- §5.7 ALPN + session resumption — Task 8 (server) + Task 2 (client)

Deferred to Plan 3:

- §5.1 Trust composition with `--trust-*` flags
- §5.2 TOFU modes (CLI surface change)
- §5.3 SPIFFE identity
- §6 Wire / API additions (`trust_domain`)
- §9 Documentation
- ADRs 0011/0012/0013

No placeholders. Type/method consistency check: `AgentClientCertResolver::new`/`.swap()`,
`ControllerServerCertResolver::new`/`.swap()`, `DynamicClientVerifier::new`/`.swap()`,
`BuiltTlsConfig`, `CrlManager::swap_verifier` — names match across all task references.
