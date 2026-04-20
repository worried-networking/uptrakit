# Manual Software Item Merge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a manual merge workflow for software items so users can choose a survivor, preview transferred vs skipped host links, and execute the
merge with backend-enforced transactional semantics and no schema changes.

**Architecture:** Add a dedicated backend merge surface alongside the existing software-item routes, with merge preview and execute handled in a new
`web-api-queries` module. Extend the existing software list API with a `query` filter for tenant-wide candidate search, and add a reusable Svelte
merge wizard component that both the dashboard and detail page open.

**Tech Stack:** Rust (`uptrakit-web-api-types`, `uptrakit-openapi-client`, `uptrakit-web-api-queries`, `uptrakit-web-api`, SeaORM, Axum, utoipa),
Svelte 5, TypeScript, Vitest, existing Uptrakit test harnesses.

---

## File Map

- Modify: `crates/shared/web-api-types/src/software_items.rs`
  - add merge request/response DTOs
  - extend `ListSoftwareItemsParams` with a text `query` filter
- Modify: `crates/shared/openapi-client/src/software_items.rs`
  - add merge preview / execute client methods
- Modify: `crates/ui/web-api-queries/src/queries/software_items/mod.rs`
  - re-export merge query functions and any helper types
- Create: `crates/ui/web-api-queries/src/queries/software_items/merge.rs`
  - merge preview and transactional execute logic
  - pure helper(s) for host-link equivalence and preview aggregation
- Modify: `crates/ui/web-api-queries/src/queries/software_items/crud.rs`
  - apply the new `query` filter in `list_software_items`
- Modify: `crates/ui/web-api/src/routes/software_items/mod.rs`
  - add `/merge/preview` and `/merge/execute` handlers
  - expose new request/response types for OpenAPI
- Modify: `crates/ui/web-api/src/integration_tests/software_items_crud.rs`
  - add route-level merge coverage
- Modify: `frontend/src/lib/types.ts`
  - add merge DTOs mirrored from shared API
- Modify: `frontend/src/lib/api.ts`
  - add merge API helpers
  - add `query` support to `getSoftwareItems`
- Create: `frontend/src/lib/components/SoftwareMergeWizard.svelte`
  - two-step merge modal/wizard
- Create: `frontend/src/lib/components/SoftwareMergeWizard.test.ts`
  - wizard state, preview rendering, success callback coverage
- Modify: `frontend/src/routes/software/+page.svelte`
  - add batch + row entry points and dashboard-owned modal state
- Modify: `frontend/src/routes/software/[id]/+page.svelte`
  - add detail-page entry point that still resolves to dashboard stay-in-place behavior via callback / navigation choice

## Task 1: Add Shared Merge Contracts and Search Filter

**Files:**

- Modify: `crates/shared/web-api-types/src/software_items.rs`
- Modify: `crates/shared/openapi-client/src/software_items.rs`
- Modify: `frontend/src/lib/types.ts`
- Modify: `frontend/src/lib/api.ts`

- [ ] **Step 1: Write the failing shared-types tests**

Add DTO round-trip coverage near the existing `software_items.rs` tests.

