# Update Trigger UX — Frontend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship live status badges, SSE-driven in-place cache updates, 409 duplicate-trigger handling, and auto-open terminal removal across both
software pages.

**Architecture:** TypeScript-first changes flow bottom-up — shared types → api helpers → shared stores → component → route pages. Each task is
independently committable and compilable. The Svelte 5 spread/replace pattern (`itemDetailsById.set(id, { ...detail, hosts: detail.hosts.map(...) })`)
is used everywhere in-place mutation is needed to guarantee reactivity in `SvelteMap`.

**Tech Stack:** Svelte 5 (runes), TypeScript, Vitest + Testing Library, `SvelteMap`/`SvelteSet` for reactive collections.

---

## File Map

| File                                                           | Change                                                                                                                                                                  |
| -------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `frontend/src/lib/types.ts`                                    | Add `'awaiting_restart'` to `UpdateHistoryStatus`; add `active_update_status?: string` to `SoftwareItemHostSummary`                                                     |
| `frontend/src/lib/stores/events.svelte.ts`                     | Fix debounce key to include `subId` (prevents two `UpdateTriggered` events for same host/different items collapsing)                                                    |
| `frontend/src/lib/stores/events.svelte.test.ts`                | Add test: two `UpdateTriggered` with different `software_item_id` on same host must fire separately                                                                     |
| `frontend/src/lib/api.ts`                                      | Add `ApiError` class; add private `extractApiError()`; update `request()` and `requestVoid()` to throw `ApiError`                                                       |
| `frontend/src/lib/api.test.ts`                                 | Add tests for `ApiError` construction and `request()` throwing `ApiError`                                                                                               |
| `frontend/src/lib/components/ui/SoftwareGroupList.svelte`      | Add `onOpenLiveModal` prop; four-way badge rendering per-host; add `allUpdatableHostsActive()`; update `UpdateAllButton` disable logic                                  |
| `frontend/src/routes/software/+page.svelte`                    | Remove auto-open state/handlers; wire `onOpenLiveModal`; update modal checkbox + selection init; add SSE cache handlers; add 409 handling                               |
| `frontend/src/routes/software/[id]/+page.svelte`               | Remove auto-open state/handlers; add SSE cache handlers; four-way badge block + `versionStatusLabel`/`versionStatusTone`; update `updateAllHostItems`; add 409 handling |
| `frontend/src/routes/software/software-trigger-status.test.ts` | Add tests: active-status badge rendering, "Update All" disable, 409 toast, SSE in-place updates                                                                         |

---

### Task 1: TypeScript types + SSE debounce key fix

**Files:**

- Modify: `frontend/src/lib/types.ts`
- Modify: `frontend/src/lib/stores/events.svelte.ts`
- Modify: `frontend/src/lib/stores/events.svelte.test.ts`

- [ ] **Step 1: Add `'awaiting_restart'` to `UpdateHistoryStatus` and `active_update_status` to `SoftwareItemHostSummary`**

In `frontend/src/lib/types.ts`, line 627:

```typescript
// Before:
export type UpdateHistoryStatus = "queued" | "pending" | "in_progress" | "completed" | "failed";

// After:
export type UpdateHistoryStatus = "queued" | "pending" | "in_progress" | "awaiting_restart" | "completed" | "failed";
```

In `frontend/src/lib/types.ts`, `SoftwareItemHostSummary` interface (currently ends at `plugins: HostPluginRoleSummary[];`). Add the new field after
`active_update_history_id`:

```typescript
export interface SoftwareItemHostSummary {
  id: string;
  host_id: string;
  hostname: string;
  friendly_name: string;
  qualifier?: string | null;
  installed_version: string | null;
  installed_version_detected_at: string | null;
  installed_display_version?: string | null;
  latest_version?: string | null;
  latest_release_metadata?: Record<string, unknown> | null;
  update_available: boolean;
  active_update_history_id?: string | null;
  active_update_status?: string | null;
  last_updated_at: string | null;
  linked_at: string;
  plugins: HostPluginRoleSummary[];
}
```

- [ ] **Step 2: Fix SSE debounce key in `events.svelte.ts`**

In `frontend/src/lib/stores/events.svelte.ts`, update `dispatchEvent` (currently at line 95):

```typescript
function dispatchEvent(eventType: AdminEventType, data: Record<string, unknown>) {
  const entityId = (data.id ?? data.host_id ?? data.task_id ?? "") as string;
  const subId = (data.update_history_id ?? data.software_item_id ?? "") as string;
  const key = `${eventType}:${entityId}:${subId}`;

  const existing = debounceTimers.get(key);
  if (existing) {
    clearTimeout(existing);
  }

  const timer = setTimeout(() => {
    debounceTimers.delete(key);
    for (const sub of subscriptions) {
      if (sub.eventType === eventType) {
        sub.callback(data);
      }
    }
  }, DEBOUNCE_MS);

  debounceTimers.set(key, timer);
}
```

- [ ] **Step 3: Write the failing test**

Append to `frontend/src/lib/stores/events.svelte.test.ts` (inside the existing `describe` block or as a new block):

```typescript
it("two UpdateTriggered events with different software_item_id on same host are not debounced together", async () => {
  const { subscribeToEvent } = await import("$lib/stores/events.svelte");

  const received: unknown[] = [];
  const unsub = subscribeToEvent(AdminEventType.UpdateTriggered, (data) => {
    received.push(data);
  });

  capturedOnEvent?.(AdminEventType.UpdateTriggered, {
    host_id: "host-1",
    software_item_id: "item-A",
    update_history_id: "hist-A",
    status: "pending",
  });
  capturedOnEvent?.(AdminEventType.UpdateTriggered, {
    host_id: "host-1",
    software_item_id: "item-B",
    update_history_id: "hist-B",
    status: "pending",
  });

  await vi.advanceTimersByTimeAsync(200);
  expect(received).toHaveLength(2);

  unsub();
});
```

- [ ] **Step 4: Run the test to verify it fails (debounce collapses the two events)**

