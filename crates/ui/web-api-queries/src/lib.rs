//! Database query helpers extracted from `uptrakit-web-api`.
//!
//! This crate contains all tenant-scoped database query logic, the `TenantDb`
//! struct (without the Axum `FromRequestParts` impl), and the `ServiceNotifier`
//! trait used by update dispatch functions.
//!
//! The crate has no dependency on Axum or any HTTP framework, allowing it to
//! compile in parallel with `uptrakit-web-api-auth`.

pub mod notifier;
pub mod queries;
pub mod settings_version;
pub mod tenant_db;
pub mod token_utils;

pub use notifier::ServiceNotifier;
pub use tenant_db::TenantDb;
