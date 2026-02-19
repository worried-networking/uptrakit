# Test Coverage: uptrakit-shared-db

> Generated: 2026-02-19 | Tool: cargo-llvm-cov 0.8.4 | Rust: 1.93.1

## Summary

| Metric | Value |
| -------- | ------- |
| Line coverage | 54.3% (324 / 597) |
| Function coverage | 33.6% (41 / 122) |
| Test count | 15 |

## Coverage by Module

| File | Line % | Lines | Function % | Functions |
| ------ | -------- | ------- | ------------ | ----------- |
| crypto.rs | 90.6% | 280/309 | 75.6% | 34/45 |
| entity/auth_method.rs | 63.2% | 12/19 | 75.0% | 3/4 |
| entity/oidc_provider.rs | 43.8% | 32/73 | 30.8% | 4/13 |
| entity/api_token.rs | 0.0% | 0/3 | 0.0% | 0/1 |
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

## Uncovered Critical Paths

### Tier 1 — Security-Critical

- **Crypto module gaps** (`crypto.rs`, 90.6% coverage, 309 lines): 29 uncovered lines include master key initialization error
  paths, key derivation edge cases, and the `EncryptedString` SeaORM integration (custom `Value` and `ValueType`
  implementations). Risk: untested crypto edge cases could cause data loss or decryption failures after key rotation.

### Tier 2 — Business-Logic

- **OIDC provider entity** (`entity/oidc_provider.rs`, 43.8% coverage): Role mapping serialization and OIDC-specific entity
  methods are partially tested. Risk: serialization bugs could corrupt OIDC role assignments.
- **Entity relation definitions** (0% coverage across 23 entity files): SeaORM `Relation` implementations, `Related<>` trait
  impls, and `ActiveModelBehavior` impls are generated code but remain untested. These are exercised indirectly by higher-level
  tests but have no direct coverage.

### Tier 3 — Supporting

- **Auth method entity** (`entity/auth_method.rs`, 63.2% coverage): `FromStr` implementation for unknown auth method values.
- **Scheduled task entity** (`entity/scheduled_task.rs`, 0% coverage, 21 lines): `ScheduledTaskType` enum and related
  conversions.

## Test Recommendations

1. **Crypto error path tests** — Test master key initialization with invalid key material, decryption with wrong key, and
   `EncryptedString` SeaORM value conversion edge cases. Covers `crypto.rs` gaps (Tier 1). Extend existing crypto test suite.
2. **OIDC role mapping tests** — Test complex role mapping serialization with nested structures, empty mappings, and invalid
   JSON. Covers `entity/oidc_provider.rs` (Tier 2). Extend existing `role_mapping_*` tests.
3. **Entity relation smoke tests** — Test that SeaORM relation definitions compile and produce correct SQL joins for key entity
   pairs (user-role, host-software_item, service-host). Covers entity relation impls (Tier 2). Requires in-memory SQLite.
4. **Scheduled task type conversion tests** — Test `ScheduledTaskType` enum serialization and `FromStr` implementation. Covers
   `entity/scheduled_task.rs` (Tier 3). Simple unit tests.