```bash
cd frontend && npm run test -- --reporter=verbose events.svelte
```

Expected: FAIL — `received` has length 1 before the fix, length 2 after.

- [ ] **Step 5: Verify the fix makes the test pass**

The step 2 edit already implements the fix. Re-run:

```bash
cd frontend && npm run test -- --reporter=verbose events.svelte
```

Expected: PASS.

- [ ] **Step 6: Type-check**

```bash
cd frontend && npm run check
```

Expected: no errors.

- [ ] **Step 7: Commit**

```bash
git add frontend/src/lib/types.ts frontend/src/lib/stores/events.svelte.ts frontend/src/lib/stores/events.svelte.test.ts
git commit -m "feat(frontend): add active_update_status to SoftwareItemHostSummary + fix SSE debounce key"
```

---

### Task 2: ApiError class

**Files:**

- Modify: `frontend/src/lib/api.ts`
- Modify: `frontend/src/lib/api.test.ts`

- [ ] **Step 1: Write the failing tests**

In `frontend/src/lib/api.test.ts`, find the existing `describe('extractErrorMessage', ...)` block and append a new describe block after it:

```typescript
// ── ApiError ──────────────────────────────────────────────────────────────────
import { ApiError } from "./api";

describe("ApiError", () => {
  it("carries errorCode and status", () => {
    const err = new ApiError("Already active", 409, "trigger_update.update_already_active");
    expect(err).toBeInstanceOf(Error);
    expect(err).toBeInstanceOf(ApiError);
    expect(err.message).toBe("Already active");
    expect(err.status).toBe(409);
    expect(err.errorCode).toBe("trigger_update.update_already_active");
    expect(err.name).toBe("ApiError");
  });

  it("accepts null errorCode", () => {
    const err = new ApiError("Not found", 404, null);
    expect(err.errorCode).toBeNull();
  });
});
```

- [ ] **Step 2: Run tests to confirm they fail (ApiError not yet exported)**

```bash
cd frontend && npm run test -- --reporter=verbose api.test
```

Expected: FAIL — `ApiError` is not exported.

- [ ] **Step 3: Add `ApiError`, `extractApiError()`, and update `request()`/`requestVoid()`**

In `frontend/src/lib/api.ts`, add `ApiError` class and `extractApiError` helper before the `request` function (i.e., after `extractErrorMessage`):

```typescript
/**
 * Error thrown for non-OK HTTP responses. Carries the HTTP status and the
 * server-provided `error_code` field (if present) so callers can distinguish
 * specific failure kinds without string-matching the message.
 */
export class ApiError extends Error {
  public readonly errorCode: string | null;
  public readonly status: number;

  constructor(message: string, status: number, errorCode: string | null) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.errorCode = errorCode;
  }
}

async function extractApiError(res: Response): Promise<ApiError> {
  const text = await res.text();
  let message: string = res.statusText;
  let errorCode: string | null = null;
  if (text) {
    try {
      const parsed = JSON.parse(text);
      if (typeof parsed === "object" && parsed !== null) {
        if (typeof parsed.error === "string") {
          message = truncateError(parsed.error);
        }
        if (typeof parsed.error_code === "string") {
          errorCode = parsed.error_code;
        }
      }
    } catch {
      message = truncateError(text);
    }
  }
  return new ApiError(message, res.status, errorCode);
}
```

Update `request<T>()` — replace `throw new Error(message)`:

```typescript
/** Performs an authenticated request and parses the JSON response body. */
async function request<T>(path: string, options: RequestInit = {}): Promise<T> {
  let res: Response;
  try {
    res = await authenticatedFetch(`${BASE}${path}`, options);
  } catch (err) {
    if (err instanceof DOMException && (err.name === "AbortError" || err.name === "TimeoutError")) {
      throw new Error("Request timed out. Please try again.");
    } else if (err instanceof TypeError) {
      throw new Error("Network error: Unable to connect to the server. Check your network connection.");
    }
    throw err;
  }
  if (!res.ok) {
    throw await extractApiError(res);
  }
  return res.json();
}
```

Update `requestVoid()` — replace `throw new Error(message)`:

```typescript
/** Performs an authenticated request expecting no response body (204 or empty). */
async function requestVoid(path: string, options: RequestInit = {}): Promise<void> {
  let res: Response;
  try {
    res = await authenticatedFetch(`${BASE}${path}`, options);
  } catch (err) {
    if (err instanceof DOMException && (err.name === "AbortError" || err.name === "TimeoutError")) {
      throw new Error("Request timed out. Please try again.");
    } else if (err instanceof TypeError) {
      throw new Error("Network error: Unable to connect to the server. Check your network connection.");
    }
    throw err;
  }
  if (!res.ok) {
    throw await extractApiError(res);
  }
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd frontend && npm run test -- --reporter=verbose api.test
```

Expected: PASS.

- [ ] **Step 5: Type-check and lint**

