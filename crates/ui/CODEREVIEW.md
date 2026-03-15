# Code Review: crates/ui (aggregate)

- **Review date**: 2026-03-15
- **Reviewer**: AI code review (architecture|security|quality|HA|standards|extensibility|tests|consistency|maintainability|database|crate-structure)
- **Branch**: docs/codereview-backend

## Sub-crate Reviews

| Crate | File | Size | Summary |
| --- | --- | ---: | --- |
| `uptrakit-web-api` | [web-api/CODEREVIEW.md](web-api/CODEREVIEW.md) | ~38K LoC | HTTP/WebSocket API server, OIDC, PKI, MQTT, notifications |
| `uptrakit-web-api-auth` | [web-api-auth/CODEREVIEW.md](web-api-auth/CODEREVIEW.md) | ~3.6K LoC | JWT, sessions, OIDC state, rate limiting, device flow |
| `uptrakit-web-api-queries` | [web-api-queries/CODEREVIEW.md](web-api-queries/CODEREVIEW.md) | ~7.8K LoC | SeaORM query modules, tenant isolation, batch operations |
| `uptrakit-cli` | [cli/CODEREVIEW.md](cli/CODEREVIEW.md) | ~11K LoC | Operator CLI binary |

## Cross-cutting Strengths

- **Security primitives**: Argon2id password hashing, JWT denylist with cross-instance NATS
  propagation, PKCE-enforced OIDC flows, `EncryptedString` at rest, `SsrfSafeResolver` on
  all user-controlled URLs, typed permission extractors with compile-time safety.
- **Tenant isolation**: `TenantDb` abstraction with `find_via_tenant_join` for join tables;
  `TenantScoped` trait for compile-time tenant filtering across query modules.
- **Rate limiting**: DB-backed sliding-window counters (HA-safe, TOCTOU-resistant) applied to
  all auth endpoints and WebSocket auth-failure paths.
- **Error propagation**: `rootcause::Report` + `context_to()` / `bail!` / `report!` used
  uniformly; no `Result<T, String>` or bare `.unwrap()` in production paths.
- **Async lock discipline**: `parking_lot::Mutex` / `parking_lot::RwLock` used correctly
  throughout; guards dropped before all `.await` points in the reviewed paths.
- **Batch loading**: N+1 prevention via `is_in()` batch queries and `HashMap`-join assembly
  in all list endpoints.
- **Transaction safety**: Multi-statement write operations wrapped in `db.begin()/txn.commit()`
  with `context_to()?` on all exec calls.
- **CLI design**: `lib` + `bin` split, `SecretString` at credential boundaries, URL scheme
  validation before browser open, `HumanOutput` trait for uniform output formatting.

## Cross-cutting Concerns

### Security

| Severity | Crate | Finding |
| --- | --- | --- |
| **High** | web-api | Extension action `ActionDef::permission` declared but never enforced at the invocation layer — any authenticated user can invoke any extension action |
| **Medium** | web-api | Login endpoint leaks deactivated-user existence via 403 vs 401 status code divergence (user enumeration) |
| **Medium** | web-api | `oidc_http_client.rs` missing `SsrfSafeResolver`; OIDC provider URL is operator-configured |
| **Medium** | web-api | Interactive WS handler (`interactive_ws.rs`) has multiple production `unwrap()` calls on `serde_json` operations |
| **Low** | web-api-auth | Un-keyed SHA-256 for API token and enrollment secret hashes (`auth/token.rs:17-21`); HMAC-SHA256 with server secret would add defense-in-depth |
| **High** | cli | API token stored in plaintext JSON on disk (`config.rs:43-50`) |

### Architecture

| Severity | Crate | Finding |
| --- | --- | --- |
| **High** | web-api | God-crate (~38K LoC, 5 concern domains); recommended two-phase extraction: `uptrakit-web-api-pki` then `uptrakit-web-api-push` |
| **High** | web-api | `AppState` god-object with 26+ `pub` fields; introduce domain-scoped accessors (`auth()`, `pki()`, `notification()`) |
| **Medium** | web-api | `router.rs` 929-line flat router with 80+ sequential `.routes()` calls; decompose into domain sub-routers |
| **High** | cli | `main.rs` 5,915-line monolith (Clap definitions + dispatch + tests); split into `src/cli.rs`, per-namespace dispatch helpers, `tests/cli_parsing.rs` |

### Tenant Isolation

| Severity | Crate | Location | Finding |
| --- | --- | --- | --- |
| **High** | web-api | `routes/service_ws/interactive_ws.rs` | `ServiceHost::find()` without tenant join |
| **High** | web-api | `routes/software_items.rs:1131`, `routes/hosts.rs:218` | `ServiceHost::find()` without tenant join |
| **Medium** | web-api-queries | `services.rs:380` | `ServiceHost::find()` without `find_via_tenant_join` |
| **Medium** | web-api-queries | `mqtt_software_states.rs:99,113` | `Host::find()` and `UpdateHistory::find()` without `tenant_id` filter |
| **Medium** | web-api-queries | `update_triggers.rs` | At least one `ServiceHost::find()` without tenant join |

### Logic Consistency

