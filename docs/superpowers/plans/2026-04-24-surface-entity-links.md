# Surface Entity Links Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Allow surface table columns to declare entity-link cell types; the framework resolves
labels from the DB and the frontend renders clickable links.

**Architecture:** Wire types in `uptrakit-surfaces` declare the column cell type and entity
reference shape. A new `EntityResolverRegistry` in `uptrakit-web-api` resolves display labels
in a single batch DB query per entity type after the proxy handler returns. The frontend
`SurfaceTable.svelte` renders entity-link cells according to a five-state table.

**Tech Stack:** Rust (sea-orm, serde, async-trait, uuid, rootcause), SvelteKit + TypeScript, Playwright parity tests.

---

## File Map

| Path | Action | Responsibility |
| --- | --- | --- |
| `crates/shared/surfaces/src/surface.rs` | Modify | Add `cell_type` to `SurfaceTableColumn`, add `SurfaceTableCellType`, `SurfaceEntityType`, `SurfaceEntityRef`, `Capability::EntityLinkColumn` |
| `crates/ui/web-api/src/entity_resolvers.rs` | Create | `EntityResolver` trait, `EntityResolverRegistry`, `HostEntityResolver`, `enrich_surface_response` |
| `crates/ui/web-api/src/lib.rs` | Modify | Declare `pub mod entity_resolvers` |
| `crates/ui/web-api/src/app_state.rs` | Modify | Add `entity_resolver_registry: Arc<EntityResolverRegistry>` field + builder method |
| `crates/core/controller/src/main.rs` | Modify | Build `EntityResolverRegistry`, register `HostEntityResolver`, pass to builder |
| `crates/ui/web-api/src/routes/surfaces.rs` | Modify | Call `enrich_surface_response` after `invoke()` succeeds |
| `crates/plugins/infrastructure/proxmox/src/plugin.rs` | Modify | Add `cell_type` to `matched_host` column + `Capability::EntityLinkColumn` |
| `crates/plugins/infrastructure/proxmox/src/surfaces.rs` | Modify | Emit `SurfaceEntityRef::unresolved` for `matched_host` + update/add tests |
| `frontend/src/lib/surfaces/contract.ts` | Modify | Add `cell_type` to `SurfaceTableColumn`, add `SurfaceEntityRef`, add `'entity_link_column'` to `SurfaceCapability` |
| `frontend/src/lib/surfaces/entity-routes.ts` | Create | `SurfaceEntityType`, `entityRoute()` |
| `frontend/src/lib/components/surfaces/SurfaceTable.svelte` | Modify | Entity-link cell rendering (5-case table) |
| `frontend/src/lib/test-fixtures/ui-parity.ts` | Modify | Add `entityLinkTable` fixture to `SharedVisualParityFixture` |
| `frontend/tests/e2e/ui-parity.test.ts` | Modify | Add entity-link parity test |

---

## Task 1: Wire types in `surface.rs`

**Files:**

- Modify: `crates/shared/surfaces/src/surface.rs`

- [ ] **Step 1: Write failing tests**

Add at end of the `tests` module in `surface.rs` (after the existing `surface_descriptor_context_selector_round_trips` test):

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
    let json = r#"{"key":"host","label":"Host","cell_type":{"kind":"future_type","extra":"data"}}"#;
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
    let t: SurfaceEntityType = serde_json::from_str(r#""unknown_type""#).expect("deserialize");
    assert_eq!(t, SurfaceEntityType::Other("unknown_type".to_string()));
}

