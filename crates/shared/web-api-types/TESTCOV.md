# Test Coverage: uptrakit-web-api-types

> Generated: 2026-02-19 | Tool: cargo-llvm-cov 0.8.4 | Rust: 1.93.1

## Summary

| Metric | Value |
| -------- | ------- |
| Line coverage | 94.3% (1,575 / 1,670) |
| Function coverage | 97.7% (168 / 172) |
| Test count | 125 |

## Coverage by Module

| File | Line % | Lines | Function % | Functions |
| ------ | -------- | ------- | ------------ | ----------- |
| lib.rs | 100.0% | 674/674 | 100.0% | 65/65 |
| pagination.rs | 100.0% | 23/23 | 100.0% | 4/4 |
| permissions.rs | 100.0% | 20/20 | 100.0% | 3/3 |
| provider_configs.rs | 100.0% | 12/12 | 100.0% | 2/2 |
| registration.rs | 100.0% | 13/13 | 100.0% | 2/2 |
| settings_mqtt.rs | 100.0% | 47/47 | 100.0% | 6/6 |
| system_alerts.rs | 100.0% | 61/61 | 100.0% | 8/8 |
| validation.rs | 100.0% | 10/10 | 100.0% | 2/2 |
| mqtt_url.rs | 97.4% | 152/156 | 100.0% | 22/22 |
| update_hooks.rs | 94.7% | 428/452 | 97.7% | 42/43 |
| oidc_providers.rs | 74.5% | 35/47 | 100.0% | 4/4 |
| scheduler.rs | 72.2% | 13/18 | 100.0% | 1/1 |
| update_history.rs | 71.4% | 15/21 | 66.7% | 2/3 |
| auth.rs | 70.9% | 39/55 | 100.0% | 2/2 |
| settings_network.rs | 67.9% | 19/28 | 100.0% | 1/1 |
| software_items.rs | 51.9% | 14/27 | 66.7% | 2/3 |
| services.rs | 0.0% | 0/6 | 0.0% | 0/1 |

## Uncovered Critical Paths

### Tier 3 — Supporting

- **Service response types** (`services.rs`, 0% coverage, 6 lines): `ServiceResponse` builder or conversion. Small and low risk.
- **Software items validation** (`software_items.rs`, 51.9% coverage): 13 uncovered lines in request validation for
  `CreateSoftwareItemRequest` and `UpdateSoftwareItemRequest` edge cases.
- **OIDC provider request validation** (`oidc_providers.rs`, 74.5% coverage): 12 uncovered lines in OIDC provider creation
  request validation for edge cases like empty scope lists and invalid URLs.
- **Network settings validation** (`settings_network.rs`, 67.9% coverage): 9 uncovered lines in trusted proxy CIDR validation.
- **Auth request validation** (`auth.rs`, 70.9% coverage): 16 uncovered lines in login request and password change validation
  edge cases.

## Test Recommendations

1. **Software item request validation tests** — Test `CreateSoftwareItemRequest` with invalid package identifiers, missing
   required fields, and conflicting options. Covers `software_items.rs` gaps (Tier 3). Simple unit tests.
2. **OIDC provider validation edge cases** — Test creation requests with empty scopes, malformed issuer URLs, and invalid slug
   characters. Covers `oidc_providers.rs` gaps (Tier 3). Extend existing validation tests.
3. **Network settings CIDR validation tests** — Test trusted proxy configuration with invalid CIDR ranges, IPv6 addresses, and
   overlapping ranges. Covers `settings_network.rs` gaps (Tier 3). Simple unit tests.
4. **Auth request edge cases** — Test login with boundary-length passwords and password change with mismatched confirmation.
   Covers `auth.rs` gaps (Tier 3). Simple unit tests.
5. **Service response conversion test** — Test `ServiceResponse` construction. Covers `services.rs` (Tier 3). Trivial unit test.
