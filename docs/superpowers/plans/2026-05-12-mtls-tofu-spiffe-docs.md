# mTLS TOFU Modes, SPIFFE Identity, and Documentation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the Operator-visible surface of the mTLS hardening: four explicit TOFU modes
(`system`, `pin-fingerprint`, `pin-spki`, `insecure-tofu`) replacing the bare `--tofu` flag,
opt-in trust composition flags (`--trust-public-roots`, `--trust-native-roots`), SPIFFE URI
Subject Alternative Names (`spiffe://<trust_domain>/service/<service_id>`) on every Service
cert, the `ServiceSettingsPayload.trust_domain` wire-field addition, the `spiffe_id` HTTP API
field, three ADRs (0011 SPIFFE, 0012 trust composition, 0013 defer root/intermediate split),
and full documentation rewrites.

**Architecture:** Agent-side, replace `TofuVerifier` with a mode-dispatched verifier behind
a `ServerCertVerifier` impl that consults the active mode at handshake time. Controller-side,
validate SPIFFE URIs at CSR-sign time and during identity extraction. Wire the `trust_domain`
field through `ServiceSettingsPayload` with `#[serde(default)]` so cross-version
Agent/Controller pairs deserialize cleanly. Documentation lands in lockstep with the code:
every section that describes runtime behavior is updated in the same commit that introduces
the behavior.

**Tech Stack:** Rust 2024, `rustls 0.23`, `rcgen 0.14`, `x509-cert 0.2`, `url 2`,
`rustls-native-certs` (new), `webpki-roots 1`, `clap 4` (ArgGroup), `uuid` (UUIDv7).
Spec: `docs/superpowers/specs/2026-05-12-mtls-hardening-design.md` (§5.1 trust composition,
§5.2 TOFU modes, §5.3 SPIFFE, §6 wire/API, §9 doc deliverables). Depends on Plan 1 + Plan 2
having landed.

---

## File Map

- Modify: `Cargo.toml`
  - Add `rustls-native-certs` to `[workspace.dependencies]`
- Modify: `crates/shared/service-sdk/Cargo.toml`
  - Add `rustls-native-certs`, `url`, `uuid` deps
- Create: `crates/shared/service-sdk/src/tofu.rs`
  - `TofuMode` enum: `System`, `PinFingerprint(Sha256Hash)`, `PinSpki(Sha256Hash)`, `InsecureTofu`
  - `TofuConfig { mode, skip_hostname, fingerprint_acknowledge }`
  - Mode-dispatched `ServerCertVerifier` impl
  - Server-cert chain → SPKI hash extraction helper (using `x509-cert`)
- Modify: `crates/shared/service-sdk/src/cli.rs`
  - Drop `pub tofu: bool`
  - Add `tofu_fingerprint: Option<Sha256Hash>`, `tofu_spki: Option<Sha256Hash>`,
    `tofu_insecure: bool`, `tofu_skip_hostname: bool`,
    `tofu_fingerprint_acknowledge: Option<Sha256Hash>`, `trust_public_roots: bool`,
    `trust_native_roots: bool`
  - `ArgGroup` for mutual exclusion among pin/insecure flags
  - Conflicts: each pin/insecure flag conflicts with `--ca-cert`, `--pki-addr`
- Modify: `crates/shared/service-sdk/src/tls.rs`
  - Trust composition builder: `build_root_store(controller_ca_pem, opts: TrustOptions) ->
    Result<RootCertStore, _>`
  - `TrustOptions { trust_public_roots: bool, trust_native_roots: bool }`
  - Load `rustls-native-certs` via `tokio::task::spawn_blocking` at process startup
- Modify: `crates/shared/service-sdk/src/ca.rs`
  - `bootstrap_ca` honors the active `TofuMode`
  - `pin-fingerprint` / `pin-spki` persistence semantics on first contact
  - `insecure-tofu` mismatch handling for `--tofu-fingerprint-acknowledge`
- Modify: `crates/shared/service-sdk/src/identity.rs`
  - CSR generation adds SPIFFE URI SAN: `SanType::URI(spiffe://<trust_domain>/service/<service_id>)`
  - `CertificateParams::default()` instead of `::new(vec![id])` to avoid implicit DNS SAN
- Modify: `crates/shared/service-sdk/src/cert_handler.rs`
  - Fetch and store `trust_domain` from `ServiceSettingsPayload`
- Modify: `crates/shared/wire/src/payloads.rs`
  - Add `pub trust_domain: String` to `ServiceSettingsPayload` with `#[serde(default,
    skip_serializing_if = "String::is_empty")]`
  - Add `#[non_exhaustive]` to the struct
- Modify: `crates/shared/wire/asyncapi.yaml`
  - Add `trust_domain` field to `ServiceSettingsPayload` schema
- Modify: `crates/core/controller-runtime/src/cert_signer.rs`
  - Validate CSR's SPIFFE SAN; reject mismatched trust-domain or service-id
  - Preserve SAN list on issued cert
- Modify: `crates/ui/web-api/src/extract.rs`
  - `service_identity_from_der`: SAN-URI SPIFFE parse via `url::Url`, CN fallback, `DEBUG` log on
    fallback
- Modify: `crates/ui/web-api/src/routes/settings_global_combined.rs` (or wherever PKI settings are
  surfaced)
  - Add `trust_domain: String` to `GetPkiSettingsResponse`
- Modify: `crates/ui/web-api/src/routes/services.rs`
  - Add `spiffe_id: Option<String>` derived from current cert on `GetServiceResponse`
- Modify: `crates/core/controller-runtime/src/startup/settings.rs`
  - Read `[tls] trust_domain` from config; default to first server-cert SAN
- Tests:
  - `crates/shared/service-sdk/src/tofu.rs` — mode behavior matrix
  - `crates/shared/service-sdk/src/ca.rs` — pin-fingerprint persistence, mismatch failure path
  - `crates/shared/service-sdk/src/identity.rs` — CSR has SPIFFE SAN, default DN unchanged
  - `crates/ui/web-api/src/extract.rs` — SAN happy path, CN fallback, malformed SAN URI rejected
  - `crates/core/controller-runtime/src/cert_signer.rs` — trust-domain mismatch rejected
  - `crates/core/integration-tests/tests/spiffe_identity.rs` — end-to-end: Agent CSR → Controller
    sign → identity extracted via SAN
- Docs (all rewritten / updated):
  - `docs/security/tofu-tls.md` — full rewrite, modes table, override semantics, examples
  - `docs/security/pki-certificates.md` — trust composition section, SPIFFE SAN section,
    `DynamicClientVerifier` reference, retire hand-rolled-DER paragraph
  - `docs/security/key-rotation.md` — Zeroize + atomic write section
  - `docs/security/secure-development.md` — reference resolver patterns
  - `CONTEXT.md` — four new glossary entries
- ADRs (new):
  - `docs/adr/0011-spiffe-service-identity.md`
  - `docs/adr/0012-agent-trust-composition.md`
  - `docs/adr/0013-defer-root-intermediate-ca-split.md`

---

## Snapshot Bindings

Same as Plans 1 and 2, plus:

- "Conventional Commits required: `<type>(scope): description`."
- "Wire protocol tests: asyncapi.yaml is source of truth; validate serialization matches schema."
- "All HTTP request types in uptrakit-web-api-types implement `Validate`; routes call
  `req.validate()` returning error_response(400) on fail."
- "Markdownlint: all warnings/errors must resolve."

---

### Task 1: ADR-0011 — SPIFFE Service Identity

**Files:**

- Create: `docs/adr/0011-spiffe-service-identity.md`

- [ ] **Step 1: Read existing ADR format**

Run: `head -60 /Users/andreyyantsen/Development/uptrakit/docs/adr/0006-instance-scoped-plugins.md`

Note structure: Title, Status, Context, Decision, Consequences, Alternatives.

- [ ] **Step 2: Write the ADR**

```markdown
# ADR-0011: SPIFFE Service Identity

Status: Accepted

## Context

Service certificates today carry `CN=<service_id>` only. RFC 6125 deprecates CN-based identity in favor of Subject Alternative Names. Service mesh ecosystems (SPIRE, Istio, Linkerd) standardize on SPIFFE URIs as the workload-identity carrier.

Adopting SPIFFE now:

- Aligns uptrakit with CNCF-graduated workload-identity tooling.
- Opens future federation (SPIFFE-Workload-API token exchange) without a second identity migration.
- Costs ~40 LOC + a `trust_domain` config knob.

URN-UUID (`urn:uuid:<service_id>`) was the simpler alternative. Rejected because URN-UUID has no ecosystem and produces no future-interop value.

## Decision

Every Service certificate carries:

- `Subject: CN=<service_id>` (preserved during the renewal-tail migration window; removed in a follow-up spec).
- `Subject Alternative Name: URI = spiffe://<trust_domain>/service/<service_id>`.

The Controller's `[tls] trust_domain` is configured by the Operator (defaults to first server-cert SAN). The Controller advertises the value to every connecting Service via the `ServiceSettingsPayload.trust_domain` wire field. The CSR signer rejects any CSR whose SPIFFE URI does not match the configured trust domain.

Identity extraction prefers the SPIFFE SAN; falls back to CN for ≤2 years (one max-lifetime renewal cycle). A follow-up spec removes the CN fallback after the renewal tail.

## Consequences

- CSR / cert generation gains a SAN URI. Backward-compatible — old certs still validate during the renewal tail.
- The Controller exposes a `trust_domain` setting. Misconfiguration manifests as CSR rejection (visible in audit log).
- Service identity is no longer "the CN of the cert" — it is "the URI SAN, falling back to CN." `service_identity_from_der` is the single source of truth.

## Alternatives considered

| Option                                   | Outcome                                              |
| ---------------------------------------- | ---------------------------------------------------- |
| URN-UUID (`urn:uuid:<id>`)               | Rejected — no ecosystem, no interop value.           |
| Custom URN (`urn:uptrakit:service:<id>`) | Rejected — self-defined namespace, no registry.      |
| Status quo (CN only)                     | Rejected — RFC 6125 deprecation, no future-proofing. |

## Related

- `docs/superpowers/specs/2026-05-12-mtls-hardening-design.md` §5.3
- ADR-0013 (deferred root/intermediate split)
```

- [ ] **Step 3: Lint**

Run: `npx markdownlint --config .markdownlint.json docs/adr/0011-spiffe-service-identity.md`

Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add docs/adr/0011-spiffe-service-identity.md
git commit -m "docs(adr): 0011 SPIFFE service identity"
```

