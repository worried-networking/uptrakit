# Code Review: `uptrakit-web-api-queries`

- Review date: 2026-03-17
- Scope: current-state review

## Summary

The query crate is functionally strong and now carries more transactional safety than it used to. Its active problems are concentrated in stale-update recovery and a still-large amount of query-shaping complexity.

## Strengths

- Batch completion and queued-update dispatch use stronger transactional and CAS-style patterns than older review snapshots.
- Tenant-scoped query structure remains clear despite the crate size.
- The current test sweep covers a large amount of query behavior directly in this crate.

## Active Findings

### [HIGH] Stale update recovery is still limited to reconnect-triggered cleanup

- Dimension: high availability, database
- Scope: `crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs`
- Why it matters: `mark_in_progress_as_failed()` is correct for reconnect recovery, but it still assumes an agent reconnect will happen at all.
- Failure scenario: agent crash, dead host, controller crash, DB outage, or permanent network partition leaves an update `InProgress` forever because no scheduled cleanup claims ownership of the problem.

### [MEDIUM] Type-complexity suppressions still mark query-shaping hot spots

- Dimension: coding standards, maintainability
- Scope: `crates/ui/web-api-queries/src/queries/services.rs`, `host_tags.rs`, `plugin_configs.rs`, `hosts.rs`, `software_items/crud.rs`, `system_services.rs`, `autodiscovery/ignore_rules.rs`
- Why it matters: the crate is doing a lot of tuple-heavy query assembly, and the remaining `#[allow(clippy::type_complexity)]` markers are a good signal for future refactoring targets.
- Failure scenario: a future schema or response-shape change requires touching one of these tuple-heavy paths and increases the chance of accidental field-order or nullability mistakes.
