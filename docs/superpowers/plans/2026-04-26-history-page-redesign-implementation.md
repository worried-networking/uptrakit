# History Page Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Redesign `/history` to match the approved hybrid
timeline/operations spec, including actor display names, a page-1-only summary
strip, title-adjacent actions, stable dialog-launch semantics, and
design-language-conformant layout.

**Architecture:** Add an optional `actor_name` field to the update-history API
contract and resolve it in the query layer with batched lookups against
existing `users`, `services`, and `system_services` tables. Keep the frontend
redesign route-local inside `frontend/src/routes/history/+page.svelte`,
reusing existing UI primitives (`PageShell`, `SectionCard`, `StatusBadge`,
`Button`, shared terminal modal) and semantic tokens rather than adding new
shared chrome. Drive the UI from derived helpers so summary-strip visibility,
actor-label fallback rules, and modal-retarget behavior are deterministic and
testable.

**Tech Stack:** Rust (`sea-orm`, `axum`, `utoipa`), Svelte 5 runes, Vitest, Tailwind v4 semantic tokens, existing shared UI primitives

---

## File Structure

### Backend / shared contract

- Modify: `crates/shared/web-api-types/src/update_history.rs`
  Purpose: add the optional `actor_name` field to `UpdateHistoryResponse`.
- Modify: `crates/ui/web-api-queries/src/queries/update_history.rs`
  Purpose: batch-resolve display names for update-history actors and populate `actor_name` in both list/detail responses.
- Modify: `crates/ui/cli/src/commands/history.rs`
  Purpose: update `UpdateHistoryResponse` test fixtures so the Rust workspace compiles after the shared type change.

### Frontend contract / route

- Modify: `frontend/src/lib/types.ts`
  Purpose: mirror the new `actor_name` field in the handwritten frontend API type.
- Modify: `frontend/src/routes/history/+page.svelte`
  Purpose: implement the page redesign, actor-label rendering, summary strip, stable modal-launch behavior, and design-language-conformant layout.

### Tests

- Modify: `frontend/src/routes/history/history.test.ts`
  Purpose: cover summary strip visibility/buckets, actor label rendering,
  stable action labels, no `Input Required` badge, no `aria-expanded`, and
  modal retarget/no-op behavior.
- Modify: `frontend/src/routes/history/history-trigger-status.test.ts`
  Purpose: keep helper fixtures aligned with the new `actor_name` field, preserve trigger-modal behavior, and remove selectors that depend on `aria-expanded`.
- Modify: `crates/ui/web-api-queries/src/queries/update_history.rs`
  Purpose: add/update Rust tests for `actor_name` resolution and shared response construction.

## Task 1: Extend The Update-History Contract With `actor_name`

**Files:**

- Modify: `crates/shared/web-api-types/src/update_history.rs`
- Modify: `crates/ui/web-api-queries/src/queries/update_history.rs`
- Modify: `crates/ui/cli/src/commands/history.rs`
- Test: `crates/ui/web-api-queries/src/queries/update_history.rs`

- [ ] **Step 1: Write the failing Rust tests for actor-name propagation**

Add one direct `build_response` test plus list-query and detail-query tests in `crates/ui/web-api-queries/src/queries/update_history.rs`:

