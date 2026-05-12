# mTLS Hardening and Modernization — Design

Status: Draft for review
Author: Andrey Yantsen
Date: 2026-05-12
ADRs:

- [`docs/adr/0011-spiffe-service-identity.md`](../../adr/0011-spiffe-service-identity.md) (new)
- [`docs/adr/0012-agent-trust-composition.md`](../../adr/0012-agent-trust-composition.md) (new)
- [`docs/adr/0013-defer-root-intermediate-ca-split.md`](../../adr/0013-defer-root-intermediate-ca-split.md) (new)

Edition: Rust 2024

## 1. Goal

Close every confirmed gap from the mTLS audit (TOFU verifier accept-anything,
end-entity-only revocation flag, no cert resolvers, no ALPN on the production
listener, hand-rolled AIA/CDP DER, duplicate PEM parsers, triple X.509 parser
stack, `ClientConfig` per-reconnect rebuild, `Issuer` re-parse, CRL number
ordering, OCSP nonce and signer-cert gaps, pending key plaintext, non-atomic
cert + CA write, no session resumption) while moving the codebase towards
idiomatic rustls 0.23 patterns and reducing the surface of hand-rolled
cryptographic plumbing.

Two architectural shifts drive the rest of the spec:

- Server-cert trust and client-cert verifier are decoupled. An Operator can
  front the Controller with a Let's Encrypt (or any public-CA) certificate
  while the internal CA continues to sign Agent, Agent-SSH, MQTT, and
  Scheduler client certificates. Today the managed CA is implicitly both
  trust anchors at once; this spec separates them.
- Service identity migrates from CN-only to SPIFFE URI SANs of the form
  `spiffe://<trust_domain>/service/<service_id>`. CN remains during the
  natural two-year cert renewal tail and is then dropped in a follow-up.

After this work, Operator deployment options include:

