# ATK-04: TOFU Bootstrap Man-in-the-Middle

| Field | Value |
| --- | --- |
| Severity | High |
| Attack surface | TLS / PKI (initial CA bootstrap) |
| Prerequisites | Network position between agent and controller during first connection |
| STRIDE | Spoofing |

## Attack description

1. A new agent (or SSH agent) starts for the first time with `--tofu` enabled and no
   `--tofu-fingerprint` specified.
2. The agent calls `bootstrap_ca()` which fetches the controller's CA certificate over
   HTTPS. The reqwest client is configured with
   `tls_danger_accept_invalid_certs(true)`, fully disabling TLS certificate
   verification.
3. An attacker with a network man-in-the-middle position intercepts this HTTPS request
   and presents their own CA certificate.
4. The agent accepts the attacker's CA, saves it to `ca.pem`, and uses it as the trust
   anchor for all subsequent TLS connections.
5. The attacker can now:
   - **Intercept all future agent-controller traffic** by presenting server
     certificates signed by the attacker's CA.
   - **Capture the enrollment secret** during the enrollment bearer-token phase.
   - **Issue fake mTLS certificates** to the agent, completing the enrollment.
   - **Inject arbitrary controller messages** into the WebSocket, including
     `execute_update` with malicious hook commands.

## Worst-case impact

- **Full agent compromise.** The attacker controls all communication between the agent
  and the controller, enabling arbitrary command execution on the agent host via
  crafted `ExecuteUpdate` payloads.
- **Credential theft.** The attacker intercepts plugin configs containing API tokens,
  registry credentials, and MQTT broker passwords sent via `check_versions` messages.
- **Invisible persistence.** The agent trusts the attacker's CA permanently. Even if
  the network position is lost, the attacker can reconnect later using certificates
  signed by the planted CA.
- **Enrollment secret capture.** The enrollment secret (used for bearer auth before
  mTLS) is exposed in transit, allowing the attacker to impersonate the agent to the
  real controller.

## Current mitigations

- **Optional fingerprint pinning.** The `--tofu-fingerprint` flag allows operators to
  specify the expected SHA-256 fingerprint of the CA certificate. When provided,
  `bootstrap_ca()` compares the fingerprint and aborts on mismatch.
- **SSH agent strict host key checking.** The `--strict-host-key-checking` flag
  combined with `--host-key-fingerprint` provides strong pinning for SSH connections,
  requiring pre-verified host keys.
- **TOFU warning log.** When TOFU accepts a CA without fingerprint verification, a
  `WARN`-level log message is emitted with the observed fingerprint, giving operators
  a chance to detect unexpected CAs.
- **CA staleness detection.** After initial bootstrap, the agent detects CA bundle
  changes via `ca_bundle_hash` comparison and re-fetches over the already-trusted TLS
  connection (not TOFU).
- **Alternative bootstrap paths.** Operators can bypass TOFU entirely by providing the
  CA certificate via `--ca-cert` file or fetching it over a TLS connection validated
  by the system trust store via `--pki-addr`.
- **Short-lived certificates.** Even if a rogue CA is accepted, agent certificates
  issued by the real controller will not validate against the rogue CA's trust chain,
  limiting the blast radius if the agent later obtains a legitimate CA.

## Residual risk

- **TOFU is the default.** When neither `--ca-cert` nor `--tofu-fingerprint` is
  provided, TOFU mode accepts any CA certificate presented during the first
  connection. Many operators may use the default without understanding the risk.
- **`tls_danger_accept_invalid_certs(true)` on the fetch path.** The reqwest client
  used for CA fetching completely disables TLS verification, not just CA chain
  validation. This means the attacker does not even need a valid TLS certificate to
  intercept the request.
- **Single point of failure.** The CA certificate is fetched once and cached forever.
  A successful MITM during this single request compromises the entire trust
  relationship permanently.
- **No post-bootstrap verification.** There is no mechanism for the agent to
  retroactively verify that its cached CA matches the controller's actual CA (e.g.,
  via an out-of-band channel).

## Recommended improvements

- Make `--tofu-fingerprint` required when `--tofu` is used, or at minimum emit a
  prominent startup warning (not just a log line) when TOFU is used without
  fingerprint pinning.
- Replace `tls_danger_accept_invalid_certs(true)` in the CA fetch path with the
  `TofuVerifier` that at least validates TLS handshake signatures, preventing passive
  interception.
- Provide a CLI command to verify the locally cached CA against the controller's
  current CA fingerprint (e.g., `uptrakit-agent verify-ca`), enabling post-bootstrap
  trust verification.
- Document a recommended bootstrap procedure that uses `--ca-cert` or
  `--tofu-fingerprint` in production, and clearly label bare `--tofu` as a
  development-only convenience.
- Consider implementing certificate transparency logging for the managed CA, allowing
  operators to detect unexpected CA certificates.

## References

- [TOFU and TLS](../security/tofu-tls.md)
- [PKI and Certificates](../security/pki-certificates.md)
- [SSH Agent Secrets — Bootstrap Security Model](../security/ssh-agent-secrets.md#bootstrap-security-model)
- `crates/shared/service-sdk/src/ca.rs` — `bootstrap_ca()`
- `crates/shared/service-sdk/src/tls.rs` — `TofuVerifier`,
  `build_tofu_client_config()`