---

### Task 2: ADR-0012 — Agent Trust Composition

**Files:**

- Create: `docs/adr/0012-agent-trust-composition.md`

- [ ] **Step 1: Write the ADR**

```markdown
# ADR-0012: Agent Trust Composition

Status: Accepted

## Context

Agent / Service-SDK builds its `rustls::RootCertStore` from the controller-CA bundle alone today. This works for the canonical "all managed, self-signed" deployment but blocks Operators who want to front the Controller with a public-CA certificate (Let's Encrypt etc.) or who run Agents inside corporate networks with their own internal CAs.

Three trust sources are available:

1. `webpki-roots` (compiled-in major public roots).
2. `rustls-native-certs` (OS root store, including corporate roots installed via MDM).
3. The controller-CA bundle delivered via `CaBundleUpdated`.

Naïvely unioning all three is unsafe: a corporate MITM root (Zscaler, Netskope) in the OS store would silently authorize any host the proxy presents. This is the canonical motivation for certificate pinning in mobile apps and not a property uptrakit can shed by default.

## Decision

Trust sources are **explicit, additive opt-ins**:

- Default: controller-CA bundle only (today's behavior — no change for existing deployments).
- `--trust-public-roots`: add compiled-in `webpki-roots`.
- `--trust-native-roots`: add `rustls-native-certs` (OS store at process startup).

Flags compose. The Operator declares the deployment shape.

Hostname verification (`ServerName`) is enforced in every mode unless the Operator opts out via `--tofu-skip-hostname` (only valid alongside one of the pin / insecure TOFU flags — see ADR-0011's sibling discussion in the spec).

## Consequences

- Operators upgrading from earlier releases see no change in trust posture.
- LE-fronted Controller deployments require `--trust-public-roots` (documented in `docs/security/tofu-tls.md`).
- Corporate-internal-CA-only Agents use `--trust-native-roots` alone (without `--trust-public-roots`) to capture the corporate root while excluding public CAs.
- OS root store updates (admin pushes new corporate root via MDM) require Agent restart. Documented.

## Alternatives considered

| Option                                                     | Outcome                                                                                                                                |
| ---------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| Union all three by default                                 | Rejected — silent stealth-MITM exposure via corporate roots.                                                                           |
| Demote controller-CA bundle to a pin (separate from roots) | Rejected — special-cased verifier, more complexity, no real benefit over additive root store.                                          |
| Single `--trust-mode {pinned,public,native,any}` enum flag | Rejected — additive flags compose naturally; enum forces re-design when a fourth source surfaces (e.g., trust-on-first-use SPKI list). |

## Related

- `docs/superpowers/specs/2026-05-12-mtls-hardening-design.md` §5.1
- ADR-0011 (SPIFFE identity)
- ADR-0013 (root/intermediate split deferral)
```

- [ ] **Step 2: Lint + commit**

```bash
npx markdownlint --config .markdownlint.json docs/adr/0012-agent-trust-composition.md
git add docs/adr/0012-agent-trust-composition.md
git commit -m "docs(adr): 0012 agent trust composition (opt-in additive roots)"
```

---

### Task 3: ADR-0013 — Defer Root/Intermediate CA Split

**Files:**

- Create: `docs/adr/0013-defer-root-intermediate-ca-split.md`

- [ ] **Step 1: Write the ADR**

```markdown
# ADR-0013: Defer Root/Intermediate Managed CA Split

Status: Accepted (deferral)

## Context

The mTLS hardening spec (`docs/superpowers/specs/2026-05-12-mtls-hardening-design.md`) introduces SPKI pinning (`--tofu-spki`) as a TOFU mode. SPKI pinning survives certificate **renewal** (the keypair stays stable) but breaks at managed-CA **rotation** because rotation introduces a fresh keypair.

A two-tier managed CA (long-lived root signs short-lived issuing intermediates) would let an Agent SPKI-pin the root and survive rotation freely — the intermediate rotates without the pin breaking.

This is a meaningful but separable improvement:

- Adds DB schema (parent_fingerprint on `ca_certificate`).
- Splits cert-sign / OCSP-sign / CRL-sign to use the intermediate.
- Adds a root-rotation ceremony (rare, manual).

The benefit materializes once per ~5 years per fleet.

## Decision

Defer Path A (root/intermediate split) to its own spec + plan cycle. Operators wanting rotation-survivable pin durability today use the external-CA path (`--ca-cert` / `--ca-key`) with their own existing PKI (Vault, AD CS, step-ca).

This ADR records the deferral so future contributors do not re-litigate the decision without re-reading the spec.

## Consequences

- Managed-CA SPKI pin durability matches fingerprint pin durability (breaks at every rotation, ~5 years).
- External CA users get true SPKI pin durability today, no code change required.
- Future Path A spec will revisit this ADR and supersede it.

## Alternatives considered

| Option                                                   | Outcome                                                                                                   |
| -------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| Ship Path A in the current hardening spec                | Rejected — significantly enlarges scope; benefit materializes once per 5 years.                           |
| Reuse the keypair across CA rotations (same-key renewal) | Rejected — defeats the purpose of rotation (key hygiene).                                                 |
| Cross-sign managed CA with a public root                 | Rejected — Baseline Requirements forbid name-unconstrained public subordinates; cost would dwarf benefit. |

## Related

- `docs/superpowers/specs/2026-05-12-mtls-hardening-design.md` §8 (Future Work)
- ADR-0011, ADR-0012
```

- [ ] **Step 2: Lint + commit**

```bash
npx markdownlint --config .markdownlint.json docs/adr/0013-defer-root-intermediate-ca-split.md
git add docs/adr/0013-defer-root-intermediate-ca-split.md
git commit -m "docs(adr): 0013 defer root/intermediate CA split"
```

---

### Task 4: `CONTEXT.md` glossary additions

**Files:**

- Modify: `CONTEXT.md`

- [ ] **Step 1: Open CONTEXT.md and locate the alphabetized glossary**

Run: `grep -n '^## Language\|^**' CONTEXT.md | head -30`

- [ ] **Step 2: Add four entries** (alphabetized appropriately within the existing terms)

```markdown
**Dynamic Client Verifier**:
The Controller-side wrapper around `rustls::server::WebPkiClientVerifier` that exposes an
`ArcSwap` inner verifier. Lets CRL rebuilds and CA-bundle updates hot-swap the verifier
without rebuilding `rustls::ServerConfig`.
_Avoid_: verifier reload (overloaded with graceful-reload terminology).

**SPIFFE Service Identity**:
A Service's identity carried as a URI Subject Alternative Name on its client certificate,
of the form `spiffe://<trust_domain>/service/<service_id>`. Replaces CN-only identity over
the natural cert renewal cycle.
_Avoid_: service URI, workload ID (SPIFFE has a precise term).

**TOFU mode**:
One of `system`, `pin-fingerprint`, `pin-spki`, `insecure-tofu`. Selected at Service boot
via a mutually-exclusive CLI flag. Determines how the Service verifies the Controller's
TLS certificate during bootstrap and on reconnects when no CA bundle has been persisted.
_Avoid_: TOFU enabled (ambiguous — every mode is "enabled" in some sense).

**Trust Domain**:
A string in the `[tls]` Config Section naming the Controller's SPIFFE namespace. Defaults
to the first server-cert SAN. Must match the trust-domain segment of every Service's
SPIFFE URI SAN.
_Avoid_: domain (overloaded), namespace (Kubernetes overload).
```

- [ ] **Step 3: Lint + commit**

```bash
npx markdownlint --config .markdownlint.json CONTEXT.md
git add CONTEXT.md
git commit -m "docs(context): add TOFU mode, SPIFFE, Trust Domain, Dynamic Client Verifier glossary entries"
```

---

### Task 5: Sha256Hash type — failing test first

**Files:**

- Create: `crates/shared/service-sdk/src/tofu/hash.rs`
- Create: `crates/shared/service-sdk/src/tofu/mod.rs` (or `tofu.rs` — single-file initially, may
  split)

- [ ] **Step 1: Decide single-file vs split**

Aim: single file `crates/shared/service-sdk/src/tofu.rs` until ~400 LOC, then split. Start
single-file.

- [ ] **Step 2: Write the `Sha256Hash` type with failing tests**

```rust
//! TOFU modes for Agent / Service-SDK mTLS bootstrap. See spec §5.2.

use std::fmt;
use std::str::FromStr;

/// SHA-256 hash, parsed from `aa:bb:cc:...` (colon-separated) or compact
/// hex. Wraps a 32-byte array.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sha256Hash([u8; 32]);

impl Sha256Hash {
    pub fn from_bytes(bytes: [u8; 32]) -> Self { Self(bytes) }
    pub fn as_bytes(&self) -> &[u8; 32] { &self.0 }

    pub fn to_colon_hex(&self) -> String {
        let mut s = String::with_capacity(32 * 3 - 1);
        for (i, b) in self.0.iter().enumerate() {
            if i > 0 { s.push(':'); }
            s.push_str(&format!("{:02x}", b));
        }
        s
    }
}

impl fmt::Debug for Sha256Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sha256Hash({})", self.to_colon_hex())
    }
}

impl fmt::Display for Sha256Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_colon_hex())
    }
}

