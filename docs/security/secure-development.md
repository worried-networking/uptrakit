# Secure Development

Developers must consult [Coding Standards](../development/coding-standards.md) for panic policies and design
boundaries, and [Error Handling](../development/error-handling.md) for rootcause/thiserror patterns and the full
decision guide. Security-sensitive changes should also reference:

- [PKI and certificates](pki-certificates.md)
- [Secrets and encryption](secrets-and-encryption.md)
- [Reverse proxy security](reverse-proxy-security.md)
- [Filesystem and dependency security](filesystem-dependency-security.md)
- [CLI output formatting](../development/cli-output.md)

Document any new behavior or configuration in the appropriate `docs/` area and ensure tests cover both success and failure paths.

Build metadata exposed by `--version` is intentionally non-secret (crate version, enabled build features, target/cfg/profile). Never include
credentials, tokens, private keys, or runtime secret material in any version/build output.
