# Update Trigger UX — Live Status Badges & Duplicate Prevention

**Date:** 2026-05-15
**Status:** Draft

## Problem

After triggering a software update from `/software` or `/software/[id]`, the UI has several
gaps:

1. A second trigger for the same (host, software item) silently queues behind the first — no
   feedback that a duplicate is blocked.
2. Per-host badges always show "Update" regardless of whether an update is already
   queued/pending/in progress/awaiting restart.
3. The interactive terminal auto-opens on `UpdateStarted` SSE — disorienting when the user has
   already navigated away.
4. The group-level "Update All" button is never disabled, even when every host is already
   updating.
5. Badge state does not update in real time; only `UpdateCompleted` triggers a reload.
6. `AwaitingRestart` status is missing from the frontend type and from the `active_updates`
   query.

---

## Out of Scope

- Dedicated `/terminal/:update_history_id` route — modal stays the only entrypoint.
- Batch update interactive mode — batch is always non-interactive; no PTY.
- Push notification for update completion.
- History page badge updates.

---

## Backend Changes

### 1. New partial unique index — `(host_id, software_item_id)`

Add a migration that creates a second partial unique index on `update_history`:

```sql
CREATE UNIQUE INDEX uix_update_history_host_software_item_active
ON update_history (host_id, software_item_id)
WHERE status IN ('queued', 'pending', 'in_progress', 'awaiting_restart');
```

**Why a new index, not expanding or replacing the existing one:**
The existing `uix_update_history_host_active` covers `(host_id)` for `pending` and
`in_progress` — it serialises all updates on a single host, preventing two software items
from updating simultaneously on the same host. This constraint remains in place.

The new `(host_id, software_item_id)` index is an **addition**, not a replacement. It
enforces a narrower invariant: no two rows for the _same_ (host, software item) pair may
exist in any non-terminal status. This allows batch updates to create multiple `Queued`
rows for different software items on the same host (which the host-level index allows once
the previous item reaches `Completed`), while still preventing the duplicate-trigger case
where a user clicks "Update" twice for the same item on the same host.

**TOCTOU safety:** the two application-level pre-checks (item-level and host-level) both
run without an enclosing transaction, so a concurrent request can race past one before the
other completes. The partial unique index on `(host_id, software_item_id) WHERE status IN
('queued','pending','in_progress','awaiting_restart')` is the DB-enforced backstop: any
concurrent insert that passes the pre-checks but violates the index will hit a unique
constraint error, which is caught by the `is_unique_constraint_violation` fallback and
returned as `UpdateAlreadyActive`. This guarantee depends on the index predicate covering
all four active statuses — confirm the migration includes all four in the `WHERE` clause.

**Migration location:** `crates/shared/db/src/migration/` — follow the pattern in
`m20260313_000001_per_host_update_locking.rs`.

### 2. Pre-check in `trigger_update_for_host`

Add a new query function in `update_dispatch.rs`:

```rust
/// Returns true if a Queued, Pending, InProgress, or AwaitingRestart row already exists
/// for the given (host_id, software_item_id) pair.
pub async fn has_active_update_for_host_software_item(
    db: &DatabaseConnection,
    host_id: Uuid,
    software_item_id: Uuid,
) -> Result<bool>
```

Call this at the start of `trigger_update_for_host`
(`update_triggers.rs:149`), **before** the existing `has_active_update_for_host` call.
Return `TriggerUpdateError::UpdateAlreadyActive` immediately if it returns `true`.

The two pre-checks serve different invariants and both remain active:

- `has_active_update_for_host` (existing) — host-level serialisation: prevents any two
  software items from updating concurrently on the same host.
- `has_active_update_for_host_software_item` (new) — item-level deduplication: prevents
  a duplicate trigger for the same (host, software item) pair in any non-terminal status.

Running the item-level check first short-circuits duplicate triggers with a clean 409
before the host-level check even runs. The DB unique index is belt-and-suspenders for
concurrent requests that race past both pre-checks.