```rust
#[test]
fn build_response_includes_actor_name() {
    let now = OffsetDateTime::now_utc();
    let record = update_history::Model {
        id: Uuid::now_v7(),
        tenant_id: Uuid::now_v7(),
        host_id: Uuid::now_v7(),
        software_item_id: Uuid::now_v7(),
        host_software_item_id: None,
        from_version: Some("1.0.0".into()),
        to_version: Some("1.1.0".into()),
        status: update_history::UpdateStatus::Completed,
        output: "done".into(),
        output_bytes: 4,
        actor_type: "user".into(),
        actor_id: "11111111-1111-1111-1111-111111111111".into(),
        execution_owner_service_id: None,
        execution_owner_instance_id: None,
        started_at: Some(now),
        completed_at: Some(now),
        created_at: now,
        update_category: "unknown".into(),
        batch_id: None,
        interactive: false,
        output_truncated: false,
        pre_update_protection_status: None,
        pre_update_protection_summary: None,
        recovery_hint: None,
    };

    let resp = build_response(
        &record,
        "Web Server".into(),
        "Node.js".into(),
        "done".into(),
        Some("Alice Smith".into()),
    );

    assert_eq!(resp.actor_name.as_deref(), Some("Alice Smith"));
}

#[tokio::test]
async fn list_update_history_resolves_user_actor_name() {
    let db = setup_test_db().await;
    let tenant_id = Uuid::now_v7();
    let host_id = Uuid::now_v7();
    let software_item_id = Uuid::now_v7();
    let update_history_id = Uuid::now_v7();
    let user_id = Uuid::now_v7();

    insert_tenant_record(&db, tenant_id).await;
    insert_host_record(&db, tenant_id, host_id).await;
    insert_software_item_record(&db, tenant_id, software_item_id).await;
    user::ActiveModel {
        id: Set(user_id),
        email: Set("alice@example.com".parse().unwrap()),
        first_name: Set("Alice".into()),
        last_name: Set("Smith".into()),
        password_hash: Set(None),
        is_active: Set(true),
        deactivated_at: Set(None),
        created_at: Set(OffsetDateTime::now_utc()),
        updated_at: Set(OffsetDateTime::now_utc()),
    }
    .insert(&db)
    .await
    .expect("insert user");

    update_history::ActiveModel {
        id: Set(update_history_id),
        tenant_id: Set(tenant_id),
        host_id: Set(host_id),
        software_item_id: Set(software_item_id),
        host_software_item_id: Set(None),
        from_version: Set(Some("1.0.0".into())),
        to_version: Set(Some("1.1.0".into())),
        status: Set(update_history::UpdateStatus::Completed),
        output: Set("done".into()),
        output_bytes: Set(4),
        actor_type: Set("user".into()),
        actor_id: Set(user_id.to_string()),
        execution_owner_service_id: Set(None),
        execution_owner_instance_id: Set(None),
        started_at: Set(Some(OffsetDateTime::now_utc())),
        completed_at: Set(Some(OffsetDateTime::now_utc())),
        created_at: Set(OffsetDateTime::now_utc()),
        update_category: Set("unknown".into()),
        batch_id: Set(None),
        interactive: Set(false),
        output_truncated: Set(false),
        pre_update_protection_status: Set(None),
        pre_update_protection_summary: Set(None),
        recovery_hint: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert update history");

    let tenant_db = TenantDb::new(db.clone(), tenant_id);
    let resp = list_update_history(&tenant_db, &UpdateHistoryQuery {
        host_id: None,
        software_item_id: None,
        status: None,
        page: Some(1),
        per_page: Some(20),
    })
    .await
    .expect("list update history");

    assert_eq!(resp.items[0].actor_name.as_deref(), Some("Alice Smith"));
}

#[tokio::test]
async fn get_update_history_resolves_user_actor_name() {
    let db = setup_test_db().await;
    let tenant_id = Uuid::now_v7();
    let host_id = Uuid::now_v7();
    let software_item_id = Uuid::now_v7();
    let update_history_id = Uuid::now_v7();
    let user_id = Uuid::now_v7();

    insert_tenant_record(&db, tenant_id).await;
    insert_host_record(&db, tenant_id, host_id).await;
    insert_software_item_record(&db, tenant_id, software_item_id).await;
    user::ActiveModel {
        id: Set(user_id),
        email: Set("alice@example.com".parse().unwrap()),
        first_name: Set("Alice".into()),
        last_name: Set("Smith".into()),
        password_hash: Set(None),
        is_active: Set(true),
        deactivated_at: Set(None),
        created_at: Set(OffsetDateTime::now_utc()),
        updated_at: Set(OffsetDateTime::now_utc()),
    }
    .insert(&db)
    .await
    .expect("insert user");

    update_history::ActiveModel {
        id: Set(update_history_id),
        tenant_id: Set(tenant_id),
        host_id: Set(host_id),
        software_item_id: Set(software_item_id),
        host_software_item_id: Set(None),
        from_version: Set(Some("1.0.0".into())),
        to_version: Set(Some("1.1.0".into())),
        status: Set(update_history::UpdateStatus::Completed),
        output: Set("done".into()),
        output_bytes: Set(4),
        actor_type: Set("user".into()),
        actor_id: Set(user_id.to_string()),
        execution_owner_service_id: Set(None),
        execution_owner_instance_id: Set(None),
        started_at: Set(Some(OffsetDateTime::now_utc())),
        completed_at: Set(Some(OffsetDateTime::now_utc())),
        created_at: Set(OffsetDateTime::now_utc()),
        update_category: Set("unknown".into()),
        batch_id: Set(None),
        interactive: Set(false),
        output_truncated: Set(false),
        pre_update_protection_status: Set(None),
        pre_update_protection_summary: Set(None),
        recovery_hint: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert update history");

    let tenant_db = TenantDb::new(db.clone(), tenant_id);
    let resp = get_update_history(&tenant_db, update_history_id)
        .await
        .expect("get update history")
        .expect("history item");

    assert_eq!(resp.actor_name.as_deref(), Some("Alice Smith"));
}
```

