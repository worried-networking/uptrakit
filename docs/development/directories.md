# Directory Management

This document covers cross-platform directory resolution, the config/state separation, CLI
overrides, and secure file/directory permissions provided by the `uptrakit-directories` crate.

## Cross-Platform Directory Resolution

All binaries (controller, agent, MQTT service, scheduler) use the `uptrakit-directories` crate for
cross-platform directory resolution. The crate uses the `directories` crate (`ProjectDirs`) to
follow platform conventions:

| Platform | Config directory                                    | State directory                                     |
| -------- | --------------------------------------------------- | --------------------------------------------------- |
| Linux    | `~/.config/{app}/` (XDG)                            | `~/.local/state/{app}/` (XDG)                       |
| macOS    | `~/Library/Application Support/org.uptrakit.{app}/` | `~/Library/Application Support/org.uptrakit.{app}/` |
| Windows  | `{FOLDERID_RoamingAppData}\uptrakit\{app}\`         | `{FOLDERID_LocalAppData}\uptrakit\{app}\`           |

Where `{app}` is one of: `controller`, `agent`, `agent-ssh`, `mqtt`, `scheduler`.

## Config vs State Separation

| Directory  | Contents                                  | Characteristics                                                          |
| ---------- | ----------------------------------------- | ------------------------------------------------------------------------ |
| **Config** | Rarely-changing, persistent configuration | External CA certificates, user-provided TLS certs                        |
| **State**  | Runtime state that may change frequently  | SQLite DB, JWT keys, service identity, private keys, issued certificates |

**Controller:**

- Config: External CA certificate/key (if configured), server TLS certificate/key
- State: SQLite database (includes managed CA history, JWT signing key)

**Agent/MQTT Service:**

- Config: Controller's CA certificate
- State: Service ID, private key, issued certificate

**SSH Agent:**

- Config: Controller's CA certificate
- State: Service ID, private key, issued certificate, local SQLite DB (`agent-ssh.db` with encrypted SSH credentials)
- Runtime: `SshAgentHandler` holds `in_flight_update: Option<InFlightUpdate>` to enforce one-update-at-a-time; the SSH
  agent is feature-complete for version checks and updates over SSH (delegates to `uptrakit-agent-core`)

## CLI Directory Flags

All binaries support `--config-dir` and `--state-dir` CLI flags (and corresponding
`UPTRAKIT_CONFIG_DIR` / `UPTRAKIT_STATE_DIR` environment variables) to override the platform
defaults. Both support `~` expansion for home directory paths.

## CLI Authentication Environment Variables

The `uptrakit` CLI binary also supports:

| Variable           | Description                                                                                                                                    |
| ------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| `UPTRAKIT_SERVER`  | Controller URL (equivalent to `--server`)                                                                                                      |
| `UPTRAKIT_TOKEN`   | API token (equivalent to `--token`)                                                                                                            |
| `UPTRAKIT_TIMEOUT` | API request timeout in seconds (equivalent to `--timeout`; default: 30). Useful for CI pipelines or operations that may take longer than 30 s. |

**Priority:** CLI flag > environment variable > stored credentials file. Using `UPTRAKIT_TOKEN` is
preferred over `--token` in automation to avoid exposing tokens in process listings.

## Secure Permissions

All created files and directories use secure permissions:

- **Directories**: 0o700 (owner read/write/execute only)
- **Files**: 0o600 (owner read/write only)

The `uptrakit-directories` crate provides helper functions (permissions are set **atomically at
creation time** on Unix, eliminating TOCTOU windows):

- `create_secure_dir(path)` — async; creates directory with 0o700 permissions using `tokio::fs`
- `write_secure_file(path, data)` / `write_secure_file_str(path, str)` — async; atomically writes
  file with 0o600 permissions (write-to-temp-then-rename on same filesystem)
- `AppDirs::resolve(app_kind, config_override, state_override)` — resolves directories for an
  application
- `AppDirs::config_path(name)` / `AppDirs::state_path(name)` — returns `Result<PathBuf>` after
  validating `name` against path traversal (rejects path separators, `..`, `.`, empty strings,
  absolute paths)
- `AppDirs::ensure_config_dir()` — async; creates config directory with secure permissions
- `AppDirs::ensure_state_dir()` — async; creates state directory with secure permissions
- `AppDirs::ensure_dirs()` — async; creates both directories with secure permissions

All crates writing sensitive files (private keys, certificates, CA bundles) **must** use these
helpers instead of raw `fs::write` / `tokio::fs::write`.

Permission hardening (0o700 directories, 0o600 files) is Unix-only. On non-Unix platforms
(Windows), directories and files are created with default OS permissions — the non-Unix code paths
exist for developer convenience and are not security-hardened.

## Cross-References

- [Coding Standards](coding-standards.md) — general Rust coding rules and quality constraints
- [Security — Filesystem and Dependency Security](../security/filesystem-dependency-security.md) —
  secure creation guarantees for `uptrakit-directories`
- [Security — SSH Agent Secrets](../security/ssh-agent-secrets.md) — `agent-ssh.db` encrypted
  credential storage
