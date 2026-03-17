# Code Review: `uptrakit-agent-ssh`

- Review date: 2026-03-17
- Scope: current-state review

## Summary

`uptrakit-agent-ssh` is functionally rich and has good operational fundamentals: per-host concurrency guards, pooled SSH sessions with TTL eviction, explicit connection timeouts, and solid test coverage across parsing and bootstrap helpers. Its active issues are concentrated in failure reporting and maintainability.

## Strengths

- Per-host concurrency guard prevents overlapping updates on the same SSH host.
- `SshConnectionPool` uses TTL-based eviction and reconnects outside the lock, which is the right shape for slow or flaky remote hosts.
- Bootstrap and Proxmox flows now have substantial unit coverage compared with older review snapshots.

## Active Findings

### [MEDIUM] Some definitive `UpdateResult` failures are still sent best-effort

- Dimension: high availability, consistency
- Scope: `crates/core/agent-ssh/src/client.rs`, `crates/core/agent-ssh/src/main.rs`
- Why it matters: host lookup failures, SSH acquire failures, and shutdown-timeout failures are terminal outcomes, but some of them still go through `send_best_effort`.
- Failure scenario: remote host is down and the controller connection is simultaneously laggy or reconnecting. The failure frame can be dropped, leaving the controller-side row pending until a later reconnect path cleans it up.

### [MEDIUM] Bootstrap logic is still split across parallel `commands/*` and `operations/*` trees

- Dimension: maintainability, crate structure
- Scope: `crates/core/agent-ssh/src/commands/bootstrap*.rs`, `crates/core/agent-ssh/src/operations/bootstrap*.rs`
- Why it matters: the duplicated shape makes it easy for future behavior fixes to land in one path but not the other.
- Failure scenario: a fault-tolerance improvement for stale keys, sudoers generation, or Proxmox guest bootstrap gets applied to one implementation path and silently diverges from the other.

### [LOW] `Vec<&String>` is still used where borrowed `&str` would be more idiomatic

- Dimension: idiomatic Rust, allocation awareness
- Scope: `crates/core/agent-ssh/src/commands/bootstrap.rs:429`
- Why it matters: this is small on its own, but it is a signal that the bootstrap path still carries avoidable reference-shape friction in already-large code.