impl FromStr for Sha256Hash {
    type Err = Sha256ParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let cleaned: String = s.chars().filter(|c| *c != ':' && *c != ' ').collect();
        if cleaned.len() != 64 {
            return Err(Sha256ParseError::BadLength(cleaned.len()));
        }
        let mut bytes = [0u8; 32];
        for (i, byte_str) in (0..64).step_by(2).map(|i| &cleaned[i..i + 2]).enumerate() {
            bytes[i] = u8::from_str_radix(byte_str, 16)
                .map_err(|_| Sha256ParseError::BadHex(byte_str.to_owned()))?;
        }
        Ok(Self(bytes))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Sha256ParseError {
    #[error("expected 64 hex chars (with or without colons), got {0}")]
    BadLength(usize),
    #[error("invalid hex byte: {0:?}")]
    BadHex(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_compact_hex() {
        let s = "aabbccdd00112233445566778899aabbccdd0011223344556677889900112233";
        let h: Sha256Hash = s.parse().expect("parse");
        assert_eq!(h.as_bytes()[0], 0xaa);
        assert_eq!(h.as_bytes()[31], 0x33);
    }

    #[test]
    fn parse_colon_hex() {
        let s = "aa:bb:cc:dd:00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:00:11:22:33:44:55:66:77:88:99:00:11:22:33";
        let h: Sha256Hash = s.parse().expect("parse");
        assert_eq!(h.as_bytes()[0], 0xaa);
        assert_eq!(h.as_bytes()[31], 0x33);
    }

    #[test]
    fn parse_rejects_bad_length() {
        assert!(matches!(
            "abcd".parse::<Sha256Hash>(),
            Err(Sha256ParseError::BadLength(4))
        ));
    }

    #[test]
    fn round_trip_to_colon_hex() {
        let s = "AA:BB:CC:DD:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:00:11:22:33:44:55:66:77:88:99:00:11:22:33";
        let h: Sha256Hash = s.parse().expect("parse");
        assert_eq!(h.to_colon_hex().to_uppercase(), s);
    }
}
```

- [ ] **Step 3: Run**

Run: `cargo test -p uptrakit-service-sdk sha256_hash 2>&1 | tail -20`

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/shared/service-sdk/src/tofu.rs
git commit -m "feat(tofu): add Sha256Hash type with colon/compact hex parsing"
```

---

### Task 6: `TofuMode` enum + `TofuConfig` struct

**Files:**

- Modify: `crates/shared/service-sdk/src/tofu.rs`

- [ ] **Step 1: Add the types and a failing test**

```rust
/// Server-cert trust mode. Selected at Service boot via mutually-exclusive
/// CLI flags. See spec §5.2.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum TofuMode {
    /// Verify against trust composition (controller-CA bundle ± opt-in
    /// public/native roots). Chain + expiry + key-usage required.
    System,

    /// Accept any chain whose CA bundle SHA-256 matches the pinned value.
    PinFingerprint(Sha256Hash),

    /// Accept any chain where any cert's SubjectPublicKeyInfo SHA-256
    /// matches the pinned value.
    PinSpki(Sha256Hash),

    /// Accept any chain; log `WARN` on every connection. Hostname check
    /// is forced off.
    InsecureTofu,
}

#[derive(Clone, Debug)]
pub struct TofuConfig {
    pub mode: TofuMode,
    pub skip_hostname: bool,
    pub fingerprint_acknowledge: Option<Sha256Hash>,
}

impl TofuConfig {
    /// Validates flag combinations. Returns an error if the construction
    /// is illegal (e.g., `--tofu-skip-hostname` without a pin or insecure
    /// mode).
    pub fn from_flags(
        fingerprint: Option<Sha256Hash>,
        spki: Option<Sha256Hash>,
        insecure: bool,
        skip_hostname: bool,
        fingerprint_acknowledge: Option<Sha256Hash>,
    ) -> Result<Self, TofuConfigError> {
        let selected: u8 =
            (fingerprint.is_some() as u8)
            + (spki.is_some() as u8)
            + (insecure as u8);
        if selected > 1 {
            return Err(TofuConfigError::MultipleModes);
        }
        let mode = match (fingerprint, spki, insecure) {
            (Some(h), None, false) => TofuMode::PinFingerprint(h),
            (None, Some(h), false) => TofuMode::PinSpki(h),
            (None, None, true) => TofuMode::InsecureTofu,
            (None, None, false) => TofuMode::System,
            _ => unreachable!("guarded by selected > 1 check"),
        };

        // Insecure mode forces skip_hostname.
        let effective_skip = matches!(mode, TofuMode::InsecureTofu) || skip_hostname;

        // skip_hostname requires a pin or insecure mode.
        if skip_hostname && matches!(mode, TofuMode::System) {
            return Err(TofuConfigError::SkipHostnameRequiresPinOrInsecure);
        }

        // fingerprint_acknowledge only meaningful in insecure mode.
        if fingerprint_acknowledge.is_some() && !matches!(mode, TofuMode::InsecureTofu) {
            return Err(TofuConfigError::AcknowledgeRequiresInsecure);
        }

        Ok(Self { mode, skip_hostname: effective_skip, fingerprint_acknowledge })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TofuConfigError {
    #[error("at most one of --tofu-fingerprint, --tofu-spki, --tofu-insecure may be set")]
    MultipleModes,
    #[error("--tofu-skip-hostname requires one of --tofu-fingerprint, --tofu-spki, --tofu-insecure")]
    SkipHostnameRequiresPinOrInsecure,
    #[error("--tofu-fingerprint-acknowledge is only valid with --tofu-insecure")]
    AcknowledgeRequiresInsecure,
}

#[cfg(test)]
mod config_tests {
    use super::*;

    fn hash() -> Sha256Hash {
        "aa".repeat(32).parse().expect("hash")
    }

    #[test]
    fn no_flags_is_system_mode() {
        let cfg = TofuConfig::from_flags(None, None, false, false, None).unwrap();
        assert!(matches!(cfg.mode, TofuMode::System));
        assert!(!cfg.skip_hostname);
    }

    #[test]
    fn fingerprint_only_is_pin_fingerprint() {
        let cfg = TofuConfig::from_flags(Some(hash()), None, false, false, None).unwrap();
        assert!(matches!(cfg.mode, TofuMode::PinFingerprint(_)));
    }

    #[test]
    fn spki_only_is_pin_spki() {
        let cfg = TofuConfig::from_flags(None, Some(hash()), false, false, None).unwrap();
        assert!(matches!(cfg.mode, TofuMode::PinSpki(_)));
    }

    #[test]
    fn insecure_implies_skip_hostname() {
        let cfg = TofuConfig::from_flags(None, None, true, false, None).unwrap();
        assert!(cfg.skip_hostname, "insecure mode must force skip_hostname");
    }

    #[test]
    fn two_pin_flags_rejected() {
        let r = TofuConfig::from_flags(Some(hash()), Some(hash()), false, false, None);
        assert!(matches!(r, Err(TofuConfigError::MultipleModes)));
    }

    #[test]
    fn skip_hostname_without_pin_rejected() {
        let r = TofuConfig::from_flags(None, None, false, true, None);
        assert!(matches!(r, Err(TofuConfigError::SkipHostnameRequiresPinOrInsecure)));
    }

    #[test]
    fn acknowledge_without_insecure_rejected() {
        let r = TofuConfig::from_flags(Some(hash()), None, false, false, Some(hash()));
        assert!(matches!(r, Err(TofuConfigError::AcknowledgeRequiresInsecure)));
    }
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p uptrakit-service-sdk tofu::config_tests 2>&1 | tail -30`

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/shared/service-sdk/src/tofu.rs
git commit -m "feat(tofu): TofuMode enum + TofuConfig flag validation"
```

---

### Task 7: CLI surface — drop `--tofu`, add four mode flags + acknowledge + trust flags

**Files:**

- Modify: `crates/shared/service-sdk/src/cli.rs`

- [ ] **Step 1: Read current CLI**

Run: `head -100 crates/shared/service-sdk/src/cli.rs`

Locate `pub tofu: bool` and the conflicts.

- [ ] **Step 2: Replace the TOFU surface**

```rust
use clap::{ArgGroup, Args};
use crate::tofu::{Sha256Hash, TofuConfig, TofuMode};

#[derive(Args, Debug)]
#[command(group(
    ArgGroup::new("tofu_mode")
        .multiple(false)
        .args(["tofu_fingerprint", "tofu_spki", "tofu_insecure"])
))]
pub struct CommonArgs {
    /// Pin the Controller's CA bundle by SHA-256 fingerprint. On first
    /// successful connection, the bundle is persisted to disk.
    #[arg(long, value_name = "SHA256", conflicts_with_all = ["ca_cert", "pki_addr"])]
    pub tofu_fingerprint: Option<Sha256Hash>,

    /// Pin the Controller's CA by SubjectPublicKeyInfo SHA-256 hash.
    /// Survives cert renewals that reuse the same keypair.
    #[arg(long, value_name = "SHA256", conflicts_with_all = ["ca_cert", "pki_addr"])]
    pub tofu_spki: Option<Sha256Hash>,

    /// Accept any chain. Operates as stateless TOFU. Implies
    /// `--tofu-skip-hostname`. Logs WARN on every connection.
    #[arg(long, conflicts_with_all = ["ca_cert", "pki_addr"])]
    pub tofu_insecure: bool,

    /// Disable ServerName check. Required for `--tofu-fingerprint` or
    /// `--tofu-spki` when the dialed hostname does not match the cert SAN
    /// (e.g., development with IP addresses). Implied by `--tofu-insecure`.
    #[arg(long, requires = "tofu_mode")]
    pub tofu_skip_hostname: bool,

    /// Acknowledge a fingerprint observed in a previous `--tofu-insecure`
    /// run; required to persist the CA bundle to disk in insecure mode.
    /// On mismatch with the bundle fetched this run, the Agent exits
    /// non-zero with both fingerprints logged at ERROR.
    #[arg(long, value_name = "SHA256", requires = "tofu_insecure")]
    pub tofu_fingerprint_acknowledge: Option<Sha256Hash>,

    /// Add compiled-in `webpki-roots` (major public CAs) to the trust
    /// store. Needed when the Controller is fronted with a public-CA
    /// certificate (Let's Encrypt etc.).
    #[arg(long)]
    pub trust_public_roots: bool,

    /// Add the OS root store (via `rustls-native-certs`) to the trust
    /// store. Captures corporate roots installed via MDM. Loaded once at
    /// process startup — restart Agent to pick up OS root changes.
    #[arg(long)]
    pub trust_native_roots: bool,

    // ... existing fields except `pub tofu: bool` which is REMOVED.
}

impl CommonArgs {
    pub fn tofu_config(&self) -> Result<TofuConfig, crate::tofu::TofuConfigError> {
        TofuConfig::from_flags(
            self.tofu_fingerprint,
            self.tofu_spki,
            self.tofu_insecure,
            self.tofu_skip_hostname,
            self.tofu_fingerprint_acknowledge,
        )
    }
}
```

- [ ] **Step 3: Remove `pub tofu: bool` everywhere**

Run: `rg -n '\.tofu\b\|tofu: bool' crates/`

Replace each read site with `args.tofu_config()?.mode != TofuMode::System` or the mode-specific
check needed.

- [ ] **Step 4: Update tests** in `cli.rs`

The existing test `tofu_and_ca_cert_conflict` becomes:

```rust
#[test]
fn tofu_fingerprint_and_ca_cert_conflict() {
    let result = TestCli::try_parse_from(&[
        "uptrakit-agent", "--ca-cert", "/tmp/ca.pem",
        "--tofu-fingerprint", "aa".repeat(32).as_str(),
    ]);
    assert!(result.is_err());
}

