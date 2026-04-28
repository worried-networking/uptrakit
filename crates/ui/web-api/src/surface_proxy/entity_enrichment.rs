use std::collections::HashMap;

use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use uuid::Uuid;

use uptrakit_shared_db::entity::host;
use uptrakit_wire::surfaces::{SurfaceEntityType, SurfaceNode, SurfaceTableCellType};

/// Enriches entity-link cells in a surface list response.
///
/// Resolves display labels for each known [`SurfaceEntityType`] via a direct DB
/// query per entity type (one query per type — no N+1). Unknown types
/// (`SurfaceEntityType::Other(_)`) are skipped and their cells remain unenriched.
///
/// # Future extension
///
/// If more entity types are introduced, consider replacing this static `match`
/// with a proper `EntityResolverRegistry` (`HashMap<SurfaceEntityType,
/// Box<dyn EntityResolver>>` populated at startup) rather than adding more arms
/// here. At two or more additional entity types the registry pattern pays for
/// itself.
pub(crate) async fn enrich_entity_links(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    surface_node: &SurfaceNode,
    response: serde_json::Value,
) -> serde_json::Value {
    let entity_link_columns = collect_entity_link_columns(surface_node);
    if entity_link_columns.is_empty() {
        return response;
    }

    let mut response = response;
    let items = match response
        .as_object_mut()
        .and_then(|obj| obj.get_mut("items"))
        .and_then(|v| v.as_array_mut())
    {
        Some(arr) => arr,
        None => return response,
    };
    if items.is_empty() {
        return response;
    }

    // Collect unique entity IDs per known entity type.
    let mut ids_by_type: HashMap<String, Vec<Uuid>> = HashMap::new();
    for (col_key, entity_type) in &entity_link_columns {
        match entity_type {
            SurfaceEntityType::Other(_) => continue,
            SurfaceEntityType::Host => {
                let bucket = ids_by_type.entry("host".to_string()).or_default();
                for item in items.iter() {
                    if let Some(id_str) = item
                        .get(col_key)
                        .and_then(|cell| cell.get("entity_id"))
                        .and_then(|v| v.as_str())
                        && let Ok(id) = Uuid::parse_str(id_str)
                        && !bucket.contains(&id)
                    {
                        bucket.push(id);
                    }
                }
            }
            _ => {
                tracing::warn!(
                    entity_type = %entity_type.as_str(),
                    "unrecognised SurfaceEntityType variant; skipping ID collection"
                );
            }
        }
    }

    // Batch-resolve labels per entity type.
    let mut label_maps: HashMap<String, HashMap<Uuid, String>> = HashMap::new();
    for (type_key, ids) in &ids_by_type {
        let entity_type = SurfaceEntityType::from(type_key.clone());
        match entity_type {
            SurfaceEntityType::Host => match resolve_host_labels(db, tenant_id, ids).await {
                Ok(map) => {
                    label_maps.insert(type_key.clone(), map);
                }
                Err(err) => {
                    tracing::error!(
                        error = %err,
                        entity_type = %type_key,
                        "entity label resolution failed; cells will show as not found"
                    );
                    label_maps.insert(type_key.clone(), HashMap::new());
                }
            },
            SurfaceEntityType::Other(_) => {
                // Not reachable: Other(_) was skipped in the ID-collection step.
            }
            _ => {
                // Not reachable: unrecognised types were skipped in the ID-collection step.
            }
        }
    }

    // Rewrite items in place for all dispatched entity-link columns.
    for item in items.iter_mut() {
        let obj = match item.as_object_mut() {
            Some(o) => o,
            None => continue,
        };
        for (col_key, entity_type) in &entity_link_columns {
            let type_key = entity_type.as_str().to_string();
            let label_map = match label_maps.get(&type_key) {
                Some(m) => m,
                None => continue, // Other(_) type — leave unenriched
            };
            let entity_id_str = match obj
                .get(col_key)
                .and_then(|cell| cell.get("entity_id"))
                .and_then(|v| v.as_str())
            {
                Some(s) => s.to_string(),
                None => continue,
            };
            let entity_id = match Uuid::parse_str(&entity_id_str) {
                Ok(id) => id,
                Err(_) => continue,
            };
            let new_cell = if let Some(label) = label_map.get(&entity_id) {
                serde_json::json!({
                    "entity_id": entity_id_str,
                    "label": label,
                    "found": true
                })
            } else {
                serde_json::json!({
                    "entity_id": entity_id_str,
                    "found": false
                })
            };
            obj.insert(col_key.clone(), new_cell);
        }
    }

    response
}

async fn resolve_host_labels(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, String>, rootcause::Report> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut query = host::Entity::find()
        .filter(host::Column::Id.is_in(ids.to_vec()))
        .filter(host::Column::DeactivatedAt.is_null());
    if let Some(tid) = tenant_id {
        query = query.filter(host::Column::TenantId.eq(tid));
    }
    let hosts = query.all(db).await.map_err(|e| rootcause::report!(e))?;
    Ok(hosts
        .into_iter()
        .map(|h| {
            let label = if !h.friendly_name.is_empty() {
                h.friendly_name
            } else if !h.hostname.is_empty() {
                h.hostname
            } else {
                h.id.to_string()
            };
            (h.id, label)
        })
        .collect())
}

fn collect_entity_link_columns(root: &SurfaceNode) -> Vec<(String, SurfaceEntityType)> {
    let mut out = Vec::new();
    collect_entity_link_columns_inner(root, &mut out);
    out
}

