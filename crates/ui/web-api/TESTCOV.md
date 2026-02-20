# Test Coverage: uptrakit-web-api

> Generated: 2026-02-20 | Tool: cargo-llvm-cov 0.8.4 | Rust: 1.93.1

## Summary

| Metric | Value | | -------- | ------- | | Line coverage | 44.4% (7,601 / 17,117) | | Function coverage | 53.4% (775 / 1,451) | | Test count | 278 |

## Coverage by Module

| File | Line % | Lines | Function % | Functions | | ------ | -------- | ------- | ------------ | ----------- | | auth/api_token.rs | 100.0% | 192/192
| 100.0% | 26/26 | | auth/token.rs | 100.0% | 57/57 | 100.0% | 11/11 | | middleware/request_log.rs | 100.0% | 20/20 | 100.0% | 4/4 | |
routes/health.rs | 100.0% | 4/4 | 100.0% | 2/2 | | setting_key.rs | 100.0% | 90/90 | 100.0% | 8/8 | | update_hooks.rs | 100.0% | 474/474 | 100.0% |
44/44 | | auth/oidc_state.rs | 98.7% | 609/617 | 100.0% | 62/62 | | auth/password.rs | 98.6% | 69/70 | 100.0% | 11/11 | | middleware/resolve_ip.rs |
98.5% | 337/342 | 94.9% | 37/39 | | extract.rs | 98.0% | 144/147 | 100.0% | 20/20 | | pki_utils.rs | 97.1% | 170/175 | 77.8% | 14/18 | |
auth/token_denylist.rs | 97.0% | 96/99 | 95.5% | 21/22 | | auth/jwt.rs | 96.4% | 107/111 | 85.7% | 12/14 | | auth/session.rs | 96.3% | 341/354 | 92.3%
| 36/39 | | auth/refresh_cookie.rs | 94.9% | 56/59 | 80.0% | 8/10 | | auth/device_flow.rs | 93.8% | 289/308 | 87.8% | 43/49 | | auth/rate_limit.rs |
92.3% | 241/261 | 100.0% | 21/21 | | middleware/require_auth.rs | 89.0% | 268/301 | 89.7% | 26/29 | | lib.rs | 87.9% | 385/438 | 75.0% | 24/32 | |
notification_service.rs | 86.9% | 252/290 | 76.9% | 20/26 | | middleware/rate_limit.rs | 85.7% | 168/196 | 100.0% | 10/10 | | auth/registration.rs |
65.2% | 202/310 | 78.4% | 29/37 | | middleware/resolve_proxy_headers.rs | 59.5% | 185/311 | 75.7% | 28/37 | | auth/authentication.rs | 51.9% | 181/349
| 71.0% | 22/31 | | mqtt_lease_coordinator.rs | 51.3% | 327/637 | 44.9% | 22/49 | | routes/server_cert.rs | 50.3% | 93/185 | 42.3% | 11/26 | |
service_connections.rs | 49.6% | 185/373 | 43.9% | 29/66 | | ocsp.rs | 48.6% | 180/370 | 45.2% | 19/42 | | routes/provider_configs.rs | 46.6% |
205/440 | 58.5% | 24/41 | | routes/update_history.rs | 44.5% | 126/283 | 35.0% | 7/20 | | routes/hosts.rs | 42.4% | 126/297 | 45.0% | 9/20 | |
routes/settings_combined.rs | 42.3% | 44/104 | 40.0% | 2/5 | | event_poller.rs | 41.6% | 101/243 | 52.0% | 13/25 | | routes/services.rs | 40.6% |
268/660 | 31.3% | 10/32 | | routes/auth.rs | 39.2% | 248/632 | 36.6% | 15/41 | | routes/software_items.rs | 24.8% | 260/1,050 | 34.0% | 16/47 | |
routes/ssh_agent_ws.rs | 22.8% | 136/596 | 37.5% | 12/32 | | routes/mqtt_ws.rs | 18.7% | 112/600 | 37.0% | 10/27 | | routes/service_ws.rs | 17.1% |
137/802 | 29.2% | 14/48 | | settings.rs | 15.1% | 59/391 | 11.8% | 10/85 | | routes/oidc_auth.rs | 4.2% | 36/859 | 17.6% | 9/51 | | settings_store.rs
| 1.1% | 3/279 | 3.6% | 1/28 | | auth/error.rs | 0.0% | 0/1 | 0.0% | 0/1 | | middleware/tenant_context.rs | 0.0% | 0/9 | 0.0% | 0/2 | |
mqtt_client_store.rs | 0.0% | 0/157 | 0.0% | 0/18 | | routes/agent_ws.rs | 0.0% | 0/711 | 0.0% | 0/16 | | routes/agents.rs | 0.0% | 0/246 | 0.0% |
0/16 | | routes/api_tokens.rs | 0.0% | 0/68 | 0.0% | 0/9 | | routes/device_auth.rs | 0.0% | 0/124 | 0.0% | 0/13 | | routes/ocsp.rs | 0.0% | 0/63 |
0.0% | 0/4 | | routes/oidc_providers.rs | 0.0% | 0/324 | 0.0% | 0/17 | | routes/scheduler.rs | 0.0% | 0/183 | 0.0% | 0/12 | | routes/settings.rs |
0.0% | 0/48 | 0.0% | 0/5 | | routes/settings_agent_certs.rs | 0.0% | 0/62 | 0.0% | 0/4 | | routes/settings_auth.rs | 0.0% | 0/53 | 0.0% | 0/4 | |
routes/settings_ca.rs | 0.0% | 0/21 | 0.0% | 0/2 | | routes/settings_mqtt.rs | 0.0% | 0/363 | 0.0% | 0/19 | | routes/settings_network.rs | 0.0% |
0/197 | 0.0% | 0/7 | | routes/system_alerts.rs | 0.0% | 0/71 | 0.0% | 0/4 |