**Fallback update required:** `trigger_update_for_host` currently has a
`is_unique_constraint_violation` fallback (after the Pending INSERT) that re-inserts as
`Queued` when a concurrent controller wins the race. With the new `(host_id,
software_item_id)` constraint covering `Queued`, that re-insert also violates the constraint.
The fallback must be updated: when the re-insert-as-Queued also hits a unique violation,
return `TriggerUpdateError::UpdateAlreadyActive` rather than propagating the DB error.

**Note:** `UpdateAlreadyActive` already maps to HTTP 409
(`api_error/mappings.rs:976`) — no mapping change needed.

### 3. Add `UpdateStatus` grouping helpers

Add to the existing `impl UpdateStatus` block in
`crates/shared/types/src/update_status.rs`:

```rust
/// All non-terminal statuses — states where a new trigger for the same
/// (host, software_item) must be rejected.
pub const fn unfinished() -> [Self; 4] {
    [Self::Queued, Self::Pending, Self::InProgress, Self::AwaitingRestart]
}

/// Statuses that block the host from running another update concurrently.
/// Excludes `Queued` — a queued update does not occupy the host's execution
/// slot; it is waiting for the preceding update to finish.
pub const fn host_blocking() -> [Self; 3] {
    [Self::Pending, Self::InProgress, Self::AwaitingRestart]
}
```

**Callsite assignments:**

- `UpdateStatus::unfinished()` — `active_updates` query in `software_items/mod.rs` and
  the new `has_active_update_for_host_software_item` in `update_dispatch.rs`.
- `UpdateStatus::host_blocking()` — `has_active_update_for_host` in `update_dispatch.rs`
  (currently inlines `[Pending, InProgress, AwaitingRestart]`).
- `UpdateStatus::unfinished()` — also fixes two callsites in `software_states.rs`
  (lines 139–141 and 407–409) which currently use `Queued | Pending | InProgress` — missing
  `AwaitingRestart`. These sites must use `unfinished()`, not `host_blocking()`: `Queued`
  must be included because a queued row still represents an active pending operation for
  purposes of state reporting (dropping `Queued` would silently hide queued updates).

No other groupings (`terminal()`, `not_started()`, etc.) have repeated multi-value callsites
— deferred until actual repetition exists.

### 4. Add `active_update_status` to `SoftwareItemHostSummary`

In `crates/shared/web-api-types/src/software_items.rs`, add alongside
`active_update_history_id`:

```rust
/// Status of the active update, if any. One of: "queued", "pending",
/// "in_progress", "awaiting_restart". None when no active update exists.
#[serde(default, skip_serializing_if = "Option::is_none")]
pub active_update_status: Option<String>,
```

Populate it in `software_items/mod.rs` from the same active update row used to set
`active_update_history_id`. The `active_updates` map currently stores only the row ID;
extend it to store `(Uuid, String)` — `(update_history_id, status_str)`.

Adding a field to a `#[non_exhaustive]` struct requires updating every struct literal in the
codebase. Run `grep -r "SoftwareItemHostSummary {" --include="*.rs"` to find all sites and
add `active_update_status: None` to each. Known sites: `crud.rs:1092`,
`cli/src/commands/software_items.rs:855,919`; there may be additional fixtures in test
helpers.

Using `Option<String>` (not `Option<UpdateStatus>`) follows the existing wire-safe pattern
(`UpdateCompleted.status: String`). `#[non_exhaustive]` enum would require a client-side
wildcard arm everywhere; a string is safer at the API boundary.

### 5. Add `status` field to `UpdateTriggered` SSE event

In `crates/shared/wire/src/admin_events.rs`:

```rust
UpdateTriggered {
    update_history_id: Uuid,
    host_id: Uuid,
    software_item_id: Uuid,
    /// "pending" or "queued"
    status: String,
},
```

`UpdateTriggered` is a `#[non_exhaustive]` enum variant — adding a new field to the struct
variant is a breaking change on the wire. All deserialisation sites must be updated:

