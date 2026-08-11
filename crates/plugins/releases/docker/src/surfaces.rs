//! Surface action handler dispatch for the Docker plugin.
//!
//! ## Action flow
//!
//! 1. The frontend opens the context menu on a host row that has a Docker
//!    plugin assignment.
//! 2. The user clicks **Switch Tag** — the form pre-loads the current image
//!    reference (without `#container` suffix) via `current-tag`.
//! 3. The user edits the tag and submits — `switch-tag` updates all
//!    `host_software_item_plugin` rows and the `host_software_item` row,
//!    then clears stale version data so the next check reflects the new tag.
//!
//! All Docker-specific logic (`ImageRef` parsing, `#container` suffix handling,
//! `validate_identifier` SSRF guard) lives here and does not leak to callers.
//!
//! No query in this module may be built from the raw connection: these tables
//! carry no `tenant_id` column, so every query must be anchored to both
//! `TenantScoped` parents. That takes two parts per site, and only the first is
//! a framework helper:
//!
//! 1. `ctx.tenant_db().find_via_tenant_join::<Target, software_item::Entity>(…)`
//!    — anchors the `software_item` parent.
//! 2. A hand-written `.join(JoinType::InnerJoin, …Relation::Host.def())` plus
//!    `.filter(host::Column::TenantId.eq(tenant_db.tenant_id()))` — anchors the
//!    `host` parent. `find_via_tenant_join` anchors one parent only, so dropping
//!    this second part still compiles and still passes same-tenant tests while
//!    reopening the cross-tenant path that `host_id`/`software_item_id` pointing
//!    at different tenants exploits.
//!
//! Both parts are required at every site; the mismatched-parent tests in this
//! file (`…_host_a_item_b_…` / `…_host_b_item_a_…`) are what catch a site that
//! keeps one anchor and loses the other. Promoting the pair to a shared
//! `TenantDb` helper is the committed response to the next occurrence of this
//! pattern outside this crate. The raw handle remains only as the executor and
//! for `begin_immediate`.

use std::future::Future;
use std::pin::Pin;
use uptrakit_shared_db::begin_immediate;

use serde::de::DeserializeOwned;

use uptrakit_plugin_infrastructure_core::{SurfaceActionContext, SurfaceActionError};

use crate::image_ref::{ImageRef, validate_identifier};

// ── Docker surface-action request types ─────────────────────────────────────

/// Typed host/software-item request for the `current-tag` surface action.
#[derive(Debug, serde::Deserialize)]
struct DockerItemHostRequest {
    pub host_id: uuid::Uuid,
    pub software_item_id: uuid::Uuid,
}

/// Typed switch-tag request for the `switch-tag` surface action.
#[derive(Debug, serde::Deserialize)]
struct DockerSwitchTagRequest {
    pub host_id: uuid::Uuid,
    pub software_item_id: uuid::Uuid,
    pub new_image_ref: String,
}

// ── String helpers ───────────────────────────────────────────────────────────

/// Docker releases plugin type identifier used as a DB filter value.
const DOCKER_RELEASES_CONFIG_TYPE: &str = "releases.docker";

/// Return the image reference without the `#container_name` suffix.
fn strip_container_suffix(id: &str) -> String {
    id.split_once('#')
        .map(|(base, _)| base)
        .unwrap_or(id)
        .to_string()
}

/// Return the container name from the `#container_name` suffix, if present.
fn extract_container_suffix(id: &str) -> Option<&str> {
    id.split_once('#').map(|(_, suffix)| suffix)
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Dispatch shim for the `switch-tag` interaction (exact-id dispatch map entry).
pub(crate) fn docker_switch_tag_handler<'a>(
    ctx: &'a SurfaceActionContext<'a>,
    params: serde_json::Value,
) -> Pin<
    Box<
        dyn Future<Output = std::result::Result<serde_json::Value, SurfaceActionError>> + Send + 'a,
    >,
> {
    Box::pin(async move {
        let request = parse_action_params::<DockerSwitchTagRequest>(params, "switch-tag")?;
        handle_switch_tag(ctx, request).await
    })
}

/// Dispatch shim for the `current-tag` interaction (exact-id dispatch map entry).
pub(crate) fn docker_get_current_tag_handler<'a>(
    ctx: &'a SurfaceActionContext<'a>,
    params: serde_json::Value,
) -> Pin<
    Box<
        dyn Future<Output = std::result::Result<serde_json::Value, SurfaceActionError>> + Send + 'a,
    >,
> {
    Box::pin(async move {
        let request = parse_action_params::<DockerItemHostRequest>(params, "current-tag")?;
        handle_get_current_tag(ctx, request).await
    })
}

// ── Action handlers ──────────────────────────────────────────────────────────

