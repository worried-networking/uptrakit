# mTLS Follow-ups: P-384 Migration and AgentControllerHarness

Status: Draft for review
Author: Andrey Yantsen
Date: 2026-05-16
Parent spec: [`docs/superpowers/specs/2026-05-12-mtls-hardening-design.md`](2026-05-12-mtls-hardening-design.md)
ADRs touched: [`docs/adr/0013-defer-root-intermediate-ca-split.md`](../../adr/0013-defer-root-intermediate-ca-split.md)

## 1. Goal

Address two deferred items from the mTLS hardening spec:

1. **P-256 → P-384 key algorithm migration.** CA keys, service client certs,
   and CLI CA trust keys migrate to P-384. Server HTTPS certificates stay at
   P-256 for broad reverse-proxy compatibility (Envoy ≤ 1.32 hardcodes
   P-256-only for static TLS config). P-384 provides a wider classical security
   margin for long-lived material without requiring new dependencies or wire
   format changes.

2. **`AgentControllerHarness`.** A Docker-based integration test harness that
   unblocks two stubbed tests (`spiffe_identity.rs`,
   `cert_rotation_hot_swap.rs`) that have been blocked since the mTLS hardening
   spec shipped. The harness wraps the existing `ControllerContainer` +
   `ServiceContainer` infrastructure and adds the richer API surface those tests
   need.

**CA root/intermediate split (ADR-0013) is explicitly out of scope.** The
2026-05-16 review found no active deployment using `--tofu-spki`, so the
durability benefit does not justify the cost. ADR-0013 is updated to record
the extended deferral (§6).

## 2. Background

### 2.1 P-256 hardcoding

Every key generation site in the codebase uses `rcgen::PKCS_ECDSA_P256_SHA256`
and the OCSP responder hard-codes
`aws_lc_rs::signature::ECDSA_P256_SHA256_ASN1_SIGNING`. Both rcgen and
aws-lc-rs already ship P-384 support (`PKCS_ECDSA_P384_SHA384`,
`ECDSA_P384_SHA384_ASN1_SIGNING`); the migration is a mechanical substitution
with no new dependencies.

P-384 is not post-quantum secure — no elliptic curve algorithm is. The
migration is a classical security margin improvement. Post-quantum PKI
(ML-DSA) is not yet supported by rcgen or rustls for certificate signatures
and is a separate future concern.

### 2.2 Stubbed tests

Two files exist in `crates/core/integration-tests/tests/` as stubs tagged
`#[ignore = "Requires AgentControllerHarness (not yet implemented) + Docker"]`:

- `spiffe_identity.rs` — SPIFFE SAN end-to-end enrollment
- `cert_rotation_hot_swap.rs` — cert renewal without session disruption

Both need a harness that can start a Controller with a configured
`trust_domain`, enroll an Agent, observe service state via the API, and
trigger per-service cert renewal and disconnect via test-only endpoints.

The existing `ControllerContainer` / `ServiceContainer` / `ApiClient` helpers
cover enrollment and service-list polling but not per-service renewal
triggering, service disconnection, or cert serial introspection.

## 3. Scope

### 3.1 In scope

**P-384 migration:**

- `crates/core/controller-runtime/src/pki.rs` — CA keygen only (server cert stays P-256, see §4.1)
- `crates/shared/service-sdk/src/identity.rs` — service CSR keygen (2 sites — ECIES
  function excluded, see §4.1)
- `crates/ui/cli/src/commands/auth.rs` — CLI CA trust keypair
- `crates/ui/web-api/src/ocsp.rs:410` — OCSP signing algorithm constant
- All test helpers (`pki_utils.rs`, `extract.rs`, middleware tests, etc.) —
  mechanical, same substitution pattern

**`AgentControllerHarness`:**

- New file `crates/core/integration-tests/tests/helpers/agent_controller_harness.rs`
- New `test-utils` feature on `uptrakit-controller-runtime` and `uptrakit-web-api`
- Two new Controller endpoints (test-utils feature only)
- `cert_serial_number: Option<String>` addition to `ServiceResponse`
- Two stubbed tests promoted from `#[ignore]` to runnable
- One stubbed system test deleted (wrong trust domain CSR rejection already covered by existing unit test in `cert_signer.rs:548`)