- `admin_events.rs`: `UpdateTriggered` uses a handwritten `Deserialize` impl with an
  `Inner` struct that mirrors the variant's fields. Add `status` to `Inner` as
  `#[serde(default = "default_pending_status")] status: String`, accompanied by
  `fn default_pending_status() -> String { "pending".into() }`. Do NOT use bare
  `#[serde(default)]` — that produces an empty string for absent fields, which the
  badge decision tree cannot map to a label. Defaulting to `"pending"` is safe: single-host
  triggers start as `Pending`, and `queued` items would also show a non-clickable badge.
  Forward `Inner.status` to the enum variant. Also update the `all_variants()` test helper
  (which constructs `UpdateTriggered` without a `status` field and will fail to compile).
- `crates/shared/openapi-client/src/events_stream.rs`: the `Payload` struct for
  `"update_triggered"` (lines 196–208) currently deserialises only `{update_history_id,
host_id, software_item_id}` — add `status: String` and forward it to
  `AdminSseEvent::UpdateTriggered`.
- `crates/shared/wire/asyncapi.yaml`: add `status` to the `UpdateTriggered` message
  schema so wire-protocol tests remain in sync.

Emit this event immediately after `trigger_update_for_host` succeeds, carrying the
`initial_status` from `TriggerUpdateResult`.

**Batch emit sites** (`crates/ui/web-api/src/actions/update_batches.rs` lines 88 and 331):
both iterate `resp.updates: Vec<BatchUpdateItem>`, which already carries
`trigger_status: TriggerUpdateStatus`. Pass `item.trigger_status.to_string()` as `status`
when constructing `AdminEvent::UpdateTriggered` at each site. (`TriggerUpdateStatus` has no
`as_str()` method — use `.to_string()` directly via its `Display` impl.)

**Single-host emit site** (`crates/ui/web-api/src/routes/service_ws/handler/update_tracking.rs`
line 165): a fourth `UpdateTriggered` emit site exists in the service-WS update tracking
handler. Update this site identically — add `status` from the local update status value.

---

## Frontend Changes

### 6. Add `awaiting_restart` to `UpdateHistoryStatus`

`frontend/src/lib/types.ts`:

```typescript
export type UpdateHistoryStatus =
  | "queued"
  | "pending"
  | "in_progress"
  | "awaiting_restart" // add
  | "completed"
  | "failed";
```

### 7. Badge rendering rules

Applies to `SoftwareGroupList.svelte` (per-host rows and compact single-host row) and
`/software/[id]/+page.svelte` host table.

**Decision tree for each host row:**

```text
if active_update_history_id is set:
  if active_update_status === 'in_progress':
    → ActionBadge variant="navigation" tone="info" label="In Progress"
      onclick → openLiveModal(host.active_update_history_id, host.hostname, item.name)
  else (queued | pending | awaiting_restart):
    → StatusBadge tone="info" label=statusLabel(active_update_status)
      non-clickable
else:
  → existing Update / Up-to-date / Update avail logic unchanged
```

Status labels:

| `active_update_status` | Badge label      |
| ---------------------- | ---------------- |
| `queued`               | Queued           |
| `pending`              | Pending          |
| `in_progress`          | In Progress      |
| `awaiting_restart`     | Awaiting Restart |

The `ActionBadge` for "In Progress" uses `variant="navigation"` and `tone="info"` (not
`tone="accent"`) to visually distinguish it from the "Update" trigger badge.

### 8. Group-level "Update All" button disable logic

In `SoftwareGroupList.svelte`, a new derived value:

```typescript
function allUpdatableHostsActive(item: SoftwareItemResponse): boolean {
  const hosts = filteredHosts(item); // hosts with update_available && latest_version
  if (hosts.length === 0) return false;
  return hosts.every((h) => !!h.active_update_history_id);
}
```

Apply to the `UpdateAllButton`:

```svelte
<UpdateAllButton
    state={hasAnyUpdateableHosts(item) && !allUpdatableHostsActive(item) ? 'idle' : 'dim'}
    ...
/>
```

When `itemDetailsById` has no entry for this item, `filteredHosts` returns `[]` →
`allUpdatableHostsActive` returns `false` → button stays enabled (correct; detail not yet
loaded).

**Inside the multi-host selection modal:** hosts with `active_update_history_id` set render
their checkbox as `disabled` with `aria-label="Update already active"`.

### 9. SSE-driven in-place cache updates