Also add one service-backed query test and one system-service-backed query test
by mirroring the user fixtures above with `service::ActiveModel` and
`system_service::ActiveModel`, asserting that:

- `actor_type` stays equal to the stored raw DB value
- `actor_name` resolves to the row `friendly_name`
- both `list_update_history()` and `get_update_history()` cover at least one non-user actor path between them

- [ ] **Step 2: Run the Rust tests to verify they fail**

Run:

```bash
cargo test -p uptrakit-web-api-queries update_history -- --nocapture
```

Expected: FAIL with errors mentioning missing `actor_name` on
`UpdateHistoryResponse` and/or `build_response` argument mismatch, not
tenant/host/software foreign-key setup failures.

- [ ] **Step 3: Implement the shared field and batched actor-name lookup**

Update the shared type and query layer.

In `crates/shared/web-api-types/src/update_history.rs`, add the field:

```rust
pub struct UpdateHistoryResponse {
    pub actor_type: String,
    pub actor_id: String,
    pub actor_name: Option<String>,
    pub started_at: OffsetDateTime,
    pub completed_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub update_category: String,
    pub interactive: bool,
    pub output_truncated: bool,
    pub pre_update_protection_status: Option<String>,
    pub pre_update_protection_summary: Option<String>,
    pub recovery_hint: Option<String>,
}
```

In `crates/ui/web-api-queries/src/queries/update_history.rs`, keep
`actor_type` unchanged, add `actor_name` to `build_response`, and add batched
lookup helpers:

```rust
fn build_response(
    record: &update_history::Model,
    host_name: String,
    software_item_name: String,
    output: String,
    actor_name: Option<String>,
) -> UpdateHistoryResponse {
    UpdateHistoryResponse {
        id: record.id,
        host_id: record.host_id,
        host_name,
        software_item_id: record.software_item_id,
        software_item_name,
        from_version: record.from_version.clone(),
        to_version: record.to_version.clone().unwrap_or_default(),
        status: db_status_to_api(&record.status),
        output,
        actor_type: record.actor_type.clone(),
        actor_id: record.actor_id.clone(),
        actor_name,
        started_at: record.started_at.unwrap_or(record.created_at),
        completed_at: record.completed_at,
        created_at: record.created_at,
        update_category: record.update_category.clone(),
        interactive: record.interactive,
        output_truncated: record.output_truncated,
        pre_update_protection_status: record.pre_update_protection_status.clone(),
        pre_update_protection_summary: record.pre_update_protection_summary.clone(),
        recovery_hint: record.recovery_hint.clone(),
    }
}

fn user_display_name(user: &user::Model) -> Option<String> {
    let full = format!("{} {}", user.first_name.trim(), user.last_name.trim())
        .trim()
        .to_string();
    (!full.is_empty()).then_some(full)
}

async fn load_actor_names(
    tenant_db: &TenantDb,
    records: &[update_history::Model],
) -> Result<HashMap<String, String>, sea_orm::DbErr> {
    let actor_ids: Vec<Uuid> = records
        .iter()
        .filter_map(|record| Uuid::parse_str(&record.actor_id).ok())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    if actor_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let user_entries = User::find()
        .filter(user::Column::Id.is_in(actor_ids.clone()))
        .all(tenant_db.db())
        .await?
        .into_iter()
        .filter_map(|row| {
            user_display_name(&row).map(|name| (row.id.to_string(), name))
        })
        .collect::<HashMap<_, _>>();

    let service_entries = Service::find()
        .filter(service::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(service::Column::Id.is_in(actor_ids.clone()))
        .all(tenant_db.db())
        .await?
        .into_iter()
        .map(|row| (row.id.to_string(), row.friendly_name))
        .collect::<HashMap<_, _>>();

    let system_service_entries = SystemService::find()
        .filter(system_service::Column::Id.is_in(actor_ids))
        .all(tenant_db.db())
        .await?
        .into_iter()
        .map(|row| (row.id.to_string(), row.friendly_name))
        .collect::<HashMap<_, _>>();

    let mut presentation = HashMap::new();
    presentation.extend(system_service_entries);
    presentation.extend(service_entries);
    presentation.extend(user_entries);
    Ok(presentation)
}
```