```rust
#[test]
fn list_software_items_params_query_filter() {
    let params: ListSoftwareItemsParams =
        serde_json::from_str(r#"{"query":"node","plugin_type":"releases_docker"}"#)
            .expect("deserialize");
    assert_eq!(params.query.as_deref(), Some("node"));
    assert_eq!(params.plugin_type.as_deref(), Some("releases_docker"));
}

#[test]
fn merge_preview_request_round_trip() {
    let req = MergeSoftwareItemsPreviewRequest {
        candidate_ids: vec![Uuid::nil(), Uuid::now_v7()],
        survivor_id: Uuid::nil(),
        seed_item_id: Some(Uuid::now_v7()),
    };
    let json = serde_json::to_string(&req).expect("serialize");
    let parsed: MergeSoftwareItemsPreviewRequest =
        serde_json::from_str(&json).expect("deserialize");
    assert_eq!(parsed.candidate_ids.len(), 2);
    assert_eq!(parsed.survivor_id, Uuid::nil());
}

#[test]
fn merge_execute_response_round_trip() {
    let resp = MergeSoftwareItemsExecuteResponse {
        survivor_id: Uuid::nil(),
        deleted_ids: vec![Uuid::now_v7()],
        moved_link_ids: vec![Uuid::now_v7()],
        skipped_duplicate_link_ids: vec![Uuid::now_v7()],
    };
    let json = serde_json::to_string(&resp).expect("serialize");
    let parsed: MergeSoftwareItemsExecuteResponse =
        serde_json::from_str(&json).expect("deserialize");
    assert_eq!(parsed.deleted_ids.len(), 1);
}
```

- [ ] **Step 2: Run the shared-types tests to verify they fail**

Run:

```bash
cargo test -p uptrakit-web-api-types merge_preview_request_round_trip -- --nocapture
```

Expected: FAIL with unknown merge request/response types or missing `query` field.

- [ ] **Step 3: Add merge DTOs and the list query field**

