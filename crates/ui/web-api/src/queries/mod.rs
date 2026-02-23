//! Database query helpers for all API domains.
//!
//! Each sub-module encapsulates SeaORM entity access and returns typed API
//! response types, so that route handlers deal only with HTTP concerns.

pub mod autodiscovery;
pub mod hosts;
pub mod provider_configs;
pub mod scheduled_tasks;
pub mod services;
pub mod software_items;
pub mod update_history;
