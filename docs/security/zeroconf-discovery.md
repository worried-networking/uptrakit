---
title: Zero-Configuration Discovery Security
weight: 180
description: Threat model and security mitigations for Uptrakit's mDNS/DNS-SD zero-configuration discovery feature.
---

# Zero-Configuration Discovery Security

This document describes the threat model and security mitigations for Uptrakit's mDNS/DNS-SD zero-configuration
discovery feature. For setup instructions, see
[Zero-Configuration Discovery (End-User)](../end-user/zeroconf-discovery.md). For implementation details, see
[Zero-Configuration Discovery (Development)](https://github.com/worried-networking/uptrakit/tree/main/docs/development/).

## Threat Model

mDNS (RFC 6762) operates over multicast UDP on the local network segment. It is inherently unauthenticated --
any device on the same LAN can respond to mDNS queries. DNS-SD (RFC 6763) builds service discovery on top of
mDNS but does not add authentication.

This means that an attacker with access to the same network segment can:

1. **Spoof mDNS responses.** The attacker advertises a fake `_uptrakit._tcp.local.` service pointing to their
   own host.
2. **Present their own CA.** The attacker runs a rogue controller that serves its own CA certificate during the
   TOFU bootstrap.
3. **Intercept service enrollment.** The service enrolls with the attacker's controller instead of the
   legitimate one, giving the attacker control over the service.

This is a classic man-in-the-middle (MITM) scenario that applies to all mDNS-based discovery protocols.
Uptrakit mitigates this through CA fingerprint verification and cache persistence.

## Mitigations

### 1. CA Fingerprint Verification

The controller includes its CA certificate fingerprint (`ca_fp`) in the mDNS TXT record. When a service
discovers a controller, it logs the fingerprint prominently:

```text
WARN CA fingerprint: SHA256:a1b2c3d4e5f6...
WARN Verify this matches the controller's fingerprint to prevent MITM.
```

The controller also logs its fingerprint at startup:

```text
INFO mDNS advertising enabled. CA fingerprint: SHA256:a1b2c3d4e5f6...
```

The user compares these two values out-of-band (visually, over a trusted channel). If they match, the
discovered controller is authentic. If they do not match, a MITM attack may be in progress.

When the discovered `ca_fp` is present, the service implicitly enables TOFU mode with the discovered
fingerprint as the expected value. This means `bootstrap_ca()` verifies the downloaded CA certificate against
the fingerprint before accepting it. An attacker who spoofs the mDNS response but cannot produce a CA
certificate matching the legitimate controller's fingerprint will be rejected automatically.

However, if the attacker also controls the `ca_fp` value in the spoofed TXT record (which they do, since mDNS
is unauthenticated), they can set it to match their own CA. This is why out-of-band verification or
`--tofu-fingerprint` is essential.

### 2. Strict Fingerprint Pinning (`--tofu-fingerprint`)

For environments where out-of-band verification is impractical or insufficient, the `--tofu-fingerprint` flag
provides strict verification:

```sh
uptrakit-agent --tofu-fingerprint a1b2c3d4e5f6...
```

The service compares the CA certificate downloaded during TOFU bootstrap against this pinned fingerprint.
If the fingerprint does not match, the service aborts immediately. This defeats MITM attacks regardless of
mDNS spoofing, because the attacker cannot produce a CA certificate that matches the legitimate controller's
fingerprint.

The `--tofu-fingerprint` value takes precedence over any fingerprint discovered via mDNS.

### 3. Cache Persistence

After the first successful discovery, the resolved URL and CA fingerprint are saved to `discovery.json` in
the service's state directory. On subsequent restarts, the service loads the cached result directly -- it does
not perform a new mDNS browse.

This means that even if an attacker begins spoofing mDNS responses after the initial discovery, existing
services are unaffected. The attack window is limited to the first connection of a new service.

### 4. Secure File Permissions

The `discovery.json` cache file is written with `0o600` permissions (owner read/write only) via
`uptrakit_directories::write_secure_file_str()`. This prevents other users on the same host from reading or
tampering with the cached discovery result.

## Recommendations for High-Security Environments

For environments where mDNS-based discovery is not appropriate, Uptrakit supports fully explicit configuration
that eliminates any trust-on-first-use window:

### Option 1: Explicit URL with CA certificate (no TOFU, no mDNS)

```sh
uptrakit-agent --url https://controller.example.com:8443 --ca-cert /path/to/ca.pem
```

The service uses the provided CA certificate directly for TLS verification. No mDNS browsing or TOFU
bootstrap occurs.

### Option 2: Explicit URL with fingerprint pinning (TOFU with pinning, no mDNS)

```sh
uptrakit-agent --url https://controller.example.com:8443 --tofu-fingerprint a1b2c3d4e5f6...
```

The service downloads the CA certificate via TOFU but verifies it against the pinned fingerprint before
accepting it. No mDNS browsing occurs.

### Option 3: Disable zeroconf at compile time

When the `zeroconf` feature is disabled, the `--url` flag becomes mandatory and all mDNS code is compiled
out. This eliminates any possibility of accidental mDNS-based discovery.

## Summary

| Scenario | mDNS | TOFU | Attack surface |
| --- | :---: | :---: | --- |
| `--url` + `--ca-cert` | No | No | None (fully pinned) |
| `--url` + `--tofu-fingerprint` | No | Pinned | First connection only; fingerprint verified |
| `--url` (no cert, no fingerprint) | No | Yes | First connection; user must verify CA manually |
| No `--url` (mDNS discovery) | Yes | Pinned | First connection; fingerprint from mDNS TXT record |
| No `--url` + `--tofu-fingerprint` | Yes | Pinned | First connection; user-provided fingerprint overrides mDNS |

Zero-configuration discovery is designed for trusted LAN environments (home lab, private infrastructure) where
the convenience of automatic discovery outweighs the risk of mDNS spoofing. For untrusted networks, WAN
deployments, or regulated environments, use explicit `--url` with `--ca-cert` or `--tofu-fingerprint`.

## Related Documentation

- [Zero-Configuration Discovery (End-User)](../end-user/zeroconf-discovery.md) -- setup instructions
- [Zero-Configuration Discovery (Development)](https://github.com/worried-networking/uptrakit/tree/main/docs/development/) -- architecture and internals
- [TOFU and TLS Hardening](tofu-tls.md) -- `TofuVerifier` and fingerprint pinning details
- [Secrets and Encryption](secrets-and-encryption.md) -- general encryption and credential handling
- [Security Architecture](security-architecture.md) -- overall security design