#[test]
fn surface_entity_ref_unresolved_serializes_without_label_or_found() {
    let entity_id = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
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
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
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

- [ ] **Step 7: Run check + clippy**

```bash
cargo check --no-default-features --features db-sqlite -p uptrakit-surfaces 2>&1 | tail -10
cargo clippy -p uptrakit-surfaces --all-targets -- -D warnings 2>&1 | tail -10
```

Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add crates/shared/surfaces/src/surface.rs
git commit -m "feat(surfaces): add entity-link cell type, SurfaceEntityType, SurfaceEntityRef"
```

---

## Task 2: Entity resolver module

**Files:**

- Create: `crates/ui/web-api/src/entity_resolvers.rs`
- Modify: `crates/ui/web-api/src/lib.rs`

- [ ] **Step 1: Write failing tests**

Create `crates/ui/web-api/src/entity_resolvers.rs` with test stubs only — the module starts with just the test module so cargo test can run:

```rust
// Module scaffolding — implementations follow in later steps.

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    // placeholder — real tests added in step 3
    #[test]
    fn placeholder() {}
}
```

Add `pub mod entity_resolvers;` to `crates/ui/web-api/src/lib.rs` after the `surface_proxy` line.

- [ ] **Step 2: Verify the module compiles**

```bash
cargo check -p uptrakit-web-api --no-default-features --features db-sqlite 2>&1 | tail -10
```

Expected: clean.

- [ ] **Step 3: Implement `EntityResolver` trait, registry, and `HostEntityResolver`**

Replace the content of `entity_resolvers.rs` with:

```rust
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use uuid::Uuid;

use uptrakit_internal_wire::surfaces::{SurfaceEntityType, SurfaceNode, SurfaceTableCellType};
use uptrakit_shared_db::entity::host;

/// Resolves display labels for a specific entity type.
#[async_trait]
pub trait EntityResolver: Send + Sync {
    fn entity_type(&self) -> SurfaceEntityType;

    /// Resolve display labels for the given entity IDs.
    ///
    /// Returns a map of entity ID → display label for all IDs that exist.
    /// IDs absent from the map are treated as deleted/unknown and receive
    /// `found: false` in the enriched response.
    ///
    /// On DB error, return `Err`. The enrichment step treats all IDs for
    /// this entity type as unresolvable (`found: false`).
    async fn resolve_labels(
        &self,
        db: &DatabaseConnection,
        tenant_id: Option<Uuid>,
        ids: &[Uuid],
    ) -> Result<HashMap<Uuid, String>, Report>;
}

/// Maps `SurfaceEntityType → Box<dyn EntityResolver>`.
/// Populated at startup; static for the application lifetime.
#[derive(Default)]
pub struct EntityResolverRegistry {
    resolvers: HashMap<String, Box<dyn EntityResolver>>,
}

impl EntityResolverRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a resolver for its declared entity type.
    pub fn register(&mut self, resolver: Box<dyn EntityResolver>) {
        self.resolvers
            .insert(resolver.entity_type().as_str().to_string(), resolver);
    }

    /// Look up a resolver by entity type.
    pub fn get(&self, entity_type: &SurfaceEntityType) -> Option<&dyn EntityResolver> {
        self.resolvers.get(entity_type.as_str()).map(|r| r.as_ref())
    }
}

/// Resolves display labels for `host` entity references.
///
/// Label selection order: `friendly_name` (if non-empty) → `hostname` → UUID string.
pub struct HostEntityResolver;

#[async_trait]
impl EntityResolver for HostEntityResolver {
    fn entity_type(&self) -> SurfaceEntityType {
        SurfaceEntityType::Host
    }

    async fn resolve_labels(
        &self,
        db: &DatabaseConnection,
        tenant_id: Option<Uuid>,
        ids: &[Uuid],
    ) -> Result<HashMap<Uuid, String>, Report> {
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
        let map = hosts
            .into_iter()
            .map(|h| {
                let label = if !h.friendly_name.is_empty() {
                    h.friendly_name.clone()
                } else if !h.hostname.is_empty() {
                    h.hostname.clone()
                } else {
                    h.id.to_string()
                };
                (h.id, label)
            })
            .collect();
        Ok(map)
    }
}