Both `/software/+page.svelte` and `/software/[id]/+page.svelte` subscribe to these events
and mutate their detail caches in place so Svelte's reactivity propagates to badges without
a full reload.

**SSE debounce note:** `dispatchEvent` in `events.svelte.ts` keys debounce on
`eventType + (data.id ?? data.host_id ?? data.task_id)`. Two `UpdateTriggered` events for
different software items on the same host within 200 ms produce identical keys — the first
is dropped. Fix: extend the entity ID extraction to also include `data.software_item_id`
for `UpdateTriggered`, e.g.:

```typescript
const entityId = (data.id ?? data.host_id ?? data.task_id ?? "") as string;
const subId = (data.update_history_id ?? data.software_item_id ?? "") as string;
const key = `${eventType}:${entityId}:${subId}`;
```

This change is in `frontend/src/lib/stores/events.svelte.ts` and scoped to the debounce
key construction only.

**Helper (extract to a shared function or inline in each page):**

Svelte 5 reactivity does not reliably track mutations to nested object properties inside a
`SvelteMap`. Use a spread/replace pattern: replace the host object and re-set the detail
entry so the map emits a reactive signal.

```typescript
function applyUpdateTriggered(
  cache: SvelteMap<string, SoftwareItemDetailResponse>,
  data: {
    software_item_id: string;
    host_id: string;
    update_history_id: string;
    status: string;
  },
): void {
  const detail = cache.get(data.software_item_id);
  if (!detail) return;
  const updatedHosts = detail.hosts.map((h) =>
    h.host_id === data.host_id
      ? {
          ...h,
          active_update_history_id: data.update_history_id,
          active_update_status: data.status,
        }
      : h,
  );
  cache.set(data.software_item_id, { ...detail, hosts: updatedHosts });
}
```

Apply the same spread/replace approach for `UpdateStarted` and `UpdateCompleted` handlers.
For the detail page, which holds a single `$state` variable, spread-assign the updated host
into a new `hosts` array and replace the `$state` variable rather than mutating in place.

| SSE event                 | Action                                                                                                                                                                                                                                                                                                                                                                   |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `UpdateTriggered`         | Set `active_update_history_id` + `active_update_status` on matching cached host                                                                                                                                                                                                                                                                                          |
| `UpdateStarted`           | Set `active_update_status = 'in_progress'` on matching cached host                                                                                                                                                                                                                                                                                                       |
| `UpdateCompleted`         | Clear `active_update_history_id` and `active_update_status` on matching cached host; **if `data.status === 'completed'`, also set `update_available = false` optimistically**; existing `loadAll()` reload still fires and corrects all values from server. Do NOT set `update_available = false` when `data.status === 'failed'` — the retry badge must remain visible. |
| `UpdateProtectionStarted` | No-op (backend still emits; frontend ignores for UI purposes)                                                                                                                                                                                                                                                                                                            |

**Why `update_available = false` on `UpdateCompleted` (completed only):** Without this, there
is a race window between the cache clearing `active_update_history_id` and `loadAll()`
resolving. During that window `update_available` is still `true` in the stale cache, so
`filteredHosts` includes the host and `allUpdatableHostsActive` may return false — briefly
re-enabling "Update All" and allowing an unintended re-trigger. Setting `update_available =
false` optimistically (only on success) closes this window; `loadAll()` restores the
authoritative value. On `failed`, `update_available` stays `true` so the retry badge is
immediately available.

**Re-queue fallback note:** `is_unique_constraint_violation` in `update_triggers.rs` is not
index-specific — it matches any unique constraint violation at that callsite. The new
`(host_id, software_item_id)` constraint is the only unique constraint that can fire on a
Queued insert (no other column combination is unique-constrained for non-terminal rows), so
the false-positive risk is negligible. If a PK collision occurred it would be a programming
error, not a duplicate trigger, and a 409 is preferable to a 500 in that scenario anyway.

The detail page (`/software/[id]`) holds a single `SoftwareItemDetailResponse` in a
`$state` variable rather than a `SvelteMap` — apply the same spread/replace mutations to
that variable.

