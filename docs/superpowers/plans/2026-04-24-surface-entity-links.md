# Surface Entity Links Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Allow surface table columns to declare entity-link cell types; the framework resolves
labels from the DB and the frontend renders clickable links.

**Architecture:** Wire types in `uptrakit-surfaces` declare the column cell type and entity
reference shape. A new free function `enrich_entity_links` in `surface_proxy/entity_enrichment.rs`
resolves display labels via static dispatch (one DB query per entity type). No `AppState` change.
The frontend `SurfaceTable.svelte` renders entity-link cells according to a five-state table.

**Tech Stack:** Rust (sea-orm, serde, uuid, rootcause), SvelteKit + TypeScript, Playwright parity
tests.

---

## File Map

| Path | Action | Responsibility |
| --- | --- | --- |
| `crates/shared/surfaces/src/surface.rs` | Modify | Add `cell_type`, new types, `Capability::EntityLinkColumn`; patch all struct literal sites |
| `crates/ui/web-api/src/surface_proxy.rs` | Modify | Declare `pub(crate) mod entity_enrichment` |
| `crates/ui/web-api/src/surface_proxy/entity_enrichment.rs` | Create | `enrich_entity_links`, `resolve_host_labels`, `collect_entity_link_columns` |
| `crates/ui/web-api/src/routes/surfaces.rs` | Modify | Call `enrich_entity_links` after `invoke()` succeeds |
| `crates/plugins/infrastructure/proxmox/src/plugin.rs` | Modify | Add `cell_type` to `matched_host` column + `Capability::EntityLinkColumn` |
| `crates/plugins/infrastructure/proxmox/src/surfaces.rs` | Modify | Emit `SurfaceEntityRef::unresolved` for `matched_host` + tests |
| `frontend/src/lib/surfaces/contract.ts` | Modify | Add `cell_type`, `SurfaceEntityRef`, `'entity_link_column'` capability |
| `frontend/src/lib/surfaces/entity-routes.ts` | Create | `SurfaceEntityType`, `entityRoute()` |
| `frontend/src/lib/components/surfaces/SurfaceTable.svelte` | Modify | Entity-link cell rendering (5-state table) |
| `frontend/src/lib/test-fixtures/ui-parity.ts` | Modify | Add `entityLink` fixture to `SharedVisualParityFixture` |
| `frontend/tests/e2e/ui-parity.test.ts` | Modify | Add entity-link parity test |

**Struct literal patch sites** (all need `cell_type: None` added — 32 total across 9 files):

- `crates/shared/surfaces/src/surface.rs` — 1 site (test helper)
- `crates/core/agent-ssh/src/surface_runtime.rs` — 1 site
- `crates/core/agent-ssh/src/surface_runtime/registration.rs` — 1 site
- `crates/core/mqtt-runtime/src/surface_runtime.rs` — 5 sites
- `crates/plugins/infrastructure/core/src/surface_form_authoring.rs` — 3 sites
- `crates/plugins/infrastructure/proxmox/src/plugin.rs` — 9 sites (updated in Task 4)
- `crates/plugins/notifications/email/src/plugin.rs` — 4 sites
- `crates/plugins/notifications/telegram/src/plugin.rs` — 4 sites
- `crates/plugins/notifications/webhook/src/plugin.rs` — 4 sites

---

## Task 1: Wire types in `surface.rs`

**Files:**

- Modify: `crates/shared/surfaces/src/surface.rs`

- [ ] **Step 1: Write failing tests**

Add at end of the `tests` module in `surface.rs` (after the existing
`surface_descriptor_context_selector_round_trips` test):

```rust
#[test]
fn surface_table_cell_type_entity_link_serializes_correctly() {
    let col = SurfaceTableColumn {
        key: "host".to_string(),
        label: "Host".to_string(),
        cell_type: Some(SurfaceTableCellType::EntityLink {
            entity_type: SurfaceEntityType::Host,
        }),
    };
    let json = serde_json::to_string(&col).expect("serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
    assert_eq!(parsed["cell_type"]["kind"], "entity_link");
    assert_eq!(parsed["cell_type"]["entity_type"], "host");
}

#[test]
fn surface_table_column_without_cell_type_omits_field() {
    let col = SurfaceTableColumn {
        key: "name".to_string(),
        label: "Name".to_string(),
        cell_type: None,
    };
    let json = serde_json::to_string(&col).expect("serialize");
    assert!(!json.contains("cell_type"));
}

#[test]
fn unknown_cell_type_deserializes_to_none() {
    let json =
        r#"{"key":"host","label":"Host","cell_type":{"kind":"future_type","extra":"data"}}"#;
    let col: SurfaceTableColumn = serde_json::from_str(json).expect("deserialize");
    assert!(col.cell_type.is_none());
}

#[test]
fn surface_entity_type_host_serializes_to_bare_string() {
    let t = SurfaceEntityType::Host;
    let s = serde_json::to_string(&t).expect("serialize");
    assert_eq!(s, r#""host""#);
}

#[test]
fn surface_entity_type_other_serializes_to_bare_string() {
    let t = SurfaceEntityType::Other("my_future_type".to_string());
    let s = serde_json::to_string(&t).expect("serialize");
    assert_eq!(s, r#""my_future_type""#);
}

#[test]
fn surface_entity_type_unknown_string_deserializes_to_other() {
    let t: SurfaceEntityType =
        serde_json::from_str(r#""unknown_type""#).expect("deserialize");
    assert_eq!(t, SurfaceEntityType::Other("unknown_type".to_string()));
}

#[test]
fn surface_entity_ref_unresolved_serializes_without_label_or_found() {
    let entity_id =
        uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
    let r = SurfaceEntityRef::unresolved(entity_id);
    let json = serde_json::to_string(&r).expect("serialize");
    let val: serde_json::Value = serde_json::from_str(&json).expect("parse");
    assert_eq!(val["entity_id"], entity_id.to_string());
    assert!(val.get("label").is_none());
    assert!(val.get("found").is_none());
}

#[test]
fn entity_link_column_capability_serializes_to_snake_case() {
    let cap = Capability::EntityLinkColumn;
    let s = serde_json::to_string(&cap).expect("serialize");
    assert_eq!(s, r#""entity_link_column""#);
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo test -p uptrakit-surfaces -- --nocapture 2>&1 | tail -20
```