/// Enrich entity-link cells in a surface action response.
///
/// Walks the surface node tree, finds entity-link columns, collects entity IDs
/// from the response items, batch-resolves labels, and rewrites the JSON in place.
///
/// Only applies to responses with shape `{ "items": [...] }`. Other shapes are
/// returned unchanged. Cells for unregistered entity types are not touched.
pub async fn enrich_surface_response(
    response: &mut uptrakit_internal_wire::surfaces::SurfaceActionResponse,
    root_node: &SurfaceNode,
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    registry: &EntityResolverRegistry,
) {
    let entity_link_columns = collect_entity_link_columns(root_node);
    if entity_link_columns.is_empty() {
        return;
    }

    let result = match response.result.as_mut() {
        Some(v) => v,
        None => return,
    };
    let items = match result.get_mut("items").and_then(|v| v.as_array_mut()) {
        Some(arr) => arr,
        None => return,
    };
    if items.is_empty() {
        return;
    }

    // Collect unique entity IDs per entity type (only for registered resolvers).
    let mut ids_by_type: HashMap<String, Vec<Uuid>> = HashMap::new();
    for (col_key, entity_type) in &entity_link_columns {
        if registry.get(entity_type).is_none() {
            continue;
        }
        let type_key = entity_type.as_str().to_string();
        let bucket = ids_by_type.entry(type_key).or_default();
        for item in items.iter() {
            let entity_id_str = item
                .get(col_key)
                .and_then(|cell| cell.get("entity_id"))
                .and_then(|v| v.as_str());
            if let Some(s) = entity_id_str {
                if let Ok(id) = Uuid::parse_str(s) {
                    if !bucket.contains(&id) {
                        bucket.push(id);
                    }
                }
            }
        }
    }

    // Batch-resolve labels per entity type.
    let mut label_maps: HashMap<String, HashMap<Uuid, String>> = HashMap::new();
    for (type_key, ids) in &ids_by_type {
        let entity_type = SurfaceEntityType::from(type_key.clone());
        let resolver = match registry.get(&entity_type) {
            Some(r) => r,
            None => continue,
        };
        match resolver.resolve_labels(db, tenant_id, ids).await {
            Ok(map) => {
                label_maps.insert(type_key.clone(), map);
            }
            Err(err) => {
                tracing::error!(
                    error = %err,
                    entity_type = %type_key,
                    "entity label resolution failed; marking cells as not found"
                );
                label_maps.insert(type_key.clone(), HashMap::new());
            }
        }
    }

    // Rewrite items in place for all registered entity-link columns.
    for item in items.iter_mut() {
        let obj = match item.as_object_mut() {
            Some(o) => o,
            None => continue,
        };
        for (col_key, entity_type) in &entity_link_columns {
            let type_key = entity_type.as_str().to_string();
            let label_map = match label_maps.get(&type_key) {
                Some(m) => m,
                None => continue, // unregistered type — skip
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
                if let Some(SurfaceTableCellType::EntityLink { entity_type }) = &col.cell_type {
                    out.push((col.key.clone(), entity_type.clone()));
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
    use super::*;
    use uptrakit_internal_wire::surfaces::{SurfaceActionResponse, SurfaceTableColumn};
    use uuid::Uuid;

    fn make_response(items: serde_json::Value) -> SurfaceActionResponse {
        SurfaceActionResponse {
            request_id: Uuid::now_v7(),
            success: true,
            result: Some(serde_json::json!({ "items": items })),
            error: None,
        }
    }

    fn entity_link_table_node(col_key: &str) -> SurfaceNode {
        SurfaceNode::Table {
            data_source_id: uptrakit_internal_wire::surfaces::DataSourceId::new("test")
                .expect("literal"),
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
            data_source_id: uptrakit_internal_wire::surfaces::DataSourceId::new("test")
                .expect("literal"),
            columns: vec![SurfaceTableColumn {
                key: "name".to_string(),
                label: "Name".to_string(),
                cell_type: None,
            }],
            row_actions: vec![],
        }
    }

    struct StubResolver {
        labels: HashMap<Uuid, String>,
    }

    #[async_trait::async_trait]
    impl EntityResolver for StubResolver {
        fn entity_type(&self) -> SurfaceEntityType {
            SurfaceEntityType::Host
        }

        async fn resolve_labels(
            &self,
            _db: &DatabaseConnection,
            _tenant_id: Option<Uuid>,
            ids: &[Uuid],
        ) -> Result<HashMap<Uuid, String>, rootcause::Report> {
            Ok(ids
                .iter()
                .filter_map(|id| self.labels.get(id).map(|l| (*id, l.clone())))
                .collect())
        }
    }

    fn registry_with_stub(labels: HashMap<Uuid, String>) -> EntityResolverRegistry {
        let mut reg = EntityResolverRegistry::new();
        reg.register(Box::new(StubResolver { labels }));
        reg
    }

    // We can't easily test enrich_surface_response without a real DB connection.
    // The collect_entity_link_columns helper is tested directly.

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
    fn entity_resolver_registry_get_known_type() {
        let reg = registry_with_stub(HashMap::new());
        assert!(reg.get(&SurfaceEntityType::Host).is_some());
    }

    #[test]
    fn entity_resolver_registry_get_unknown_type_returns_none() {
        let reg = registry_with_stub(HashMap::new());
        assert!(reg
            .get(&SurfaceEntityType::Other("future_type".to_string()))
            .is_none());
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p uptrakit-web-api entity_resolvers -- --nocapture 2>&1 | tail -20
```

Expected: all tests PASS (no DB-dependent tests in this task).

- [ ] **Step 5: Run check + clippy**

```bash
cargo check -p uptrakit-web-api --no-default-features --features db-sqlite 2>&1 | tail -10
cargo clippy -p uptrakit-web-api --all-targets --no-default-features --features db-sqlite -- -D warnings 2>&1 | tail -10
```

Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/ui/web-api/src/entity_resolvers.rs crates/ui/web-api/src/lib.rs
git commit -m "feat(web-api): add EntityResolverRegistry, HostEntityResolver, enrich_surface_response"
```

---

## Task 3: AppState + controller registration

**Files:**

- Modify: `crates/ui/web-api/src/app_state.rs`
- Modify: `crates/core/controller/src/main.rs`

- [ ] **Step 1: Add field to `AppState`**

In `crates/ui/web-api/src/app_state.rs`, add the import near the top (with other `crate::` imports):

```rust
use crate::entity_resolvers::EntityResolverRegistry;
```

In the `AppState` struct, after the `surface_proxy` field, add:

```rust
    /// Registry of entity resolvers for enriching entity-link cells in surface responses.
    pub entity_resolver_registry: Arc<EntityResolverRegistry>,
```

- [ ] **Step 2: Add field to `AppStateBuilder`**

In the `AppStateBuilder` struct, after the `surface_proxy` field, add:

```rust
    entity_resolver_registry: Option<Arc<EntityResolverRegistry>>,
```

In `AppStateBuilder::default()` (inside the `impl Default for AppStateBuilder` or the `fn new()` equivalent), initialize the field:

```rust
    entity_resolver_registry: None,
```

- [ ] **Step 3: Add builder setter method**

In `impl AppStateBuilder`, after the `surface_proxy` setter, add:

```rust
    /// Override the entity resolver registry.
    ///
    /// Optional — defaults to a registry with only `HostEntityResolver` registered.
    pub fn entity_resolver_registry(mut self, v: Arc<EntityResolverRegistry>) -> Self {
        self.entity_resolver_registry = Some(v);
        self
    }
```

- [ ] **Step 4: Wire into `build()`**

In `AppStateBuilder::build()`, after the `surface_proxy` field assignment, add:

```rust
            entity_resolver_registry: self.entity_resolver_registry.unwrap_or_else(|| {
                let mut reg = EntityResolverRegistry::new();
                reg.register(Box::new(crate::entity_resolvers::HostEntityResolver));
                Arc::new(reg)
            }),
```

- [ ] **Step 5: Verify AppState compiles**

```bash
cargo check -p uptrakit-web-api --no-default-features --features db-sqlite 2>&1 | tail -10
```

Expected: clean.

- [ ] **Step 6: Wire into controller `main.rs`**

In `crates/core/controller/src/main.rs`, add an import for `EntityResolverRegistry` and
`HostEntityResolver` near the other `uptrakit_web_api` imports. Then before the
`AppState::builder()` call, add:

```rust
    let mut entity_resolvers = uptrakit_web_api::entity_resolvers::EntityResolverRegistry::new();
    entity_resolvers.register(Box::new(uptrakit_web_api::entity_resolvers::HostEntityResolver));
    let entity_resolver_registry = Arc::new(entity_resolvers);
```

Then in the builder chain (after `.surface_proxy(surface_proxy)`), add:

```rust
        .entity_resolver_registry(entity_resolver_registry)
```

- [ ] **Step 7: Check that `entity_resolvers` is pub in lib.rs**

The module was declared `pub mod entity_resolvers` in Task 2. Also export `EntityResolverRegistry` and `HostEntityResolver` in `lib.rs`:

```rust
pub use entity_resolvers::{EntityResolverRegistry, HostEntityResolver};
```

Add this line after the existing `pub use app_state::{...}` block.

- [ ] **Step 8: Update all AppState struct literals in test helpers**

Adding a non-optional field to `AppState` breaks every struct-literal construction in tests.
Run:

```bash
grep -rn "Arc::new(AppState {" crates/ui/web-api/src/ --include="*.rs"
```

For every location found (routes/services.rs, routes/auth.rs, routes/service_ws/**/*.rs,
routes/settings_nats.rs, routes/surfaces.rs, test_harness/mod.rs, lib.rs,
middleware/require_auth.rs, middleware/audit_log.rs, middleware/resolve_ip.rs, etc.),
add this field to the `AppState { ... }` literal:

```rust
    entity_resolver_registry: Arc::new(crate::entity_resolvers::EntityResolverRegistry::new()),
```

Place it after the `surface_proxy:` field in each literal to keep visual grouping consistent.
No import needed — use the full path `crate::entity_resolvers::EntityResolverRegistry::new()`.

The builder in `app_state.rs` (line ~904) is **not** a struct literal — it calls `AppState { ... }`
inside `build()` itself. That one already gets the value from `self.entity_resolver_registry`
(added in Step 4 above). Do not add the default there.

- [ ] **Step 9: Full check**

```bash
cargo check --all-features 2>&1 | tail -10
```

Expected: clean.

- [ ] **Step 10: Commit**

Step 8 touches many test helper files across `crates/ui/web-api/src/`. Stage everything in
the directory plus controller main:

```bash
git add crates/ui/web-api/src/ crates/core/controller/src/main.rs
git commit -m "feat(web-api): register EntityResolverRegistry in AppState and controller startup"
```

---

## Task 4: Route handler enrichment

**Files:**

- Modify: `crates/ui/web-api/src/routes/surfaces.rs`

- [ ] **Step 1: Add enrichment call after `invoke()`**

In `crates/ui/web-api/src/routes/surfaces.rs`, import the enrichment function at the top of the file with the other `crate::` imports:

```rust
use crate::entity_resolvers::enrich_surface_response;
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
    if let Ok(ref mut response) = result {
        enrich_surface_response(
            response,
            &resolved.descriptor.root_node,
            state.db(),
            Some(tenant_ctx.tenant_id),
            &state.entity_resolver_registry,
        )
        .await;
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

## Task 5: Proxmox plugin updates

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/plugin.rs`
- Modify: `crates/plugins/infrastructure/proxmox/src/surfaces.rs`

- [ ] **Step 1: Write a failing test for the new matched_host shape**

In `crates/plugins/infrastructure/proxmox/src/surfaces.rs`, inside the `tests` module (after the existing `unmatch_action_has_row_visibility` test), add:

```rust
#[tokio::test]
async fn handle_list_matched_host_emits_entity_ref_object() {
    let tenant_id = Uuid::now_v7();
    let config_id = Uuid::now_v7();
    let host_id = Uuid::now_v7();

    // Create a mapping with a matched host_id
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
        .append_query_results([[mock_proxmox_host_mapping(tenant_id, config_id, "vm-unmatched")]])
        .append_query_results([Vec::<uptrakit_shared_db::entity::host::Model>::new()])
        .append_query_results([[mock_plugin_config_model(tenant_id, config_id)]])
        .into_connection();

    let result = handle_list(&db, Some(tenant_id), serde_json::json!({}))
        .await
        .expect("handle_list should succeed");

    let items = result["items"].as_array().expect("items must be an array");
    assert!(items[0]["matched_host"].is_null(), "unmatched host must serialize as null");
}
```

- [ ] **Step 2: Run the new tests to confirm they fail**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox matched_host -- --nocapture 2>&1 | tail -20
```

Expected: FAIL — `matched_host` is currently a string, not an object.

- [ ] **Step 3: Update `handle_list` row builder**

In `crates/plugins/infrastructure/proxmox/src/surfaces.rs`, find the row builder block (around line 806–818):

```rust
            "matched_host": m.host_id.map(|id| id.to_string()),
```

Replace with:

```rust
            "matched_host": m.host_id.map(surfaces::SurfaceEntityRef::unresolved),
```

Make sure `surfaces` is in scope — it already is (imported at the top of the file as part of the module structure).

- [ ] **Step 4: Run the new tests to confirm they pass**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox matched_host -- --nocapture 2>&1 | tail -20
```

Expected: PASS.

- [ ] **Step 5: Run all proxmox tests**

```bash
cargo test -p uptrakit-plugin-infrastructure-proxmox -- --nocapture 2>&1 | tail -20
```

Expected: all pass (existing tests don't assert `matched_host` shape so they are unaffected).

- [ ] **Step 6: Update `matched_host` column declaration in `plugin.rs`**

In `crates/plugins/infrastructure/proxmox/src/plugin.rs`, find:

```rust
                            surfaces::SurfaceTableColumn {
                                key: "matched_host".to_string(),
                                label: "Matched Host".to_string(),
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

In `plugin.rs`, find the `required_capabilities` block and add `surfaces::Capability::EntityLinkColumn` after `surfaces::Capability::ContextSelector`:

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
git add crates/plugins/infrastructure/proxmox/src/plugin.rs crates/plugins/infrastructure/proxmox/src/surfaces.rs
git commit -m "feat(proxmox): emit SurfaceEntityRef for matched_host with EntityLinkColumn capability"
```

---

## Task 6: Frontend types

**Files:**

- Modify: `frontend/src/lib/surfaces/contract.ts`
- Create: `frontend/src/lib/surfaces/entity-routes.ts`

- [ ] **Step 1: Update `contract.ts`**

In `frontend/src/lib/surfaces/contract.ts`, replace:

```ts
export type SurfaceCapability =
	| 'section_node'
    ...
	| 'context_selector';
```

Add `'entity_link_column'` at the end of the union (before the closing `;`):

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

Add the `SurfaceEntityType` import reference — it comes from `entity-routes.ts` (next step),
so add the import after the file is created. For now, declare the type inline in contract.ts
temporarily:

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
export function entityRoute(entityType: SurfaceEntityType, entityId: string): string | null {
	switch (entityType) {
		case 'host':
			return `/hosts/${entityId}`;
		default:
			return null;
	}
}
```

- [ ] **Step 3: Consolidate `SurfaceEntityType`**

Since `entity-routes.ts` defines `SurfaceEntityType`, update `contract.ts` to import it instead of declaring it inline:

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

## Task 7: SurfaceTable entity link rendering

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
    resolvedColumns.some((col) => col.cell_type?.kind === 'entity_link')
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
                <TableFooterBar {total} {currentPage} {totalPages} onPageChange={handlePageChange} />
            {/if}
        {/snippet}
    </DataTable>
{:else if hasRowActions}
    <!-- Plain cells with row actions: DataTable renders default <tr> + rowActions snippet. -->
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
                <TableFooterBar {total} {currentPage} {totalPages} onPageChange={handlePageChange} />
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
                <TableFooterBar {total} {currentPage} {totalPages} onPageChange={handlePageChange} />
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

## Task 8: UI parity tests

**Files:**

- Modify: `frontend/src/lib/test-fixtures/ui-parity.ts`
- Modify: `frontend/tests/e2e/ui-parity.test.ts`

- [ ] **Step 1: Add entity-link fixture to `ui-parity.ts`**

In `frontend/src/lib/test-fixtures/ui-parity.ts`, add a new export interface and extend `SharedVisualParityFixture`:

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

In `buildSharedVisualParityFixture()`, add the entity link fixture before the `return` statement:

```ts
    const entityLinkDataLoadInteractionId = 'entity-link.load';
    const entityLinkDataSourceId = 'entity-link.data';
    const entityLinkSurface = buildParitySurfaceTab('surface.entity-link', 'Entity Link Surface', {
        slot: 'surface.page',
        provider_kind: 'service',
        root_node: {
            kind: 'table',
            data_source_id: entityLinkDataSourceId,
            columns: [
                { key: 'label_col', label: 'Label' },
                {
                    key: 'host_col',
                    label: 'Host',
                    cell_type: { kind: 'entity_link', entity_type: 'host' }
                }
            ]
        }
    });

    const entityLinkInteractions: InteractionDescriptor[] = [
        {
            interaction_id: entityLinkDataLoadInteractionId,
            kind: 'data_load',
            label: 'Load entity link parity data',
            input_schema: 'object',
            result_schema: 'object',
            transport: { mode: 'provider_proxied' }
        }
    ];

    const entityLinkDataSources: DataSourceDescriptor[] = [
        {
            data_source_id: entityLinkDataSourceId,
            kind: { kind: 'provider_query', operation_id: entityLinkDataLoadInteractionId },
            result_schema: 'object',
            pagination: { default_page_size: 10, max_page_size: 10 },
            refresh_policy: { type: 'manual' },
            empty_state: { title: 'No rows', description: undefined }
        }
    ];

    const { provider_count: _entityLinkProviderCount, ...entityLinkDescriptor } = entityLinkSurface;
    const entityLinkReadModel: SurfaceReadResponse = {
        descriptor: entityLinkDescriptor,
        interactions: entityLinkInteractions,
        data_sources: entityLinkDataSources
    };

    // Five states: found+known-route, found+unknown-route, not-found, unenriched, null
    const entityLinkDataLoadResponse = {
        items: [
            {
                label_col: 'found – link',
                host_col: { entity_id: '00000000-0000-0000-0000-000000000001', label: 'web-01', found: true }
            },
            {
                label_col: 'found – no route',
                host_col: { entity_id: '00000000-0000-0000-0000-000000000002', label: 'node-02', found: true }
            },
            {
                label_col: 'not found',
                host_col: { entity_id: '00000000-0000-0000-0000-000000000003', found: false }
            },
            {
                label_col: 'unenriched',
                host_col: { entity_id: '00000000-0000-0000-0000-000000000004' }
            },
            {
                label_col: 'null cell',
                host_col: null
            }
        ],
        total: 5,
        page: 1,
        per_page: 10,
        total_pages: 1
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
            dataLoadResponse: entityLinkDataLoadResponse
        }
    };
```

Note: To cover the "found + route unknown" state, the fixture uses two entity-link columns:
`host_col` (type `'host'`, has a known route) and `future_col` (type `'future_entity'`, no
route). This demonstrates all five rendering states across the two columns.

Revise the fixture to use two entity-link columns:

```ts
    const entityLinkSurface = buildParitySurfaceTab('surface.entity-link', 'Entity Link Surface', {
        slot: 'surface.page',
        provider_kind: 'service',
        root_node: {
            kind: 'table',
            data_source_id: entityLinkDataSourceId,
            columns: [
                { key: 'label_col', label: 'State' },
                {
                    key: 'host_col',
                    label: 'Host (known route)',
                    cell_type: { kind: 'entity_link', entity_type: 'host' }
                },
                {
                    key: 'future_col',
                    label: 'Future (no route)',
                    cell_type: { kind: 'entity_link', entity_type: 'future_entity' }
                }
            ]
        }
    });
```

And the data rows:

```ts
    const entityLinkDataLoadResponse = {
        items: [
            // Row 1: host found + known route → link
            {
                label_col: 'found – link',
                host_col: { entity_id: '00000000-0000-0000-0000-000000000001', label: 'web-01', found: true },
                future_col: null
            },
            // Row 2: future entity found + no route → plain label
            {
                label_col: 'found – no route',
                host_col: null,
                future_col: { entity_id: '00000000-0000-0000-0000-000000000002', label: 'node-02', found: true }
            },
            // Row 3: not found → warning badge
            {
                label_col: 'not found',
                host_col: { entity_id: '00000000-0000-0000-0000-000000000003', found: false },
                future_col: null
            },
            // Row 4: unenriched (found absent) → plain entity_id
            {
                label_col: 'unenriched',
                host_col: { entity_id: '00000000-0000-0000-0000-000000000004' },
                future_col: null
            },
            // Row 5: null cell → —
            {
                label_col: 'null cell',
                host_col: null,
                future_col: null
            }
        ],
        total: 5,
        page: 1,
        per_page: 10,
        total_pages: 1
    };
```

- [ ] **Step 2: Update `buildDefaultReadModels` in `ui-parity.test.ts`**

In `ui-parity.test.ts`, find:

```ts
function buildDefaultReadModels(surfaces: SurfaceResponse[]): Record<string, SurfaceReadResponse> {
    const models = Object.fromEntries(
        surfaces.map((surface) => [surface.surface_id, buildSurfaceRead(surface, `${surface.label} Loaded Content`)])
    );
    models[sharedVisualParity.tableFooter.surface.surface_id] = sharedVisualParity.tableFooter.readModel;
    return models;
}
```

Add the entity link read model:

```ts
    models[sharedVisualParity.entityLink.surface.surface_id] = sharedVisualParity.entityLink.readModel;
```

- [ ] **Step 3: Add surface invoke mock for entity link surface**

In the `mockParityApi` route handler, find:

```ts
if (
    surfaceId === sharedVisualParity.tableFooter.surface.surface_id &&
    interactionId === sharedVisualParity.tableFooter.dataLoadInteractionId
) {
    return json(sharedVisualParity.tableFooter.dataLoadResponse);
}
```

Add a case for the entity link fixture immediately after:

```ts
if (
    surfaceId === sharedVisualParity.entityLink.surface.surface_id &&
    interactionId === sharedVisualParity.entityLink.dataLoadInteractionId
) {
    return json(sharedVisualParity.entityLink.dataLoadResponse);
}
```

- [ ] **Step 4: Add the parity test**

After the `'shared primitive ui parity: table footer totals and pagination alignment'` test, add:

```ts
test('shared primitive ui parity: entity link cell rendering states', async ({ page }) => {
    if (!isCanonicalUiParityHost) {
        test.skip(true, canonicalUiParityReason);
    }

    const entityLinkSurfaces = [...paritySurfaces, sharedVisualParity.entityLink.surface];
    const entityLinkReadModels = buildDefaultReadModels(entityLinkSurfaces);
    await mockParityApi(page, {
        surfaces: entityLinkSurfaces,
        readModels: entityLinkReadModels
    });

    await page.goto(`/surfaces/${sharedVisualParity.entityLink.surface.surface_id}`);

    const dataTable = page.locator('[data-ui="data-table"]');
    await expect(dataTable).toBeVisible();

    // Wait for entity link cells to render (found link visible)
    await expect(dataTable.locator('a[href="/hosts/00000000-0000-0000-0000-000000000001"]')).toBeVisible();

    await captureParityScreenshot(page, dataTable, 'ui-parity-entity-link-cells.png');
});
```

- [ ] **Step 5: Run frontend tests**

```bash
cd frontend && npm run test 2>&1 | tail -20
```

Expected: all unit tests pass (the parity screenshot test is skipped outside macOS CI).

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

Verify the new `ui-parity-entity-link-cells.png` baseline files appear in the test snapshots directory.

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