Then wire actor names into both `list_update_history()` and `get_update_history()` without rewriting `actor_type`:

```rust
let actor_names = load_actor_names(tenant_db, &records).await?;
let actor_name = actor_names.get(&record.actor_id).cloned();
let item = build_response(record, host_name, si_name, output, actor_name);
```

For `get_update_history()`, use the same helper against the single record so the detail path cannot drift:

```rust
let actor_names = load_actor_names(tenant_db, std::slice::from_ref(&record)).await?;
let actor_name = actor_names.get(&record.actor_id).cloned();
build_response(&record, host.friendly_name, si_name, output, actor_name)
```

Update `crates/ui/cli/src/commands/history.rs` sample fixtures to include `actor_name: None,`.

- [ ] **Step 4: Run the Rust tests again to verify they pass**

Run:

```bash
cargo test -p uptrakit-web-api-queries update_history -- --nocapture
```

Expected: PASS with the new user, service, and system-service actor-name assertions succeeding.

- [ ] **Step 5: Commit**

```bash
git add crates/shared/web-api-types/src/update_history.rs \
        crates/ui/web-api-queries/src/queries/update_history.rs \
        crates/ui/cli/src/commands/history.rs
git commit -m "feat: add actor names to update history responses"
```

## Task 2: Mirror `actor_name` In Frontend Types And Test Fixtures

**Files:**

- Modify: `frontend/src/lib/types.ts`
- Modify: `frontend/src/routes/history/history.test.ts`
- Modify: `frontend/src/routes/history/history-trigger-status.test.ts`

- [ ] **Step 1: Write the failing frontend tests/fixtures**

Update the history-route fixtures so the intended actor-name behavior is represented directly in the tests:

```ts
const queuedItem = {
  id: 'hist-queued',
  host_id: 'host-1',
  host_name: 'prod-01',
  software_item_id: 'software-1',
  software_item_name: 'nginx',
  from_version: '1.24.0',
  to_version: '1.25.0',
  status: 'queued',
  started_at: '2026-02-01T10:00:00Z',
  completed_at: null,
  output: '',
  output_truncated: true,
  interactive: false,
  actor_type: 'user',
  actor_id: 'actor-1',
  actor_name: 'Alice Smith',
  created_at: '2026-02-01T10:00:00Z'
} satisfies UpdateHistoryResponse;
```

In `history-trigger-status.test.ts`, update `makeHistoryEntry()` so the new
field exists in all mocked payloads, and replace any row-action selectors that
still depend on `aria-expanded` with role/name queries scoped to the relevant
history entry:

```ts
function makeHistoryEntry(overrides: Partial<UpdateHistoryResponse> = {}): UpdateHistoryResponse {
  return {
    id: 'history-1',
    host_id: 'host-1',
    host_name: 'Host One',
    software_item_id: 'software-1',
    software_item_name: 'Demo App',
    from_version: '1.0.0',
    to_version: '1.1.0',
    status: 'completed',
    actor_type: 'user',
    actor_id: adminUser.id,
    actor_name: 'History User',
    started_at: '2024-01-01T00:00:00Z',
    completed_at: '2024-01-01T00:05:00Z',
    output: 'Update finished.',
    created_at: '2024-01-01T00:00:00Z',
    interactive: false,
    output_truncated: false,
    pre_update_protection_status: null,
    pre_update_protection_summary: null,
    recovery_hint: null,
    ...overrides
  };
}
```

For the existing “Additional details” regression, replace:

```ts
const viewLogButton = demoEntry.querySelector('button[aria-expanded="false"]') as HTMLElement;
```

with a stable accessible query such as:

```ts
const viewLogButton = within(demoEntry).getByRole('button', { name: 'View logs' });
```

- [ ] **Step 2: Run the focused frontend tests to verify the type mismatch fails**

Run:

```bash
(cd frontend && npm run check)
```

Expected: FAIL because the new `satisfies UpdateHistoryResponse` fixtures mention `actor_name` before `frontend/src/lib/types.ts` defines it.

