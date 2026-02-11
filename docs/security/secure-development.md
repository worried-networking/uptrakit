# Secure Development

Developers must consult [docs/development/coding-standards.md](../development/coding-standards.md) for error handling, panic policies, and design
boundaries. Security-sensitive changes should also reference:

- [PKI and certificates](pki-certificates.md)
- [Secrets and encryption](secrets-and-encryption.md)
- [Reverse proxy security](reverse-proxy/index.md)
- [Filesystem and dependency security](filesystem-dependency-security.md)

Document any new behavior or configuration in the appropriate `docs/` area and ensure tests cover both success and failure paths.