> **Note:** This crate requires `--features db-sqlite` for the mock-database tests. Many route handlers remain at 0% because they need integration
> test infrastructure (full HTTP server with database).

## Uncovered Critical Paths

### Tier 1 — Security-Critical

- **OIDC authentication routes** (`routes/oidc_auth.rs`, 4.2% coverage, 859 lines): Authorization URL generation, callback handling, token exchange,
  account linking, and registration-via-OIDC. Risk: flawed OIDC flows could enable authentication bypass.
- **OCSP responder route** (`routes/ocsp.rs`, 0% coverage, 63 lines): HTTP endpoint that serves OCSP responses for certificate validation. The core
  OCSP logic (`ocsp.rs`) has 48.6% coverage but the route handler is untested.
- **Agent certificate settings** (`routes/settings_agent_certs.rs`, 0% coverage, 62 lines): Agent certificate lifetime and renewal policy
  configuration.

### Tier 2 — Business-Logic

- **Agent WebSocket handler** (`routes/agent_ws.rs`, 0% coverage, 711 lines): Full agent communication lifecycle including enrollment
  approval/rejection, certificate signing, version checks, and update execution dispatch.
- **SSH agent WebSocket handler** (`routes/ssh_agent_ws.rs`, 22.8% coverage, 596 lines): SSH agent enrollment, host reporting, and command execution
  over WebSocket.
- **Service WebSocket handler** (`routes/service_ws.rs`, 17.1% coverage, 802 lines): Generic service WebSocket lifecycle, message routing, and
  graceful shutdown.
- **MQTT WebSocket handler** (`routes/mqtt_ws.rs`, 18.7% coverage, 600 lines): MQTT service enrollment and credential distribution.
- **Scheduler routes** (`routes/scheduler.rs`, 0% coverage, 183 lines): CRUD for scheduled tasks and manual task triggering.
- **Settings routes** (`routes/settings*.rs`, 0% coverage across 5 files, ~544 lines total): Network settings, auth settings, CA settings, and MQTT
  settings management.
- **MQTT client store** (`mqtt_client_store.rs`, 0% coverage, 157 lines): MQTT client lifecycle management and connection tracking.
- **Device auth routes** (`routes/device_auth.rs`, 0% coverage, 124 lines): Device authorization flow HTTP endpoints.
- **OIDC provider management** (`routes/oidc_providers.rs`, 0% coverage, 324 lines): OIDC provider CRUD operations.
- **System alerts** (`routes/system_alerts.rs`, 0% coverage, 71 lines): System health alert generation and retrieval.

### Tier 3 — Supporting

- **Settings store** (`settings_store.rs`, 1.1% coverage): Raw settings persistence and caching layer.
- **Settings reconciliation** (`settings.rs`, 15.1% coverage): Settings change detection and runtime reconfiguration.
- **Service connections** (`service_connections.rs`, 49.6% coverage): Service connection registry and load balancing.
- **API token routes** (`routes/api_tokens.rs`, 0% coverage, 68 lines): API token CRUD endpoints.
- **Agent management routes** (`routes/agents.rs`, 0% coverage, 246 lines): Agent listing, CSR signing, and certificate revocation.

## Test Recommendations

1. **OIDC callback and account linking tests** — Test authorization URL generation, token exchange, auto-create user, and account linking. Covers
   `routes/oidc_auth.rs` (Tier 1). Mock OIDC provider HTTP responses.
1. **Agent WebSocket lifecycle tests** — Test enrollment handshake, certificate signing, version check dispatch, and update execution message flow.
   Covers `routes/agent_ws.rs` (Tier 2). Use in-memory WebSocket pairs.
1. **OCSP route handler test** — Test the HTTP POST endpoint with valid/invalid DER-encoded requests. Covers `routes/ocsp.rs` (Tier 1). Reuse existing
   `ocsp.rs` test infrastructure.
1. **Scheduler CRUD tests** — Test task creation, listing, update, deletion, and manual trigger. Covers `routes/scheduler.rs` (Tier 2). Mock database
   with in-memory SQLite.
1. **Settings management round-trip tests** — Test each settings category (network, auth, CA, MQTT, agent certs) update and retrieval. Covers
   `routes/settings*.rs` (Tier 2). Use `AppState` test helper.
1. **Device auth flow route tests** — Test device code generation, polling, and approval endpoints. Covers `routes/device_auth.rs` (Tier 2). Reuse
   `auth/device_flow.rs` test patterns.
1. **MQTT client store tests** — Test client registration, deregistration, and connection tracking. Covers `mqtt_client_store.rs` (Tier 2).
   Unit-testable with mock state.
1. **Service connection load balancing tests** — Test connection registration, deregistration, and least-busy selection under concurrent load. Covers
   `service_connections.rs` (Tier 3). Extend existing tests.