```bash
cd frontend && npm run check && npm run lint
```

Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/lib/api.ts frontend/src/lib/api.test.ts
git commit -m "feat(frontend): add ApiError class with errorCode for structured error handling"
```

---

### Task 3: Remove auto-open terminal from both pages

**Files:**

- Modify: `frontend/src/routes/software/+page.svelte`
- Modify: `frontend/src/routes/software/[id]/+page.svelte`

The goal: users open the live terminal by clicking "In Progress" badge. Remove the server-push auto-open path that fired on `UpdateProtectionStarted`
and `UpdateStarted`.

- [ ] **Step 1: Remove auto-open state from `/software/+page.svelte`**

Remove the three state declarations at lines 159–161:

```typescript
// Remove these three lines:
let pendingLiveHistoryId: string | null = $state(null);
let pendingLiveHostName: string = $state("");
let pendingLiveItemName: string = $state("");
```

- [ ] **Step 2: Remove auto-open SSE handlers from `/software/+page.svelte`**

In `onMount`, replace the two handlers at lines 294–315 (the `UpdateProtectionStarted` and `UpdateStarted` subscriptions with auto-open logic) with
nothing. The list is now:

```typescript
unsubscribers.push(
  subscribeToEvent(AdminEventType.SoftwareItemUpdated, () => loadAll(currentPage, true)),
  subscribeToEvent(AdminEventType.SoftwareItemCreated, () => loadAll(currentPage, true)),
  subscribeToEvent(AdminEventType.VersionCheckCompleted, () => loadAll(currentPage, true)),
  subscribeToEvent(AdminEventType.UpdateCompleted, () => loadAll(currentPage, true)),
);
```

(The `UpdateCompleted`, `UpdateTriggered`, and `UpdateStarted` in-place handlers are added in Task 5.)

- [ ] **Step 3: Remove `pendingLive*` assignments in `executeSingleHostUpdate`**

In `executeSingleHostUpdate` (around line 644), after the `showSuccess(...)` call, remove the three lines that set `pendingLiveHistoryId`,
`pendingLiveHostName`, `pendingLiveItemName`. The function body becomes:

```typescript
async function executeSingleHostUpdate() {
  if (!singleHostUpdateModal || singleHostUpdateTriggering || !canTriggerUpdates) return;
  singleHostUpdateTriggering = true;
  try {
    const { host, toVersion, itemId, itemName } = singleHostUpdateModal;
    const res = await triggerSoftwareUpdate(itemId, host.host_id, {
      to_version: toVersion,
    });
    singleHostUpdateModal = null;
    if (res.status === "failed") {
      showError(`Update failed before dispatch — history ID: ${res.update_history_id}`);
      void loadAll(currentPage, true);
      return;
    }
    showSuccess(`Update triggered — history ID: ${res.update_history_id}`);
    void loadAll(currentPage, true);
  } catch (e) {
    showError(e instanceof Error ? e.message : "Failed to trigger update");
  } finally {
    singleHostUpdateTriggering = false;
  }
}
```

Note: the unused `itemName` destructuring can be removed too since it's no longer needed.

- [ ] **Step 4: Remove auto-open state from `/software/[id]/+page.svelte`**

Remove lines 154–155:

```typescript
// Remove these two lines:
let pendingLiveHistoryId: string | null = $state(null);
let pendingLiveHostName: string = $state("");
```

- [ ] **Step 5: Remove auto-open SSE handlers from `/software/[id]/+page.svelte`**

In `onMount`, remove the auto-open `UpdateProtectionStarted` and `UpdateStarted` handlers (lines 290–308). Keep the body of each handler that calls
`loadItem(true)` — this will be replaced by in-place updates in Task 6. For now the handlers become:

```typescript
subscribeToEvent(AdminEventType.UpdateCompleted, (data) => {
    if (data.software_item_id === id) loadItem(true);
}),
subscribeToEvent(AdminEventType.UpdateTriggered, (data) => {
    if (data.software_item_id === id) loadItem(true);
}),
```

(The detail page has no `UpdateProtectionStarted` that needs to stay. `UpdateStarted` used to call `loadItem(true)` — leave that in temporarily. Task
6 replaces these with in-place updates.)

- [ ] **Step 6: Remove `pendingLive*` assignments in `executeUpdate`**

In `executeUpdate` (around line 563 in `[id]/+page.svelte`), remove the lines that set `pendingLiveHistoryId` and `pendingLiveHostName`. The function
body becomes:

```typescript
async function executeUpdate() {
  if (!item || !updateModal || updateTriggering || !canTriggerUpdates) return;
  updateTriggering = true;
  try {
    const res = await triggerSoftwareUpdate(item.id, updateModal.host.host_id, {
      to_version: updateModal.toVersion,
    });
    updateModal = null;

    if (res.status === "failed") {
      showError(`Update failed before dispatch — history ID: ${res.update_history_id}`);
      await loadItem(true);
      return;
    }

    showSuccess(`Update triggered — history ID: ${res.update_history_id}`);
  } catch (e) {
    showError(e instanceof Error ? e.message : "Failed to trigger update");
  } finally {
    updateTriggering = false;
  }
}
```

- [ ] **Step 7: Type-check**

```bash
cd frontend && npm run check
```

Expected: no errors (no references to removed state variables should remain).

- [ ] **Step 8: Commit**

```bash
git add "frontend/src/routes/software/+page.svelte" "frontend/src/routes/software/[id]/+page.svelte"
git commit -m "feat(frontend): remove auto-open terminal on update trigger"
```

---

### Task 4: SoftwareGroupList — badge rendering, onOpenLiveModal prop, allUpdatableHostsActive

**Files:**

- Modify: `frontend/src/lib/components/ui/SoftwareGroupList.svelte`

The per-host badge at line 417 currently shows a single "Update" action badge or "Update avail" / "Up to date" status badge. Replace it with a
four-way active-status decision tree, add the `onOpenLiveModal` callback prop, and add `allUpdatableHostsActive` for the group-level button.

- [ ] **Step 1: Add `onOpenLiveModal` prop to the props destructuring**

In `SoftwareGroupList.svelte`, update the props block (starting at line 10):

```typescript
let {
  items,
  itemDetailsById,
  itemDetailLoadingIds,
  collapsedGroupIds,
  expandedOverflowGroupIds,
  batchSelectedIds,
  canManage,
  canTriggerUpdates,
  pluginTypeNames,
  totalItems,
  currentPage,
  totalPages,
  onToggleGroup,
  onToggleOverflow,
  onToggleBatch,
  onOpenMenu,
  onOpenUpdateAllModal,
  onOpenSingleHostUpdate,
  onOpenLiveModal,
  onPageChange,
  onToggleFeatured,
  showUpdatableOnly = false,
}: {
  items: SoftwareItemResponse[];
  itemDetailsById: SvelteMap<string, SoftwareItemDetailResponse>;
  itemDetailLoadingIds: SvelteSet<string>;
  collapsedGroupIds: SvelteSet<string>;
  expandedOverflowGroupIds: SvelteSet<string>;
  batchSelectedIds: SvelteSet<string>;
  canManage: boolean;
  canTriggerUpdates: boolean;
  pluginTypeNames: Map<string, string>;
  totalItems: number;
  currentPage: number;
  totalPages: number;
  onToggleGroup: (id: string) => void;
  onToggleOverflow: (id: string) => void;
  onToggleBatch: (id: string) => void;
  onOpenMenu: (id: string, button: HTMLElement) => void;
  onOpenUpdateAllModal: (item: SoftwareItemResponse) => void;
  onOpenSingleHostUpdate: (item: SoftwareItemResponse, host: SoftwareItemHostSummary) => void;
  onOpenLiveModal: (updateHistoryId: string, hostName: string, itemName: string) => void;
  onPageChange: (page: number) => void;
  onToggleFeatured: (item: SoftwareItemResponse) => void;
  showUpdatableOnly?: boolean;
} = $props();
```

- [ ] **Step 2: Add `allUpdatableHostsActive` helper function**

After the existing `hasAnyUpdateableHosts` function (around line 97), add:

```typescript
function allUpdatableHostsActive(item: SoftwareItemResponse): boolean {
  const detail = itemDetailsById.get(item.id);
  if (!detail) return false;
  const updatable = detail.hosts.filter((h) => h.update_available && h.latest_version);
  return updatable.length > 0 && updatable.every((h) => !!h.active_update_history_id);
}
```

- [ ] **Step 3: Update `UpdateAllButton` disable logic**

Find the `UpdateAllButton` render block (around line 313):

```svelte
<!-- Before: -->
<UpdateAllButton
    state={hasAnyUpdateableHosts(item) ? 'idle' : 'dim'}
    ariaLabel={hasAnyUpdateableHosts(item) ? undefined : 'No updates available'}
    onclick={() => onOpenUpdateAllModal(item)}
