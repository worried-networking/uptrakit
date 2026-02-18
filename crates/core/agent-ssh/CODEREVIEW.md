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

#### H1: Agent-ssh depends on `uptrakit-shared-db` (extensibility)

**Location:** `Cargo.toml:32`

The SSH agent depends on `uptrakit-shared-db`, which contains all 34 entity
definitions for the entire system (controller entities, OIDC entities, MQTT
entities, etc.). The SSH agent only uses its own local SQLite tables for
encrypted SSH credential storage. It does not need controller-side entities
like `oidc_provider`, `mqtt_lease`, `scheduled_task`, or `api_rate_limit`.

**Impact:** The SSH agent compiles and links all 34 entity models plus the
`crypto` module even though it only uses a small subset. This increases
compile time and conceptually couples the agent to the controller's schema.

**Recommendation:** Two approaches to reduce this coupling:

1. **Feature-gate entity groups** in `shared-db` -- e.g.,
   `controller-entities`, `agent-entities`, `crypto`. The SSH agent would
   enable only `crypto` and `agent-entities`.
2. **Extract agent-ssh's local DB schema** into its own minimal crate (e.g.,
   `uptrakit-agent-ssh-db`) that depends only on `sea-orm` and the `crypto`
   module from `shared-db`.

#### H2: `SshKeyType` uses manual SeaORM conversions instead of `DeriveActiveEnum`

**File:** `src/db/entity/ssh_host.rs:52-99`

48 lines of manual `From<SshKeyType> for Value`, `ValueType`, `Nullable`,
and `TryGetable` implementations. Per the coding standards
(`docs/development/coding-standards.md`, "Database Enum Columns" section),
all entity columns storing a fixed set of string values must use
`DeriveActiveEnum`:

```rust
// Current: 48 lines of manual conversion code
impl From<SshKeyType> for sea_orm::Value { ... }
impl sea_orm::sea_query::ValueType for SshKeyType { ... }
impl sea_orm::sea_query::Nullable for SshKeyType { ... }
impl sea_orm::TryGetable for SshKeyType { ... }
```

**Recommendation:** Refactor to use the `DeriveActiveEnum` pattern from
`uptrakit-shared-types`, following the existing `DeviceAuthStatus` template:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "sea-orm", derive(strum::EnumIter, sea_orm::DeriveActiveEnum))]
#[cfg_attr(feature = "sea-orm", sea_orm(rs_type = "String", db_type = "Text"))]
pub enum SshKeyType {
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "ed25519"))]
    Ed25519,
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "rsa"))]
    Rsa,
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "ecdsa"))]
    Ecdsa,
}
```

Since `SshKeyType` is local to this crate (not in `uptrakit-shared-types`),
the `cfg_attr` feature gates may not be needed, and the derives can be
applied directly.

### Medium — Security

#### MS1: Bootstrap grants `NOPASSWD: ALL` sudoers access

**File:** `src/commands/bootstrap.rs`

The bootstrap workflow creates a sudoers entry granting the target user
unlimited passwordless sudo (`NOPASSWD: ALL`). While this is warned about
in the CLI output and documented, it represents a significant security
surface:

- Any compromise of the SSH key grants full root access to the target host.
- There is no option to restrict sudo to specific commands.

**Recommendation:** Consider adding an option to generate a restricted
sudoers entry (e.g., `NOPASSWD: /usr/bin/apt-get, /usr/bin/systemctl`)
or at minimum document the security implications in the security
documentation with a hardening guide for production deployments.

#### MS2: TOFU for SSH host keys accepts any key on first use

**File:** `src/ssh_transport.rs:76-81`

`BootstrapHandler::check_server_key` accepts any host key when no
`expected_fingerprint` is provided (TOFU mode). This is standard SSH TOFU
behavior and the observed fingerprint is stored for future verification.

```rust
} else {
    // TOFU: accept and record.
    let mut fp = self.observed_fingerprint.lock().await;
    *fp = Some(fingerprint);
    Ok(true)
}
```

The host key fingerprint is persisted in the database
(`ssh_hosts.host_key_fingerprint`) and used for subsequent connections.

**Recommendation:** Consider adding a `--strict-host-key-checking` flag
that requires `--host-key-fingerprint` for the initial connection,
disabling TOFU. Document the TOFU behavior in security documentation.

### Medium — Code Quality

#### M1: Error conversion in `ServiceHandler` impl duplicates agent pattern

**File:** `src/main.rs`

Like the agent crate (agent `src/main.rs:58-76`), the SSH agent's
`ServiceHandler::run_authenticated_loop` likely reconstructs
`EnrollmentError` variants manually. This duplicates the same fragile
pattern.

**Recommendation:** See agent crate finding M1. A shared conversion utility
in the service-sdk would eliminate this duplication.

#### M2: Several error variants use `String` for foreign errors

**File:** `src/error.rs:21-57`

Multiple error variants (`Database`, `SshConnection`, `SshAuth`,
`SshCommand`, `KeyGeneration`, `BootstrapVerification`, `InvalidInput`)
wrap `String` instead of the original error type. While some of these are
necessary (e.g., `russh::Error` does not implement `std::error::Error`
compatibly, per line 91-92), others like `Database` could wrap `sea_orm::DbErr`
directly with `#[from]`.

**Recommendation:** Where the foreign error type implements
`std::error::Error`, use `#[from]` to preserve the error chain. The
`sea_orm::DbErr` case (line 89) is documented as intentional, but
`DbErr` does implement `Error`, so `#[from]` should work.

### Low

#### L1: `LineBuffer` output limit silently truncates

**File:** `src/ssh_transport.rs:90-167`

When accumulated output exceeds `MAX_OUTPUT_BYTES` (10 MB), new lines are
still sent to the optional channel (streaming) but not appended to
`accumulated`. This means the streaming output and the final collected
output may diverge.

**Recommendation:** Consider adding a truncation marker to the accumulated
output (e.g., `\n[output truncated at 10 MB]`) so callers know the output
was truncated.

#### L2: `SshTarget::from_str` does not validate hostname

**File:** `src/ssh_target.rs`

The `FromStr` impl for `SshTarget` parses the target into user, host, and
port components but does not validate that the hostname is syntactically
valid (e.g., no spaces, valid DNS characters).

**Recommendation:** Consider basic hostname validation or document that
validation happens at connection time.

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
