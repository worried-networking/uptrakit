# 0012 — Agent Trust Composition

Date: 2026-05-13

## Status

Accepted

## Context

Agent / Service-SDK builds its `rustls::RootCertStore` from the controller-CA bundle alone today. This works for the
canonical "all managed, self-signed" deployment but blocks Operators who want to front the Controller with a public-CA
certificate (Let's Encrypt etc.) or who run Agents inside corporate networks with their own internal CAs.

Three trust sources are available:

1. `webpki-roots` (compiled-in major public roots).
2. `rustls-native-certs` (OS root store, including corporate roots installed via MDM).
3. The controller-CA bundle delivered via `CaBundleUpdated`.

Naïvely unioning all three is unsafe: a corporate MITM root (Zscaler, Netskope) in the OS store would silently authorize
any host the proxy presents. This is the canonical motivation for certificate pinning in mobile apps and not a property
uptrakit can shed by default.

## Decision

Trust sources are **explicit, additive opt-ins**:

- Default: controller-CA bundle only (today's behavior — no change for existing deployments).
- `--trust-public-roots`: add compiled-in `webpki-roots`.
- `--trust-native-roots`: add `rustls-native-certs` (OS store at process startup).

Flags compose. The Operator declares the deployment shape.

Hostname verification (`ServerName`) is enforced in every mode unless the Operator opts out via `--tofu-skip-hostname`
(only valid alongside one of the pin / insecure TOFU flags — see ADR-0011's sibling discussion in the spec).

## Consequences

- Operators upgrading from earlier releases see no change in trust posture.
- LE-fronted Controller deployments require `--trust-public-roots` (documented in `docs/security/tofu-tls.md`).
- Corporate-internal-CA-only Agents use `--trust-native-roots` alone (without `--trust-public-roots`) to capture the
  corporate root while excluding public CAs.
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