Add the new structs in `crates/shared/web-api-types/src/software_items.rs`.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MergeSoftwareItemsPreviewRequest {
    pub candidate_ids: Vec<Uuid>,
    pub survivor_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed_item_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MergeSoftwareItemsPreviewResponse {
    pub candidates: Vec<MergeSoftwareItemCandidate>,
    pub survivor: MergeSoftwareItemCandidate,
    pub losers: Vec<MergeSoftwareItemCandidate>,
    pub moved_links: Vec<MergeSoftwareItemPreviewLink>,
    pub skipped_duplicate_links: Vec<MergeSoftwareItemPreviewLink>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MergeSoftwareItemsExecuteRequest {
    pub candidate_ids: Vec<Uuid>,
    pub survivor_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MergeSoftwareItemsExecuteResponse {
    pub survivor_id: Uuid,
    pub deleted_ids: Vec<Uuid>,
    pub moved_link_ids: Vec<Uuid>,
    pub skipped_duplicate_link_ids: Vec<Uuid>,
}
```

Extend the existing list params:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub query: Option<String>,
```

Update the TS mirror types and API helper signatures.

```ts
export interface MergeSoftwareItemsPreviewRequest {
  candidate_ids: string[];
  survivor_id: string;
  seed_item_id?: string | null;
}

export interface MergeSoftwareItemsExecuteRequest {
  candidate_ids: string[];
  survivor_id: string;
}

export function getSoftwareItems(
  page?: number,
  perPage?: number,
  featured?: boolean,
  hostId?: string,
  updatable?: boolean,
  pluginType?: string,
  query?: string,
): Promise<PaginatedResponse<SoftwareItemResponse>> {
  const params = new URLSearchParams();
  if (page != null) params.set("page", String(page));
  if (perPage != null) params.set("per_page", String(perPage));
  if (featured != null) params.set("featured", String(featured));
  if (hostId != null) params.set("host_id", hostId);
  if (updatable != null) params.set("updatable", String(updatable));
  if (pluginType != null) params.set("plugin_type", pluginType);
  if (query != null && query.trim() !== "") params.set("query", query.trim());
  const qs = params.toString();
  return request(`/software-items${qs ? `?${qs}` : ""}`);
}

export function previewSoftwareItemMerge(req: MergeSoftwareItemsPreviewRequest): Promise<MergeSoftwareItemsPreviewResponse> {
  return request("/software-items/merge/preview", {
    method: "POST",
    body: JSON.stringify(req),
  });
}
```

- [ ] **Step 4: Run the shared contract checks**

Run:

```bash
cargo test -p uptrakit-web-api-types list_software_items_params_query_filter -- --nocapture
cargo test -p uptrakit-web-api-types merge_execute_response_round_trip -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/shared/web-api-types/src/software_items.rs \
        crates/shared/openapi-client/src/software_items.rs \
        frontend/src/lib/types.ts \
        frontend/src/lib/api.ts
git commit -m "feat: add software merge contracts"
```

## Task 2: Add Tenant-Wide Candidate Search to the Existing List Query

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/software_items/crud.rs`
- Test: `crates/ui/web-api/src/integration_tests/software_items_crud.rs`

- [ ] **Step 1: Add the failing route-level list search test**

```rust
#[tokio::test]
async fn list_filters_by_query_case_insensitively() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    for name in ["Node.js", "node exporter", "Redis"] {
        client
            .post_json("/api/v1/software-items", &serde_json::json!({ "name": name }))
            .bearer(&token)
            .send_status()
            .await;
    }

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/software-items?query=node")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 2);
}
```

- [ ] **Step 2: Run the targeted web-api test to verify it fails**

Run:

```bash
cargo test -p uptrakit-web-api list_filters_by_query_case_insensitively -- --nocapture
```

Expected: FAIL because `query` is ignored and all three items are returned.

- [ ] **Step 3: Implement the query filter in `list_software_items`**

Add a case-insensitive substring filter before pagination:

```rust
if let Some(query) = params.query.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
    base_query = base_query.filter(
        Expr::expr(
            sea_orm::sea_query::Func::lower(
                Expr::col(software_item::Column::Name)
            )
        ).like(format!("%{}%", query.to_lowercase()))
    );
}
```

Keep the existing tenant, deactivation, featured, host, updatable, and plugin filters unchanged.

- [ ] **Step 4: Re-run the targeted route test**

Run:

```bash
cargo test -p uptrakit-web-api list_filters_by_query_case_insensitively -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/web-api-queries/src/queries/software_items/crud.rs \
        crates/ui/web-api/src/integration_tests/software_items_crud.rs
git commit -m "feat: add software item query filter"
```

## Task 3: Implement Merge Preview Query Logic

**Files:**

- Create: `crates/ui/web-api-queries/src/queries/software_items/merge.rs`
- Modify: `crates/ui/web-api-queries/src/queries/software_items/mod.rs`

- [ ] **Step 1: Write the failing pure helper tests in `merge.rs`**

Start with pure equivalence logic so the duplicate rules are locked down first.

```rust
#[test]
fn equivalent_links_require_matching_qualifier_semantics() {
    let unqualified_a = MergeLinkIdentity { host_id: Uuid::nil(), qualifier: None };
    let unqualified_b = MergeLinkIdentity { host_id: Uuid::nil(), qualifier: None };
    let qualified = MergeLinkIdentity { host_id: Uuid::nil(), qualifier: Some("web".to_string()) };

    assert!(unqualified_a.equivalent_to(&unqualified_b));
    assert!(!unqualified_a.equivalent_to(&qualified));
}

#[test]
fn preview_groups_moves_and_skips() {
    let survivor_links = vec![MergeLinkIdentity { host_id: Uuid::nil(), qualifier: None }];
    let loser_links = vec![
        MergeLinkIdentity { host_id: Uuid::nil(), qualifier: None },
        MergeLinkIdentity { host_id: Uuid::now_v7(), qualifier: Some("api".to_string()) },
    ];

    let plan = build_transfer_plan(&survivor_links, &loser_links);
    assert_eq!(plan.skipped_duplicate_link_ids.len(), 1);
    assert_eq!(plan.moved_link_ids.len(), 1);
}
```

- [ ] **Step 2: Run the targeted query-crate tests to verify they fail**

Run:

```bash
cargo test -p uptrakit-web-api-queries equivalent_links_require_matching_qualifier_semantics -- --nocapture
```

Expected: FAIL because the new module and helper types do not exist yet.

- [ ] **Step 3: Implement preview types and logic**

Create `merge.rs` with:

```rust
pub async fn preview_merge_software_items(
    tenant_db: &TenantDb,
    req: &MergeSoftwareItemsPreviewRequest,
) -> super::Result<MergeSoftwareItemsPreviewResponse> {
    validate_merge_candidate_ids(req)?;
    let items = load_active_merge_candidates(tenant_db, &req.candidate_ids).await?;
    let survivor = require_survivor(&items, req.survivor_id)?;
    let losers = items
        .iter()
        .filter(|item| item.id != req.survivor_id)
        .cloned()
        .collect::<Vec<_>>();
    let transfer_plan = load_transfer_plan(tenant_db.db(), survivor.id, &losers).await?;
    Ok(build_preview_response(items, survivor, losers, transfer_plan))
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MergeLinkIdentity {
    host_id: Uuid,
    qualifier: Option<String>,
}

impl MergeLinkIdentity {
    fn equivalent_to(&self, other: &Self) -> bool {
        self.host_id == other.host_id && self.qualifier == other.qualifier
    }
}
```

Preview algorithm:

- load all candidate `software_item` rows for the tenant and ensure they are active
- verify `survivor_id` is in `candidate_ids`
- load all `host_software_items` rows for the candidate set
- load host summaries using the existing `load_item_hosts`/host assignment helpers where possible
- compute `moved_links` vs `skipped_duplicate_links` based on `(host_id, qualifier)`
- return explicit survivor, losers, and preview counts

Re-export from `mod.rs`:

```rust
mod merge;
pub use merge::{execute_merge_software_items, preview_merge_software_items};
```

- [ ] **Step 4: Re-run the pure helper tests**

Run:

```bash
cargo test -p uptrakit-web-api-queries equivalent_links_require_matching_qualifier_semantics -- --nocapture
cargo test -p uptrakit-web-api-queries preview_groups_moves_and_skips -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/web-api-queries/src/queries/software_items/mod.rs \
        crates/ui/web-api-queries/src/queries/software_items/merge.rs
git commit -m "feat: add software merge preview logic"
```

## Task 4: Implement Transactional Merge Execution and HTTP Routes

**Files:**

- Modify: `crates/ui/web-api-queries/src/queries/software_items/merge.rs`
- Modify: `crates/ui/web-api/src/routes/software_items/mod.rs`
- Modify: `crates/ui/web-api/src/integration_tests/software_items_crud.rs`

- [ ] **Step 1: Add the failing execute route tests**

```rust
#[tokio::test]
async fn merge_execute_soft_deletes_losers_and_moves_links() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    // Use fixture helpers / direct inserts to create:
    // - survivor item
    // - loser item
    // - distinct host_software_items rows
    // Then call /api/v1/software-items/merge/execute and assert:
    // - 200 OK
    // - loser is no longer returned by GET
    // - survivor detail now includes both host links
}

#[tokio::test]
async fn merge_execute_skips_equivalent_survivor_link() {
    // Arrange survivor and loser with same host_id + same qualifier.
    // Assert execute response reports skipped_duplicate_link_ids and survivor host count is unchanged.
}
```

- [ ] **Step 2: Run the targeted merge execute test to verify it fails**

Run:

```bash
cargo test -p uptrakit-web-api merge_execute_soft_deletes_losers_and_moves_links -- --nocapture
```

Expected: FAIL because the routes do not exist.

- [ ] **Step 3: Implement `execute_merge_software_items` in one transaction**

Inside `merge.rs`:

```rust
pub async fn execute_merge_software_items(
    tenant_db: &TenantDb,
    req: &MergeSoftwareItemsExecuteRequest,
) -> super::Result<MergeSoftwareItemsExecuteResponse> {
    let txn = tenant_db.db().begin().await.context_to()?;
    let preview = preview_merge_software_items(tenant_db, &MergeSoftwareItemsPreviewRequest {
        candidate_ids: req.candidate_ids.clone(),
        survivor_id: req.survivor_id,
        seed_item_id: None,
    }).await?;

    // Move non-duplicate host_software_items to the survivor.
    // Update linked host_software_item_plugins rows via host_software_item_id + software_item_id.
    // Soft-delete losers by setting deactivated_at / updated_at.
    // Commit and return explicit ids.
}
```

Execution details to enforce:

- move `host_software_items.software_item_id` to the survivor for non-duplicates
- update sibling `host_software_item_plugins.software_item_id` to the survivor so the plugin rows stay aligned
- do not mutate the survivor row beyond receiving new links
- soft-delete losers after link transfer

Add routes in `crates/ui/web-api/src/routes/software_items/mod.rs`:

```rust
#[utoipa::path(
    post,
    path = "/api/v1/software-items/merge/preview",
    request_body = MergeSoftwareItemsPreviewRequest,
    responses((status = 200, body = MergeSoftwareItemsPreviewResponse)),
    tag = "Software Items",
    extensions(("x-required-permission" = json!("update_software"))),
    security(("bearer_token" = []))
)]
pub async fn preview_merge_software_items(
    tenant_db: TenantDb,
    CanUpdateSoftware(_user): CanUpdateSoftware,
    Json(req): Json<MergeSoftwareItemsPreviewRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let resp = item_queries::preview_merge_software_items(&tenant_db, &req).await?;
    Ok((StatusCode::OK, Json(resp)))
}

