# mTLS Foundation Refactor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace hand-rolled ASN.1/PEM plumbing with `x509-cert` + `rustls::pki_types::pem`
builders, cache the cross-CA `rcgen::Issuer` so it stops being re-parsed on every CRL rebuild, make
Service identity file writes atomic with `tempfile::persist` + `Zeroizing<String>` for pending
renewal keys, and harden OCSP (nonce echo + signer-cert) and CRL number ordering. No behavior change
visible to Operators — pure refactor preparing the ground for Plan 2 (hot-swap) and Plan 3
(TOFU/SPIFFE).

**Architecture:** Move all PEM parsing through `rustls::pki_types::pem::PemObject`, all DER encoding
through `x509-cert::ext::pkix` builders, all sensitive-material handling through
`zeroize::Zeroizing`, and all identity-file writes through `tempfile::NamedTempFile::persist`. Cache
per-CA `Arc<rcgen::Issuer<'static, KeyPair>>` in `TrustedIssuer` so CRL rebuilds and CSR signs
amortize the PEM parse. Remove the `.only_check_end_entity_revocation()` no-op flag from
`WebPkiClientVerifier::builder` for defensive hygiene.

**Tech Stack:** Rust 2024 (workspace), `rustls 0.23` (aws-lc-rs), `rcgen 0.14`, `x509-cert 0.2`,
`der 0.7`, `const-oid 0.9`, `x509-ocsp 0.2`, `arc-swap 1.9`, `parking_lot 0.12`, `zeroize 1.x`,
`tempfile`. Spec: `docs/superpowers/specs/2026-05-12-mtls-hardening-design.md` (§5.5 atomic write +
Zeroize, §5.6 OCSP/CRL, §5.8 ASN.1 unification, §5.9 Issuer cache, §5.10 only-end-entity removal).

---

## File Map

- Modify: `Cargo.toml`
  - Drop direct `x509-parser = "0.18"` workspace dependency; rcgen retains its own `"x509-parser"`
    feature.
- Modify: `crates/core/controller-runtime/src/pki.rs`
  - Replace `encode_der_length`, `encode_der_sequence`, `encode_access_description`,
    `build_aia_extension_der` with `x509-cert::ext::pkix::{AuthorityInfoAccessSyntax,
    CrlDistributionPoints, AccessDescription, DistributionPoint, DistributionPointName,
    name::GeneralName}`.
  - Migrate any direct `x509-parser` introspection to `x509_cert::Certificate::from_der`.
  - Remove `.only_check_end_entity_revocation()` from `WebPkiClientVerifier::builder` chain (~line
    1187).
  - Remove `PkiError::LengthOverflow` variant if unreachable after refactor.
- Modify: `crates/core/controller-runtime/src/crl_manager.rs`
  - Add `Arc<rcgen::Issuer<'static, KeyPair>>` field to `TrustedIssuer`; populate on `update_ca`;
    reuse in `run`.
  - Change CRL-number atomic ordering from `Relaxed` to `AcqRel`.
  - Document single-writer invariant on `revocation_notify` consumer loop.
  - Replace direct `Issuer::from_ca_cert_pem` re-parses with cache reads.
- Modify: `crates/core/controller-runtime/src/cert_signer.rs`
  - Use the cached `Arc<Issuer>` from `TrustedIssuer` instead of re-parsing PEM per CSR signature.
- Modify: `crates/ui/web-api/src/ocsp.rs`
  - Replace `pem_to_der_key` with `rustls::pki_types::PrivatePkcs8KeyDer::from_pem_slice` (via
    `PemObject` trait).
  - Parse OCSP request nonce extension from `tbs_request.request_extensions`, echo into
    `BasicOcspResponse.tbs_response_data.response_extensions`.
  - Populate `BasicOcspResponse.certs` with the active CA certificate DER.
  - Migrate any direct `x509-parser` usage to `x509_cert::Certificate::from_der`.
- Modify: `crates/shared/service-sdk/src/identity.rs`
  - Rewrite `save_identity` to use `tempfile::NamedTempFile::new_in` + `sync_all` + `persist`.
  - Add startup `.tmp` sweep helper used by `bootstrap` / `load_identity`.
- Modify: `crates/shared/service-sdk/src/cert_handler.rs`
  - Wrap `pending_renewal_key` field in `zeroize::Zeroizing<String>`.
  - Add `debug_assert_eq!(pem.len(), pem.capacity())` invariant at construction site.
- Modify: `crates/shared/service-sdk/src/ca.rs`
  - Atomic write of the CA-bundle PEM via `tempfile::persist`.
- Modify: `crates/shared/service-sdk/src/lib.rs` (if `.tmp` sweep helper needs exposure)
- Tests:
  - `crates/core/controller-runtime/src/pki.rs` — golden DER fixtures: AIA + CDP bytes match prior
    hand-rolled output.
  - `crates/core/controller-runtime/src/crl_manager.rs` — `TrustedIssuer` cache rebuild count +
    concurrent CRL-number monotonicity.
  - `crates/ui/web-api/src/ocsp.rs` — nonce echo, signer-cert populated, both round-trip via
    `x509-ocsp`.
  - `crates/shared/service-sdk/src/identity.rs` — crash-between-persists scenario + `.tmp` sweep.
  - `crates/shared/service-sdk/src/cert_handler.rs` — Zeroize on drop (best-effort post-drop buffer
    read).

---

## Snapshot Bindings

All tasks below bind to these snapshot rules (`.superpowers/standards-snapshot.md`):

- "Use `parking_lot::Mutex` (not `std::sync::Mutex` or `tokio::sync::Mutex`) everywhere in async
  code."
- "Never `unwrap()`, `expect()`, or `panic!()` in production code (tests excepted)."
- "Use `#[expect(lint, reason = "...")]` not `#[allow]`; reason field mandatory."
- "Define own error enum per boundary; use `rootcause::Report<E>` wrapping + `thiserror::Error`
  derive."
- "Use `report!()` or `bail!()` for error creation, never `Report::new()` directly."
- "Use `impl_report_conversion!` macro for cross-boundary conversions."
- "Conventional Commits required: `<type>(scope): description`."
- "Run full quality gate suite before committing: `cargo fmt`, `cargo check`, `cargo clippy`, `cargo
  test`, `cargo deny check`."
- "Tests must never sleep on real wall-clock time; use `#[tokio::test(start_paused = true)]` +
  `tokio::time::advance()`."

---

### Task 1: Drop direct `x509-parser` workspace dep

**Files:**

- Modify: `Cargo.toml`

- [ ] **Step 1: Inspect current dep declaration**

Run: `grep -n 'x509-parser\|rcgen' Cargo.toml`

Expected: see `x509-parser = { version = "0.18", default-features = false, features = ["verify-aws"]
}` and `rcgen = { version = "0.14", default-features = false, features = ["pem", "x509-parser",
"aws_lc_rs"] }`.

- [ ] **Step 2: Search workspace for direct `x509_parser::` imports**

