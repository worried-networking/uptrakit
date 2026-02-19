# Test Coverage: uptrakit-openapi-client

> Generated: 2026-02-19 | Tool: cargo-llvm-cov 0.8.4 | Rust: 1.93.1

## Summary

| Metric | Value |
| -------- | ------- |
| Line coverage | 44.9% (592 / 1,318) |
| Function coverage | 27.2% (72 / 265) |
| Test count | 59 |

## Coverage by Module

| File | Line % | Lines | Function % | Functions |
| ------ | -------- | ------- | ------------ | ----------- |
| update_history.rs | 68.4% | 26/38 | 33.3% | 2/6 |
| oidc_auth.rs | 60.7% | 34/56 | 33.3% | 4/12 |
| auth.rs | 59.5% | 69/116 | 30.8% | 8/26 |
| hosts.rs | 58.5% | 31/53 | 33.3% | 4/12 |
| settings_mqtt.rs | 50.6% | 42/83 | 17.6% | 3/17 |
| provider_configs.rs | 49.3% | 33/67 | 23.1% | 3/13 |
| services.rs | 49.2% | 58/118 | 33.3% | 9/27 |
| oidc_providers.rs | 48.7% | 38/78 | 12.5% | 2/16 |
| software_items.rs | 47.0% | 63/134 | 20.0% | 5/25 |
| scheduler.rs | 45.2% | 19/42 | 20.0% | 2/10 |
| settings.rs | 41.4% | 41/99 | 15.4% | 4/26 |
| lib.rs | 33.2% | 131/395 | 41.7% | 25/60 |
| api_tokens.rs | 30.4% | 7/23 | 14.3% | 1/7 |
| health.rs | 0.0% | 0/4 | 0.0% | 0/2 |
| pki.rs | 0.0% | 0/8 | 0.0% | 0/4 |
| system_alerts.rs | 0.0% | 0/4 | 0.0% | 0/2 |

## Uncovered Critical Paths

### Tier 3 — Supporting

- **Client core** (`lib.rs`, 33.2% coverage, 395 lines): HTTP client construction, token management, request building, and
  response parsing. 264 uncovered lines include error handling for HTTP failures, token refresh logic, and TLS configuration.
  Risk: client failures could prevent CLI and other consumers from communicating with the controller.
- **PKI endpoints** (`pki.rs`, 0% coverage, 8 lines): CA certificate download client methods. Risk: untested PKI client could
  fail during TOFU bootstrap.
- **Health check** (`health.rs`, 0% coverage, 4 lines): Health endpoint client method. Risk: minimal.
- **System alerts** (`system_alerts.rs`, 0% coverage, 4 lines): System alert retrieval client method. Risk: minimal.
- **Settings methods** (`settings.rs`, 41.4% coverage): 58 uncovered lines across settings read/write methods.
- **OIDC provider methods** (`oidc_providers.rs`, 48.7% coverage): 40 uncovered lines across OIDC provider CRUD methods.
- **API token methods** (`api_tokens.rs`, 30.4% coverage): 16 uncovered lines across token CRUD methods.

## Test Recommendations

1. **Client construction and token management tests** — Test client builder with various configurations, token refresh on 401,
   and TLS certificate pinning. Covers `lib.rs` gaps (Tier 3). Use `mockito` or `wiremock` for HTTP mocking.
2. **PKI client method tests** — Test CA certificate download and error handling. Covers `pki.rs` (Tier 3). Mock HTTP response.
3. **Settings client round-trip tests** — Test each settings category read/write with mock server. Covers `settings.rs` and
   `settings_mqtt.rs` gaps (Tier 3). Use `wiremock` with expected request/response pairs.
4. **OIDC provider client tests** — Test CRUD operations for OIDC providers. Covers `oidc_providers.rs` gaps (Tier 3).
   Mock HTTP.
5. **API token client tests** — Test token creation, listing, and revocation. Covers `api_tokens.rs` gaps (Tier 3). Mock HTTP.
6. **Error response handling tests** — Test client behavior on 4xx and 5xx HTTP responses across different endpoints. Covers
   error handling gaps across all modules (Tier 3). Systematic mock error responses.
