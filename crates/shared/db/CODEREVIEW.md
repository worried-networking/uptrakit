# Code Review: `uptrakit-shared-db`

- Review date: 2026-03-17
- Scope: current-state review

## Summary

The database crate is strong on schema correctness, migration hygiene, and repair tooling. The biggest remaining risk is operational: the schema now enforces per-host active-update locking correctly, but the wider system still lacks a generic recovery path for orphaned `InProgress` rows.

## Strengths

- SQLite table-recreation helpers have explicit crash-recovery semantics instead of relying on brittle ad hoc migration code.
- The migration suite contains targeted repair tests for historically tricky storage problems.
- Newer schema work uses clearer indexes and transactional patterns than the oldest migrations.

## Active Findings

### [HIGH] The schema now depends on operational stale-update cleanup that does not yet exist

- Dimension: database, high availability
- Scope: `update_history` active-update locking plus the surrounding scheduler/query layer
- Why it matters: the partial unique locking pattern is correct, but it assumes the application layer will eventually clear orphaned `InProgress` rows.
- Failure scenario: controller crash, DB failover, host crash, or dead network link interrupts an update after it is marked active. The schema then correctly prevents concurrent work, but nothing clears the stranded lock generically.

### [MEDIUM] Migration history is becoming hard to review safely

- Dimension: maintainability
- Scope: `crates/shared/db/src/migration/m20260209_000001_initial.rs` and several later large migrations
- Why it matters: correctness is still good, but some migration files are now large enough that future schema changes will be difficult to audit and reason about in review.
- Failure scenario: a future cross-backend schema change touches one of the monolithic migration files and accidentally regresses an older repair or index recreation rule.