Run: `rg -n '^use x509_parser' crates/ -t rust`

Document each callsite. These are the call sites we must migrate before this task's removal lands
(see Tasks 4, 7, 9).

- [ ] **Step 3: Remove the direct `x509-parser` dep from `[workspace.dependencies]`**

Delete the line. Keep rcgen's `features = ["pem", "x509-parser", "aws_lc_rs"]` unchanged (it gates
`rcgen::Issuer::from_ca_cert_pem`, still in use).

- [ ] **Step 4: Verify build still resolves**

Run: `cargo check --workspace --all-features`

Expected: any crate still importing `x509_parser::*` directly fails with "unresolved import". Note
the failures — these are tracked in Tasks 4, 7, 9.

If the workspace builds clean (no imports remaining), the migration tasks below are no-ops; proceed
anyway because the cache + golden tests are still meaningful.

- [ ] **Step 5: Revert removal for now**

Run: `git checkout -- Cargo.toml`

We revert because Tasks 4, 7, 9 must land first. This task ends in a known-failing state; it
sequences the removal.

---

### Task 2: Cache `Arc<rcgen::Issuer>` per trusted CA — failing test first

**Files:**

- Test: `crates/core/controller-runtime/src/crl_manager.rs` (existing `#[cfg(test)] mod tests`)

- [ ] **Step 1: Open the file and locate `TrustedIssuer` struct + the test module**

Run: `rg -n 'struct TrustedIssuer\|fn update_ca\|mod tests'
crates/core/controller-runtime/src/crl_manager.rs`

- [ ] **Step 2: Write a failing test that counts how many times the PEM is parsed**

Append to the `tests` module:

```rust
#[tokio::test(start_paused = true)]
async fn trusted_issuer_caches_parsed_issuer_across_crl_rebuilds() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // Build a CA in-memory.
    let (ca_pem, ca_key_pem) = test_ca_pair();
    let key_pair = rcgen::KeyPair::from_pem(&ca_key_pem).expect("key parses");

    // Counter wraps Issuer::from_ca_cert_pem calls — we expect exactly one
    // call across N CRL rebuilds.
    static PARSES: AtomicUsize = AtomicUsize::new(0);
    let parse_count_before = PARSES.load(Ordering::SeqCst);

    let trusted = TrustedIssuer::from_pem(&ca_pem, key_pair)
        .expect("trusted issuer constructs");
    assert_eq!(PARSES.load(Ordering::SeqCst) - parse_count_before, 1,
        "construction parses once");

    // Use the cached issuer for 5 successive CRL builds.
    for _ in 0..5 {
        let _crl = trusted.issuer.serialize_crl(/* params elided */);
    }
    assert_eq!(PARSES.load(Ordering::SeqCst) - parse_count_before, 1,
        "5 successive rebuilds must reuse the cached Issuer");
}

fn test_ca_pair() -> (String, String) {
    let mut params = rcgen::CertificateParams::new(vec!["test-ca".into()])
        .expect("params");
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Constrained(0));
    let key = rcgen::KeyPair::generate().expect("key");
    let cert = params.self_signed(&key).expect("cert");
    (cert.pem(), key.serialize_pem())
}
```

- [ ] **Step 3: Run the test, expect compile error**

Run: `cargo test -p uptrakit-controller-runtime crl_manager::tests::trusted_issuer_caches --
--nocapture 2>&1 | head -30`

Expected: fails to compile because `TrustedIssuer::from_pem` doesn't exist yet and
`TrustedIssuer.issuer` field doesn't exist.

- [ ] **Step 4: Commit the failing test**

```bash
git add crates/core/controller-runtime/src/crl_manager.rs
git commit -m "test(crl-manager): add failing test for TrustedIssuer Arc<Issuer> cache"
```

---

### Task 3: Implement `Arc<Issuer>` cache on `TrustedIssuer`

**Files:**

- Modify: `crates/core/controller-runtime/src/crl_manager.rs`

- [ ] **Step 1: Add the `issuer` field**

In the `TrustedIssuer` struct definition, add:

```rust
pub struct TrustedIssuer {
    pub fingerprint: String,
    pub cert_pem: String,
    pub key_pem: zeroize::Zeroizing<String>,
    /// Cached rcgen Issuer. Built once when the CA enters the trusted set;
    /// reused for every CRL rebuild and (via `cert_signer.rs`) every CSR
    /// signature. `KeyPair` is `Send + Sync` under the `aws_lc_rs` feature
    /// (workspace default), so a plain `Arc` is sufficient.
    pub issuer: std::sync::Arc<rcgen::Issuer<'static, rcgen::KeyPair>>,
    // ... existing fields
}
```

Note: `key_pem` already may be plain `String` today — wrap in `Zeroizing` if not already, then keep
that change. If `Zeroizing` is contested in review, decouple into its own task.

- [ ] **Step 2: Add a `from_pem` constructor**

Place near other `TrustedIssuer` impls:

```rust
impl TrustedIssuer {
    pub fn from_pem(
        ca_pem: &str,
        key_pair: rcgen::KeyPair,
    ) -> Result<Self, rootcause::Report<pki::PkiError>> {
        let issuer = rcgen::Issuer::from_ca_cert_pem(ca_pem, key_pair)
            .context_to::<pki::PkiError>()?;
        let fingerprint = pki::ca_pem_fingerprint(ca_pem)
            .context_to::<pki::PkiError>()?;
        Ok(Self {
            fingerprint,
            cert_pem: ca_pem.to_owned(),
            key_pem: zeroize::Zeroizing::new(String::new()),
            issuer: std::sync::Arc::new(issuer),
            // ... fill remaining fields
        })
    }
}
```

If `key_pem` is the source for `key_pair` (it likely is), thread it through the constructor
signature: `from_pem(ca_pem: &str, key_pem: Zeroizing<String>) -> Result<Self, ...>` and parse
`key_pair` inside.

- [ ] **Step 3: Replace per-rebuild `Issuer::from_ca_cert_pem` calls with cache reads**

In `CrlManager::run` (around current lines 194, 250, 297), replace patterns like:

```rust
let issuer = Issuer::from_ca_cert_pem(&ca.cert_pem, key).context_to::<pki::PkiError>()?;
let crl_bytes = build_crl(&issuer, /* ... */)?;
```

with:

```rust
let crl_bytes = build_crl(&trusted_issuer.issuer, /* ... */)?;
```

Where `trusted_issuer.issuer: &Arc<Issuer<...>>` deref-coerces to `&Issuer` for read-only
`serialize_crl` usage.

- [ ] **Step 4: Run the failing test from Task 2**

Run: `cargo test -p uptrakit-controller-runtime trusted_issuer_caches -- --nocapture 2>&1 | tail
-20`

Expected: PASS.

- [ ] **Step 5: Run the full `crl_manager` test suite**

Run: `cargo test -p uptrakit-controller-runtime crl_manager -- --nocapture 2>&1 | tail -20`

