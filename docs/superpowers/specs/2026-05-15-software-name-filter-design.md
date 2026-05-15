# Software Item Name Filtering

**Date:** 2026-05-15
**Status:** Approved

## Problem

`list_software_items` in `crates/ui/web-api-queries/src/queries/software_items/crud.rs`
handles the `query` param via in-memory filtering: it loads all items matching other
filters, calls `items.retain()`, then paginates in Rust. At ~4k items per tenant this
causes three problems:

1. Loads all rows regardless of filter — unnecessary DB work.
2. Breaks pagination correctness — `total` and `offset`/`limit` are computed after a
   full DB fetch, so page 2 of a filtered result set is wrong.
3. Ignores the existing index
   `idx_software_items_tenant_lower_name ON software_items (tenant_id, lower(name))`.

The `query` field in `ListSoftwareItemsParams` and the `getSoftwareItems(query?)` frontend
API client already exist. No migration or API type changes are needed.

## Solution

Two-layer fix: push the filter to SQL in the backend, expose a filter input in the
frontend.

---

## Backend

**File:** `crates/ui/web-api-queries/src/queries/software_items/crud.rs`

### Validation

`ListSoftwareItemsParams` is extracted via `Query(params)` (not `Validated<...>`), so
any `Validate` impl on it will never be called automatically. Add an explicit inline
guard in the `list_software_items` route handler before delegating to the query function:

```rust
if params.query.as_deref().map_or(false, |q| q.len() > 200) {
    return error_response(StatusCode::BAD_REQUEST, "query too long");
}
```

This bounds the LIKE pattern at 200 characters and prevents absurdly large inputs.

### Filter construction

