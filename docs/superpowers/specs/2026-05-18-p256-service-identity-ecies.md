# Spec: P-256 Service Identity and ECIES Unification

**Date:** 2026-05-18
**Status:** Draft
**Prerequisite:** `docs/superpowers/specs/2026-05-18-embedded-service-identity.md` must ship first.

---

## Problem

The mTLS follow-up spec (`2026-05-16-mtls-followup-design.md`) migrated the service
TLS keypair from P-256 to P-384 at two sites in `identity.rs`. It correctly excluded
`generate_p256_keypair_for_ecies()` from migration, but did not update
`sensitive_params.rs::sealed_box_decrypt()`, which is hardcoded to `agreement::ECDH_P256`.

Result: ECIES sealed-box decryption is broken for all enrolled standalone services. The
service presents a P-384 TLS key for ECIES but `aws_lc_rs` is asked to parse it as
P-256 (`PrivateKey::from_private_key_der(&agreement::ECDH_P256, ...)`), which fails.

Additionally, `ProviderEncryptionAlgorithm` in `crates/shared/surfaces/src/protocol.rs`
is a wire-protocol enum without the `Other(String)` catch-all required by project
standards.

---

## Root Cause

`identity.rs` comment block (lines 6–7) stated "ECDSA P-256 keypair generation" before the
mTLS follow-up changed the implementation to P-384 without updating `sensitive_params.rs`.
The discrepancy was introduced when the two functions were treated as independent concerns.

---

## Goals

1. Service TLS keypair generation (enrollment and renewal) uses P-256.
2. ECIES sealed-box decryption works for all enrolled standalone services.
3. No separate ECIES keypair — the service's TLS keypair is used for ECIES.
4. `ProviderEncryptionAlgorithm` satisfies the wire-safe enum standard.
5. `generate_p256_keypair_for_ecies()` removed (dead code after prerequisite ships).

---

## Non-Goals

- Changing CA keypair algorithm (stays P-384, unchanged).
- Changing server TLS cert algorithm (already P-256, unchanged).
- Changing CLI CA trust keypair (stays P-384, set by mTLS follow-up, unrelated to
  service identity).
- Changing OCSP signing algorithm (stays P-384, signs with CA key).
- Wire protocol changes.
- Frontend changes.

---

## Desired End State

| Certificate / Key                       | Algorithm | Notes                                        |
| --------------------------------------- | --------- | -------------------------------------------- |
| CA cert                                 | P-384     | Unchanged                                    |
| Server TLS cert                         | P-256     | Already correct, not touched                 |
| Service TLS cert (enrollment + renewal) | P-256     | **Fixed by this spec**                       |
| ECIES keypair (enrolled services)       | P-256     | Reuses TLS keypair — no separate key         |
| ECIES keypair (embedded services)       | P-256     | Ephemeral, from `for_embedded()` — unchanged |
| CLI CA trust keypair                    | P-384     | Not touched                                  |

---

## Prerequisite Dependency

The embedded-service-identity spec removes `ssh_agent::generate_ecies_keypair()` and
`mqtt::generate_ecies_keypair()` — the only callers of `generate_p256_keypair_for_ecies()`.
This spec depends on those removals being in place before it ships; otherwise the deletion
of `generate_p256_keypair_for_ecies()` will fail to compile.

After the prerequisite ships:

- Embedded services use `ServiceIdentityState::for_embedded(service_id, keypair)` where
  `keypair` is a freshly generated P-256 key inside `run_embedded_service`. No separate
  ECIES keypair generation in the controller.
- `generate_p256_keypair_for_ecies()` has no callers and can be safely deleted.

---

## Design

### 1 — Service keypair generation: P-384 → P-256

Two sites in `crates/shared/service-sdk/src/identity.rs`:

**`ensure_keypair()` (line 325):**

```rust
// Before
rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384)

// After
rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
```

**`generate_keypair_and_csr()` (line 614):**

```rust
// Before
rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384)

// After
rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
```

Update the module-level doc comment (line 7) to match:

```rust
//! - ECDSA P-256 keypair generation and persistence (`service.key`)
```

### 2 — Delete `generate_p256_keypair_for_ecies()`

Remove the function `generate_p256_keypair_for_ecies()` (lines 667–678) and its
re-export from `lib.rs`:

```rust
// In lib.rs — remove generate_p256_keypair_for_ecies from pub use
pub use identity::ServiceIdentityState;
// (generate_keypair_and_csr and generate_keypair_and_csr_with_spiffe remain)
```

Remove the corresponding test `generate_p256_keypair_for_ecies_produces_valid_pair`
(lines 1303–1322).

### 3 — `ProviderEncryptionAlgorithm` wire-safe migration

**Current** (`crates/shared/surfaces/src/protocol.rs` ~line 759):

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderEncryptionAlgorithm {
    EciesP256,
}
```

**After:**

```rust
wire_safe_enum! {
    /// Encryption algorithm used for ECIES sealed-box parameter encryption.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum ProviderEncryptionAlgorithm {
        EciesP256 => "ecies_p256",
    }
    parse_error = ParseProviderEncryptionAlgorithmError("invalid provider encryption algorithm");
}
```

The macro generates `#[non_exhaustive]`, `Other(String)`, `as_str()`, `Display`,
`From<String>`, `Serialize`, `Deserialize`, and `FromStr`. The manual `#[derive]` for
`Serialize`/`Deserialize` and the `#[serde(rename_all)]` attribute are removed.

**`Cargo.toml` additions for `uptrakit-surfaces`:**

```toml
uptrakit-shared-macros = { workspace = true }
tracing = { workspace = true }
```

Both are required by `wire_safe_enum!`. Add under `[dependencies]`.

### 4 — Match site wildcard arms

`ProviderEncryptionAlgorithm` becomes `#[non_exhaustive]`. Any exhaustive `match` on it
must add a wildcard arm. Audit all match sites:

| File                                                                    | Site                                                              | Required change                                                                                                                     |
| ----------------------------------------------------------------------- | ----------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| `crates/ui/cli/src/commands/surfaces.rs:836`                            | `if !matches!(algorithm, ProviderEncryptionAlgorithm::EciesP256)` | No change — `matches!` is not an exhaustive match; returns `false` for `Other(_)`, and the existing error path handles it correctly |
| `crates/shared/surfaces/tests/protocol.rs:338`                          | Struct literal                                                    | No change — construction site, not a match                                                                                          |
| `crates/ui/surface-proxy/src/registry.rs:1303`                          | Struct literal                                                    | No change — construction site                                                                                                       |
| `crates/core/mqtt-runtime/src/lib.rs:1201`                              | Struct literal                                                    | No change — construction site                                                                                                       |
| `crates/core/mqtt-runtime/src/surface_runtime.rs:158`                   | Struct literal                                                    | No change — construction site                                                                                                       |
| `crates/core/agent-ssh-runtime/src/surface_runtime.rs:156`              | Struct literal                                                    | No change — construction site                                                                                                       |
| `crates/core/agent-ssh-runtime/src/surface_runtime/registration.rs:80`  | Struct literal                                                    | No change — construction site                                                                                                       |
| `crates/ui/surface-proxy/src/proxy/tests/provider_proxied/mod.rs:70,99` | Struct literals                                                   | No change — construction sites                                                                                                      |
| `crates/ui/cli/tests/command_execution.rs:227`                          | Struct literal                                                    | No change — construction site                                                                                                       |

No exhaustive `match` sites exist on this enum today. If any appear during compilation
after the `#[non_exhaustive]` change, add a wildcard arm:

```rust
other => {
    tracing::warn!(algorithm = ?other, "unknown ProviderEncryptionAlgorithm; treating as unsupported");
    // handle as unsupported — same path as the existing error return
}
```

### 5 — Prerequisite comment cleanup

The embedded-service-identity plan writes this two-line comment into
`run_embedded_service` inside `crates/shared/service-sdk/src/embedded.rs`:

```rust
// P-256 is intentional: sealed_box_decrypt in sensitive_params.rs is
// hardcoded to ECDH_P256. Migration to P-384 is a separate future spec.
```

After this spec ships the second sentence is false: the "separate future spec" has
arrived and it goes to P-256, not P-384. Remove it. The first sentence remains valid
(the constraint is real and worth calling out).

```rust
// P-256 is intentional: sealed_box_decrypt in sensitive_params.rs is
// hardcoded to ECDH_P256.
```

### 6 — Test updates

Tests in `identity.rs` that construct a service keypair with P-384 must be updated to P-256:

| Test                                          | Location  | Change                                              |
| --------------------------------------------- | --------- | --------------------------------------------------- |
| `certificate_save_clears_enrollment_secret`   | line 890  | `PKCS_ECDSA_P384_SHA384` → `PKCS_ECDSA_P256_SHA256` |
| `is_cert_expired_works`                       | line 1112 | Same substitution                                   |
| `tenant_id_preserved_across_certificate_save` | line 1199 | Same substitution                                   |
| `pem_to_der_real_certificate`                 | line 1022 | Same substitution                                   |

These tests use a P-384 key to sign a synthetic certificate for testing cert parsing and
expiry logic. The algorithm is incidental — changing to P-256 has identical test coverage.

Tests in `cert_signer.rs` (`generate_test_csr`, `generate_test_csr_with_spiffe`) use P-384
for CSR generation but the signer is algorithm-agnostic. Leave unchanged — they test the
signer's behaviour, not the service keypair algorithm.

---

## Migration / Rollout

**Already-enrolled services with P-384 keys on disk:**

- TLS continues to work. The controller's `cert_signer` accepts CSRs from any curve; the
  CA signs whatever public key the CSR carries.
- ECIES was already broken for enrolled standalone services before this spec (P-384 key vs
  P-256 decrypt). This spec does not introduce a regression; it fixes the path forward.
- On next certificate renewal: `generate_keypair_and_csr()` generates a new P-256 keypair,
  `save_private_key()` replaces the on-disk key, and ECIES begins working.
- Operators who need ECIES immediately can force re-enrollment.

No forced migration, no dual-accept period, no new wire fields required.

---

## Error Handling

No new error paths. `rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)` has
identical error semantics to P-384 keygen. Existing `IdentityError::KeypairGeneration`
propagation is unchanged.

---

## Testing

### Unit — `service-sdk/src/identity.rs`

- `ensure_keypair()` produces a 65-byte uncompressed public key (`0x04` prefix) — confirms P-256.
- `generate_keypair_and_csr()` produces a CSR whose embedded public key is 65 bytes (P-256).
- Existing `keypair_generation_persists`, `idempotent_ensure_keypair`, and `csr_generation`
  tests continue to pass without modification (algorithm-agnostic assertions).

### Unit — `crates/shared/surfaces`

- `ProviderEncryptionAlgorithm::EciesP256` serializes to `"ecies_p256"` and deserializes
  back correctly.
- An unknown string `"ecies_p384"` deserializes to `Other("ecies_p384".to_string())` without
  panicking.
- `Other("ecies_p384")` serializes back to `"ecies_p384"` (round-trip identity).

### Compile-time

Both `cargo check --no-default-features --features db-sqlite` and `cargo check --all-features`
must pass. `generate_p256_keypair_for_ecies` deletion will produce a compile error if any
caller was missed.

---

## Affected Files

| File                                          | Change                                                                                                                                                             |
| --------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `crates/shared/service-sdk/src/identity.rs`   | `ensure_keypair()`: P-384 → P-256; `generate_keypair_and_csr()`: P-384 → P-256; delete `generate_p256_keypair_for_ecies()`; delete its test; update 4 test helpers |
| `crates/shared/service-sdk/src/lib.rs`        | Remove `generate_p256_keypair_for_ecies` from `pub use` (line 108); remove doc-comment reference at line 44                                                        |
| `crates/shared/service-sdk/src/embedded.rs`   | Remove stale "Migration to P-384 is a separate future spec." sentence from keypair-generation comment block (§5 above)                                             |
| `crates/shared/surfaces/Cargo.toml`           | Add `uptrakit-shared-macros` and `tracing` to `[dependencies]`                                                                                                     |
| `crates/shared/surfaces/src/protocol.rs`      | Replace `ProviderEncryptionAlgorithm` definition with `wire_safe_enum!`                                                                                            |
| `docs/adr/0016-p384-ca-p256-service-certs.md` | New ADR (see §Documentation)                                                                                                                                       |