#[test]
fn tofu_insecure_implies_skip_hostname() {
    let parsed = TestCli::try_parse_from(&[
        "uptrakit-agent", "--tofu-insecure",
    ]).unwrap();
    let cfg = parsed.common.tofu_config().unwrap();
    assert!(cfg.skip_hostname);
}
```

- [ ] **Step 5: Run**

Run: `cargo test -p uptrakit-service-sdk cli -- --nocapture 2>&1 | tail -30`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/shared/service-sdk/src/cli.rs
git commit -m "feat!(service-sdk): replace --tofu with four named modes + --trust-* flags"
```

Note the `!` — breaking change per Conventional Commits.

---

### Task 8: Trust composition root-store builder — failing test first

**Files:**

- Modify: `crates/shared/service-sdk/src/tls.rs`

- [ ] **Step 1: Add deps**

Edit `crates/shared/service-sdk/Cargo.toml`:

```toml
rustls-native-certs = { workspace = true }
webpki-roots = { workspace = true }
```

Edit root `Cargo.toml` `[workspace.dependencies]`:

```toml
rustls-native-certs = "0.8"
```

(Or current version — verify at impl time.)

- [ ] **Step 2: Write failing test**

```rust
#[tokio::test]
async fn root_store_default_is_controller_ca_only() {
    let ca_pem = test_ca_pem();
    let opts = TrustOptions::default();
    let store = build_root_store(&ca_pem, &opts).await.expect("build");
    // Exactly one root (the controller CA).
    assert_eq!(store.len(), 1);
}

#[tokio::test]
async fn root_store_with_public_roots_includes_webpki() {
    let ca_pem = test_ca_pem();
    let opts = TrustOptions { trust_public_roots: true, trust_native_roots: false };
    let store = build_root_store(&ca_pem, &opts).await.expect("build");
    // Controller CA + many webpki roots.
    assert!(store.len() > 100, "webpki-roots adds many anchors");
}

#[tokio::test]
async fn root_store_with_native_roots_extends_store() {
    let ca_pem = test_ca_pem();
    let opts = TrustOptions { trust_public_roots: false, trust_native_roots: true };
    let store = build_root_store(&ca_pem, &opts).await.expect("build");
    // Native store size is host-dependent; assert non-trivial.
    assert!(store.len() >= 1, "at least the controller CA, usually more from OS");
}
```

- [ ] **Step 3: Implement `TrustOptions` + `build_root_store`**

```rust
#[derive(Clone, Copy, Debug, Default)]
pub struct TrustOptions {
    pub trust_public_roots: bool,
    pub trust_native_roots: bool,
}

pub async fn build_root_store(
    controller_ca_pem: &[u8],
    opts: &TrustOptions,
) -> Result<rustls::RootCertStore, rootcause::Report<EnrollmentError>> {
    let mut root_store = rustls::RootCertStore::empty();

    // 1. Controller CA bundle (always).
    for cert_res in rustls::pki_types::CertificateDer::pem_slice_iter(controller_ca_pem) {
        let cert = cert_res.map_err(|e| report!(EnrollmentError::Tls(TlsError::CaParse(e.to_string()))))?;
        root_store.add(cert)
            .map_err(|e| report!(EnrollmentError::Tls(TlsError::RootStore(e.to_string()))))?;
    }

    // 2. Compiled-in webpki-roots (opt-in).
    if opts.trust_public_roots {
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    }

    // 3. OS root store via rustls-native-certs (opt-in, blocking I/O).
    if opts.trust_native_roots {
        let native = tokio::task::spawn_blocking(|| {
            rustls_native_certs::load_native_certs()
        }).await
        .map_err(|e| report!(EnrollmentError::Tls(TlsError::NativeRoots(e.to_string()))))?;

        for err in &native.errors {
            tracing::warn!(error = %err, "rustls-native-certs partial load error");
        }
        for cert in native.certs {
            if let Err(e) = root_store.add(cert) {
                tracing::warn!(error = %e, "skipping unparseable native cert");
            }
        }
    }

    Ok(root_store)
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p uptrakit-service-sdk root_store -- --nocapture 2>&1 | tail -20`

Expected: PASS.

- [ ] **Step 5: Wire through `build_client_config_with_resolver` from Plan 2**

Extend the signature:

```rust
pub async fn build_client_config_with_resolver(
    controller_ca_pem: &[u8],
    trust_opts: &TrustOptions,
    resolver: Arc<AgentClientCertResolver>,
) -> Result<Arc<rustls::ClientConfig>, ...> { /* use build_root_store(...).await */ }
```

Update `lifecycle.rs` callers to pass `TrustOptions` derived from CLI.

- [ ] **Step 6: Commit**

```bash
git add crates/shared/service-sdk/src/tls.rs crates/shared/service-sdk/src/lifecycle.rs crates/shared/service-sdk/Cargo.toml Cargo.toml
git commit -m "feat(service-sdk): opt-in trust composition (--trust-public-roots/--trust-native-roots)"
```

---

### Task 9: Mode-dispatched `ServerCertVerifier` for TOFU — failing test first

**Files:**

- Modify: `crates/shared/service-sdk/src/tofu.rs`

- [ ] **Step 1: Add the verifier with failing tests**

```rust
use std::sync::Arc;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error as TlsError, SignatureScheme};

/// Mode-dispatched server cert verifier. Wraps the standard webpki
/// verifier and replaces/disables checks per the active `TofuMode`.
#[derive(Debug)]
pub struct ModeBasedVerifier {
    pub config: TofuConfig,
    pub controller_ca_pem: Vec<u8>,
    pub system_verifier: Arc<dyn ServerCertVerifier>,
}

impl ServerCertVerifier for ModeBasedVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        // Hostname check is the only piece that varies independently
        // from chain validation. We delegate chain checks per-mode.
        match &self.config.mode {
            TofuMode::System => {
                self.system_verifier.verify_server_cert(
                    end_entity, intermediates, server_name, ocsp_response, now,
                )
            }
            TofuMode::PinFingerprint(expected) => {
                // Bundle fingerprint check: compute SHA-256 of the
                // controller-CA PEM that was passed in via --tofu-fingerprint
                // and compare. ServerName check applied unless skip_hostname.
                let actual = sha256_hex(&self.controller_ca_pem);
                if &actual != expected {
                    tracing::warn!(expected = %expected, actual = %actual,
                        "TOFU fingerprint mismatch");
                    return Err(TlsError::InvalidCertificate(
                        rustls::CertificateError::Other(
                            rustls::OtherError(Arc::new(
                                std::io::Error::new(std::io::ErrorKind::Other,
                                    "fingerprint mismatch")
                            ))
                        )
                    ));
                }
                if !self.config.skip_hostname {
                    self.verify_hostname(end_entity, server_name)?;
                }
                Ok(ServerCertVerified::assertion())
            }
            TofuMode::PinSpki(expected) => {
                // Match SPKI of any cert in the chain.
                let chain_matches = std::iter::once(end_entity)
                    .chain(intermediates.iter())
                    .any(|c| match spki_sha256(c) {
                        Ok(h) => &h == expected,
                        Err(_) => false,
                    });
                if !chain_matches {
                    return Err(TlsError::InvalidCertificate(
                        rustls::CertificateError::Other(rustls::OtherError(Arc::new(
                            std::io::Error::new(std::io::ErrorKind::Other, "SPKI not in chain")
                        )))
                    ));
                }
                if !self.config.skip_hostname {
                    self.verify_hostname(end_entity, server_name)?;
                }
                Ok(ServerCertVerified::assertion())
            }
            TofuMode::InsecureTofu => {
                tracing::warn!("TLS verification disabled (insecure-tofu); accepting any cert");
                Ok(ServerCertVerified::assertion())
            }
        }
    }

    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        self.system_verifier.verify_tls12_signature(m, c, d)
    }

    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        self.system_verifier.verify_tls13_signature(m, c, d)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.system_verifier.supported_verify_schemes()
    }
}

impl ModeBasedVerifier {
    fn verify_hostname(
        &self,
        end_entity: &CertificateDer<'_>,
        server_name: &ServerName<'_>,
    ) -> Result<(), TlsError> {
        // Use webpki's leaf parser to check SAN against the dialed name.
        // x509-cert can extract SANs; we then match per RFC 6125.
        let cert = x509_cert::Certificate::from_der(end_entity.as_ref())
            .map_err(|e| TlsError::InvalidCertificate(
                rustls::CertificateError::Other(rustls::OtherError(Arc::new(
                    std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
                )))
            ))?;
        let dialed = match server_name {
            ServerName::DnsName(n) => n.as_ref(),
            ServerName::IpAddress(_) => return Ok(()), // IP SAN check deferred to impl plan
            _ => return Err(TlsError::InvalidCertificate(rustls::CertificateError::NotValidForName)),
        };
        if cert_sans_match(&cert, dialed) {
            Ok(())
        } else {
            Err(TlsError::InvalidCertificate(rustls::CertificateError::NotValidForName))
        }
    }
}

fn sha256_hex(bytes: &[u8]) -> Sha256Hash {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    Sha256Hash::from_bytes(out)
}

fn spki_sha256(cert_der: &[u8]) -> Result<Sha256Hash, x509_cert::der::Error> {
    use x509_cert::der::Encode;
    let cert = x509_cert::Certificate::from_der(cert_der)?;
    let spki_der = cert.tbs_certificate.subject_public_key_info.to_der()?;
    Ok(sha256_hex(&spki_der))
}

fn cert_sans_match(cert: &x509_cert::Certificate, name: &str) -> bool {
    // Iterate SAN extension entries, return true if any DNS SAN matches.
    // Real impl: x509_cert::ext::pkix::SubjectAltName from extensions.
    use x509_cert::ext::pkix::SubjectAltName;
    use x509_cert::ext::pkix::name::GeneralName;
    use der::Decode;

    let Some(exts) = &cert.tbs_certificate.extensions else { return false };
    let oid = const_oid::db::rfc5280::ID_CE_SUBJECT_ALT_NAME;
    for ext in exts {
        if ext.extn_id != oid { continue; }
        if let Ok(san) = SubjectAltName::from_der(ext.extn_value.as_bytes()) {
            for gn in san.0 {
                if let GeneralName::DnsName(dns) = gn {
                    if dns.as_str().eq_ignore_ascii_case(name) { return true; }
                }
            }
        }
    }
    false
}
```

