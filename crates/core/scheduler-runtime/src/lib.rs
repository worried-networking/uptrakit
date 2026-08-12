pub mod ca_utils;
pub mod claim;
pub mod error;
pub mod executor;
pub mod executors;
pub mod interval;
pub mod notifier;
pub mod scheduler;
#[cfg(feature = "test-support")]
pub mod test_support;
pub mod tick_executor;

mod runtime;
#[cfg(feature = "standalone")]
mod standalone;

pub use error::{Result, SchedulerError};
pub use executor::TaskExecutor;
pub use notifier::SchedulerNotifier;
pub use scheduler::{HEARTBEAT_INTERVAL, Scheduler, SchedulerConfig, TASK_EXECUTION_TIMEOUT};
pub use tick_executor::TickExecutor;

pub use runtime::{ManagedSchedulerRuntime, SchedulerRunConfig, SchedulerStopMode, run_scheduler};
#[cfg(feature = "standalone")]
pub use standalone::{
    STANDALONE_SCHEDULER_APP_NAME, STANDALONE_SCHEDULER_DIR_NAME, STANDALONE_SCHEDULER_LABEL,
    SchedulerHandler, standalone_scheduler_capabilities,
};
