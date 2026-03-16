//! Shared scheduler engine for cross-controller task scheduling.
//!
//! This crate provides the core scheduling framework used by both the embedded
//! scheduler (inside the controller binary) and the external scheduler binary.
//!
//! ## Architecture
//!
//! - [`Scheduler`] polls the `scheduled_tasks` table for due tasks, claims them
//!   via optimistic locking (HA-safe), executes the matching [`TaskExecutor`],
//!   and releases the claim with updated metadata.
//! - [`TaskExecutor`] is the trait for individual task implementations.
//! - [`SchedulerNotifier`] abstracts message delivery so the same executors can
//!   run in-process (via `NotificationService`) or out-of-process (via NATS).
//! - [`ca_utils`] provides shared CA certificate rotation checks.

pub mod ca_utils;
pub mod claim;
pub mod error;
pub mod executor;
pub mod executors;
pub mod interval;
pub mod notifier;

pub use error::{Result, SchedulerError};
pub use executor::TaskExecutor;
pub use notifier::SchedulerNotifier;

// Re-export the scheduler struct, config, and constants.
pub mod scheduler;
pub use scheduler::{Scheduler, SchedulerConfig, TASK_EXECUTION_TIMEOUT};
