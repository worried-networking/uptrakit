---
title: Cryptographic Details
weight: 20
description: Cryptographic primitives used by Uptrakit, including TLS, key hierarchies, password hashing, and encryption-at-rest algorithms.
---

# Cryptographic Details

| Component | Library | Notes |
| --- | --- | --- |
| TLS | [Rustls](https://github.com/rustls/rustls) (aws-lc-rs backend) | Secures all controller HTTPS and agent WebSocket connections. |
| CA Key | ECDSA P-256 | Used for the managed CA and all issued certs. |
| Certificate Hashing | SHA-256 | Signing, CRL generation, OCSP responses. |
| Password Hashing | Argon2id (OWASP parameters: 19 MiB, 2 iterations) | Stores user passwords. |
| JWT Signing | `jsonwebtoken` | Signs access and refresh tokens. |
| Session Tokens | SHA-256 hashed, 7-day expiry, rotated on every use | Prevents replay attacks. |
| Encryption At Rest | AES-256-GCM (`aws-lc-rs` crate) | Envelope encryption: KEK wraps DEKs; DEKs encrypt data. `ENC:v3:<key_id>:<hex>` format with per-column AAD. Random 96-bit nonces; birthday-bound collision at ~2^48 encryptions (per NIST SP 800-38D). O(1) master key rotation. |
| DEK Key ID | SHA-256 (first 8 hex chars) | Identifies which DEK encrypted a value. Embedded in the `ENC:v3:` ciphertext prefix. |
| TOFU Verification | `TofuVerifier` with SHA-256 fingerprints | Secures CA bootstrap with signature verification. |

No custom cryptographic primitives are implemented; the project relies on audited crates and hard-coded parameters.

See [Secrets and Encryption](secrets-and-encryption.md) for operational details and
[Key Rotation](key-rotation.md) for the master key rotation procedure.