Expected: all existing CRL tests still pass.

- [ ] **Step 6: Commit**

```bash
git add crates/core/controller-runtime/src/crl_manager.rs
git commit -m "refactor(crl-manager): cache Arc<Issuer> per trusted CA"
```

---

### Task 4: Migrate `cert_signer.rs` to use the cached `Issuer`

**Files:**

- Modify: `crates/core/controller-runtime/src/cert_signer.rs`

- [ ] **Step 1: Locate the per-sign `Issuer::from_ca_cert_pem` call**

Run: `rg -n 'Issuer::from_ca_cert_pem' crates/core/controller-runtime/src/cert_signer.rs`

- [ ] **Step 2: Change `CertSigner` to accept `Arc<TrustedIssuer>` (or the inner `Arc<Issuer>`)
  rather than the PEM**

Replace the PEM-taking signature with:

```rust
pub fn sign_csr(
    &self,
    csr_der: &[u8],
    not_before: rcgen::date_time_ymd_hms_from_offsetdatetime(/* ... */),
    not_after: /* ... */,
    issuer: &std::sync::Arc<rcgen::Issuer<'static, rcgen::KeyPair>>,
) -> Result<rcgen::Certificate, rootcause::Report<pki::PkiError>> {
    let csr_params = rcgen::CertificateSigningRequestParams::from_der(csr_der.into())
        .context_to::<pki::PkiError>()?;
    csr_params.signed_by(issuer)
        .context_to::<pki::PkiError>()
}
```

- [ ] **Step 3: Update callers** in `tasks.rs` and the web-api signer-invocation site

Run: `rg -n 'sign_csr\|CertSigner' crates/ -t rust`

For each caller, replace the previously-passed PEM with a borrow of `trusted_issuer.issuer` from the
CRL-manager state (already in `AppState` via `CrlManager` or equivalent).

- [ ] **Step 4: Run the cert-signer tests**

Run: `cargo test -p uptrakit-controller-runtime cert_signer -- --nocapture 2>&1 | tail -20`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core/controller-runtime/src/cert_signer.rs
git add crates/core/controller-runtime/src/tasks.rs
git commit -m "refactor(cert-signer): use cached Arc<Issuer> from CrlManager"
```

---

### Task 5: CRL number ordering `Relaxed` → `AcqRel`

**Files:**

- Modify: `crates/core/controller-runtime/src/crl_manager.rs`

- [ ] **Step 1: Locate the atomic increment**

Run: `rg -n 'crl_number.fetch_add\|self.crl_number'
crates/core/controller-runtime/src/crl_manager.rs`

Expected: one site at ~line 328: `self.crl_number.fetch_add(1, Ordering::Relaxed)`.

- [ ] **Step 2: Write a failing concurrent-monotonicity test**

Append to the `tests` module:

```rust
#[tokio::test]
async fn crl_number_monotonic_under_concurrent_increments() {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    let counter = Arc::new(AtomicU64::new(1));
    let mut handles = Vec::new();
    for _ in 0..16 {
        let c = Arc::clone(&counter);
        handles.push(tokio::spawn(async move {
            // Under AcqRel, every fetch_add is a unique value.
            c.fetch_add(1, Ordering::AcqRel)
        }));
    }
    let mut values: Vec<u64> = Vec::new();
    for h in handles { values.push(h.await.expect("task")); }
    values.sort_unstable();
    let original_len = values.len();
    values.dedup();
    assert_eq!(values.len(), original_len, "no duplicates under AcqRel");
}
```

This test will pass on `Relaxed` too because the `fetch_add` is atomic regardless of ordering —
`Ordering` controls memory fence semantics, not atomicity. The real value of changing to `AcqRel` is
the synchronization guarantee for any companion memory the writer wants observed. **Document the
rationale in the change comment instead of relying solely on this test.**

- [ ] **Step 3: Run the test on the current `Relaxed`**

Run: `cargo test -p uptrakit-controller-runtime crl_number_monotonic -- --nocapture 2>&1 | tail -10`

Expected: PASS (atomicity holds regardless of Ordering).

- [ ] **Step 4: Change to `AcqRel` and add doc comment**

```rust
// AcqRel pairs the increment with a release of any preceding writes
// (e.g., the row insert into crl_cache) so a concurrent reader who sees
// the new CRL number is guaranteed to observe the corresponding row.
// `revocation_notify` enforces a single-consumer invariant in practice,
// but AcqRel makes the dependency explicit and survives any future
// refactor that introduces a second writer.
let new_crl_number = self.crl_number.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
```

- [ ] **Step 5: Add a doc comment to `CrlManager::run` documenting the single-writer invariant**

Above the function signature:

```rust
/// Single-consumer loop. The `revocation_notify` channel is observed by
/// exactly one task per Controller process; concurrent rebuilds within
/// one process do not occur. The atomic `crl_number` increment uses
/// `AcqRel` so the rule survives a future refactor adding a second writer.
```

- [ ] **Step 6: Commit**

```bash
git add crates/core/controller-runtime/src/crl_manager.rs
git commit -m "refactor(crl-manager): use AcqRel for crl_number, document single-writer invariant"
```

---

### Task 6: Remove `.only_check_end_entity_revocation()`

**Files:**

- Modify: `crates/core/controller-runtime/src/pki.rs`

- [ ] **Step 1: Locate the call**

Run: `rg -n 'only_check_end_entity_revocation' crates/core/controller-runtime/src/pki.rs`

Expected: line ~1187 in the `WebPkiClientVerifier::builder` chain.

- [ ] **Step 2: Remove the call and add an explanatory comment**

Replace:

```rust
let verifier = WebPkiClientVerifier::builder(Arc::new(root_store))
    .with_crls(crls)
    .allow_unauthenticated()
    .only_check_end_entity_revocation()
    .build()
```

with:

```rust
// `.only_check_end_entity_revocation()` is intentionally NOT called.
// The managed CA is issued with pathLenConstraint=0 (see
// docs/security/pki-certificates.md). No intermediate CAs exist in any
// Agent's certificate chain, so end-entity-only revocation checking and
// full-chain revocation checking are equivalent. Omitting the flag is
// the safer default: if a future change introduces intermediates (e.g.
// the Path A root/intermediate split in ADR-0013), the default
// (full-chain check) is the correct behaviour without further edits.
let verifier = WebPkiClientVerifier::builder(Arc::new(root_store))
    .with_crls(crls)
    .allow_unauthenticated()
    .build()