Trim the query, return no filter if the result is empty. Otherwise, lowercase it, escape
all three LIKE-significant characters (`\`, `%`, `_`) using `\` as the escape character,
then build a `%pattern%` bind param. The `\` must be escaped first — if done after `%`,
a user-supplied `\%` would survive as `\\%` in the pattern, and without an `ESCAPE`
clause the trailing `%` remains a wildcard.

```rust
let query = params.query.as_deref().map(str::trim).filter(|q| !q.is_empty());

if let Some(q) = query {
    let escaped = q.to_lowercase()
        .replace('\\', r"\\")  // must be first
        .replace('%',  r"\%")
        .replace('_',  r"\_");
    let pattern = format!("%{escaped}%");
    base_query = base_query.filter(
        sea_orm::sea_query::Expr::cust_with_values(
            "LOWER(name) LIKE ? ESCAPE '\\'",
            [pattern],
        )
    );
}
```

The Rust string `"ESCAPE '\\'"` delivers one backslash byte to the SQL text
(`ESCAPE '\'`), a valid single-character escape literal in standard SQL, SQLite, and
Postgres 9.1+ (standard_conforming_strings = on).

**SQL injection protection:** the SQL string is a static literal. The pattern is a
positional bind parameter (`?`). sea-query's `cust_with_values` tokenizer replaces `?`
with `$1` for Postgres and leaves it as `?` for SQLite — this is its designed purpose.
Since there is no existing use of `cust_with_values` in this codebase, the implementation
must include a Postgres integration test (see Testing) to confirm bind-param substitution
works correctly. If it does not, the fallback is to use two `SimpleExpr::Custom` strings
selected by `#[cfg(feature = "db-postgres")]` / SQLite conditional. SeaORM's `.like()` is
not used because it emits `LIKE ?` with no `ESCAPE` clause.

This filter is added to `base_query` alongside the other filters (`featured`, `host_id`,
`updatable`, `plugin_type`) before the paginator runs.

### Pagination unification

The special in-memory branch (`if let Some(query) = query { … }` / `else { … }`) is
replaced by a single path: apply all filters to `base_query`, then use the standard
`count()` + paginated `all()` pattern unconditionally. The branch disappears entirely.

### Index usage

`idx_software_items_tenant_lower_name ON (tenant_id, lower(name))` already exists. Both
SQLite and Postgres use the composite index to scope to the tenant's rows, then scan ~4k
`lower(name)` values for the LIKE check. No new migration needed.

### Docstring update

Update the `query` field doc in `ListSoftwareItemsParams`:

> Filter by name — case-insensitive substring match, max 200 chars. The caller
> lowercases and escapes LIKE metacharacters before the SQL bind; the database
> evaluates `LOWER(name) LIKE ? ESCAPE '\'`. Empty/whitespace-only values are ignored.

---

## Frontend

**File:** `frontend/src/routes/software/+page.svelte`

All changes follow existing filter patterns in the file.

### State

```svelte
let nameFilter: string = $state(page.url.searchParams.get('query') ?? '');
let nameFilterDebounce: ReturnType<typeof setTimeout> | null = null;
```

`nameFilterDebounce` is a plain `let` — write-only, never read in a reactive expression
or template binding. This matches the existing `searchTimeout` pattern in
`host-tags/+page.svelte`. `nameFilter` is `$state` because it drives both URL
persistence and the API call.

### URL persistence

Add to the `$effect` URL builder:

```svelte
if (isItemsTab && nameFilter) parts.push(`query=${encodeURIComponent(nameFilter)}`);
```

### Tab switching

Add `clearTimeout(nameFilterDebounce); nameFilter = ''` to the non-items tab clearing
block in `switchTab` (alongside the existing `showUpdatableOnly = false;
pluginTypeFilter = ''`). Cancelling the timer prevents a queued `loadAll` from firing
after the tab has switched.

### `loadAll`

Pass `nameFilter || undefined` as the 7th (query) argument to `getSoftwareItems`:

```svelte
const result = await getSoftwareItems(
    page,
    undefined,
    featuredFilter(),
    undefined,
    showUpdatableOnly ? true : undefined,
    pluginTypeFilter || undefined,
    nameFilter || undefined     // ← new
);
```

### Filter UI

Add an `Input` component in the filter row alongside "Updates available" and the plugin
type `Select`. Debounce at 300ms. On fire: `currentPage = 1; loadAll(1)`. Clear the
debounce timer in `onDestroy`.

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

No changes to `getSoftwareItems` in `api.ts`.

---

## Testing

### Backend (integration)

Add test in `crates/ui/web-api/src/integration_tests/software_items_crud.rs`. Test
setup must insert a parent `Tenant` row before any `software_items` (FK constraint).

**Filter correctness:**

- Create 3 items: `"1Password"`, `"Node.js"`, `"nginx"`.
- `query = Some("pass")` → `items = ["1Password"]`, `total = 1`.
- `query = Some("PASS")` (uppercase) → same result (case-insensitive).
- `query = Some("z")` → `items = ["nginx"]`, `total = 1`.
- `query = Some("   ")` (whitespace-only) → all 3 items, no filter applied.

**Metacharacter escaping (literal match, not wildcard):**

- `query = Some("1%")` → `total = 0` (no item contains the literal substring `1%`).
- Create a 4th item named `r"pass\%word"`. Then `query = Some(r"\%")` → only that item
  returned (`\%` is a literal substring, not a wildcard). Confirms the backslash-first
  escape order is correct end-to-end.

**Pagination correctness:**

- Create 5 items whose names match `"app"` and 5 that do not.
- `query = Some("app")`, `per_page = 3`, `page = 1` → `items.len() == 3`,
  `total == 5`.
- Same query, `page = 2` → `items.len() == 2`, `total == 5`.

### Frontend

Add one Svelte test: assert `nameFilter` initializes from URL `?query=foo` param.

---

## Documentation impact

No `CONTEXT.md`, ADR, or README changes required — no new domain terms and no
architectural decisions. The `query` field docstring in `ListSoftwareItemsParams`
is updated inline (described above).

---

## Out of scope

- FTS5 / trigram indexes — unnecessary at current scale.
- Fuzzy/similarity matching — not requested.
- Searching fields other than `name` — `query` field comment leaves room, but no
  metadata fields are searched in this iteration.
