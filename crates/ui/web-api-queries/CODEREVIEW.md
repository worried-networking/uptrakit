# Code Review: `uptrakit-web-api-queries`

- Review date: 2026-03-17
- Scope: full 14-dimension review of all ~41 source files in the query crate

## Summary

The query crate is functionally strong with good transactional safety for batch dispatch, update
completion, and tenant data reset. CAS-style dispatch patterns are well-implemented and
multi-controller-safe. This review cycle validated all prior findings, confirmed one has been
partially mitigated, added two new findings (Queued status mapping gap, interactive flag
inconsistency in batch path), and retained all prior findings that remain valid.

## Strengths

- Batch completion and queued-update dispatch use CAS-style patterns with partial unique index
  enforcement.
- Tenant-scoped query structure remains clear and consistent despite the crate size.
- `mark_in_progress_as_failed()` correctly uses CAS guards and wraps cleanup in a single
  transaction.
- `reset_tenant_data` correctly deletes in FK-safe order within a single transaction.
- Software states queries (`load_software_states_for_tenant`,
  `load_software_states_page_for_tenant`) use bulk queries with no N+1 patterns.
- `list_update_history` batch-loads output lines for streamed records in a single query instead
  of per-record lookups.
- The `trigger_update_for_host` race-condition fallback (Pending INSERT fails unique constraint,
  re-inserts as Queued) is a correct belt-and-suspenders pattern.
- Candidate queries (`find_outdated_items_for_host`, `find_outdated_hosts_for_item`) batch-load
  all related data (items, hosts, plugin assignments) in three queries instead of per-row lookups.
- Test coverage is strong across the update dispatch pipeline: CAS miss, host-busy queuing,
  precondition validation, and batch sequencing are all exercised.

## Active Findings

### [HIGH] Stale update recovery is still limited to reconnect-triggered cleanup

- **Dimension**: high availability, database
- **Scope**: `crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs`
- **Description**: `mark_in_progress_as_failed()` is correct for reconnect recovery but assumes
  an agent reconnect will happen at some point. There is no scheduler-driven or age-based cleanup
  for updates orphaned by broader multi-component failures.
- **Why it matters**: updates stranded in `InProgress` are never recovered if the agent never
  reconnects (dead host, permanent network partition, controller crash during dispatch).
- **Failure scenario**: a host crashes permanently while an update is `InProgress`. The
  update_history record remains `InProgress` forever. Any queued updates for that host are
  never promoted. The software states MQTT feed shows `update_in_progress: true` indefinitely.

### [HIGH] TOCTOU race in `find_or_create_software_item` collision recovery

- **Dimension**: database, correctness
- **Scope**: `crates/ui/web-api-queries/src/queries/autodiscovery/discovery_items.rs:find_or_create_software_item` (line ~402-422)
- **Description**: the collision recovery after a UNIQUE constraint violation in Phase 3 falls
  back to `(tenant_id, name)` lookup without verifying that the colliding row belongs to the
  same plugin config scope. Two concurrent autodiscovery targets with the same software name
  but different plugin configs can end up sharing the wrong `software_item` row.
- **Why it matters**: the collision fallback at line 410-422 queries only `(tenant_id, name,
  DeactivatedAt IS NULL)`. If two concurrent discovery runs for different plugin configs both
  attempt Phase 3 for the same software name, the losing insert's fallback returns the winner's
  item. Subsequent host_software_item and plugin assignment rows then point to a software_item
  that is associated with the wrong plugin config scope.
- **Failure scenario**: two concurrent autodiscovery runs for different plugin configs (e.g.
  GitHub and Docker both discovering a package named "nginx") race through Phase 3. The losing
  insert's fallback returns the winner's item, causing incorrect plugin config routing for
  all subsequent operations on that software item.

### [MEDIUM] `db_status_to_api` maps `Queued` to `Pending` with a warning log

- **Dimension**: correctness, consistency
- **Scope**: `crates/ui/web-api-queries/src/queries/update_history.rs:db_status_to_api` (line 17-28)
- **Description**: the API-level `UpdateStatus` enum in `web-api-types` has four variants
  (`Pending`, `InProgress`, `Completed`, `Failed`) but no `Queued` variant. The `db_status_to_api`
  mapping function sends `Queued` records through the wildcard `_ =>` branch, which logs a warning
  and returns `Pending`. This means the update history list and detail endpoints misrepresent
  `Queued` records as `Pending`.
- **Why it matters**: users see `Pending` for updates that are actually `Queued` (waiting for the
  host to become free). This is confusing because `Pending` implies the update is about to be
  dispatched, while `Queued` means it is blocked behind another active update. The warning log
  fires on every `Queued` record load, generating noise in production logs for a normal state.
- **Failure scenario**: a host with 5 queued updates shows all 5 as "Pending" in the UI. The
  user cannot distinguish which update is actively waiting for dispatch vs. which are blocked.

### [MEDIUM] N+1 sequential plugin role queries in `load_target_for_dispatch`

