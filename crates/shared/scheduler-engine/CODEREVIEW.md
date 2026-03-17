# Code Review: `uptrakit-scheduler-engine`

- Review date: 2026-03-17
- Scope: current-state review

## Summary

The scheduler engine remains a strong HA foundation for scheduled task execution. Claiming, stale-lease recovery, and cancellation handling are all good. The active problem is that the executor set still does not cover stale update cleanup.

## Strengths

- Optimistic task claiming and stale-lease recovery remain well designed for multi-controller or external-scheduler setups.
- Shutdown semantics are clear: drain versus abort are distinct and implemented explicitly.
- Current unit coverage for claim logic and scheduling policy is strong.

## Active Findings

### [HIGH] The engine still has no executor for orphaned update cleanup

- Dimension: high availability, database
- Scope: `crates/shared/scheduler-engine/src/executors/mod.rs`
- Why it matters: the engine already owns stale scheduler-lease cleanup, so it is the natural place to recover stale update state too. That executor still does not exist.
- Failure scenario: any update is stranded in `InProgress` by an agent/controller/network failure and the responsible agent never reconnects. Scheduled maintenance keeps running, but host update progress never unlocks.
