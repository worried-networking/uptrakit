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
restrictions). Key issues are the heavy `shared-db` dependency, manual SeaORM
conversions, missing platform guards, and the security implications of the
bootstrap sudoers configuration.

## Dependency Analysis

| Dependency | Purpose | Concern |
| --- | --- | --- |
| `uptrakit-shared-db` | Local SQLite for encrypted SSH credentials | Pulls in **all 34 entity definitions** |
| `uptrakit-service-sdk` | Service lifecycle, enrollment, TLS | Clean |
| `uptrakit-internal-wire` | Wire protocol messages | Clean |
| `uptrakit-command` | Command execution abstraction | Clean |
| `sea-orm` + `sea-orm-migration` | SQLite ORM and migrations | Appropriate for local DB |

## Findings

### High

#### ~~H1: Agent-ssh depends on `uptrakit-shared-db` (extensibility)~~ RESOLVED

**Resolution:** The crypto module was extracted into a standalone `uptrakit-crypto` crate.
Agent-ssh now depends on `uptrakit-crypto` instead of `uptrakit-shared-db`, eliminating
the compile-time and binary-size cost of 34 entity definitions.

### Medium — Security

#### ~~MS1: Bootstrap grants `NOPASSWD: ALL` sudoers access~~ (ACCEPTED)

~~**File:** `src/commands/bootstrap.rs`~~

~~The bootstrap workflow creates a sudoers entry granting the target user
unlimited passwordless sudo (`NOPASSWD: ALL`). While this is warned about
in the CLI output and documented, it represents a significant security
surface:~~

~~- Any compromise of the SSH key grants full root access to the target host.~~
~~- There is no option to restrict sudo to specific commands.~~

~~**Recommendation:** Consider adding an option to generate a restricted
sudoers entry (e.g., `NOPASSWD: /usr/bin/apt-get, /usr/bin/systemctl`)
or at minimum document the security implications in the security
documentation with a hardening guide for production deployments.~~

**Resolution:** Accepted risk. The `NOPASSWD: ALL` configuration is required for the agent's operational scope and is clearly warned about in CLI output.

#### ~~MS2: TOFU for SSH host keys accepts any key on first use~~ (FIXED)

~~**File:** `src/ssh_transport.rs:76-81`~~

~~`BootstrapHandler::check_server_key` accepts any host key when no
`expected_fingerprint` is provided (TOFU mode). This is standard SSH TOFU
behavior and the observed fingerprint is stored for future verification.~~

~~```rust
} else {
    // TOFU: accept and record.
    let mut fp = self.observed_fingerprint.lock().await;
    *fp = Some(fingerprint);
    Ok(true)
}
```~~

~~The host key fingerprint is persisted in the database
(`ssh_hosts.host_key_fingerprint`) and used for subsequent connections.~~

~~**Recommendation:** Consider adding a `--strict-host-key-checking` flag
that requires `--host-key-fingerprint` for the initial connection,
disabling TOFU. Document the TOFU behavior in security documentation.~~

**Resolution:** Added `--strict-host-key-checking` flag to bootstrap and host-add commands. When set, `--host-key-fingerprint` is required. TOFU key acceptance is now logged via `tracing::info!`.

### Medium — Code Quality

#### ~~M1: Error conversion in `ServiceHandler` impl duplicates agent pattern~~ (FIXED)

**Resolution:** Added `EnrollmentError::from_agent_error(cert_expired, receive_closed, msg)`
in the service SDK. Both agent and agent-ssh now use this shared helper.

#### ~~M2: `Database` error variant uses `String` instead of typed `DbErr`~~ (FIXED)

**Resolution:** Changed `Error::Database(String)` to
`Error::Database(#[from] sea_orm::DbErr)`. The `#[from]` generates the
`From<sea_orm::DbErr>` conversion, preserving the full error chain. Added
`sea_orm::DbErr` to the simple `impl_report_conversion!` list. Remaining
`String`-based variants (`SshConnection`, `SshAuth`, `SshCommand`,
`KeyGeneration`, `BootstrapVerification`, `InvalidInput`) are kept as-is
because their source types do not implement `std::error::Error` compatibly.

### Low

#### ~~L1: `LineBuffer` output limit silently truncates~~ (FIXED)

~~**File:** `src/ssh_transport.rs:90-167`~~

~~When accumulated output exceeds `MAX_OUTPUT_BYTES` (10 MB), new lines are
still sent to the optional channel (streaming) but not appended to
`accumulated`. This means the streaming output and the final collected
output may diverge.~~

**Resolution:** Added a `truncated: bool` flag to `LineBuffer`. When the limit is first hit,
the flag is set, `tracing::warn!()` is emitted, and `\n[output truncated at 10 MB]\n` is
appended to `accumulated`. All lines continue streaming via the channel. The same pattern
is applied in both `push()` and `flush()` methods. Tests added to verify the truncation
marker and continued streaming behavior.

#### ~~L2: `SshTarget::from_str` does not validate hostname~~ (FIXED)

**Resolution:** Added `validate_hostname()` function and `InvalidHostname`
error variant. Validation checks: no whitespace/control characters, length
<= 253 (DNS limit), labels <= 63 chars, no leading/trailing hyphens per
label, valid DNS characters (alphanumeric, hyphen, underscore). IPv4 and
IPv6 addresses pass through without DNS label validation. FQDN trailing
dots are allowed. 15 new tests cover valid and invalid hostname patterns.

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
