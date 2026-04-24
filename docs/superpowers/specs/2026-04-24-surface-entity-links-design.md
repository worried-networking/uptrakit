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

Unknown `cell_type` variants received from a newer peer must not break deserialization. Use a
lenient `deserialize_with` function that returns `None` on any parse failure, so old framework
versions silently fall back to plain-text rendering for unrecognized cell types.

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
    // Deserialize as a raw Value first so that an unknown `kind` tag does not
    // propagate a hard error — instead return None (plain-text rendering).
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.and_then(|v| serde_json::from_value(v).ok()))
}
```

#### `SurfaceTableCellType`

Tagged enum. `#[non_exhaustive]` per codebase convention for extensible public enums.
Forward-compat at the deserialization boundary is handled by the lenient `cell_type` deserializer
above, not by an `Other` variant on this enum.

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
Requires both custom `Serialize` and custom `Deserialize` (same pattern as `EnrollmentStatus`
and `ErrorCode` in the `uptrakit-internal-wire` crate) — the derived `Serialize` would emit
`{ "other": "..." }` for the `Other(String)` variant instead of the bare string, which breaks
wire compatibility. `serde_json` must be present in `uptrakit-surfaces/Cargo.toml` (verify at
implementation time — it may already be a transitive dependency via `DataSourceKind::Static`).

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SurfaceEntityType {
    Host,
    Other(String),
}

impl From<String> for SurfaceEntityType {
    fn from(s: String) -> Self {
        match s.as_str() {
            "host" => Self::Host,
            _ => Self::Other(s),
        }
    }
}

// Custom Serialize: emit the bare snake_case string (e.g. "host", "my_future_type").
// Custom Deserialize: deserialize as String, delegate to From<String>.
// Follow the canonical pattern in the uptrakit-internal-wire crate.
```

Note: `Other(String)` removes `Copy` from `SurfaceEntityType`. This is intentional and consistent
with the wire-safe pattern used elsewhere in the codebase.

#### `SurfaceEntityRef`

The cell value type for entity-link columns. Valid only in list-response data sources
(`{ "items": [...] }` shape) — `SurfaceEntityRef` must not be emitted by non-list interactions
(key-value panels, direct-object responses), because the enrichment step only runs on list
responses.

Plugin constructs via `SurfaceEntityRef::unresolved(id)` (only `entity_id` set). Framework
enriches by populating `label` and `found` in the JSON response value before it is sent.
`found: None` is a transient pre-enrichment state — it must not appear in the wire response.

```rust
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfaceEntityRef {
    pub entity_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,   // None when found == false
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub found: Option<bool>,     // None = pre-enrichment (transient); Some(false) = not found
}

impl SurfaceEntityRef {
    pub fn unresolved(entity_id: Uuid) -> Self {
        Self { entity_id, label: None, found: None }
    }
}
```

Note: enrichment operates on `serde_json::Value` in the response body, not on `SurfaceEntityRef`
struct instances — plugins serialize via `serde_json::json!`, and the framework rewrites the
resulting JSON object fields directly.

#### `Capability`

`EntityLinkColumn` appended at the end of the existing `Capability` enum (preserves the
existing sort order of the `BTreeSet<Capability>` serialization for all pre-existing surfaces):

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

    /// Resolve display labels for the given entity IDs.
    ///
    /// Returns a map of entity ID → display label for all IDs that exist.
    /// IDs absent from the map are treated as deleted/unknown and will receive
    /// `found: false` in the enriched response.
    ///
    /// On DB error, return `Err`. The enrichment step will treat all IDs for
    /// this entity type as unresolvable and set `found: false`. This conflates
    /// transient DB errors with permanently deleted entities — the frontend
    /// cannot distinguish the two. Accepted limitation; the operator must
    /// reload to recover.
    async fn resolve_labels(
        &self,
        db: &DatabaseConnection,
        tenant_id: Option<Uuid>,
        ids: &[Uuid],
    ) -> Result<HashMap<Uuid, String>, rootcause::Report>;
}
```

#### `EntityResolverRegistry`

Stored in `AppState`. Maps `SurfaceEntityType → Box<dyn EntityResolver>`. Populated at startup.
Registry is static for the application lifetime — no hot-reload.

Initial registration: `HostEntityResolver` — queries `host::Entity` by IDs (scoped to
`tenant_id`), returns the `friendly_name` column as the display label. Fallback chain if
`friendly_name` is absent: `hostname`, then the entity ID string. When the ID string is used,
the cell is still emitted as `found: true` (the entity exists; it just has no human-readable
name yet). This renders as a link labeled with the raw UUID — acceptable UX for a newly
enrolled host.

#### Enrichment step

Runs in `surface_proxy.rs` after a surface action handler returns, before the HTTP response
is sent. Enrichment applies **only** to responses with shape `{ "items": [ ... ] }` (paginated
list responses). Responses with other shapes are returned unchanged.

Algorithm:

1. Walk the full surface node tree (recursing into `Section.children`, `Tabs.tabs[*].root`,
   `ModalTrigger.modal_nodes`, `WorkflowTrigger.step_nodes`) and collect all `Table` nodes.
2. Collect columns with `cell_type = Some(EntityLink { entity_type })`.
3. If none, return the response unchanged.
4. Check that `response` is an object with an `"items"` key whose value is an array.
   If not, return the response unchanged.