| Severity | Crate | Finding |
| --- | --- | --- |
| **High** | web-api | `dispatch_update_to_agent` errors in `create_batch` silently logged, not propagated |
| **High** | web-api | `handle_update_started` and `handle_update_result`: `active.update()` errors logged but not propagated — state machine can diverge from DB |
| **High** | web-api | `host_software_item` version update errors (lines 756-777) logged but not propagated |
| **Medium** | web-api | `deliver_pending_updates` early-returns on reconnect failure, leaving pending items undelivered (reconnect deadlock) |
| **Medium** | web-api | `deliver_pending_updates` hardcodes `interactive=false` on reconnect — violates persisted `update_history.interactive` |
| **Medium** | web-api | `deliver_pending_updates` silently skips items missing `execute_update` plugin without marking them Failed (pending-forever bug) |
| **Medium** | web-api-queries | `update_batches/dispatch.rs`: `maybe_complete_batch` lacks status re-check before transition (concurrent double-transition risk) |
| **Medium** | web-api-queries | `update_batches/dispatch.rs`: `load_target_for_dispatch` failure marks item Failed without triggering batch completion (stuck batch) |
| **Medium** | web-api-queries | `update_triggers.rs`: `is_unique_constraint_violation()` relies on fragile DB error message string-matching |

### Coding Standards Violations

| Severity | Crate | Location | Finding |
| --- | --- | --- | --- |
| **High** | web-api | `routes/oidc_auth.rs:420,512,725,773` | 4× `#[allow(clippy::too_many_arguments)]` |
| **High** | web-api | `routes/service_ws/handler/mod.rs:605` | `#[allow(clippy::too_many_arguments)]` |
| **High** | web-api | `extension_proxy.rs:114` | `#[allow(clippy::too_many_arguments)]` |
| **High** | web-api | `settings.rs:183,296` | `tokio::sync::Mutex` instead of `parking_lot::Mutex` |
| **High** | web-api | `service_connections.rs` | `tokio::sync::RwLock` instead of `parking_lot::RwLock` |
| **High** | web-api | `event_broadcaster.rs` | `tokio::sync::RwLock` instead of `parking_lot::RwLock` |
| **High** | web-api-queries | 12 modules | `#[allow(clippy::type_complexity)]` — all need type aliases |

### Test Coverage Gaps (Critical Paths)

| Severity | Crate | Path | Coverage |
| --- | --- | --- | ---: |
| **High** | web-api | `routes/service_ws/handler/updates.rs` (update lifecycle) | 0% |
| **High** | web-api | `routes/service_ws/handler/messages.rs` | 0% |
| **High** | web-api | `routes/service_ws/handler/mqtt.rs` | 0% |
| **High** | web-api | `routes/oidc_auth.rs` (`oidc_callback` 7 paths) | 8.8% |
| **High** | web-api | `routes/device_auth.rs` | 9.7% |
| **High** | web-api | Extension action permission enforcement | 0% |
| **High** | web-api-auth | `resolve_oidc_user` (6-variant resolution, DB-touching) | ~0% |
| **High** | web-api-auth | `settings_store.rs` (`generate_or_load_jwt_key`, `load_settings_snapshot`) | 0% |
| **High** | web-api-queries | `notifications.rs`, `host_packages.rs`, `services.rs`, `plugin_configs.rs` | 0% |
| **Medium** | web-api | Route permission enforcement (403 for under-privileged users) | 0% |
| **Medium** | cli | 10 of 13 command namespaces have no mock-server integration tests | — |

### Maintainability Hot Spots

| File | Lines | Issue |
| --- | ---: | --- |
| `web-api/routes/service_ws/handler/mod.rs` | 1720 | `handle_authenticated_loop` 700+ lines, high cyclomatic complexity |
| `web-api/routes/oidc_auth.rs` | 1515 | `oidc_callback` 413 lines, 7 untested paths, no state machine docs |
| `web-api/routes/service_ws/handler/updates.rs` | 1433 | Full update lifecycle, 0% coverage |
| `web-api/queries/autodiscovery.rs` | 1848 | Mixed concerns, no tests |
| `web-api/queries/software_items.rs` | 1376 | Mixed CRUD + assignment + version-check |
| `web-api/app_state.rs` | 777 | 26 public fields, god-object |
| `web-api/router.rs` | 929 | 80+ flat `.routes()` calls |
| `cli/main.rs` | 5915 | Clap definitions + dispatch + tests combined |

## Open Action Items (by priority)

1. **CRITICAL — Security**: Enforce `ActionDef::permission` in extension action invocation
   (`routes/extensions.rs:143`).
2. **HIGH — Correctness**: Fix silent error swallowing in update lifecycle state machine
   (`handle_update_started`, `handle_update_result`, `create_batch` dispatch errors).
3. **HIGH — Tenant isolation**: Replace all bare `ServiceHost::find()` calls with
   `find_via_tenant_join` in `interactive_ws.rs`, `software_items.rs`, `hosts.rs`,
   `services.rs` (queries), `update_triggers.rs` (queries).
4. **HIGH — Standards**: Replace `tokio::sync::RwLock`/`Mutex` with `parking_lot` equivalents
   in `service_connections.rs`, `event_broadcaster.rs`, `settings.rs`.
5. **HIGH — Standards**: Eliminate all `#[allow(clippy::too_many_arguments)]` and
   `#[allow(clippy::type_complexity)]` suppressions.
6. **HIGH — Tests**: Add integration tests for update lifecycle, OIDC callback paths,
   extension permission enforcement, and `resolve_oidc_user`.
7. **HIGH — Architecture**: Decompose `router.rs` into domain sub-routers; split `cli/main.rs`.
8. **MEDIUM — Correctness**: Fix `deliver_pending_updates` reconnect logic (interactive flag,
   missing-plugin skip, early-return on failure).
9. **MEDIUM — Correctness**: Fix `maybe_complete_batch` double-transition race;
   fix stuck-batch on `load_target_for_dispatch` failure.
10. **MEDIUM — Architecture**: Begin god-crate extraction (`uptrakit-web-api-pki` phase 1);
    introduce domain-scoped `AppState` accessors.
