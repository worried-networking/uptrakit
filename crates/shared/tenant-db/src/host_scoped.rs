use sea_orm::EntityTrait;

/// Declares how an entity's rows resolve to the visibility axes.
///
/// `host_id_column` is mandatory: every visibility-queryable entity is
/// host-addressable (the `host` entity maps it to its own `Id`). The two
/// item-axis columns default to `None`; an axis whose column is `None`
/// contributes no condition — fail-closed, mirroring `Selector::covers()`.
///
/// An axis column may be the entity's **own primary key** when the entity
/// *is* the axis object: `host` answers `host_id_column` with `Id`, and
/// `host_software_item` answers `host_software_item_id_column` with `Id`.
/// Wire each axis method to whichever column holds that axis's id — own PK
/// or FK — and never to a similarly named column that holds something else.
pub trait HostScoped: EntityTrait {
    /// Column holding the host-axis id for this entity's rows.
    fn host_id_column() -> Self::Column;

    /// Column holding the software-item-axis id, if this entity declares
    /// that axis. `None` (the default) means the axis contributes nothing.
    fn software_item_id_column() -> Option<Self::Column> {
        None
    }

    /// Column holding the host-software-item-axis id, if this entity
    /// declares that axis. `None` (the default) means the axis contributes
    /// nothing.
    fn host_software_item_id_column() -> Option<Self::Column> {
        None
    }
}
