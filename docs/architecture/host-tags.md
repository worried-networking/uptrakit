# Host tags

Host tags provide user-defined labels for organizing and classifying hosts within a tenant. Tags are
lightweight metadata with a name, color, and optional description. A host can have multiple tags, and
a tag can be assigned to multiple hosts.

## Database schema

Two tables support the feature:

### `host_tags`

| Column | Type | Constraints | Description |
| --- | --- | --- | --- |
| `id` | UUID | PK | Tag identifier (UUID v7) |
| `tenant_id` | UUID | FK `tenants.id`, NOT NULL | Owning tenant |
| `name` | TEXT | NOT NULL | Human-readable tag name (max 100 characters) |
| `color` | TEXT | NOT NULL | Hex color code (e.g. `#3B82F6`) |
| `description` | TEXT | nullable | Optional description (max 500 characters) |
| `created_at` | TIMESTAMPTZ | NOT NULL | Creation timestamp |
| `updated_at` | TIMESTAMPTZ | NOT NULL | Last modification timestamp |
| `deactivated_at` | TIMESTAMPTZ | nullable | Soft-delete timestamp; non-null means deleted |

**Indexes:**

- `idx_host_tags_tenant_id` on `(tenant_id)` -- tenant-scoped listing queries.
- `uix_host_tags_tenant_name` partial unique on `(tenant_id, name) WHERE deactivated_at IS NULL` --
  ensures unique active tag names within a tenant while allowing multiple deactivated tags with the
  same name.

**Foreign keys:**

- `fk_host_tags_tenant_id` references `tenants(id)` with `ON DELETE RESTRICT`.

### `host_tag_assignments`

| Column | Type | Constraints | Description |
| --- | --- | --- | --- |
| `host_tag_id` | UUID | PK (composite), FK `host_tags.id` | Tag being assigned |
| `host_id` | UUID | PK (composite), FK `hosts.id` | Host receiving the tag |
| `assigned_at` | TIMESTAMPTZ | NOT NULL | When the assignment was created |

**Indexes:**

- Composite PK on `(host_tag_id, host_id)` -- prevents duplicate assignments.
- `idx_host_tag_assignments_host_id` on `(host_id)` -- "find all tags for a host" queries.
- `idx_host_tag_assignments_tag_id` on `(host_tag_id)` -- "find all hosts with a tag" queries.

**Foreign keys:**

- `fk_host_tag_assignments_tag_id` references `host_tags(id)` with `ON DELETE CASCADE`.
- `fk_host_tag_assignments_host_id` references `hosts(id)` with `ON DELETE CASCADE`.

## Entity relationships

```text
tenant ──1:N──► host_tags ◄──N:M via host_tag_assignments──► hosts
```

- A **tenant** owns zero or more **host tags**.
- A **host tag** belongs to exactly one **tenant**.
- A **host** can have zero or more **tags** (via `host_tag_assignments`).
- A **tag** can be assigned to zero or more **hosts** (via `host_tag_assignments`).

`host_tag_assignments` is a pure join table with no `tenant_id` column. Tenant isolation is
enforced by scoping all tag queries through `TenantDb`, which filters on the `host_tags.tenant_id`
column. The `TenantScoped` trait is implemented on `host_tag::Entity`.

## Soft-delete behavior

Deleting a host tag performs two operations inside a transaction:

1. **Hard-delete** all rows in `host_tag_assignments` where `host_tag_id` matches.
2. **Soft-delete** the `host_tags` row by setting `deactivated_at` to the current timestamp.

The partial unique index on `(tenant_id, name)` only covers active tags
(`WHERE deactivated_at IS NULL`), so a new tag with the same name can be created after deletion.

When a **host** is deactivated, the `ON DELETE CASCADE` on `host_tag_assignments.host_id` cleans up
assignments automatically if the host row is hard-deleted.

## Auto-generated colors

When creating a tag without an explicit `color`, the server picks one from a curated 12-color
palette based on the count of existing active tags for the tenant:

```text
#3B82F6  #EF4444  #10B981  #F59E0B  #8B5CF6  #EC4899
#06B6D4  #F97316  #6366F1  #14B8A6  #E11D48  #84CC16
```

The formula is `COLOR_PALETTE[active_tag_count % 12]`. This provides visually distinct colors for the
first 12 tags; after that, colors cycle. Users can override the color on create or update with any
valid 7-character hex code (e.g. `#FF5733`).

## Tenant isolation

Host tags implement `TenantScoped` via the `tenant_id` column on `host_tags`. All query functions
use `TenantDb` helpers (`tenant_db.find::<host_tag::Entity>()`, `tenant_db.find_by_id::<host_tag::Entity, _>(id)`)
to enforce tenant isolation automatically.

The `set_host_tags` function additionally verifies that:

- The target host belongs to the requesting tenant and is active (not deactivated).
- All provided tag IDs belong to the requesting tenant and are active.

Invalid or cross-tenant tag IDs are silently filtered out during assignment.

## Host response integration

The `HostResponse` type includes a `tags: Vec<HostTagSummary>` field containing the slim tag summary
(id, name, color) for each assigned tag. Tags are batch-loaded via `load_host_tags_batch()` to avoid
N+1 queries when listing hosts.

## Admin events

Four SSE admin events are emitted for real-time UI updates:

| Event | Trigger |
| --- | --- |
| `host_tag_created` | New tag created |
| `host_tag_updated` | Tag name, color, or description changed |
| `host_tag_deleted` | Tag soft-deleted |
| `host_tags_changed` | Tags assigned/unassigned on a host |

## Key files

| File | Purpose |
| --- | --- |
| `crates/shared/db/src/entity/host_tag.rs` | SeaORM entity for `host_tags` |
| `crates/shared/db/src/entity/host_tag_assignment.rs` | SeaORM entity for `host_tag_assignments` |
| `crates/shared/db/src/migration/m20260309_000003_host_tags.rs` | Migration creating both tables |
| `crates/shared/web-api-types/src/host_tags.rs` | Request/response types and validation |
| `crates/ui/web-api-queries/src/queries/host_tags.rs` | Query functions (CRUD, batch load, batch delete) |
| `crates/ui/web-api/src/routes/host_tags.rs` | Axum route handlers |
| `crates/shared/openapi-client/src/host_tags.rs` | Typed API client methods |
| `crates/ui/cli/src/commands/host_tags.rs` | CLI command implementations |

## See also

- [Host Entity](host-entity.md) -- host data model and agent linking
- [Multi-Tenancy](multi-tenancy.md) -- tenant isolation model
- [Host Tags API](../api/host-tags.md) -- REST endpoint reference
- [CLI Usage Guide](../end-user/cli-usage.md#host-tags) -- CLI commands
