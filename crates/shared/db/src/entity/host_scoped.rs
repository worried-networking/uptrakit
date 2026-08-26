//! [`HostScoped`] implementations for the M2.2 visibility-queryable entities.
//!
//! The set is the entities behind the list/single-get/batch endpoints M2.3
//! converts; stragglers are added in M2.3 when their call sites convert (the
//! `HostScoped` bound on the visible query methods forces it at compile time).

use uptrakit_tenant_db::HostScoped;

use super::{host, host_software_item, host_software_item_plugin, update_history};

impl HostScoped for host::Entity {
    fn host_id_column() -> Self::Column {
        // The entity *is* the axis object: host axis = own PK.
        host::Column::Id
    }
}

impl HostScoped for host_software_item::Entity {
    fn host_id_column() -> Self::Column {
        host_software_item::Column::HostId
    }

    fn software_item_id_column() -> Option<Self::Column> {
        Some(host_software_item::Column::SoftwareItemId)
    }

    fn host_software_item_id_column() -> Option<Self::Column> {
        // The entity *is* the axis object: items axis = own PK.
        Some(host_software_item::Column::Id)
    }
}

impl HostScoped for host_software_item_plugin::Entity {
    // The software/items axes are deliberately undeclared even though the
    // columns exist: no designed consumer scopes those axes to this entity.
    // Declaring them needs the NULL-parity design tracked in
    // `uptrakit-def-host-scoped-stragglers` — never a mechanical addition.
    fn host_id_column() -> Self::Column {
        host_software_item_plugin::Column::HostId
    }
}

impl HostScoped for update_history::Entity {
    // The software/items axes are deliberately undeclared: legacy rows carry
    // NULL `host_software_item_id`, whose only constructible target is
    // `TargetRef::Host` — matching on `software_item_id` would allow rows
    // `Selector::covers()` denies (permissive parity break). Declaring them
    // needs the NULL-parity design tracked in
    // `uptrakit-def-host-scoped-stragglers` — never a mechanical addition.
    fn host_id_column() -> Self::Column {
        update_history::Column::HostId
    }
}

#[cfg(test)]
mod tests {
    use uptrakit_tenant_db::HostScoped;

    use super::super::{host, host_software_item, host_software_item_plugin, update_history};

    /// Pins the spec's axis table, including the deliberate `None`s on
    /// `update_history` / `host_software_item_plugin` item axes.
    #[test]
    fn axis_columns_match_spec_table() {
        assert!(matches!(host::Entity::host_id_column(), host::Column::Id));
        assert!(host::Entity::software_item_id_column().is_none());
        assert!(host::Entity::host_software_item_id_column().is_none());

        assert!(matches!(
            host_software_item::Entity::host_id_column(),
            host_software_item::Column::HostId
        ));
        assert!(matches!(
            host_software_item::Entity::software_item_id_column(),
            Some(host_software_item::Column::SoftwareItemId)
        ));
        assert!(matches!(
            host_software_item::Entity::host_software_item_id_column(),
            Some(host_software_item::Column::Id)
        ));

        assert!(matches!(
            host_software_item_plugin::Entity::host_id_column(),
            host_software_item_plugin::Column::HostId
        ));
        assert!(host_software_item_plugin::Entity::software_item_id_column().is_none());
        assert!(host_software_item_plugin::Entity::host_software_item_id_column().is_none());

        assert!(matches!(
            update_history::Entity::host_id_column(),
            update_history::Column::HostId
        ));
        assert!(update_history::Entity::software_item_id_column().is_none());
        assert!(update_history::Entity::host_software_item_id_column().is_none());
    }
}
