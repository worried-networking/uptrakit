---
title: TOFU and TLS Hardening
weight: 60
description: TofuVerifier behavior, fingerprint pinning, and TLS trust bootstrap considerations for Uptrakit agents and services.
---

# TOFU and TLS Hardening

## TofuVerifier

The `TofuVerifier` (`crates/shared/service-sdk/src/tls.rs`) accepts any server certificate chain but
still delegates TLS handshake signature verification to the installed crypto provider. This prevents
trivial MITM attacks where an attacker presents a certificate with an invalid signature.

The CA fetch path in `bootstrap_ca()` (`crates/shared/service-sdk/src/ca.rs`) uses
`build_tofu_client_config()` with `reqwest::ClientBuilder::use_preconfigured_tls()` to apply the
`TofuVerifier` during TOFU mode. This replaces the previous `tls_danger_accept_invalid_certs(true)`
which disabled all TLS verification at the reqwest level.

After download, SHA-256 fingerprint pinning (`--tofu-fingerprint`) provides the primary security
guarantee.

## Fingerprint Pinning

- Use `--tofu` with `--tofu-fingerprint` to pin the CA certificate fingerprint (32-byte SHA-256, colon-separated or not).
- `bootstrap_ca()` compares the fingerprint computed via `ca_pem_fingerprint()` to the expected value and aborts on mismatch.

## CLI TLS Options

The CLI client (`crates/ui/cli/src/client.rs`) uses the system trust store by default for TLS verification. The
previous hardcoded `tls_danger_accept_invalid_certs(true)` has been removed. An explicit `--insecure` flag is now
required to skip TLS certificate verification:

| Flag | Default | Description |
| --- | --- | --- |
| `--insecure` | `false` | Skip TLS certificate verification (self-signed certs). Use only for development or initial setup. |