/>

<!-- After: -->
{@const anyUpdatable = hasAnyUpdateableHosts(item)}
{@const allActive = allUpdatableHostsActive(item)}
<UpdateAllButton
    state={anyUpdatable && !allActive ? 'idle' : 'dim'}
    ariaLabel={!anyUpdatable ? 'No updates available' : allActive ? 'All hosts already updating' : undefined}
    onclick={() => onOpenUpdateAllModal(item)}
/>
```

- [ ] **Step 4: Replace per-host badge block with four-way status decision tree**

Find the per-host badge block (around line 416–429):

```svelte
<!-- Before: -->
<div class="flex justify-end">
    {#if host.update_available && canTriggerUpdates}
        <ActionBadge
            variant="navigation"
            tone="accent"
            idleLabel="Update"
            hoverLabel="Update"
            onclick={() => onOpenSingleHostUpdate(item, host)}
        />
    {:else if host.update_available}
        <StatusBadge tone="info" label="Update avail" />
    {:else}
        <StatusBadge tone="success" label="Up to date" />
    {/if}
</div>

<!-- After: -->
<div class="flex justify-end">
    {#if host.active_update_history_id}
        {#if host.active_update_status === 'in_progress'}
            <ActionBadge
                variant="navigation"
                tone="info"
                idleLabel="In Progress"
                hoverLabel="View"
                onclick={() => onOpenLiveModal(host.active_update_history_id!, host.hostname, item.name)}
            />
        {:else if host.active_update_status === 'queued'}
            <StatusBadge tone="info" label="Queued" />
        {:else if host.active_update_status === 'pending'}
            <StatusBadge tone="info" label="Pending" />
        {:else if host.active_update_status === 'awaiting_restart'}
            <StatusBadge tone="info" label="Awaiting Restart" />
        {:else}
            <StatusBadge tone="info" label="In Progress" />
        {/if}
    {:else if host.update_available && canTriggerUpdates}
        <ActionBadge
            variant="navigation"
            tone="accent"
            idleLabel="Update"
            hoverLabel="Update"
            onclick={() => onOpenSingleHostUpdate(item, host)}
        />
    {:else if host.update_available}
        <StatusBadge tone="info" label="Update avail" />
    {:else}
        <StatusBadge tone="success" label="Up to date" />
    {/if}
</div>
```

The `{:else}` fallback inside the `{#if host.active_update_history_id}` block handles the case where `active_update_status` is
`null`/`undefined`/unrecognized — backend always sets it alongside the ID, but this defends against stale cache.

- [ ] **Step 5: Type-check**

```bash
cd frontend && npm run check
```

Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/lib/components/ui/SoftwareGroupList.svelte
git commit -m "feat(frontend): add live status badges + allUpdatableHostsActive to SoftwareGroupList"
```

---

### Task 5: Wire /software/+page.svelte — onOpenLiveModal, modal, SSE cache, 409 handling

**Files:**

- Modify: `frontend/src/routes/software/+page.svelte`
- Modify: `frontend/src/routes/software/software-trigger-status.test.ts`

- [ ] **Step 1: Write the failing tests**

In `frontend/src/routes/software/software-trigger-status.test.ts`, add a new `describe` block for the new behaviors:

```typescript
// At the top of the file, update the api mock to include ApiError:
import { ApiError } from "$lib/api";

// In the api mock:
vi.mock("$lib/api", () => ({
  getSoftwareItems: vi.fn(),
  deleteSoftwareItem: vi.fn(),
  checkSoftwareItemVersions: vi.fn(),
  updateSoftwareItem: vi.fn(),
  listPluginTypes: vi.fn(),
  getSoftwareItem: vi.fn(),
  triggerSoftwareUpdate: vi.fn(),
  batchSoftwareItems: vi.fn(),
  executeBatchChunked: vi.fn(),
  previewSoftwareItemMerge: vi.fn(),
  executeSoftwareItemMerge: vi.fn(),
  ApiError, // re-export so instanceof checks work
}));
```

Add tests in a new `describe` block:

```typescript
describe("active update status badge rendering", () => {
  it('shows "In Progress" ActionBadge (not "Update") when host has active_update_history_id with in_progress status', async () => {
    const item = makeSoftwareItem("software-1", "Demo App");
    item.host_count = 1;
    const host = {
      ...makeHostSummary("host-1", "host-one"),
      active_update_history_id: "hist-abc",
      active_update_status: "in_progress",
    };
    const detail = makeDetail(item, [host]);

    vi.mocked(api.getSoftwareItems).mockResolvedValue(makeItemsPage([item]));
    vi.mocked(api.getSoftwareItem).mockResolvedValue(detail);

    render(SoftwarePage);
    await waitFor(() => expect(api.getSoftwareItems).toHaveBeenCalled());

    // Expand the group to see host rows
    const header = await screen.findByText("Demo App");
    await fireEvent.click(header);

    await waitFor(() => expect(screen.queryByText("In Progress")).toBeInTheDocument());
    expect(screen.queryByText("Update")).not.toBeInTheDocument();
  });

  it('shows "Queued" StatusBadge when active_update_status is queued', async () => {
    const item = makeSoftwareItem("software-1", "Demo App");
    item.host_count = 1;
    const host = {
      ...makeHostSummary("host-1", "host-one"),
      active_update_history_id: "hist-abc",
      active_update_status: "queued",
    };
    const detail = makeDetail(item, [host]);

    vi.mocked(api.getSoftwareItems).mockResolvedValue(makeItemsPage([item]));
    vi.mocked(api.getSoftwareItem).mockResolvedValue(detail);

    render(SoftwarePage);
    await waitFor(() => expect(api.getSoftwareItems).toHaveBeenCalled());

    const header = await screen.findByText("Demo App");
    await fireEvent.click(header);

    await waitFor(() => expect(screen.queryByText("Queued")).toBeInTheDocument());
  });

  it("shows 409-specific toast when trigger returns update_already_active", async () => {
    const item = makeSoftwareItem("software-1", "Demo App");
    item.host_count = 1;
    const host = makeHostSummary("host-1", "host-one");
    const detail = makeDetail(item, [host]);

    vi.mocked(api.getSoftwareItems).mockResolvedValue(makeItemsPage([item]));
    vi.mocked(api.getSoftwareItem).mockResolvedValue(detail);
    vi.mocked(api.triggerSoftwareUpdate).mockRejectedValue(new ApiError("Update already active", 409, "trigger_update.update_already_active"));

    render(SoftwarePage);
    await waitFor(() => expect(api.getSoftwareItems).toHaveBeenCalled());

    const header = await screen.findByText("Demo App");
    await fireEvent.click(header);

    const updateBtn = await screen.findByText("Update");
    await fireEvent.click(updateBtn);

    // Confirm single-host modal
    const confirmBtn = await screen.findByRole("button", {
      name: /trigger update/i,
    });
    await fireEvent.click(confirmBtn);

    await waitFor(() => expect(vi.mocked(notifications.showError)).toHaveBeenCalledWith("An update is already active for this host"));
  });
});
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cd frontend && npm run test -- --reporter=verbose software-trigger-status
```

Expected: FAIL — badge rendering tests expect elements that don't exist yet; 409 test expects specific toast that isn't there.

- [ ] **Step 3: Pass `onOpenLiveModal` to `SoftwareGroupList`**

In `+page.svelte`, find the `<SoftwareGroupList` usage (around line 1103) and add the new prop:

```svelte
<SoftwareGroupList
    {items}
    {itemDetailsById}
    {itemDetailLoadingIds}
    {collapsedGroupIds}
    {expandedOverflowGroupIds}
    {batchSelectedIds}
    {canManage}
    {canTriggerUpdates}
    {pluginTypeNames}
    {totalItems}
    {currentPage}
    {totalPages}
    {showUpdatableOnly}
    onToggleGroup={toggleGroupCollapsed}
    onToggleOverflow={toggleGroupOverflow}
    onToggleBatch={toggleBatchSelect}
    onOpenMenu={toggleMenu}
    onOpenUpdateAllModal={openUpdateAllModal}
    onOpenSingleHostUpdate={openSingleHostUpdate}
    onOpenLiveModal={openLiveModal}
    onPageChange={loadAll}
    onToggleFeatured={toggleFeatured}
/>
```

- [ ] **Step 4: Update multi-host modal to disable already-active hosts**

In `openUpdateAllModal`, update the `selectedHostIds` initialization to exclude hosts that already have an active update (around line 606):

```typescript
// Before:
selectedHostIds = new Set(detail.hosts.filter((h) => h.update_available).map((h) => h.host_id));

// After:
selectedHostIds = new Set(detail.hosts.filter((h) => h.update_available && !h.active_update_history_id).map((h) => h.host_id));
```

In the modal template (around line 1306), update the `<li>` and `<Checkbox>` to disable active hosts:

```svelte
{#each updateModalDetail.hosts as host (host.host_id)}
    {@const upToDate = !host.update_available}
    {@const alreadyActive = !!host.active_update_history_id}
    {@const isDisabled = upToDate || alreadyActive}
    <li class="flex items-start gap-3 {isDisabled ? 'opacity-50' : ''}">
        <Checkbox
            id="software-host-select-{host.host_id}"
            class="mt-0.5"
            disabled={isDisabled}
            checked={selectedHostIds.has(host.host_id)}
            onchange={(e) => {
                const next = new Set(selectedHostIds);
                if ((e.target as HTMLInputElement).checked) {
                    next.add(host.host_id);
                } else {
                    next.delete(host.host_id);
                }
                selectedHostIds = next;
            }}
        />
        <div class="flex-1 min-w-0">
            <p class="text-sm font-medium truncate">
                {host.friendly_name || host.hostname}
            </p>
            {#if alreadyActive}
                <p class="text-table-header text-[var(--text-muted)]">Update already active</p>
            {:else if upToDate}
                <p class="text-table-header text-[var(--text-muted)]">Already up to date</p>
            {:else}
                <p class="text-table-header text-[var(--text-muted)]">
                    {host.installed_version ?? 'unknown'} -> {host.latest_version}
                </p>
            {/if}
        </div>
    </li>
{/each}
```

- [ ] **Step 5: Add SSE in-place cache handlers and fix 409 toast**

Replace the `onMount` subscriptions block with the full updated set. After the existing `VersionCheckCompleted` subscription, replace
`UpdateCompleted` and the removed auto-open handlers with:

```typescript
unsubscribers.push(
  subscribeToEvent(AdminEventType.SoftwareItemUpdated, () => loadAll(currentPage, true)),
  subscribeToEvent(AdminEventType.SoftwareItemCreated, () => loadAll(currentPage, true)),
  subscribeToEvent(AdminEventType.VersionCheckCompleted, () => loadAll(currentPage, true)),
  subscribeToEvent(AdminEventType.UpdateCompleted, (data) => {
    const softwareItemId = data.software_item_id as string;
    const hostId = data.host_id as string;
    const detail = itemDetailsById.get(softwareItemId);
    if (detail) {
      itemDetailsById.set(softwareItemId, {
        ...detail,
        hosts: detail.hosts.map((h) =>
          h.host_id === hostId
            ? {
                ...h,
                active_update_history_id: null,
                active_update_status: null,
              }
            : h,
        ),
      });
    }
    void loadAll(currentPage, true);
  }),
  subscribeToEvent(AdminEventType.UpdateTriggered, (data) => {
    const softwareItemId = data.software_item_id as string;
    const hostId = data.host_id as string;
    const detail = itemDetailsById.get(softwareItemId);
    if (!detail) return;
    itemDetailsById.set(softwareItemId, {
      ...detail,
      hosts: detail.hosts.map((h) =>
        h.host_id === hostId
          ? {
              ...h,
              active_update_history_id: data.update_history_id as string,
              active_update_status: ((data.status as string) ?? "pending") || "pending",
            }
          : h,
      ),
    });
  }),
  subscribeToEvent(AdminEventType.UpdateStarted, (data) => {
    const softwareItemId = data.software_item_id as string;
    const hostId = data.host_id as string;
    const detail = itemDetailsById.get(softwareItemId);
    if (!detail) return;
    itemDetailsById.set(softwareItemId, {
      ...detail,
      hosts: detail.hosts.map((h) => (h.host_id === hostId ? { ...h, active_update_status: "in_progress" } : h)),
    });
  }),
);
```

Update `executeSingleHostUpdate` to handle 409. Add `import { ApiError } from '$lib/api';` at the top of the script (if not already imported). Then
update the catch block:

```typescript
} catch (e) {
    if (e instanceof ApiError && e.errorCode === 'trigger_update.update_already_active') {
        showError('An update is already active for this host');
    } else {
        showError(e instanceof Error ? e.message : 'Failed to trigger update');
    }
}
```

Update `executeUpdate` (batch multi-host) to handle 409s distinctly. Update the results loop:

```typescript
let succeeded = 0;
let failed = 0;
let alreadyActive = 0;
for (const result of results) {
  if (result.status === "rejected") {
    if (result.reason instanceof ApiError && result.reason.errorCode === "trigger_update.update_already_active") {
      alreadyActive += 1;
    } else {
      failed += 1;
    }
    continue;
  }
  if (result.value.status === "failed") {
    failed += 1;
    continue;
  }
  succeeded += 1;
}
if (succeeded > 0) showSuccess(`Update triggered for ${succeeded} host(s).`);
if (alreadyActive > 0) showError(`${alreadyActive} host(s) already have an active update.`);
if (failed > 0) showError(`Failed to trigger update for ${failed} host(s).`);
```

- [ ] **Step 6: Add `AdminEventType` import for new events**

Make sure `AdminEventType.UpdateTriggered` and `AdminEventType.UpdateStarted` are already in scope (they are, via the existing SSE import). Verify the
`AdminEventType` enum has these variants:

```bash
grep -n "UpdateTriggered\|UpdateStarted\|UpdateCompleted" frontend/src/lib/sse.ts
```

Expected: all three present.

- [ ] **Step 7: Run tests**

```bash
cd frontend && npm run test -- --reporter=verbose software-trigger-status
```

Expected: PASS (including new badge and 409 tests).

- [ ] **Step 8: Type-check**

```bash
cd frontend && npm run check
```

Expected: no errors.

- [ ] **Step 9: Commit**

```bash
git add "frontend/src/routes/software/+page.svelte" frontend/src/routes/software/software-trigger-status.test.ts
git commit -m "feat(frontend): SSE cache updates, 409 handling, modal active-host disable in software list page"
```

---

### Task 6: Wire /software/[id]/+page.svelte — SSE cache, badge rendering, 409 handling

**Files:**

- Modify: `frontend/src/routes/software/[id]/+page.svelte`
- Modify: `frontend/src/routes/software/[id]/software-detail-update-trigger.test.ts`

- [ ] **Step 1: Write the failing tests**

Open `frontend/src/routes/software/[id]/software-detail-update-trigger.test.ts` and add tests for the new behaviors. Add after existing tests:

```typescript
describe("active status badge rendering on detail page", () => {
  it('shows "Queued" StatusBadge (not "Update") when host active_update_status is queued', async () => {
    // Use the existing test setup pattern from this file.
    // Provide a host with active_update_history_id and active_update_status: 'queued'.
    const host = {
      ...makeHostSummary("host-1", "host-one"),
      active_update_history_id: "hist-abc",
      active_update_status: "queued",
    };
    // ... render, assert 'Queued' visible, 'Update' not present
  });

  it('shows "In Progress" ActionBadge when active_update_status is in_progress', async () => {
    const host = {
      ...makeHostSummary("host-1", "host-one"),
      active_update_history_id: "hist-abc",
      active_update_status: "in_progress",
    };
    // ... render, assert 'In Progress' visible, clickable
  });

  it("shows 409-specific toast in single-host executeUpdate", async () => {
    // Mock triggerSoftwareUpdate to throw ApiError with update_already_active
    // Trigger update, verify showError called with specific message
  });
});
```

Follow the exact fixture and mock patterns from the existing tests in that file. Look at how `makeHostSummary`, `makeDetail`, and
`render(SoftwareDetailPage)` are used in the existing tests.

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cd frontend && npm run test -- --reporter=verbose software-detail-update-trigger
```

Expected: FAIL.

- [ ] **Step 3: Replace `UpdateTriggered` handler with in-place update**

In `/software/[id]/+page.svelte`, find the existing `UpdateTriggered` subscription (which currently calls `loadItem(true)`). Replace it with:

```typescript
subscribeToEvent(AdminEventType.UpdateTriggered, (data) => {
    if (data.software_item_id !== id) return;
    if (!item) return;
    item = {
        ...item,
        hosts: item.hosts.map((h) =>
            h.host_id === (data.host_id as string)
                ? {
                      ...h,
                      active_update_history_id: data.update_history_id as string,
                      active_update_status: ((data.status as string) ?? 'pending') || 'pending'
                  }
                : h
        )
    };
}),
```

- [ ] **Step 4: Add `UpdateStarted` in-place handler**

After the `UpdateTriggered` subscription, add:

```typescript
subscribeToEvent(AdminEventType.UpdateStarted, (data) => {
    if (data.software_item_id !== id) return;
    if (!item) return;
    item = {
        ...item,
        hosts: item.hosts.map((h) =>
            h.host_id === (data.host_id as string)
                ? { ...h, active_update_status: 'in_progress' }
                : h
        )
    };
}),
```

The existing `UpdateCompleted` handler (`loadItem(true)`) stays as is — it does a full refresh which clears the active state from the server response.

- [ ] **Step 5: Update badge rendering block for four-way split**

Find the badge rendering block at line ~1033:

```svelte
<!-- Before: -->
{#if canView && host.active_update_history_id}
    <span class="inline-flex" title="View update progress">
        <ActionBadge
            variant="navigation"
            tone="info"
            idleLabel="In Progress"
            hoverLabel="→ Log"
            onclick={() => openLiveModal(host.active_update_history_id!, host.hostname)}
        />
    </span>
{:else if canTriggerUpdates && host.update_available}
    ...
{:else}
    <StatusBadge tone={versionStatusTone(host)} label={versionStatusLabel(host)} />
{/if}

<!-- After: -->
{#if host.active_update_history_id}
    {#if host.active_update_status === 'in_progress' && canView}
        <span class="inline-flex" title="View update progress">
            <ActionBadge
                variant="navigation"
                tone="info"
                idleLabel="In Progress"
                hoverLabel="→ Log"
                onclick={() => openLiveModal(host.active_update_history_id!, host.hostname)}
            />
        </span>
    {:else if host.active_update_status === 'queued'}
        <StatusBadge tone="info" label="Queued" />
    {:else if host.active_update_status === 'pending'}
        <StatusBadge tone="info" label="Pending" />
    {:else if host.active_update_status === 'awaiting_restart'}
        <StatusBadge tone="info" label="Awaiting Restart" />
    {:else}
        <span class="inline-flex" title="View update progress">
            <ActionBadge
                variant="navigation"
                tone="info"
                idleLabel="In Progress"
                hoverLabel="→ Log"
                onclick={() => openLiveModal(host.active_update_history_id!, host.hostname)}
            />
        </span>
    {/if}
{:else if canTriggerUpdates && host.update_available}
    <span
        class="inline-flex"
        title={`Update to ${formatVersion(resolveDisplayVersion(host.latest_version ?? item?.latest_version, getReleaseMeta(host)?.display_version))}`}
    >
        <ActionBadge
            variant="navigation"
            tone="accent"
            idleLabel="Update"
            hoverLabel="Update"
            onclick={() => openUpdateModal(host)}
        />
    </span>
{:else}
    <StatusBadge tone={versionStatusTone(host)} label={versionStatusLabel(host)} />
{/if}
```

The `{:else}` fallback inside the active block defaults to "In Progress" clickable (for backward compatibility with cache entries that have the ID but
not the status field).

- [ ] **Step 6: Update `versionStatusLabel` and `versionStatusTone` for four-way split**

Replace `versionStatusLabel` (line 732):

```typescript
function versionStatusLabel(host: SoftwareItemHostSummary): string {
  if (host.active_update_history_id) {
    const s = host.active_update_status;
    if (s === "queued") return "Queued";
    if (s === "pending") return "Pending";
    if (s === "awaiting_restart") return "Awaiting Restart";
    return "In Progress";
  }
  if (!host.installed_version) return "Unknown";
  const latest = effectiveLatestVersion(host);
  if (!latest) return "Unknown latest";
  if (host.update_available) return "Update Available";
  if (host.latest_version == null && host.installed_version !== latest) {
    return "Update may be available";
  }
  return "Up-to-date";
}
```

Replace `versionStatusTone` (line 744):

```typescript
function versionStatusTone(host: SoftwareItemHostSummary): "info" | "neutral" | "warning" | "success" {
  if (host.active_update_history_id) return "info";
  if (!host.installed_version) return "neutral";
  const latest = effectiveLatestVersion(host);
  if (!latest) return "neutral";
  if (host.update_available) return "warning";
  if (host.latest_version == null && host.installed_version !== latest) return "warning";
  return "success";
}
```

(`versionStatusTone` remains unchanged — `info` covers all four active states.)

- [ ] **Step 7: Update `updateAllHostItems` derived to disable active hosts**

Replace the `updateAllHostItems` derived block (line 103):

```typescript
const updateAllHostItems = $derived.by<CheckboxListItem[]>(() => {
  const hosts: SoftwareItemHostSummary[] = updateAllDetail?.hosts ?? [];
  return hosts.map((h) => {
    const upToDate = !h.update_available;
    const alreadyActive = !!h.active_update_history_id;
    const isDisabled = upToDate || alreadyActive;
    return {
      value: h.host_id,
      label: h.friendly_name || h.hostname,
      sublabel: alreadyActive
        ? "Update already active"
        : upToDate
          ? "Already up to date"
          : `${h.installed_version ?? "unknown"} → ${h.latest_version}`,
      disabled: isDisabled,
    };
  });
});
```

In `openUpdateAllModal`, exclude already-active hosts from initial selection (line 663):

```typescript
// Before:
for (const h of detail.hosts.filter((h) => h.update_available)) {
  updateAllSelectedHostIds.add(h.host_id);
}

// After:
for (const h of detail.hosts.filter((h) => h.update_available && !h.active_update_history_id)) {
  updateAllSelectedHostIds.add(h.host_id);
}
```

- [ ] **Step 8: Add 409 handling to `executeUpdate` and `executeUpdateAll`**

Add import at top of script block (if not already there):

```typescript
import { ApiError } from "$lib/api";
```

Update `executeUpdate` catch block:

```typescript
} catch (e) {
    if (e instanceof ApiError && e.errorCode === 'trigger_update.update_already_active') {
        showError('An update is already active for this host');
    } else {
        showError(e instanceof Error ? e.message : 'Failed to trigger update');
    }
}
```

Update `executeUpdateAll` results loop (add `alreadyActive` counting same as Task 5, Step 5):

```typescript
let succeeded = 0;
let failed = 0;
let alreadyActive = 0;
for (const result of results) {
  if (result.status === "rejected") {
    if (result.reason instanceof ApiError && result.reason.errorCode === "trigger_update.update_already_active") {
      alreadyActive += 1;
    } else {
      failed += 1;
    }
    continue;
  }
  if (result.value.status === "failed") {
    failed += 1;
    continue;
  }
  succeeded += 1;
}
if (succeeded > 0) showSuccess(`Update triggered for ${succeeded} host(s).`);
if (alreadyActive > 0) showError(`${alreadyActive} host(s) already have an active update.`);
if (failed > 0) showError(`Failed to trigger update for ${failed} host(s).`);
```

- [ ] **Step 9: Run tests**

```bash
cd frontend && npm run test -- --reporter=verbose software-detail-update-trigger
```

Expected: PASS.

- [ ] **Step 10: Type-check and full test suite**

```bash
cd frontend && npm run check && npm run test
```

Expected: no errors.

- [ ] **Step 11: Commit**

```bash
git add "frontend/src/routes/software/[id]/+page.svelte" "frontend/src/routes/software/[id]/software-detail-update-trigger.test.ts"
git commit -m "feat(frontend): SSE cache, four-way badge split, and 409 handling on software detail page"
```

---

### Task 7: Quality gate + markdownlint

**Files:** none changed

- [ ] **Step 1: Run all frontend quality gates**

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

Expected: all pass. Fix any lint/format errors before proceeding.

- [ ] **Step 2: Run markdownlint on any changed docs**

```bash
npx markdownlint --config .markdownlint.json docs/superpowers/plans/2026-05-15-update-trigger-ux-b-frontend.md
```

Expected: no errors.

- [ ] **Step 3: Final commit if any fixes were needed**

If lint/format fixes were made:

```bash
git add -p  # stage only the lint/format changes
git commit -m "chore(frontend): fix lint and format warnings from update-trigger-ux plan B"
```

---

## Self-Review

### Spec Coverage

| Spec requirement                                                       | Task                                             |
| ---------------------------------------------------------------------- | ------------------------------------------------ |
| Block duplicate triggers (backend)                                     | Plan A                                           |
| Enrich `SoftwareItemHostSummary.active_update_status`                  | Task 1 (types), Task 5+6 (SSE usage)             |
| `UpdateHistoryStatus` includes `awaiting_restart`                      | Task 1                                           |
| SSE debounce key collision fix                                         | Task 1                                           |
| `ApiError` with `errorCode` for 409 detection                          | Task 2                                           |
| Remove auto-open terminal from both pages                              | Task 3                                           |
| Four-way badge rendering (Queued/Pending/In Progress/Awaiting Restart) | Task 4+6                                         |
| `allUpdatableHostsActive` + UpdateAllButton disable                    | Task 4                                           |
| Pass `onOpenLiveModal` through SoftwareGroupList                       | Task 4+5                                         |
| Modal checkbox disabled + "Update already active" label                | Task 5+6                                         |
| SSE in-place cache update on `UpdateTriggered` (list page)             | Task 5                                           |
| SSE in-place cache update on `UpdateStarted` (list page)               | Task 5                                           |
| SSE in-place cache clear on `UpdateCompleted` (list page)              | Task 5                                           |
| 409 toast: "An update is already active for this host"                 | Task 5+6                                         |
| No re-fetch / no auto-dismiss on 409                                   | Task 5+6 (`catch` block does not call `loadAll`) |
| Detail page: SSE in-place cache update on `UpdateTriggered`            | Task 6                                           |
| Detail page: SSE in-place cache update on `UpdateStarted`              | Task 6                                           |
| Detail page: `updateAllHostItems` disables active hosts                | Task 6                                           |
| Detail page: four-way badge block + `versionStatusLabel`               | Task 6                                           |

### Type Consistency

- `host.active_update_status` typed as `string | null | undefined` (via `?: string | null`) throughout — compatible with SSE data casts and `null`
  assignments on clear.
- `ApiError` exported from `api.ts`, imported in both page files.
- `onOpenLiveModal: (updateHistoryId: string, hostName: string, itemName: string) => void` — matches `openLiveModal` signature in `+page.svelte`
  (three-arg version).
- `onOpenLiveModal` on detail page: `openLiveModal(histId, hostName)` only takes two args — NOT passed to `SoftwareGroupList` from detail page (detail
  page doesn't use `SoftwareGroupList`; it has its own badge block updated in Task 6 Step 5).

### No Placeholders

All code blocks are complete. No "implement later" or "TBD" in task steps.
