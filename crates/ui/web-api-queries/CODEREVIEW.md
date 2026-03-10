# Code Review: uptrakit-web-api-queries

- **Review date**: 2026-03-06
- **Parallel review date**: 2026-03-06
- **Reviewer**: AI coverage analysis (cargo-llvm-cov), AI code review (architecture|security|quality|HA|standards|extensibility|tests|consistency|maintainability|database)
- **Branch**: docs/test-coverage, docs/codereview-backend

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

## Coding Standards

### Issues

~~**[MEDIUM]** `queries/system_enrollment_tokens.rs:39,61,85,99,118,144,160` -- All seven public
functions return `Result<T, sea_orm::DbErr>` instead of the crate-local `Result<T>` alias
(which resolves to `Result<T, rootcause::Report>`). Every other query module in the workspace
uses the `rootcause` error propagation pattern with `context_to` / `bail!` / `report!`
conventions. Leaking `sea_orm::DbErr` through the public API boundary forces callers to handle
ORM-specific errors directly and bypasses the structured error context chain provided by
`rootcause::Report`. Fix: define a `SystemEnrollmentTokenError` enum or use the crate-local
`Result<T>` alias with `context_to` on every DB call, consistent with the pattern in
`queries/system_services.rs` and `queries/host_packages.rs`.~~ *(Fixed: added `SystemEnrollmentTokenError` + `Result<T>` alias; all 7 functions now use `context_to()`; `agents.rs` updated with `impl_report_conversion!`.)*

## Database

### Issues

**[MEDIUM]** `queries/system_services.rs:48-60` -- `service_status_to_db_status` has a
wildcard `_ =>` arm that silently maps unknown `ServiceStatus` variants to
`SystemServiceStatus::Pending`. Because `ServiceStatus` is `#[non_exhaustive]`, any new
variant added in a future release will match this arm and silently apply a `Pending` filter
instead of failing loudly. The conversion should return `Option<SystemServiceStatus>` (or
use `#[deny(unreachable_patterns)]` after a exhaustive match) so callers can decide how to
handle unrecognised variants rather than receiving misleading query results.

## Tenant Isolation

### Issues

~~**[MEDIUM] DB-1:** `queries/hosts.rs:123` -- `ServiceHost::find()` in `list_hosts` bypasses
`find_via_tenant_join`. The query loads `service_host` rows without any tenant filter. Although
the `host_ids` were already tenant-filtered from the previous query, and the subsequent
services fetch re-applies tenant filtering, this violates the stated convention. The
single-host helper `load_host_agents` on line 43 correctly uses `find_via_tenant_join`.
*Found in parallel database review (2026-03-06).*~~ *(Fixed: replaced with `tenant_db.find_via_tenant_join::<service_host::Entity, service::Entity>(...)`.)*

**[LOW] DB-2:** `queries/software_items.rs:237,255,269,284` -- `load_item_hosts` helper takes
a raw `&DatabaseConnection` rather than `&TenantDb` and queries `HostSoftwareItem::find()`,
`Host::find()`, `HostSoftwareItemPlugin::find()`, and `PluginConfig::find()` without tenant
filters. While the upstream caller has already scoped the `software_item_id` to the tenant,
any host from any tenant that happens to be linked to this item would appear in results.
*Found in parallel database review (2026-03-06).*

**[LOW] DB-3:** `queries/host_packages.rs:435` -- `find_or_create_host_package` omits
`tenant_id` filter. `host_package` is `TenantScoped` but the function receives a raw
`&DatabaseConnection`. The `host_id` uniqueness likely prevents cross-tenant data leakage,
but this is a defense-in-depth gap. The same pattern repeats at line 485 (race condition
recovery path). *Found in parallel database review (2026-03-06).*

**[LOW] DB-4:** `queries/software_items.rs:824` -- `assign_hosts` host existence check uses
`Host::find_by_id(host_id)` without `TenantId` filter. A user in tenant A could theoretically
assign a host belonging to tenant B. The `host_id` is provided by the caller in
`AssignHostsRequest`. *Found in parallel database review (2026-03-06).*

## N+1 Query Patterns

