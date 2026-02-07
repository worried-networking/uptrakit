use crate::db::{DbError, Result};
use rootcause::prelude::*;
use sea_orm::DatabaseConnection;
use sea_orm_migration::prelude::*;

mod m20260129_000001_initial;
mod m20260129_000002_create_users;
mod m20260129_000003_create_rbac;
mod m20260129_000004_create_oidc;
mod m20260129_000005_create_sessions;
mod m20260129_000006_create_settings;
mod m20260129_000007_create_agents;
mod m20260129_000008_create_agent_certificates;
mod m20260130_000009_jwt_refresh_tokens;
mod m20260131_000010_create_api_tokens;
mod m20260131_000011_update_rbac_permissions;
mod m20260131_000012_create_pending_auth_stores;
mod m20260201_000013_create_hosts;
mod m20260201_000014_create_provider_configs;
mod m20260201_000015_create_software_items;
mod m20260202_000016_create_mqtt_clients;
mod m20260202_000017_create_mqtt_leases;
mod m20260203_000018_create_update_history;
mod m20260203_000019_create_mqtt_services;
mod m20260205_000020_create_pending_oidc_registrations;
mod m20260207_000021_create_api_rate_limits;
mod m20260207_000022_create_settings_version;
mod m20260207_000023_add_revocation_version;
mod m20260207_000024_create_controller_events;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260129_000001_initial::Migration),
            Box::new(m20260129_000002_create_users::Migration),
            Box::new(m20260129_000003_create_rbac::Migration),
            Box::new(m20260129_000004_create_oidc::Migration),
            Box::new(m20260129_000005_create_sessions::Migration),
            Box::new(m20260129_000006_create_settings::Migration),
            Box::new(m20260129_000007_create_agents::Migration),
            Box::new(m20260129_000008_create_agent_certificates::Migration),
            Box::new(m20260130_000009_jwt_refresh_tokens::Migration),
            Box::new(m20260131_000010_create_api_tokens::Migration),
            Box::new(m20260131_000011_update_rbac_permissions::Migration),
            Box::new(m20260131_000012_create_pending_auth_stores::Migration),
            Box::new(m20260201_000013_create_hosts::Migration),
            Box::new(m20260201_000014_create_provider_configs::Migration),
            Box::new(m20260201_000015_create_software_items::Migration),
            Box::new(m20260202_000016_create_mqtt_clients::Migration),
            Box::new(m20260202_000017_create_mqtt_leases::Migration),
            Box::new(m20260203_000018_create_update_history::Migration),
            Box::new(m20260203_000019_create_mqtt_services::Migration),
            Box::new(m20260205_000020_create_pending_oidc_registrations::Migration),
            Box::new(m20260207_000021_create_api_rate_limits::Migration),
            Box::new(m20260207_000022_create_settings_version::Migration),
            Box::new(m20260207_000023_add_revocation_version::Migration),
            Box::new(m20260207_000024_create_controller_events::Migration),
        ]
    }
}

/// Run all pending migrations
pub async fn run_migrations(db: &DatabaseConnection) -> Result<()> {
    Migrator::up(db, None)
        .await
        .context_to::<DbError>()
        .context(DbError::Migration(
            "running database migrations".to_string(),
        ))
}