- "All managed, self-signed" (today's default) — unchanged.
- "External-CA for clients, managed server cert" — unchanged.
- "Public-CA server cert (Let's Encrypt), managed-CA clients" — newly
  supported. The Operator points `--tls-cert`/`--tls-key` at the LE chain;
  Agents validate via `webpki-roots` + `rustls-native-certs` + the
  controller-CA bundle in a single root store.

The Operator override surface for "force invalid certificates" is preserved
and made explicit: the historical bare `--tofu` flag is replaced with four
named modes plus a `--tofu-skip-hostname` modifier. Insecure paths must be
opted into by name.

## 2. Background

The uptrakit Controller and its four Service profiles (Agent, Agent-SSH,
MQTT, Scheduler) communicate over mTLS. The Controller terminates HTTPS for
the Dashboard and the wire protocol; Services present client certificates
signed by the managed CA (`docs/security/pki-certificates.md`). A separate
TOFU bootstrap path lets a Service join a Controller it has never spoken to,
optionally pinning the CA bundle by SHA-256 fingerprint
(`docs/security/tofu-tls.md`).

A May 2026 audit ran against the current implementation surfaced sixteen
findings spanning correctness (TOFU accept-anything, end-entity-only
revocation flag), maintainability (triple X.509 parser stack, hand-rolled
DER for AIA and CDP extensions, duplicate PEM parsers), security hygiene
(plaintext renewal key in memory, non-atomic cert + CA write, OCSP nonce
echo missing, OCSP response missing signer cert), and performance
(`ClientConfig` rebuilt every reconnect, `Issuer` re-parsed on every CRL
rebuild, server `ServerConfig` rebuilt on every CRL refresh and CA-bundle
update, no TLS session resumption, no ALPN on the production listener).

Three findings were re-evaluated against existing documentation:

- The `.only_check_end_entity_revocation()` call is functionally moot
  because the managed CA is issued with `pathLenConstraint=0`
  (`pki-certificates.md`). The flag is defensively removed but the change
  has no behavioral consequence; a comment explains why.
- Hand-rolled AIA/CDP DER is documented as intentional in
  `pki-certificates.md` with a length-overflow guard. Replacement with
  `x509-cert::ext::pkix` builders is still justified on maintainability
  grounds.
- The TOFU verifier's accept-anything behavior is documented in
  `tofu-tls.md`. The real gap is twofold: it has no `ServerName` binding
  even when a fingerprint or SPKI hash is known, and bare `--tofu` (no pin)
  is silently insecure with no per-connection warning. Both are addressed.

This spec coordinates with the in-flight graceful-reload work
(`docs/superpowers/specs/2026-05-12-graceful-reload-design.md`,
ADR-0008). The Controller's TLS plumbing becomes a participant in the
graceful-reload `[tls]` Config Section and never requires a process restart
for cert rotation, CA-bundle update, CRL rebuild, or verifier swap.

## 3. Scope

### 3.1 In scope

**Agent / Service-SDK side** (`crates/shared/service-sdk`):

- Trust composition: `RootCertStore` built once from the controller-CA
  bundle by default. Operator-opt-in flags `--trust-public-roots` and
  `--trust-native-roots` additively pull in `webpki-roots` and
  `rustls-native-certs`. Single store, additive. Rebuilt on
  `CaBundleUpdated`; native-roots refresh at process restart only.
- Four explicit TOFU modes selected via mutually-exclusive CLI flags
  (`system`, `pin-fingerprint`, `pin-spki`, `insecure-tofu`); umbrella
  `--tofu` flag removed.
- `--tofu-skip-hostname` modifier for `pin-*` modes; implied by
  `--tofu-insecure`.
- `ServerName` binding enforced in every mode unless explicitly skipped.
- SPKI pin variant alongside the existing fingerprint pin. Both pin modes
  reject mid-renewal CA key changes; durability across CA rotation is a
  non-goal (see §8).
- `ResolvesClientCert` resolver backed by `ArcSwap<CertifiedKey>`.
  `Certificate` wire message swaps the resolver inner; current TLS session
  continues to use the old certificate until the next handshake.
- `Arc<ClientConfig>` cached and reused; rebuilt only on `CaBundleUpdated`
  or trust-composition change.
- Pending renewal private key held as `Zeroizing<String>`.
- Cert + CA bundle write becomes atomic using `tempfile::NamedTempFile::new_in`
  followed by `write_all`, `sync_all`, then `persist`. Both files written to temp,
  fsync'd, then renamed. Leftover `.tmp` siblings detected and removed at
  startup.
- TLS session resumption via `ClientSessionMemoryCache`.

**Controller side** (`crates/core/controller-runtime`,
`crates/ui/web-api`):

- `ResolvesServerCert` resolver backed by `ArcSwap<CertifiedKey>` for
  server-cert renewal. Server cert hot-swap no longer rebuilds
  `ServerConfig`.
- `DynamicClientVerifier`: a `ClientCertVerifier` implementation wrapping
  `ArcSwap<WebPkiClientVerifier>` (the alias resolves internally to
  `Arc<WebPkiClientVerifier>`). CRL rebuild and CA-bundle update swap the
  inner verifier without touching `ServerConfig` or the `axum_server`
  `RustlsConfig`.
- `RustlsConfig::reload_from_config()` retained for changes that must
  rebuild `ServerConfig`: ALPN list, cipher suites, TLS protocol versions,
  crypto provider. Driven exclusively by the graceful-reload `[tls]` Config
  Section.
- `.only_check_end_entity_revocation()` removed; comment notes that
  `pathLenConstraint=0` makes either choice equivalent today.
- ALPN configured on the production HTTPS listener: `h2`, `http/1.1`.
- TLS session resumption: `ServerSessionMemoryCache`.
- `Arc<Issuer>` cached per trusted CA; built once on init and on CA
  rotation, reused for every CRL rebuild and every CSR signature.
- CRL number atomic ordering changed from `Relaxed` to `AcqRel` on
  `fetch_add`; single-consumer serialization documented (the
  `revocation_notify` consumer loop is the only writer).

**PKI / ASN.1 unification**:

- AIA and CDP extension construction in
  `crates/core/controller-runtime/src/pki.rs` replaced with `x509-cert`
  builders (`AuthorityInfoAccessSyntax`, `CrlDistributionPoints`,
  `AccessDescription`, `DistributionPoint`) and `der::Encode::to_der`.
- The hand-rolled `pem_to_der_key` helper in `crates/ui/web-api/src/ocsp.rs`
  replaced with `rustls::pki_types::pem::PemObject` (already used in
  `pki.rs:1119`).
- The `x509-parser` dependency is dropped. Cert introspection
  (CN/SAN/validity extraction) migrates to
  `x509_cert::Certificate::from_der`.

**OCSP responder** (`crates/ui/web-api/src/ocsp.rs`):

- Request nonce extension parsed and echoed in `response_extensions`
  (RFC 6960 §4.4.1).
- `BasicOcspResponse.certs` populated with the active CA certificate DER so
  clients without the signer pre-trusted can validate the response.

**Service identity (SPIFFE)**:

- CSR generation in `crates/shared/service-sdk/src/identity.rs` adds
  `SanType::URI("spiffe://<trust_domain>/service/<service_id>")` to every
  CSR.
- The Controller's CSR signer preserves the SPIFFE SAN and rejects CSRs
  whose SAN URI does not match the expected trust domain.
- Identity extraction (`crates/ui/web-api/src/extract.rs`) reads the
  SPIFFE URI SAN first, falls back to CN. The fallback exists for the
  natural two-year renewal tail and is removed in a follow-up spec.
- `[tls] trust_domain` is added to the graceful-reload `[tls]` Config
  Section. Defaults to the first server-cert SAN.

**Dependencies**:

- Add `rustls-native-certs` to the workspace.
- Use the existing workspace `url = "2"` dep for SPIFFE URI parsing in
  identity extraction (no new addition).
- Verify `tempfile` is present in the workspace (used by tests already); if
  absent, add it.
- Drop the **direct** `x509-parser` workspace dependency. The crate remains
  a transitive dep of `rcgen` because `rcgen::Issuer::from_ca_cert_pem` —
  used in `controller-runtime/src/pki.rs`, `crl_manager.rs`, and
  `tasks.rs` — is gated behind rcgen's `x509-parser` feature, which the
  workspace already enables. Keeping the rcgen feature is required for
  those code paths and is unrelated to uptrakit's own use of x509-parser.
  After this spec, no uptrakit crate imports `x509_parser::*` directly; all
  cert introspection goes through `x509-cert`. The net dependency-tree
  effect is one fewer **direct** dep, not one fewer total dep.

**Documentation deliverables**:

- Rewrite `docs/security/tofu-tls.md` (modes table, override semantics,
  examples).
- Update `docs/security/pki-certificates.md` (trust composition, SPIFFE
  SAN, AIA/CDP refactor note, `DynamicClientVerifier` mention).
- Update `docs/security/key-rotation.md` (Zeroize + atomic write).
- Update `docs/security/secure-development.md` for any new patterns.
- Update `CONTEXT.md` with four glossary entries.
- New ADRs: 0011, 0012, 0013.
- Update `crates/shared/wire/CHANGELOG` and `asyncapi.yaml` for the
  `ServiceSettingsPayload.trust_domain` field addition (the only wire-
  message change). All other wire payloads — `Certificate`, `CSR`,
  `CaBundle`, enrollment flow — are unchanged.

### 3.2 Out of scope

- **Root/intermediate managed-CA split.** Recorded in §8 as Future Work and
  in ADR-0013 as a deferred decision. SPKI pin durability across CA
  rotation is achievable today only via the external-CA path
  (`--ca-cert`/`--ca-key`).
- **Out-of-process Service reload-without-restart.** Inherits the
  graceful-reload §3.2 boundary. Native-roots refresh on the Agent side
  requires a process restart.
- **OCSP key format auto-detection.** Managed CA emits PKCS#8 via rcgen
  exclusively. SEC1 / PKCS#1 detection is deferred unless real-world
  reports surface.
- **CSR `BasicConstraints` or `KeyUsage` rework.** Current values are
  correct; this spec adds SPIFFE SAN only.
- **Wire-protocol changes** (other than the single additive field).
  `Certificate`, `CSR`, `CaBundle`, and enrollment-flow payloads are
  unchanged. SPIFFE SAN lives inside the certificate-payload bytes already
  carried on the wire. The one in-scope wire change is the additive
  `trust_domain: String` field on `ServiceSettingsPayload` (§6).
- **Frontend changes.** The Dashboard does not surface TOFU mode or trust
  composition controls in this spec.
- **External-CA mode improvements.** This spec preserves the existing
  `--ca-cert`/`--ca-key` behavior unchanged.
- **Cross-controller CA bundle replication.** Already handled by NATS-
  propagated `CaBundleUpdated`; no changes here.

## 4. Domain Language Additions

The following terms land in `CONTEXT.md`. They appear in Operator-facing
documentation, CLI help text, and audit log entries.

- **TOFU mode** — one of `system`, `pin-fingerprint`, `pin-spki`,
  `insecure-tofu`. Selected at Service boot via a mutually-exclusive CLI
  flag. Determines how the Service verifies the Controller's TLS
  certificate during bootstrap and on subsequent reconnects when no CA
  bundle has been persisted. _Avoid_: "TOFU enabled" (ambiguous now —
  every mode is "enabled" in some sense).
- **SPIFFE Service Identity** — A Service's identity carried as a URI
  Subject Alternative Name on its client certificate, of the form
  `spiffe://<trust_domain>/service/<service_id>`. Replaces CN-only
  identity over the natural cert renewal cycle. _Avoid_: "service URI",
  "workload ID" (SPIFFE has a precise term).
- **Trust Domain** — A string in the `[tls]` Config Section naming the
  Controller's SPIFFE namespace. Defaults to the first server-cert SAN.
  Must match the trust-domain segment of every Service's SPIFFE URI SAN.
  _Avoid_: "domain" (overloaded), "namespace" (Kubernetes overload).
- **Dynamic Client Verifier** — The Controller-side wrapper around
  `WebPkiClientVerifier` that exposes an `ArcSwap` inner verifier. Lets
  CRL rebuilds and CA-bundle updates hot-swap the verifier without
  rebuilding `ServerConfig`. _Avoid_: "verifier reload" (overloaded with
  graceful-reload terminology).

## 5. Architecture

The spec touches several semi-independent subsystems. Each subsection
covers the change, the surrounding rationale, and explicit alternatives
where a real tradeoff exists.

### 5.1 Trust composition (Agent side)

The Agent's `RootCertStore` is built from up to three sources, **selected
explicitly by the Operator**:

1. **Controller-CA bundle** — the historical anchor. Always included.
   Continues to arrive over the wire as the `CaBundleUpdated` payload and
   is persisted to `service.json`. This alone is the default and matches
   the current security posture.
2. **`webpki-roots`** — compiled into the binary. Included only when the
   Operator passes `--trust-public-roots`. Used by Operators who front the
   Controller with a public-CA certificate (Let's Encrypt etc.).
3. **`rustls-native-certs`** — loaded at Agent startup from the OS root
   store. Included only when the Operator passes `--trust-native-roots`.
   Loaded on a blocking thread (`tokio::task::spawn_blocking`) to avoid
   blocking the runtime. Used by Operators with corporate-internal CAs or
   trust-store-managed environments.

Each enabled source feeds the same `RootCertStore`. rustls's verifier
accepts any chain that terminates at any anchor in the store.

**Default is conservative.** Today's Agent trusts only the controller-CA
bundle. Adding `webpki-roots` or `rustls-native-certs` by default would
silently expand the set of identities able to impersonate the Controller —
notably, anyone with access to a corporate MITM root (Zscaler, Netskope,
etc.) in the host OS store could intercept Agent→Controller traffic with
a valid chain. Operators must opt into each broader trust source.

The resulting `Arc<ClientConfig>` is cached on the Agent and reused across
reconnects. The cache is invalidated only when `CaBundleUpdated` arrives
or the trust composition itself changes (process restart, configuration
change).

**Native-roots staleness.** OS root store updates (an administrator
pushing a new corporate root via MDM, or a system package update of the
`ca-certificates` package) are **not** observed by a running Agent. The
native-roots snapshot is taken once at process startup and reused. To
pick up changes, restart the Agent. This is documented as expected
behavior in the rewritten `docs/security/tofu-tls.md`.

**Rationale for keeping the controller-CA bundle in the same store**: the
existing CA-bundle update path is the only way an Agent can learn about a
freshly rotated CA before its old anchor expires. Demoting the bundle to
a pin (rather than a root) would force the Agent to special-case the
bundle, complicating the verifier. The chosen approach reuses rustls's
native chain-validation logic across whichever sources the Operator
enabled.

**LE-fronted Controller deployment** uses `--trust-public-roots`. The
ServerName check (§5.2) ensures the chain validates _for the dialed
hostname_, not any hostname a public CA happens to have issued.

**CLI / config flags** for trust sources:

| Flag                   | Effect                                        |
| ---------------------- | --------------------------------------------- |
| (none)                 | Controller-CA bundle only (today's behavior). |
| `--trust-public-roots` | Add compiled-in `webpki-roots`.               |
| `--trust-native-roots` | Add OS root store via `rustls-native-certs`.  |

Flags compose additively. `--trust-native-roots` without `--trust-public-roots`
is unusual but supported (corporate-internal-CA-only deployments).

### 5.2 TOFU modes

`TofuVerifier` (`crates/shared/service-sdk/src/tls.rs`) is replaced with
four explicit modes. The umbrella `--tofu` boolean is removed; mode is
derived from which flag is present.

| Flag                          | Mode              | Server cert check                                                     | `ServerName`                                 |
| ----------------------------- | ----------------- | --------------------------------------------------------------------- | -------------------------------------------- |
| (none)                        | `system`          | Trust composition store; chain + expiry + key-usage required          | Enforced                                     |
| `--tofu-fingerprint=<sha256>` | `pin-fingerprint` | Any chain accepted iff the CA bundle SHA-256 matches                  | Enforced; opt-out via `--tofu-skip-hostname` |
| `--tofu-spki=<sha256>`        | `pin-spki`        | Any chain accepted iff any cert's `SubjectPublicKeyInfo` hash matches | Enforced; opt-out via `--tofu-skip-hostname` |
| `--tofu-insecure`             | `insecure-tofu`   | Accept any chain, log `WARN` every connection                         | Off (forced)                                 |

`--tofu-skip-hostname` is a modifier that requires one of the `pin-*` or
`--tofu-insecure` flags. `--tofu-insecure` implies it. Each pin/insecure
flag conflicts with `--ca-cert`, `--pki-addr`, and the other pin/insecure
flags via clap's `ArgGroup` mutual exclusion.

The CLI surface change is the primary backwards-incompatible break in this
spec. The historical bare `--tofu` is no longer expressible. Operators
who used it must now choose explicitly: pin via fingerprint or SPKI, or
accept-anything via `--tofu-insecure`. Following the graceful-reload §3.2
precedent, no compatibility shim is shipped.

**Bootstrap persistence rules:**

- `pin-fingerprint`: on first successful connection where the fetched CA
  bundle SHA-256 matches the supplied flag, the bundle is persisted to
  `service.json` as if `--ca-cert` had been used. Subsequent reconnects
  load the bundle from disk and use the `system` verifier path (with the
  bundle added to the root store per §5.1). The fingerprint flag is no
  longer required after persistence; if supplied on a subsequent run, the
  on-disk bundle is validated against it and a mismatch causes startup
  failure.
- `pin-spki`: same persistence flow. The matched SPKI hash is stored in
  `service.json` alongside the bundle so that future renewals validating
  via the same flag confirm key continuity (a fresh CA cert with the same
  public key still matches).
- `insecure-tofu`: by default the Agent operates as a stateless TOFU
  client — every reconnect re-fetches the bundle, no fingerprint pin is
  written to disk, and a `WARN` log fires on every connection. To
  persist the bundle, the Operator must pass
  `--tofu-fingerprint-acknowledge=<sha256>` matching the fingerprint
  observed on the previous run; this elevates the run to
  `pin-fingerprint` semantics for the persistence write.
- **Acknowledge mismatch behavior:** if `--tofu-fingerprint-acknowledge`
  is supplied and the bundle fetched on the current run produces a
  fingerprint different from the supplied value, the Agent logs both
  fingerprints at `ERROR` level and **exits with non-zero status before
  persisting anything**. The Operator must investigate — either the
  Controller's CA rotated legitimately (re-run without acknowledge to
  observe the new fingerprint, then re-acknowledge), or an attacker is
  intercepting the connection.

Two CLI helpers ease the transition:

- `--tofu-fingerprint-acknowledge=<sha256>` on the `insecure-tofu` mode is
  required before the Agent will persist the CA bundle. The first
  successful connection logs the fingerprint at `WARN` level; the
  Operator copies it back into the configuration and re-runs.
- `--tofu-spki=<sha256>` accepts either colon-separated or compact hex.
- `--tofu-fingerprint=<sha256>` accepts the same formats.

**ServerName enforcement details**: the verifier consults
`rustls::pki_types::ServerName` against the certificate's SAN list. For the
`pin-*` modes the leaf certificate's SAN must include the dialed
hostname; otherwise the handshake is rejected with
`CertificateError::NotValidForName`. The `--tofu-skip-hostname` modifier
disables this check while keeping the pin check.

**Rationale for four modes rather than a single insecure flag**: the audit
flagged the bare `--tofu` flag as a silent footgun. Forcing an explicit
mode forbids accidental insecure operation. SPKI pin survives certificate
renewal (the key stays stable across renewals) and is the IETF-recommended
pinning idiom; fingerprint pin remains for the "I have a hash someone
emailed me" workflow.

### 5.3 SPIFFE service identity

Every Service profile (Agent, Agent-SSH, MQTT, Scheduler) generates a CSR
with a SPIFFE URI SAN in addition to the existing CN. CN remains in the
Subject for the duration of the natural cert renewal cycle (max
`cert_lifetime_hours` is 17520 hours ≈ 2 years per
`pki-certificates.md`). A follow-up spec drops the CN once the renewal
tail completes.

**CSR generation** (`identity.rs::generate_keypair_and_csr`):

```rust
let mut params = CertificateParams::default();
params.distinguished_name.push(DnType::CommonName, service_id.to_string());
let spiffe_uri = format!("spiffe://{trust_domain}/service/{service_id}");
params.subject_alt_names = vec![
    SanType::URI(spiffe_uri.as_str().try_into()?),
];
```

`CertificateParams::default()` is used instead of `::new(vec![service_id])`
because `::new` pre-populates `subject_alt_names` with the input list as
`DnsName` SANs; the explicit `subject_alt_names = vec![...]` assignment would
silently discard them. `SanType::URI` wraps `rcgen::string::Ia5String`; the
URI is built into an owned `String` first and converted via `TryFrom<&str>`.

The `trust_domain` is fetched from the Controller via the existing
`ServiceSettings` payload (extended with a `trust_domain: String` field).
Until the Controller advertises one, the Agent falls back to the dialed
hostname; the Controller's CSR signer validates and may reject mismatches.

**Controller CSR signer** (`cert_signer.rs`):

- Reads the SPIFFE URI from the CSR.
- Validates the URI parses as `spiffe://<trust_domain>/service/<service_id>`.
- Rejects the CSR if `<trust_domain>` does not equal the configured
  `[tls] trust_domain`.
- Rejects the CSR if `<service_id>` does not equal the enrolled service's
  ID.
- Preserves the SAN list verbatim on the issued certificate.

**Identity extraction**
(`crates/ui/web-api/src/extract.rs::service_identity_from_der`):

- Parses the peer cert via `x509_cert::Certificate::from_der`.
- Iterates extensions, locates `SubjectAlternativeName` (OID 2.5.29.17).
- For each `GeneralName::UniformResourceIdentifier`, parses the value as a
  SPIFFE ID via the `url::Url` parser. Match the URI against:
  - `url.scheme() == "spiffe"`
  - `url.host_str() == Some(trust_domain)` (configured `[tls] trust_domain`)
  - `url.path_segments()` returns `Some(iter)` (rootless / cannot-be-a-base
    URIs return `None` and are rejected); collecting the iterator yields
    exactly `["service", service_id]`
  - `service_id` parses as `Uuid` (UUIDv7 expected; rejection of other
    versions is impl-plan concern).
- On match, returns `ServiceIdentity { service_id }`.
- On any rejection (parse failure, scheme mismatch, host mismatch, path
  mismatch, UUID parse failure, `path_segments()` returning `None`), the
  URI is skipped and the next SAN URI is tried.
- If no SAN URI matches the SPIFFE shape, falls back to the existing
  CN-extraction path.
- Logs `DEBUG` on every CN fallback so the renewal tail is observable.

Using `url::Url` (workspace dep — verify in impl plan) avoids the
naïve-split bug where `spiffe://trust.domain/service/uuid` produces five
segments after splitting on `/`. The Url parser also enforces SPIFFE's
lowercase-scheme and non-empty-authority requirements.

The `[tls] trust_domain` is a graceful-reload `[tls]` Section field. The
Controller validates `trust_domain` is non-empty and contains only
DNS-compatible characters. The default value is the first SAN of the
server certificate.

**Rationale for SPIFFE over URN-UUID**: SPIFFE is CNCF-graduated and the
de-facto identity scheme for mTLS workload identity across Istio, SPIRE,
Linkerd, and the AWS App Mesh ecosystem. Adopting it now keeps the door
open for future federation, OIDC token exchange via the SPIFFE Workload
API, and interop with service-mesh sidecars. URN-UUID has no such
ecosystem.

**Rationale for keeping CN during migration**: the renewal cycle is
self-healing — every cert is renewed at least once per
`cert_lifetime_hours` (≤ 2 years). Forcing a fleet-wide cert re-issue at
the rollout boundary would be operationally hostile.

### 5.4 Hot-swap resolvers and `DynamicClientVerifier`

Three hot-swap mechanisms replace the three full-rebuild paths:

#### 5.4.1 Agent client cert resolver

`AgentClientCertResolver` lives in `crates/shared/service-sdk/src/tls.rs`:

```rust
pub struct AgentClientCertResolver {
    current: arc_swap::ArcSwap<rustls::sign::CertifiedKey>,
}

impl rustls::client::ResolvesClientCert for AgentClientCertResolver {
    fn resolve(
        &self,
        _root_hint_subjects: &[&[u8]],
        _sigschemes: &[rustls::SignatureScheme],
    ) -> Option<std::sync::Arc<rustls::sign::CertifiedKey>> {
        Some(self.current.load_full())
    }

    fn has_certs(&self) -> bool { true }
}
```

The `_root_hint_subjects` parameter carries DER-encoded distinguished names
of CAs the server is willing to accept (rustls 0.23 names it
`root_hint_subjects`). The Agent has a single identity at any time; no
filtering by hint is required.

The resolver is installed once on the `ClientConfig`. On `Certificate` wire
message, the Agent builds a new `CertifiedKey` and calls
`resolver.current.store(Arc::new(new_certified_key))`. The currently
running WebSocket connection keeps using the old cert; the next handshake
(whatever causes it — network blip, planned reconnect, server cert renewal
on the Controller, session ticket rotation) presents the new cert.

The `Outcome::Reconnect` flow in `cert_handler.rs` is the default path
for normal renewals (well before expiry). When the new certificate arrives
within a **deadline-bound forced-reconnect window** of the existing cert's
`not_after`, the Agent still forces a reconnect to guarantee the new cert
is presented before the old one expires. The window is
`max(60 s, cert_lifetime / 50)`; rationale: long-lived idle connections
(keep-alive working, no natural handshake) would otherwise sit on an
about-to-expire cert and fail at the moment the TLS session next renegotiates,
which may be after expiry. The deadline-bound path retains the existing
`CERT_RECONNECT_DELAY` behavior (now reduced to ~100 ms; the previous 2 s
sleep was a safety margin against tight reconnect loops, no longer needed
once the resolver pattern is in place).

Outside the deadline-bound window, the resolver swap is sufficient. The
typical renewal lands well before expiry (33 h window at the default
168 h lifetime; 14-day cap), so deadline-bound reconnect is a rare safety
net, not the common path.

For the `CaBundleUpdated` case the Agent continues to rebuild
`Arc<ClientConfig>` because the verifier root store changes — this is rare
(CA rotation every 5 years per `pki-certificates.md`) and warrants the
full rebuild.

#### 5.4.2 Controller server cert resolver

`ControllerServerCertResolver` mirrors the Agent's resolver on the server
side:

```rust
pub struct ControllerServerCertResolver {
    current: arc_swap::ArcSwap<rustls::sign::CertifiedKey>,
}

impl rustls::server::ResolvesServerCert for ControllerServerCertResolver {
    fn resolve(
        &self,
        _client_hello: rustls::server::ClientHello<'_>,
    ) -> Option<std::sync::Arc<rustls::sign::CertifiedKey>> {
        Some(self.current.load_full())
    }
}
```

`arc_swap::ArcSwap<CertifiedKey>` stores `Arc<CertifiedKey>` internally
(the alias is `ArcSwap<T> = ArcSwapAny<Arc<T>>`); `load_full()` returns
`Arc<CertifiedKey>`, matching the trait return type exactly.

Server-cert renewal (every 60 days per `pki-certificates.md`) becomes a
single `store` call. `RustlsConfig::reload_from_config` is no longer
invoked on this path.

#### 5.4.3 `DynamicClientVerifier`

The most architecturally significant change. `ServerConfig` does not expose
its installed `Arc<dyn ClientCertVerifier>` for replacement after construction,
but the trait object itself is reached on every handshake — if the trait
object internally swaps its delegate, the verifier behavior updates without
touching `ServerConfig`. A wrapper that swaps an inner verifier behind an
`ArcSwap` and a parallel cached subjects slice sidesteps the immutability:

```rust
pub struct DynamicClientVerifier {
    inner: arc_swap::ArcSwap<rustls::server::WebPkiClientVerifier>,
    // Empty slice. See `root_hint_subjects` rationale.
    empty_subjects: Vec<rustls::DistinguishedName>,
}

impl rustls::server::danger::ClientCertVerifier for DynamicClientVerifier {
    fn offer_client_auth(&self) -> bool { true }
    fn client_auth_mandatory(&self) -> bool { false }

    fn root_hint_subjects(&self) -> &[rustls::DistinguishedName] {
        // Always empty. rustls treats the hint as advisory: clients send
        // whatever client cert they have; servers validate against the
        // verifier's actual root store on the next call. Returning an
        // empty hint avoids the lifetime puzzle of exposing an
        // `ArcSwap`-backed slice through a `&self`-tied borrow, and is
        // operationally equivalent for uptrakit Agents (each Agent holds
        // exactly one identity certificate and has nothing to filter).
        &self.empty_subjects
    }

    fn verify_client_cert(
        &self,
        end_entity: &rustls::pki_types::CertificateDer<'_>,
        intermediates: &[rustls::pki_types::CertificateDer<'_>],
        now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::server::danger::ClientCertVerified, rustls::Error> {
        self.inner.load().verify_client_cert(end_entity, intermediates, now)
    }

    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &rustls::pki_types::CertificateDer<'_>,
        d: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        self.inner.load().verify_tls12_signature(m, c, d)
    }

    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &rustls::pki_types::CertificateDer<'_>,
        d: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        self.inner.load().verify_tls13_signature(m, c, d)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.inner.load().supported_verify_schemes()
    }
}
```

**`root_hint_subjects` design.** The rustls 0.23 trait signature is
`fn root_hint_subjects(&self) -> &[DistinguishedName]` — a borrow tied to
`&self`. `ArcSwap::load()` returns a guard whose lifetime is independent
of `&self`, so the inner verifier's dynamically-loaded slice cannot be
returned through it without `unsafe` lifetime extension. The spec
**returns an empty slice** instead:

- rustls treats the hint as advisory (RFC 8446 §4.3.2 / RFC 5246 §7.4.4).
  A server returning no `certificate_authorities` extension is
  compliant; clients fall back to "send what you have."
- Every uptrakit Agent holds exactly one identity certificate; there is
  nothing for a hint to filter. The performance benefit of advertising
  trust anchors is zero in this deployment shape.
- The empty slice is owned by a `Vec<DistinguishedName>` field on
  `DynamicClientVerifier`; the `&self` borrow is satisfied trivially
  with no lifetime gymnastics, no leaks, no reclamation.

Trade-off accepted: a future deployment that wants Agents holding
multiple identities and selecting among them via hint would have to
replace this with a real implementation. None exists today. The
follow-up Path A (root/intermediate CA split, §8) does not introduce
multi-identity Agents either.

The previously considered alternatives — leaked-box reclamation,
`Arc<[_]>` via `ArcSwap` with clone-on-read, `unsafe` lifetime
widening of an `arc_swap::Guard` — all carry either soundness risk
(use-after-free under concurrent swap+read) or sustained per-swap
leakage that the spec is unwilling to ship in a security-critical code
path.

**Type note.** `arc_swap::ArcSwap<T>` is an alias for `ArcSwapAny<Arc<T>>`,
so the field type is `ArcSwap<WebPkiClientVerifier>` (not
`ArcSwap<Arc<WebPkiClientVerifier>>`); the load returns an
`arc_swap::Guard<Arc<WebPkiClientVerifier>>`.

`DynamicClientVerifier::new(initial)` builds a `WebPkiClientVerifier` with
the current root store and CRLs, wraps it in `ArcSwap`, and is installed on
the `ServerConfig` once at Controller startup.
`DynamicClientVerifier::swap(new_verifier)` is called on:

- CRL rebuild — every 4 hours via the existing `CrlRenewal` scheduler task
  plus on every revocation event.
- CA-bundle update — on every CA rotation or external `--ca-cert`/`--ca-key`
  swap.

**Mid-handshake swap consistency.** rustls calls `verify_client_cert`,
then `verify_tls12_signature` or `verify_tls13_signature` within the same
handshake. A swap that lands between these calls observes _different_
inner verifiers across one handshake. The naïve `inner.load()` per method
is not monotonic across all axes:

- CRL number is monotonic (write path takes `Ordering::AcqRel`), so a
  revoked-then-still-revoked outcome is consistent.
- CA bundle can _shrink_ across rotation when an aged-out root is dropped;
  CRL entries past `nextUpdate` of expired certs can be pruned. Either
  shrinkage can flip a `verify_client_cert` `Ok` to a subsequent
  `verify_tls*_signature` `Err` (chain trust loss between calls). Worst-
  case observable behavior is a one-off **handshake failure**, not a
  security regression — the client retries; the next handshake takes the
  newer verifier consistently.

To eliminate the inconsistency, `verify_client_cert` snapshots
`self.inner.load_full()` into a per-handshake `Arc<WebPkiClientVerifier>`
and stores it in a `parking_lot::Mutex<HashMap<HandshakeId, Arc<_>>>`
keyed by a handshake identifier. The subsequent `verify_tls*_signature`
call loads the same Arc from the map.

Practical caveat: rustls 0.23 does not expose a stable per-handshake
identifier to verifier methods. Two implementation options:

1. **No snapshot — accept transient handshake failures.** Simpler.
   Failures are observable, retryable, security-neutral. Default.
2. **Snapshot via address-of-CertificateDer.** Use the address of the
   `end_entity` slice as a key. Fragile (lifetime games) and only works
   if rustls preserves the slice between calls. Not recommended.

The spec chooses **Option 1**. The property test in §10 asserts that:

- No swap-mid-handshake produces a `verify_client_cert` `Ok` followed by
  a `verify_tls*_signature` panic or `Ok` on a chain the new verifier
  would reject — only `Err` is acceptable across the split.
- Across N concurrent handshakes + M swaps, no use-after-free, no panic,
  every Err is a recoverable retryable error.

**Rationale**: the audit's observation that the Controller rebuilds
`ServerConfig` on every CRL refresh holds, but the resolver pattern alone
doesn't fix it — resolvers swap server certs, not verifiers. Custom-
verifier wrapping is the only rustls-idiomatic way to dynamicize the
verifier. The pattern is documented in rustls' own examples for revocation
list refresh.

**The retired path**: `CrlManager::reload_tls_config` is renamed to
`CrlManager::swap_verifier` and reduced to a single `swap` call. The
`axum_server::tls_rustls::RustlsConfig` reference held by `CrlManager` is
dropped; `RustlsConfig::reload_from_config` continues to live in the
graceful-reload `[tls]` Section path for ALPN / cipher / TLS-version
changes only.

### 5.5 Cert and CA write atomicity, key zeroization

Two coupled disk/memory hygiene fixes in
`crates/shared/service-sdk/src/identity.rs`,
`crates/shared/service-sdk/src/ca.rs`, and
`crates/shared/service-sdk/src/cert_handler.rs`.

**Pending renewal key (`cert_handler.rs`)**:

```rust
struct CertificateRenewalHandler {
    pending_renewal_key: Option<zeroize::Zeroizing<String>>,
    // ...
}
```

`Zeroizing<String>` zeroes the string buffer on drop. This mirrors the
`CaKeyStore` pattern already used on the Controller side
(`pki-certificates.md:297`). The wrapper is transparent
(`Deref<Target=String>`) so the rest of the renewal flow needs no further
changes.

**Reallocation hazard.** `String::push_str` or `format!` operations on a
`Zeroizing<String>` can grow the backing allocation, copy the contents
into a fresh allocation, and free the old allocation **without zeroing**.
The pending key is therefore constructed once via the rcgen
`KeyPair::serialize_pem` call (which returns an already-final `String`)
and immediately wrapped:

```rust
let pem: String = key_pair.serialize_pem();
let pending = Zeroizing::new(pem);
debug_assert_eq!(pending.len(), pending.capacity(),
    "pending renewal key must not have spare capacity that could be left un-zeroized on growth");
```

No mutation of the wrapped `String` after construction. Renewal flow
treats `pending_renewal_key` as read-only between CSR send and Certificate
receipt. The `debug_assert` catches future refactors that introduce
growth.

**Atomic cert + CA write (`identity.rs::save_identity`)**:

```rust
let cert_path = base.join("service.json");
let key_path = base.join("service.key");

let mut cert_tmp = tempfile::NamedTempFile::new_in(&base)?;
cert_tmp.write_all(service_json.as_bytes())?;
cert_tmp.as_file().sync_all()?;

let mut key_tmp = tempfile::NamedTempFile::new_in(&base)?;
key_tmp.write_all(key_pem.as_bytes())?;
key_tmp.as_file().sync_all()?;

// Persist atomically. POSIX rename is atomic; tempfile handles Windows.
cert_tmp.persist(&cert_path)?;
key_tmp.persist(&key_path)?;
```

The two `persist` calls are sequential, not atomic together. Crash between
the first and the second leaves a fresh `service.json` paired with the
previous `service.key`. Both files are signed by the controller CA — the
old key cannot validate against the new cert, so on next start the Agent
detects mismatch (existing `cert_handler.rs` logic) and re-enrolls. This
is strictly safer than today's behavior (half-written file → panic on
parse).

**Startup `.tmp` sweep**: on Agent startup, `service-sdk` enumerates
sibling `.tmp` files in the identity directory and removes them. A
warning is logged for each. The presence of a `.tmp` file means a previous
process was killed between write and rename; the rename being incomplete
means the corresponding "real" file is the older, valid version.

**Permissions**: `tempfile::NamedTempFile` defaults to 0600 on POSIX, which
matches the existing identity-file permissions. The `persist` call retains
the temp file's mode.

**Rationale for `tempfile`**: the alternative is hand-rolling
`open(O_CREAT | O_EXCL) + write + fsync + rename`. `tempfile` is a one-line
dependency that handles edge cases (Windows atomic rename, directory
fsync, cleanup on drop) the hand-roll would miss.

### 5.6 OCSP responder and CRL hygiene

**OCSP nonce echo** (`ocsp.rs`):

The responder iterates `request.tbs_request.request_extensions` for the
nonce extension (OID 1.3.6.1.5.5.7.48.1.2). If present, the same nonce
value is included in the response's `response_extensions`. RFC 6960
§4.4.1 specifies the encoding: a `Nonce` extension carrying an `OCTET
STRING` of 1 to 32 bytes.

**OCSP signer cert in response** (`ocsp.rs`):

`BasicOcspResponse.certs` is populated with the active CA's certificate
DER. Clients that lack the signer in their pre-trusted store can validate
the response via this embedded certificate.

The responder continues to sign with the active CA key directly (no
delegated OCSP signing cert). Per `pki-certificates.md`, OCSP responses
use ECDSA P-256 SHA-256, matching the CA signature algorithm.

**CRL number ordering** (`crl_manager.rs`):

```rust
let new_crl_number = self.crl_number.fetch_add(1, Ordering::AcqRel);
```

The existing `Relaxed` ordering provides no inter-thread guarantee. While
the `revocation_notify` consumer loop is single-consumer by construction
(documented invariant), `AcqRel` makes the dependency explicit and survives
any future refactor that introduces a second writer. A new doc comment on
`CrlManager::run` calls out the single-writer invariant.

The DB-side `crl_cache.crl_number` is already monotonic across restarts
(`pki-certificates.md:235-236`). No schema change.

### 5.7 ALPN and session resumption

**ALPN on production HTTPS listener** (`pki.rs`):

```rust
let mut config = rustls::ServerConfig::builder()
    .with_client_cert_verifier(dynamic_verifier)
    .with_cert_resolver(server_cert_resolver);
config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
```

The reverse-proxy integration tests already set ALPN
(`reverse_proxy/server.rs:300`); this change brings the production
listener to parity.

**Session resumption** (both sides):

- Server: `config.session_storage = rustls::server::ServerSessionMemoryCache::new(1024);`
  (`ServerSessionMemoryCache::new(n)` returns `Arc<Self>`, which assigns
  into `ServerConfig.session_storage: Arc<dyn StoresServerSessions>`
  directly; no further `Arc::new` wrap is needed.)
- Client: `config.resumption = rustls::client::Resumption::in_memory_sessions(256);`

In-memory only. Cluster-wide resumption (Redis / shared-storage) is out of
scope. Defaults are overridden because the rustls 0.23 stock defaults are
intentionally small (256 sessions server-side) and assume a single-instance
TLS terminator; the Controller's reverse-proxy-fronted deployments expect
fleet-scale reconnect churn. The cost is a few KB per
cached session and a measurable handshake-time reduction on reconnect.

**Rationale**: TLS 1.3 1-RTT handshakes are already cheap (~30 ms on
modern hardware); 0-RTT resumption shaves another 30 ms. Across a fleet
that reconnects on network blips, this materially reduces tail latency
without complicating the security model — rustls's resumption tickets are
forward-secret and authenticated by the same anchors as the original
handshake.

### 5.8 ASN.1 / DER unification

**AIA / CDP extensions** (`pki.rs`):

The hand-rolled DER builders (`encode_der_length`, `encode_der_sequence`,
`encode_access_description`, `build_aia_extension_der`) are replaced with
`x509-cert`'s PKIX builders. The new flow:

```rust
use x509_cert::ext::pkix::{
    AccessDescription, AuthorityInfoAccessSyntax, CrlDistributionPoints,
    DistributionPoint, name::GeneralName,
};
use der::{Encode, asn1::Ia5String};

let ocsp_uri = Ia5String::new(ocsp_url.as_bytes())?;
let ca_issuers_uri = Ia5String::new(ca_issuers_url.as_bytes())?;
let cdp_uri = Ia5String::new(crl_url.as_bytes())?;

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
let aia_der = aia.to_der()?;

let cdp = CrlDistributionPoints(vec![DistributionPoint {
    distribution_point: Some(
        x509_cert::ext::pkix::DistributionPointName::FullName(vec![
            GeneralName::UniformResourceIdentifier(cdp_uri),
        ]),
    ),
    reasons: None,
    crl_issuer: None,
}]);
let cdp_der = cdp.to_der()?;
```

`GeneralName::UniformResourceIdentifier` wraps `der::asn1::Ia5String`; the
owned-bytes constructor `Ia5String::new(&[u8])` validates the IA5 alphabet
and returns an owned `Ia5String`. `der::asn1::Ia5String` does not have
`TryFrom<&str>` (only `TryFrom<String>`) and has no `FromStr` impl, so
`.parse()` would not compile. OIDs come from the `const-oid::db::rfc5280`
constants — `const-oid 0.9` is already in the workspace with the `db`
feature enabled.

The hand-rolled 64-KiB length-overflow guard is retired — `der` enforces
DER length encoding correctly across all sizes. The
`PkiError::LengthOverflow` variant is removed.

**`pem_to_der_key`** (`ocsp.rs`):

```rust
use rustls::pki_types::pem::PemObject;
let key_der = rustls::pki_types::PrivateKeyDer::from_pem_slice(pem.as_bytes())?;
```

The hand-rolled base64-strip helper is deleted. `PemObject` is already used
in `pki.rs:1119`; unifying makes the codebase use a single PEM entry
point.

**`x509-parser` removal**:

Two production-side consumers and a handful of test consumers. Migration
targets `x509-cert::Certificate::from_der` for parsing and
`x509-cert::ext::pkix` for extension access. The shape change:

```rust
// before
let (_, cert) = x509_parser::parse_x509_certificate(der)?;
let cn = cert.subject().iter_common_name().next();

// after
use der::asn1::{Utf8StringRef, PrintableStringRef};
let cert = x509_cert::Certificate::from_der(der)?;
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

The new shape is more verbose but uses the same `der`-based parser that
`x509-cert` and `x509-ocsp` already pull in. The direct `x509-parser`
workspace dep is removed; `x509-parser` itself remains in `Cargo.lock` as
a transitive dep of `rcgen` (its `x509-parser` feature is required by the
`Issuer::from_ca_cert_pem` paths in `pki.rs`, `crl_manager.rs`, and
`tasks.rs`). The reduction is in uptrakit's own surface, not in the
transitive dependency tree.

### 5.9 `Arc<Issuer>` caching

`crl_manager.rs` and `cert_signer.rs` re-parse the CA PEM into `rcgen::Issuer`
on every CRL rebuild and every CSR signature. The cached form:

```rust
struct TrustedIssuer {
    fingerprint: String,
    cert_der: Vec<u8>,
    issuer: Arc<rcgen::Issuer<'static, rcgen::KeyPair>>,
    // ...
}
```

The `Issuer` is built once when a CA is added to the trusted set (on
init, on rotation, on external-CA load) and reused for every signing
operation. CA rotation invalidates the prior entry; the next signing
operation hits the new cache entry.

`rcgen::Issuer<'static, KeyPair>` is `Send + Sync` under the
`aws_lc_rs` feature (the workspace default — see `Cargo.toml`) and the
existing codebase already stores it in a plain `Arc` without inner
locking. The cache reuses the same shape: `Arc<Issuer<'static, KeyPair>>`,
no `RwLock` wrapper required. If a future signer trait change introduces
interior mutability requirements, switch to `parking_lot::RwLock` per the
coding-standards Panic Policy rule (never `tokio::sync` or `std::sync` for
sync locks in async code).

### 5.10 `.only_check_end_entity_revocation()` removal

The flag is removed from the `WebPkiClientVerifier::builder` chain in
`pki.rs:1187`. A comment explains the equivalence:

```rust
// The managed CA is issued with pathLenConstraint=0 (see
// docs/security/pki-certificates.md). No intermediate CAs exist in any
// agent's certificate chain, so end-entity revocation checking and full-
// chain revocation checking are equivalent. The flag is omitted for
// defensive hygiene: if a future change introduces intermediates, the
// default (full-chain check) is the safer behavior.
```

No behavioral change today. The removed surface is the audit's CRITICAL #2.

## 6. Wire and API changes

**Wire protocol** (`crates/shared/wire`):

- `ServiceSettingsPayload` gains a `trust_domain: String` field, declared
  as:

  ```rust
  #[serde(default, skip_serializing_if = "String::is_empty")]
  pub trust_domain: String,
  ```

  `serde(default)` is required so that pre-rollout Agents receiving
  payloads from a new Controller, **and** new Agents receiving payloads
  from a pre-rollout Controller, both deserialize without error. The
  empty-string default coincides with "no trust domain configured" and
  matches the Agent's fallback-to-dialed-hostname behavior.

- The existing `ServiceSettingsPayload` struct gains `#[non_exhaustive]`
  per the coding-standards rule for extensible public structs (the
  workspace MEMORY notes this is required for any extensible wire-payload
  struct). External call sites switch to `..Default::default()` patterns
  if they construct the struct directly.
- No other wire changes. `Certificate`, `CSR`, `CaBundle`, and revocation
  payloads are unchanged. The SPIFFE SAN lives inside the certificate
  payload bytes — no envelope change required.

**HTTP API** (`crates/ui/web-api`):

- No new endpoints.
- `GET /api/v1/settings/pki` response gains `trust_domain: String`.
- `GET /api/v1/services/{id}` response gains `spiffe_id: Option<String>`
  derived from the service's current certificate. Read-only.

**Configuration / graceful-reload**:

- `[tls] trust_domain` lives in the graceful-reload `[tls]` Section.
  Reload validates the value is DNS-compatible and non-empty.

**CLI flags** (Agent / Service SDK):

| Old                               | New                                                         |
| --------------------------------- | ----------------------------------------------------------- |
| `--tofu` (bare)                   | removed; no shim                                            |
| `--tofu --tofu-fingerprint=<hex>` | `--tofu-fingerprint=<hex>`                                  |
| n/a                               | `--tofu-spki=<hex>`                                         |
| n/a                               | `--tofu-insecure`                                           |
| n/a                               | `--tofu-skip-hostname` (modifier on pin / insecure modes)   |
| n/a                               | `--tofu-fingerprint-acknowledge=<hex>` (on `insecure-tofu`) |

## 7. Rollout

Single release. All changes ship together: the audit findings are coupled
(e.g., the resolver pattern needs `Arc<ClientConfig>` caching to be
useful; `DynamicClientVerifier` needs `ResolvesServerCert` to fully remove
`ServerConfig` rebuild paths). Splitting would create a half-modernized
intermediate state that is harder to test and reason about.

**Order of work** (each step gated by full quality-gate green and
reverse-proxy + system integration tests passing per
`docs/development/quality-gates.md`):

1. Workspace deps: add `rustls-native-certs`, verify `tempfile`, drop
   `x509-parser`.
2. ASN.1/PEM unification: migrate `pem_to_der_key`, AIA/CDP builders, and
   `x509-cert::Certificate::from_der` introspection paths.
3. `Arc<Issuer>` cache.
4. `Zeroizing<String>` pending key + atomic cert/CA write.
5. OCSP nonce + signer cert.
6. CRL number `AcqRel` + invariant doc.
7. Trust composition: `webpki-roots` + `rustls-native-certs` +
   controller-CA bundle merged.
8. `Arc<ClientConfig>` cache.
9. `AgentClientCertResolver`.
10. `ControllerServerCertResolver` + `DynamicClientVerifier`.
11. `.only_check_end_entity_revocation` removal + comment.
12. ALPN + session resumption.
13. SPIFFE SAN: `[tls] trust_domain` field, CSR generation, signer
    validation, identity extractor.
14. TOFU modes: CLI surface, verifier impl, mode-aware bootstrap.
15. ADRs 0011, 0012, 0013 (concurrent with the relevant impl step).
16. Documentation: rewrite `tofu-tls.md`, update `pki-certificates.md`,
    `key-rotation.md`, `secure-development.md`; add four `CONTEXT.md`
    glossary entries.

The implementation plan (separate spec → plan cycle) sequences these into
incremental PRs.

## 8. Future work

**Path A — root/intermediate managed CA split.**

The current managed CA is a single self-signed certificate that signs both
the server cert and all Service client certs. Pinning a SPKI hash on the
Agent side gives the Agent a stable identity for the Controller, but the
hash changes on every CA rotation (every 5 years per
`pki-certificates.md`) because rotation introduces a fresh keypair.

A two-tier managed CA — a long-lived (15–25 year) root with a stable
keypair, signing short-lived (5-year) issuing intermediates — would let
the Agent SPKI-pin the root. The intermediate rotates freely; the pin
survives.

Shape of the change:

- Two-step CA bootstrap: generate root, generate intermediate signed by
  root, persist both.
- `bundle_pem` includes root + active intermediate (+ historical
  intermediates within their lifetime).
- CRLs signed by the intermediate (current behavior); OCSP responses
  signed by the intermediate; Agent SPKI pin targets the root.
- CA rotation rotates the intermediate only. Root rotation is a separate,
  rare, manual ceremony.
- DB schema: `ca_certificate` gains a `parent_fingerprint` nullable
  column.

This is a separable improvement and is deferred to its own spec.
ADR-0013 records the deferral so future contributors don't re-litigate
the decision without re-reading this spec.

**Drop CN from Service identity.**

After two years of natural cert renewal (one cert lifetime cycle), every
Service certificate carries a SPIFFE URI SAN. A follow-up spec removes the
CN-fallback path in `service_identity_from_der` and drops the CN entirely
from new CSRs.

**OCSP key format detection.**

Today the OCSP signer assumes the active CA key is PKCS#8 (always true for
rcgen-generated managed CAs). If real-world deployments via external CA
need SEC1 or PKCS#1, add `pkcs8::SecretDocument::from_pem` sniffing as a
follow-up.