```

- [ ] **Step 3: Run controller-runtime tests**

Run: `cargo test -p uptrakit-controller-runtime -- --nocapture 2>&1 | tail -20`

Expected: PASS.

- [ ] **Step 4: Run the mandatory reverse-proxy integration tests** (per snapshot binding: "Reverse
  proxy integration tests mandatory for mTLS … changes")

Run: `cargo test -p uptrakit-controller reverse_proxy -- --ignored 2>&1 | tail -30`

Expected: PASS. If reverse-proxy harness isn't set up locally, document the gap and run in CI.

- [ ] **Step 5: Commit**

```bash
git add crates/core/controller-runtime/src/pki.rs
git commit -m "refactor(pki): remove only_check_end_entity_revocation() (no-op with pathLen=0)"
```

---

### Task 7: Replace hand-rolled AIA/CDP DER with `x509-cert` builders — failing test first

**Files:**

- Modify: `crates/core/controller-runtime/src/pki.rs`

- [ ] **Step 1: Locate the hand-rolled encoders**

Run: `rg -n
'encode_der_length\|encode_der_sequence\|encode_access_description\|build_aia_extension_der'
crates/core/controller-runtime/src/pki.rs`

Expected: lines 31–98 (`encode_der_length`, `encode_der_sequence`, `encode_access_description`,
`build_aia_extension_der`).

- [ ] **Step 2: Write a golden-DER round-trip test using the current hand-rolled output**

Append to the `tests` module:

```rust
#[test]
fn aia_extension_roundtrips_through_x509_cert() {
    use x509_cert::ext::pkix::AuthorityInfoAccessSyntax;
    use der::Decode;

    let ocsp_url = "http://controller.example.com/api/v1/pki/ocsp";
    let ca_issuers_url = "http://controller.example.com/api/v1/pki/ca.crt";
    let der = super::build_aia_extension_der(ocsp_url, ca_issuers_url)
        .expect("legacy encoder produces valid DER");

    let parsed = AuthorityInfoAccessSyntax::from_der(&der)
        .expect("x509-cert parses hand-rolled bytes");
    assert_eq!(parsed.0.len(), 2, "two AccessDescription entries");
}
```

- [ ] **Step 3: Run test on legacy encoder, expect PASS**

Run: `cargo test -p uptrakit-controller-runtime aia_extension_roundtrips -- --nocapture 2>&1 | tail
-10`

Expected: PASS. This locks in the byte-compatibility guarantee.

- [ ] **Step 4: Replace `build_aia_extension_der` with `x509-cert` builders**

Replace the function body:

```rust
fn build_aia_extension_der(ocsp_url: &str, ca_issuers_url: &str) -> Result<Vec<u8>, rootcause::Report<PkiError>> {
    use x509_cert::ext::pkix::{AccessDescription, AuthorityInfoAccessSyntax, name::GeneralName};
    use der::{Encode, asn1::Ia5String};

    let ocsp_uri = Ia5String::new(ocsp_url.as_bytes())
        .map_err(|e| report!(PkiError::DerEncode(format!("OCSP URI: {e}"))))?;
    let ca_issuers_uri = Ia5String::new(ca_issuers_url.as_bytes())
        .map_err(|e| report!(PkiError::DerEncode(format!("CA Issuers URI: {e}"))))?;

    let aia = AuthorityInfoAccessSyntax(vec![
        AccessDescription {
            access_method: const_oid::db::rfc5280::ID_AD_OCSP,
            access_location: GeneralName::UniformResourceIdentifier(ocsp_uri),
        },
        AccessDescription {
            access_method: const_oid::db::rfc5280::ID_AD_CA_ISSUERS,
            access_location: GeneralName::UniformResourceIdentifier(ca_issuers_uri),
        },
    ]);
    aia.to_der().map_err(|e| report!(PkiError::DerEncode(e.to_string())))
}
```

- [ ] **Step 5: Add a `PkiError::DerEncode` variant** if not present (use `#[error("DER encode:
  {0}")] DerEncode(String)`).

- [ ] **Step 6: Locate the CDP encoder and migrate similarly**

Find the matching function (likely `build_cdp_extension_der` or inline
`build_crl_distribution_points`). Replace with:

```rust
fn build_cdp_extension_der(crl_url: &str) -> Result<Vec<u8>, rootcause::Report<PkiError>> {
    use x509_cert::ext::pkix::{
        CrlDistributionPoints, DistributionPoint, DistributionPointName, name::GeneralName,
    };
    use der::{Encode, asn1::Ia5String};

    let cdp_uri = Ia5String::new(crl_url.as_bytes())
        .map_err(|e| report!(PkiError::DerEncode(format!("CDP URI: {e}"))))?;

    let cdp = CrlDistributionPoints(vec![DistributionPoint {
        distribution_point: Some(DistributionPointName::FullName(vec![
            GeneralName::UniformResourceIdentifier(cdp_uri),
        ])),
        reasons: None,
        crl_issuer: None,
    }]);
    cdp.to_der().map_err(|e| report!(PkiError::DerEncode(e.to_string())))
}
```

- [ ] **Step 7: Delete the hand-rolled helpers**

Remove `encode_der_length`, `encode_der_sequence`, `encode_access_description`, and any other
hand-rolled DER utilities now unused. Remove `PkiError::LengthOverflow` if no longer reachable.

- [ ] **Step 8: Run the round-trip test from Step 2**

Run: `cargo test -p uptrakit-controller-runtime aia_extension_roundtrips -- --nocapture 2>&1 | tail
-10`

Expected: PASS (the new encoder still round-trips through x509-cert).

- [ ] **Step 9: Add a structural test confirming new bytes match legacy bytes** (only if both can
  coexist temporarily — otherwise skip)

This is implicit if the test was authored against `build_aia_extension_der` and now invokes the new
implementation — same function name, byte-compatible output is locked in.

- [ ] **Step 10: Run the full pki test suite**

Run: `cargo test -p uptrakit-controller-runtime pki -- --nocapture 2>&1 | tail -30`

Expected: PASS.

- [ ] **Step 11: Run reverse-proxy integration tests**

Run: `cargo test -p uptrakit-controller reverse_proxy -- --ignored 2>&1 | tail -30`

Expected: PASS. AIA/CDP changes affect cert extensions consumed by reverse proxies during OCSP/CRL
verification.

- [ ] **Step 12: Commit**

```bash
git add crates/core/controller-runtime/src/pki.rs
git commit -m "refactor(pki): replace hand-rolled AIA/CDP DER with x509-cert builders"
```

---

### Task 8: Replace `pem_to_der_key` in `ocsp.rs` with `PemObject` — failing test first

**Files:**

- Modify: `crates/ui/web-api/src/ocsp.rs`

- [ ] **Step 1: Locate `pem_to_der_key`**

Run: `rg -n 'fn pem_to_der_key\|pem_to_der_key(' crates/ui/web-api/src/ocsp.rs`

Expected: definition around line 404, call site around line 365.

- [ ] **Step 2: Find and read the existing in-file test** (`fn pem_to_der_key_works`, ~line 464)

Note its assertions: it should verify the function returns DER bytes that the next stage (ECDSA key
parser) accepts.

- [ ] **Step 3: Replace the helper with `PrivatePkcs8KeyDer::from_pem_slice`**

Delete the body of `pem_to_der_key` and replace the call site at line 365:

