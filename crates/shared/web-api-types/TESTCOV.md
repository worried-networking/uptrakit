# Test Coverage: uptrakit-web-api-types

> Generated: 2026-02-20 | Tool: cargo-llvm-cov 0.8.4 | Rust: 1.93.1

## Summary

| Metric | Value |
| -------- | ------- |
| Line coverage | 86.0% (1,625 / 1,889) |
| Function coverage | 88.4% (175 / 198) |
| Test count | 130 |

## Coverage by Module

| File | Line % | Lines | Function % | Functions |
| ------ | -------- | ------- | ------------ | ----------- |
| lib.rs | 100.0% | 698/698 | 100.0% | 68/68 |
| pagination.rs | 100.0% | 23/23 | 100.0% | 4/4 |
| permissions.rs | 100.0% | 20/20 | 100.0% | 3/3 |
| settings_mqtt.rs | 100.0% | 47/47 | 100.0% | 6/6 |
| system_alerts.rs | 100.0% | 61/61 | 100.0% | 8/8 |
| validation.rs | 100.0% | 10/10 | 100.0% | 2/2 |
| mqtt_url.rs | 97.4% | 152/156 | 100.0% | 22/22 |
| update_hooks.rs | 78.1% | 452/579 | 80.7% | 46/57 |
| oidc_providers.rs | 74.5% | 35/47 | 100.0% | 4/4 |
| scheduler.rs | 72.2% | 13/18 | 100.0% | 1/1 |
| auth.rs | 70.9% | 39/55 | 100.0% | 2/2 |
| registration.rs | 69.6% | 16/23 | 75.0% | 3/4 |
| settings_network.rs | 67.9% | 19/28 | 100.0% | 1/1 |
| update_history.rs | 47.4% | 18/38 | 50.0% | 3/6 |
| provider_configs.rs | 42.9% | 9/21 | 33.3% | 1/3 |
| software_items.rs | 24.5% | 13/53 | 20.0% | 1/5 |
| services.rs | 0.0% | 0/12 | 0.0% | 0/2 |

## Uncovered Critical Paths

### Tier 2 — Important

- **Update hooks validation** (`update_hooks.rs`, 78.1% coverage): 127 uncovered lines across webhook URL validation,
  retry policy configuration, and conditional trigger logic. Significant regression from prior 94.7% coverage likely due
  to new code additions.

### Tier 3 — Supporting

- **Update history types** (`update_history.rs`, 47.4% coverage): 20 uncovered lines in update history response
  construction and status conversion logic.
- **Software items validation** (`software_items.rs`, 24.5% coverage): 40 uncovered lines in request validation for
  `CreateSoftwareItemRequest` and `UpdateSoftwareItemRequest` edge cases. Significant regression from prior 51.9% coverage.
- **Provider configs validation** (`provider_configs.rs`, 42.9% coverage): 12 uncovered lines in provider configuration
  request validation. Regression from prior 100% coverage due to new code paths.
- **Registration request validation** (`registration.rs`, 69.6% coverage): 7 uncovered lines in device registration
  request validation edge cases.
- **Service response types** (`services.rs`, 0% coverage, 12 lines): `ServiceResponse` builder or conversion. Small and low risk.
- **OIDC provider request validation** (`oidc_providers.rs`, 74.5% coverage): 12 uncovered lines in OIDC provider creation
  request validation for edge cases like empty scope lists and invalid URLs.
- **Network settings validation** (`settings_network.rs`, 67.9% coverage): 9 uncovered lines in trusted proxy CIDR validation.
- **Auth request validation** (`auth.rs`, 70.9% coverage): 16 uncovered lines in login request and password change validation
  edge cases.

## Test Recommendations

1. **Update hooks validation tests** — Test webhook URL validation, retry policy edge cases, and conditional trigger
   configuration. Covers `update_hooks.rs` gaps (Tier 2). High priority due to coverage regression.
2. **Software item request validation tests** — Test `CreateSoftwareItemRequest` with invalid package identifiers, missing
   required fields, and conflicting options. Covers `software_items.rs` gaps (Tier 3). High priority due to coverage regression.
3. **Provider configs validation tests** — Test provider configuration creation and update requests with invalid fields
   and missing required values. Covers `provider_configs.rs` gaps (Tier 3). High priority due to coverage regression.
4. **Update history type tests** — Test update history response construction, status enum conversion, and serialization
   edge cases. Covers `update_history.rs` gaps (Tier 3). Simple unit tests.
5. **Registration request edge cases** — Test device registration with boundary values and missing optional fields.
   Covers `registration.rs` gaps (Tier 3). Simple unit tests.
6. **OIDC provider validation edge cases** — Test creation requests with empty scopes, malformed issuer URLs, and invalid slug
   characters. Covers `oidc_providers.rs` gaps (Tier 3). Extend existing validation tests.
7. **Network settings CIDR validation tests** — Test trusted proxy configuration with invalid CIDR ranges, IPv6 addresses, and
   overlapping ranges. Covers `settings_network.rs` gaps (Tier 3). Simple unit tests.
8. **Auth request edge cases** — Test login with boundary-length passwords and password change with mismatched confirmation.
   Covers `auth.rs` gaps (Tier 3). Simple unit tests.
9. **Service response conversion test** — Test `ServiceResponse` construction. Covers `services.rs` (Tier 3). Trivial unit test.