/// Pre-load handler: return the current image reference (without `#container`
/// suffix) so the Switch Tag form can pre-populate the `new_image_ref` field.
async fn handle_get_current_tag(
    ctx: &SurfaceActionContext<'_>,
    request: DockerItemHostRequest,
) -> std::result::Result<serde_json::Value, SurfaceActionError> {
    use sea_orm::{
        ColumnTrait as _, JoinType, QueryFilter as _, QuerySelect as _, RelationTrait as _,
    };
    use uptrakit_shared_db::entity::{host, host_software_item_plugin, software_item};

    let host_id = request.host_id;
    let software_item_id = request.software_item_id;

    tracing::debug!(%host_id, %software_item_id, "fetching current Docker tag");

    let tenant_db = ctx.tenant_db();
    let plugin_rows = tenant_db
        .find_via_tenant_join::<host_software_item_plugin::Entity, software_item::Entity>(
            host_software_item_plugin::Relation::SoftwareItem.def(),
        )
        .join(
            JoinType::InnerJoin,
            host_software_item_plugin::Relation::Host.def(),
        )
        .filter(host::Column::TenantId.eq(tenant_db.tenant_id()))
        .filter(host_software_item_plugin::Column::HostId.eq(host_id))
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(software_item_id))
        .filter(host_software_item_plugin::Column::PluginType.eq(DOCKER_RELEASES_CONFIG_TYPE))
        .all(tenant_db.db())
        .await
        .map_err(|e| SurfaceActionError::ControllerIntegration(e.to_string()))?;

    let image_ref = plugin_rows
        .into_iter()
        .next()
        .map(|r| strip_container_suffix(&r.package_identifier))
        .unwrap_or_default();

    tracing::debug!(%host_id, %software_item_id, image_ref = %image_ref, "resolved current Docker tag");

    Ok(serde_json::json!({ "new_image_ref": image_ref }))
}

/// Switch tag handler: update all Docker plugin rows for a specific
/// `(host_id, software_item_id)` pair and clear stale version data.
///
/// Preserves the `#container_name` suffix on each plugin row so subsequent
/// update operations still target the correct named container.
async fn handle_switch_tag(
    ctx: &SurfaceActionContext<'_>,
    request: DockerSwitchTagRequest,
) -> std::result::Result<serde_json::Value, SurfaceActionError> {
    use sea_orm::{
        ActiveModelTrait as _, ColumnTrait as _, JoinType, QueryFilter as _, QuerySelect as _,
        RelationTrait as _, Set,
    };
    use uptrakit_shared_db::entity::{
        host, host_software_item, host_software_item_plugin, software_item,
    };

    let host_id = request.host_id;
    let software_item_id = request.software_item_id;
    let new_image_ref = request.new_image_ref.trim().to_string();

    tracing::debug!(%host_id, %software_item_id, new_image_ref = %new_image_ref, "switching Docker tag");

    // Validate format first (parses the image ref) then SSRF check.
    new_image_ref
        .parse::<ImageRef>()
        .map_err(|e| SurfaceActionError::InvalidInput(format!("invalid image reference: {e}")))?;
    validate_identifier(&new_image_ref)
        .map_err(|e| SurfaceActionError::InvalidInput(format!("invalid image reference: {e}")))?;

    let tenant_db = ctx.tenant_db();
    let db = tenant_db.db();

    // Use BEGIN IMMEDIATE so SQLite promotes to RESERVED lock before the first read,
    // preventing SQLITE_BUSY_SNAPSHOT when another connection commits mid-transaction.
    let txn = begin_immediate(db).await.map_err(|e| {
        SurfaceActionError::ControllerIntegration(format!("failed to begin transaction: {e}"))
    })?;

    let plugin_rows = tenant_db
        .find_via_tenant_join::<host_software_item_plugin::Entity, software_item::Entity>(
            host_software_item_plugin::Relation::SoftwareItem.def(),
        )
        .join(
            JoinType::InnerJoin,
            host_software_item_plugin::Relation::Host.def(),
        )
        .filter(host::Column::TenantId.eq(tenant_db.tenant_id()))
        .filter(host_software_item_plugin::Column::HostId.eq(host_id))
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(software_item_id))
        .all(&txn)
        .await
        .map_err(|e| {
            SurfaceActionError::ControllerIntegration(format!(
                "database error loading plugin rows: {e}"
            ))
        })?;

    if plugin_rows.is_empty() {
        return Err(SurfaceActionError::ControllerIntegration(
            "no plugin assignments found for this host".to_string(),
        ));
    }

    let hsi_row = tenant_db
        .find_via_tenant_join::<host_software_item::Entity, software_item::Entity>(
            host_software_item::Relation::SoftwareItem.def(),
        )
        .join(
            JoinType::InnerJoin,
            host_software_item::Relation::Host.def(),
        )
        .filter(host::Column::TenantId.eq(tenant_db.tenant_id()))
        .filter(host_software_item::Column::HostId.eq(host_id))
        .filter(host_software_item::Column::SoftwareItemId.eq(software_item_id))
        .one(&txn)
        .await
        .map_err(|e| {
            SurfaceActionError::ControllerIntegration(format!(
                "database error loading host_software_item: {e}"
            ))
        })?
        .ok_or_else(|| {
            SurfaceActionError::ControllerIntegration(format!(
                "host_software_item not found for host {host_id} / item {software_item_id}"
            ))
        })?;

    for row in plugin_rows {
        if row.plugin_type != DOCKER_RELEASES_CONFIG_TYPE {
            continue;
        }
        let new_pkg_id = match extract_container_suffix(&row.package_identifier) {
            Some(container) => format!("{new_image_ref}#{container}"),
            None => new_image_ref.clone(),
        };
        let mut active: host_software_item_plugin::ActiveModel = row.into();
        active.package_identifier = Set(new_pkg_id);
        active.update(&txn).await.map_err(|e| {
            SurfaceActionError::ControllerIntegration(format!("failed to update plugin row: {e}"))
        })?;
    }

    let mut hsi_active: host_software_item::ActiveModel = hsi_row.into();
    hsi_active.package_identifier = Set(Some(new_image_ref.clone()));
    hsi_active.installed_version = Set(None);
    hsi_active.installed_display_version = Set(None);
    hsi_active.installed_version_detected_at = Set(None);
    hsi_active.latest_version = Set(None);
    hsi_active.latest_version_fetched_at = Set(None);
    hsi_active.latest_release_metadata = Set(None);
    hsi_active.update_category = Set("unknown".to_string());
    hsi_active.update(&txn).await.map_err(|e| {
        SurfaceActionError::ControllerIntegration(format!(
            "failed to update host_software_item: {e}"
        ))
    })?;

    txn.commit().await.map_err(|e| {
        SurfaceActionError::ControllerIntegration(format!("failed to commit transaction: {e}"))
    })?;

    tracing::info!(
        %host_id,
        %software_item_id,
        %new_image_ref,
        "Docker tag switched successfully"
    );

    Ok(serde_json::json!({
        "ok": true,
        "message": "Tag switched. Run a version check to update status.",
    }))
}