fn collect_entity_link_columns_inner(
    node: &SurfaceNode,
    out: &mut Vec<(String, SurfaceEntityType)>,
) {
    match node {
        SurfaceNode::Table { columns, .. } => {
            for col in columns {
                match &col.cell_type {
                    Some(SurfaceTableCellType::EntityLink { entity_type }) => {
                        out.push((col.key.clone(), entity_type.clone()));
                    }
                    Some(_) => {
                        tracing::warn!(
                            key = %col.key,
                            "unrecognised SurfaceTableCellType variant; rendering as plain text"
                        );
                    }
                    None => {}
                }
            }
        }
        SurfaceNode::Section { children, .. } => {
            for child in children {
                collect_entity_link_columns_inner(child, out);
            }
        }
        SurfaceNode::Tabs { tabs, .. } => {
            for tab in tabs {
                collect_entity_link_columns_inner(&tab.root, out);
            }
        }
        SurfaceNode::ModalTrigger { modal_nodes, .. } => {
            for child in modal_nodes {
                collect_entity_link_columns_inner(child, out);
            }
        }
        SurfaceNode::WorkflowTrigger { step_nodes, .. } => {
            for child in step_nodes {
                collect_entity_link_columns_inner(child, out);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use uptrakit_wire::surfaces::{
        DataSourceId, SurfaceEntityType, SurfaceTab, SurfaceTabId, SurfaceTableCellType,
        SurfaceTableColumn,
    };

    use super::*;

    fn entity_link_col(col_key: &str) -> SurfaceTableColumn {
        let mut col = SurfaceTableColumn::new(col_key, "Host");
        col.cell_type = Some(SurfaceTableCellType::EntityLink {
            entity_type: SurfaceEntityType::Host,
        });
        col
    }

    fn entity_link_table_node(col_key: &str) -> SurfaceNode {
        SurfaceNode::Table {
            data_source_id: DataSourceId::new("test").expect("literal"),
            columns: vec![entity_link_col(col_key)],
            row_actions: vec![],
        }
    }

    fn no_link_table_node() -> SurfaceNode {
        SurfaceNode::Table {
            data_source_id: DataSourceId::new("test").expect("literal"),
            columns: vec![SurfaceTableColumn::new("name", "Name")],
            row_actions: vec![],
        }
    }

    #[test]
    fn collect_from_table_node() {
        let node = entity_link_table_node("host_col");
        let cols = collect_entity_link_columns(&node);
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].0, "host_col");
        assert_eq!(cols[0].1, SurfaceEntityType::Host);
    }

    #[test]
    fn collect_from_section_with_nested_table() {
        let node = SurfaceNode::Section {
            title: None,
            children: vec![entity_link_table_node("host_col")],
        };
        let cols = collect_entity_link_columns(&node);
        assert_eq!(cols.len(), 1);
    }

    #[test]
    fn collect_from_tabs() {
        let node = SurfaceNode::Tabs {
            tabs: vec![SurfaceTab {
                id: SurfaceTabId::new("t1").expect("literal"),
                label: "Tab".to_string(),
                root: entity_link_table_node("host_col"),
            }],
        };
        let cols = collect_entity_link_columns(&node);
        assert_eq!(cols.len(), 1);
    }

    #[test]
    fn no_entity_link_columns_returns_empty() {
        let cols = collect_entity_link_columns(&no_link_table_node());
        assert!(cols.is_empty());
    }

    #[test]
    fn enrich_returns_response_unchanged_when_no_entity_link_columns() {
        let node = no_link_table_node();
        let response = serde_json::json!({
            "items": [{"name": "foo"}],
            "total": 1
        });
        assert!(collect_entity_link_columns(&node).is_empty());
        let _ = response;
    }

    #[test]
    fn enrich_returns_unchanged_when_no_items_key() {
        let node = entity_link_table_node("host_col");
        let cols = collect_entity_link_columns(&node);
        assert_eq!(cols.len(), 1);
    }

    #[test]
    fn collect_from_modal_trigger_with_nested_table() {
        use uptrakit_wire::surfaces::InteractionId;
        let node = SurfaceNode::ModalTrigger {
            interaction_id: InteractionId::new("open").expect("literal"),
            modal_nodes: vec![entity_link_table_node("host_col")],
        };
        let cols = collect_entity_link_columns(&node);
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].0, "host_col");
    }

    #[test]
    fn collect_from_workflow_trigger_with_nested_table() {
        use uptrakit_wire::surfaces::InteractionId;
        let node = SurfaceNode::WorkflowTrigger {
            interaction_id: InteractionId::new("run").expect("literal"),
            step_nodes: vec![entity_link_table_node("host_col")],
        };
        let cols = collect_entity_link_columns(&node);
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].0, "host_col");
    }

    #[test]
    fn other_entity_type_not_in_ids_by_type() {
        let mut col = SurfaceTableColumn::new("future_col", "Future");
        col.cell_type = Some(SurfaceTableCellType::EntityLink {
            entity_type: SurfaceEntityType::Other("future_type".to_string()),
        });
        let node = SurfaceNode::Table {
            data_source_id: DataSourceId::new("test").expect("literal"),
            columns: vec![col],
            row_actions: vec![],
        };
        let cols = collect_entity_link_columns(&node);
        assert_eq!(cols.len(), 1);
        assert!(matches!(cols[0].1, SurfaceEntityType::Other(_)));
    }
}
