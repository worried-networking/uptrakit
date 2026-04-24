# Surface Entity Links

**Date:** 2026-04-24
**Status:** Approved

## Problem

Surface table columns that reference internal entities currently emit raw UUIDs or plain strings.
There is no way for a plugin or service to declare that a column value is an entity reference,
and the UI has no mechanism to render it as a navigable link.

## Goal

Allow any surface table column to be declared as an entity-link column. The plugin emits an entity
ID; the controller framework resolves the human-readable label via a batch DB lookup; the frontend
renders a clickable link. If the entity has been deleted or is otherwise unknown, the UI shows a
warning rather than a broken value.

## Design

### 1. Wire layer — `crates/shared/surfaces/src/surface.rs`

#### `SurfaceTableColumn`

Add an optional `cell_type` field. `None` means plain-text rendering (existing behavior).

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceTableColumn {
    pub key: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell_type: Option<SurfaceTableCellType>,
}
```

#### `SurfaceTableCellType`

Tagged enum. `#[non_exhaustive]` per codebase convention for extensible public enums.

```rust
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SurfaceTableCellType {
    EntityLink { entity_type: SurfaceEntityType },
}
```

#### `SurfaceEntityType`

Wire-safe enum following the `Other(String)` pattern (see coding standards).
Known variants are type-safe; an unknown value from a newer peer falls through gracefully.

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceEntityType {
    Host,
    Other(String),
}

impl From<String> for SurfaceEntityType {
    fn from(s: String) -> Self {
        match s.as_str() {
            "host" => Self::Host,
            _ => {
                tracing::debug!(value = s, "received unknown SurfaceEntityType from peer");
                Self::Other(s)
            }
        }
    }
}

// Custom infallible Deserialize: same pattern as EnrollmentStatus / ErrorCode
// in crates/shared/wire/src/lib.rs — deserializes as String, delegates to From<String>.
```

#### `SurfaceEntityRef`

The cell value type. Plugin constructs via `SurfaceEntityRef::unresolved(id)` (only `entity_id`
set). Framework enriches in-place by populating `label` and `found`.

```rust
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfaceEntityRef {
    pub entity_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,   // None when found == false
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub found: Option<bool>,     // None = pre-enrichment (transient); Some(false) = deleted/unknown
}

impl SurfaceEntityRef {
    pub fn unresolved(entity_id: Uuid) -> Self {
        Self { entity_id, label: None, found: None }
    }
}
```

#### `Capability`

New variant added to the existing `Capability` enum:

```rust
EntityLinkColumn,
```

---

### 2. Framework enrichment — `crates/ui/web-api`

#### `EntityResolver` trait

```rust
#[async_trait]
pub trait EntityResolver: Send + Sync {
    fn entity_type(&self) -> SurfaceEntityType;
    async fn resolve_labels(
        &self,
        db: &DatabaseConnection,
        tenant_id: Option<Uuid>,
        ids: &[Uuid],
    ) -> HashMap<Uuid, String>;
}
```

#### `EntityResolverRegistry`

Stored in `AppState`. Maps `SurfaceEntityType → Box<dyn EntityResolver>`. Populated at startup.

Initial registration: `HostEntityResolver` — queries `host::Entity` by IDs (scoped to
`tenant_id`), returns `friendly_name`.

#### Enrichment step

Runs in `surface_proxy.rs` after a surface action handler returns, before the HTTP response
is sent.

Algorithm:

1. Walk the full surface node tree (recursing into `Section.children`, `Tabs.tabs[*].root`,
   `ModalTrigger.modal_nodes`, `WorkflowTrigger.step_nodes`) and collect all `Table` nodes.
2. Collect columns with `cell_type = Some(EntityLink { entity_type })`.
3. If none, return the response unchanged.
4. Parse `response["items"]` as an array. If absent or not an array, skip.
5. For each entity-link column key, scan all items and collect non-null `entity_id` values,
   grouped by `entity_type`.
6. For each entity type, call `registry.resolve(entity_type, db, tenant_id, ids)` —
   one DB query per type, not per row.
7. Rewrite each matching cell in-place:
   - ID found in resolver result → `{ entity_id, label: "...", found: true }`
   - ID absent from result → `{ entity_id, label: null, found: false }`
   - Cell is `null`/absent → leave unchanged (unmatched rows stay null)
8. For `SurfaceEntityType::Other(_)` with no registered resolver → leave cells unenriched
   (no crash, no label).

---

### 3. Proxmox plugin — `crates/plugins/infrastructure/proxmox/src/`

#### `plugin.rs` — column declaration

```rust
surfaces::SurfaceTableColumn {
    key: "matched_host".to_string(),
    label: "Matched Host".to_string(),
    cell_type: Some(surfaces::SurfaceTableCellType::EntityLink {
        entity_type: surfaces::SurfaceEntityType::Host,
    }),
},
```

`Capability::EntityLinkColumn` added to `required_capabilities` on `proxmox.hosts`.

#### `surfaces.rs` — `handle_list` row builder

```rust
let matched_host = m.host_id.map(SurfaceEntityRef::unresolved);

let row = serde_json::json!({
    // ...existing fields...
    "matched_host": matched_host,
    // ...
});
```

`matched_host` previously emitted a raw UUID string; it now emits `SurfaceEntityRef::unresolved(id)`.
No host-name DB query was ever done for this field — the framework now provides the label.
The `suggested_host` plain-string column is unchanged.

---

### 4. Frontend — `frontend/src/`

#### Contract types (`src/lib/surfaces/contract.ts`)

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

#### Entity type and route map (`src/lib/surfaces/entity-routes.ts`)

```ts
// Known entity types. `string & {}` preserves autocomplete while accepting
// unknown types from newer backend versions (forward-compatible).
export type SurfaceEntityType = 'host' | (string & {});

export function entityRoute(entityType: SurfaceEntityType, entityId: string): string | null {
    switch (entityType) {
        case 'host': return `/hosts/${entityId}`;
        default: return null;
    }
}
```

#### `SurfaceTable.svelte`

When any column in the surface node has a `cell_type`, passes a custom `row` snippet to `DataTable`.
Rendering rules per cell:

| Condition | Rendering |
| --- | --- |
| `entity_link` + `found !== false` + route known | `<a href={route}>{label}</a>` |
| `entity_link` + `found !== false` + route unknown | plain label or raw ID |
| `entity_link` + `found === false` | warning badge ("Unknown entity") |
| `entity_link` + cell is null/absent | `—` |
| no `cell_type` | `String(value ?? '')` |

If no column has a `cell_type`, no row snippet is passed — zero overhead for plain surfaces.

---

## Data flow summary

```text
Plugin handler
  └─ emits { "matched_host": { "entity_id": "uuid" } }

surface_proxy.rs enrichment step
  └─ detects EntityLink column
  └─ batch-resolves host labels from DB
  └─ rewrites to { "entity_id": "uuid", "label": "my-host", "found": true }
     or         { "entity_id": "uuid", "label": null, "found": false }

Frontend SurfaceTable
  └─ renders <a href="/hosts/uuid">my-host</a>
     or warning badge if found === false
     or — if cell is null
```

## Out of scope

- Navigation hints (tab, anchor) — `SurfaceEntityRef` is `#[non_exhaustive]` to allow future
  addition without a breaking change. No implementation now.
- Auto-resolution for `SurfaceEntityType::Other` — unknown types pass through unenriched;
  link rendering falls back to raw ID display.
- New entity type resolvers beyond `host` — add by implementing `EntityResolver` and registering
  in `AppState`.