**Frontend TOFU/trust controls.**

The Dashboard could surface trust composition and Service certificate
identity (SPIFFE URI, expiry, renewal window) in a read-only TLS panel.
Out of scope here.

## 9. Documentation deliverables

Implementation must touch every doc listed below. Each entry is non-
optional unless marked.

**Security docs** (`docs/security/`):

- `tofu-tls.md` — rewrite. New modes table, override semantics, examples
  for each mode, deprecation note for bare `--tofu`.
- `pki-certificates.md` — update. Trust composition section, SPIFFE SAN
  section, `DynamicClientVerifier` mention in "State Management", AIA/CDP
  refactor note replaces the hand-rolled-DER section, `.only_check_end_
entity_revocation` rationale.
- `key-rotation.md` — update. `Zeroizing<String>` for pending renewal key,
  atomic cert/CA write semantics, `.tmp` sweep.
- `secure-development.md` — update. Reference the new resolver patterns
  and `DynamicClientVerifier` as the canonical hot-swap idiom.

**Domain language** (`CONTEXT.md`):

- Four new entries: TOFU mode, SPIFFE Service Identity, Trust Domain,
  Dynamic Client Verifier.

**ADRs** (`docs/adr/`):

- `0011-spiffe-service-identity.md` (new) — covers SPIFFE adoption, the
  CN-fallback migration window, and the URN-UUID alternative considered
  and rejected.
