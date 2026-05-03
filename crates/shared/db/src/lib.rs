pub mod db_error;
pub mod entity;
#[cfg(feature = "migration")]
pub mod migration;
pub mod provider_settings;
pub mod raw_settings;
pub use uptrakit_tenant_db::{TenantDb, TenantScoped};

pub use db_error::is_unique_constraint_violation;
