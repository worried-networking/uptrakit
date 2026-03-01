use sea_orm_migration::prelude::*;

/// Create a dedicated `global_settings` table and migrate global setting keys
/// out of the per-tenant `settings` table.
///
/// Global settings (network, PKI, MQTT limit, JWT key, master key verification,
/// multi-tenancy flag) were previously stored under the default tenant's row in
/// the `settings` table. This migration:
///
/// 1. Creates a new `global_settings` table keyed solely by `key` (no tenant_id).
/// 2. Copies the 13 global setting keys from the default tenant's `settings`
///    rows into `global_settings`.
/// 3. Deletes those keys from the `settings` table for ALL tenants.
#[derive(DeriveMigrationName)]
pub struct Migration;

/// The 13 global setting keys that move to the new table.
const GLOBAL_KEYS: &[&str] = &[
    "network.trusted_proxies",
    "network.real_ip_header",
    "network.extra_sans",
    "network.https_addr",
    "network.forwarded_client_cert_info_header",
    "network.forwarded_client_cert_pem_header",
    "network.pki_addr",
    "pki.active_ca_fingerprint",
    "pki.ca_version",
    "multi_tenancy.enabled",
    "mqtt.max_clients_per_tenant",
    "auth.jwt_signing_key",
    "crypto.master_key_verification",
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // 1. Create global_settings table
        manager
            .create_table(
                Table::create()
                    .table(GlobalSettings::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(GlobalSettings::Key).string().not_null())
                    .col(ColumnDef::new(GlobalSettings::Value).json().not_null())
                    .col(
                        ColumnDef::new(GlobalSettings::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .primary_key(Index::create().col(GlobalSettings::Key))
                    .to_owned(),
            )
            .await?;

        // 2. Copy global keys from the default tenant's settings rows
        let keys_list = GLOBAL_KEYS
            .iter()
            .map(|k| format!("'{k}'"))
            .collect::<Vec<_>>()
            .join(", ");

        let db = manager.get_connection();
        db.execute_unprepared(&format!(
            "INSERT INTO global_settings (key, value, updated_at) \
             SELECT key, value, updated_at FROM settings \
             WHERE tenant_id = (SELECT id FROM tenants WHERE is_default = 1) \
             AND key IN ({keys_list})"
        ))
        .await?;

        // 3. Delete global keys from settings for ALL tenants
        db.execute_unprepared(&format!(
            "DELETE FROM settings WHERE key IN ({keys_list})"
        ))
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let keys_list = GLOBAL_KEYS
            .iter()
            .map(|k| format!("'{k}'"))
            .collect::<Vec<_>>()
            .join(", ");

        // Move global settings back to the default tenant's settings rows
        db.execute_unprepared(&format!(
            "INSERT INTO settings (tenant_id, key, value, updated_at) \
             SELECT (SELECT id FROM tenants WHERE is_default = 1), key, value, updated_at \
             FROM global_settings \
             WHERE key IN ({keys_list})"
        ))
        .await?;

        manager
            .drop_table(Table::drop().table(GlobalSettings::Table).to_owned())
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum GlobalSettings {
    Table,
    Key,
    Value,
    UpdatedAt,
}