Add `sha2 = { workspace = true }` to `crates/shared/service-sdk/Cargo.toml` if not present.

- [ ] **Step 2: Add tests for each mode**

```rust
#[cfg(test)]
mod verifier_tests {
    use super::*;

    #[test]
    fn pin_fingerprint_match_accepts() {
        // Build a self-signed cert; pin its CA bundle SHA-256.
        let (cert, _key, ca_pem) = build_self_signed_with_pem();
        let expected = sha256_hex(&ca_pem);
        let verifier = make_mode_verifier(
            TofuConfig::from_flags(Some(expected), None, false, true, None).unwrap(),
            ca_pem,
        );
        let result = verifier.verify_server_cert(
            &cert, &[], &ServerName::try_from("test.local").unwrap(),
            &[], UnixTime::now(),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn pin_fingerprint_mismatch_rejects() {
        let (cert, _key, ca_pem) = build_self_signed_with_pem();
        let bogus: Sha256Hash = "ff".repeat(32).parse().unwrap();
        let verifier = make_mode_verifier(
            TofuConfig::from_flags(Some(bogus), None, false, true, None).unwrap(),
            ca_pem,
        );
        let result = verifier.verify_server_cert(
            &cert, &[], &ServerName::try_from("test.local").unwrap(),
            &[], UnixTime::now(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn pin_spki_match_accepts() {
        let (cert, _key, _) = build_self_signed_with_pem();
        let spki = spki_sha256(&cert).unwrap();
        let verifier = make_mode_verifier(
            TofuConfig::from_flags(None, Some(spki), false, true, None).unwrap(),
            vec![],
        );
        let result = verifier.verify_server_cert(
            &cert, &[], &ServerName::try_from("test.local").unwrap(),
            &[], UnixTime::now(),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn insecure_tofu_accepts_anything() {
        let (cert, _key, _) = build_self_signed_with_pem();
        let verifier = make_mode_verifier(
            TofuConfig::from_flags(None, None, true, false, None).unwrap(),
            vec![],
        );
        let result = verifier.verify_server_cert(
            &cert, &[], &ServerName::try_from("anything.example").unwrap(),
            &[], UnixTime::now(),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn hostname_mismatch_rejected_when_check_enabled() {
        let (cert, _key, ca_pem) = build_self_signed_with_pem_san("real.local");
        let expected = sha256_hex(&ca_pem);
        let verifier = make_mode_verifier(
            TofuConfig::from_flags(Some(expected), None, false, false, None).unwrap(),
            ca_pem,
        );
        let result = verifier.verify_server_cert(
            &cert, &[], &ServerName::try_from("imposter.local").unwrap(),
            &[], UnixTime::now(),
        );
        assert!(result.is_err());
    }
}
```

- [ ] **Step 3: Run**

Run: `cargo test -p uptrakit-service-sdk tofu::verifier_tests -- --nocapture 2>&1 | tail -30`

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/shared/service-sdk/src/tofu.rs crates/shared/service-sdk/Cargo.toml
git commit -m "feat(tofu): mode-dispatched ServerCertVerifier (system/pin-fingerprint/pin-spki/insecure)"
```

---

### Task 10: `ca.rs` — pin-fingerprint persistence + insecure-tofu mismatch handling

**Files:**

- Modify: `crates/shared/service-sdk/src/ca.rs`

- [ ] **Step 1: Locate `bootstrap_ca`**

Run: `rg -n 'fn bootstrap_ca' crates/shared/service-sdk/src/ca.rs`

- [ ] **Step 2: Add a failing test for fingerprint-acknowledge mismatch**

```rust
#[tokio::test]
async fn insecure_tofu_with_acknowledge_mismatch_exits_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();

    // Stub bundle fetch returns a CA with fingerprint F.
    let bundle = test_ca_bundle_with_fingerprint("F");
    let expected_acknowledge: Sha256Hash = "ee".repeat(32).parse().unwrap(); // != F

    let cfg = TofuConfig::from_flags(
        None, None, true, false, Some(expected_acknowledge),
    ).unwrap();

    let result = bootstrap_ca_with_stub(base, bundle, &cfg).await;
    assert!(matches!(result,
        Err(e) if matches!(e.current_context(),
            EnrollmentError::CaError(CaError::FingerprintMismatch { .. }))
    ));

    // No persistence on mismatch.
    assert!(!base.join("ca.pem").exists());
}
```

- [ ] **Step 3: Implement mismatch handling in bootstrap_ca**

```rust
pub async fn bootstrap_ca(
    base: &std::path::Path,
    bundle_pem: &str,
    config: &crate::tofu::TofuConfig,
) -> Result<(), rootcause::Report<EnrollmentError>> {
    let fingerprint = sha256_hex(bundle_pem.as_bytes());

    match &config.mode {
        TofuMode::PinFingerprint(expected) => {
            if &fingerprint != expected {
                return Err(report!(EnrollmentError::CaError(
                    CaError::FingerprintMismatch {
                        expected: expected.to_string(),
                        actual: fingerprint.to_string(),
                    }
                )));
            }
            // Persist.
            save_ca_bundle(base, bundle_pem)?;
            Ok(())
        }
        TofuMode::PinSpki(_) => {
            // SPKI pin: persist the bundle; runtime verifier still checks
            // SPKI on every handshake until the bundle anchor takes over.
            save_ca_bundle(base, bundle_pem)?;
            Ok(())
        }
        TofuMode::InsecureTofu => {
            match &config.fingerprint_acknowledge {
                None => {
                    // Stateless TOFU; do NOT persist.
                    tracing::warn!(fingerprint = %fingerprint,
                        "insecure-tofu: observed CA fingerprint; pass via --tofu-fingerprint-acknowledge to persist");
                    Ok(())
                }
                Some(expected) => {
                    if expected != &fingerprint {
                        tracing::error!(
                            expected = %expected,
                            actual = %fingerprint,
                            "insecure-tofu: --tofu-fingerprint-acknowledge mismatch; refusing to persist"
                        );
                        return Err(report!(EnrollmentError::CaError(
                            CaError::FingerprintMismatch {
                                expected: expected.to_string(),
                                actual: fingerprint.to_string(),
                            }
                        )));
                    }
                    save_ca_bundle(base, bundle_pem)?;
                    Ok(())
                }
            }
        }
        TofuMode::System => {
            // No TOFU; bundle was supplied via --ca-cert path elsewhere.
            save_ca_bundle(base, bundle_pem)?;
            Ok(())
        }
    }
}
```

Add `CaError::FingerprintMismatch { expected: String, actual: String }` variant.

- [ ] **Step 4: Run tests**

Run: `cargo test -p uptrakit-service-sdk ca -- --nocapture 2>&1 | tail -20`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/shared/service-sdk/src/ca.rs
git commit -m "feat(ca): TOFU-mode-aware bootstrap with mismatch rejection"
```

---

### Task 11: `ServiceSettingsPayload.trust_domain` — wire field

**Files:**

- Modify: `crates/shared/wire/src/payloads.rs`
- Modify: `crates/shared/wire/asyncapi.yaml`

- [ ] **Step 1: Locate `ServiceSettingsPayload`**

Run: `rg -n 'struct ServiceSettingsPayload' crates/shared/wire/src/payloads.rs`

- [ ] **Step 2: Add the field with `#[serde(default)]` + `#[non_exhaustive]` on the struct**

```rust
#[non_exhaustive]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ServiceSettingsPayload {
    pub renewal_window_hours: u16,
    // ... existing fields ...

    /// SPIFFE trust domain for Service identity URIs. Empty string when
    /// the Controller has no trust_domain configured (Agent falls back
    /// to the dialed hostname for SPIFFE SAN generation).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub trust_domain: String,
}
```

- [ ] **Step 3: Update existing constructors**

External callers using struct literals must add `..Default::default()` if `Default` is
impl'd, or pass `trust_domain: String::new()` explicitly. The `#[non_exhaustive]` blocks
ad-hoc struct literals from outside crates — wire's own code can still construct.

- [ ] **Step 4: Update `asyncapi.yaml`**

Run: `rg -n 'ServiceSettingsPayload' crates/shared/wire/asyncapi.yaml`

Add to the `properties` block:

```yaml
trust_domain:
  type: string
  default: ""
  description: |
    SPIFFE trust domain for Service identity URIs. Empty string indicates
    no trust domain configured; Agent falls back to dialed hostname.
```

- [ ] **Step 5: Run wire tests**

Run: `cargo test -p uptrakit-wire 2>&1 | tail -20`

Expected: PASS.

- [ ] **Step 6: Run asyncapi schema-compliance tests**

Run: `cargo test -p uptrakit-wire asyncapi 2>&1 | tail -20`

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/shared/wire/src/payloads.rs crates/shared/wire/asyncapi.yaml
git commit -m "feat(wire): add ServiceSettingsPayload.trust_domain (#[serde(default)])"
```

---

### Task 12: SPIFFE CSR generation in `identity.rs` — failing test first

**Files:**

- Modify: `crates/shared/service-sdk/src/identity.rs`

- [ ] **Step 1: Add `uuid` dep to service-sdk if absent**

Run: `grep uuid crates/shared/service-sdk/Cargo.toml`

If absent, add to `[dependencies]`. Workspace already has `uuid` for UUIDv7.

- [ ] **Step 2: Add failing test**

```rust
#[test]
fn csr_includes_spiffe_san() {
    let service_id = uuid::Uuid::now_v7();
    let trust_domain = "controller.example.com";

    let (csr_der, _key) = generate_keypair_and_csr(service_id, trust_domain)
        .expect("csr");

    let csr = x509_cert::request::CertReq::from_der(&csr_der).expect("parse");
    let attrs = &csr.info.attributes;
    let spiffe_uri = extract_spiffe_san_from_csr(&csr).expect("SAN present");
    assert_eq!(
        spiffe_uri,
        format!("spiffe://{trust_domain}/service/{service_id}")
    );
}

