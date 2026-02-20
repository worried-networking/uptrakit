# Test Coverage: uptrakit-cli

> Generated: 2026-02-20 | Tool: cargo-llvm-cov 0.8.4 | Rust: 1.93.1

## Summary

| Metric | Value |
| -------- | ------- |
| Line coverage | 25.6% (715 / 2,788) |
| Function coverage | 34.8% (77 / 221) |
| Test count | 68 |

## Coverage by Module

| File | Line % | Lines | Function % | Functions |
| ------ | -------- | ------- | ------------ | ----------- |
| output.rs | 67.6% | 46/68 | 70.0% | 7/10 |
| main.rs | 47.2% | 555/1,175 | 90.3% | 56/62 |
| commands/auth.rs | 33.4% | 114/341 | 45.2% | 14/31 |
| client.rs | 0.0% | 0/28 | 0.0% | 0/6 |
| commands/api.rs | 0.0% | 0/45 | 0.0% | 0/3 |
| commands/check.rs | 0.0% | 0/32 | 0.0% | 0/5 |
| commands/history.rs | 0.0% | 0/75 | 0.0% | 0/6 |
| commands/hosts.rs | 0.0% | 0/67 | 0.0% | 0/4 |
| commands/scheduler.rs | 0.0% | 0/55 | 0.0% | 0/6 |
| commands/services.rs | 0.0% | 0/128 | 0.0% | 0/16 |
| commands/settings.rs | 0.0% | 0/629 | 0.0% | 0/56 |
| commands/software_items.rs | 0.0% | 0/68 | 0.0% | 0/4 |
| commands/update.rs | 0.0% | 0/21 | 0.0% | 0/2 |
| config.rs | 0.0% | 0/41 | 0.0% | 0/7 |
| error.rs | 0.0% | 0/15 | 0.0% | 0/3 |

## Uncovered Critical Paths

### Tier 3 — Supporting

- **Settings commands** (`commands/settings.rs`, 0% coverage, 629 lines): All settings subcommands (network, auth, CA, MQTT,
  agent certs, enrollment) including display formatting and update logic. Largest uncovered module.
- **Auth commands** (`commands/auth.rs`, 33.4% coverage, 341 lines): 227 uncovered lines include login flow, token management,
  device auth polling, and OIDC browser flow. Risk: auth command bugs could prevent users from authenticating.
- **Service commands** (`commands/services.rs`, 0% coverage, 128 lines): Service listing, detail, and management operations.
- **All CRUD commands** (0% coverage across 7 files, 1,092 lines total): hosts, software_items, check, update, history,
  scheduler, and api commands. These are the primary user-facing CLI operations.
- **Client wrapper** (`client.rs`, 0% coverage, 28 lines): `UptrakitClient` construction from CLI arguments.
- **Config persistence** (`config.rs`, 0% coverage, 41 lines): Credential storage and retrieval from config files.

## Test Recommendations

1. **Settings command output tests** — Test each settings subcommand with mock API responses, verifying table/JSON output
   formatting. Covers `commands/settings.rs` (Tier 3). Mock `UptrakitClient` responses.
2. **Auth login flow tests** — Test password login, token storage, and `whoami` command. Covers `commands/auth.rs` gaps
   (Tier 3). Mock API responses for login endpoint.
3. **CRUD command tests** — Test list/create/update/delete for hosts, software items, and services with mock API. Covers
   `commands/hosts.rs`, `commands/software_items.rs`, `commands/services.rs` (Tier 3). Systematic mock responses.
4. **Config file round-trip tests** — Test credential save and load from config files. Covers `config.rs` (Tier 3). Use
   temp directories.
5. **Output formatting tests** — Test JSON and table output modes with various data shapes. Covers `output.rs` gaps (Tier 3).
   Extend existing output tests.
6. **Error display tests** — Test CLI error formatting for various error types. Covers `error.rs` (Tier 3). Simple unit tests.