#[utoipa::path(
    post,
    path = "/api/v1/software-items/merge/execute",
    request_body = MergeSoftwareItemsExecuteRequest,
    responses((status = 200, body = MergeSoftwareItemsExecuteResponse)),
    tag = "Software Items",
    extensions(("x-required-permission" = json!("delete_software"))),
    security(("bearer_token" = []))
)]
pub async fn execute_merge_software_items(
    tenant_db: TenantDb,
    CanDeleteSoftware(_user): CanDeleteSoftware,
    Json(req): Json<MergeSoftwareItemsExecuteRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let resp = item_queries::execute_merge_software_items(&tenant_db, &req).await?;
    Ok((StatusCode::OK, Json(resp)))
}
```

- [ ] **Step 4: Re-run the merge route tests**

Run:

```bash
cargo test -p uptrakit-web-api merge_execute_soft_deletes_losers_and_moves_links -- --nocapture
cargo test -p uptrakit-web-api merge_execute_skips_equivalent_survivor_link -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/web-api-queries/src/queries/software_items/merge.rs \
        crates/ui/web-api/src/routes/software_items/mod.rs \
        crates/ui/web-api/src/integration_tests/software_items_crud.rs
git commit -m "feat: add software merge endpoints"
```

## Task 5: Build the Reusable Merge Wizard Component

**Files:**

- Create: `frontend/src/lib/components/SoftwareMergeWizard.svelte`
- Create: `frontend/src/lib/components/SoftwareMergeWizard.test.ts`
- Modify: `frontend/src/lib/api.ts`
- Modify: `frontend/src/lib/types.ts`

- [ ] **Step 1: Write the failing wizard tests**

```ts
it("renders preview sections after clicking Next", async () => {
  const preview = {
    candidates: [candidate("survivor"), candidate("loser")],
    survivor: candidate("survivor"),
    losers: [candidate("loser")],
    moved_links: [previewLink("host-a")],
    skipped_duplicate_links: [previewLink("host-b")],
  };

  render(SoftwareMergeWizard, {
    props: {
      initialCandidates: [candidate("survivor"), candidate("loser")],
      previewMerge: vi.fn().mockResolvedValue(preview),
      executeMerge: vi.fn(),
      onclose: vi.fn(),
      onsuccess: vi.fn(),
    },
  });

  await user.click(screen.getByRole("button", { name: "Next" }));
  expect(await screen.findByText("Keep")).toBeInTheDocument();
  expect(screen.getByText("Affected host links")).toBeInTheDocument();
});

