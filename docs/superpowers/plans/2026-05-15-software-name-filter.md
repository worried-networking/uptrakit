# Software Item Name Filtering Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace in-memory `items.retain()` name filtering with a SQL
`LOWER(name) LIKE ? ESCAPE '\'` filter using the sea-query typed `LikeExpr` API,
and add a debounced filter input to the frontend `/software` page.

**Architecture:** Trim, lowercase, and escape LIKE metacharacters (backslash first) in
Rust before building a `Func::lower(col).like(LikeExpr::new(pattern).escape('\\'))`
filter on `base_query`. Remove the two-branch `if let Some(query)` pagination block and
replace with a single `count() + paginated all()` path. Add a 200-char inline guard in
the route handler (`Query(params)` never calls `validate()` automatically — inline guard
is the correct approach for GET query params). Frontend adds `nameFilter` `$state`, a
debounced `Input`, URL persistence in the `$effect` block, and timer cancellation in
`switchTab`.

**Tech Stack:** Rust / SeaORM + sea-query 1.0.0-rc.33 (`LikeExpr`, `Func::lower`),
Axum; Svelte 5 runes; vitest / `@testing-library/svelte`.

---

## Tasks

### Task 1: Add three missing integration tests (failing first)

**Files:**

- Modify: `crates/ui/web-api/src/integration_tests/software_items_crud.rs`

These three tests cover gaps not yet in the file: backslash-escape round-trip,
validation guard (>200 chars → 400), and whitespace-only query (no filter). Write them
before the backend fix so they fail, confirming the gaps.

Existing tests at lines 242–379 already cover: case-insensitive matching, `%` and `_`
as literals, Unicode, and pagination correctness. Do not duplicate them.

- [ ] **Step 1: Append the three new tests after line 380**

```rust
#[tokio::test]
async fn list_query_validates_max_length() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let long_query = "a".repeat(201);
    let url = format!("/api/v1/software-items?query={long_query}");
    let (status, _): (_, serde_json::Value) =
        client.get(&url).bearer(&token).send_json().await;

    assert_eq!(status, http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn list_whitespace_only_query_returns_all() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    for name in ["alpha", "beta"] {
        client
            .post_json("/api/v1/software-items", &serde_json::json!({ "name": name }))
            .bearer(&token)
            .send_status()
            .await;
    }

    // query = "   " (URL-encoded spaces) — whitespace only, must be treated as no filter
    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/software-items?query=%20%20%20")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["total"], 2);
}

#[tokio::test]
async fn list_treats_backslash_as_literal_in_query() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    for name in ["pass\\%word", "password", "normal"] {
        client
            .post_json("/api/v1/software-items", &serde_json::json!({ "name": name }))
            .bearer(&token)
            .send_status()
            .await;
    }

    // query = `\%` (URL-encoded: %5C%25) — literal backslash-percent, not a wildcard
    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/software-items?query=%5C%25")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    let items = body["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "pass\\%word");
}
```

- [ ] **Step 2: Run the three new tests to confirm they fail**

```bash
cargo test --all-features -p uptrakit-web-api \
  list_query_validates_max_length \
  list_whitespace_only_query_returns_all \
  list_treats_backslash_as_literal_in_query \
  -- --nocapture 2>&1 | tail -20
```

Expected: all three FAIL (`list_query_validates_max_length` and
`list_treats_backslash_as_literal_in_query` fail; `list_whitespace_only_query_returns_all`
may pass already since trim+empty-check already exists — verify).

---

### Task 2: Push filter to SQL and add validation guard

**Files:**

- Modify: `crates/ui/web-api/src/routes/software_items/mod.rs`
- Modify: `crates/ui/web-api-queries/src/queries/software_items/crud.rs`

- [ ] **Step 1: Add validation guard in the route handler**

`Query(params)` never calls `params.validate()` automatically. Add an inline guard in
`list_software_items` (line 334 of `crates/ui/web-api/src/routes/software_items/mod.rs`),
before the `match`:

Old:

```rust
pub async fn list_software_items(
    tenant_db: TenantDb,
    CanViewSoftware(_user): CanViewSoftware,
    Query(params): Query<ListSoftwareItemsParams>,
) -> Response {
    match item_queries::list_software_items(&tenant_db, &params).await {
```

New:

```rust
pub async fn list_software_items(
    tenant_db: TenantDb,
    CanViewSoftware(_user): CanViewSoftware,
    Query(params): Query<ListSoftwareItemsParams>,
) -> Response {
    if params.query.as_deref().is_some_and(|q| q.chars().count() > 200) {
        return error_response(StatusCode::BAD_REQUEST, "query too long");
    }
    match item_queries::list_software_items(&tenant_db, &params).await {
```

- [ ] **Step 2: Replace the in-memory filter with `LikeExpr` SQL filter in `crud.rs`**

In `crates/ui/web-api-queries/src/queries/software_items/crud.rs`,
replace lines 616–645 (the `query` extraction + two-branch `if let` block):

Old:

```rust
    let query = params
        .query
        .as_deref()
        .map(str::trim)
        .filter(|query| !query.is_empty());

    let (total, items): (u64, Vec<software_item::Model>) = if let Some(query) = query {
        let mut items = base_query.all(tenant_db.db()).await.context_to()?;
        let query = query.to_lowercase();
        items.retain(|item| item.name.to_lowercase().contains(&query));

        let total = items.len() as u64;
        let offset = pagination.offset() as usize;
        let per_page = pagination.per_page as usize;
        let items: Vec<_> = items.into_iter().skip(offset).take(per_page).collect();
        (total, items)
    } else {
        let total = base_query
            .clone()
            .count(tenant_db.db())
            .await
            .context_to()?;
        let items = base_query
            .offset(Some(pagination.offset()))
            .limit(Some(pagination.per_page))
            .all(tenant_db.db())
            .await
            .context_to()?;
        (total, items)
    };
```

New:

```rust
    if let Some(q) = params.query.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
        let escaped = q
            .to_lowercase()
            .replace('\\', r"\\") // must be first — prevents \% from surviving as wildcard
            .replace('%', r"\%")
            .replace('_', r"\_");
        let pattern = format!("%{escaped}%");
        base_query = base_query.filter(
            sea_orm::sea_query::Func::lower(sea_orm::sea_query::Expr::col(
                software_item::Column::Name,
            ))
            // '\' must be ASCII: sea-query Value::Char renders via `*v as u8` (truncates non-ASCII)
            .like(sea_orm::sea_query::LikeExpr::new(pattern).escape('\\')),
        );
    }

    let total = base_query.clone().count(tenant_db.db()).await.context_to()?;
    let items = base_query
        .offset(Some(pagination.offset()))
        .limit(Some(pagination.per_page))
        .all(tenant_db.db())
        .await
        .context_to()?;
```

`sea_orm::sea_query::Func::lower` and `sea_orm::sea_query::Expr::col` already appear in
this file at line 564 (the `order_by` clause). `LikeExpr` is from the same crate;
`LikeExpr::escape('\\')` passes one backslash byte to the SQL ESCAPE clause, valid in
SQLite and Postgres 9.1+. The typed `LikeExpr` API handles `?`→`$1` dialect differences
natively, so no Postgres-specific bind-param test is needed.

- [ ] **Step 3: Run all query-filter integration tests**

```bash
cargo test --all-features -p uptrakit-web-api list_ -- --nocapture 2>&1 | tail -30
```

Expected: all tests pass, including the three new ones from Task 1.

- [ ] **Step 4: Run full backend quality gate**

```bash
cargo fmt --all && \
cargo check --no-default-features --features db-sqlite && \
cargo check --all-features && \
cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings && \
cargo clippy --all-targets --all-features -- -D warnings && \
cargo test --all-features
```

Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add \
  crates/ui/web-api/src/routes/software_items/mod.rs \
  crates/ui/web-api-queries/src/queries/software_items/crud.rs \
  crates/ui/web-api/src/integration_tests/software_items_crud.rs