- `0012-agent-trust-composition.md` (new) — covers webpki-roots ∪
  rustls-native-certs ∪ controller-CA bundle, the LE-fronted-Controller
  use case, and the demote-to-pin alternative considered and rejected.
- `0013-defer-root-intermediate-ca-split.md` (new) — records the explicit
  deferral of Path A and the rationale (separable improvement, materializes
  every 5 years).

**API docs** (`docs/api/`):

- `Settings.md` — add `trust_domain` to the PKI settings response shape.
- `Services.md` — add `spiffe_id` to the service detail response.

**Wire protocol** (`crates/shared/wire/`):

- `asyncapi.yaml` — add `trust_domain: string` to
  `ServiceSettingsPayload`.

**Public type / interface docstrings**:

- `TofuMode`, `TofuVerifier` (new shape), `AgentClientCertResolver`,
  `ControllerServerCertResolver`, `DynamicClientVerifier`.

**No-impact**:

- Frontend: untouched (no UI surface in this spec).

## 10. Testing

Every change has unit-test coverage; many also require integration tests
per `docs/development/quality-gates.md`.

**Unit tests** (per change):

- TOFU modes: each mode's accept/reject decision under matching, mismatching,
  expired, hostname-mismatching cert inputs. ServerName binding on/off
  combinations.