it("calls onsuccess and does not navigate away on execute success", async () => {
  // Arrange preview + execute mocks.
  // Assert onsuccess fires with the execute payload.
});
```

- [ ] **Step 2: Run the component test to verify it fails**

Run:

```bash
cd frontend && npm run test -- SoftwareMergeWizard.test.ts
```

Expected: FAIL because the component does not exist.

- [ ] **Step 3: Implement the two-step wizard**

Create the component with explicit props instead of route coupling:

```svelte
<script lang="ts">
    import Modal from '$lib/components/Modal.svelte';
    import { showError } from '$lib/notifications.svelte';
    let {
        initialCandidates,
        previewMerge,
        executeMerge,
        onclose,
        onsuccess
    } = $props();

    let step = $state<'select' | 'confirm'>('select');
    let candidates = $state(initialCandidates);
    let survivorId = $state(initialCandidates[0]?.id ?? '');
    let preview = $state<MergeSoftwareItemsPreviewResponse | null>(null);
    let searchQuery = $state('');
    let searchResults = $state<SoftwareItemResponse[]>([]);
```

Key UI behavior:

- Step 1 shows selected candidates, survivor radio buttons, and search input
- `Next` calls `previewSoftwareItemMerge`
- Step 2 renders `Keep`, `Delete`, `Moved links`, `Already present`
- `Merge` calls `executeSoftwareItemMerge`
- success invokes `onsuccess` so the parent can refresh the dashboard and toast

- [ ] **Step 4: Re-run the component test**

Run:

```bash
cd frontend && npm run test -- SoftwareMergeWizard.test.ts
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/lib/components/SoftwareMergeWizard.svelte \
        frontend/src/lib/components/SoftwareMergeWizard.test.ts \
        frontend/src/lib/api.ts \
        frontend/src/lib/types.ts