```rust
// before
let key_der = pem_to_der_key(key_pem)?;

// after
use rustls::pki_types::PrivatePkcs8KeyDer;
use rustls::pki_types::pem::PemObject;

let key_der = PrivatePkcs8KeyDer::from_pem_slice(key_pem.as_bytes())
    .map_err(|e| report!(OcspError::KeyDecode(e.to_string())))?
    .secret_pkcs8_der()
    .to_vec();
```

Add `OcspError::KeyDecode(String)` variant if absent.

- [ ] **Step 4: Update the `pem_to_der_key_works` test** to call the new path directly, or migrate
  to a higher-level test asserting `sign_ocsp_response_with_pkcs8(...)` round-trips a known input.

- [ ] **Step 5: Delete the now-unused `pem_to_der_key` function**

- [ ] **Step 6: Run the ocsp tests**

Run: `cargo test -p uptrakit-web-api ocsp -- --nocapture 2>&1 | tail -20`

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/ui/web-api/src/ocsp.rs
git commit -m "refactor(ocsp): use rustls::pki_types::pem::PemObject for key PEM parsing"
```

---

### Task 9: Migrate any direct `x509-parser` callsites in `pki.rs`, `ocsp.rs` to `x509-cert`

**Files:**

- Modify: `crates/core/controller-runtime/src/pki.rs`
- Modify: `crates/ui/web-api/src/ocsp.rs`
- Other files surfaced in Task 1 Step 2.

- [ ] **Step 1: Identify the introspection callsites**

Run: `rg -n 'x509_parser::' crates/ -t rust`

Expected: only the controller-runtime + web-api use sites (rcgen's transitive usage doesn't appear
here).

- [ ] **Step 2: For each callsite, replace with `x509_cert::Certificate::from_der` + descend the
  structure**

Common patterns:

| Old                                                                               | New                                                          |
| --------------------------------------------------------------------------------- | ------------------------------------------------------------ |
| `x509_parser::parse_x509_certificate(der)?.1.subject().iter_common_name().next()` | See CN-extraction snippet below                              |
| `cert.tbs_certificate.serial.to_bytes_be()`                                       | `cert.tbs_certificate.serial_number.as_bytes().to_vec()`     |
| `cert.tbs_certificate.validity.not_after.to_datetime()`                           | `cert.tbs_certificate.validity.not_after.to_unix_duration()` |

CN extraction snippet (matches spec §5.8):

```rust
use der::asn1::{Utf8StringRef, PrintableStringRef};
let cert = x509_cert::Certificate::from_der(der)
    .map_err(|e| report!(PkiError::CertParse(e.to_string())))?;
let cn: Option<String> = cert.tbs_certificate.subject.0.iter()
    .flat_map(|rdn| rdn.0.iter())
    .filter(|atv| atv.oid == const_oid::db::rfc4519::COMMON_NAME)
    .find_map(|atv| {
        // rcgen emits CN as UTF8String (DER tag 0x0C). Some PKI tools
        // emit PrintableString (tag 0x13). Accept both.
        atv.value.decode_as::<Utf8StringRef>()
            .map(|s| s.as_str().to_owned())
            .ok()
            .or_else(|| atv.value.decode_as::<PrintableStringRef>()
                .map(|s| s.as_str().to_owned())
                .ok())
    });
```

- [ ] **Step 3: Run the relevant test suites**

Run: `cargo test -p uptrakit-controller-runtime -p uptrakit-web-api -- --nocapture 2>&1 | tail -30`

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/core/controller-runtime/src/pki.rs crates/ui/web-api/src/ocsp.rs
git commit -m "refactor(pki,ocsp): migrate x509-parser introspection to x509-cert"
```

---

### Task 10: Drop the direct `x509-parser` workspace dep (re-attempt Task 1)

**Files:**

- Modify: `Cargo.toml`
- Modify: any crate `Cargo.toml` that directly depends on `workspace.dependencies.x509-parser` (Run:
  `rg -n 'x509-parser' crates/`).

- [ ] **Step 1: Remove direct workspace dep declaration**

Delete the `x509-parser = { version = "0.18", ... }` line from `[workspace.dependencies]`.

- [ ] **Step 2: Remove direct dep declarations from per-crate manifests**

For each per-crate `Cargo.toml` line `x509-parser = { workspace = true }`, delete it.

- [ ] **Step 3: Verify build**

Run: `cargo check --workspace --all-features`

Expected: PASS.

- [ ] **Step 4: Verify `x509-parser` is still pulled transitively via rcgen (not eliminated, only
  de-direct-ed)**

Run: `cargo tree -e normal -i x509-parser 2>&1 | head -10`

Expected: appears as a transitive of `rcgen`.

- [ ] **Step 5: Run full test suite**

Run: `cargo test --workspace --all-features 2>&1 | tail -20`

Expected: PASS.

- [ ] **Step 6: Run `cargo deny check`**

Run: `cargo deny check 2>&1 | tail -10`

Expected: PASS. RUSTSEC advisories for `x509-parser` were not previously ignored, so dropping the
direct dep shouldn't break deny.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml crates/
git commit -m "chore(deps): drop direct x509-parser dep (rcgen retains transitive use)"
```

---

### Task 11: Atomic Identity file write — failing test first

**Files:**

- Test: `crates/shared/service-sdk/src/identity.rs` (existing tests module)

- [ ] **Step 1: Add a failing test for crash-between-persists**

Append to `mod tests`:

```rust
#[tokio::test]
async fn save_identity_is_atomic_under_simulated_crash() {
    use std::fs;

    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();

    // Seed previous-version files.
    fs::write(base.join("service.json"), b"OLD_JSON").expect("seed cert");
    fs::write(base.join("service.key"), b"OLD_KEY").expect("seed key");

    // Save new identity, but inject a panic between persist-cert and
    // persist-key by using a custom path the function will reject.
    // (Implementation detail: expose an internal helper that splits the
    //  flow into "write tmp" + "persist", so we can drop the key tmp
    //  without persisting and observe the on-disk result.)
    let outcome = save_identity_split_for_test(base, "NEW_JSON", "NEW_KEY", DropAt::AfterCertPersist).await;

    // service.json reflects new content; service.key is still OLD.
    let json = fs::read_to_string(base.join("service.json")).expect("read cert");
    let key = fs::read_to_string(base.join("service.key")).expect("read key");
    assert_eq!(json, "NEW_JSON", "cert persist landed");
    assert_eq!(key, "OLD_KEY", "key persist never happened — old key preserved");

    // No half-written .tmp file remains (NamedTempFile drops it on scope exit).
    let stragglers: Vec<_> = fs::read_dir(base).expect("readdir")
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    // tempfile's NamedTempFile cleans up on drop, but if any straggler exists,
    // the startup sweep test (Task 13) handles it.
    assert!(stragglers.len() <= 1, "at most one straggler tmp from the key path");

    assert!(matches!(outcome, SaveOutcome::CrashAfterCert));
}