- `AgentClientCertResolver` and `ControllerServerCertResolver`: swap takes
  effect on next handshake, current session unaffected.
- `DynamicClientVerifier`: concurrent swap + verification fuzz test
  asserts (a) no use-after-free, (b) no panic, (c) no spuriously-accepted
  chain — every `verify_client_cert`+`verify_tls*_signature` pair that
  crosses a swap either succeeds against one of the two verifiers
  consistently, or returns `Err` (a recoverable handshake failure the
  client retries). The previously-asserted "strictly-more-current" outcome
  is **not** required; CA-bundle and CRL shrinkage make it unprovable.
- SPIFFE URI parsing and validation: malformed URIs rejected; trust-domain
  mismatch rejected; service-id mismatch rejected.
- Identity extraction: SAN URI happy path, CN fallback path, both-present
  prefers SAN, neither-present returns `None`.
- OCSP nonce: present nonce echoed verbatim, absent nonce omitted from
  response.
- OCSP `certs` field populated and decodable.
- Atomic write: crash between `write_all` and `persist` simulated via
  drop-of-`NamedTempFile`; result is intact previous-version files,
  zero-byte `.tmp` cleaned on startup.
- `Zeroizing<String>` test confirms memory is wiped on drop (best-effort;
  test reads the buffer via raw pointer post-drop).
