# Test Coverage: uptrakit-shared-db

> Generated: 2026-02-20 | Tool: cargo-llvm-cov 0.8.4 | Rust: 1.93.1

## Summary

| Metric | Value |
| -------- | ------- |
| Line coverage | 19.6% (100 / 510) |
| Function coverage | 10.4% (12 / 115) |
| Test count | 8 |

## Coverage by Module

| File | Line % | Lines | Function % | Functions |
| ------ | -------- | ------- | ------------ | ----------- |
| entity/oidc_provider.rs | 64.7% | 66/102 | 52.9% | 9/17 |
| crypto.rs | 17.6% | 34/193 | 8.8% | 3/34 |
| entity/api_token.rs | 0.0% | 0/3 | 0.0% | 0/1 |
| entity/auth_method.rs | 0.0% | 0/19 | 0.0% | 0/1 |
| entity/available_version.rs | 0.0% | 0/3 | 0.0% | 0/1 |
| entity/host.rs | 0.0% | 0/18 | 0.0% | 0/6 |
| entity/host_software_item.rs | 0.0% | 0/6 | 0.0% | 0/2 |
| entity/mqtt_client.rs | 0.0% | 0/3 | 0.0% | 0/1 |
| entity/mqtt_lease.rs | 0.0% | 0/6 | 0.0% | 0/2 |
| entity/permission.rs | 0.0% | 0/9 | 0.0% | 0/3 |
| entity/provider_config.rs | 0.0% | 0/6 | 0.0% | 0/2 |
| entity/role.rs | 0.0% | 0/18 | 0.0% | 0/6 |
| entity/role_permission.rs | 0.0% | 0/6 | 0.0% | 0/2 |
| entity/scheduled_task.rs | 0.0% | 0/21 | 0.0% | 0/3 |
| entity/service.rs | 0.0% | 0/12 | 0.0% | 0/4 |
| entity/service_certificate.rs | 0.0% | 0/3 | 0.0% | 0/1 |
| entity/service_host.rs | 0.0% | 0/6 | 0.0% | 0/2 |
| entity/session.rs | 0.0% | 0/6 | 0.0% | 0/2 |
| entity/setting.rs | 0.0% | 0/3 | 0.0% | 0/1 |
| entity/settings_version.rs | 0.0% | 0/3 | 0.0% | 0/1 |
| entity/software_item.rs | 0.0% | 0/22 | 0.0% | 0/6 |
| entity/update_history.rs | 0.0% | 0/6 | 0.0% | 0/2 |
| entity/update_output_line.rs | 0.0% | 0/3 | 0.0% | 0/1 |
| entity/user.rs | 0.0% | 0/18 | 0.0% | 0/6 |
| entity/user_oidc_link.rs | 0.0% | 0/6 | 0.0% | 0/2 |
| entity/user_role.rs | 0.0% | 0/9 | 0.0% | 0/3 |

> **Note:** `crypto.rs` is now a re-export module only (the crypto implementation was extracted to the
> `uptrakit-crypto` crate). The 34/193 lines shown here reflect re-exported symbols, not actual crypto
> business logic. See `uptrakit-crypto` for crypto-specific coverage.

## Uncovered Critical Paths

### Tier 2 — Business-Logic

- **OIDC provider RoleMapping serialization** (`entity/oidc_provider.rs`, 64.7%): Custom `Serialize`/`Deserialize`
  implementations for `RoleMapping` are now tested. Remaining uncovered lines are SeaORM-generated relation and
  `ActiveModel` conversion code.

### Tier 3 — Supporting

- **Scheduled task entity** (`entity/scheduled_task.rs`, 0%, 21 lines): `ScheduledTaskType` enum and related
  conversions remain untested.
- **Auth method entity** (`entity/auth_method.rs`, 0%, 19 lines): `FromStr` implementation for auth method values
  is not covered.

> Most entity files (23 at 0%) are SeaORM auto-generated models containing only `ActiveModel` conversion and
> `Relation` boilerplate. These are exercised indirectly by integration tests in higher-level crates and do not
> contain business logic worth testing directly.

## Test Recommendations

1. **Scheduled task type conversion tests** -- Test `ScheduledTaskType` enum serialization and `FromStr`
   implementation. Covers `entity/scheduled_task.rs` (Tier 3). Simple unit tests.
2. **Auth method parsing tests** -- Test `FromStr` for `AuthMethod` enum, including unknown/invalid values.
   Covers `entity/auth_method.rs` (Tier 3). Simple unit tests.
