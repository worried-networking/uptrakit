# Cross-Cutting Findings (from CLI + workspace review)

These findings affect multiple crates and should be merged into the root `CODEREVIEW.md`.

---

## Validated Existing Findings (still active in current code)

### [HIGH] TOCTOU race in `find_or_create_software_item`

- **Dimension**: database, correctness
- **Scope**: `crates/ui/web-api-queries/src/queries/autodiscovery/discovery_items.rs`
- **Status**: Still present. The three-phase upsert with `(tenant_id, name)` fallback remains.

### [HIGH] `unreachable!()` panics in production WebSocket handler

- **Dimension**: fault tolerance, maintainability
- **Scope**: `crates/ui/web-api/src/routes/service_ws/handler/mod.rs:420,444,465`
- **Status**: Still present. Three `unreachable!()` macros in capability dispatch arms.

### [HIGH] No generic stale-update recovery for orphaned `update_history` rows

- **Dimension**: high availability, fault tolerance, database
- **Scope**: update dispatch and handler paths across scheduler, web-api-queries, web-api
- **Status**: Still present. No age-based cleanup executor for `InProgress` rows.

### [MEDIUM] Silent drop under backpressure in control-plane channels

- **Dimension**: high availability, consistency
- **Scope**: `crates/ui/web-api/src/notifications/dispatcher.rs:48`,
  `crates/ui/web-api/src/service_connections.rs:334`
- **Status**: Still present. `try_send()` with warning-only logging.

### [MEDIUM] `dispatch_loop` has no timeout on `recv()` and unmonitored spawned tasks

- **Dimension**: fault tolerance, observability
- **Scope**: `crates/ui/web-api/src/notifications/dispatcher.rs:107`
- **Status**: Still present.

### [MEDIUM] Encryption AAD lookup falls back to empty string

- **Dimension**: security, correctness
- **Scope**: `crates/shared/crypto/src/encrypted_string.rs`
- **Status**: Not re-verified in this review cycle (crypto crate not in CLI scope). Carried forward from prior review.

### [MEDIUM] OIDC state store does not distinguish expired from never-existed tokens

- **Dimension**: security
- **Scope**: `crates/ui/web-api-auth/src/auth/oidc_state.rs:OidcFlowStore::take`
- **Status**: Not re-verified in this review cycle. Carried forward from prior review.

### [MEDIUM] `start_paused` rule violations in service WebSocket integration tests

- **Dimension**: test correctness
- **Scope**: `crates/ui/web-api/src/integration_tests/service_ws.rs`
- **Status**: Still present. Tests at lines 21, 58, 141, 171 use `tokio::time::timeout()`
  without `start_paused = true`.

### [LOW] `uptrakit-shared-types` is still too broad for a high-fanout crate

- **Dimension**: extensibility, crate structure, maintainability
- **Scope**: `crates/shared/types/src/lib.rs`
- **Status**: Still present.

### [LOW] `deliver_controller_event` and CLI `run` both exceed CC threshold

- **Dimension**: code quality, maintainability
- **Scope**: `crates/ui/web-api/src/event_delivery.rs` (CC=37), `crates/ui/cli/src/main.rs` (CC=38)
- **Status**: Still present.

---

## Potentially Stale Findings

### [HIGH] Potential migration ordering defect on fresh installs

- **Dimension**: database, high availability
- **Scope**: `crates/shared/db/src/migration/mod.rs`
- **Recommendation**: Downgrade or remove. After detailed inspection,
  `m20260302_000001_add_missing_indexes` only creates indexes on tables from migrations that
  precede it in the vec (`update_history`, `host_software_items`, `mqtt_leases`,
  `service_hosts`, `sessions`, `host_software_item_plugins`). None of these are created by
  `m20260302_000002_host_packages`. The vec ordering between these two specific migrations
  is safe. However, note that `m20260302_000002_host_packages` appears at line 78, after
  `m20260306_000002_update_batches` at line 77 -- the date-based filename ordering does not
  match vec ordering. This is cosmetic but potentially confusing during future migration
  additions. The fresh-install test `migrations_run_on_empty_sqlite` covers this.
  **Suggest reclassifying as [LOW] cosmetic ordering concern or removing entirely.**

---

## New Cross-Cutting Findings

### [MEDIUM] `uptrakit-backoff` is adopted in only 4 of ~15 crates that make outbound requests

- **Dimension**: consistency, HA/fault-tolerance
- **Scope**: `crates/shared/backoff/` is depended on by `web-api`, `service-sdk`, `nats`,
  `agent-core`, and `npm`. However, the CLI (`crates/ui/cli/`), all notification plugins
  (`webhook`, `telegram`, `email`), and several release plugins make outbound HTTP calls
  without retry.
- **Description**: The CLI does not retry any API calls. For short-lived commands this is
  acceptable (the user can re-run), but the `--follow` SSE paths in `tail.rs` and
  `batch_update.rs` would benefit from reconnect-with-backoff when the stream drops. Currently
  a transient network blip during a long-running `follow` operation terminates the CLI with
  exit code 2 ("disconnected").
- **Why it matters**: The SSE streams are the CLI's only long-lived connections. They are the
  most likely to encounter transient failures (TCP reset, server restart, LB timeout).
- **Failure scenario**: User runs `uptrakit update trigger --follow`, server performs a rolling
  restart, SSE stream drops, CLI exits with "Stream ended without completion event" even though
  the update is still running successfully.

### [LOW] No structured exit status for `api` subcommand error responses

- **Dimension**: consistency, code quality
- **Scope**: `crates/ui/cli/src/commands/api.rs:68-73`
- **Description**: The `api` command prints the HTTP response body to stdout *before* checking
  if the status is an error. If the response is a 4xx/5xx, it then returns a `CliError::Api`,
  which causes `main()` to print the error to stderr and exit(1). This means error responses
  are printed twice: once as formatted output, once as the error message.
- **Why it matters**: Scripts parsing CLI output may see the response body on stdout and the
  error summary on stderr for the same failed request.
- **Failure scenario**: `uptrakit api GET /api/v1/nonexistent --output json` prints the 404
  response JSON on stdout and then "Error: API error (404 Not Found): HTTP 404 Not Found" on
  stderr.

### [LOW] Inconsistent parameter threading style across command modules

- **Dimension**: consistency, maintainability
- **Scope**: All command modules in `crates/ui/cli/src/commands/`
- **Description**: Three different patterns coexist for passing connection parameters to
  command functions: (1) dedicated `*Params` structs (e.g., `hosts.rs`, `plugin_configs.rs`),
  (2) individual function arguments (e.g., `users.rs:list`, `services.rs:show`), (3) direct
  `CliContext` reference (only in dispatch functions, never in leaf functions). The choice
  appears to depend on when the module was written rather than any structural criterion.
- **Why it matters**: A future change to add a cross-cutting parameter (e.g., a request ID,
  tenant override) touches all three patterns differently.
- **Failure scenario**: Not a runtime failure, but increases maintenance burden for any
  cross-cutting CLI feature.