git commit -m "feat(software-items): push name filter to SQL with LikeExpr + ESCAPE

Replace in-memory items.retain() with Func::lower(col).like(LikeExpr)
pushing LOWER(name) LIKE ? ESCAPE into base_query. Unify two-branch
pagination into single count()+paginated-all() path. Add 200-char guard
in route handler. Add three integration tests: max-length validation,
whitespace-only passthrough, backslash-literal round-trip."
```

---

### Task 3: Update `query` field docstring

**Files:**

- Modify: `crates/shared/web-api-types/src/software_items.rs`

- [ ] **Step 1: Update the docstring for `query` field (line 532)**

Old:

```rust
    /// Free-text search query applied against item name and related metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
```

New:

```rust
    /// Filter by name — case-insensitive substring match, max 200 chars.
    /// The caller lowercases and escapes LIKE metacharacters before the SQL
    /// bind; the database evaluates `LOWER(name) LIKE ? ESCAPE '\'`.
    /// Empty/whitespace-only values are ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
```

- [ ] **Step 2: Check and commit**

```bash
cargo check --all-features 2>&1 | tail -5
```

Expected: no errors.

```bash
git add crates/shared/web-api-types/src/software_items.rs
git commit -m "docs(software-items): update query field docstring to reflect SQL push"
```

---

### Task 4: Frontend state, URL persistence, and loadAll wiring

**Files:**

- Modify: `frontend/src/routes/software/+page.svelte`

- [ ] **Step 1: Add `nameFilter` state and debounce handle**

After line 93 (`let pluginTypeFilter: string = $state(...)`), add:

```svelte
let nameFilter: string = $state(page.url.searchParams.get('query') ?? '');
let nameFilterDebounce: ReturnType<typeof setTimeout> | undefined;
```

`nameFilter` is `$state` — it drives URL persistence and the API call. `nameFilterDebounce`
is a plain `let` (no `$state`) — timer handles don't need to trigger re-renders. This
matches the `let searchTimeout` pattern in `host-tags/+page.svelte`.

- [ ] **Step 2: Add `nameFilter` to the URL `$effect` builder**

In the `$effect` URL builder (around line 246), after the `pluginTypeFilter` line:

```svelte
if (isItemsTab && showUpdatableOnly) parts.push('updatable=true');
if (isItemsTab && pluginTypeFilter) parts.push(`plugin_type=${encodeURIComponent(pluginTypeFilter)}`);
if (isItemsTab && nameFilter) parts.push(`query=${encodeURIComponent(nameFilter)}`);
if (isItemsTab && currentPage > 1) parts.push(`page=${currentPage}`);
```

- [ ] **Step 3: Cancel debounce timer and clear `nameFilter` in `switchTab`**

In `switchTab` (around line 414), add to the non-items tab clearing block:

Old:

```svelte
    } else {
        showUpdatableOnly = false;
        pluginTypeFilter = '';
    }
```

New:

```svelte
    } else {
        clearTimeout(nameFilterDebounce);
        nameFilterDebounce = undefined;
        nameFilter = '';
        showUpdatableOnly = false;
        pluginTypeFilter = '';
    }
```

- [ ] **Step 4: Pass `nameFilter` as 7th arg to `getSoftwareItems` in `loadAll`**

In `loadAll` (around line 362):

Old:

```svelte
            const result = await getSoftwareItems(
                page,
                undefined,
                featuredFilter(),
                undefined,
                showUpdatableOnly ? true : undefined,
                pluginTypeFilter || undefined
            );
```

New:

```svelte
            const result = await getSoftwareItems(
                page,
                undefined,
                featuredFilter(),
                undefined,
                showUpdatableOnly ? true : undefined,
                pluginTypeFilter || undefined,
                nameFilter || undefined
            );