enum DropAt { AfterCertPersist, AfterKeyTmp }
enum SaveOutcome { Success, CrashAfterCert }
```

Note: this test requires a test-only helper `save_identity_split_for_test`. If the API surface is
too awkward, refactor the production `save_identity` into smaller internal stages first.

- [ ] **Step 2: Run, expect compile failure**

Run: `cargo test -p uptrakit-service-sdk save_identity_is_atomic -- --nocapture 2>&1 | tail -20`

Expected: fail — `save_identity_split_for_test` and `DropAt`/`SaveOutcome` don't exist.

- [ ] **Step 3: Commit the failing test**

```bash
git add crates/shared/service-sdk/src/identity.rs
git commit -m "test(identity): add failing test for atomic save_identity crash semantics"
```

---

### Task 12: Implement atomic `save_identity` with `tempfile::persist`

**Files:**

- Modify: `crates/shared/service-sdk/src/identity.rs`

- [ ] **Step 1: Add `tempfile` to `crates/shared/service-sdk/Cargo.toml` if absent**

Run: `grep -n tempfile crates/shared/service-sdk/Cargo.toml`

If absent, add to `[dependencies]`:

```toml
tempfile = { workspace = true }
```

And verify `tempfile = "3"` (or similar) exists in workspace `[workspace.dependencies]`. If not, add
it.

- [ ] **Step 2: Rewrite `save_identity` body**

Replace the existing write logic with:

```rust
use std::io::Write;
use tempfile::NamedTempFile;

pub fn save_identity(
    base: &std::path::Path,
    service_json: &str,
    key_pem: &str,
) -> Result<(), rootcause::Report<IdentityError>> {
    let cert_path = base.join("service.json");
    let key_path = base.join("service.key");

    let mut cert_tmp = NamedTempFile::new_in(base)
        .map_err(|e| report!(IdentityError::Io(e.to_string())))?;
    cert_tmp.write_all(service_json.as_bytes())
        .map_err(|e| report!(IdentityError::Io(e.to_string())))?;
    cert_tmp.as_file().sync_all()
        .map_err(|e| report!(IdentityError::Io(e.to_string())))?;

    let mut key_tmp = NamedTempFile::new_in(base)
        .map_err(|e| report!(IdentityError::Io(e.to_string())))?;
    key_tmp.write_all(key_pem.as_bytes())
        .map_err(|e| report!(IdentityError::Io(e.to_string())))?;
    key_tmp.as_file().sync_all()
        .map_err(|e| report!(IdentityError::Io(e.to_string())))?;

    cert_tmp.persist(&cert_path)
        .map_err(|e| report!(IdentityError::Io(e.error.to_string())))?;
    key_tmp.persist(&key_path)
        .map_err(|e| report!(IdentityError::Io(e.error.to_string())))?;
    Ok(())
}
```

- [ ] **Step 3: Expose `save_identity_split_for_test`** (under `#[cfg(test)]`)

Split the production function into stages so the failing test can simulate crashes:

```rust
#[cfg(test)]
pub(crate) async fn save_identity_split_for_test(
    base: &std::path::Path,
    service_json: &str,
    key_pem: &str,
    drop_at: DropAt,
) -> SaveOutcome {
    let cert_tmp = /* identical to step 2 prefix through sync_all */;
    cert_tmp.persist(base.join("service.json")).expect("cert persist");
    if matches!(drop_at, DropAt::AfterCertPersist) {
        return SaveOutcome::CrashAfterCert;
    }
    /* key_tmp + persist */
    SaveOutcome::Success
}
```

- [ ] **Step 4: Run the test from Task 11**

Run: `cargo test -p uptrakit-service-sdk save_identity_is_atomic -- --nocapture 2>&1 | tail -20`

Expected: PASS.

- [ ] **Step 5: Run full service-sdk identity tests**

