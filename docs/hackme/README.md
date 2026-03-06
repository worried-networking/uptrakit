# Attack Vector Analysis

This directory contains a structured threat analysis of the Uptrakit codebase.
Each document describes a realistic attack scenario, assesses severity, documents
existing mitigations, identifies residual risk, and suggests improvements.

Each document tracks both existing mitigations and implemented fixes.
Resolved items are marked with ~~strikethrough~~ and a **Fixed/Done** label.

## Severity Table

| ID | Scenario | Severity | Attack Surface |
| --- | --- | --- | --- |
| [ATK-01](01-enrollment-token-brute-force.md) | Enrollment token brute force and timing | Medium | Authentication |
| [ATK-02](02-rogue-compromised-agent.md) | Rogue or compromised agent | High | Agent / wire protocol |
| [ATK-03](03-master-key-compromise.md) | Master key compromise | Critical | Cryptography |
| [ATK-04](04-tofu-bootstrap-mitm.md) | TOFU bootstrap man-in-the-middle | High | TLS / PKI |
| [ATK-05](05-mqtt-broker-compromise.md) | MQTT broker compromise | High | MQTT integration |
| [ATK-06](06-multi-tenancy-isolation-bypass.md) | Multi-tenancy isolation bypass | Medium | Database / API (future) |
| [ATK-07](07-ssrf-plugin-configuration.md) | SSRF via plugin configuration | Medium | Plugin system |
| [ATK-08](08-shell-injection-plugins.md) | Shell injection via plugins | High | Command execution |
| [ATK-09](09-discovery-result-poisoning.md) | Discovery result poisoning | Low | Discovery subsystem |
| [ATK-10](10-oidc-provider-compromise.md) | OIDC provider compromise | High | Authentication / OIDC |
| [ATK-11](11-supply-chain-dependency.md) | Supply chain dependency attack | Medium | Build / dependencies |
| [ATK-12](12-webhook-notification-ssrf.md) | Webhook notification SSRF | Medium | Notifications |
| [ATK-13](13-jwt-session-token-attacks.md) | JWT and session token attacks | Medium | Authentication |
| [ATK-14](14-scheduler-credential-exposure.md) | Scheduler credential exposure | High | External scheduler |
| [ATK-15](15-certificate-revocation-bypass.md) | Certificate revocation bypass | Medium | PKI / TLS |
| [ATK-16](16-rce-plugin-config-manipulation.md) | RCE via plugin config manipulation | Critical | Plugin system |
| [ATK-17](17-rce-controller-to-agent.md) | RCE on agents via compromised controller | Critical | Controller / wire protocol |
| [ATK-18](18-rce-deserialization-wire.md) | RCE via wire protocol deserialization | High | Wire protocol / serde |
| [ATK-19](19-rce-controller-via-api.md) | RCE on controller via API or network input | High | Controller / HTTP API |

## STRIDE Coverage

| Category | Scenarios |
| --- | --- |
| **S**poofing | ATK-01, ATK-04, ATK-10, ATK-13 |
| **T**ampering | ATK-02, ATK-05, ATK-09, ATK-18 |
| **R**epudiation | ATK-02, ATK-13 |
| **I**nformation Disclosure | ATK-03, ATK-06, ATK-07, ATK-12, ATK-14 |
| **D**enial of Service | ATK-05, ATK-15, ATK-18 |
| **E**levation of Privilege | ATK-08, ATK-10, ATK-16, ATK-17, ATK-19 |

## How to Read These Documents

Each scenario file follows a consistent template:

1. **Metadata table** with severity, attack surface, prerequisites, and STRIDE category.
2. **Attack description** with a step-by-step attack flow.
3. **Worst-case impact** describing the outcome of full exploitation.
4. **Current mitigations** listing what Uptrakit already does.
5. **Residual risk** identifying what remains despite mitigations.
6. **Recommended improvements** with actionable suggestions.
7. **References** linking to relevant security and API documentation.

## Related Documentation

- [Security Architecture](../security/security-architecture.md)
- [Auth and Authorization](../security/auth-and-authorization.md)
- [Cryptography](../security/cryptography.md)
- [PKI and Certificates](../security/pki-certificates.md)
- [Secrets and Encryption](../security/secrets-and-encryption.md)
- [Secure Development](../security/secure-development.md)
