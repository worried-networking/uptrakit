# Filesystem and Dependency Security

## Filesystem Permissions

- Directories are created with `0o700` permissions, files with `0o600`.
- The `uptrakit-directories` crate enforces secure creation via `create_secure_dir` and `write_secure_file`
  (plus async variants `write_secure_file_async` / `write_secure_file_str_async`).
- On Unix, permissions are set **atomically at creation time** using `OpenOptionsExt::mode(0o600)` for files and
  `DirBuilderExt::mode(0o700)` for directories. This eliminates the TOCTOU window where a file could be briefly
  world-readable between creation and `chmod`.
- Applies to controller config/state (CA keys, database), agent state (service certificates, private keys), and
  MQTT state directories.
- All crates that write sensitive files (`controller/pki`, `service-sdk/identity`, `agent/client`) delegate to the
  `uptrakit-directories` crate rather than using raw `fs::write` / `tokio::fs::write`.

## Dependency Security

- `cargo-deny` runs in CI to check RustSec advisories, license compliance, and dependency anomalies.
- Dependabot tracks Cargo, npm, and GitHub Action dependencies weekly with automatic PRs.
- Dependencies touching command execution, parsing untrusted input, cryptography, or networking receive extra scrutiny during review.