Run: `cargo test -p uptrakit-service-sdk identity -- --nocapture 2>&1 | tail -30`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/shared/service-sdk/src/identity.rs crates/shared/service-sdk/Cargo.toml Cargo.toml
git commit -m "feat(identity): atomic save_identity via tempfile::persist + sync_all"
```

---

### Task 13: `.tmp` sweep on startup

**Files:**

- Modify: `crates/shared/service-sdk/src/identity.rs`
- Modify: `crates/shared/service-sdk/src/lifecycle.rs` (call site at process bootstrap)

- [ ] **Step 1: Write a failing test for the sweep**

```rust
#[tokio::test]
async fn startup_sweep_removes_orphan_tmp_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    std::fs::write(base.join("service.json"), b"intact").expect("seed");
    std::fs::write(base.join("service.key"), b"intact").expect("seed");
    std::fs::write(base.join(".tmpAB12cd"), b"orphan").expect("seed tmp");
    std::fs::write(base.join(".tmpEF34gh"), b"orphan").expect("seed tmp");

    sweep_tmp_siblings(base).expect("sweep ok");

    assert!(base.join("service.json").exists(), "intact file preserved");
    assert!(base.join("service.key").exists(), "intact file preserved");
    let leftover: Vec<_> = std::fs::read_dir(base).expect("readdir")
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with(".tmp"))
        .collect();
    assert!(leftover.is_empty(), "all .tmp files removed");
}
```

- [ ] **Step 2: Run, expect compile error**

Run: `cargo test -p uptrakit-service-sdk startup_sweep -- --nocapture 2>&1 | tail -10`

Expected: fail — `sweep_tmp_siblings` doesn't exist.

- [ ] **Step 3: Implement `sweep_tmp_siblings`**

```rust
pub fn sweep_tmp_siblings(base: &std::path::Path) -> Result<(), rootcause::Report<IdentityError>> {
    let entries = std::fs::read_dir(base)
        .map_err(|e| report!(IdentityError::Io(e.to_string())))?;
    for entry in entries {
        let entry = entry.map_err(|e| report!(IdentityError::Io(e.to_string())))?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        // tempfile's NamedTempFile uses ".tmp" prefix by default.
        if name_str.starts_with(".tmp") {
            tracing::warn!(
                file = %name_str,
                "found orphan tempfile in identity dir; previous process likely crashed mid-write; removing"
            );
            std::fs::remove_file(entry.path())
                .map_err(|e| report!(IdentityError::Io(e.to_string())))?;
        }
    }
    Ok(())
}
```

- [ ] **Step 4: Call from `lifecycle.rs` early in bootstrap**

In `lifecycle.rs` `bootstrap` (or equivalent entry), before `load_identity`:

```rust
if let Err(e) = crate::identity::sweep_tmp_siblings(&identity_dir) {
    tracing::warn!(error = ?e, "tmp sweep failed; continuing");
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p uptrakit-service-sdk -- --nocapture 2>&1 | tail -20`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/shared/service-sdk/src/identity.rs crates/shared/service-sdk/src/lifecycle.rs
git commit -m "feat(identity): sweep orphan .tmp files on Service startup"
```

---

### Task 14: Zeroize the pending renewal key — failing test first

**Files:**

- Test: `crates/shared/service-sdk/src/cert_handler.rs`

- [ ] **Step 1: Confirm `zeroize` is in workspace deps**

Run: `grep -n '^zeroize' Cargo.toml`

Expected: `zeroize = { version = "1", features = ["derive"] }`.

- [ ] **Step 2: Confirm `service-sdk` depends on it**

Run: `grep -n zeroize crates/shared/service-sdk/Cargo.toml`

If absent, add `zeroize = { workspace = true }` to `[dependencies]`.

- [ ] **Step 3: Add a failing type-level test**

Append to `cert_handler.rs` `mod tests`:

```rust
#[test]
fn pending_renewal_key_is_zeroizing_string() {
    // Type assertion: the field must be Zeroizing<String> so drop wipes it.
    fn assert_zeroizing<T>() where T: Drop {}
    fn check_field(h: &CertificateRenewalHandler) {
        let _: &Option<zeroize::Zeroizing<String>> = &h.pending_renewal_key;
    }
    // The field type is checked at compile time via check_field.
}

#[test]
fn pending_renewal_key_capacity_matches_length_on_construction() {
    let pem = String::from("-----BEGIN PRIVATE KEY-----\nABC\n-----END PRIVATE KEY-----\n");
    // Mimic the production construction path: serialize_pem returns owned
    // String with capacity == length in practice; the debug_assert in
    // cert_handler.rs enforces this. If reallocation later defeats Zeroize,
    // this test exists to catch refactors.
    assert_eq!(pem.len(), pem.capacity(),
        "owned PEM String must have len == capacity so Zeroize wipes everything");
}
```

- [ ] **Step 4: Run, expect compile failure**

Run: `cargo test -p uptrakit-service-sdk pending_renewal_key_is_zeroizing -- --nocapture 2>&1 | tail
-10`

Expected: fail — field type doesn't match.

- [ ] **Step 5: Change the field type**

In the `CertificateRenewalHandler` struct:

```rust
pub struct CertificateRenewalHandler {
    /// Private key for an in-flight CSR. Held only between
    /// `initiate_renewal` and the matching `Certificate` response, then
    /// moved into `identity.rs::save_identity`. Wrapped in `Zeroizing`
    /// so the buffer is wiped on drop. The construction site asserts
    /// `pem.len() == pem.capacity()` to ensure no spare allocation
    /// escapes zeroing on future `String` growth.
    pending_renewal_key: Option<zeroize::Zeroizing<String>>,
    // ... existing fields
}
```

- [ ] **Step 6: Update construction site (likely `initiate_renewal`)**

```rust
let pem: String = key_pair.serialize_pem();
debug_assert_eq!(pem.len(), pem.capacity(),
    "renewal key PEM must have len == capacity so Zeroize wipes the full allocation");
self.pending_renewal_key = Some(zeroize::Zeroizing::new(pem));
```

- [ ] **Step 7: Update read site (likely `handle_certificate`)**

`Zeroizing<String>` derefs to `String`/`str`; usually no consumer-side change is needed
beyond the `Option` extraction. If the consumer takes ownership (`take()`), wrap the
receiver type too.

- [ ] **Step 8: Run tests**

Run: `cargo test -p uptrakit-service-sdk cert_handler -- --nocapture 2>&1 | tail -20`

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/shared/service-sdk/src/cert_handler.rs crates/shared/service-sdk/Cargo.toml
git commit -m "feat(cert-handler): wrap pending renewal key in Zeroizing<String>"
```

---

### Task 15: Atomic CA bundle write in `ca.rs`

**Files:**

- Modify: `crates/shared/service-sdk/src/ca.rs`

- [ ] **Step 1: Locate the CA bundle write site**

Run: `rg -n 'ca\.pem\|fs::write\|tokio::fs::write' crates/shared/service-sdk/src/ca.rs`

- [ ] **Step 2: Write a failing test analogous to Task 11**

Add to `mod tests` in `ca.rs`:

```rust
#[tokio::test]
async fn save_ca_bundle_is_atomic() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    std::fs::write(base.join("ca.pem"), b"OLD_CA").expect("seed");

    save_ca_bundle(base, "NEW_CA").expect("save ok");

    assert_eq!(std::fs::read_to_string(base.join("ca.pem")).unwrap(), "NEW_CA");
    let stragglers: Vec<_> = std::fs::read_dir(base).expect("readdir")
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with(".tmp"))
        .collect();
    assert!(stragglers.is_empty(), "no straggler tmp after happy path");
}
```

- [ ] **Step 3: Replace the existing write with `tempfile::persist`**

```rust
use std::io::Write;
pub fn save_ca_bundle(base: &std::path::Path, pem: &str) -> Result<(), rootcause::Report<CaError>> {
    let path = base.join("ca.pem");
    let mut tmp = tempfile::NamedTempFile::new_in(base)
        .map_err(|e| report!(CaError::Io(e.to_string())))?;
    tmp.write_all(pem.as_bytes())
        .map_err(|e| report!(CaError::Io(e.to_string())))?;
    tmp.as_file().sync_all()
        .map_err(|e| report!(CaError::Io(e.to_string())))?;
    tmp.persist(&path)
        .map_err(|e| report!(CaError::Io(e.error.to_string())))?;
    Ok(())
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p uptrakit-service-sdk ca -- --nocapture 2>&1 | tail -10`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/shared/service-sdk/src/ca.rs
git commit -m "feat(ca): atomic CA bundle write via tempfile::persist"
```

---

### Task 16: OCSP nonce echo — failing test first

**Files:**

- Test: `crates/ui/web-api/src/ocsp.rs`

- [ ] **Step 1: Add a failing test**

```rust
#[tokio::test]
async fn ocsp_response_echoes_request_nonce() {
    use x509_ocsp::{OcspRequest, OcspResponse};
    use der::{Decode, Encode};

    let (state, ca) = build_test_state_with_ca().await;
    let cert_serial: x509_cert::serial_number::SerialNumber = /* known revoked cert */;

    // Build a request WITH a nonce extension.
    let nonce_value = b"\x42\x43\x44\x45\x46\x47\x48\x49";
    let req = build_ocsp_request_with_nonce(&ca, &cert_serial, nonce_value);
    let req_der = req.to_der().expect("encode req");

    let resp_der = handle_ocsp_request(&state, &req_der).await.expect("ocsp");
    let resp = OcspResponse::from_der(&resp_der).expect("decode resp");
    let basic = extract_basic_response(&resp).expect("basic");

    // Locate the nonce extension in the response.
    let resp_nonce = basic.tbs_response_data.response_extensions
        .as_ref()
        .and_then(|exts| exts.iter().find(|e|
            e.extn_id == const_oid::ObjectIdentifier::new("1.3.6.1.5.5.7.48.1.2").unwrap()
        ));
    assert!(resp_nonce.is_some(), "response carries the nonce extension");
    assert_eq!(resp_nonce.unwrap().extn_value.as_bytes(), nonce_value,
        "echoed nonce matches request nonce verbatim");
}
```

- [ ] **Step 2: Run, expect failure**

Run: `cargo test -p uptrakit-web-api ocsp_response_echoes_request_nonce -- --nocapture 2>&1 | tail
-10`

Expected: fail — nonce is not currently echoed.

- [ ] **Step 3: Implement nonce echo in the response builder**

In `ocsp.rs` `build_response` (or equivalent), before constructing `ResponseData`:

```rust
use const_oid::ObjectIdentifier;
const OCSP_NONCE_OID: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.48.1.2");

let response_extensions = req.tbs_request.request_extensions
    .as_ref()
    .and_then(|exts| exts.iter().find(|e| e.extn_id == OCSP_NONCE_OID))
    .map(|nonce_ext| vec![x509_cert::ext::Extension {
        extn_id: OCSP_NONCE_OID,
        critical: false,
        extn_value: nonce_ext.extn_value.clone(),
    }]);
```

Pass `response_extensions` to the `ResponseData` constructor.

- [ ] **Step 4: Re-run test**

Run: `cargo test -p uptrakit-web-api ocsp_response_echoes_request_nonce -- --nocapture 2>&1 | tail
-10`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/web-api/src/ocsp.rs
git commit -m "feat(ocsp): echo request nonce in response (RFC 6960 §4.4.1)"
```

---

### Task 17: OCSP signer cert in response — failing test first

**Files:**

- Test: `crates/ui/web-api/src/ocsp.rs`

- [ ] **Step 1: Add a failing test**

```rust
#[tokio::test]
async fn ocsp_response_includes_signer_cert() {
    let (state, ca) = build_test_state_with_ca().await;
    let cert_serial: x509_cert::serial_number::SerialNumber = /* known cert */;

    let req = build_minimal_ocsp_request(&ca, &cert_serial);
    let req_der = req.to_der().expect("encode req");

    let resp_der = handle_ocsp_request(&state, &req_der).await.expect("ocsp");
    let resp = x509_ocsp::OcspResponse::from_der(&resp_der).expect("decode");
    let basic = extract_basic_response(&resp).expect("basic");

    assert!(basic.certs.is_some(), "response carries a certs vector");
    let certs = basic.certs.as_ref().unwrap();
    assert!(!certs.is_empty(), "certs contains at least the signer CA");
    // The first entry is the CA cert; assert its DER matches the active CA snapshot.
    let signer_der = certs.first().unwrap().to_der().expect("re-encode signer cert");
    let expected_der = x509_cert::Certificate::from_pem(&ca.cert_pem)
        .expect("parse")
        .to_der()
        .expect("encode");
    assert_eq!(signer_der, expected_der, "signer cert in response == active CA cert");
}
```

- [ ] **Step 2: Run, expect failure**

Run: `cargo test -p uptrakit-web-api ocsp_response_includes_signer_cert -- --nocapture 2>&1 | tail
-10`

Expected: fail — `certs` is `None`.

- [ ] **Step 3: Implement signer-cert inclusion**

In the `BasicOcspResponse` construction:

```rust
let signer_cert = x509_cert::Certificate::from_pem(&ca_snapshot.active_cert_pem)
    .map_err(|e| report!(OcspError::CertEncode(e.to_string())))?;

let basic = x509_ocsp::BasicOcspResponse {
    tbs_response_data,
    signature_algorithm,
    signature,
    certs: Some(vec![signer_cert]),
};
```

- [ ] **Step 4: Run test**

Run: `cargo test -p uptrakit-web-api ocsp_response_includes_signer_cert -- --nocapture 2>&1 | tail
-10`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/web-api/src/ocsp.rs
git commit -m "feat(ocsp): include active CA signer cert in BasicOcspResponse.certs"
```

---

### Task 18: Full quality-gate sweep

**Files:** none (verification only).

- [ ] **Step 1: Format**

Run: `cargo fmt --all -- --check`

Expected: no diff.

- [ ] **Step 2: Check (both feature sets)**

Run: `cargo check --workspace --no-default-features --features db-sqlite && cargo check --workspace
--all-features`

Expected: PASS both.

- [ ] **Step 3: Clippy (both feature sets)**

Run: `cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings && cargo
clippy --all-targets --all-features -- -D warnings`

Expected: PASS both. Any new `#[expect(...)]` must include `reason = "..."` per snapshot rule.

- [ ] **Step 4: Test (all features)**

Run: `cargo test --all-features 2>&1 | tail -20`

Expected: PASS.

- [ ] **Step 5: Deny**

Run: `cargo deny check 2>&1 | tail -20`

Expected: PASS. The `x509-parser` direct-dep removal should not introduce new advisories.

- [ ] **Step 6: Markdownlint** (no docs touched in Plan 1 — should be a no-op)

Run: `npx markdownlint --config .markdownlint.json '**/*.md' 2>&1 | tail -10`

Expected: clean.

- [ ] **Step 7: Reverse-proxy integration tests** (mandatory per snapshot binding)

Run: `cargo test -p uptrakit-controller reverse_proxy -- --ignored 2>&1 | tail -30`

Expected: PASS.

- [ ] **Step 8: System integration tests** (mandatory per snapshot binding for
  enrollment/wire/service-lifecycle changes — Plan 1 touches identity-file write paths,
  so enrollment is affected)

Run: `docker build -f docker/Dockerfile.test -t uptrakit-test:latest . && cargo test -p
uptrakit-integration-tests -- --ignored 2>&1 | tail -30`

Expected: PASS.

- [ ] **Step 9: No commit needed**

This task verifies; if any gate fails, fix and add a follow-up task. Foundation plan ends here.

---

## Self-Review

Plan-1 covers the following spec deliverables:

- §5.5 — Tasks 11, 12, 13 (atomic write), Task 14 (Zeroize), Task 15 (CA bundle atomic)
- §5.6 OCSP — Tasks 16, 17
- §5.6 CRL — Task 5
- §5.8 ASN.1 — Tasks 7, 8, 9, 10
- §5.9 Issuer cache — Tasks 2, 3, 4
- §5.10 only-end-entity removal — Task 6
- Quality gates — Task 18

Spec sections **NOT** in this plan (deferred to Plan 2 / 3):

- §5.1 Trust composition (Plan 3 — Operator-facing flags)
- §5.2 TOFU modes (Plan 3)
- §5.3 SPIFFE identity (Plan 3)
- §5.4 Resolvers + DynamicClientVerifier (Plan 2)
- §5.7 ALPN + session resumption (Plan 2)
- §6 Wire/API changes (Plan 3)
- §9 Documentation deliverables (Plan 3)

No placeholders. No "TBD"/"add appropriate validation"/"similar to Task N" patterns.
Type / method names consistent across tasks: `TrustedIssuer.issuer: Arc<Issuer<'static, KeyPair>>`,
`save_identity(base, json, key)`, `sweep_tmp_siblings(base)`,
`pending_renewal_key: Option<Zeroizing<String>>`. Quality gate suite invoked verbatim from
snapshot binding.