```

- [ ] **Step 5: Run frontend type-check**

```bash
cd frontend && npm run check 2>&1 | tail -20
```

Expected: no type errors.

---

### Task 5: Add filter Input UI with debounce and onDestroy cleanup

**Files:**

- Modify: `frontend/src/routes/software/+page.svelte`

`Input` is already imported from `$lib/components/forms` (line 74). No new imports needed.

- [ ] **Step 1: Add the Input component to the filter row**

After the closing `{/if}` of the `{#if pluginTypeOptions.length > 0}` block (around
line 1082), before the closing `</div>` of the filter row:

```svelte
<Input
    id="software-name-filter"
    type="search"
    placeholder="Filter by name"
    bind:value={nameFilter}
    oninput={() => {
        clearTimeout(nameFilterDebounce);
        nameFilterDebounce = setTimeout(() => {
            currentPage = 1;
            loadAll(1);
        }, 300);
    }}
/>
```

- [ ] **Step 2: Add timer cancel to the existing `onDestroy`**

The existing `onDestroy` at line 331–335 is:

```svelte
onDestroy(() => {
    for (const unsub of unsubscribers) unsub();
    if (refreshInterval) clearInterval(refreshInterval);
    liveWsHandle?.disconnect();
});
```

Replace it with:

```svelte
onDestroy(() => {
    for (const unsub of unsubscribers) unsub();
    if (refreshInterval) clearInterval(refreshInterval);
    liveWsHandle?.disconnect();
    clearTimeout(nameFilterDebounce);
});
```

- [ ] **Step 3: Run frontend lint and type-check**

```bash
cd frontend && npm run lint && npm run format:check && npm run check 2>&1 | tail -20
```

Expected: clean.

---

### Task 6: Add Svelte test for URL `?query=` initialization

**Files:**

- Create: `frontend/src/routes/software/software-name-filter.test.ts`

- [ ] **Step 1: Write the failing test**

```typescript
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/svelte";

vi.mock("$app/state", () => ({
  page: {
    url: new URL("http://localhost/software?tab=all&query=nginx"),
  },
}));

vi.mock("$app/navigation", () => ({
  goto: vi.fn(),
}));

vi.mock("$lib/auth.svelte", () => ({
  getUser: vi.fn(() => ({
    id: "user-1",
    email: "user@example.com",
    first_name: "Test",
    last_name: "User",
    has_pending_email_change: false,
    permissions: ["view_software"],
  })),
}));

vi.mock("$lib/api", () => ({
  getSoftwareItems: vi.fn(async () => ({
    items: [],
    page: 1,
    per_page: 50,
    total: 0,
    total_pages: 1,
  })),
  deleteSoftwareItem: vi.fn(async () => undefined),
  checkSoftwareItemVersions: vi.fn(async () => undefined),
  updateSoftwareItem: vi.fn(async () => undefined),
  listPluginTypes: vi.fn(async () => []),
  getSoftwareItem: vi.fn(async () => undefined),
  triggerSoftwareUpdate: vi.fn(async () => undefined),
  batchSoftwareItems: vi.fn(async () => undefined),
  executeBatchChunked: vi.fn(async () => undefined),
  previewSoftwareItemMerge: vi.fn(async () => undefined),
  executeSoftwareItemMerge: vi.fn(async () => undefined),
}));

vi.mock("$lib/stores/events.svelte", () => ({
  subscribeToEvent: vi.fn(() => () => {}),
}));

vi.mock("$lib/surfaces/registry.svelte", () => ({
  getSurfaceReadLoading: vi.fn(() => false),
  getSurfaceReadModel: vi.fn(() => undefined),
  getSurfaceReadRequested: vi.fn(() => false),
  getSurfacesBySlot: vi.fn(() => []),
  loadSurfaceReadModels: vi.fn(async () => {}),
}));

import SoftwarePage from "./+page.svelte";
import { getSoftwareItems } from "$lib/api";

describe("software name filter", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it("initializes nameFilter from URL ?query= param", () => {
    render(SoftwarePage);

    const input = screen.getByPlaceholderText(
      "Filter by name",
    ) as HTMLInputElement;
    expect(input.value).toBe("nginx");
  });

  it("passes nameFilter to getSoftwareItems on mount", async () => {
    render(SoftwarePage);

    await vi.waitFor(() => {
      expect(vi.mocked(getSoftwareItems)).toHaveBeenCalledWith(
        expect.any(Number),
        undefined,
        expect.anything(),
        undefined,
        expect.anything(),
        expect.anything(),
        "nginx",
      );
    });
  });
});
```