**`versionStatusLabel` / `versionStatusTone` on detail page:** `/software/[id]/+page.svelte`
currently derives a single `versionStatusLabel` and `versionStatusTone` (around lines 733
and 745) which collapse all active states into `"In Progress"`. These functions must be
updated to the same four-way split as the badge decision tree in §7: `queued` → `"Queued"`,
`pending` → `"Pending"`, `in_progress` → `"In Progress"` (clickable), `awaiting_restart` →
`"Awaiting Restart"` (non-clickable).

### 10. Remove auto-open terminal

From `/software/+page.svelte` **and** `/software/[id]/+page.svelte`:

- Remove `pendingLiveHistoryId`, `pendingLiveHostName`, `pendingLiveItemName` reactive state.
- Remove the `UpdateProtectionStarted` SSE handler that called `openLiveModal`.
- Remove the `UpdateStarted` SSE handler that called `openLiveModal`.
- The `UpdateStarted` event **is still subscribed** for the in-place cache update
  (`active_update_status → 'in_progress'`) — only the auto-open side-effect is removed.

Terminal opens exclusively via user clicking the "In Progress" `ActionBadge`.

### 11. 409 error handling

When the trigger API call returns a 409 with `error_code === 'trigger_update.update_already_active'`:

- Show a toast notification: `"An update is already active for this host."`
- Do not close the confirmation modal.
- Do not re-fetch or reload.

Add this as a specific case in the trigger error handler, before the generic error fallback.

---

## Data Flow Summary

```text
User clicks "Update" badge
  → confirmation modal opens
  → user confirms
  → POST /software-items/{id}/hosts/{host_id}/update

Backend:
  → has_active_update_for_host_software_item? → 409 (UpdateAlreadyActive)
  → trigger_update_for_host succeeds → emit UpdateTriggered{..., status}
  → TriggerUpdateResponse{update_history_id, status: Pending|Queued}

Frontend (on success):
  → close confirmation modal
  → show toast "Update triggered"
  (badge updates via SSE below)

SSE: UpdateTriggered{software_item_id, host_id, update_history_id, status}
  → cache update: host.active_update_history_id = id, host.active_update_status = status
  → badge: StatusBadge "Pending" or "Queued"

SSE: UpdateStarted{...}
  → cache update: host.active_update_status = 'in_progress'
  → badge: ActionBadge "In Progress" (clickable → terminal modal)

User clicks "In Progress" badge
  → openLiveModal(update_history_id, hostname, itemName)
  → connectInteractiveSession(update_history_id, ...)

SSE: UpdateCompleted{...}
  → cache update: clear active_update_history_id + active_update_status
  → badge: reverts to "Up to date" or "Update avail"
  → loadAll() fires → full list refresh
```

---

## Affected Files

### Backend

| File                                                                   | Change                                                                                                             |
| ---------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| `crates/shared/db/src/migration/`                                      | New migration: `(host_id, software_item_id)` partial unique index                                                  |
| `crates/shared/db/src/migration/lib.rs`                                | Register new migration                                                                                             |
| `crates/shared/types/src/update_status.rs`                             | Add `UpdateStatus::unfinished() -> [Self; 4]` and `host_blocking() -> [Self; 3]`                                   |
| `crates/ui/web-api-queries/src/queries/update_dispatch.rs`             | Add `has_active_update_for_host_software_item`; use `unfinished()` and `host_blocking()`                           |
| `crates/ui/web-api-queries/src/queries/update_triggers.rs`             | Call new pre-check; switch `has_active_update_for_host` to `host_blocking()`                                       |
| `crates/ui/web-api-queries/src/queries/software_items/mod.rs`          | Switch to `UpdateStatus::unfinished()`; carry `status` alongside `id` in map                                       |
| `crates/ui/web-api-queries/src/queries/software_states.rs`             | Fix `AwaitingRestart` omission at lines 139–141 and 407–409; switch to `unfinished()` (Queued must be included)    |
| `crates/shared/web-api-types/src/software_items.rs`                    | Add `active_update_status: Option<String>` to `SoftwareItemHostSummary`                                            |
| `crates/shared/wire/src/admin_events.rs`                               | Add `status: String` to `UpdateTriggered`; update serialisation, deserialisation, and `all_variants()` test helper |
| `crates/shared/wire/asyncapi.yaml`                                     | Add `status: string` to `update_triggered` message schema                                                          |
| `crates/shared/openapi-client/src/events_stream.rs`                    | Add `status: String` to `UpdateTriggered` payload struct; forward to `AdminSseEvent::UpdateTriggered`              |
| `crates/ui/web-api/src/actions/update_batches.rs`                      | Pass `item.trigger_status.to_string()` as `status` in both `UpdateTriggered` emit sites (lines 88, 331)            |
| `crates/ui/web-api/src/routes/software_items/mod.rs` (or action crate) | Pass `initial_status` from `TriggerUpdateResult` as `status` in single-host `UpdateTriggered` emit                 |
| `crates/ui/web-api/src/routes/service_ws/handler/update_tracking.rs`   | Add `status` to `UpdateTriggered` emit at line 165 (fourth emit site)                                              |
| `crates/ui/web-api-queries/src/queries/software_items/crud.rs`         | Add `active_update_status: None` to `SoftwareItemHostSummary` struct literal in test fixture (line 1092)           |
| `crates/ui/cli/src/commands/software_items.rs`                         | Add `active_update_status: None` to both `SoftwareItemHostSummary` struct literals (lines 855, 919)                |