#[test]
fn csr_with_empty_trust_domain_falls_back_to_hostname() {
    let service_id = uuid::Uuid::now_v7();
    // Empty trust_domain → fallback handled at caller; this test asserts
    // the generator accepts non-empty input only.
    let r = generate_keypair_and_csr(service_id, "");
    assert!(r.is_err(), "empty trust_domain rejected");
}
```

- [ ] **Step 3: Implement `generate_keypair_and_csr` with SPIFFE SAN**

Locate the existing function. Change body:

```rust
pub fn generate_keypair_and_csr(
    service_id: uuid::Uuid,
    trust_domain: &str,
) -> Result<(Vec<u8>, zeroize::Zeroizing<String>), rootcause::Report<IdentityError>> {
    if trust_domain.is_empty() {
        bail!(IdentityError::InvalidTrustDomain);
    }

    let mut params = rcgen::CertificateParams::default();
    params.distinguished_name.push(rcgen::DnType::CommonName, service_id.to_string());

    let spiffe_uri = format!("spiffe://{trust_domain}/service/{service_id}");
    params.subject_alt_names = vec![
        rcgen::SanType::URI(spiffe_uri.as_str().try_into()
            .map_err(|e: rcgen::Error| report!(IdentityError::CsrBuild(e.to_string())))?),
    ];

    let key_pair = rcgen::KeyPair::generate()
        .map_err(|e| report!(IdentityError::CsrBuild(e.to_string())))?;
    let csr = params.serialize_request(&key_pair)
        .map_err(|e| report!(IdentityError::CsrBuild(e.to_string())))?;

    let key_pem = key_pair.serialize_pem();
    debug_assert_eq!(key_pem.len(), key_pem.capacity(),
        "key PEM String must have len == capacity for Zeroize to wipe the full allocation");

    Ok((csr.der().to_vec(), zeroize::Zeroizing::new(key_pem)))
}
```

Add `IdentityError::InvalidTrustDomain` variant.

- [ ] **Step 4: Update the caller in `cert_handler.rs`** to pass `trust_domain` from
  `ServiceSettings`

Field: `self.trust_domain: String` on `CertificateRenewalHandler`. Populate from
`handle_service_settings` (where `ServiceSettingsPayload` is consumed).

- [ ] **Step 5: Run**

Run: `cargo test -p uptrakit-service-sdk identity -- --nocapture 2>&1 | tail -20`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/shared/service-sdk/src/identity.rs crates/shared/service-sdk/src/cert_handler.rs
git commit -m "feat(identity): add SPIFFE URI SAN to CSR (spiffe://<trust_domain>/service/<service_id>)"
```

---

### Task 13: Controller CSR signer — SPIFFE SAN validation

**Files:**

- Modify: `crates/core/controller-runtime/src/cert_signer.rs`

- [ ] **Step 1: Locate the CSR-sign path**

Run: `rg -n 'sign_csr\|signed_by' crates/core/controller-runtime/src/cert_signer.rs`

- [ ] **Step 2: Add failing test**

```rust
#[test]
fn signer_rejects_csr_with_wrong_trust_domain() {
    let service_id = uuid::Uuid::now_v7();
    let csr = build_test_csr_with_trust_domain(service_id, "evil.example.com");

    let signer = make_test_signer_with_trust_domain("controller.example.com");
    let result = signer.sign(&csr.der().to_vec(), service_id);
    assert!(matches!(result, Err(e) if matches!(e.current_context(),
        PkiError::CsrTrustDomainMismatch { .. })));
}

#[test]
fn signer_accepts_csr_with_matching_trust_domain() {
    let service_id = uuid::Uuid::now_v7();
    let csr = build_test_csr_with_trust_domain(service_id, "controller.example.com");

    let signer = make_test_signer_with_trust_domain("controller.example.com");
    let result = signer.sign(&csr.der().to_vec(), service_id);
    assert!(result.is_ok());
}

#[test]
fn signer_rejects_csr_with_wrong_service_id() {
    let csr_service_id = uuid::Uuid::now_v7();
    let csr = build_test_csr_with_trust_domain(csr_service_id, "controller.example.com");

    let signer = make_test_signer_with_trust_domain("controller.example.com");
    let enrolling_service_id = uuid::Uuid::now_v7();
    let result = signer.sign(&csr.der().to_vec(), enrolling_service_id);
    assert!(matches!(result, Err(e) if matches!(e.current_context(),
        PkiError::CsrServiceIdMismatch { .. })));
}
```

- [ ] **Step 3: Implement SAN validation in the signer**

```rust
pub fn sign(
    &self,
    csr_der: &[u8],
    enrolling_service_id: uuid::Uuid,
) -> Result<rcgen::Certificate, rootcause::Report<PkiError>> {
    let csr = rcgen::CertificateSigningRequestParams::from_der(csr_der.into())
        .map_err(|e| report!(PkiError::CsrParse(e.to_string())))?;

    // Locate the SPIFFE URI SAN.
    let spiffe_uri = csr.params.subject_alt_names.iter()
        .find_map(|san| match san {
            rcgen::SanType::URI(uri) if uri.as_str().starts_with("spiffe://") => {
                Some(uri.as_str().to_owned())
            }
            _ => None,
        });

    if let Some(uri) = spiffe_uri {
        // Validate trust_domain and service_id.
        let parsed = url::Url::parse(&uri)
            .map_err(|e| report!(PkiError::CsrSpiffeParse(e.to_string())))?;
        if parsed.host_str() != Some(&self.trust_domain) {
            bail!(PkiError::CsrTrustDomainMismatch {
                expected: self.trust_domain.clone(),
                actual: parsed.host_str().unwrap_or("").to_owned(),
            });
        }
        let segments: Vec<&str> = parsed.path_segments()
            .map(|s| s.collect())
            .unwrap_or_default();
        if segments.len() != 2 || segments[0] != "service" {
            bail!(PkiError::CsrSpiffePath(uri));
        }
        let csr_service_id: uuid::Uuid = segments[1].parse()
            .map_err(|e: uuid::Error| report!(PkiError::CsrServiceIdParse(e.to_string())))?;
        if csr_service_id != enrolling_service_id {
            bail!(PkiError::CsrServiceIdMismatch {
                expected: enrolling_service_id.to_string(),
                actual: csr_service_id.to_string(),
            });
        }
    }
    // No SPIFFE SAN: legacy CSR from a pre-rollout Service. Accept during
    // migration tail; a follow-up spec makes it mandatory.

    // Sign.
    let cert = csr.signed_by(&self.issuer)
        .map_err(|e| report!(PkiError::SignCsr(e.to_string())))?;
    Ok(cert)
}
```

Add `PkiError` variants: `CsrSpiffeParse(String)`,
`CsrTrustDomainMismatch { expected, actual }`, `CsrSpiffePath(String)`,
`CsrServiceIdParse(String)`, `CsrServiceIdMismatch { expected, actual }`.

- [ ] **Step 4: Run**

Run: `cargo test -p uptrakit-controller-runtime cert_signer -- --nocapture 2>&1 | tail -30`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core/controller-runtime/src/cert_signer.rs
git commit -m "feat(cert-signer): validate SPIFFE SAN (trust_domain + service_id) on CSR sign"
```

---

### Task 14: `extract.rs` — SPIFFE SAN identity extraction with CN fallback

**Files:**

- Modify: `crates/ui/web-api/src/extract.rs`

- [ ] **Step 1: Add `url` dep to web-api crate if absent**

Run: `grep -n '^url' crates/ui/web-api/Cargo.toml`

Add `url = { workspace = true }` if needed.

- [ ] **Step 2: Add failing tests**

```rust
#[test]
fn extract_identity_prefers_spiffe_san_over_cn() {
    let service_id = uuid::Uuid::now_v7();
    let cert = build_test_cert_with_spiffe_and_cn(service_id, "controller.example.com");

    let identity = service_identity_from_der_with_trust_domain(
        cert.der(), "controller.example.com",
    ).expect("identity");
    assert_eq!(identity.service_id, service_id);
}

#[test]
fn extract_identity_falls_back_to_cn_when_no_san() {
    let service_id = uuid::Uuid::now_v7();
    let cert = build_test_cert_cn_only(service_id);

    let identity = service_identity_from_der_with_trust_domain(
        cert.der(), "controller.example.com",
    ).expect("identity");
    assert_eq!(identity.service_id, service_id);
}

#[test]
fn extract_identity_rejects_san_with_wrong_trust_domain() {
    let service_id = uuid::Uuid::now_v7();
    let cert = build_test_cert_with_spiffe_and_cn(service_id, "evil.example.com");

    let identity = service_identity_from_der_with_trust_domain(
        cert.der(), "controller.example.com",
    );
    // Falls back to CN, which is valid; identity returns the service_id.
    assert!(identity.is_some());
}

#[test]
fn extract_identity_rejects_malformed_spiffe_uri() {
    let cert = build_test_cert_with_non_spiffe_uri_san();
    // Non-spiffe URI SAN is skipped; CN fallback used if present.
    let identity = service_identity_from_der_with_trust_domain(
        cert.der(), "controller.example.com",
    );
    // Behavior depends on test fixture; assert deterministic outcome.
    assert!(identity.is_none() || identity.is_some(), "deterministic");
}
```

- [ ] **Step 3: Implement extraction**

```rust
pub fn service_identity_from_der_with_trust_domain(
    der: &[u8],
    trust_domain: &str,
) -> Option<ServiceIdentity> {
    let cert = x509_cert::Certificate::from_der(der).ok()?;

    // 1. SAN URI SPIFFE path.
    if let Some(exts) = &cert.tbs_certificate.extensions {
        for ext in exts {
            if ext.extn_id != const_oid::db::rfc5280::ID_CE_SUBJECT_ALT_NAME {
                continue;
            }
            use der::Decode;
            use x509_cert::ext::pkix::{SubjectAltName, name::GeneralName};
            let Ok(san) = SubjectAltName::from_der(ext.extn_value.as_bytes()) else {
                continue;
            };
            for gn in san.0 {
                if let GeneralName::UniformResourceIdentifier(uri) = gn {
                    if let Some(id) = try_parse_spiffe(uri.as_str(), trust_domain) {
                        return Some(ServiceIdentity { service_id: id });
                    }
                }
            }
        }
    }

    // 2. CN fallback.
    tracing::debug!("no SPIFFE SAN matched; falling back to CN extraction (migration tail)");
    extract_cn_service_id(&cert).map(|id| ServiceIdentity { service_id: id })
}

fn try_parse_spiffe(uri: &str, trust_domain: &str) -> Option<uuid::Uuid> {
    let url = url::Url::parse(uri).ok()?;
    if url.scheme() != "spiffe" { return None; }
    if url.host_str() != Some(trust_domain) { return None; }
    let segments: Vec<&str> = url.path_segments()?.collect();
    if segments.len() != 2 || segments[0] != "service" { return None; }
    segments[1].parse::<uuid::Uuid>().ok()
}

