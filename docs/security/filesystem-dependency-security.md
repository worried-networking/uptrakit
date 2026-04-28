---
title: Filesystem and Dependency Security
weight: 70
description: Filesystem permission hardening, TOCTOU-safe file creation, path traversal prevention, and dependency security safeguards in Uptrakit.
---

# Filesystem and Dependency Security

## Filesystem Permissions

- Directories are created with `0o700` permissions, files with `0o600`.
- The `uptrakit-directories` crate enforces secure creation via `create_secure_dir` and `write_secure_file`
  (plus async variants `write_secure_file_async` / `write_secure_file_str_async`).
- On Unix, permissions are set **atomically at creation time** using `OpenOptionsExt::mode(0o600)` for files and
  `DirBuilderExt::mode(0o700)` for directories. This eliminates the TOCTOU window where a file could be briefly
  world-readable between creation and `chmod`.
- When creating nested directory hierarchies, `create_secure_dir` ensures all intermediate directories (not just the
  leaf) receive `0o700` permissions. After the recursive `DirBuilder` call, it walks all newly created path components
  and applies `set_dir_permissions()` to each one.
- Applies to controller config/state (CA keys, database), agent state (service certificates, private keys), and
  MQTT state directories.
- All crates that write sensitive files (`controller/pki`, `service-sdk/identity`, `agent/client`) delegate to the
  `uptrakit-directories` crate rather than using raw `fs::write` / `tokio::fs::write`.

## Path Traversal Prevention

`AppDirs::config_path()` and `AppDirs::state_path()` validate the `name` argument before joining it to the base
directory. The `validate_path_name()` function rejects:

- Empty strings
- Names containing path separators (`/` or `\`)
- Relative path components (`.` or `..`)
- Absolute paths

This prevents path traversal attacks where a malicious or malformed name could escape the intended config/state
directory via `..` components or by replacing the base path entirely (since `PathBuf::join` with an absolute path
discards the base). Both methods return `Result<PathBuf>` with a `DirectoryError::PathTraversal` error on violation.

See also: [Secrets Handling and Encryption](secrets-and-encryption.md) for master key zeroization.

## Dependency Security

- `cargo-deny` runs in CI to check RustSec advisories, license compliance, and dependency anomalies.
- Dependabot tracks Cargo, npm, and GitHub Action dependencies weekly with automatic PRs.
- Dependencies touching command execution, parsing untrusted input, cryptography, or networking receive extra scrutiny during review.
