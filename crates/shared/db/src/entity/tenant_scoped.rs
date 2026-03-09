use sea_orm::EntityTrait;

use super::{
    audit_log, enrollment_token, host, host_discovery_allowlist, host_package, host_package_ignore,
    host_package_update_history, host_tag, mqtt_client, notification_channel, notification_log,
    notification_rule, oidc_provider, plugin_config, proxmox_host_mapping, scheduled_task, service,
    setting, settings_version, software_item, tenant_discovery_allowlist, update_batch, user_role,
};

/// Marker trait for SeaORM entities that are scoped to a tenant via a `tenant_id` column.
///
/// Implementing this trait allows `TenantDb` (in the web-api crate) to automatically
/// apply tenant-scoping filters in query helpers, eliminating repeated
/// `.filter(Column::TenantId.eq(...))` call sites throughout the codebase.
pub trait TenantScoped: EntityTrait {
    fn tenant_id_column() -> Self::Column;
}

impl TenantScoped for audit_log::Entity {
    fn tenant_id_column() -> Self::Column {
        audit_log::Column::TenantId
    }
}

impl TenantScoped for host::Entity {
    fn tenant_id_column() -> Self::Column {
        host::Column::TenantId
    }
}

impl TenantScoped for service::Entity {
    fn tenant_id_column() -> Self::Column {
        service::Column::TenantId
    }
}

impl TenantScoped for oidc_provider::Entity {
    fn tenant_id_column() -> Self::Column {
        oidc_provider::Column::TenantId
    }
}

impl TenantScoped for scheduled_task::Entity {
    fn tenant_id_column() -> Self::Column {
        scheduled_task::Column::TenantId
    }
}

impl TenantScoped for software_item::Entity {
    fn tenant_id_column() -> Self::Column {
        software_item::Column::TenantId
    }
}

impl TenantScoped for plugin_config::Entity {
    fn tenant_id_column() -> Self::Column {
        plugin_config::Column::TenantId
    }
}

impl TenantScoped for mqtt_client::Entity {
    fn tenant_id_column() -> Self::Column {
        mqtt_client::Column::TenantId
    }
}

impl TenantScoped for setting::Entity {
    fn tenant_id_column() -> Self::Column {
        setting::Column::TenantId
    }
}

impl TenantScoped for settings_version::Entity {
    fn tenant_id_column() -> Self::Column {
        settings_version::Column::TenantId
    }
}

impl TenantScoped for user_role::Entity {
    fn tenant_id_column() -> Self::Column {
        user_role::Column::TenantId
    }
}

impl TenantScoped for enrollment_token::Entity {
    fn tenant_id_column() -> Self::Column {
        enrollment_token::Column::TenantId
    }
}

impl TenantScoped for tenant_discovery_allowlist::Entity {
    fn tenant_id_column() -> Self::Column {
        tenant_discovery_allowlist::Column::TenantId
    }
}

impl TenantScoped for host_discovery_allowlist::Entity {
    fn tenant_id_column() -> Self::Column {
        host_discovery_allowlist::Column::TenantId
    }
}

impl TenantScoped for notification_channel::Entity {
    fn tenant_id_column() -> Self::Column {
        notification_channel::Column::TenantId
    }
}

impl TenantScoped for notification_rule::Entity {
    fn tenant_id_column() -> Self::Column {
        notification_rule::Column::TenantId
    }
}

impl TenantScoped for notification_log::Entity {
    fn tenant_id_column() -> Self::Column {
        notification_log::Column::TenantId
    }
}

impl TenantScoped for update_batch::Entity {
    fn tenant_id_column() -> Self::Column {
        update_batch::Column::TenantId
    }
}

impl TenantScoped for host_package::Entity {
    fn tenant_id_column() -> Self::Column {
        host_package::Column::TenantId
    }
}

impl TenantScoped for host_package_ignore::Entity {
    fn tenant_id_column() -> Self::Column {
        host_package_ignore::Column::TenantId
    }
}

impl TenantScoped for host_package_update_history::Entity {
    fn tenant_id_column() -> Self::Column {
        host_package_update_history::Column::TenantId
    }
}

impl TenantScoped for host_tag::Entity {
    fn tenant_id_column() -> Self::Column {
        host_tag::Column::TenantId
    }
}

impl TenantScoped for proxmox_host_mapping::Entity {
    fn tenant_id_column() -> Self::Column {
        proxmox_host_mapping::Column::TenantId
    }
}