**ADR-0013 update** — extended deferral note added.

### 3.2 Out of scope

- CA root/intermediate CA split (ADR-0013, extended deferral)
- Post-quantum key algorithms (ecosystem not ready)
- DB schema changes
- Wire protocol changes (SPIFFE SAN already in cert bytes on the wire; no
  envelope change)
- Frontend changes
- New CLI flags

## 4. P-384 Migration

### 4.1 Key generation

Replace `rcgen::PKCS_ECDSA_P256_SHA256` with `rcgen::PKCS_ECDSA_P384_SHA384`
at every CA and client key generation call site:

| File                            | Sites                                     |
| ------------------------------- | ----------------------------------------- |
| `controller-runtime/src/pki.rs` | CA bootstrap (`:476`) only                |
| `service-sdk/src/identity.rs`   | CSR generation (2 sites — see note below) |
| `cli/src/commands/auth.rs`      | CLI CA trust keypair                      |

**Server cert stays P-256.** `controller-runtime/src/pki.rs:697`
(`generate_server_cert`) and `web-api/src/routes/server_cert.rs:171` (HTTP
renewal endpoint) remain `PKCS_ECDSA_P256_SHA256`. Envoy ≤ 1.32 hardcodes
"only P-256 ECDSA certificates are supported" for static TLS config; migrating
server certs to P-384 would break any Envoy reverse proxy in front of the
controller. Corresponding test helpers at `server_cert.rs:349`
(`generate_test_server_cert`) also stay P-256.

**`generate_p256_keypair_for_ecies` exclusion.** `identity.rs` also contains
`generate_p256_keypair_for_ecies` (line 667), which generates a P-256 keypair
intentionally for ECIES sealed-box encryption in the shared-surface flow. This
function is **not** a TLS key-generation site — it must remain P-256 because
the ECIES protocol and its callers depend on the specific byte layout of P-256
uncompressed public keys. Do not migrate it. The "2 sites" count above
excludes this function.

Test helpers (~15 sites in `pki_utils.rs`, `extract.rs`, middleware and handler
tests) receive the same substitution. These are in `#[cfg(test)]` blocks or
test modules; the change is mechanical. `cert_handler.rs` and `cert_resolver.rs`
contain P-256 keygen only in their `#[cfg(test)]` modules — include all of
these in the mechanical sweep (server cert helpers excepted as noted above).

### 4.2 OCSP signing algorithm

The OCSP responder in `crates/ui/web-api/src/ocsp.rs` hard-codes the signing
algorithm at line 410:

```rust
// Before
&aws_lc_rs::signature::ECDSA_P256_SHA256_ASN1_SIGNING,

// After
&aws_lc_rs::signature::ECDSA_P384_SHA384_ASN1_SIGNING,
```

The active CA key is now P-384; the signing algorithm must match. No other
changes to `ocsp.rs` are required — the key loading and response encoding paths
are algorithm-agnostic.

### 4.3 No dependency changes

Both `rcgen` and `aws_lc_rs` are workspace dependencies that already include
P-384 support. `rcgen::PKCS_ECDSA_P384_SHA384` and
`aws_lc_rs::signature::ECDSA_P384_SHA384_ASN1_SIGNING` exist in the versions
already pinned in `Cargo.toml`.

### 4.4 Rollout behaviour

Existing keys (CA cert, service client certs) are unaffected until their normal
renewal cycle. The single semi-production deployment will pick up P-384 keys
organically: CA at next rotation (~5 years), service certs at their next renewal
window. Server certs stay P-256 permanently. No forced migration, no
re-enrollment required.

**CA rotation gap.** The CA is the highest-value target for the P-384 upgrade
(it is the longest-lived key), yet it migrates last under organic rotation. The
P-384 migration has near-zero security value for the CA without an explicit
rotation: an attacker who breaks the existing P-256 CA key can mint arbitrary
certs regardless of the leaf key algorithm. The semi-production deployment
**should trigger an immediate CA re-issue** as the first post-deploy step via
`POST /api/v1/settings/rotate-ca`. This triggers cert renewal for all connected
Agents within the renewal window. The re-issue is not required for correctness,
but is the recommended step to realize the P-384 security improvement for the
highest-value key material.