git commit -m "feat: add software merge wizard"
```

## Task 6: Wire the Wizard into the Dashboard Software Page

**Files:**

- Modify: `frontend/src/routes/software/+page.svelte`
- Modify: `frontend/src/lib/components/SoftwareMergeWizard.test.ts`

- [ ] **Step 1: Add a failing batch-origin regression test to the wizard test file**

```ts
it("keeps a multi-item initial selection and lets the user choose a different survivor", async () => {
  const items = [candidate("apt-node"), candidate("docker-node"), candidate("npm-node")];
  render(SoftwareMergeWizard, {
    props: {
      initialCandidates: items,
      previewMerge: vi.fn().mockResolvedValue(previewFor(items, "docker-node")),
      executeMerge: vi.fn(),
      onclose: vi.fn(),
      onsuccess: vi.fn(),
    },
  });

  await user.click(screen.getByLabelText("Keep docker-node"));
  await user.click(screen.getByRole("button", { name: "Next" }));
  expect(await screen.findByText("Delete")).toBeInTheDocument();
  expect(screen.getByText("apt-node")).toBeInTheDocument();
  expect(screen.getByText("npm-node")).toBeInTheDocument();
});
```

- [ ] **Step 2: Implement dashboard-owned merge state**

Add local state next to the existing batch and delete modal state:

```ts
let mergeModalOpen = $state(false);
let mergeInitialCandidates: SoftwareItemResponse[] = $state([]);

function openBatchMerge() {
  mergeInitialCandidates = [...batchSelectedItemsMap.values()];
  mergeModalOpen = true;
}

function openSingleItemMerge(item: SoftwareItemResponse) {
  mergeInitialCandidates = [item];
  mergeModalOpen = true;
}
```

Update the batch actions and row context menu:

```ts
if (selected.length >= 2) {
  acts.push({ id: "merge", label: "Merge" });
}
```

```svelte
<button role="menuitem" onclick={() => openSingleItemMerge(item)}>
    Merge…
