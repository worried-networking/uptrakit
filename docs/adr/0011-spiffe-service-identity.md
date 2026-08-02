# 0011 — SPIFFE Service Identity

Date: 2026-05-13

## Status

Accepted

## Context

Service certificates today carry `CN=<service_id>` only. RFC 6125 deprecates CN-based identity in favor of Subject Alternative
Names. Service mesh ecosystems (SPIRE, Istio, Linkerd) standardize on SPIFFE URIs as the workload-identity carrier.

Adopting SPIFFE now:

- Aligns uptrakit with CNCF-graduated workload-identity tooling.
- Opens future federation (SPIFFE-Workload-API token exchange) without a second identity migration.
- Costs ~40 LOC + a `trust_domain` config knob.

URN-UUID (`urn:uuid:<service_id>`) was the simpler alternative. Rejected because URN-UUID has no ecosystem and produces no future-interop value.

## Decision

Every Service certificate carries:

- `Subject: CN=<service_id>` (preserved during the renewal-tail migration window; removed in a follow-up spec).
- `Subject Alternative Name: URI = spiffe://<trust_domain>/service/<service_id>`.

The Controller's `[tls] trust_domain` is configured by the Operator (defaults to first server-cert SAN). The Controller
advertises the value to every connecting Service via the `ServiceSettingsPayload.trust_domain` wire field. The CSR signer
rejects any CSR whose SPIFFE URI does not match the configured trust domain.

Identity extraction prefers the SPIFFE SAN; falls back to CN for ≤2 years (one max-lifetime renewal cycle). A follow-up spec
removes the CN fallback after the renewal tail.

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