## 5. AgentControllerHarness

### 5.1 Architecture

The harness is a Docker-based wrapper. It owns a `ControllerContainer`, a
NATS sidecar (same pattern as existing `system/` tests), and zero or more
`AgentHandle`s, each wrapping a `ServiceContainer`. All Controller interaction
goes through the existing `ApiClient` pointed at the mapped host port.

```text
AgentControllerHarness
├── ControllerContainer   (owns NATS sidecar internally, same pattern as system/ tests)
├── ApiClient             (points at controller.host_port())
└── Vec<AgentHandle>
    └── AgentHandle
        ├── ServiceContainer
        └── cached service_id: Option<Uuid>
```

The harness does not import `uptrakit-controller-runtime` as a library.
All assertions go through HTTP. Controller-side behaviour is exercised
indirectly via the API — exactly the pattern the existing `system/` tests use.

### 5.2 `test-utils` feature flag

A new `test-utils` feature is added to both `uptrakit-controller-runtime` and
`uptrakit-web-api`. The feature is **additive-only** — no
`#[cfg(not(feature = "test-utils"))]` usage anywhere. **Both crates must
declare the feature in their own `Cargo.toml`.** The routes are registered in
`uptrakit-web-api`; the feature must be present in `uptrakit-web-api/Cargo.toml`
directly or the gated `test_routes` module will not compile into the binary
even if `uptrakit-controller-runtime` enables the feature. The integration-tests
crate does **not** add `uptrakit-controller-runtime` as a direct dependency —
the harness is HTTP-only and imports nothing from the controller library. The
Dockerfile.test build already compiles with `--all-features`; no Dockerfile
change is required.

All test-utils route handlers are gated by both compile-time feature and a
runtime env var. Routes are registered only when `test-utils` is compiled in
AND `UPTRAKIT_TEST_UTILS_ENABLED=true` is set in the environment:

```rust
#[cfg(feature = "test-utils")]
if std::env::var("UPTRAKIT_TEST_UTILS_ENABLED").as_deref() == Ok("true") {
    router = router.merge(test_routes::router());
}
```

This eliminates the accidental-promotion risk: even if `Dockerfile.test` is
mistakenly tagged as the production image, the endpoints are not registered
unless the env var is explicitly set. The integration-tests runner sets
`UPTRAKIT_TEST_UTILS_ENABLED=true` via the container env var mechanism.

### 5.3 New Controller endpoints