**Not changed:**

| File                             | Reason                                                |
| -------------------------------- | ----------------------------------------------------- |
| `pki.rs::generate_ca()`          | CA stays P-384                                        |
| `pki.rs::generate_server_cert()` | Server TLS stays P-256, already correct               |
| `web-api/src/ocsp.rs`            | OCSP signing uses CA key (P-384); must match          |
| `sensitive_params.rs`            | Hardcoded `ECDH_P256` becomes correct after this spec |
| `cli/src/commands/auth.rs`       | CLI CA trust keypair (P-384), not service identity    |
| `cert_signer.rs`                 | Algorithm-agnostic; no changes needed                 |

---

## Documentation Deliverables

### ADR-0016: P-384 for CA, P-256 for Server/Service/ECIES Certs

**Status:** Accepted

**Context:**

The mTLS hardening spec (`2026-05-12-mtls-hardening-design.md`) introduced P-256 as the
uniform key algorithm across all certificate roles. The mTLS follow-up spec
(`2026-05-16-mtls-followup-design.md`) partially migrated service TLS to P-384 for a wider
classical security margin, but left the ECIES decrypt path on P-256, breaking the
sealed-box flow for enrolled standalone services.

**Decision:**

- **CA certs: P-384.** Highest-value long-lived key material; wider classical security
  margin justifies the larger key size.
- **Server TLS certs: P-256.** Envoy ≤ 1.32 supports only P-256 for static TLS config;
  P-256 is the safe default for server-facing material that must pass through arbitrary
  reverse proxies.
- **Service TLS certs (enrollment + renewal): P-256.** Leaf certs are short-lived (max 2
  years). P-256 is NIST-recommended for TLS at ≤128-bit security level. Reusing the TLS
  keypair for ECIES (which is hardcoded to `ECDH_P256` in `aws_lc_rs`) eliminates the
  dual-keypair split. The combination of P-384 CA + P-256 leaf is standard practice
  (cf. Let's Encrypt R3/E1 chain).
- **ECIES keypair: reuse service TLS keypair (P-256).** No separate ECIES keypair for
  enrolled services. Embedded services retain their ephemeral P-256 keypair
  (generated in `run_embedded_service`; never persisted).

**Alternatives considered:**

| Option                                                                 | Outcome                                                                                                                                     |
| ---------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| Service TLS stays P-384; update `sensitive_params.rs` to `ECDH_P384`   | Rejected — requires updating all ECIES encrypt callers (CLI, surface-proxy); adds complexity with no net benefit for short-lived leaf certs |
| Service TLS stays P-384; keep separate P-256 ECIES keypair per service | Rejected — dual-keypair complexity, two on-disk files, identity fragmentation                                                               |
| Uniform P-384 everywhere including server certs                        | Rejected — breaks Envoy ≤ 1.32                                                                                                              |

**Consequences:**

- One keypair per enrolled service, used for both mTLS client auth and ECIES
  sealed-box decryption. No separate ECIES key material to manage.
- Existing enrolled services with P-384 keys continue to function for TLS; ECIES
  (already broken) is fixed on next renewal cycle.
- Future changes to the ECIES algorithm require updating `sensitive_params.rs` and
  `ProviderEncryptionAlgorithm` in `protocol.rs`. The `Other(String)` catch-all on the
  enum allows a newer agent to advertise a new algorithm without crashing an older
  controller.

**Related:** ADR-0013, mTLS hardening spec, embedded-service-identity spec.
