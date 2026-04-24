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

`SurfaceTableColumn` is not currently `#[non_exhaustive]`, so adding a new field is a
compilation-breaking change for all struct literal usages. There are approximately 32 such
usages across 9 files (`agent-ssh`, `mqtt-runtime`, `proxmox/plugin.rs`, `webhook`, `telegram`,
`email` plugins, `surface_form_authoring.rs`, etc.). All existing struct literal sites must add
`cell_type: None` as part of this change.

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

Per codebase standards, all `match` sites on `SurfaceTableCellType` outside the defining crate
must include a wildcard arm with `tracing::warn!` and a documented safe fallback (e.g., plain-text
rendering). The enrichment step in `entity_enrichment.rs` is one such match site and must follow
this pattern.

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
wire compatibility. `serde_json` is already a direct dependency of `uptrakit-surfaces`
(`Cargo.toml` line 12) — no new dependency needed.

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

#### `enrich_entity_links` — free function, static dispatch

Lives in a new module `surface_proxy/entity_enrichment.rs`. No trait, no registry, no
`AppState` change. `AppState` already exposes `state.db()` and the surface node is available
at the call site in `surfaces.rs`.

```rust
/// Enriches entity-link cells in a surface list response.
///
/// Resolves display labels for each known `SurfaceEntityType` via a direct DB
/// query per entity type (one query per type — no N+1). Unknown types
/// (`SurfaceEntityType::Other(_)`) are skipped and their cells remain unenriched.
///
/// # Future extension
///
/// If more entity types are introduced, consider replacing this static `match`
/// with a proper `EntityResolverRegistry` (a `HashMap<SurfaceEntityType,
/// Box<dyn EntityResolver>>` populated at startup) rather than adding more arms
/// here. At two or more additional entity types the registry pattern pays for
/// itself.
pub async fn enrich_entity_links(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    surface_node: &SurfaceNode,
    response: serde_json::Value,
) -> serde_json::Value {
    // ...
}
```

Label resolution for `SurfaceEntityType::Host`: queries `host::Entity` by IDs scoped to
`tenant_id`. Both `friendly_name` and `hostname` are non-nullable `String` columns (never
`Option`). Fallback chain: use `friendly_name` if non-empty; else use `hostname` if non-empty;
else use the entity ID UUID string. When the UUID string is the fallback label, the cell is still
emitted as `found: true` (the entity exists; it just has no human-readable name yet). This
renders as a link labeled with the raw UUID — acceptable UX for a newly enrolled host.

When `tenant_id` is `None` (system-level surface invocation without tenant scope), the host
query is performed without a tenant filter — all hosts are eligible. This matches the behaviour
of other system-level routes that bypass tenant isolation.

#### Enrichment step

`enrich_entity_links` is called in `surfaces.rs` after a surface action handler returns
successfully (HTTP 200 path only), before the HTTP response is sent. Error responses (4xx, 5xx)
are not enriched. Enrichment applies **only** to successful responses with shape
`{ "items": [ ... ] }` (paginated list responses). Responses with other shapes are returned
unchanged.

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
   a. Match on `entity_type`. `SurfaceEntityType::Other(_)` — skip: leave those cells
      unenriched. `found` remains absent in the JSON; the frontend renders these as plain
      `entity_id` text (see rendering table row 4 in section 4). Any future named variant
      added to `SurfaceEntityType` before a match arm is added here should be handled the
      same way (treat as unrecognised — leave unenriched).
   b. Call the static resolver for the matched type. All IDs for that type are collected into
      a single call — one DB query per type, no N+1.
   c. On `Err`: log the error; treat all IDs for this type as unresolvable (empty label map).
      Cells will receive `found: false`. This conflates transient DB errors with permanently
      deleted entities — the frontend cannot distinguish the two. Accepted limitation; the
      operator must reload to recover.
7. For each entity-link cell whose type was dispatched in step 6, rewrite the JSON object
   in-place. `found` is set unconditionally — `found: None` must not appear in the final wire
   response for cells that were dispatched (successfully or not). Cells for skipped types
   (step 6a) are not touched and retain `found` absent.
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

The `handle_list` test block (search for `fn handle_list` in `surfaces.rs`) does not currently
assert on `matched_host` value. A new assertion should be added to cover the unenriched shape:
`{ "entity_id": "..." }` (no `label`, no `found`). These tests use `MockDatabase` and do not
go through the web-api enrichment step, so they should never expect the enriched shape. The
enriched shape `{ "entity_id": "...", "label": "...", "found": true/false }` is verified at
the web-api integration-test level.

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
     (found is always set for dispatched entity types — never absent in wire response)

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
- New entity type resolvers beyond `host` — add a match arm in `enrich_entity_links`. If the
  number of supported types grows to three or more, replace the static match with an
  `EntityResolverRegistry` (see the doc comment on `enrich_entity_links` for the migration
  note).