### Issues

**[LOW] DB-5:** `queries/services.rs:386-407` -- Per-row INSERT in `merge_service` for host
links. Each source link generates a separate INSERT. For a small number of host links per
service (typically 1-3), this is acceptable. A bulk INSERT with `insert_many` would be more
efficient but is not critical. *Found in parallel database review (2026-03-06).*

**[LOW] DB-6:** `queries/update_batches.rs:1025-1046` -- Per-row INSERT in batch creation.
Each `host_package_update_history` record is inserted individually inside a loop within a
transaction. For large batch updates this could be slow. `insert_many` would reduce
round-trips. *Found in parallel database review (2026-03-06).*

**[LOW] DB-11:** Multiple paginated list functions (e.g., `hosts.rs:109`,
`services.rs:97-101`) clone the full query builder to get the count and then re-execute for
the page. This results in two database round-trips with identical WHERE clauses. This is a
common SeaORM pattern and not easily avoidable without raw SQL window functions.
*Found in parallel database review (2026-03-06).*

**[LOW] DB-12:** `queries/software_items.rs:550-681` -- `list_software_items` executes 5-6
queries for a single page: COUNT, SELECT items, GROUP BY host counts, JOIN plugin types,
SELECT installed versions, plus `bulk_load_latest_versions`. Each is individually efficient,
but acceptable for the page sizes involved (20-100 items). *Found in parallel database review
(2026-03-06).*

## Transaction Safety

### Issues

**[LOW] DB-7:** `queries/host_packages.rs:188-228` -- `deactivate_host_package` lacks
transaction for ignore + deactivate. When `create_ignore` is true, the function first inserts
an ignore rule (line 216), then updates the host package (line 226). These are two separate
statements without a transaction. If the second statement fails, the ignore rule is orphaned.
*Found in parallel database review (2026-03-06).*

**[MEDIUM] DB-8:** `queries/host_packages.rs:645-776` -- `promote_host_package` performs
cross-function mutations without wrapping transaction. Calls `create_software_item` (its own
transaction), then `assign_hosts` (another transaction), then `HostSoftwareItem::update_many`
(no transaction). If the version copy fails after `assign_hosts` succeeds, the software item
exists without version data. These should be wrapped in a single encompassing transaction.
*Found in parallel database review (2026-03-06).*

## Multi-Backend Compatibility

### Issues

**[LOW] DB-13:** `queries/services.rs:89` -- LIKE for capability filtering in
`list_services`. `.contains()` generates a `LIKE '%value%'` query on a JSON text column. A
capability named `"ssh"` would match `"ssh_remote"`. This is a semantic issue rather than a
backend compatibility issue. *Found in parallel database review (2026-03-06).*

## Soft-Delete

### Issues

**[INFO] DB-14:** `queries/hosts.rs:261-277` -- `deactivate_host` does not cascade to
related join tables. When a host is soft-deleted, the `service_host`, `host_software_item`,
and `host_software_item_plugin` rows remain active. Queries on these tables filter via joins,
so deactivated hosts are excluded from results. However, orphaned rows accumulate. This is a
known trade-off of soft-deletes and is likely acceptable. *Found in parallel database review
(2026-03-06).*

## Architecture

### Issues

**[MEDIUM]** `queries/autodiscovery.rs` -- At 2,129 lines, the largest query file. Handles
discovery for targeted items, host packages, ignore rules, and target-based vs config-ID-based
processing. Given the complexity of the discovery subsystem (PHS, Docker, APT, Homebrew all
have different target emission strategies), this module could benefit from sub-module
extraction into a `queries/autodiscovery/` directory. *Found in parallel architecture review
(2026-03-06).*

## Tests

### Issues

**[HIGH]** 7 query modules with zero tests totaling 2,659 lines: `notifications.rs` (521),
`host_packages.rs` (793), `services.rs` (412), `plugin_configs.rs` (356),
`enrollment_tokens.rs` (206), `scheduled_tasks.rs` (170), `audit_logs.rs` (201). Some may be
exercised indirectly by `web-api` integration tests, but `host_packages`, `audit_logs`, and
`scheduled_tasks` have no corresponding integration test files either. *Found in parallel
tests review (2026-03-06).*

