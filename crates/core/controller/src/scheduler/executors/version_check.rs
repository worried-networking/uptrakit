use std::collections::HashMap;
use std::str::FromStr;

use rootcause::prelude::*;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, JoinType, QueryFilter, QuerySelect, RelationTrait,
};
use uptrakit_internal_wire::{CheckVersionsPayload, ControllerMessage, VersionCheckAssignment};
use uptrakit_shared_db::entity::{
    host_software_item, provider_config, scheduled_task, service, service_host, software_item,
};
use uptrakit_shared_types::ProviderType;
use uptrakit_web_api::notification_service::NotificationService;
use uuid::Uuid;

use crate::scheduler::error::SchedulerError;
use crate::scheduler::executor::TaskExecutor;

/// Sends `CheckVersions` messages to connected agents for installed-version detection.
///
/// Groups software items by the agent (service) responsible for the host they are
/// assigned to, then sends one `CheckVersions` message per agent.
pub struct VersionCheckExecutor {
    db: DatabaseConnection,
    notification_service: NotificationService,
}

impl VersionCheckExecutor {
    pub fn new(db: DatabaseConnection, notification_service: NotificationService) -> Self {
        Self {
            db,
            notification_service,
        }
    }
}

/// Intermediate row produced by the joined query.
#[derive(Debug)]
struct AssignmentRow {
    service_id: Uuid,
    software_item_id: Uuid,
    name: String,
    provider_type: String,
    package_identifier: String,
    config: serde_json::Value,
    config_override: Option<serde_json::Value>,
}

#[async_trait::async_trait]
impl TaskExecutor for VersionCheckExecutor {
    async fn execute(&self, task: &scheduled_task::Model) -> crate::scheduler::error::Result<()> {
        let tenant_id = task.tenant_id;

        // Fetch all enabled software items joined through hosts -> agents for this tenant.
        let rows = self.fetch_assignments(tenant_id).await?;
        if rows.is_empty() {
            tracing::debug!("no software items assigned to agents for version check");
            return Ok(());
        }

        // Group by agent (service_id)
        let mut by_agent: HashMap<Uuid, Vec<VersionCheckAssignment>> = HashMap::new();
        for row in rows {
            let provider_type = ProviderType::from_str(&row.provider_type).map_err(|_| {
                report!(SchedulerError::Execution(format!(
                    "unknown provider type: {}",
                    row.provider_type
                )))
            })?;
            let config = match row.config_override {
                Some(ovr) => merge_config(&row.config, &ovr),
                None => row.config,
            };
            by_agent
                .entry(row.service_id)
                .or_default()
                .push(VersionCheckAssignment {
                    software_item_id: row.software_item_id,
                    name: row.name,
                    provider_type,
                    package_identifier: row.package_identifier,
                    config,
                });
        }

        let agent_count = by_agent.len();
        let item_count: usize = by_agent.values().map(|v| v.len()).sum();

        for (service_id, assignments) in by_agent {
            let msg = ControllerMessage::CheckVersions(CheckVersionsPayload { assignments });
            self.notification_service.send(&service_id, msg).await;
        }

        tracing::debug!(
            agents = agent_count,
            items = item_count,
            "sent version check requests"
        );
        Ok(())
    }
}

impl VersionCheckExecutor {
    /// Query enabled software items with their host-to-agent mapping.
    ///
    /// Returns one row per (agent_service_id, software_item) pair.
    async fn fetch_assignments(
        &self,
        tenant_id: Uuid,
    ) -> crate::scheduler::error::Result<Vec<AssignmentRow>> {
        // software_item -> provider_config (for provider_type + config)
        // software_item -> host_software_item -> host -> service_host -> service (agent)
        //
        // We use a raw-ish select with joins since SeaORM's relation chaining
        // doesn't easily produce a flat tuple across 5 tables.
        #[derive(Debug, sea_orm::FromQueryResult)]
        struct Row {
            service_id: Uuid,
            software_item_id: Uuid,
            name: String,
            provider_type: String,
            package_identifier: String,
            config: serde_json::Value,
            config_override: Option<serde_json::Value>,
        }

        let rows: Vec<Row> = software_item::Entity::find()
            .select_only()
            .column_as(service::Column::Id, "service_id")
            .column_as(software_item::Column::Id, "software_item_id")
            .column_as(software_item::Column::Name, "name")
            .column_as(provider_config::Column::ProviderType, "provider_type")
            .column_as(
                software_item::Column::PackageIdentifier,
                "package_identifier",
            )
            .column_as(provider_config::Column::Config, "config")
            .column_as(software_item::Column::ConfigOverride, "config_override")
            .join(
                JoinType::InnerJoin,
                software_item::Relation::ProviderConfig.def(),
            )
            .join(
                JoinType::InnerJoin,
                host_software_item::Relation::SoftwareItem.def().rev(),
            )
            .join(
                JoinType::InnerJoin,
                service_host::Relation::Host.def().rev(),
            )
            .join(JoinType::InnerJoin, service_host::Relation::Service.def())
            .filter(software_item::Column::TenantId.eq(tenant_id))
            .filter(software_item::Column::Enabled.eq(true))
            .filter(software_item::Column::DeactivatedAt.is_null())
            .filter(provider_config::Column::Enabled.eq(true))
            .filter(provider_config::Column::DeactivatedAt.is_null())
            .filter(service::Column::ServiceType.eq(uptrakit_shared_types::ServiceType::Agent))
            .filter(service::Column::DeactivatedAt.is_null())
            .into_model::<Row>()
            .all(&self.db)
            .await
            .context_to::<SchedulerError>()?;

        Ok(rows
            .into_iter()
            .map(|r| AssignmentRow {
                service_id: r.service_id,
                software_item_id: r.software_item_id,
                name: r.name,
                provider_type: r.provider_type,
                package_identifier: r.package_identifier,
                config: r.config,
                config_override: r.config_override,
            })
            .collect())
    }
}

/// Merge a base provider config with per-item overrides.
fn merge_config(base: &serde_json::Value, overrides: &serde_json::Value) -> serde_json::Value {
    match (base, overrides) {
        (serde_json::Value::Object(b), serde_json::Value::Object(o)) => {
            let mut merged = b.clone();
            for (k, v) in o {
                merged.insert(k.clone(), v.clone());
            }
            serde_json::Value::Object(merged)
        }
        _ => base.clone(),
    }
}