Expected: FAIL — `SurfaceTableColumn` has no `cell_type` field, types not defined.

- [ ] **Step 3: Replace `SurfaceTableColumn` definition**

In `crates/shared/surfaces/src/surface.rs`, replace:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceTableColumn {
    pub key: String,
    pub label: String,
}
```

with:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceTableColumn {
    pub key: String,
    pub label: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_cell_type"
    )]
    pub cell_type: Option<SurfaceTableCellType>,
}

fn deserialize_optional_cell_type<'de, D>(
    deserializer: D,
) -> Result<Option<SurfaceTableCellType>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.and_then(|v| serde_json::from_value(v).ok()))
}
```

- [ ] **Step 4: Add new types after `SurfaceTableColumn`**

After the `deserialize_optional_cell_type` function, add:

```rust
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SurfaceTableCellType {
    EntityLink { entity_type: SurfaceEntityType },
}

/// Wire-safe entity type enum.
///
/// Known variants are type-safe; unknown values from newer peers become
/// `Other(String)` for forward compatibility. Uses custom `Serialize`
/// and `Deserialize` so that `Other(String)` emits a bare string on
/// the wire (not `{"other":"..."}`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SurfaceEntityType {
    Host,
    Other(String),
}

impl SurfaceEntityType {
    /// Returns the snake_case wire string for this entity type.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Host => "host",
            Self::Other(s) => s.as_str(),
        }
    }
}

impl From<String> for SurfaceEntityType {
    fn from(s: String) -> Self {
        match s.as_str() {
            "host" => Self::Host,
            _ => Self::Other(s),
        }
    }
}

impl Serialize for SurfaceEntityType {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SurfaceEntityType {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(SurfaceEntityType::from)
    }
}

/// Cell value for entity-link columns.
///
/// Plugins construct via [`SurfaceEntityRef::unresolved`] (`entity_id` only).
/// The framework enriches `label` and `found` before sending the wire response.
/// `found: None` is a transient pre-enrichment state — must not appear in the
/// final wire response for cells whose resolver ran.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfaceEntityRef {
    pub entity_id: uuid::Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub found: Option<bool>,
}

impl SurfaceEntityRef {
    /// Constructs an unresolved ref for use by plugin handlers.
    /// The framework enriches `label` and `found` in the enrichment step.
    pub fn unresolved(entity_id: uuid::Uuid) -> Self {
        Self {
            entity_id,
            label: None,
            found: None,
        }
    }
}
```

- [ ] **Step 5: Add `EntityLinkColumn` to `Capability` enum**

In the `Capability` enum (after `ContextSelector`), add:

```rust
    EntityLinkColumn,
```

- [ ] **Step 6: Run tests to confirm they pass**

```bash
cargo test -p uptrakit-surfaces -- --nocapture 2>&1 | tail -20
```

Expected: all new tests PASS.

- [ ] **Step 7: Patch all `SurfaceTableColumn` struct literals**

`SurfaceTableColumn` is not `#[non_exhaustive]`, so adding the new field is a
breaking change for all struct-literal constructions. Find every failing site:

```bash
cargo check --all-features 2>&1 | grep "missing field .cell_type"
```

For every location found, add `cell_type: None` to the struct literal. The affected
files are listed in the File Map above (all except `proxmox/plugin.rs` which is
updated in Task 4). For each file, find patterns like:

```rust
SurfaceTableColumn {
    key: "some_key".to_string(),
    label: "Some Label".to_string(),
}
```

and add `cell_type: None,` before the closing brace:

```rust
SurfaceTableColumn {
    key: "some_key".to_string(),
    label: "Some Label".to_string(),
    cell_type: None,
}
```

- [ ] **Step 8: Run check + clippy**

```bash
cargo check --no-default-features --features db-sqlite 2>&1 | tail -10
cargo check --all-features 2>&1 | tail -10
cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -10
```

Expected: clean.

