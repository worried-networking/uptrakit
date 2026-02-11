# TOFU and TLS Hardening

## TofuVerifier

The controller uses `TofuVerifier` with SHA-256 fingerprint pinning during initial CA bootstrap. It always verifies the TLS signature and only skips
CA chain validation.

## Fingerprint Pinning

- Use `--tofu` with `--tofu-fingerprint` to pin the CA certificate fingerprint (32-byte SHA-256, colon-separated or not).
- `bootstrap_ca()` compares the fingerprint computed via `ca_pem_fingerprint()` to the expected value and aborts on mismatch.

## CLI `--insecure`

The CLI client (`crates/ui/cli/src/client.rs`) no longer calls `tls_danger_accept_invalid_certs(true)`. Passing `--insecure` is the only way to skip
TLS verification, making the risk explicit to operators.
