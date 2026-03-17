# Code Review: `uptrakit-agent-ssh`

- Review date: 2026-03-17
- Scope: current-state review

## Summary

`uptrakit-agent-ssh` is functionally rich and has good operational fundamentals: per-host concurrency guards, pooled SSH sessions with TTL eviction, explicit connection timeouts, and solid test coverage across parsing and bootstrap helpers. Its active issues are concentrated in failure reporting, maintainability, and a type safety divergence between the two bootstrap paths.

## Strengths

- Per-host concurrency guard prevents overlapping updates on the same SSH host.
- `SshConnectionPool` uses TTL-based eviction and reconnects outside the lock, which is the right shape for slow or flaky remote hosts.
- Bootstrap and Proxmox flows now have substantial unit coverage compared with older review snapshots.
- TOFU and strict fingerprint verification with RSA hash algorithm negotiation (SHA-512, SHA-256, legacy fallback) is well-implemented.
- Interactive PTY support with coalesced output (50ms flush interval) and 10MB output limit prevents runaway memory consumption.
- SSH agent authentication support is cleanly integrated.

## Active Findings

### [MEDIUM] Some definitive `UpdateResult` failures are still sent best-effort

- Dimension: high availability, consistency
- Scope: `crates/core/agent-ssh/src/client.rs:427,444`, `crates/core/agent-ssh/src/main.rs:585`
- Why it matters: host lookup failures, SSH acquire failures, and shutdown-timeout failures are terminal outcomes, but some of them still go through `transport_send_best_effort` / `send_best_effort`.
- Failure scenario: remote host is down and the controller connection is simultaneously laggy or reconnecting. The failure frame can be dropped, leaving the controller-side row pending until a later reconnect path cleans it up.

### [MEDIUM] Bootstrap logic is still split across parallel `commands/*` and `operations/*` trees

- Dimension: maintainability, crate structure
- Scope: `crates/core/agent-ssh/src/commands/bootstrap*.rs`, `crates/core/agent-ssh/src/operations/bootstrap*.rs`
- Why it matters: the duplicated shape makes it easy for future behavior fixes to land in one path but not the other.
- Failure scenario: a fault-tolerance improvement for stale keys, sudoers generation, or Proxmox guest bootstrap gets applied to one implementation path and silently diverges from the other.

### [MEDIUM] Bootstrap `commands/` path uses plain `String` for secrets while `operations/` uses `SecretString`

- Dimension: security, coding standards
- Scope: `crates/core/agent-ssh/src/commands/bootstrap.rs:45-50` (`Option<String>` for `auth_password`, `auth_private_key_pem`, `target_private_key_pem`), `crates/core/agent-ssh/src/operations/bootstrap.rs:46-51` (`Option<SecretString>` for the same fields)
- Why it matters: the `commands/` bootstrap path stores SSH passwords and private key PEM material in plain `String` fields, which are not zeroized on drop and may appear in debug output. The `operations/` path correctly uses `SecretString` with automatic zeroization.
- Failure scenario: a memory dump, core file, or overly verbose log captures the plaintext SSH password or private key from the `commands/` path's `BootstrapParams` struct.
- Fix: align `commands/bootstrap.rs` to use `SecretString` for all secret fields, matching `operations/bootstrap.rs`.

### [LOW] `Vec<&String>` is still used where borrowed `&str` would be more idiomatic

- Dimension: idiomatic Rust, allocation awareness
- Scope: `crates/core/agent-ssh/src/commands/bootstrap.rs:429`
- Why it matters: this is small on its own, but it is a signal that the bootstrap path still carries avoidable reference-shape friction in already-large code.
