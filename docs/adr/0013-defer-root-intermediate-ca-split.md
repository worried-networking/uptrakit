# ADR-0013: Defer Root/Intermediate Managed CA Split

Status: Accepted (deferral)

## Context

The mTLS hardening spec (`docs/superpowers/specs/2026-05-12-mtls-hardening-design.md`) introduces SPKI pinning
(`--tofu-spki`) as a TOFU mode. SPKI pinning survives certificate **renewal** (the keypair stays stable) but breaks at
managed-CA **rotation** because rotation introduces a fresh keypair.

A two-tier managed CA (long-lived root signs short-lived issuing intermediates) would let an Agent SPKI-pin the root and
survive rotation freely — the intermediate rotates without the pin breaking.

This is a meaningful but separable improvement:

- Adds DB schema (parent_fingerprint on `ca_certificate`).
- Splits cert-sign / OCSP-sign / CRL-sign to use the intermediate.
- Adds a root-rotation ceremony (rare, manual).

The benefit materializes once per ~5 years per fleet.

## Decision

Defer Path A (root/intermediate split) to its own spec + plan cycle. Operators wanting rotation-survivable pin durability
today use the external-CA path (`--ca-cert` / `--ca-key`) with their own existing PKI (Vault, AD CS, step-ca).

This ADR records the deferral so future contributors do not re-litigate the decision without re-reading the spec.

## Consequences

- Managed-CA SPKI pin durability matches fingerprint pin durability (breaks at every rotation, ~5 years).
- External CA users get true SPKI pin durability today, no code change required.
- Future Path A spec will revisit this ADR and supersede it.

## Alternatives considered

| Option                                                   | Outcome                                                                              |
| -------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| Ship Path A in the current hardening spec                | Rejected — significantly enlarges scope; benefit materializes once per 5 years.      |
| Reuse the keypair across CA rotations (same-key renewal) | Rejected — defeats the purpose of rotation (key hygiene).                            |
| Cross-sign managed CA with a public root                 | Rejected — Baseline Requirements forbid name-unconstrained public subordinates.      |

## Related

- `docs/superpowers/specs/2026-05-12-mtls-hardening-design.md` §8 (Future Work)
- ADR-0011, ADR-0012
