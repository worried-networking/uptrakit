# Software Item Name Filtering

**Date:** 2026-05-15
**Status:** Approved

## Problem

`list_software_items` in `crates/ui/web-api-queries/src/queries/software_items/crud.rs`
handles the `query` param via in-memory filtering: it loads all items matching other
filters, calls `items.retain()`, then paginates in Rust. At ~4k items per tenant this
causes three problems:

1. Loads all rows regardless of filter — unnecessary DB work.
2. Breaks pagination correctness — `total` reflects post-filter count, but offset/limit are applied in Rust after a full DB fetch.
3. Ignores the existing index
   `idx_software_items_tenant_lower_name ON software_items (tenant_id, lower(name))`.

The `query` field in `ListSoftwareItemsParams` and the `getSoftwareItems(query?)` frontend
API client already exist. No migration or API type changes are needed.

## Solution

Two-layer fix: push the filter to SQL in the backend, expose a filter input in the frontend.

---

## Backend

**File:** `crates/ui/web-api-queries/src/queries/software_items/crud.rs`

### Filter construction

Lowercase the query in Rust, escape LIKE metacharacters (`%` and `_`), then build a
`%pattern%` bind param. SQL receives a fully-escaped, pre-lowercased string — no
`LOWER()` call needed on the pattern side, and no metacharacter injection possible:

```rust
let escaped = query.to_lowercase()
    .replace('%', r"\%")
    .replace('_', r"\_");
let pattern = format!("%{escaped}%");
base_query = base_query.filter(
    Expr::expr(Func::lower(Expr::col(software_item::Column::Name)))
        .like(pattern)
);
```

> **Note:** SeaORM's `.like()` sends the pattern as a bind parameter, preventing SQL
> injection. The `\%` / `\_` escapes prevent the user's literal `%` or `_` characters
> from acting as LIKE wildcards. SQLite and Postgres both honour backslash escaping in
> `LIKE` by default.

This is added to `base_query` alongside the other filters (`featured`, `host_id`,
`updatable`, `plugin_type`) before the paginator runs.

### Pagination unification

The special in-memory branch (`if let Some(query) = query { … }` / `else { … }`) is
replaced by a single path: apply the filter to `base_query`, then use the standard
`count()` + paginated `all()` pattern that the non-query branch already uses. The branch
disappears entirely.

### Index usage

`idx_software_items_tenant_lower_name ON (tenant_id, lower(name))` already exists. Both
SQLite and Postgres use the composite index to scope to the tenant's rows, then scan ~4k
`lower(name)` values for the LIKE check. No new migration needed.

### Docstring update

Update the `query` field doc in `ListSoftwareItemsParams`:

> Filter by name — case-insensitive substring match. Lowercased by the caller before the SQL bind; the database evaluates `LOWER(name) LIKE ?`.

---

## Frontend

**File:** `frontend/src/routes/software/+page.svelte`

All changes follow existing filter patterns in the file.

### State

```svelte
let nameFilter: string = $state(page.url.searchParams.get('query') ?? '');
```

Alongside `pluginTypeFilter` and `showUpdatableOnly`.

### URL persistence

Add to the `$effect` URL builder:

```svelte
if (isItemsTab && nameFilter) parts.push(`query=${encodeURIComponent(nameFilter)}`);
```

### Tab switching

Add `nameFilter = ''` to the non-items tab clearing block in `switchTab` (alongside the existing `showUpdatableOnly = false; pluginTypeFilter = ''`).

### `loadAll`

Pass `nameFilter || undefined` as the `query` argument to `getSoftwareItems`.

### Filter UI

Add an `Input` component in the filter row alongside "Updates available" and the plugin
type `Select`. Debounce at 300ms via inline `setTimeout`/`clearTimeout`. On fire:
`currentPage = 1; loadAll(1)`.

```svelte
<Input
  id="software-name-filter"
  type="search"
  placeholder="Filter by name"
  bind:value={nameFilter}
  oninput={() => {
    clearTimeout(nameFilterDebounce);
    nameFilterDebounce = setTimeout(() => { currentPage = 1; loadAll(1); }, 300);
  }}
/>
```

`nameFilterDebounce` is a `let nameFilterDebounce: ReturnType<typeof setTimeout> | null = $state(null)` alongside other local state. Clear it in `onDestroy`.

No changes to `getSoftwareItems` in `api.ts`.

---

## Testing

### Backend (integration)

Add test in `crates/ui/web-api/src/integration_tests/software_items_crud.rs`:

- Create 3 items: `"1Password"`, `"Node.js"`, `"nginx"`.
- Call `list_software_items` with `query = Some("pass")`.
- Assert: `items.len() == 1`, `items[0].name == "1Password"`, `total == 1`.
- Call with `query = Some("PASS")` (uppercase) — assert same result (case-insensitive).
- Call with `query = Some("z")` — assert `items` contains `"nginx"` only.
- Call with `query = Some("   ")` (whitespace-only, trimmed to empty) — assert all 3
  items returned (no filter applied).
- Call with `query = Some("1%")` — assert zero results (literal `%` is escaped, not a
  wildcard; no item named `"1"` followed by anything).

### Frontend

Add one Svelte test: assert `nameFilter` initializes from URL `?query=foo` param.

---

## Out of scope

- FTS5 / trigram indexes — unnecessary at current scale.
- Fuzzy/similarity matching — not requested.
- Searching fields other than `name` — `query` field comment intentionally leaves room, but no metadata fields are searched in this iteration.