- [ ] **Step 3: Add the optional frontend field**

In `frontend/src/lib/types.ts`, mirror the shared contract exactly:

```ts
export interface UpdateHistoryResponse {
  id: string;
  host_id: string;
  host_name: string;
  software_item_id: string;
  software_item_name: string;
  from_version: string | null;
  to_version: string;
  status: UpdateHistoryStatus;
  actor_type: string;
  actor_id: string;
  actor_name?: string | null;
  started_at: string | null;
  completed_at: string | null;
  output: string | null;
  created_at: string;
  interactive: boolean;
  output_truncated: boolean;
  pre_update_protection_status?: string | null;
  pre_update_protection_summary?: string | null;
  recovery_hint?: string | null;
}
```

- [ ] **Step 4: Run the focused frontend tests again**

Run:

```bash
(cd frontend && npm run check)
```

Expected: PASS on type/fixture setup, with any remaining failures now coming from the route markup expectations that Task 3 introduces.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/lib/types.ts \
        frontend/src/routes/history/history.test.ts \
        frontend/src/routes/history/history-trigger-status.test.ts
git commit -m "test: align history frontend types with actor names"
```

## Task 3: Redesign The History Route Markup And Derived State

**Files:**

- Modify: `frontend/src/routes/history/+page.svelte`
- Test: `frontend/src/routes/history/history.test.ts`

- [ ] **Step 1: Write the failing route tests for the redesigned UI**

Add/replace assertions in `frontend/src/routes/history/history.test.ts` for the approved behavior:

```ts
it('renders the summary strip only on page 1 with the all filter', async () => {
  render(HistoryPage);
  await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

  expect(screen.getByText('Running')).toBeInTheDocument();
  expect(screen.getByText('Waiting')).toBeInTheDocument();
  expect(screen.getByText('Failed')).toBeInTheDocument();
  expect(screen.getByText('Completed')).toBeInTheDocument();
});

it('hides the summary strip for non-all filters and later pages', async () => {
  page.url.search = '?status=completed&page=2';
  vi.mocked(api.listUpdateHistory).mockResolvedValue({
    items: [completedItem],
    total: 5,
    page: 2,
    per_page: 25,
    total_pages: 2
  });

  render(HistoryPage);
  await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

  expect(document.querySelector('[data-ui="history-summary-strip"]')).toBeNull();
});

it('does not render the summary strip while the page-1 all-results load is pending', async () => {
  vi.mocked(api.listUpdateHistory).mockImplementation(
    () => new Promise(() => undefined) as ReturnType<typeof api.listUpdateHistory>
  );

  render(HistoryPage);
  expect(screen.getByText('Loading update history…')).toBeInTheDocument();
  expect(document.querySelector('[data-ui="history-summary-strip"]')).toBeNull();
});

it('renders actor display names in collapsed row metadata', async () => {
  render(HistoryPage);
  await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

  const nginxEntry = screen.getByText('nginx on prod-01').closest('article')!;
  expect(nginxEntry).toHaveTextContent('Triggered by user Alice Smith');
});

it('does not render the Input Required badge in the feed', async () => {
  render(HistoryPage);
  await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

  expect(screen.queryByText(/input required/i)).not.toBeInTheDocument();
});

it('falls back to trigger source unknown when actor type is missing', async () => {
  vi.mocked(api.listUpdateHistory).mockResolvedValue({
    items: [{ ...queuedItem, actor_type: '', actor_name: null }],
    total: 1,
    page: 1,
    per_page: 25,
    total_pages: 1
  });

  render(HistoryPage);
  await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

  expect(screen.getByText('Trigger source unknown')).toBeInTheDocument();
});

it('keeps stable visible row action labels after opening the modal', async () => {
  render(HistoryPage);
  await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

  const pgEntry = screen.getByText('postgresql on prod-03').closest('article')!;
  const attachBtn = screen.getByRole('button', { name: 'Attach terminal' });
  await fireEvent.click(attachBtn);
  vi.runOnlyPendingTimers();

  expect(pgEntry).toHaveTextContent('Attach terminal');
  expect(screen.queryByRole('button', { name: /close terminal/i })).not.toBeInTheDocument();
});

