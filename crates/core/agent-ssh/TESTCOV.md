# Test Coverage: uptrakit-agent-ssh

> Generated: 2026-02-20 | Tool: cargo-llvm-cov 0.8.4 | Rust: 1.93.1

## Summary

| Metric | Value |
| -------- | ------- |
| Line coverage | 62.7% (2,194 / 3,498) |
| Function coverage | 71.6% (287 / 401) |
| Test count | 177 |

## Coverage by Module

| File | Line % | Lines | Function % | Functions |
| ------ | -------- | ------- | ------------ | ----------- |
| db/entity/ssh_host.rs | 100.0% | 40/40 | 100.0% | 7/7 |
| db/migration/mod.rs | 100.0% | 7/7 | 100.0% | 3/3 |
| db/mod.rs | 100.0% | 30/30 | 100.0% | 6/6 |
| host_ops.rs | 98.8% | 337/341 | 100.0% | 35/35 |
| cli.rs | 97.2% | 445/458 | 96.9% | 31/32 |
| error.rs | 96.9% | 94/97 | 85.7% | 18/21 |
| ssh_target.rs | 94.5% | 260/275 | 95.1% | 39/41 |
| ssh_key.rs | 84.5% | 289/342 | 84.1% | 37/44 |
| ssh_config.rs | 83.7% | 72/86 | 75.0% | 9/12 |
| ssh_executor.rs | 78.2% | 61/78 | 64.3% | 9/14 |
| host_info.rs | 53.7% | 109/203 | 65.7% | 23/35 |
| ssh_transport.rs | 50.2% | 207/412 | 56.9% | 29/51 |
| db/migration/m20260215_000001_initial.rs | 50.0% | 2/4 | 50.0% | 1/2 |
| commands/bootstrap.rs | 38.8% | 150/387 | 70.6% | 24/34 |
| main.rs | 26.5% | 59/223 | 41.7% | 10/24 |
| client.rs | 15.4% | 32/208 | 46.2% | 6/13 |
| commands/host.rs | 0.0% | 0/307 | 0.0% | 0/27 |

## Uncovered Critical Paths

### Tier 1 — Security

- **SSH transport** (`ssh_transport.rs`, 50.2% coverage, 412 lines): SSH tunnel setup, key authentication, and password
  authentication. 205 uncovered lines include error recovery, timeout handling, and keyboard-interactive auth fallback.
  Risk: transport failures could cause silent connection drops or authentication bypass.

### Tier 2 — Business-Logic

- **Host management commands** (`commands/host.rs`, 0% coverage, 307 lines): Host add, remove, list, and credential management
  operations. Risk: untested host operations could corrupt the local SSH host database.
- **Client lifecycle** (`client.rs`, 15.4% coverage, 208 lines): Controller communication, authenticated loop, and message
  handling. Risk: client failures could cause the SSH agent to disconnect silently.
- **Bootstrap command** (`commands/bootstrap.rs`, 38.8% coverage, 387 lines): Agent bootstrap flow including SSH host
  discovery, credential setup, and initial enrollment. Risk: bootstrap failures could leave the agent in an inconsistent state.
- **Host info collection gaps** (`host_info.rs`, 53.7% coverage, 203 lines): Remaining uncovered lines are in the async
  SSH-dependent collection functions (`collect_remote_host_info`, `read_remote_*`). The OS release parsing helpers are now
  tested.
- **SSH key management gaps** (`ssh_key.rs`): Remaining uncovered lines include encrypted key handling and edge-case key
  formats. PKCS#8 key detection is now tested (up from 67.5% to 84.5%).

### Tier 3 — Supporting

- **Main entry point** (`main.rs`, 26.5% coverage): Service startup and shutdown orchestration.
- **Migration initial** (`db/migration/m20260215_000001_initial.rs`, 50.0% coverage): Database migration setup.

## Test Recommendations

1. **SSH transport security tests** — Test SSH tunnel setup, key auth, password auth, host key verification, and timeout
   handling. Covers `ssh_transport.rs` (Tier 1). Use `russh` test server or mock.
2. **Host management command tests** — Test host add/remove/list with mock database, credential CRUD, and error handling. Covers
   `commands/host.rs` (Tier 2). Use in-memory SQLite.
3. **Bootstrap flow integration tests** — Test the full bootstrap sequence with mock SSH server and controller. Covers
   `commands/bootstrap.rs` (Tier 2). Requires mock SSH and HTTP endpoints.
4. **Client authenticated loop tests** — Test message handling, reconnection, and graceful shutdown. Covers `client.rs` (Tier 2).
   Use in-memory WebSocket pairs.