Both endpoints live under `/api/v1/test/` and are compiled only with
`test-utils`. Per coding standards ("Route handlers enforce permission via
typed Axum extractors; never inline has_permission"), each handler carries a
`TestUtilsAllowed` extractor — a zero-sized type that implements
`FromRequestParts` with `type Rejection = Infallible` and always extracts
successfully, making the no-auth policy explicit in the type system rather than
implicit in a bare async function. The `ApiClient` in the harness is already
authenticated (standard session token) so the real-auth routes work normally;
the test-utils routes deliberately bypass auth for test ergonomics.

**`POST /api/v1/test/services/{id}/request-renewal`**

Sends a `ControllerMessage::RequestCertRenewal(RequestCertRenewalPayload { .. })`
wire message to the WebSocket connection for the given service ID. Returns:

- `200 OK` — message sent
- `404 Not Found` — service not currently connected (no open WebSocket)
- `422` — `{id}` is not a valid UUID (Axum default)

The implementation looks up the service's broadcast sender from the in-memory
connection map (already used by the existing broadcast paths), sends the
message, and returns. It does not wait for the Agent to complete renewal.

**`POST /api/v1/test/services/{id}/disconnect`**

Closes the WebSocket connection for the given service ID by dropping or
signalling its connection handle. The Agent's reconnect loop reconnects
automatically within its backoff window (base 2s, cap 60s per coding
standards). Returns:

- `200 OK` — connection closed
- `404 Not Found` — service not currently connected
- `422` — malformed UUID

### 5.4 `cert_serial_number` on `ServiceResponse`

`ServiceResponse` in `crates/shared/web-api-types/src/services.rs` gains:

```rust
#[serde(skip_serializing_if = "Option::is_none")]
pub cert_serial_number: Option<String>,
```

`ServiceResponse` does **not** currently carry `#[non_exhaustive]`. Adding a
field breaks all exhaustive struct literal construction sites. This spec adds
`#[non_exhaustive]` to `ServiceResponse` as part of this work (coding standards
require it on all extensible public structs). `ServiceResponse` has required
non-optional fields and does not implement `Default`, so existing struct literal
construction sites must add `cert_serial_number: None` explicitly — not
`..Default::default()`. This is a mechanical fix alongside the field addition.

**Population strategy.** `cert_serial_number` is populated only on the
single-service detail endpoint (`GET /api/v1/services/{id}`), not on the list
endpoint (`GET /api/v1/services`). Populating it for list responses would
require a per-row query on `service_certificates`, violating the N+1 prevention
rule ("Batch N rows into one is_in(ids) query; no per-item queries"). The
field returns `None` in list responses via the new explicit `None` literal at
list construction sites.

**Detail vs. list fork.** `model_to_response` in `web-api-queries` is called by
both list and detail paths — it must remain innocent of the extra join. The
detail-endpoint handler (`get_service` in `routes/services.rs`) calls a new
`build_service_detail_response` helper that, after calling `model_to_response`,
issues a second targeted query to `service_certificates` for the most recent
non-revoked row scoped to the service ID, then sets `cert_serial_number`. `None`
if no cert exists yet (e.g. newly enrolled but cert not yet signed). The list
handler continues to call the existing path unchanged.

**Future-drift guard.** Any future detail-only field (not appropriate for list
responses) must be added in `build_service_detail_response` only, never in
`model_to_response`. A comment on `build_service_detail_response` documents this
invariant to prevent silent drift across the two code paths.

### 5.5 Harness API

New file: `crates/core/integration-tests/tests/helpers/agent_controller_harness.rs`

```rust
pub struct HarnessOptions {
    pub trust_domain: String,
}

impl Default for HarnessOptions {
    fn default() -> Self {
        Self { trust_domain: "controller.test.local".into() }
    }
}

pub struct AgentControllerHarness {
    pub controller: ControllerHandle,
    // ControllerContainer owns _nats_container internally — no separate NATS handle needed here.
    _controller_container: ControllerContainer,
}

pub struct ControllerHandle {
    client: ApiClient,
}

pub struct AgentHandle {
    _container: ContainerAsync<GenericImage>,
    service_id: Uuid,
}
```

Public methods:

| Method                                                           | Maps to                                                                                                                                                                                                      |
| ---------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `AgentControllerHarness::start_with(opts)`                       | `ControllerContainer::start()` + writes `[tls] trust_domain` into config TOML                                                                                                                                |
| `harness.spawn_agent(name) -> AgentHandle`                       | `ServiceContainer::start_agent()` + `wait_for_connected()`                                                                                                                                                   |
| `AgentHandle::service_id() -> Uuid`                              | cached from enrollment poll                                                                                                                                                                                  |
| `AgentHandle::wait_for_connected(&self)` internal                | polls until service is `Approved` **and** `POST /test/services/{id}/request-renewal` returns `200` (not `404`), confirming WebSocket is live                                                                 |
| `AgentHandle::wait_for_cert_renewed(&self, before_serial: &str)` | polls `GET /api/v1/services/{id}` until `cert_serial_number != before_serial`, 60s timeout                                                                                                                   |
| `ControllerHandle::identity_of(service_id) -> ServiceResponse`   | `GET /api/v1/services/{id}` — returns full response; callers read `.spiffe_id` and `.cert_serial_number`                                                                                                     |
| `ControllerHandle::request_cert_renewal(service_id)`             | `POST /api/v1/test/services/{id}/request-renewal`; retries up to 5× on `404` with 500ms backoff (guards against unexpected mid-test disconnect; in normal flow the service is already connected when called) |
| `ControllerHandle::disconnect_service(service_id)`               | `POST /api/v1/test/services/{id}/disconnect`                                                                                                                                                                 |

`spawn_agent` internally calls `wait_for_connected`, which polls
`GET /api/v1/services` with a 60s timeout until the service appears as
`ServiceStatus::Approved`. The `service_id` is cached on first observation.

### 5.6 Tests promoted from `#[ignore]`

**`spiffe_identity.rs::agent_enrolls_and_authenticates_via_spiffe_san`**

```rust
#[tokio::test]
#[ignore = "System integration test — requires uptrakit-test:latest Docker image"]
async fn agent_enrolls_and_authenticates_via_spiffe_san() {
    let harness = AgentControllerHarness::start_with(HarnessOptions {
        trust_domain: "controller.test.local".into(),
    }).await;

    let agent = harness.spawn_agent("agent-spiffe").await;

    let identity = harness.controller.identity_of(agent.service_id()).await;
    let expected = format!("spiffe://controller.test.local/service/{}", agent.service_id());
    assert_eq!(identity.spiffe_id.as_deref(), Some(expected.as_str()));
}
```

The `#[ignore]` reason changes from "Requires AgentControllerHarness (not yet
implemented)" to the standard system integration test note. The test no longer
calls `agent.current_cert_pem()` — `spiffe_id` on the service response is the
authoritative assertion.