- [ ] **Step 2: Run test to confirm it fails before Task 4+5 are complete**

```bash
cd frontend && npm run test -- software-name-filter 2>&1 | tail -20
```

Expected: FAIL — `Input` with placeholder "Filter by name" not found.

- [ ] **Step 3: Run test after Task 5 is complete to confirm it passes**

```bash
cd frontend && npm run test -- software-name-filter 2>&1 | tail -20
```

Expected: PASS.

- [ ] **Step 4: Run full frontend quality gate**

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build 2>&1 | tail -30
```

Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add \
  frontend/src/routes/software/+page.svelte \
  frontend/src/routes/software/software-name-filter.test.ts
git commit -m "feat(software): add debounced name filter input to /software page

nameFilter \$state init from ?query= URL param; debounced Input component
(300ms); URL persistence in \$effect; switchTab timer cancel + state clear;
7th-arg passthrough to getSoftwareItems."
```

---

## Execution Order

Tasks are sequential (each depends on the previous):

1. Task 1 — write failing tests (backend)
2. Task 2 — fix backend (tests pass)
3. Task 3 — update docstring
4. Task 4 — frontend state + URL + loadAll wiring
5. Task 5 — frontend UI (Input + onDestroy)
6. Task 6 — write + run Svelte test (verifies Tasks 4+5)

---

## Self-Review Checklist

- [x] **Spec coverage**
  - Validation guard (200-char) → Task 2 Step 1 (`.is_some_and(...)`)
  - Backslash-first escaping → Task 2 Step 2
  - `LikeExpr::escape('\\')` ESCAPE clause → Task 2 Step 2
  - Pagination unification (two-branch → single) → Task 2 Step 2
  - `query` docstring update → Task 3
  - `nameFilter` state + URL init → Task 4 Step 1
  - URL persistence → Task 4 Step 2
  - `switchTab` timer cancel + clear → Task 4 Step 3
  - `loadAll` 7th arg → Task 4 Step 4
  - `Input` component + debounce → Task 5 Step 1
  - `onDestroy` cleanup → Task 5 Step 2
  - Svelte test (URL init) → Task 6
  - Backend integration tests (backslash, validation, whitespace) → Task 1

- [x] **No placeholders** — all code blocks complete

- [x] **Type consistency**
  - `nameFilter: string` throughout
  - `nameFilterDebounce: ReturnType<typeof setTimeout> | undefined` — `clearTimeout`
    accepts `undefined`, valid under strict TypeScript
  - `getSoftwareItems` 7th arg `string | undefined` matches existing signature

- [x] **Standards**
  - `Func::lower` + `LikeExpr` — uses same `sea_orm::sea_query::Func::lower` already
    at line 564 in `crud.rs`; no `cust_with_values` raw SQL
  - `.is_some_and(...)` — Clippy-clean idiom (stabilised Rust 1.70)
  - Inline validation guard (not `Validate` trait) — `Query(params)` never calls
    `validate()`; inline guard is the correct pattern for GET query params
  - `$state` for reactive values; plain `let` for timer handle — matches
    `host-tags/+page.svelte` `let searchTimeout` pattern
  - `nameFilterDebounce = undefined` after each `clearTimeout` — consistent with
    codebase pattern of resetting handles after clearing
  - No new imports in frontend (`Input` already at line 74)
  - No new external dependencies

- [x] **Known accepted trade-off — `count()`/`all()` race window**: a concurrent write
      between the two DB calls can make `total` and `items` reflect different snapshots.
      This race already existed in the original non-filter `else` branch (lines 633–644 of
      `crud.rs`). The plan does not regress on this. A `BEGIN IMMEDIATE` transaction is not
      warranted for a UI list counter.
