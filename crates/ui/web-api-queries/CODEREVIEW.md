# Code Review: uptrakit-web-api-queries

- **Review date**: 2026-03-05
- **Reviewer**: AI coverage analysis (cargo-llvm-cov)
- **Branch**: docs/test-coverage

## Test Coverage Analysis

Overall crate coverage: 4,319 / 7,786 lines (55.5%).

### Query Modules With 0% Coverage

| File | Lines | Description |
| --- | ---: | --- |
| `queries/system_services.rs` | 175 | System service CRUD (list, get, approve, reject, deactivate) |
| `queries/system_enrollment_tokens.rs` | 133 | System enrollment token CRUD + active token lookup |
| `queries/audit_logs.rs` | 149 | Tenant + system audit log list with filters |
| `queries/scheduled_tasks.rs` | 110 | Scheduled task CRUD + cron normalization |

### Query Modules Below 50% Coverage

| File | Coverage | Lines |
| --- | ---: | ---: |
| `queries/discovery_allowlist.rs` | 18.6% | 317 |
| `queries/host_packages.rs` | 24.8% | 565 |
| `queries/mqtt_software_states.rs` | 30.9% | 317 |
| `queries/plugin_configs.rs` | 36.5% | 233 |
| `queries/notifications.rs` | 37.0% | 386 |
| `queries/update_history.rs` | 45.0% | 300 |
| `queries/update_batches.rs` | 47.9% | 1,014 |

### Critical Uncovered Paths

~~**[SECURITY] `system_enrollment_tokens.rs` — active token compound filter (0% coverage)**~~

> **Fixed:** Added 6 unit tests in `system_enrollment_tokens.rs` covering: expired token
> excluded, revoked token excluded, exhausted token excluded, unlimited token included,
> partially-used token (below max) included, and `revoke_system_enrollment_token` idempotency.

~~**[DATA INTEGRITY] `system_services.rs` — transactional deactivation (0% coverage)**~~

> **Fixed:** Added 6 unit tests in `system_services.rs` covering: `deactivate_system_service`
> atomically sets `deactivated_at` and revokes all certs with `ServiceDeactivated` reason,
> already-deactivated returns `false`, `approve_system_service` on non-pending returns
> `NotPending`, `reject_system_service` sets `Rejected` status and `deactivated_at`, and
> `update_system_service_settings` with `Some(0)` clears columns to `NULL`.

**[BUSINESS] `scheduled_tasks.rs` — cron validation (0% coverage)**

`update_scheduled_task` normalizes 5-field cron expressions by prepending `0`, then validates
via `cron::Schedule::from_str`. Invalid cron must return `InvalidCronExpression`.

Recommended tests:

- 5-field expression gets `0` prepended and parses correctly
- 6-field expression passes through unchanged
- Invalid cron string returns `InvalidCronExpression`
- `task_config: null` sets column to `NULL`
- Non-existent task ID returns `NotFound`
- `trigger_scheduled_task` sets `next_run_at` to now

**[BUSINESS] `discovery_allowlist.rs` — idempotent insert + fail-open (18.6% coverage)**

Only the `is_valid_discovery_plugin` unit tests have coverage. All DB functions are at 0%.

Recommended tests:

- `add_tenant_allowlist_entry`: first call inserts; second call returns existing (idempotent)
- `add_tenant_allowlist_entry` with `Other(...)` type returns `InvalidPluginType`
- `add_tenant_allowlist_entry` with non-discovery type returns `InvalidPluginType`
- `remove_tenant_allowlist_entry` with wrong `tenant_id` returns `false`
- `load_tenant_allowlist_set` DB failure returns empty set (fail-open)
- Host allowlist: same patterns as tenant allowlist

**[DATA INTEGRITY] `host_packages.rs` — find-or-create + deactivation (24.8% coverage)**

`find_or_create_host_package` handles concurrent-insert races via unique-violation recovery.
`deactivate_missing_host_packages` must respect the ignore set. `promote_host_package` is a
multi-step operation creating software items and plugin assignments.

Recommended tests:

- `find_or_create_host_package`: new package created; existing package updated; ignored package skipped
- `deactivate_missing_host_packages`: absent packages deactivated; ignored packages retained
- `promote_host_package`: creates software item + all 3 plugin roles; idempotent on second call
- `promote_host_package` with non-existent `software_item_id` returns `SoftwareItemNotFound`
- `compute_update_summaries_batch` with empty `host_ids` returns empty `HashMap`

**[BUSINESS] `audit_logs.rs` — timestamp filters + tenant isolation (0% coverage)**

Recommended tests:

- Tenant isolation: logs for tenant A not visible when querying tenant B
- Invalid RFC3339 `from`/`to` returns `InvalidFilter`
- Combined filters (`actor_type` + `status`) apply additively
- Empty result set returns `items: [], total: 0`

**[BUSINESS] `update_batches.rs` — batch lifecycle (47.9% coverage)**

The `maybe_complete_batch` function and `dispatch_next_in_batch` logic are partially covered.
Key uncovered paths include batch completion race conditions and the sequential dispatch
ordering.

Recommended tests:

- `maybe_complete_batch` inside a transaction completes atomically
- `dispatch_next_in_batch` sends to the correct host's agent
- Batch with all items completed transitions to `Completed` status
- Batch with a failed item still completes remaining items
