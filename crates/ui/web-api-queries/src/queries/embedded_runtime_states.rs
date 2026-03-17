use std::collections::HashMap;

use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use uptrakit_shared_db::entity::embedded_service_runtime_state;

/// Runtime-state rows older than this are ignored to avoid showing stale
/// yield ownership after an owning controller disappears.
const YIELD_STATE_FRESHNESS: Duration = Duration::seconds(30);

pub(crate) async fn load_fresh_yielded_to(
    db: &DatabaseConnection,
    service_ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<Uuid>>, sea_orm::DbErr> {
    if service_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let cutoff = OffsetDateTime::now_utc() - YIELD_STATE_FRESHNESS;
    let rows = embedded_service_runtime_state::Entity::find()
        .filter(
            embedded_service_runtime_state::Column::ServiceId.is_in(service_ids.iter().copied()),
        )
        .filter(embedded_service_runtime_state::Column::UpdatedAt.gte(cutoff))
        .all(db)
        .await?;

    let mut yielded = HashMap::with_capacity(rows.len());
    for row in rows {
        let Some(json) = row.yielded_to_json.as_deref() else {
            continue;
        };

        let parsed: Vec<Uuid> = match serde_json::from_str(json) {
            Ok(ids) => ids,
            Err(err) => {
                tracing::warn!(
                    service_id = %row.service_id,
                    error = %err,
                    "failed to parse embedded yielded_to_json"
                );
                continue;
            }
        };

        if !parsed.is_empty() {
            yielded.insert(row.service_id, parsed);
        }
    }

    Ok(yielded)
}