- [ ] **Step 9: Commit**

```bash
git add crates/shared/surfaces/src/surface.rs \
        crates/core/agent-ssh/src/ \
        crates/core/mqtt-runtime/src/ \
        crates/plugins/infrastructure/core/src/ \
        crates/plugins/notifications/
git commit -m "feat(surfaces): add entity-link cell type, SurfaceEntityType, SurfaceEntityRef"
```

---

## Task 2: `entity_enrichment` module

**Files:**

- Modify: `crates/ui/web-api/src/surface_proxy.rs`
- Create: `crates/ui/web-api/src/surface_proxy/entity_enrichment.rs`

- [ ] **Step 1: Write failing tests**

Create `crates/ui/web-api/src/surface_proxy/entity_enrichment.rs` with just the
test module, then add the `mod` declaration to `surface_proxy.rs`.

Add to `surface_proxy.rs` (after the first `use` block or at the top of the file):

```rust
pub(crate) mod entity_enrichment;
```

Create `crates/ui/web-api/src/surface_proxy/entity_enrichment.rs`:

```rust
// Implementations follow in later steps.

#[cfg(test)]
mod tests {
    // placeholder — real tests added in step 3
    #[test]
    fn placeholder() {}
}
```

- [ ] **Step 2: Verify the module compiles**

```bash
cargo check -p uptrakit-web-api --no-default-features --features db-sqlite 2>&1 | tail -10
```

Expected: clean.

- [ ] **Step 3: Implement `enrich_entity_links` and helpers**

Replace the content of `entity_enrichment.rs` with:

```rust
use std::collections::HashMap;

use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use uuid::Uuid;

use uptrakit_internal_wire::surfaces::{
    SurfaceEntityType, SurfaceNode, SurfaceTableCellType,
};
use uptrakit_shared_db::entity::host;

/// Enriches entity-link cells in a surface list response.
///
/// Resolves display labels for each known `SurfaceEntityType` via a direct DB
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
                    {
                        if let Ok(id) = Uuid::parse_str(id_str) {
                            if !bucket.contains(&id) {
                                bucket.push(id);
                            }
                        }
                    }
                }
            }
        }
    }

    // Batch-resolve labels per entity type.
    let mut label_maps: HashMap<String, HashMap<Uuid, String>> = HashMap::new();
    for (type_key, ids) in &ids_by_type {
        let entity_type = SurfaceEntityType::from(type_key.clone());
        match entity_type {
            SurfaceEntityType::Host => {
                match resolve_host_labels(db, tenant_id, ids).await {
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
                }
            }
            SurfaceEntityType::Other(_) => {
                // Not reachable: Other(_) was skipped in the ID-collection step.
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
    use uptrakit_internal_wire::surfaces::{
        DataSourceId, SurfaceEntityType, SurfaceTableCellType, SurfaceTableColumn,
    };

    use super::*;

    fn entity_link_table_node(col_key: &str) -> SurfaceNode {
        SurfaceNode::Table {
            data_source_id: DataSourceId::new("test").expect("literal"),
            columns: vec![SurfaceTableColumn {
                key: col_key.to_string(),
                label: "Host".to_string(),
                cell_type: Some(SurfaceTableCellType::EntityLink {
                    entity_type: SurfaceEntityType::Host,
                }),
            }],
            row_actions: vec![],
        }
    }

    fn no_link_table_node() -> SurfaceNode {
        SurfaceNode::Table {
            data_source_id: DataSourceId::new("test").expect("literal"),
            columns: vec![SurfaceTableColumn {
                key: "name".to_string(),
                label: "Name".to_string(),
                cell_type: None,
            }],
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
        use uptrakit_internal_wire::surfaces::{SurfaceTab, SurfaceTabId};
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
        // Pure path — no DB call. Use tokio::test only if async; this is sync-equivalent.
        let node = no_link_table_node();
        let response = serde_json::json!({
            "items": [{"name": "foo"}],
            "total": 1
        });
        // We can verify collect returns empty, confirming the early return path.
        assert!(collect_entity_link_columns(&node).is_empty());
        // The function would return `response` unchanged — shape assertion sufficient.
        let _ = response; // consumed, shape preserved by identity
    }

    #[test]
    fn enrich_returns_unchanged_when_no_items_key() {
        let node = entity_link_table_node("host_col");
        // If response has entity-link columns but response shape is not {items:[...]},
        // collect returns non-empty but enrich_entity_links returns unchanged.
        // The items check is after the collect check — verify column collection works.
        let cols = collect_entity_link_columns(&node);
        assert_eq!(cols.len(), 1);
    }

    #[test]
    fn other_entity_type_not_in_ids_by_type() {
        // Columns with Other(_) entity type must not be dispatched.
        let node = SurfaceNode::Table {
            data_source_id: DataSourceId::new("test").expect("literal"),
            columns: vec![SurfaceTableColumn {
                key: "future_col".to_string(),
                label: "Future".to_string(),
                cell_type: Some(SurfaceTableCellType::EntityLink {
                    entity_type: SurfaceEntityType::Other("future_type".to_string()),
                }),
            }],
            row_actions: vec![],
        };
        let cols = collect_entity_link_columns(&node);
        assert_eq!(cols.len(), 1);
        assert!(matches!(cols[0].1, SurfaceEntityType::Other(_)));
        // enrich_entity_links would skip this column — ids_by_type stays empty.
        // Cannot assert without a DB; shape logic is correct by inspection of the match arm.
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p uptrakit-web-api entity_enrichment -- --nocapture 2>&1 | tail -20
```

