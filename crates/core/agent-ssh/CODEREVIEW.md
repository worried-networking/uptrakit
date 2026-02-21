# Code Review: `uptrakit-agent-ssh`

Reviewed: `src/main.rs` (333 lines), `src/client.rs` (452 lines),
`src/error.rs` (224 lines), `src/cli.rs` (798 lines), `src/host_info.rs`
(275 lines), `src/host_ops.rs` (423 lines), `src/ssh_transport.rs` (601
lines), `src/ssh_executor.rs` (187 lines), `src/ssh_key.rs` (407 lines),
`src/ssh_target.rs` (439 lines), `src/ssh_config.rs` (145 lines),
`src/commands/mod.rs`, `src/commands/host.rs` (424 lines),
`src/commands/bootstrap.rs` (587 lines), `src/db/mod.rs` (54 lines),
`src/db/entity/mod.rs`, `src/db/entity/ssh_host.rs` (167 lines),
`src/db/migration/mod.rs`, `src/db/migration/m20260215_000001_initial.rs`
(60 lines), `Cargo.toml`.

## Summary

The agent-ssh crate is a complex, well-organized crate with SSH transport,
bootstrap workflow, and local credential storage. It has strong security
practices (encrypted keys at rest, shell escaping, authorized_keys
restrictions).

## Dependency Analysis

| Dependency | Purpose | Concern |
| --- | --- | --- |
| `uptrakit-crypto` | Local SQLite for encrypted SSH credentials | Clean; extracted from shared-db |
| `uptrakit-service-sdk` | Service lifecycle, enrollment, TLS | Clean |
| `uptrakit-internal-wire` | Wire protocol messages | Clean |
| `uptrakit-command` | Command execution abstraction | Clean |
| `sea-orm` + `sea-orm-migration` | SQLite ORM and migrations | Appropriate for local DB |

## Findings

### Medium — Security

#### MS1: Bootstrap grants `NOPASSWD: ALL` sudoers access (ACCEPTED)

The bootstrap workflow creates a sudoers entry granting the target user
unlimited passwordless sudo (`NOPASSWD: ALL`). While this is warned about
in the CLI output and documented, it represents a significant security
surface:

- Any compromise of the SSH key grants full root access to the target host.
- There is no option to restrict sudo to specific commands.

**Resolution:** Accepted risk. The `NOPASSWD: ALL` configuration is required for the agent's operational scope and is clearly warned about in CLI output.

### Info

#### I1: Shell escaping for remote commands

**File:** `src/ssh_executor.rs`

`build_remote_command_string()` uses `shell_escape()` for constructing
remote commands. Injection prevention tests exist, verifying that
metacharacters are properly escaped.

#### I2: Encrypted private keys at rest

**File:** `src/db/entity/ssh_host.rs:112`

SSH private keys use `EncryptedString` (AES-256-GCM) for storage in the
local SQLite database. The master key is loaded from file or environment
variable with hex validation (`parse_master_key_hex` in `src/main.rs`).

#### I3: `authorized_keys` restrictions

**File:** `src/commands/bootstrap.rs`

The bootstrap workflow deploys authorized_keys entries with restrictions:
`no-pty,no-agent-forwarding,no-X11-forwarding`. These limit the attack
surface if the SSH key is compromised (though `NOPASSWD: ALL` sudoers
access remains the primary concern — see MS1).

#### I4: POSIX username validation

**File:** `src/commands/bootstrap.rs:17`

The `validate_posix_username` function enforces POSIX username constraints
(max 32 characters, valid character set). Applied to both auth and target
usernames during bootstrap.

#### I5: RSA hash algorithm negotiation

**File:** `src/ssh_transport.rs`

The SSH client negotiates RSA hash algorithms for compatibility with modern
OpenSSH servers that prefer `rsa-sha2-256`/`rsa-sha2-512` over the
deprecated `ssh-rsa` (SHA-1).

#### I6: Comprehensive error type coverage

**File:** `src/error.rs:82-98`

Six `impl_report_conversion!` entries cover all foreign error types:
`EnrollmentError`, `std::io::Error`, `DirectoryError`, `sea_orm::DbErr`,
`russh::Error`, `russh::AgentAuthError`, and `CommandError`. Each uses
the appropriate conversion pattern (direct `#[from]` or closure-based
for incompatible types).

#### I7: Well-structured command pattern

The `commands/` module cleanly separates `host.rs` (CRUD subcommands) from
`bootstrap.rs` (the multi-step bootstrap workflow). Each command function
receives parsed parameters and delegates to the appropriate module
(`host_ops`, `ssh_transport`, `ssh_key`).

#### I8: `SshCommandExecutor` validates trait extensibility

Demonstrates a custom `CommandExecutor` implementation, validating the
trait's extensibility for remote command execution over SSH.