it('does not render aria-expanded on row actions', async () => {
  render(HistoryPage);
  await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

  const action = screen.getByRole('button', { name: 'Attach terminal' });
  expect(action).not.toHaveAttribute('aria-expanded');
});
```

- [ ] **Step 2: Run the route tests to verify they fail**

Run:

```bash
(cd frontend && npx vitest run src/routes/history/history.test.ts)
```

Expected: FAIL because the current page still shows the old two-card layout,
old row-action placement, label-toggling behavior, and the `Input Required`
badge.

- [ ] **Step 3: Implement the route redesign with design-language-conformant markup**

In `frontend/src/routes/history/+page.svelte`, add deterministic helpers and route-local tokenized summary cards.

Script additions:

```ts
type SummaryBucket = {
  label: 'Running' | 'Waiting' | 'Failed' | 'Completed';
  value: number;
  tone: 'warning' | 'info' | 'danger' | 'success';
};

const showSummaryStrip = $derived(
  statusFilter === 'all' && currentPage === 1 && !loading && !error
);

const summaryBuckets = $derived.by<SummaryBucket[]>(() => {
  const counts = {
    running: items.filter((item) => item.status === 'in_progress').length,
    waiting: items.filter((item) => item.status === 'queued' || item.status === 'pending').length,
    failed: items.filter((item) => item.status === 'failed').length,
    completed: items.filter((item) => item.status === 'completed').length
  };
  return [
    { label: 'Running', value: counts.running, tone: 'warning' },
    { label: 'Waiting', value: counts.waiting, tone: 'info' },
    { label: 'Failed', value: counts.failed, tone: 'danger' },
    { label: 'Completed', value: counts.completed, tone: 'success' }
  ];
});

function historySummaryValueClass(tone: SummaryBucket['tone']): string {
  switch (tone) {
    case 'warning':
      return 'text-[var(--color-warning)]';
    case 'info':
      return 'text-[var(--color-info)]';
    case 'danger':
      return 'text-[var(--color-danger)]';
    case 'success':
      return 'text-[var(--color-success)]';
  }
}

function historyActorLabel(item: UpdateHistoryResponse): string {
  const normalizedType = item.actor_type?.replaceAll(/[_-]+/g, ' ').trim().toLowerCase();
  const actorName = item.actor_name?.trim();
  if (normalizedType === 'user' && actorName) return `Triggered by user ${actorName}`;
  if (normalizedType === 'scheduler' && actorName) return `Triggered by scheduler ${actorName}`;
  if (actorName) return `Triggered by service ${actorName}`;
  if (normalizedType) return `Triggered by ${normalizedType}`;
  return 'Trigger source unknown';
}

function closeHistoryModal() {
  disconnectStream();
  expandedId = null;
}