- AIA/CDP DER builders produce byte-for-byte output that x509-cert can
  round-trip.
- `Arc<Issuer>` cache: rebuild count == 1 per CA across N CRL refreshes.
- CRL number `AcqRel`: deliberate spawn of two writers (test-only) does
  not produce duplicate numbers.

**Integration tests**:

- `crates/core/integration-tests/tests/reverse_proxy/` — every reverse
  proxy harness (nginx, haproxy, envoy, traefik, caddy, with CRL and
  OCSP variants) reruns under the new code paths.
- `crates/core/integration-tests/tests/enrollment/` — new test: SPIFFE
  SAN end-to-end (Agent CSR → Controller sign → reconnect with new cert
  → identity extracted from SAN).
- `crates/core/integration-tests/tests/enrollment/` — new test: TOFU
  fingerprint pin mode succeeds with matching fingerprint, rejects on
  mismatch.
- `crates/core/integration-tests/tests/enrollment/` — new test: TOFU
  SPKI pin mode survives CA cert renewal that reuses the keypair
  (manually constructed test fixture; managed CA does not do this, but
  the spec promises SPKI durability if the key is stable).
- `crates/core/integration-tests/tests/enrollment/` — new test:
  LE-style server cert + managed-CA client cert combination handshakes
  successfully on the Agent side.