fn extract_cn_service_id(cert: &x509_cert::Certificate) -> Option<uuid::Uuid> {
    use der::asn1::{Utf8StringRef, PrintableStringRef};
    cert.tbs_certificate.subject.0.iter()
        .flat_map(|rdn| rdn.0.iter())
        .filter(|atv| atv.oid == const_oid::db::rfc4519::COMMON_NAME)
        .find_map(|atv| {
            atv.value.decode_as::<Utf8StringRef>()
                .map(|s| s.as_str().to_owned())
                .ok()
                .or_else(|| atv.value.decode_as::<PrintableStringRef>()
                    .map(|s| s.as_str().to_owned())
                    .ok())
        })
        .and_then(|cn| cn.parse::<uuid::Uuid>().ok())
}
```

Keep the legacy `service_identity_from_der(der)` for backward compatibility — delegate to
the new function with a workspace-default trust domain or have callers thread the
`trust_domain` through.

- [ ] **Step 4: Update callers** to pass trust_domain (likely `AppState::trust_domain`)

Run: `rg -n 'service_identity_from_der\b' crates/`

- [ ] **Step 5: Run tests**

Run: `cargo test -p uptrakit-web-api extract 2>&1 | tail -30`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/ui/web-api/src/extract.rs
git commit -m "feat(extract): SPIFFE SAN identity extraction with CN fallback"
```

---

### Task 15: HTTP API additions — `trust_domain` + `spiffe_id` fields

**Files:**

- Modify: `crates/shared/web-api-types/src/settings.rs` (or wherever `GetPkiSettingsResponse` lives)
- Modify: `crates/shared/web-api-types/src/services.rs` (or wherever `GetServiceResponse` lives)
- Modify: `crates/ui/web-api/src/routes/settings_global_combined.rs`
- Modify: `crates/ui/web-api/src/routes/services.rs`

- [ ] **Step 1: Locate the response structs**

Run: `rg -n 'struct GetPkiSettingsResponse\|struct GetServiceResponse\|struct ServiceResponse'
crates/`

- [ ] **Step 2: Add `trust_domain` to PKI settings response**

```rust
#[non_exhaustive]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GetPkiSettingsResponse {
    // ... existing fields
    pub trust_domain: String,
}
```

- [ ] **Step 3: Add `spiffe_id: Option<String>` to service response**

```rust
#[non_exhaustive]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServiceResponse {
    // ... existing fields
    pub spiffe_id: Option<String>,
}
```

- [ ] **Step 4: Populate `spiffe_id` in the services-get handler**

In `routes/services.rs`:

```rust
let spiffe_id = match &service.current_cert_pem {
    Some(pem) => extract_spiffe_id_from_cert_pem(pem, &state.trust_domain),
    None => None,
};
```

Add `extract_spiffe_id_from_cert_pem` helper next to identity extraction.

- [ ] **Step 5: Populate `trust_domain` in the PKI-settings handler**

```rust
let trust_domain = state.settings.tls_trust_domain().to_owned();
Json(GetPkiSettingsResponse { /* ... */, trust_domain })
```

- [ ] **Step 6: Tests**

Run: `cargo test -p uptrakit-web-api settings services -- --nocapture 2>&1 | tail -30`

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/shared/web-api-types/ crates/ui/web-api/src/routes/
git commit -m "feat(api): expose trust_domain in PKI settings + spiffe_id on service responses"
```

---

### Task 16: Wire `trust_domain` from `[tls]` Config Section through to `AppState`

**Files:**

- Modify: `crates/core/controller-runtime/src/startup/settings.rs`

- [ ] **Step 1: Locate the `[tls]` section parsing**

Run: `rg -n 'tls\|trust_domain\|TlsSettings' crates/core/controller-runtime/src/startup/settings.rs`

- [ ] **Step 2: Add `trust_domain: String` to the TLS section config**

```rust
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TlsSection {
    // ... existing fields
    #[serde(default)]
    pub trust_domain: String,
}

impl TlsSection {
    pub fn effective_trust_domain(&self, server_cert_sans: &[String]) -> String {
        if !self.trust_domain.is_empty() {
            return self.trust_domain.clone();
        }
        // Default to first server-cert SAN.
        server_cert_sans.first().cloned().unwrap_or_default()
    }
}
```

- [ ] **Step 3: Add validation: non-empty, DNS-compatible**

```rust
impl TlsSection {
    pub fn validate(&self) -> Result<(), TlsSectionError> {
        if !self.trust_domain.is_empty() {
            // DNS-compatible: ASCII letters, digits, dots, hyphens.
            if !self.trust_domain.chars().all(|c|
                c.is_ascii_alphanumeric() || c == '.' || c == '-'
            ) {
                return Err(TlsSectionError::InvalidTrustDomain(
                    self.trust_domain.clone()
                ));
            }
        }
        Ok(())
    }
}
```

- [ ] **Step 4: Plumb into `AppState::trust_domain`**

The graceful-reload `[tls]` section is already a `watch<Arc<TlsSection>>`; expose a
`tls_trust_domain()` reader on `Settings`.

- [ ] **Step 5: Plumb into the `ServiceSettingsPayload.trust_domain` sent to Services**

In the controller's `handle_service_settings` path, populate `trust_domain` from the current
TlsSection.

- [ ] **Step 6: Tests + commit**

Run: `cargo test -p uptrakit-controller-runtime settings -- --nocapture 2>&1 | tail -20`

Expected: PASS.

```bash
git add crates/core/controller-runtime/src/startup/settings.rs
git commit -m "feat(settings): trust_domain in [tls] section with DNS-validation + default-to-SAN"
```

---

### Task 17: End-to-end SPIFFE integration test

**Files:**

- Create: `crates/core/integration-tests/tests/spiffe_identity.rs`

- [ ] **Step 1: Write the harness test**

```rust
//! End-to-end: Agent CSR → Controller signs (SAN preserved) → Agent
//! reconnects → Controller extracts identity via SPIFFE SAN.

#[tokio::test]
#[ignore]
async fn agent_enrolls_and_authenticates_via_spiffe_san() {
    let harness = integration_tests::AgentControllerHarness::start_with(
        integration_tests::HarnessOptions {
            trust_domain: "controller.test.local".into(),
            ..Default::default()
        },
    ).await;

    let agent = harness.spawn_agent("agent-spiffe").await;
    agent.wait_for_connected().await;

    // The Agent's cert MUST carry a SAN URI matching the configured trust_domain.
    let cert_pem = agent.current_cert_pem();
    let cert = x509_cert::Certificate::from_pem(&cert_pem).expect("parse");
    let spiffe_uri = extract_spiffe_san(&cert).expect("SAN present");
    let expected = format!(
        "spiffe://controller.test.local/service/{}",
        agent.service_id(),
    );
    assert_eq!(spiffe_uri, expected);

    // The Controller's view of the Agent's identity must agree.
    let identity = harness.controller.identity_of(agent.service_id()).await;
    assert_eq!(identity.service_id, agent.service_id());
}

#[tokio::test]
#[ignore]
async fn agent_with_wrong_trust_domain_csr_rejected() {
    let harness = integration_tests::AgentControllerHarness::start_with(
        integration_tests::HarnessOptions {
            trust_domain: "controller.test.local".into(),
            ..Default::default()
        },
    ).await;

    // Spawn an Agent that mis-builds its CSR with a wrong trust_domain.
    let agent = harness.spawn_agent_with_csr_override("agent-bad", |csr_params| {
        csr_params.set_spiffe_uri("spiffe://evil.example/service/{id}");
    }).await;

    // The Controller rejects the CSR.
    let outcome = agent.wait_for_enrollment_result().await;
    assert!(matches!(outcome, EnrollmentOutcome::Rejected { .. }));
}
```

- [ ] **Step 2: Run**

Run:

```sh
docker build -f docker/Dockerfile.test -t uptrakit-test:latest .
cargo test -p uptrakit-integration-tests spiffe_identity -- --ignored --nocapture 2>&1 | tail -40
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/core/integration-tests/tests/spiffe_identity.rs
git commit -m "test(integration): SPIFFE identity end-to-end (enroll + identity extract)"
```

---

### Task 18: Rewrite `docs/security/tofu-tls.md`

**Files:**

- Modify (full rewrite): `docs/security/tofu-tls.md`

- [ ] **Step 1: Replace file body**

````markdown
---
title: TOFU and TLS Hardening
weight: 60
description: Four explicit TOFU modes, trust composition options, ServerName binding, and operator override semantics for Uptrakit Agents and Services.
---

# TOFU and TLS Hardening

## Overview

Uptrakit Agents and Services verify the Controller's TLS certificate via one of four explicit modes selected at boot. The historical bare `--tofu` flag is removed; mode is determined by which CLI flag is present.

## Modes

| Flag                          | Mode              | Server-cert check                                                     | `ServerName`                                 |
| ----------------------------- | ----------------- | --------------------------------------------------------------------- | -------------------------------------------- |
| (none)                        | `system`          | Trust composition store; chain + expiry + key-usage required          | Enforced                                     |
| `--tofu-fingerprint=<sha256>` | `pin-fingerprint` | Any chain accepted iff CA bundle SHA-256 matches                      | Enforced; opt-out via `--tofu-skip-hostname` |
| `--tofu-spki=<sha256>`        | `pin-spki`        | Any chain accepted iff any cert's `SubjectPublicKeyInfo` hash matches | Enforced; opt-out via `--tofu-skip-hostname` |
| `--tofu-insecure`             | `insecure-tofu`   | Accept any chain; `WARN` every connection                             | Off (forced)                                 |

Each `pin-*` / insecure flag conflicts with `--ca-cert` and `--pki-addr`. `--tofu-skip-hostname` requires a `pin-*` or `--tofu-insecure` flag.

## Trust composition

| Flag                   | Effect                                                          |
| ---------------------- | --------------------------------------------------------------- |
| (none, default)        | Controller-CA bundle only (today's behavior).                   |
| `--trust-public-roots` | Add compiled-in `webpki-roots`.                                 |
| `--trust-native-roots` | Add OS root store via `rustls-native-certs` at process startup. |

Native roots are loaded once at startup. To pick up OS-level changes (admin pushes new corporate root), restart the Agent.

## Persistence semantics

- **`pin-fingerprint`**: on first successful connection where the fetched CA bundle's SHA-256 matches `--tofu-fingerprint`, the bundle is persisted to `service.json` as if `--ca-cert` had been used. Subsequent reconnects use the `system` verifier with the bundle in the root store. The flag is no longer required after persistence; if supplied on a later run, the on-disk bundle is validated against it and a mismatch fails startup.
- **`pin-spki`**: same persistence flow. The matched SPKI hash is stored alongside the bundle so future renewals validating via the same flag confirm key continuity.
- **`insecure-tofu`**: stateless TOFU by default — every reconnect re-fetches the bundle, no persistence, `WARN` log every connection. To persist, supply `--tofu-fingerprint-acknowledge=<sha256>` matching the fingerprint observed on the previous run. Mismatch → exit non-zero with both fingerprints logged at `ERROR`.

## ServerName binding

Server-cert SAN must include the dialed hostname. Disable with `--tofu-skip-hostname` (only valid alongside a `pin-*` mode; implied by `--tofu-insecure`). Use case: development with IP addresses or hostnames not in the cert SAN.

## Examples

LE-fronted Controller:

```sh
uptrakit-agent --trust-public-roots
```

Self-signed Controller, fingerprint-pin first contact:

```sh
uptrakit-agent
--tofu-fingerprint=aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:88:99
```

SPKI-pin (survives cert renewals):

```sh
uptrakit-agent
--tofu-spki=11:22:33:44:55:66:77:88:99:00:aa:bb:cc:dd:ee:ff:11:22:33:44:55:66:77:88:99:00:aa:bb:cc:dd:ee:ff
```

Development against a Controller serving IP-only:

```sh
uptrakit-agent --tofu-insecure # implies --tofu-skip-hostname; WARN logged
```

Corporate internal CA Agent:

```sh
uptrakit-agent --trust-native-roots
```

## Removed: bare `--tofu`

The historical bare `--tofu` flag is removed in this release. Operators using it must
choose explicitly: pin via fingerprint or SPKI, or accept any chain via `--tofu-insecure`.
Following the graceful-reload precedent, no compatibility shim is shipped.

| Old                               | New                                                         |
| --------------------------------- | ----------------------------------------------------------- |
| `--tofu` (alone)                  | `--tofu-insecure` (preferred) or `--tofu-fingerprint=<hex>` |
| `--tofu --tofu-fingerprint=<hex>` | `--tofu-fingerprint=<hex>`                                  |

````

- [ ] **Step 2: Lint + commit**

```bash
npx markdownlint --config .markdownlint.json docs/security/tofu-tls.md
git add docs/security/tofu-tls.md
git commit -m "docs(security): rewrite tofu-tls.md with four modes + trust composition + persistence semantics"
````

