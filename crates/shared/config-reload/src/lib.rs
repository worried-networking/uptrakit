//! Graceful-reload runtime for the uptrakit Controller.
//!
//! See `docs/superpowers/specs/2026-05-12-graceful-reload-design.md`.

pub mod alerts;
pub mod audit;
pub mod channels;
pub mod config;
pub mod coordinator;
pub mod defaults;
pub mod delta;
pub mod error;
pub mod loader;
pub mod reconciler;
pub mod reloadable;
pub mod triggers;

pub use alerts::{AlertSeverity, NoopAlertWriter, SystemAlertWriter};
pub use audit::ReloadAuditEvent;
pub use channels::{RuntimeConfigChannels, RuntimeConfigReceivers};
pub use config::{RuntimeConfig, Scope};
pub use coordinator::{
    CoordinatorState, DegradedInfo, ReloadCoordinator, ReloadCoordinatorHandle, ReloadPhase,
    ReloadRequest, ReloadSource,
};
pub use delta::RuntimeConfigDelta;
pub use error::ConfigReloadError;
pub use loader::{LoadedConfig, TomlConfigLoader};
pub use reconciler::SettingsVersionCache;
pub use reloadable::{Reloadable, ReloadableErased};