- `crates/core/integration-tests/tests/wire/` — `trust_domain` field
  round-trips on `ServiceSettingsPayload`.

**Quality gates**:

The full quality-gate suite from `docs/development/quality-gates.md`
must pass for every PR: `cargo fmt --all`, `cargo check --no-default-
features --features db-sqlite`, `cargo check --all-features`, `cargo
clippy --all-targets` (both feature sets), `cargo test --all-features`,
`cargo deny check`, markdownlint. Reverse-proxy and system integration
tests are mandatory per quality-gates.md for any mTLS / cert-forwarding
change.

## 11. Risks and mitigations

**Risk**: CLI break for Operators using bare `--tofu`.

Mitigation: release notes call out the change with a migration table.
The graceful-reload spec sets the precedent for breaking CLI changes
without a deprecation window in this codebase. The Operator population is
small and the replacement flags are clearly named.

**Risk**: `rustls-native-certs` adds a runtime OS dependency that can
fail in stripped containers.

Mitigation: native-certs loader is fallible and best-effort. If the OS
root store is unreadable, the Agent logs a `WARN` and continues with
`webpki-roots` + controller-CA bundle. Service operation is unaffected
in container deployments that use the controller-CA bundle anyway.

**Risk**: `DynamicClientVerifier` mid-handshake swap produces inconsistent
verifier state across the `verify_client_cert` → `verify_tls*_signature`
call sequence.

