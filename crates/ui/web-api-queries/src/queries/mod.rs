//! Database query helpers for all API domains.
//!
//! Each sub-module encapsulates SeaORM entity access and returns typed API
//! response types, so that route handlers deal only with HTTP concerns.

pub mod audit_logs;
pub mod autodiscovery;
pub mod discovery_allowlist;
pub mod enrollment_tokens;
pub mod host_tags;
pub mod hosts;
pub mod mqtt_software_states;
pub mod notifications;
pub mod plugin_configs;
pub mod scheduled_tasks;
pub mod services;
pub mod software_items;
pub mod system_enrollment_tokens;
pub mod system_services;
pub mod update_batches;
pub mod update_history;
pub mod update_triggers;
pub mod update_types;