fn parse_action_params<T>(
    params: serde_json::Value,
    action_id: &str,
) -> Result<T, SurfaceActionError>
where
    T: DeserializeOwned,
{
    serde_json::from_value(params).map_err(|error| {
        SurfaceActionError::InvalidInput(format!(
            "invalid params for action '{action_id}': {error}"
        ))
    })
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_core::InteractionDeliveryKind;

    use crate::plugin::docker_plugin_surfaces;

    #[test]
    fn docker_plugin_surfaces_pair_every_interaction_with_plugin_handled_delivery() {
        let registrations = docker_plugin_surfaces();
        let mut seen: Vec<(String, String, InteractionDeliveryKind)> = Vec::new();
        for registration in &registrations {
            for surface in &registration.surfaces {
                for interaction in &surface.interactions {
                    assert_eq!(
                        interaction.descriptor().transport,
                        uptrakit_plugin_infrastructure_core::surfaces::InteractionTransport::ControllerLocal
                    );
                    seen.push((
                        surface.descriptor.surface_id.as_str().to_string(),
                        interaction.descriptor().interaction_id.as_str().to_string(),
                        interaction.delivery().kind(),
                    ));
                }
            }
        }
        let expected: Vec<(&str, &str, InteractionDeliveryKind)> = vec![
            (
                "docker.item-host-actions",
                "switch-tag",
                InteractionDeliveryKind::PluginHandled,
            ),
            (
                "docker.item-host-actions",
                "current-tag",
                InteractionDeliveryKind::PluginHandled,
            ),
        ];
        for (surface, id, kind) in &expected {
            assert!(
                seen.iter()
                    .any(|(s, i, k)| s == surface && i == id && k == kind),
                "missing ({surface}, {id}, {kind:?})"
            );
        }
        assert_eq!(
            seen.len(),
            expected.len(),
            "unexpected total interaction count across docker_plugin_surfaces()"
        );
    }

    #[test]
    fn parse_action_params_switch_tag_valid() {
        let params = serde_json::json!({
            "host_id": "01944c3c-6a3a-7000-8000-000000000001",
            "software_item_id": "01944c3c-6a3a-7000-8000-000000000002",
            "new_image_ref": "ghcr.io/example/app:1.2.3",
        });
        let parsed = parse_action_params::<DockerSwitchTagRequest>(params, "switch-tag")
            .expect("request should parse");
        assert_eq!(parsed.new_image_ref, "ghcr.io/example/app:1.2.3");
    }

    #[test]
    fn parse_action_params_get_current_tag_missing_field_is_invalid_input() {
        let params = serde_json::json!({
            "software_item_id": "01944c3c-6a3a-7000-8000-000000000002",
        });
        let error = parse_action_params::<DockerItemHostRequest>(params, "current-tag")
            .expect_err("request must fail");
        assert!(
            error
                .to_string()
                .contains("invalid params for action 'current-tag'"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn parse_action_params_switch_tag_invalid_uuid_is_invalid_input() {
        let params = serde_json::json!({
            "host_id": "not-a-uuid",
            "software_item_id": "01944c3c-6a3a-7000-8000-000000000002",
            "new_image_ref": "ghcr.io/example/app:1.2.3",
        });
        let error = parse_action_params::<DockerSwitchTagRequest>(params, "switch-tag")
            .expect_err("request must fail");
        assert!(
            error
                .to_string()
                .contains("invalid params for action 'switch-tag'"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn strip_container_suffix_with_suffix() {
        assert_eq!(
            strip_container_suffix("ghcr.io/example/app:1.0#web"),
            "ghcr.io/example/app:1.0"
        );
    }

    #[test]
    fn strip_container_suffix_without_suffix() {
        assert_eq!(
            strip_container_suffix("ghcr.io/example/app:1.0"),
            "ghcr.io/example/app:1.0"
        );
    }

    #[test]
    fn extract_container_suffix_with_suffix() {
        assert_eq!(
            extract_container_suffix("ghcr.io/example/app:1.0#web"),
            Some("web")
        );
    }

    #[test]
    fn extract_container_suffix_without_suffix() {
        assert_eq!(extract_container_suffix("ghcr.io/example/app:1.0"), None);
    }

    // ── Tenant-isolation harness ────────────────────────────────────────────

    use sea_orm::{ActiveModelTrait, Database, DatabaseConnection, Set};
    use time::OffsetDateTime;
    use uptrakit_plugin_infrastructure_core::SurfaceActionContext;
    use uptrakit_shared_db::entity::{
        host, host_software_item, host_software_item_plugin, software_item, tenant,
    };
    use uptrakit_tenant_db::TenantDb;
    use uuid::Uuid;

    struct TestController {
        tenant_db: TenantDb,
    }

    impl uptrakit_plugin_infrastructure_core::SurfaceActionController for TestController {
        fn tenant_id(&self) -> Uuid {
            self.tenant_db.tenant_id()
        }
        fn user_id(&self) -> Option<Uuid> {
            None
        }
        fn tenant_db(&self) -> &TenantDb {
            &self.tenant_db
        }
    }

    async fn setup_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory");
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .expect("run migrations");
        db
    }

    async fn seed_tenant(db: &DatabaseConnection, id: Uuid) {
        let now = OffsetDateTime::now_utc();
        tenant::ActiveModel {
            id: Set(id),
            name: Set(format!("tenant-{id}")),
            slug: Set(id.to_string()),
            is_default: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("seed tenant");
    }

    async fn seed_host(db: &DatabaseConnection, id: Uuid, tenant_id: Uuid) {
        let now = OffsetDateTime::now_utc();
        host::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant_id),
            machine_id: Set(id.to_string()),
            hostname: Set("test-host".to_string()),
            friendly_name: Set("Test Host".to_string()),
            os_type: Set(None),
            os_version: Set(None),
            architecture: Set(None),
            ip_address: Set(None),
            host_features: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("seed host");
    }

    async fn seed_software_item(db: &DatabaseConnection, id: Uuid, tenant_id: Uuid) {
        let now = OffsetDateTime::now_utc();
        software_item::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant_id),
            name: Set("nginx".to_string()),
            featured: Set(false),
            icon_url: Set(None),
            last_checked_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            awaiting_restart_timeout: Set(None),
        }
        .insert(db)
        .await
        .expect("seed software_item");
    }

    async fn seed_host_software_item(
        db: &DatabaseConnection,
        id: Uuid,
        host_id: Uuid,
        software_item_id: Uuid,
        qualifier: Option<&str>,
    ) {
        let now = OffsetDateTime::now_utc();
        host_software_item::ActiveModel {
            id: Set(id),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            qualifier: Set(qualifier.map(str::to_string)),
            plugin_config_id: Set(None),
            package_identifier: Set(Some("nginx:1.0".to_string())),
            installed_version: Set(Some("1.0".to_string())),
            installed_version_detected_at: Set(Some(now)),
            installed_display_version: Set(Some("1.0".to_string())),
            latest_version: Set(Some("1.1".to_string())),
            latest_version_fetched_at: Set(Some(now)),
            latest_release_metadata: Set(None),
            last_updated_at: Set(None),
            linked_at: Set(now),
            update_category: Set("patch".to_string()),
            deactivated_at: Set(None),
            last_discovered_at: Set(None),
            discovery_source: Set(None),
            missing_since: Set(None),
        }
        .insert(db)
        .await
        .expect("seed host_software_item");
    }

    /// Docker call sites pass `DOCKER_RELEASES_CONFIG_TYPE` and ordinal 0.
    /// The live unique index is `uq_hsip_host_item_role_ordinal` on
    /// `(host_id, software_item_id, role, ordinal)` (verified against the
    /// running SQLite schema — NOT `host_software_item_id`-scoped, despite
    /// the index created by `m20260318_000001_host_software_item_qualifier.rs`'s
    /// `up()`; a later migration in the chain keeps the original
    /// `(host_id, software_item_id, role, ordinal)` form live), so any two
    /// rows sharing `(host_id, software_item_id, role)` — even across
    /// different `host_software_item_id` values — must use distinct ordinals
    /// or the insert panics at seed time.
    ///
    /// Deviation from the plan's 8-param signature (clippy::too_many_arguments
    /// caps at 7 and is deny-level): the `id` parameter is dropped and
    /// generated internally, since every plan call site passed a throwaway
    /// `Uuid::now_v7()` and never read it back.
    async fn seed_plugin_row(
        db: &DatabaseConnection,
        host_id: Uuid,
        software_item_id: Uuid,
        host_software_item_id: Uuid,
        plugin_type: &str,
        package_identifier: &str,
        ordinal: i32,
    ) {
        let id = Uuid::now_v7();
        let now = OffsetDateTime::now_utc();
        host_software_item_plugin::ActiveModel {
            id: Set(id),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            host_software_item_id: Set(host_software_item_id),
            plugin_config_id: Set(None),
            plugin_type: Set(plugin_type.to_string()),
            role: Set("fetch_releases".to_string()),
            ordinal: Set(ordinal),
            package_identifier: Set(package_identifier.to_string()),
            config: Set(None),
            execution_site: Set("auto".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .expect("seed host_software_item_plugin");
    }

    /// Convenience: seed a full tenant→host+item→hsi→docker-plugin chain, all
    /// four rows owned by `tenant_id`. Returns (host_id, software_item_id).
    async fn seed_full_chain(
        db: &DatabaseConnection,
        tenant_id: Uuid,
        package_identifier: &str,
    ) -> (Uuid, Uuid) {
        let host_id = Uuid::now_v7();
        let item_id = Uuid::now_v7();
        let hsi_id = Uuid::now_v7();
        seed_host(db, host_id, tenant_id).await;
        seed_software_item(db, item_id, tenant_id).await;
        seed_host_software_item(db, hsi_id, host_id, item_id, None).await;
        seed_plugin_row(
            db,
            host_id,
            item_id,
            hsi_id,
            DOCKER_RELEASES_CONFIG_TYPE,
            package_identifier,
            0,
        )
        .await;
        (host_id, item_id)
    }

    #[tokio::test]
    async fn get_current_tag_same_tenant_returns_image_ref() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        seed_tenant(&db, tenant_id).await;
        // Suffixed seed: spec case 3 requires the returned reference to have
        // the `#container` suffix STRIPPED, so the fixture must carry one —
        // an unsuffixed seed would leave `strip_container_suffix` unexercised
        // on the scoped path.
        let (host_id, software_item_id) = seed_full_chain(&db, tenant_id, "nginx:1.0#web").await;

        let controller = TestController {
            tenant_db: TenantDb::new(db.clone(), tenant_id),
        };
        let ctx = SurfaceActionContext {
            controller: &controller,
        };
        let req = DockerItemHostRequest {
            host_id,
            software_item_id,
        };

        let out = handle_get_current_tag(&ctx, req).await.expect("read ok");
        assert_eq!(out["new_image_ref"], "nginx:1.0", "suffix must be stripped");
    }

    #[tokio::test]
    async fn get_current_tag_cross_tenant_returns_empty() {
        let db = setup_db().await;
        let victim = Uuid::now_v7();
        let attacker = Uuid::now_v7();
        seed_tenant(&db, victim).await;
        seed_tenant(&db, attacker).await;
        // Chain belongs to the victim tenant.
        let (host_id, software_item_id) = seed_full_chain(&db, victim, "nginx:1.0").await;

        // Controller scoped to the attacker tenant, replaying the victim's UUIDs.
        let controller = TestController {
            tenant_db: TenantDb::new(db.clone(), attacker),
        };
        let ctx = SurfaceActionContext {
            controller: &controller,
        };
        let req = DockerItemHostRequest {
            host_id,
            software_item_id,
        };

        let out = handle_get_current_tag(&ctx, req).await.expect("read ok");
        assert_eq!(
            out["new_image_ref"], "",
            "cross-tenant read must not leak the tag"
        );
    }

    // Spec case 5 — mismatched-parent, BOTH orientations. A single orientation
    // only pins one of the two single-anchor mutants, so both are required.
    // Caller = tenant A (`attacker`), victim = tenant B (`victim`). Each row is
    // an *unreachable* state via the API (host_software_item rows are only
    // created for a same-tenant (host, software_item) pair); the tests fabricate
    // the row directly to prove BOTH anchors are enforced independently.

    /// Spec 5a — `host∈A`, `software_item∈B`. Catches a `host`-anchored
    /// single-parent mutant (drops the `software_item` filter → would leak).
    #[tokio::test]
    async fn get_current_tag_mismatched_parents_host_a_item_b_returns_empty() {
        let db = setup_db().await;
        let victim = Uuid::now_v7();
        let attacker = Uuid::now_v7();
        seed_tenant(&db, victim).await;
        seed_tenant(&db, attacker).await;

        let host_id = Uuid::now_v7();
        let item_id = Uuid::now_v7();
        let hsi_id = Uuid::now_v7();
        seed_host(&db, host_id, attacker).await; // host owned by A (attacker)
        seed_software_item(&db, item_id, victim).await; // item owned by B (victim)
        seed_host_software_item(&db, hsi_id, host_id, item_id, None).await;
        seed_plugin_row(
            &db,
            host_id,
            item_id,
            hsi_id,
            DOCKER_RELEASES_CONFIG_TYPE,
            "nginx:1.0",
            0,
        )
        .await;

        // Controller scoped to `attacker`: owns the host (host anchor passes) but
        // not the item (software_item anchor fails) → empty read.
        let controller = TestController {
            tenant_db: TenantDb::new(db.clone(), attacker),
        };
        let ctx = SurfaceActionContext {
            controller: &controller,
        };
        let req = DockerItemHostRequest {
            host_id,
            software_item_id: item_id,
        };

        let out = handle_get_current_tag(&ctx, req).await.expect("read ok");
        assert_eq!(
            out["new_image_ref"], "",
            "5a: software_item anchor must reject a foreign item"
        );
    }

    /// Spec 5b (higher-value) — `host∈B`, `software_item∈A`. Catches a
    /// `software_item`-anchored single-parent mutant that keeps the chosen anchor
    /// but drops the **added** `host` defense-in-depth join. Because the fix
    /// anchors on `software_item`, the most likely future regression is removing
    /// the "extra" chained host join — only 5b keeps that mutant dead.
    #[tokio::test]
    async fn get_current_tag_mismatched_parents_host_b_item_a_returns_empty() {
        let db = setup_db().await;
        let victim = Uuid::now_v7();
        let attacker = Uuid::now_v7();
        seed_tenant(&db, victim).await;
        seed_tenant(&db, attacker).await;

        let host_id = Uuid::now_v7();
        let item_id = Uuid::now_v7();
        let hsi_id = Uuid::now_v7();
        seed_host(&db, host_id, victim).await; // host owned by B (victim)
        seed_software_item(&db, item_id, attacker).await; // item owned by A (attacker)
        seed_host_software_item(&db, hsi_id, host_id, item_id, None).await;
        seed_plugin_row(
            &db,
            host_id,
            item_id,
            hsi_id,
            DOCKER_RELEASES_CONFIG_TYPE,
            "nginx:1.0",
            0,
        )
        .await;

        // Controller scoped to `attacker`: owns the item (software_item anchor
        // passes) but not the host (host anchor fails) → empty read.
        let controller = TestController {
            tenant_db: TenantDb::new(db.clone(), attacker),
        };
        let ctx = SurfaceActionContext {
            controller: &controller,
        };
        let req = DockerItemHostRequest {
            host_id,
            software_item_id: item_id,
        };

        let out = handle_get_current_tag(&ctx, req).await.expect("read ok");
        assert_eq!(
            out["new_image_ref"], "",
            "5b: added host join must reject a foreign host"
        );
    }

    async fn load_pkg_id(db: &DatabaseConnection, host_id: Uuid, software_item_id: Uuid) -> String {
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
        host_software_item_plugin::Entity::find()
            .filter(host_software_item_plugin::Column::HostId.eq(host_id))
            .filter(host_software_item_plugin::Column::SoftwareItemId.eq(software_item_id))
            .one(db)
            .await
            .expect("query plugin row")
            .expect("plugin row exists")
            .package_identifier
    }

    #[tokio::test]
    async fn switch_tag_same_tenant_updates_rows() {
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        seed_tenant(&db, tenant_id).await;

        // Production multi-container shape (spec case 4): two hsi rows for the
        // pair with distinct qualifiers (the qualifier IS the container name), a
        // Docker plugin row under each with distinct #container suffixes, plus
        // one non-Docker plugin row that the update loop must skip.
        let host_id = Uuid::now_v7();
        let software_item_id = Uuid::now_v7();
        seed_host(&db, host_id, tenant_id).await;
        seed_software_item(&db, software_item_id, tenant_id).await;
        let hsi_web = Uuid::now_v7();
        let hsi_worker = Uuid::now_v7();
        seed_host_software_item(&db, hsi_web, host_id, software_item_id, Some("web")).await;
        seed_host_software_item(&db, hsi_worker, host_id, software_item_id, Some("worker")).await;
        seed_plugin_row(
            &db,
            host_id,
            software_item_id,
            hsi_web,
            DOCKER_RELEASES_CONFIG_TYPE,
            "nginx:1.0#web",
            0,
        )
        .await;
        seed_plugin_row(
            &db,
            host_id,
            software_item_id,
            hsi_worker,
            DOCKER_RELEASES_CONFIG_TYPE,
            "nginx:1.0#worker",
            1,
        )
        .await;
        // Ordinal 2: the live unique index is `uq_hsip_host_item_role_ordinal`
        // on `(host_id, software_item_id, role, ordinal)` — NOT
        // `host_software_item_id`-scoped (verified against the running SQLite
        // schema; the m20260318 migration's up() creates a
        // host_software_item_id-scoped index but a later migration in this
        // chain restores/keeps the (host_id, software_item_id, role, ordinal)
        // form as the live constraint). All three rows here share the same
        // `(host_id, software_item_id, role="fetch_releases")`, so each needs
        // a distinct ordinal regardless of which `host_software_item_id` it
        // belongs to.
        seed_plugin_row(
            &db,
            host_id,
            software_item_id,
            hsi_web,
            "package_managers.apt",
            "nginx",
            2,
        )
        .await;

        let controller = TestController {
            tenant_db: TenantDb::new(db.clone(), tenant_id),
        };
        let ctx = SurfaceActionContext {
            controller: &controller,
        };
        let req = DockerSwitchTagRequest {
            host_id,
            software_item_id,
            new_image_ref: "nginx:2.0".to_string(),
        };

        handle_switch_tag(&ctx, req).await.expect("switch ok");

        // All Docker plugin rows rewritten, per-row #container suffix preserved.
        let plugin_rows = host_software_item_plugin::Entity::find()
            .filter(host_software_item_plugin::Column::HostId.eq(host_id))
            .all(&db)
            .await
            .expect("load plugin rows");
        let mut docker_ids: Vec<_> = plugin_rows
            .iter()
            .filter(|r| r.plugin_type == DOCKER_RELEASES_CONFIG_TYPE)
            .map(|r| r.package_identifier.as_str())
            .collect();
        docker_ids.sort_unstable();
        assert_eq!(docker_ids, ["nginx:2.0#web", "nginx:2.0#worker"]);

        // Non-Docker row skipped.
        let apt_row = plugin_rows
            .iter()
            .find(|r| r.plugin_type == "package_managers.apt")
            .expect("apt row exists");
        assert_eq!(
            apt_row.package_identifier, "nginx",
            "non-Docker row must be skipped"
        );

        // Exactly one of the two hsi rows updated (`.one()` picks arbitrarily;
        // which one is not asserted). The updated row has its version state
        // cleared and update_category set to "unknown".
        let hsi_rows = host_software_item::Entity::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .all(&db)
            .await
            .expect("load hsi rows");
        assert_eq!(hsi_rows.len(), 2);
        let updated: Vec<_> = hsi_rows
            .iter()
            .filter(|r| r.package_identifier.as_deref() == Some("nginx:2.0"))
            .collect();
        assert_eq!(
            updated.len(),
            1,
            "exactly one hsi row updated (pre-existing multi-qualifier quirk, pinned on purpose)"
        );
        // .first().expect(): `indexing_slicing` is denied workspace-wide; do not
        // trade this for an `#[expect]` (in-test coverage would make it unfulfilled).
        let updated_row = updated.first().expect("len asserted above");
        assert_eq!(updated_row.update_category, "unknown");
        assert!(updated_row.installed_version.is_none());
        assert!(updated_row.installed_display_version.is_none());
        assert!(updated_row.latest_version.is_none());
    }

    #[tokio::test]
    async fn switch_tag_cross_tenant_rejected_and_unchanged() {
        let db = setup_db().await;
        let victim = Uuid::now_v7();
        let attacker = Uuid::now_v7();
        seed_tenant(&db, victim).await;
        seed_tenant(&db, attacker).await;
        let (host_id, software_item_id) = seed_full_chain(&db, victim, "nginx:1.0").await;

        let controller = TestController {
            tenant_db: TenantDb::new(db.clone(), attacker),
        };
        let ctx = SurfaceActionContext {
            controller: &controller,
        };
        let req = DockerSwitchTagRequest {
            host_id,
            software_item_id,
            new_image_ref: "evil:latest".to_string(),
        };

        let err = handle_switch_tag(&ctx, req).await.expect_err("must reject");
        // Pin the exact reject string: it is the spec's error contract (mapped
        // to AuditOutcome::Denied in surface-proxy). A bare variant match would
        // also accept tx-begin/load failures and mask a broken deny path.
        assert!(matches!(
            &err,
            SurfaceActionError::ControllerIntegration(msg)
                if msg == "no plugin assignments found for this host"
        ));
        // Victim plugin row untouched.
        assert_eq!(
            load_pkg_id(&db, host_id, software_item_id).await,
            "nginx:1.0"
        );
        // Spec case 2 also requires the victim's host_software_item row —
        // package_identifier AND version columns — asserted unchanged.
        {
            use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
            let hsi = host_software_item::Entity::find()
                .filter(host_software_item::Column::HostId.eq(host_id))
                .filter(host_software_item::Column::SoftwareItemId.eq(software_item_id))
                .one(&db)
                .await
                .expect("query hsi row")
                .expect("hsi row exists");
            assert_eq!(hsi.package_identifier.as_deref(), Some("nginx:1.0"));
            assert_eq!(hsi.installed_version.as_deref(), Some("1.0"));
            assert_eq!(hsi.latest_version.as_deref(), Some("1.1"));
            assert_eq!(hsi.update_category, "patch");
        }
    }

    // Spec case 5 — mismatched-parent write guard, BOTH orientations. Each row
    // is unreachable via the API (host_software_item rows are only created for a
    // same-tenant (host, software_item) pair); fabricated directly to prove BOTH
    // anchors are enforced. Caller = tenant A (`attacker`), victim = tenant B.

    /// Spec 5a — `host∈A`, `software_item∈B`. Catches a `host`-anchored mutant
    /// (drops the `software_item` filter → would leak the write).
    #[tokio::test]
    async fn switch_tag_mismatched_parents_host_a_item_b_rejected() {
        let db = setup_db().await;
        let victim = Uuid::now_v7();
        let attacker = Uuid::now_v7();
        seed_tenant(&db, victim).await;
        seed_tenant(&db, attacker).await;

        let host_id = Uuid::now_v7();
        let item_id = Uuid::now_v7();
        let hsi_id = Uuid::now_v7();
        seed_host(&db, host_id, attacker).await; // host owned by A (attacker)
        seed_software_item(&db, item_id, victim).await; // item owned by B (victim)
        seed_host_software_item(&db, hsi_id, host_id, item_id, None).await;
        seed_plugin_row(
            &db,
            host_id,
            item_id,
            hsi_id,
            DOCKER_RELEASES_CONFIG_TYPE,
            "nginx:1.0",
            0,
        )
        .await;

        let controller = TestController {
            tenant_db: TenantDb::new(db.clone(), attacker),
        };
        let ctx = SurfaceActionContext {
            controller: &controller,
        };
        let req = DockerSwitchTagRequest {
            host_id,
            software_item_id: item_id,
            new_image_ref: "evil:latest".to_string(),
        };

        let err = handle_switch_tag(&ctx, req)
            .await
            .expect_err("must reject 5a");
        // Pin the exact reject string: it is the spec's error contract (mapped
        // to AuditOutcome::Denied in surface-proxy). A bare variant match would
        // also accept tx-begin/load failures and mask a broken deny path.
        assert!(matches!(
            &err,
            SurfaceActionError::ControllerIntegration(msg)
                if msg == "no plugin assignments found for this host"
        ));
        assert_eq!(load_pkg_id(&db, host_id, item_id).await, "nginx:1.0");
    }

    /// Spec 5b (higher-value) — `host∈B`, `software_item∈A`. Catches a
    /// `software_item`-anchored mutant that keeps the anchor but drops the added
    /// `host` defense-in-depth join → would leak the write.
    #[tokio::test]
    async fn switch_tag_mismatched_parents_host_b_item_a_rejected() {
        let db = setup_db().await;
        let victim = Uuid::now_v7();
        let attacker = Uuid::now_v7();
        seed_tenant(&db, victim).await;
        seed_tenant(&db, attacker).await;

        let host_id = Uuid::now_v7();
        let item_id = Uuid::now_v7();
        let hsi_id = Uuid::now_v7();
        seed_host(&db, host_id, victim).await; // host owned by B (victim)
        seed_software_item(&db, item_id, attacker).await; // item owned by A (attacker)
        seed_host_software_item(&db, hsi_id, host_id, item_id, None).await;
        seed_plugin_row(
            &db,
            host_id,
            item_id,
            hsi_id,
            DOCKER_RELEASES_CONFIG_TYPE,
            "nginx:1.0",
            0,
        )
        .await;

        let controller = TestController {
            tenant_db: TenantDb::new(db.clone(), attacker),
        };
        let ctx = SurfaceActionContext {
            controller: &controller,
        };
        let req = DockerSwitchTagRequest {
            host_id,
            software_item_id: item_id,
            new_image_ref: "evil:latest".to_string(),
        };

        let err = handle_switch_tag(&ctx, req)
            .await
            .expect_err("must reject 5b");
        // Pin the exact reject string: it is the spec's error contract (mapped
        // to AuditOutcome::Denied in surface-proxy). A bare variant match would
        // also accept tx-begin/load failures and mask a broken deny path.
        assert!(matches!(
            &err,
            SurfaceActionError::ControllerIntegration(msg)
                if msg == "no plugin assignments found for this host"
        ));
        assert_eq!(load_pkg_id(&db, host_id, item_id).await, "nginx:1.0");
    }
}