Mitigation: the swap is **not** guaranteed monotonic — CA-bundle rotation
and CRL pruning can shrink trust. Worst-case observable behavior is a
**transient handshake failure**, not a security regression. Clients retry;
the next handshake takes the newer verifier consistently. The property
test in §10 asserts no UAF, no panic, and no spuriously-accepted chain
across swap+verify races. The Agent's reconnect-with-backoff path already
handles handshake failures uniformly with network blips.

**Risk**: SPIFFE trust-domain misconfiguration locks out the entire
fleet.

Mitigation: the Controller refuses to start with an invalid
`trust_domain` (empty, non-DNS-compatible characters). The Agent falls
back to CN extraction during the renewal tail, so a misconfigured
trust-domain on the Controller side initially manifests as new CSRs
being rejected — visible in the audit log, not silent.

**Risk**: `Arc<Issuer>` cache miss after CA rotation causes signing
storm.

Mitigation: CA rotation populates the new cache entry synchronously
before marking the new CA active. The signing path observes the new
entry on first read.

**Risk**: Session resumption ticket key compromise across restart.

Mitigation: in-memory only. Restart invalidates all tickets. Cluster-wide
resumption (Redis-backed) is explicitly out of scope.

## 12. References

- Audit transcript: in-conversation, 2026-05-12.
- Standards snapshot: `.superpowers/standards-snapshot.md`.
- `docs/security/tofu-tls.md`
- `docs/security/pki-certificates.md`
- `docs/security/key-rotation.md`
- `docs/security/secure-development.md`
- `docs/superpowers/specs/2026-05-12-graceful-reload-design.md`
- `docs/adr/0008-graceful-reload-architecture.md` (in-flight)
- RFC 6960 (OCSP)
- RFC 5280 (PKIX)
- SPIFFE specifications: <https://github.com/spiffe/spiffe>
- rustls 0.23 documentation: <https://docs.rs/rustls/0.23>
  ls/0.23>
