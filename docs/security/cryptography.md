# Cryptographic Details

| Component | Library | Notes |
| --- | --- | --- |
| TLS | [Rustls](https://github.com/rustls/rustls) (aws-lc-rs backend) | Secures all controller HTTPS and agent WebSocket connections. |
| CA Key | ECDSA P-256 | Used for the managed CA and all issued certs. |
| Certificate Hashing | SHA-256 | Signing, CRL generation, OCSP responses. |
| Password Hashing | Argon2id (OWASP parameters: 19 MiB, 2 iterations) | Stores user passwords. |
| JWT Signing | `jsonwebtoken` | Signs access and refresh tokens. |
| Session Tokens | SHA-256 hashed, 7-day expiry, rotated on every use | Prevents replay attacks. |
| Encryption At Rest | AES-256-GCM (`aes-gcm` crate) | Encrypts MQTT passwords, OIDC secrets, CA keys. |
| TOFU Verification | `TofuVerifier` with SHA-256 fingerprints | Secures CA bootstrap with signature verification. |

No custom cryptographic primitives are implemented; the project relies on audited crates and hard-coded parameters.