---

## Review — 2026-03-10

### Summary

Second review pass focusing on transaction safety in write paths, tenant isolation in batch
operations, N+1 patterns in multi-row mutations, and the coupling of the query layer to the
notification plugin registry. Items from the 2026-03-06 review that remain open are confirmed
below. New findings are additive.

### Strengths

- All list endpoints correctly implement batch loading to avoid N+1: `list_hosts` uses two
  queries total + in-memory map; `list_update_history` batches `update_output_line` records
  in a single `IS IN` query; `batch_count_hosts` aggregates assignment counts in one query.

### Concerns

#### Code Quality

| Severity | Location | Finding |
| --- | --- | --- |
| **High** | `queries/services.rs:344-367` | **`bump_revocation_version` failure swallowed in `merge_service`**: `tracing::warn!` is used instead of `?`, allowing the transaction to commit with an unbumped revocation counter. Revoked certificates may remain visible as valid until the next CRL refresh. Propagate the error with `?` consistent with `deactivate_service`. *Also confirmed in `web-api` review.* |
| **Medium** | `queries/services.rs:451-461,498-508` | **`batch_approve_services` and `batch_reject_services` N+1 without transaction**: one `UPDATE` per service in a loop with no wrapping transaction. Partial failures leave the DB in a partially-updated state. Use a single `update_many()` with `is_in()` filter on eligible IDs. |
| **Medium** | `queries/update_history.rs:218-260` | **Authorization-after-data-load in `get_update_history`**: the `update_history` row is loaded before the tenant authorization check via a secondary `find_by_id` on the host. Join the host tenant filter into the initial `update_history` query so data is never fetched for unauthorised callers. |
| **Medium** | `queries/hosts.rs:80-92,182-198` | **Duplicated `ServiceStatus` mapping logic** in `load_host_agents` and `list_hosts`. Extract to a private `fn map_service_status(s: service::ServiceStatus) -> ServiceStatus`. |
| **Medium** | `queries/notifications.rs:74-97` | **`list_channels` builds the base query twice** (once for count, once for data). Other list queries share the base via `.clone()`. Build `base_query` once and clone for the count: `let total = base_query.clone().count(...)`. |
| **Low** | `queries/services.rs:451-508` | **No structured log per item** in batch approve/reject loops. Add `tracing::debug!` per approval and `tracing::warn!` per ineligible item. |

#### Maintainability

| Severity | Location | Finding |
| --- | --- | --- |
| **Medium** | `Cargo.toml` | **Query layer depends on `uptrakit-notification-plugin-registry`** with hardcoded `features = ["webhook"]`, pulling `reqwest` and all notification plugin structs into a crate intended as a pure DB abstraction. Five query functions accept `&dyn NotificationOps`. Define a narrower `NotificationConfigValidator` trait in the query crate or in `notification-plugin-core`; pass the concrete `NotificationOps` from the route-handler layer. |

#### Database — Tenant Isolation

| Severity | Location | Finding |
| --- | --- | --- |
| **Medium** | `queries/host_tags.rs:279-290` | **`load_host_tags_batch` is unscoped**: `host_tag_assignment::Entity::find()` filtered only by `host_id` with no structural tenant isolation on this join table. Currently safe because `host_ids` are pre-scoped by callers, but the precondition is implicit. Add an explicit `#[doc]` comment stating `host_ids` must be pre-scoped to the caller's tenant; add a debug-mode assertion. |

#### Tests

| Severity | Finding |
| --- | --- |
| **Medium** | Missing integration tests for `settings_mqtt.rs` routes (729 LOC, credential storage, password encryption). At minimum: GET returns 200, PUT with valid config returns 200, GET after PUT does not return the plaintext password (returns `has_password: true`). *Adds to prior HIGH test-gap finding.* |
| **Low** | `test_harness/fixtures.rs` — `register_and_get_token` hardcodes email `owner@test.local`. Calling it twice per `TestApp` instance fails on a duplicate email constraint. Accept `email` and `password` parameters, or document the single-use constraint. |
