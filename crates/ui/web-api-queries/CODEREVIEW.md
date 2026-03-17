# Code Review: `uptrakit-web-api-queries`

- Review date: 2026-03-17
- Scope: current-state review

## Summary

The query crate is functionally strong and carries good transactional safety for batch dispatch and
update completion. This review cycle added three new findings: a TOCTOU race in the discovery
upsert pipeline, N+1 sequential queries in the update dispatch critical path, and two validation
consistency gaps in the route layer that calls into this crate.

## Strengths

- Batch completion and queued-update dispatch use CAS-style patterns and transactions.
- Tenant-scoped query structure remains clear despite the crate size.
- The current test sweep covers a large amount of query behavior directly in this crate.
- `mark_in_progress_as_failed()` correctly clears orphaned InProgress rows on agent reconnect
  inside a transaction.

## Active Findings

### [HIGH] Stale update recovery is still limited to reconnect-triggered cleanup

- Dimension: high availability, database
- Scope: `crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs`
- Why it matters: `mark_in_progress_as_failed()` is correct for reconnect recovery, but it assumes
  an agent reconnect will happen at some point. There is no scheduler-driven or age-based cleanup
  for updates orphaned by broader multi-component failures.
- Failure scenario: agent crash, dead host, controller crash, DB outage, or permanent network
  partition leaves an update `InProgress` forever because no scheduled cleanup claims ownership of
  the problem.

### [HIGH] TOCTOU race in `find_or_create_software_item` with wrong-item fallback

- Dimension: database, correctness
- Scope:
  `crates/ui/web-api-queries/src/queries/autodiscovery/discovery_items.rs:find_or_create_software_item`
- Why it matters: the collision recovery after a UNIQUE constraint violation in Phase 3 falls back
  to `(tenant_id, name)` lookup without verifying that the colliding row belongs to the same plugin
  config scope. Two concurrent autodiscovery targets with the same software name can end up sharing
  the wrong `software_item` row.
- Failure scenario: two concurrent autodiscovery runs for different plugin configs both insert for
  the same software name; the losing insert's fallback returns the winner's item, causing incorrect
  plugin config routing.
- Fix: after the collision recovery lookup, verify the returned item is linked to a compatible
  plugin config scope before returning it; otherwise create a distinct item with a disambiguated
  name or return an explicit conflict error.

### [MEDIUM] N+1 sequential plugin role queries in `load_target_for_dispatch`

- Dimension: database, performance
- Scope: `crates/ui/web-api-queries/src/queries/update_dispatch.rs:load_target_for_dispatch`
- Why it matters: 8–9 sequential DB round-trips per update trigger: one each for software_item,
  host, host_software_item, service_host, service, and then 3–5 separate plugin role queries. Under
  burst load this multiplies dispatch latency linearly.
- Fix: combine all plugin role lookups into a single `Role IN (...)` query and filter by role in
  application code, reducing round-trips from ~8 to ~5.

### [MEDIUM] `UpdateSoftwareItemRequest` is not validated before reaching the query layer

- Dimension: coding standards, consistency
- Scope: `crates/ui/web-api/src/routes/software_items/mod.rs:update_software_item`
- Why it matters: the handler uses plain `Json(req)` extraction instead of
  `Validated(req): Validated<UpdateSoftwareItemRequest>`. If the type implements `Validate` (which
  it does), schema constraints are silently skipped. Invalid input may produce unintelligible
  database errors instead of a clear 400.
- Fix: change to `Validated(req): Validated<UpdateSoftwareItemRequest>`.

### [MEDIUM] `create_plugin_config` validates; `update_plugin_config` does not

- Dimension: coding standards, consistency
- Scope: `crates/ui/web-api/src/routes/plugin_configs.rs:create_plugin_config` vs.
  `update_plugin_config`
- Why it matters: create uses `Validated(req)` extractor; update uses raw `Json(req)` and only
  checks dangerous patterns, not schema constraints. Schema invariants that apply at creation are
  silently bypassed on update.
- Fix: add `Validated(req): Validated<UpdatePluginConfigRequest>` to the update handler.

### [MEDIUM] Type-complexity suppressions still mark query-shaping hot spots

- Dimension: coding standards, maintainability
- Scope: `crates/ui/web-api-queries/src/queries/services.rs`, `host_tags.rs`,
  `plugin_configs.rs`, `hosts.rs`, `software_items/crud.rs`, `system_services.rs`,
  `autodiscovery/ignore_rules.rs`
- Why it matters: the remaining `#[allow(clippy::type_complexity)]` markers are good signals for
  future refactoring targets in tuple-heavy query assembly paths.
- Failure scenario: a future schema or response-shape change requires touching one of these
  tuple-heavy paths and increases the chance of accidental field-order or nullability mistakes.

### [LOW] Inconsistent permission extractor placement across similar route handlers

- Dimension: consistency
- Scope: `crates/ui/web-api/src/routes/plugin_configs.rs` vs.
  `crates/ui/web-api/src/routes/software_items/mod.rs` vs.
  `crates/ui/web-api/src/routes/services.rs`
- Why it matters: permission extractors appear at different positions in handler signatures
  (sometimes after `Path`, sometimes before). Inconsistent ordering makes it harder to spot
  handlers missing a permission check during code review.
- Fix: standardize the parameter order to `(State, TenantDb, Path, Permission, Json/Query)`
  across all route handlers.
