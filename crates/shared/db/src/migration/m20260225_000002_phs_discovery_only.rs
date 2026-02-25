use sea_orm_migration::prelude::*;

/// Clean up legacy PHS data to align with the discovery-only refactor.
///
/// ### Steps
///
/// 1. **Hard-delete** `host_software_items` rows linked to PHS provider configs.
///    These rows carried `script_url`-based assignments from the old model; the new
///    model will re-discover items with `github_owner`/`github_repo` or `apt_package`
///    metadata set in `extra`.
///
/// 2. **Soft-delete** `software_items` that are now orphaned (pending state, no
///    remaining `host_software_items` link after step 1).
///    Only `pending` items are soft-deleted; user-approved items are left untouched
///    so any approval history is preserved in the audit log.
///
/// 3. **Reset** PHS `provider_configs` to the new empty-object form `{}`.
///    The old configs contained `script_url` and optional `github` fields which no
///    longer exist in `ProxmoxHelperScriptsConfig`.
///
/// ### Down
///
/// Data cannot be recovered; the down migration is a no-op.
///
/// ### Why `execute_unprepared`
///
/// Steps 1 and 2 are conditional bulk operations whose predicates reference two
/// tables at once (a subquery pattern not directly expressible in SeaORM's typed
/// `update_many` / `delete_many` API without loading every row into memory).
/// Step 3 is a JSON field reset with no typed equivalent.  This mirrors the
/// rationale already accepted for the `rate_limit.rs` migration.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        // Step 1: hard-delete host_software_items linked to PHS provider configs.
        conn.execute_unprepared(
            "DELETE FROM host_software_items \
             WHERE provider_config_id IN ( \
               SELECT id FROM provider_configs \
               WHERE provider_type = 'proxmox_helper_scripts' \
             )",
        )
        .await?;

        // Step 2: soft-delete pending software_items now orphaned.
        conn.execute_unprepared(
            "UPDATE software_items \
             SET deactivated_at = CURRENT_TIMESTAMP \
             WHERE discovery_state = 'pending' \
               AND deactivated_at IS NULL \
               AND id NOT IN (SELECT software_item_id FROM host_software_items)",
        )
        .await?;

        // Step 3: reset PHS provider configs to the new empty-object form.
        conn.execute_unprepared(
            "UPDATE provider_configs \
             SET config = '{}' \
             WHERE provider_type = 'proxmox_helper_scripts'",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Data cannot be recovered; intentional no-op.
        Ok(())
    }
}