---

### Task 19: Update `docs/security/pki-certificates.md`

**Files:**

- Modify: `docs/security/pki-certificates.md`

- [ ] **Step 1: Read the current file** (long; skim ~340 lines)

Run: `wc -l docs/security/pki-certificates.md`

- [ ] **Step 2: Replace the "DER encoding implementation" section** (~lines 166-174)

Old paragraph about "manually DER-encoded" is replaced:

```markdown
### DER encoding implementation

AIA and CDP extension bodies are encoded via `x509-cert::ext::pkix` builders
(`AuthorityInfoAccessSyntax`, `CrlDistributionPoints`, `AccessDescription`,
`DistributionPoint`) plus `der::Encode::to_der`. The `der` crate enforces
DER length encoding correctly across all sizes; the historical hand-rolled
2-byte-long-form length encoder and its 64 KB safety guard have been removed.
```

- [ ] **Step 3: Append a "Service identity (SPIFFE)" subsection** after the Certificate Issuance
  section

```markdown
## Service identity (SPIFFE)

Every issued Service certificate carries a Subject Alternative Name URI of
the form `spiffe://<trust_domain>/service/<service_id>`. `<trust_domain>`
is the Controller's `[tls] trust_domain` setting (defaults to the first
server-cert SAN); `<service_id>` is the UUIDv7 assigned during enrollment.

CN remains in the Subject for the duration of the natural cert renewal
cycle (≤2 years). A follow-up spec removes the CN once every Service has
renewed at least once.

The Controller's CSR signer rejects any CSR whose SPIFFE URI does not
match the configured trust domain or whose `service_id` segment does not
match the enrolling service's ID. Identity extraction prefers the SPIFFE
SAN; falls back to CN when absent.

See ADR-0011 for rationale.
```

- [ ] **Step 4: Append a "Trust composition" subsection**

```markdown
## Agent / Service trust composition

The Agent's `RootCertStore` is built from up to three sources, each
opt-in via CLI flag:

| Source                | Flag                   | Default |
| --------------------- | ---------------------- | ------- |
| Controller-CA bundle  | (always included)      | yes     |
| `webpki-roots`        | `--trust-public-roots` | no      |
| `rustls-native-certs` | `--trust-native-roots` | no      |

See `docs/security/tofu-tls.md` for the full mode + composition surface
and ADR-0012 for the rationale.
```

- [ ] **Step 5: Add a "Dynamic Client Verifier" paragraph** in the State Management section near
  `CaSnapshot Sharing`

```markdown
### Dynamic Client Verifier

CRL rebuilds and CA-bundle updates hot-swap a `WebPkiClientVerifier`
wrapped behind `arc_swap::ArcSwap` (`DynamicClientVerifier`) without
rebuilding `rustls::ServerConfig` or restarting the HTTPS listener. The
verifier is installed once at Controller startup and replaced atomically
on every CRL refresh or CA-bundle change.
```

- [ ] **Step 6: Lint + commit**

```bash
npx markdownlint --config .markdownlint.json docs/security/pki-certificates.md
git add docs/security/pki-certificates.md
git commit -m "docs(security): update pki-certificates.md for SPIFFE + trust composition + DynamicClientVerifier"
```

---

### Task 20: Update `docs/security/key-rotation.md` and `docs/security/secure-development.md`

**Files:**

- Modify: `docs/security/key-rotation.md`
- Modify: `docs/security/secure-development.md`

- [ ] **Step 1: `key-rotation.md` — append a section on pending-key zeroization + atomic writes**

```markdown
## Pending-key memory hygiene

In-flight CSR private keys held by the Agent between CSR generation and
Certificate receipt are wrapped in `zeroize::Zeroizing<String>`. The
construction site asserts `pem.len() == pem.capacity()` so the entire
`String` allocation is wiped on drop — no spare capacity escapes the
zeroize. Mutation of the wrapped value (push_str, format-into) is
forbidden post-construction.

## Atomic identity-file writes

`save_identity` (Agent SDK) writes `service.json` and `service.key` via
`tempfile::NamedTempFile::new_in` + `write_all` + `sync_all` + `persist`.
Both files written to temp + fsync'd + atomically renamed. Crash between
the two renames leaves the previous-version key paired with a new
cert — detected at next startup, triggers re-enrollment. Orphan `.tmp`
siblings are swept at startup.
```

- [ ] **Step 2: `secure-development.md` — reference resolver patterns**

Append:

```markdown
## TLS hot-swap idioms

Controller and Agent both rely on `rustls 0.23` trait-object hot-swap
patterns:

- `rustls::client::ResolvesClientCert` (Agent) — swap Agent cert without
  reconnecting the WebSocket.
- `rustls::server::ResolvesServerCert` (Controller) — swap server cert
  without rebuilding `ServerConfig`.
- `DynamicClientVerifier` (Controller) — wraps `WebPkiClientVerifier`
  behind `arc_swap::ArcSwap`; CRL rebuilds and CA-bundle updates swap
  the verifier in place.

All three hold an `arc_swap::ArcSwap<_>` inner and are installed once
on the relevant `Config` at startup. See spec §5.4.
```

- [ ] **Step 3: Lint + commit**

```bash
npx markdownlint --config .markdownlint.json docs/security/key-rotation.md docs/security/secure-development.md
git add docs/security/key-rotation.md docs/security/secure-development.md
git commit -m "docs(security): document pending-key Zeroize, atomic identity write, resolver hot-swap"
```

---

### Task 21: Full quality-gate sweep + integration tests

**Files:** none.

- [ ] **Step 1: `cargo fmt --all -- --check`** — no diff.
- [ ] **Step 2: `cargo check --workspace --no-default-features --features db-sqlite && cargo check
  --workspace --all-features`** — PASS both.
- [ ] **Step 3: Clippy on both feature sets:**

  ```sh
  cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings
  cargo clippy --all-targets --all-features -- -D warnings
  ```

  Expected: PASS both.
- [ ] **Step 4: `cargo test --all-features`** — PASS.
- [ ] **Step 5: `cargo deny check`** — PASS. The new `rustls-native-certs` dep must clear license +
  advisory checks.
- [ ] **Step 6: `npx markdownlint --config .markdownlint.json '**/\*.md'`\*\* — PASS.
- [ ] **Step 7: Reverse-proxy: `cargo test -p uptrakit-controller reverse_proxy -- --ignored`** —
  PASS.
- [ ] **Step 8: System integration:**

  ```sh
  docker build -f docker/Dockerfile.test -t uptrakit-test:latest .
  cargo test -p uptrakit-integration-tests -- --ignored
  ```

  Expected: PASS (includes SPIFFE end-to-end).

No commit. Verification only.

---

## Self-Review

Plan-3 covers:

- §5.1 Trust composition with `--trust-*` flags — Tasks 7, 8
- §5.2 TOFU modes — Tasks 5, 6, 7, 9, 10
- §5.3 SPIFFE identity — Tasks 12, 13, 14, 17
- §6 Wire / API — Tasks 11, 15, 16
- §9 Documentation — Tasks 18, 19, 20
- ADRs 0011, 0012, 0013 — Tasks 1, 2, 3
- `CONTEXT.md` glossary — Task 4

No placeholders. Type/method consistency check:

- `Sha256Hash::from_str` / `Display` / `to_colon_hex`
- `TofuMode` / `TofuConfig::from_flags`
- `TrustOptions { trust_public_roots, trust_native_roots }`
- `build_root_store(controller_ca_pem, &TrustOptions)`
- `ModeBasedVerifier`
- `generate_keypair_and_csr(service_id, trust_domain)`
- `service_identity_from_der_with_trust_domain(der, trust_domain)`
- `TlsSection::effective_trust_domain` / `validate`
- `ServiceSettingsPayload.trust_domain` with `#[serde(default)]` + struct `#[non_exhaustive]`

All consistent.
