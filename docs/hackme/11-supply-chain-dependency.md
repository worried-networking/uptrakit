# ATK-11: Supply Chain Dependency Attack

| Field | Value |
| --- | --- |
| Severity | Medium |
| Attack surface | Build / dependencies |
| Prerequisites | Compromise of a crate on crates.io or a transitive dependency |
| STRIDE | Tampering |

## Attack description

1. An attacker publishes a malicious version of a crate that Uptrakit depends on
   (directly or transitively).
2. The malicious crate introduces a backdoor, data exfiltration routine, or
   vulnerability into the compiled Uptrakit binary.
3. The compromise propagates through the next build and release cycle, affecting all
   deployed instances that update to the affected version.

High-value targets in the dependency tree:

| Crate | Role | Impact of compromise |
| --- | --- | --- |
| `rustls` / `aws-lc-rs` | TLS and cryptography | Key extraction, MITM, broken encryption |
| `argon2` | Password hashing | Weakened hashing, credential theft |
| `jsonwebtoken` | JWT signing/verification | Token forgery, auth bypass |
| `openidconnect` | OIDC protocol | Auth bypass, token manipulation |
| `reqwest` | HTTP client | Request interception, SSRF |
| `serde` / `serde_json` | Serialization | Data manipulation, code execution |
| `rcgen` | Certificate generation | Rogue certificates, PKI compromise |
| `rumqttc` | MQTT client | Broker credential theft, message manipulation |
| `sea-orm` | Database ORM | SQL injection, data exfiltration |
| `mail-send` | SMTP email | Credential theft, email manipulation |

## Worst-case impact

- **Full system compromise.** A backdoor in a core dependency (e.g., `rustls`,
  `serde`) provides the attacker with arbitrary code execution in the context of the
  controller or agent process.
- **Credential theft.** A compromised crypto or auth crate can exfiltrate master keys,
  JWT signing keys, passwords, and API tokens.
- **Silent persistence.** Supply chain attacks are difficult to detect because the
  malicious code runs within the trusted binary, with full access to process memory
  and network.
- **Cascading impact.** The controller distributes plugin configs and commands to all
  agents. A compromised controller binary can inject malicious payloads into the
  entire managed fleet.

## Current mitigations

- **`cargo-deny` in CI.** The `deny.toml` configuration runs RustSec advisory checks,
  license compliance, and source validation in CI. Known vulnerabilities trigger build
  failures.
- **Source restrictions.** `deny.toml` allows only `crates.io` as a registry source.
  No git sources are permitted (`allow-git = []`). Unknown registries and git sources
  produce warnings.
- **Dependabot monitoring.** Dependabot tracks Cargo, npm, and GitHub Actions
  dependencies weekly and creates automatic PRs for version updates.
- **Workspace pinning.** All dependencies are workspace-pinned by major version in the
  root `Cargo.toml`. No `*` wildcard versions are used.
- **Strict compiler lints.** `warnings = "deny"` and `clippy::all = "deny"` are set
  as workspace lints, catching many categories of suspicious or incorrect code.
- **Release hardening.** The release profile uses `panic = "abort"` (eliminating
  unwinding-based exploits), `strip = true` (removing debug symbols), and `lto = "fat"`
  (whole-program optimization that may remove dead code paths).
- **`rustls` over `native-tls`.** All HTTP clients use the `rustls` feature of
  `reqwest`, avoiding the OpenSSL dependency and its historical vulnerability surface.

## Residual risk

- **Known advisory exceptions.** Three RustSec advisories are currently ignored in
  `deny.toml`:
  - `RUSTSEC-2025-0134` (`rustls-pemfile` unmaintained, via `rumqttc`) — low risk.
  - `RUSTSEC-2024-0436` (`paste` unmaintained, via `utoipa-axum`) — low risk
    (proc-macro build dependency).
  - `RUSTSEC-2023-0071` (`rsa` Marvin Attack, via `openidconnect`) — medium risk.
    Documented as non-exploitable for public-key verification, but no patched crate
    exists and no remediation timeline is tracked.
- **Multiple-versions policy is `warn`, not `deny`.** Duplicate crate versions produce
  warnings only, potentially allowing conflicting versions with different security
  properties.
- **Proc-macro build dependencies.** Proc-macro crates execute arbitrary code at
  compile time. A compromised proc-macro (e.g., `serde_derive`, `tokio-macros`)
  could inject code into the binary without appearing in runtime dependency audits.
- **No reproducible builds.** Without reproducible builds, it is difficult to verify
  that the released binary was compiled from the expected source code.
- **Transitive dependency depth.** The full dependency tree includes hundreds of
  transitive crates. Manual review of all transitive dependencies is impractical.

## Recommended improvements

- Promote `multiple-versions` from `warn` to `deny` in `deny.toml` to eliminate
  duplicate crate versions that may have different security properties.
- Track remediation timelines for ignored advisories and set calendar reminders to
  re-evaluate them when upstream fixes become available.
- Consider enabling `cargo-vet` or `cargo-crev` for supply chain trust verification,
  requiring explicit audit attestations for security-critical dependencies.
- Implement reproducible builds to enable binary verification against source code.
- Add a CI check that flags newly added dependencies touching cryptography, command
  execution, or network I/O for mandatory security review.
- Pin exact versions (not just major) for the most security-critical dependencies
  (`rustls`, `aws-lc-rs`, `argon2`, `jsonwebtoken`) to prevent automatic minor
  version bumps from introducing compromised code.

## References

- [Filesystem and Dependency Security](../security/filesystem-dependency-security.md)
- [Quality Gates](../development/quality-gates.md)
- [Dependency Policy](../development/dependency-policy.md)
- `Cargo.toml` — workspace dependency declarations
- `deny.toml` — `cargo-deny` configuration