**`cert_rotation_hot_swap.rs::agent_cert_renewal_via_resolver_keeps_session_alive`**

```rust
#[tokio::test]
#[ignore = "System integration test — requires uptrakit-test:latest Docker image"]
async fn agent_cert_renewal_via_resolver_keeps_session_alive() {
    let harness = AgentControllerHarness::start_with(HarnessOptions::default()).await;
    let agent = harness.spawn_agent("agent-1").await;

    // Capture serial before renewal.
    let before = harness.controller
        .identity_of(agent.service_id()).await
        .cert_serial_number
        .expect("cert serial present after enrollment");

    // Trigger renewal — must not disrupt the session.
    harness.controller.request_cert_renewal(agent.service_id()).await;

    // Service must remain Approved throughout renewal (no reconnect).
    // wait_for_cert_renewed polls until the serial changes, which happens
    // only after the Agent sends back the new CSR and the Controller signs it.
    agent.wait_for_cert_renewed(&before).await;

    // Force reconnect — Agent presents the new cert on the next handshake.
    harness.controller.disconnect_service(agent.service_id()).await;

    // After reconnect the serial on the Controller must still be the new one.
    let after = harness.controller
        .identity_of(agent.service_id()).await
        .cert_serial_number
        .expect("cert serial present after reconnect");
    assert_ne!(before, after, "cert serial must change after renewal");
}
```

The session-continuity property ("no reconnect during renewal") is implicit:
if the renewal caused a reconnect, the service would briefly disappear from
the Approved list or the serial would change via a re-enrollment rather than
an in-session renewal. `wait_for_cert_renewed` polls the Approved service for
a serial change — it would time out if the service disconnected and failed to
reconnect within 60s. **Limitation:** a fast reconnect (disconnect + reconnect
within one 2s poll interval) would not be detected by this test. The test
verifies that cert renewal completes and a new cert is presented; it does not
provide a hard guarantee of zero-downtime continuity. Full zero-downtime
verification requires in-process TLS session introspection (deferred until
the controller lib/bin split).

### 5.7 Test demoted to unit test

**`spiffe_identity.rs::agent_with_wrong_trust_domain_csr_rejected`**

The `spiffe_identity.rs` stub for this test is **deleted** — the behavior is
already covered by `spiffe_san_wrong_trust_domain_rejected` in
`crates/core/controller-runtime/src/cert_signer.rs` (line 548). No new unit
test is needed; the implementation plan should verify that the existing test
covers the full rejection path and extend it if any gap exists.

## 6. ADR-0013 Update

The following text is appended to the Consequences section of
`docs/adr/0013-defer-root-intermediate-ca-split.md`:

> **2026-05-16 review:** No active deployments use `--tofu-spki`. The SPKI pin
> durability benefit (surviving CA rotation every ~5 years) does not justify the
> implementation cost (two-tier bootstrap, dual-trust-anchor transition period,
> intermediate cert chain threading through wire → Agent `CertifiedKey`).
> Deferral extended indefinitely. Revisit only when a concrete deployment with
> `--tofu-spki` requirements exists.