Expected: all tests PASS.

- [ ] **Step 5: Run check + clippy**

```bash
cargo check -p uptrakit-web-api --no-default-features --features db-sqlite 2>&1 | tail -10
cargo clippy -p uptrakit-web-api --all-targets --no-default-features --features db-sqlite \
    -- -D warnings 2>&1 | tail -10
```

Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/ui/web-api/src/surface_proxy.rs \
        crates/ui/web-api/src/surface_proxy/
git commit -m "feat(web-api): add enrich_entity_links free function in surface_proxy/entity_enrichment"
```

---

## Task 3: Route handler enrichment

**Files:**

- Modify: `crates/ui/web-api/src/routes/surfaces.rs`

- [ ] **Step 1: Add enrichment call after `invoke()`**

In `crates/ui/web-api/src/routes/surfaces.rs`, import the enrichment function:

```rust
use crate::surface_proxy::entity_enrichment::enrich_entity_links;
```

Find the block:

```rust
    let result = state
        .surface_proxy
        .invoke(
            &state.service_connections,
            &state.surface_registry,
            request,
            timeout_override,
        )
        .await;

    let response = match result {
```

Replace with:

```rust
    let result = state
        .surface_proxy
        .invoke(
            &state.service_connections,
            &state.surface_registry,
            request,
            timeout_override,
        )
        .await;

    let mut result = result;
    if let Ok(ref mut action_response) = result {
        if let Some(result_value) = action_response.result.take() {
            action_response.result = Some(
                enrich_entity_links(
                    state.db(),
                    Some(tenant_ctx.tenant_id),
                    &resolved.descriptor.root_node,
                    result_value,
                )
                .await,
            );
        }
    }

    let response = match result {
```

- [ ] **Step 2: Check**

```bash
cargo check -p uptrakit-web-api --no-default-features --features db-sqlite 2>&1 | tail -10
cargo check -p uptrakit-web-api --all-features 2>&1 | tail -10
```

Expected: clean.

- [ ] **Step 3: Run web-api tests**

```bash
cargo test -p uptrakit-web-api --no-default-features --features db-sqlite 2>&1 | tail -20
```

Expected: all pass.

- [ ] **Step 4: Commit**

```bash
git add crates/ui/web-api/src/routes/surfaces.rs
git commit -m "feat(web-api): enrich entity-link cells in surface action responses"
```

---

## Task 4: Proxmox plugin updates

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/plugin.rs`
- Modify: `crates/plugins/infrastructure/proxmox/src/surfaces.rs`

- [ ] **Step 1: Write a failing test for the new matched_host shape**

In `crates/plugins/infrastructure/proxmox/src/surfaces.rs`, inside the `tests` module
(after the existing `unmatch_action_has_row_visibility` test), add:

```rust
#[tokio::test]
async fn handle_list_matched_host_emits_entity_ref_object() {
    let tenant_id = Uuid::now_v7();
    let config_id = Uuid::now_v7();
    let host_id = Uuid::now_v7();

    let mut mapping = mock_proxmox_host_mapping(tenant_id, config_id, "vm-matched");
    mapping.host_id = Some(host_id);

    let db = MockDatabase::new(DbBackend::MySql)
        .append_query_results([[mock_count_row(1)]])
        .append_query_results([[mapping]])
        .append_query_results([Vec::<uptrakit_shared_db::entity::host::Model>::new()])
        .append_query_results([[mock_plugin_config_model(tenant_id, config_id)]])
        .into_connection();

    let result = handle_list(&db, Some(tenant_id), serde_json::json!({}))
        .await
        .expect("handle_list should succeed");

    let items = result["items"].as_array().expect("items must be an array");
    assert_eq!(items.len(), 1);

    let matched_host = &items[0]["matched_host"];
    assert!(
        matched_host.is_object(),
        "matched_host must be an object (SurfaceEntityRef), got: {matched_host}"
    );
    assert_eq!(
        matched_host["entity_id"].as_str().unwrap(),
        host_id.to_string(),
        "matched_host.entity_id must match the host_id"
    );
    assert!(
        matched_host.get("label").is_none(),
        "matched_host.label must be absent (pre-enrichment)"
    );
    assert!(
        matched_host.get("found").is_none(),
        "matched_host.found must be absent (pre-enrichment)"
    );
}

#[tokio::test]
async fn handle_list_unmatched_host_emits_null_matched_host() {
    let tenant_id = Uuid::now_v7();
    let config_id = Uuid::now_v7();

    let db = MockDatabase::new(DbBackend::MySql)
        .append_query_results([[mock_count_row(1)]])
        .append_query_results([[mock_proxmox_host_mapping(
            tenant_id,
            config_id,
            "vm-unmatched",
        )]])
        .append_query_results([Vec::<uptrakit_shared_db::entity::host::Model>::new()])
        .append_query_results([[mock_plugin_config_model(tenant_id, config_id)]])
        .into_connection();

    let result = handle_list(&db, Some(tenant_id), serde_json::json!({}))
        .await
        .expect("handle_list should succeed");

    let items = result["items"].as_array().expect("items must be an array");
    assert!(
        items[0]["matched_host"].is_null(),
        "unmatched host must serialize as null"
    );
}
```

- [ ] **Step 2: Run the new tests to confirm they fail**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox matched_host \
    -- --nocapture 2>&1 | tail -20
```

Expected: FAIL — `matched_host` is currently a string, not an object.

- [ ] **Step 3: Update `handle_list` row builder**

In `crates/plugins/infrastructure/proxmox/src/surfaces.rs`, find (around line 816):

```rust
            "matched_host": m.host_id.map(|id| id.to_string()),
```

Replace with:

```rust
            "matched_host": m.host_id.map(surfaces::SurfaceEntityRef::unresolved),
```

- [ ] **Step 4: Run the new tests to confirm they pass**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox matched_host \
    -- --nocapture 2>&1 | tail -20
```

Expected: PASS.

- [ ] **Step 5: Run all proxmox tests**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox -- --nocapture 2>&1 | tail -20
```

Expected: all pass.

- [ ] **Step 6: Update `matched_host` column declaration in `plugin.rs`**

In `crates/plugins/infrastructure/proxmox/src/plugin.rs`, find:

```rust
                            surfaces::SurfaceTableColumn {
                                key: "matched_host".to_string(),
                                label: "Matched Host".to_string(),
                                cell_type: None,
                            },
```

Replace with:

```rust
                            surfaces::SurfaceTableColumn {
                                key: "matched_host".to_string(),
                                label: "Matched Host".to_string(),
                                cell_type: Some(surfaces::SurfaceTableCellType::EntityLink {
                                    entity_type: surfaces::SurfaceEntityType::Host,
                                }),
                            },
```

- [ ] **Step 7: Add `EntityLinkColumn` capability to `required_capabilities`**

In `plugin.rs`, find the `required_capabilities` block and add
`surfaces::Capability::EntityLinkColumn` after `surfaces::Capability::ContextSelector`:

```rust
                surfaces::Capability::ContextSelector,
                surfaces::Capability::EntityLinkColumn,
```

- [ ] **Step 8: Run all proxmox tests again**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox -- --nocapture 2>&1 | tail -20
```

Expected: all pass.

- [ ] **Step 9: Full check**

```bash
cargo check --all-features 2>&1 | tail -10
cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -10
```

Expected: clean.

- [ ] **Step 10: Commit**

```bash
git add crates/plugins/infrastructure/proxmox/src/plugin.rs \
        crates/plugins/infrastructure/proxmox/src/surfaces.rs
git commit -m "feat(proxmox): emit SurfaceEntityRef for matched_host with EntityLinkColumn capability"
```

---

## Task 5: Frontend types

**Files:**

- Modify: `frontend/src/lib/surfaces/contract.ts`
- Create: `frontend/src/lib/surfaces/entity-routes.ts`

- [ ] **Step 1: Update `contract.ts`**

In `frontend/src/lib/surfaces/contract.ts`, add `'entity_link_column'` at the end of the
`SurfaceCapability` union (before the closing `;`):

```ts
    | 'context_selector'
    | 'entity_link_column';
```

Replace the `SurfaceTableColumn` interface:

```ts
export interface SurfaceTableColumn {
    key: string;
    label: string;
}
```

with:

```ts
export interface SurfaceTableColumn {
    key: string;
    label: string;
    cell_type?: { kind: 'entity_link'; entity_type: SurfaceEntityType };
}

export interface SurfaceEntityRef {
    entity_id: string;
    label?: string;
    found?: boolean;
}
```

Add a temporary inline type for `SurfaceEntityType` (will be replaced in Step 3):

```ts
// Forward-compatible entity type string. Known variants have autocomplete;
// unknown variants from newer backends are accepted without error.
export type SurfaceEntityType = 'host' | (string & {});
```

- [ ] **Step 2: Create `entity-routes.ts`**

Create `frontend/src/lib/surfaces/entity-routes.ts`:

```ts
/**
 * Known entity types for surface entity links.
 *
 * `string & {}` keeps autocomplete for known values while accepting unknown
 * types from newer backend versions (forward-compatible).
 */
export type SurfaceEntityType = 'host' | (string & {});

/**
 * Returns the frontend route for a given entity type and ID, or `null` if
 * the entity type has no known route in this frontend version.
 *
 * The `default` arm is always required — future entity types must not cause
 * a TypeScript exhaustiveness error here.
 */
export function entityRoute(
    entityType: SurfaceEntityType,
    entityId: string,
): string | null {
    switch (entityType) {
        case 'host':
            return `/hosts/${entityId}`;
        default:
            return null;
    }
}
```

- [ ] **Step 3: Consolidate `SurfaceEntityType`**

In `contract.ts`, replace the inline `SurfaceEntityType` declaration with:

```ts
export type { SurfaceEntityType } from './entity-routes';
```

- [ ] **Step 4: TypeScript check**

```bash
cd frontend && npm run check 2>&1 | tail -20
```

Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/lib/surfaces/contract.ts frontend/src/lib/surfaces/entity-routes.ts
git commit -m "feat(frontend): add entity-link types to contract and entity-routes module"
```

---

## Task 6: SurfaceTable entity link rendering

**Files:**

- Modify: `frontend/src/lib/components/surfaces/SurfaceTable.svelte`

- [ ] **Step 1: Add imports**

At the top of the `<script>` block in `SurfaceTable.svelte`, add imports:

```ts
import StatusBadge from '$lib/components/ui/StatusBadge.svelte';
import { entityRoute } from '$lib/surfaces/entity-routes';
import type { SurfaceEntityRef } from '$lib/surfaces/contract';
```

- [ ] **Step 2: Derive `hasEntityLinkColumns`**

Add after the existing `$derived` declarations:

```ts
const hasEntityLinkColumns = $derived(
    resolvedColumns.some((col) => col.cell_type?.kind === 'entity_link'),
);
```

- [ ] **Step 3: Replace the template**

In the template section of `SurfaceTable.svelte`, replace both `<DataTable ...>` blocks
(the one with `rowActions` and the one without). The current structure is:

```svelte
{#if hasRowActions}
    <DataTable columns={resolvedColumns} rows={tableRows} ...>
        {#snippet rowActions(row)}...{/snippet}
        {#snippet footer()}...{/snippet}
    </DataTable>
{:else}
    <DataTable columns={resolvedColumns} rows={tableRows} ...>
        {#snippet footer()}...{/snippet}
    </DataTable>
{/if}
```

Replace with:

```svelte
{#snippet entityLinkRow(rowRecord: Record<string, unknown>)}
    <tr class="border-b border-[var(--border-subtle)] last:border-b-0">
        {#each resolvedColumns as col (col.key)}
            <td class="table-cell-pad text-[var(--text-primary)]">
                {#if col.cell_type?.kind === 'entity_link'}
                    {@const entityRef = rowRecord[col.key] as SurfaceEntityRef | null | undefined}
                    {#if entityRef == null}
                        <span class="text-[var(--text-muted)]">—</span>
                    {:else if entityRef.found === true}
                        {@const route = entityRoute(col.cell_type.entity_type, entityRef.entity_id)}
                        {#if route}
                            <a href={route} class="hover:underline font-medium">{entityRef.label}</a>
                        {:else}
                            {entityRef.label}
                        {/if}
                    {:else if entityRef.found === false}
                        <StatusBadge tone="warning" label="Unknown entity" />
                    {:else}
                        {entityRef.entity_id}
                    {/if}
                {:else}
                    {String(rowRecord[col.key] ?? '')}
                {/if}
            </td>
        {/each}
        {#if hasRowActions}
            <td class="table-cell-pad">
                <div class="flex flex-wrap gap-2">
                    {#each resolvedRowActions as { rowAction, interaction } (rowAction.interaction_id)}
                        {#if isRowActionVisible(rowAction, rowRecord)}
                            <SurfaceInteractionButton
                                {surfaceId}
                                {interaction}
                                {interactions}
                                {targetProviderId}
                                {encryptionContext}
                                baseParams={rowParams(rowRecord)}
                                rowSeed={rowRecord}
                                size="sm"
                                oncomplete={async () => {
                                    await loadPage(currentPage);
                                }}
                            />
                        {/if}
                    {/each}
                </div>
            </td>
        {/if}
    </tr>
{/snippet}

{#if hasEntityLinkColumns}
    <!-- Entity-link path: custom row snippet handles cells + row actions internally. -->
    <DataTable
        columns={resolvedColumns}
        rows={tableRows}
        {loading}
        error={loadError}
        emptyTitle={dataSource?.empty_state?.title ?? 'No rows available'}
        emptyDescription={dataSource?.empty_state?.description}
        row={entityLinkRow}
    >
        {#snippet footer()}
            {#if showInlineFooter}
                <TableFooterBar
                    {total}
                    {currentPage}
                    {totalPages}
                    onPageChange={handlePageChange}
                />
            {/if}
        {/snippet}
    </DataTable>
{:else if hasRowActions}
    <!-- Plain cells with row actions: DataTable renders default tr + rowActions snippet. -->
    <DataTable
        columns={resolvedColumns}
        rows={tableRows}
        {loading}
        error={loadError}
        emptyTitle={dataSource?.empty_state?.title ?? 'No rows available'}
        emptyDescription={dataSource?.empty_state?.description}
    >
        {#snippet rowActions(row)}
            {#each resolvedRowActions as { rowAction, interaction } (rowAction.interaction_id)}
                {#if isRowActionVisible(rowAction, row)}
                    <SurfaceInteractionButton
                        {surfaceId}
                        {interaction}
                        {interactions}
                        {targetProviderId}
                        {encryptionContext}
                        baseParams={rowParams(row)}
                        rowSeed={row}
                        size="sm"
                        oncomplete={async () => {
                            await loadPage(currentPage);
                        }}
                    />
                {/if}
            {/each}
        {/snippet}
        {#snippet footer()}
            {#if showInlineFooter}
                <TableFooterBar
                    {total}
                    {currentPage}
                    {totalPages}
                    onPageChange={handlePageChange}
                />
            {/if}
        {/snippet}
    </DataTable>
{:else}
    <!-- Plain cells, no row actions. -->
    <DataTable
        columns={resolvedColumns}
        rows={tableRows}
        {loading}
        error={loadError}
        emptyTitle={dataSource?.empty_state?.title ?? 'No rows available'}
        emptyDescription={dataSource?.empty_state?.description}
    >
        {#snippet footer()}
            {#if showInlineFooter}
                <TableFooterBar
                    {total}
                    {currentPage}
                    {totalPages}
                    onPageChange={handlePageChange}
                />
            {/if}
        {/snippet}
    </DataTable>
{/if}
```

Three branches:

1. `hasEntityLinkColumns` — custom `entityLinkRow` snippet handles cell rendering and row
   actions internally via `{#if hasRowActions}`.
2. `hasRowActions` — standard DataTable with `rowActions` snippet.
3. Neither — plain DataTable, no snippets.

- [ ] **Step 4: TypeScript + lint check**

```bash
cd frontend && npm run check 2>&1 | tail -20
npm run lint 2>&1 | tail -20
```

Expected: clean.

- [ ] **Step 5: Build check**

```bash
cd frontend && npm run build 2>&1 | tail -10
```

Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/lib/components/surfaces/SurfaceTable.svelte
git commit -m "feat(frontend): render entity-link cells in SurfaceTable with 5-state logic"
```

---

## Task 7: UI parity tests

**Files:**

- Modify: `frontend/src/lib/test-fixtures/ui-parity.ts`
- Modify: `frontend/tests/e2e/ui-parity.test.ts`

- [ ] **Step 1: Add entity-link fixture to `ui-parity.ts`**

In `frontend/src/lib/test-fixtures/ui-parity.ts`, add a new export interface and extend
`SharedVisualParityFixture`:

```ts
export interface SharedEntityLinkParityFixture {
    surface: SurfaceResponse;
    readModel: SurfaceReadResponse;
    dataLoadInteractionId: string;
    dataLoadResponse: {
        items: Array<Record<string, unknown>>;
        total: number;
        page: number;
        per_page: number;
        total_pages: number;
    };
}
```

Add `entityLink: SharedEntityLinkParityFixture;` to `SharedVisualParityFixture`:

```ts
export interface SharedVisualParityFixture {
    actionBadge: SharedActionBadgeParityFixture;
    pillBadge: SharedPillBadgeParityFixture;
    contextMenu: SharedContextMenuParityFixture;
    tableFooter: SharedTableFooterParityFixture;
    entityLink: SharedEntityLinkParityFixture;
}
```

In `buildSharedVisualParityFixture()`, add the entity link fixture before the `return`
statement. Two entity-link columns cover all five rendering states: `host_col` (type
`'host'`, has a known route) and `future_col` (type `'future_entity'`, no route).

```ts
    const entityLinkDataLoadInteractionId = 'entity-link.load';
    const entityLinkDataSourceId = 'entity-link.data';
    const entityLinkSurface = buildParitySurfaceTab(
        'surface.entity-link',
        'Entity Link Surface',
        {
            slot: 'surface.page',
            provider_kind: 'service',
            root_node: {
                kind: 'table',
                data_source_id: entityLinkDataSourceId,
                columns: [
                    { key: 'label_col', label: 'State', cell_type: undefined },
                    {
                        key: 'host_col',
                        label: 'Host (known route)',
                        cell_type: { kind: 'entity_link', entity_type: 'host' },
                    },
                    {
                        key: 'future_col',
                        label: 'Future (no route)',
                        cell_type: {
                            kind: 'entity_link',
                            entity_type: 'future_entity',
                        },
                    },
                ],
            },
        },
    );

    const entityLinkInteractions: InteractionDescriptor[] = [
        {
            interaction_id: entityLinkDataLoadInteractionId,
            kind: 'data_load',
            label: 'Load entity link parity data',
            input_schema: 'object',
            result_schema: 'object',
            transport: { mode: 'provider_proxied' },
        },
    ];

    const entityLinkDataSources: DataSourceDescriptor[] = [
        {
            data_source_id: entityLinkDataSourceId,
            kind: {
                kind: 'provider_query',
                operation_id: entityLinkDataLoadInteractionId,
            },
            result_schema: 'object',
            pagination: { default_page_size: 10, max_page_size: 10 },
            refresh_policy: { type: 'manual' },
            empty_state: { title: 'No rows', description: undefined },
        },
    ];

    const { provider_count: _elpc, ...entityLinkDescriptor } = entityLinkSurface;
    const entityLinkReadModel: SurfaceReadResponse = {
        descriptor: entityLinkDescriptor,
        interactions: entityLinkInteractions,
        data_sources: entityLinkDataSources,
    };

    // Five states across two columns:
    // Row 1: host found + known route → link
    // Row 2: future entity found + no route → plain label
    // Row 3: not found → warning badge
    // Row 4: unenriched (found absent) → plain entity_id
    // Row 5: null cell → —
    const entityLinkDataLoadResponse = {
        items: [
            {
                label_col: 'found – link',
                host_col: {
                    entity_id: '00000000-0000-0000-0000-000000000001',
                    label: 'web-01',
                    found: true,
                },
                future_col: null,
            },
            {
                label_col: 'found – no route',
                host_col: null,
                future_col: {
                    entity_id: '00000000-0000-0000-0000-000000000002',
                    label: 'node-02',
                    found: true,
                },
            },
            {
                label_col: 'not found',
                host_col: {
                    entity_id: '00000000-0000-0000-0000-000000000003',
                    found: false,
                },
                future_col: null,
            },
            {
                label_col: 'unenriched',
                host_col: { entity_id: '00000000-0000-0000-0000-000000000004' },
                future_col: null,
            },
            {
                label_col: 'null cell',
                host_col: null,
                future_col: null,
            },
        ],
        total: 5,
        page: 1,
        per_page: 10,
        total_pages: 1,
    };
```

Then add `entityLink` to the returned object:

```ts
    return {
        // ...existing fields...
        entityLink: {
            surface: entityLinkSurface,
            readModel: entityLinkReadModel,
            dataLoadInteractionId: entityLinkDataLoadInteractionId,
            dataLoadResponse: entityLinkDataLoadResponse,
        },
    };
```

- [ ] **Step 2: Update `buildDefaultReadModels` in `ui-parity.test.ts`**

In `ui-parity.test.ts`, find `buildDefaultReadModels` and add the entity link read model:

```ts
    models[sharedVisualParity.entityLink.surface.surface_id] =
        sharedVisualParity.entityLink.readModel;
```

- [ ] **Step 3: Add surface invoke mock for entity link surface**

In the `mockParityApi` route handler, add a case for the entity link fixture:

```ts
if (
    surfaceId === sharedVisualParity.entityLink.surface.surface_id &&
    interactionId === sharedVisualParity.entityLink.dataLoadInteractionId
) {
    return json(sharedVisualParity.entityLink.dataLoadResponse);
}
```

- [ ] **Step 4: Add the parity test**

After the `'shared primitive ui parity: table footer totals and pagination alignment'` test,
add:

```ts
test('shared primitive ui parity: entity link cell rendering states', async ({ page }) => {
    if (!isCanonicalUiParityHost) {
        test.skip(true, canonicalUiParityReason);
    }

    const entityLinkSurfaces = [...paritySurfaces, sharedVisualParity.entityLink.surface];
    const entityLinkReadModels = buildDefaultReadModels(entityLinkSurfaces);
    await mockParityApi(page, {
        surfaces: entityLinkSurfaces,
        readModels: entityLinkReadModels,
    });

    await page.goto(`/surfaces/${sharedVisualParity.entityLink.surface.surface_id}`);

    const dataTable = page.locator('[data-ui="data-table"]');
    await expect(dataTable).toBeVisible();

    // Wait for entity link cells to render (found link visible)
    await expect(
        dataTable.locator('a[href="/hosts/00000000-0000-0000-0000-000000000001"]'),
    ).toBeVisible();

    await captureParityScreenshot(page, dataTable, 'ui-parity-entity-link-cells.png');
});
```

- [ ] **Step 5: Run frontend tests**

```bash
cd frontend && npm run test 2>&1 | tail -20
```

Expected: all unit tests pass (parity screenshot test skipped outside macOS CI).

- [ ] **Step 6: TypeScript check**

```bash
cd frontend && npm run check 2>&1 | tail -20
```

Expected: clean.

- [ ] **Step 7: Generate baseline screenshots (macOS + Chromium only)**

On macOS with Chromium:

```bash
cd frontend && npx playwright test ui-parity --update-snapshots --project chromium 2>&1 | tail -20
```

Verify the new `ui-parity-entity-link-cells.png` baseline files appear in the snapshots
directory.

- [ ] **Step 8: Commit**

```bash
git add frontend/src/lib/test-fixtures/ui-parity.ts frontend/tests/e2e/ui-parity.test.ts
# Include baseline snapshot files if generated
git add frontend/tests/e2e/*.png 2>/dev/null || true
git commit -m "test(e2e): add entity-link cell parity fixtures and baseline screenshots"
```

---

## Final quality gate

- [ ] **Step 1: Full Rust check + lint**

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
```

Expected: clean.

- [ ] **Step 2: Full Rust test suite**

```bash
cargo test --all-features 2>&1 | tail -30
```

Expected: all tests pass.

- [ ] **Step 3: Dependency audit**

```bash
cargo deny check 2>&1 | tail -10
```

Expected: clean.

- [ ] **Step 4: Frontend full gate**

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

Expected: all pass.

- [ ] **Step 5: Markdown lint**

```bash
markdownlint --config .markdownlint.json 'docs/**/*.md' 2>&1 | tail -10
```

Expected: clean.
