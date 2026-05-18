# ADR 0016 — P-384 for CA, P-256 for Server/Service/ECIES Certs

Date: 2026-05-18
Status: Accepted

## Context

The mTLS hardening spec (`2026-05-12-mtls-hardening-design.md`) introduced P-256 as the
uniform key algorithm across all certificate roles. The mTLS follow-up spec
(`2026-05-16-mtls-followup-design.md`) partially migrated service TLS to P-384 for a wider
classical security margin, but left `sealed_box_decrypt` in `sensitive_params.rs` hardcoded to
`ECDH_P256`, breaking the sealed-box flow for all enrolled standalone services.

## Decision

- **CA certs: P-384.** Highest-value long-lived key material; wider classical security margin
  justifies the larger key size.
- **Server TLS certs: P-256.** Envoy ≤ 1.32 supports only P-256 for static TLS config; P-256 is
  the safe default for server-facing material that must pass through arbitrary reverse proxies.
- **Service TLS certs (enrollment + renewal): P-256.** Leaf certs are short-lived (max 2 years).
  P-256 is NIST-recommended for TLS at ≤128-bit security level. Reusing the TLS keypair for ECIES
  (which is hardcoded to `ECDH_P256` in `aws_lc_rs`) eliminates the dual-keypair split. The
  combination of P-384 CA + P-256 leaf is standard practice (cf. Let's Encrypt R3/E1 chain).
- **ECIES keypair: reuse service TLS keypair (P-256).** No separate ECIES keypair for enrolled
  services. Embedded services retain their ephemeral P-256 keypair (generated in
  `run_embedded_service`; never persisted).

## Alternatives Rejected

| Option                                                                 | Outcome                                                                                                                                     |
| ---------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| Service TLS stays P-384; update `sensitive_params.rs` to `ECDH_P384`   | Rejected — requires updating all ECIES encrypt callers (CLI, surface-proxy); adds complexity with no net benefit for short-lived leaf certs |
| Service TLS stays P-384; keep separate P-256 ECIES keypair per service | Rejected — dual-keypair complexity, two on-disk files, identity fragmentation                                                               |
| Uniform P-384 everywhere including server certs                        | Rejected — breaks Envoy ≤ 1.32                                                                                                              |

## Consequences

- One keypair per enrolled service, used for both mTLS client auth and ECIES sealed-box
  decryption. No separate ECIES key material to manage.
- Existing enrolled services with P-384 keys continue to function for TLS; ECIES (already
  broken) is fixed on next renewal cycle. Operators who need ECIES restored immediately can
  force renewal by deleting `service.key` and restarting the agent.
- Future changes to the ECIES algorithm require updating `sensitive_params.rs` and
  `ProviderEncryptionAlgorithm` in `protocol.rs`. The `Other(String)` catch-all on the enum
  allows a newer agent to advertise a new algorithm without crashing an older controller.

## Related

ADR-0013, mTLS hardening spec (`2026-05-12-mtls-hardening-design.md`), embedded-service-identity
spec (`2026-05-18-embedded-service-identity.md`).