### Frontend

| File                                                      | Change                                                                                                            |
| --------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| `frontend/src/lib/types.ts`                               | Add `awaiting_restart` to `UpdateHistoryStatus`; add `active_update_status?: string` to `SoftwareItemHostSummary` |
| `frontend/src/lib/stores/events.svelte.ts`                | Fix debounce key to include `software_item_id` for `UpdateTriggered` events                                       |
| `frontend/src/lib/components/ui/SoftwareGroupList.svelte` | New badge rendering logic; `allUpdatableHostsActive` guard; remove Update button when active                      |
| `frontend/src/routes/software/+page.svelte`               | SSE in-place cache handlers; remove auto-open state + handlers; 409 toast                                         |
| `frontend/src/routes/software/[id]/+page.svelte`          | Same SSE + badge changes; remove auto-open; modal checkbox disable; 409 toast                                     |

### Tests

- `update_dispatch.rs`: unit test for `has_active_update_for_host_software_item` covering all
  4 active statuses and verifying terminal statuses (`Completed`, `Failed`) return `false`.
- `update_triggers.rs`: test that duplicate trigger for same (host, software_item) returns
  `UpdateAlreadyActive` for each of the four non-terminal statuses.
- `software_items/mod.rs` query: test `AwaitingRestart` row populates
  `active_update_history_id` and `active_update_status = "awaiting_restart"`.
- `admin_events.rs`: update `all_variants()` helper and `KNOWN_VARIANTS` to include `status`
  in the `UpdateTriggered` constructor.
- `crates/ui/web-api/src/routes/events.rs` (lines 164–183): existing test constructs
  `UpdateTriggered` without `status` — update the struct literal to add `status: "pending".to_string()`.
- `events_stream.rs` (openapi-client): update `parse_update_triggered` test to include
  `status` in the JSON fixture.
- Frontend: update `SoftwareGroupList.test.ts` for all four active-status badge variants;
  update `software-trigger-status.test.ts` for 409 path; run `npm run test:e2e` (Playwright
  on macOS+Chromium) for badge DOM changes per quality-gate requirement.

---

## Documentation Deliverables

- `docs/development/coding-standards.md` — add note to "Database Query Patterns" that
  `UpdateStatus::unfinished()` and `host_blocking()` are the canonical groupings for active
  vs. host-occupying statuses; do not inline the status arrays at new callsites.
- `CONTEXT.md` — update the `UpdateStatus` glossary entry to list the two new grouping
  helpers and their semantics.
- `crates/shared/wire/asyncapi.yaml` — already listed in Affected Files; serves as protocol
  documentation.
- No new ADR required — the partial unique index and pre-check are implementation details of
  an existing constraint, not a reversible architectural decision.

---

## Deferred

- Dedicated `/terminal/:update_history_id` route.
- Batch update interactive mode.
- Push notification for update completion.
- History page live badge updates.