## 7. Testing

### 7.1 Unit tests

- `cert_signer.rs` — verify existing `spiffe_san_wrong_trust_domain_rejected`
  (`:548`) covers the full rejection path; extend if any gap exists (see §5.7)
- OCSP `ocsp.rs` — existing tests pass with P-384 CA key (verify no test
  hard-codes the P-256 OID; fix if found)
- `pki.rs` — existing CA roundtrip tests pass with P-384 keys

### 7.2 Integration tests

- `spiffe_identity::agent_enrolls_and_authenticates_via_spiffe_san` — promoted
  from stub (Docker required)
- `cert_rotation_hot_swap::agent_cert_renewal_via_resolver_keeps_session_alive`
  — promoted from stub (Docker required)
- All existing `system/` tests continue to pass — P-384 CA keys are
  transparent to enrollment and service-list assertions

### 7.3 Quality gates

Full gate required for every PR:

```sh
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo deny check
markdownlint --config .markdownlint.json '**/*.md'
```

Enrollment/wire changes and the P-384 key migration trigger the mandatory
Docker integration gates:

```sh
docker build -f docker/Dockerfile.test -t uptrakit-test:latest .
cargo test -p uptrakit-integration-tests -- --ignored
cargo test -p uptrakit-integration-tests --test reverse_proxy -- --ignored
```

The P-384 migration changes how CA and server certs are generated, which the
reverse proxy integration tests exercise directly (mTLS + OCSP + CRL paths).
Both Docker gates are mandatory for any PR in this spec.

## 8. Documentation deliverables

- `docs/security/pki-certificates.md` — update algorithm column: CA and
  agent/service client certs → P-384; server HTTPS cert → P-256 (with note
  explaining Envoy compatibility rationale); note existing keys not force-rotated
- `docs/adr/0013-defer-root-intermediate-ca-split.md` — extended deferral note
  (§6)
- `CONTEXT.md` — no new glossary terms
- `docs/development/coding-standards.md` — if P-256 is mentioned explicitly
  anywhere, update to P-384; otherwise no change

## 9. Risks and mitigations

**Risk:** Test-utils endpoints compiled into a production build if
`--all-features` is used outside tests.

Mitigation: Routes are registered only when both `test-utils` is compiled in
AND `UPTRAKIT_TEST_UTILS_ENABLED=true` is set (§5.2). Both `Cargo.toml`
feature declarations carry a `# test-only feature — do not enable in
production builds` comment. The production Dockerfile does not set
`UPTRAKIT_TEST_UTILS_ENABLED`. `cargo deny` does not support feature-usage
gating; the env var guard, code review, and Dockerfile separation are the real
controls. The `TestUtilsAllowed` extractor makes the no-auth policy explicit and
auditable in code review.

**Risk:** P-384 keys break an existing interop assumption with a reverse-proxy
that requires P-256 for mTLS client validation.

Mitigation: existing reverse proxy integration tests exercise mTLS with the new
key type after the migration. Any breakage surfaces in CI before shipping. The
reverse proxy configurations (nginx, haproxy, envoy, traefik, caddy) are
algorithm-agnostic for ECDSA chains.

**Risk:** `wait_for_cert_renewed` times out in slow CI environments.

Mitigation: the 60s timeout covers the Agent's renewal round-trip (CSR
generation → send → Controller sign → Certificate wire message → Agent
persist). The full cycle completes in under 5s on any non-pathological
network. The timeout can be raised without design changes if CI proves slow.

## 10. References

- Parent spec: `docs/superpowers/specs/2026-05-12-mtls-hardening-design.md`
- ADR-0013: `docs/adr/0013-defer-root-intermediate-ca-split.md`
- `docs/security/pki-certificates.md`
- `docs/development/quality-gates.md`
- NIST FIPS 186-5 (P-384 specification)
- rcgen 0.x docs: `PKCS_ECDSA_P384_SHA384`
- aws-lc-rs docs: `ECDSA_P384_SHA384_ASN1_SIGNING`
