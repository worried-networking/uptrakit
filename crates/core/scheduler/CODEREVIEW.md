# Code Review: `uptrakit-scheduler`

- Review date: 2026-03-17
- Scope: current-state review

## Summary

The external scheduler remains structurally clean: it delegates task semantics to `uptrakit-scheduler-engine`, uses optimistic claim handling, and compiles and tests cleanly. Its main live concern is inherited from the scheduler engine's task set.

## Strengths

- Clear separation between service lifecycle, NATS/database credential handling, and task execution.
- Optimistic claim handling and stale lease recovery remain the right basis for HA scheduler behavior.
- The crate itself is small and easier to reason about than the controller and agent runtimes.

## Active Findings

### [HIGH] The scheduled executor set still has no generic stale-update cleanup task

- Dimension: high availability, database
- Scope: external scheduler behavior over `update_history`
- Why it matters: the external scheduler is the natural place to age out orphaned `InProgress` rows, but the task set still only handles stale scheduler leases, not stale updates.
- Failure scenario: controller, agent, DB, or network failure leaves an update stuck `InProgress`. The external scheduler keeps running but never clears the host lock.
