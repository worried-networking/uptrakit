# Filesystem and Dependency Security

## Filesystem Permissions

- Directories are created with `0o700` permissions, files with `0o600`.
- The `uptrakit-directories` crate enforces secure creation via `create_secure_dir` and `write_secure_file`.
- Applies to controller config/state (CA keys, database) and agent/MQTT state directories.

## Dependency Security

- `cargo-deny` runs in CI to check RustSec advisories, license compliance, and dependency anomalies.
- Dependabot tracks Cargo, npm, and GitHub Action dependencies weekly with automatic PRs.
- Dependencies touching command execution, parsing untrusted input, cryptography, or networking receive extra scrutiny during review.
