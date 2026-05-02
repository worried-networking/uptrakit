//! Database query helpers for all API domains.
//!
//! Each sub-module encapsulates SeaORM entity access and returns typed API
//! response types, so that route handlers deal only with HTTP concerns.

use uuid::Uuid;

/// Outcome of a batch operation: `(succeeded_ids, failed_with_reason)`.
///
/// `failed_with_reason` pairs the failing entity ID with a human-readable error message.
pub type BatchOutcome = (Vec<Uuid>, Vec<(Uuid, String)>);

pub mod audit_logs;
pub mod autodiscovery;
pub mod discovery_allowlist;
pub mod embedded_runtime_states;
pub mod enrollment_tokens;
pub mod host_tags;
pub mod hosts;
pub mod notifications;
pub mod plugin_configs;
pub mod plugin_type_settings;
pub mod reset_data;
pub mod scheduled_tasks;
pub mod service_config;
pub mod services;
pub mod software_items;
pub mod software_states;
pub mod system_enrollment_tokens;
pub mod system_services;
pub mod update_batches;
pub mod update_dispatch;
pub mod update_history;
pub mod update_tracking_states;
pub mod update_triggers;
pub mod update_types;
pub mod users;