- **Dimension**: database, performance
- **Scope**: `crates/ui/web-api-queries/src/queries/update_dispatch.rs:load_target_for_dispatch` (line 275-365)
- **Description**: 8-9 sequential DB round-trips per update trigger: one each for software_item,
  host, host_software_item, service_host, service, and then 3 separate `load_role_plugin` calls
  (execute_update, detect_version, fetch_releases) plus 2 `load_role_plugins_ordered` calls
  (pre_update_hook, post_update_hook). Each `load_role_plugin` performs 1-2 queries (assignment
  lookup + optional config lookup).
- **Why it matters**: under burst load (e.g. a batch update dispatching 50 items), this
  multiplies dispatch latency linearly. Each dispatch call runs 8-12 sequential queries.
- **Failure scenario**: a "host update all" batch for 50 outdated items triggers 50 calls to
  `validate_update_preconditions`, each performing ~10 sequential queries. Total: ~500
  sequential DB round-trips during the validation phase alone.

### [MEDIUM] Batch path always sets `interactive: false` without checking `config_prefers_interactive`

- **Dimension**: correctness, consistency
- **Scope**: `crates/ui/web-api-queries/src/queries/update_batches/mod.rs:create_batch` (line 198)
- **Description**: the batch creation path hardcodes `interactive: false` in the
  `CreateUpdateRecordParams` for all batch items. The single-update path in
  `trigger_update_for_host` correctly resolves `interactive` by checking
  `config_prefers_interactive()` before creating the record. The batch path skips this check.
- **Why it matters**: Proxmox Helper Scripts targets set `prefer_interactive: true` so the agent
  allocates a PTY for `/usr/bin/update`. Batch updates for PHS targets will dispatch without
  `interactive: true`, potentially causing the PHS update script to fail because `/dev/tty` is
  not available.
- **Failure scenario**: a "host update all" batch on a Proxmox host with 3 PHS-discovered
  packages dispatches all 3 with `interactive: false`. The PHS update scripts fail because
  they cannot read from `/dev/tty` without a PTY.

### [MEDIUM] `UpdateSoftwareItemRequest` is not validated before reaching the query layer

- **Dimension**: coding standards, consistency
- **Scope**: `crates/ui/web-api/src/routes/software_items/mod.rs:update_software_item`
- **Description**: the handler uses plain `Json(req)` extraction instead of
  `Validated(req): Validated<UpdateSoftwareItemRequest>`. If the type implements `Validate`
  (which it does), schema constraints are silently skipped.
- **Why it matters**: invalid input may produce unintelligible database errors instead of a
  clear 400 response.

### [MEDIUM] `create_plugin_config` validates; `update_plugin_config` does not

- **Dimension**: coding standards, consistency
- **Scope**: `crates/ui/web-api/src/routes/plugin_configs.rs:create_plugin_config` vs.
  `update_plugin_config`
- **Description**: create uses `Validated(req)` extractor; update uses raw `Json(req)` and only
  checks dangerous patterns, not schema constraints. Schema invariants that apply at creation
  are silently bypassed on update.

### [MEDIUM] Type-complexity suppressions still mark query-shaping hot spots

- **Dimension**: coding standards, maintainability
- **Scope**: `crates/ui/web-api-queries/src/queries/services.rs`, `host_tags.rs`,
  `plugin_configs.rs`, `hosts.rs`, `software_items/crud.rs`, `system_services.rs`,
  `autodiscovery/ignore_rules.rs`
- **Description**: the remaining `#[allow(clippy::type_complexity)]` markers are good signals
  for future refactoring targets in tuple-heavy query assembly paths.
- **Why it matters**: a future schema or response-shape change requires touching one of these
  tuple-heavy paths and increases the chance of accidental field-order or nullability mistakes.

### [LOW] Inconsistent permission extractor placement across similar route handlers

- **Dimension**: consistency
- **Scope**: `crates/ui/web-api/src/routes/plugin_configs.rs` vs.
  `crates/ui/web-api/src/routes/software_items/mod.rs` vs.
  `crates/ui/web-api/src/routes/services.rs`
- **Description**: permission extractors appear at different positions in handler signatures
  (sometimes after `Path`, sometimes before). Inconsistent ordering makes it harder to spot
  handlers missing a permission check during code review.

### [LOW] `load_output_lines` in single-record path is not bounded by page size

- **Dimension**: performance, resource management
- **Scope**: `crates/ui/web-api-queries/src/queries/update_history.rs:load_output_lines` (line 64-90)
- **Description**: the `load_output_lines` function loads all output lines for an update
  history record and concatenates them in memory up to `UPDATE_OUTPUT_BYTES_CAP` (50 MB).
  However, it loads ALL rows from the database before applying the byte cap in application code.
  The `list_update_history` path has a similar pattern but at least batches across records.
- **Why it matters**: for a single update with millions of output lines (e.g. a verbose package
  manager), the database transfers all rows even though only 50 MB worth are used. The query
  lacks a `LIMIT` clause to bound the DB-side transfer.