</button>
```

Render the wizard:

```svelte
{#if mergeModalOpen}
    <SoftwareMergeWizard
        initialCandidates={mergeInitialCandidates}
        previewMerge={previewSoftwareItemMerge}
        executeMerge={executeSoftwareItemMerge}
        onclose={() => (mergeModalOpen = false)}
        onsuccess={async () => {
            mergeModalOpen = false;
            batchSelectedIds.clear();
            batchSelectedItemsMap.clear();
            showSuccess('Software items merged.');
            await loadAll(currentPage);
        }}
    />
{/if}
```

- [ ] **Step 3: Run the targeted frontend checks**

Run:

```bash
cd frontend && npm run lint
cd frontend && npm run check
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/routes/software/+page.svelte
git commit -m "feat: add dashboard software merge flow"
```

## Task 7: Wire the Wizard into the Software Detail Page

**Files:**

- Modify: `frontend/src/routes/software/[id]/+page.svelte`

- [ ] **Step 1: Add detail-page modal state**

```ts
let mergeModalOpen = $state(false);
```

- [ ] **Step 2: Add the `Merge...` entry point and reuse the wizard**

Add a button near the existing edit / assign / update actions:

```svelte
<button class="btn preset-tonal-surface" onclick={() => (mergeModalOpen = true)}>
    Merge…
</button>
```

Render the wizard with the current detail item as the seed candidate:

```svelte
{#if mergeModalOpen && item}
    <SoftwareMergeWizard
        initialCandidates={[item]}
        previewMerge={previewSoftwareItemMerge}
        executeMerge={executeSoftwareItemMerge}
        onclose={() => (mergeModalOpen = false)}
        onsuccess={async () => {
            mergeModalOpen = false;
            showSuccess('Software items merged.');
            await goto('/software');
        }}
    />
{/if}
```

Note: `goto('/software')` is required here because the approved behavior is to stay on the dashboard after a successful merge regardless of origin.

- [ ] **Step 3: Run the same frontend checks**

Run:

```bash
cd frontend && npm run lint
cd frontend && npm run check
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/routes/software/[id]/+page.svelte
git commit -m "feat: add detail-page software merge entry"
```

## Task 8: Final Verification

**Files:**

- Verify all files touched above

- [ ] **Step 1: Run targeted Rust test suites**

```bash
cargo test -p uptrakit-web-api-types
cargo test -p uptrakit-web-api-queries
cargo test -p uptrakit-web-api software_items_crud -- --nocapture
```

Expected: PASS.

- [ ] **Step 2: Run required repo checks for route/query changes**

```bash
bash ci/verify_handler_state_contract.sh
python3 ci/verify_db_access_policy.py
```

Expected: PASS.

- [ ] **Step 3: Run frontend verification**

```bash
cd frontend && npm run test -- SoftwareMergeWizard.test.ts
cd frontend && npm run lint
cd frontend && npm run check
```

Expected: PASS.

- [ ] **Step 4: Format the workspace**

```bash
cargo fmt --all
```

Expected: PASS with no diff or only formatting-only diff.

- [ ] **Step 5: Final commit**

```bash
git add crates/shared/web-api-types/src/software_items.rs \
        crates/shared/openapi-client/src/software_items.rs \
        crates/ui/web-api-queries/src/queries/software_items/crud.rs \
        crates/ui/web-api-queries/src/queries/software_items/mod.rs \
        crates/ui/web-api-queries/src/queries/software_items/merge.rs \
        crates/ui/web-api/src/routes/software_items/mod.rs \
        crates/ui/web-api/src/integration_tests/software_items_crud.rs \
        frontend/src/lib/types.ts \
        frontend/src/lib/api.ts \
        frontend/src/lib/components/SoftwareMergeWizard.svelte \
        frontend/src/lib/components/SoftwareMergeWizard.test.ts \
        frontend/src/routes/software/+page.svelte \
        frontend/src/routes/software/[id]/+page.svelte
git commit -m "feat: add manual software item merge workflow"
```