function openHistoryModal(id: string) {
  if (expandedId === id) {
    return;
  }

  disconnectStream();
  expandedId = id;

  const item = items.find((entry) => entry.id === id);
  if (item && isLiveStatus(item.status)) {
    setTimeout(() => connectInteractive(id), 0);
  }
}
```

Replace the feed-card body structure so the summary strip is rendered before
the controls card and each row uses a title band + metadata band. Keep
primitives and tokens only; do **not** use `StatCard` because it is always
navigable:

```svelte
{#if showSummaryStrip}
  <section class="grid gap-3 sm:grid-cols-2 lg:grid-cols-4" data-ui="history-summary-strip">
    {#each summaryBuckets as bucket (bucket.label)}
      <div class="rounded-card border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-4 py-4">
        <p class="text-badge font-bold uppercase tracking-badge text-[var(--text-secondary)]">{bucket.label}</p>
        <p class={`mt-1 text-sm font-bold ${historySummaryValueClass(bucket.tone)}`}>
          {bucket.value}
        </p>
      </div>
    {/each}
  </section>
{/if}
```

Row header/body shape:

```svelte
<article class="rounded-panel border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-3 py-3">
  <div class="grid grid-cols-[24px_1fr] gap-3">
    <div class={`flex h-6 w-6 items-center justify-center rounded-card border text-table-body font-bold ${historyStatusGlyphClasses(item.status)}`}>
      {historyStatusGlyph(item.status)}
    </div>
    <div class="space-y-2">
      <div class="flex flex-col gap-2 md:flex-row md:items-start md:justify-between">
        <div class="space-y-0.5">
          <p class="text-table-body font-semibold leading-tight text-[var(--text-primary)]">
            {historyEntryLabel(item)}
          </p>
        </div>
        <Button
          variant="ghost"
          size="sm"
          aria-haspopup="dialog"
          loading={expandedId === item.id && wsState === 'connecting'}
          onclick={() => openHistoryModal(item.id)}
        >
          {item.status === 'in_progress' && item.interactive ? 'Attach terminal' : 'View logs'}
        </Button>
      </div>
      <div class="flex flex-wrap items-center gap-2 text-table-body text-[var(--text-secondary)]">
        <span class="font-mono">{formatVersion(item.from_version, '?')} → {formatVersion(item.to_version)}</span>
        <StatusBadge tone={statusBadgeTone(item.status)} label={statusLabel(item.status)} />
        <span>{formatRelativeTime(item.started_at)}</span>
        <span>{historyActorLabel(item)}</span>
      </div>
    </div>
  </div>
</article>
```

- [ ] **Step 4: Run the route tests again**

Run:

```bash
(cd frontend && npx vitest run src/routes/history/history.test.ts)
```

Expected: PASS on summary-strip visibility, actor-label rendering,
`Trigger source unknown` fallback, removal of the old `Input Required` feed
badge, and stable visible row-action labels.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/routes/history/+page.svelte \
        frontend/src/routes/history/history.test.ts
git commit -m "feat: redesign history page layout"
```

## Task 4: Lock In Modal Retarget Behavior And Regression Coverage

**Files:**

- Modify: `frontend/src/routes/history/+page.svelte`
- Modify: `frontend/src/routes/history/history.test.ts`

- [ ] **Step 1: Add the failing modal-behavior tests**

Add two explicit behavior tests:

```ts
it('does not close the modal when clicking the action for the already-open row', async () => {
  render(HistoryPage);
  await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

  const attachBtn = screen.getByRole('button', { name: 'Attach terminal' });
  await fireEvent.click(attachBtn);
  vi.runOnlyPendingTimers();
  expect(document.querySelector('[data-ui="terminal-shell"]')).toBeInTheDocument();

  await fireEvent.click(attachBtn);
  expect(document.querySelector('[data-ui="terminal-shell"]')).toBeInTheDocument();
});

it('retargets the existing modal when clicking a different row action', async () => {
  render(HistoryPage);
  await waitFor(() => expect(screen.getByText('Update History')).toBeInTheDocument());

  const grafanaButton = screen.getByText('grafana on prod-05').closest('article')!
    .querySelector('button') as HTMLElement;
  await fireEvent.click(grafanaButton);
  expect(await screen.findByRole('dialog', { name: 'grafana on prod-05' })).toBeInTheDocument();

  const secondButton = screen.getByText('nginx on prod-01').closest('article')!
    .querySelector('button') as HTMLElement;
  await fireEvent.click(secondButton);

  expect(await screen.findByRole('dialog', { name: 'nginx on prod-01' })).toBeInTheDocument();
  expect(screen.queryByRole('dialog', { name: 'grafana on prod-05' })).not.toBeInTheDocument();
});
```

- [ ] **Step 2: Run the route tests to verify the current toggle behavior fails**

Run:

```bash
(cd frontend && npx vitest run src/routes/history/history.test.ts)
```

Expected: FAIL if same-row clicks still close the modal, if different-row
clicks do not retarget the single existing modal, or if the route still
renders disclosure semantics such as `aria-expanded`.

- [ ] **Step 3: Keep the explicit modal-launch implementation and refine any failing edge cases**

In `frontend/src/routes/history/+page.svelte`, keep the explicit modal-launch model added in Task 3 and refine it if needed:

```ts
function closeHistoryModal() {
  disconnectStream();
  expandedId = null;
}

function openHistoryModal(id: string) {
  if (expandedId === id) {
    return;
  }

  disconnectStream();
  expandedId = id;

  const item = items.find((entry) => entry.id === id);
  if (item && isLiveStatus(item.status)) {
    setTimeout(() => connectInteractive(id), 0);
  }
}
```

Update terminal close wiring:

```svelte
<TerminalOutput
  bind:this={terminalRef}
  open={true}
  title={`${expandedItem.software_item_name} on ${expandedItem.host_name}`}
  statusLabel={terminalStatusLabelFor(expandedItem)}
  statusTone={terminalStatusToneFor(expandedItem)}
  metadata={terminalMetadataFor(expandedItem)}
  callouts={terminalCalloutsFor(expandedItem)}
  actions={terminalActionsFor(expandedItem)}
  showTerminal={isLiveStatus(expandedItem.status) || Boolean(expandedItem.output)}
  output={expandedItem.output ?? ''}
  onInput={isLiveStatus(expandedItem.status)
    ? (data) => (activeStreamId === expandedItem.id ? activeWsHandle?.sendInput(data) : undefined)
    : undefined}
  onclose={closeHistoryModal}
/>
```

Ensure the route still satisfies all of these conditions:

- no chevron/toggle affordance remains in the row action
- no visible `Close terminal` / `Hide logs` label swapping occurs
- no `aria-expanded` is rendered on the row action
- same-row action click is a no-op
- different-row action click retargets the same modal instance

- [ ] **Step 4: Run the route tests again**

Run:

```bash
(cd frontend && npx vitest run src/routes/history/history.test.ts)
```

Expected: PASS on no-op same-row clicks, retargeted modal behavior, stable visible labels, and absence of `aria-expanded`.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/routes/history/+page.svelte \
        frontend/src/routes/history/history.test.ts
git commit -m "fix: make history row actions stable dialog launches"
```

## Task 5: Run Quality Gates And Design-Language Verification

**Files:**

- Modify: none unless a verification failure requires a targeted fix
- Verify: `frontend/src/routes/history/+page.svelte`
- Verify: `docs/development/ui/README.md`
- Verify: `docs/development/ui/tokens.md`
- Verify: `docs/development/ui/primitives.md`
- Verify: `docs/development/ui/layout.md`

- [ ] **Step 1: Run focused mixed-stack verification**

Run:

```bash
cargo test -p uptrakit-web-api-queries update_history -- --nocapture
(cd frontend && npx vitest run src/routes/history/history.test.ts src/routes/history/history-trigger-status.test.ts)
```

Expected: PASS in both commands.

- [ ] **Step 2: Run required frontend quality gates**

Run:

```bash
(cd frontend && npm run lint)
(cd frontend && npm run format:check)
(cd frontend && npm run check)
(cd frontend && npm run build)
```

Expected: all four commands exit `0`.

- [ ] **Step 3: Run required Rust verification**

Run:

```bash
cargo fmt --all --check
cargo check --no-default-features --features db-sqlite
```

Expected: both commands exit `0`.

- [ ] **Step 4: Perform a design-language pass against the actual route**

Verify these items directly in `frontend/src/routes/history/+page.svelte` against
`docs/development/ui/README.md`, `tokens.md`, `primitives.md`, and `layout.md`:

```text
1. Summary strip uses semantic tokens and named utilities only.
2. Controls stay inside SectionCard/PageShell conventions.
3. Row metadata uses StatusBadge/Button/token utilities instead of ad-hoc palette classes.
4. No new route-level visual pattern was introduced where an existing primitive already fits.
5. The terminal modal remains the existing shared shell.
```

If any item fails, fix it before proceeding.

- [ ] **Step 5: Commit only if Step 4 required follow-up fixes**

If the verification run in Step 4 exposed a defect and Step 4 changed files,
stage and commit those follow-up fixes here. If Step 4 was clean and no files
changed, skip this step.

```bash
git add frontend/src/routes/history/+page.svelte \
        frontend/src/routes/history/history.test.ts \
        frontend/src/routes/history/history-trigger-status.test.ts \
        crates/shared/web-api-types/src/update_history.rs \
        crates/ui/web-api-queries/src/queries/update_history.rs \
        crates/ui/cli/src/commands/history.rs \
        frontend/src/lib/types.ts
git commit -m "chore: verify history page redesign"
```

## Self-Review

### Spec Coverage

- Actor display name contract: Task 1 and Task 2
- Summary-strip data source/taxonomy/loading behavior: Task 3
- Stable `Attach terminal` / `View logs` copy and dialog-launch semantics: Task 4
- No guessed `Input Required` feed badge: Task 3
- Responsive and design-language conformance: Task 3 and Task 5

### Placeholder Scan

- No `TODO`, `TBD`, or “similar to previous task” references remain.
- Each task contains exact file paths, concrete code snippets, run commands, and expected outcomes.

### Type Consistency

- Shared/backend/frontend all use the same wire-contract name: `actor_name`
- Frontend actor-label helper uses `actor_type` + optional `actor_name`
- Summary taxonomy uses `Running`, `Waiting`, `Failed`, `Completed` consistently

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-04-26-history-page-redesign-implementation.md`. Two execution options:

**1. Subagent-Driven (recommended)** - I dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints

**Which approach?**
