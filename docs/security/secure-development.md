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

## Plugin Input Validation

Plugins are a security boundary — they interpolate user-controlled values (package identifiers,
version strings) into system commands. All input that flows into `CommandSpec` arguments must be
validated at the plugin level before command construction.

### Package identifier validation

Every plugin that accepts a `package_identifier` parameter must validate it via a
`validate_identifier()` function before any use. The validation enforces:

- Character whitelist specific to the package manager's naming rules
- Length bounds (typically 2–128 characters)
- Path traversal rejection (`..` segments)
- First-character constraints (e.g., must start with a letter or digit)

See [Plugin Guidelines — Package identifier validation](../development/plugin-guidelines.md#package-identifier-validation)
for the implementation pattern.

### Version string validation

Plugins that interpolate a `to_version` parameter into install commands (e.g.,
`npm install -g pkg@version`, `apt-get install pkg=version`) must validate the version string.
Even though `CommandSpec::exec()` mode prevents shell injection, package managers have their
own argument parsing:

- **npm:** A version like `file:../malicious` or `git+https://attacker.com` could install
  attacker-controlled packages.
- **apt:** A version like `1.0 --allow-unauthenticated` could alter command behavior through
  flag injection.

Validation rules per plugin:

| Plugin | Allowed characters | Rejected patterns |
| :--- | :--- | :--- |
| npm | `[a-zA-Z0-9._+-]` | Empty, >256 chars, `file:`, `git+`, `http:`, `https:` prefixes |
| apt | `[a-zA-Z0-9.+~:-]` | Empty, >256 chars, leading `-` (flag injection) |

See [Plugin Guidelines — Version string validation](../development/plugin-guidelines.md#version-string-validation)
for the implementation pattern and testing requirements.

### Atomic ordering for security flags

`AtomicBool` flags that control security-sensitive behavior (such as `PLAINTEXT_MODE` in
`uptrakit-crypto`) must use `Ordering::Release` for stores and `Ordering::Acquire` for loads.
See [Coding Standards — Atomic Ordering Requirements](../development/coding-standards.md#atomic-ordering-requirements).

Build metadata exposed by `--version` is intentionally non-secret (crate version, enabled build features, target/cfg/profile). Never include
credentials, tokens, private keys, or runtime secret material in any version/build output.
