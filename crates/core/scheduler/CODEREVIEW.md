# Code Review: `uptrakit-scheduler`

- Review date: 2026-03-17
- Scope: current-state review

## Summary

The external scheduler remains structurally clean: it delegates task semantics to `uptrakit-scheduler-engine`, uses optimistic claim handling, and compiles and tests cleanly. Its main live concern is inherited from the scheduler engine's task set, and it carries duplicated initialization code from the controller.

## Strengths

- Clear separation between service lifecycle, NATS/database credential handling, and task execution.
- Optimistic claim handling and stale lease recovery remain the right basis for HA scheduler behavior.
- The crate itself is small and easier to reason about than the controller and agent runtimes.
- Drain-vs-abort shutdown distinction (graceful for ServerRestarting/SIGHUP, hard for SIGTERM/SIGINT) is correct.

## Active Findings

### [HIGH] The scheduled executor set still has no generic stale-update cleanup task

- Dimension: high availability, database
- Scope: external scheduler behavior over `update_history`
- Why it matters: the external scheduler is the natural place to age out orphaned `InProgress` rows, but the task set (AuthCleanup, StaleLeaseCleanup, FetchReleases, DetectVersion, DiscoverSoftware) still only handles stale scheduler leases, not stale updates.
- Failure scenario: controller, agent, DB, or network failure leaves an update stuck `InProgress`. The external scheduler keeps running but never clears the host lock.

### [INFO] `register_column_aad_mappings` and `init_data_key_ring` are duplicated from the controller

- Dimension: maintainability, security
- Scope: `crates/core/scheduler/src/handler.rs`
- Why it matters: the scheduler maintains its own copy of the AAD column registration and DEK ring initialization logic, mirroring what the controller does in `main.rs`, `reencrypt.rs`, and `pki.rs`. If a new encrypted column is added, this site must be updated in lockstep with the controller's copy.
- Recommendation: extract a shared `register_all_aad_mappings()` function into a common crate to eliminate the duplication risk.