5. For each entity-link column key, scan all items and collect non-null `entity_id` string
   values, grouped by `entity_type`. Null/absent cells are skipped (unmatched rows).
6. For each entity type with collected IDs:
   a. Look up the resolver in `EntityResolverRegistry`.
   b. If no resolver found (`SurfaceEntityType::Other(_)` or unregistered type) — skip:
      leave those cells unenriched. `found` remains absent in the JSON; the frontend
      renders these as links (see rendering table row 5 in section 4).
   c. Call `resolver.resolve_labels(db, tenant_id, ids).await`. All IDs for this entity type
      are collected into a single call — one DB query per type, no N+1.
   d. On `Err`: log the error; treat all IDs for this type as unresolvable (empty label map).
      Cells will receive `found: false`. See the doc comment on `EntityResolver` for the
      accepted limitation.
7. For each entity-link cell that has a registered resolver, rewrite the JSON object in-place.
   `found` is set unconditionally — `found: None` must not appear in the final wire response
   for cells whose resolver ran (successfully or not). Cells for unregistered types (step 6b)
   are not touched by this step and retain `found` absent.
   - `entity_id` found in label map →
     rewrite JSON object to `{ "entity_id": "...", "label": "...", "found": true }`
   - `entity_id` absent from label map (deleted or DB error) →
     rewrite JSON object to `{ "entity_id": "...", "found": false }`
   - Cell is `null`/absent → leave unchanged

#### Row-action compatibility

The `unmatch` action on `proxmox.hosts` uses `visible_when: { field: "matched_host", condition:
Present }`. After this change, `matched_host` is a JSON object `{ entity_id, label, found }`
rather than a UUID string. A non-null JSON object satisfies the `Present` condition
(`value != null && value !== ''`). No change to the row-action descriptor is needed.

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

`Capability::EntityLinkColumn` added to the `required_capabilities` set in
`proxmox_hosts_surface()`.

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
No host-name DB query was ever done for this field — the framework provides the label.
The `suggested_host` plain-string column is unchanged.

The existing tests in the `handle_list` test block (around lines 1984–2059 in `surfaces.rs`)
assert the shape of `matched_host` items. These tests must be updated to expect the new
`{ "entity_id": "...", "label": "...", "found": true/false }` object shape.

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

The `Capability` union type in `contract.ts` gains `'entity_link_column'` to match the new
Rust variant.

#### Entity type and route map (`src/lib/surfaces/entity-routes.ts`)

```ts
// Known entity types. `string & {}` preserves autocomplete while accepting
// unknown types from newer backend versions (forward-compatible).
// TypeScript switch exhaustiveness is not enforced — the `default` arm is
// always required to handle unknown future entity types gracefully.
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
Entity-link cells are parsed as `SurfaceEntityRef`. Rendering rules per cell:

| Condition | Rendering |
| --- | --- |
| `entity_link` + `found === true` + route known | `<a href={route}>{label}</a>` |
| `entity_link` + `found === true` + route unknown | plain `label` text (no link) |
| `entity_link` + `found === false` | warning badge ("Unknown entity") |
| `entity_link` + `found` absent (unenriched) | plain `entity_id` text (no link) |
| `entity_link` + cell is null/absent | `—` |
| no `cell_type` | `String(value ?? '')` |

The `found` absent row covers two cases: (a) an unregistered entity type with no resolver,
and (b) any future code path that emits `SurfaceEntityRef` outside the enrichment flow.
Rendering as plain `entity_id` text is a safe, non-broken fallback.

If no column has a `cell_type`, no row snippet is passed — zero overhead for plain surfaces.

#### Parity tests (`frontend/tests/e2e/ui-parity.test.ts`)

The new `entity_link` rendering path requires parity fixtures in `ui-parity.test.ts` (the
CI-gated parity harness, not `surface-preview.spec.ts`) covering light and dark themes for
all five entity-link cell states listed in the rendering table above.

---

## Data flow summary

```text
Plugin handler
  └─ emits { "matched_host": { "entity_id": "uuid" } }

surface_proxy.rs enrichment step
  └─ detects EntityLink column
  └─ batch-resolves host friendly_name from DB (one query for all IDs on page)
  └─ rewrites to { "entity_id": "uuid", "label": "my-host", "found": true }
     or         { "entity_id": "uuid", "found": false }  (deleted or DB error)
     (found is always set for registered resolvers — never absent in wire response)

Frontend SurfaceTable
  └─ found === true  → <a href="/hosts/uuid">my-host</a>
  └─ found === false → warning badge ("Unknown entity")
  └─ found absent   → plain entity_id text
  └─ cell null      → —
```

## Known limitations

- **DB errors indistinguishable from deleted entities.** A transient DB error during label
  resolution causes `found: false` on the affected cells, which the UI renders as a warning
  badge. The operator must reload to recover once the DB is healthy. A three-state design
  (found/not-found/error) was considered and deferred — complexity not justified for now.
- **Enrichment applies only to `{ items: [...] }` responses.** `SurfaceEntityRef` must not
  be emitted from non-list interactions. Key-value panels and other surface nodes that need
  entity display names must fetch and embed them directly in the plugin handler.

## Out of scope

- Navigation hints (tab, anchor) — `SurfaceEntityRef` is `#[non_exhaustive]` to allow future
  addition without a breaking change. No implementation now.
- New entity type resolvers beyond `host` — add by implementing `EntityResolver` and registering
  in `AppState` at startup.
