# Services + System Services Routes Migration — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md` (§4.3 Buttons, §4.6 Loading State,
§4.7 Tables)

**Sub-spec #3i of the UI design-language rollout.** Depends on sub-spec #2 (Button primitive) merged and #2c merged
(`ariaLabel` prop, `--bg-hover` token). Form-input sites (select, input, checkbox) defer to a future #3i2 pass after
sub-spec #2b primitives land.

## Overview

Migrate two sibling route pages:

- `/services/+page.svelte` (675 lines) — runtime service admin: capability filter chips, paginated list with per-row
  context-menu trigger, error-state Retry, and two file-local modals (Merge, Edit Ping Interval). Row-level state
  transitions (Approve/Reject/Delete/ Merge Into…/Edit Ping Interval) are rendered inside `<ContextMenuShell>` via
  `<ContextMenuItem>` — those deferred to #3k.
- `/system-services/+page.svelte` (642 lines) — system-service admin: status filter chips, paginated list with per-row
  context-menu trigger, error-state Retry, and a file-local Ping Interval modal. Row-level transitions
  (Approve/Reject/Delete/Edit Ping Interval) are `<ContextMenuItem>` entries inside `<ContextMenuShell>` — also deferred
  to #3k.

Scope here is strictly the Skeleton preset buttons that live directly inside these two `+page.svelte` files. Shared
components (`ContextMenuShell`, `ContextMenuItem`, `BatchActionBar`, `ConfirmDialog`, `BatchResultDialog`) are migrated
end-to-end by #3k and are out of scope — including their `confirmClass` `preset-filled-*-500` string inputs computed at
call sites in these two pages (those become #3k's tone enum inputs on the same PR that migrates the shared components).

## Design decisions

**Q1 — Row-level state transitions (Approve / Reject / Deactivate / Delete / Merge Into… / Edit Ping Interval).**

- Options:
  - (chosen) Defer entirely to #3k. These actions are rendered as `<ContextMenuItem label="…" destructive?>` children
    inside `<ContextMenuShell>`. `ContextMenuItem` has no `variant` or `loading` prop today; re-shaping its API is #3k's
    job, not a per- consumer concern. The `confirmClass` preset strings fed into `<ConfirmDialog confirmClass={...}>`
    from `confirmLabels` (lines 394–398 services, 370–374 system-services) are likewise #3k concerns — ConfirmDialog's
    own confirm button is migrated on the same PR that swaps its external API from `confirmClass` preset strings to a
    `confirmTone: 'primary' | 'danger'` enum (or similar).
  - Migrate `ContextMenuItem` inline here. Rejected — shared primitive owned by #3k; double-migration risk.
- Reasoning: each surface migrates its own buttons; primitive components are owned by their own sub-specs (same boundary
  applied in #3h for host-detail context menu).

**Q2 — Capability / status filter chips.**

- Options:
  - (chosen) `<Button variant="ghost" size="sm">` for inactive,
    `<Button variant="ghost" size="sm" class="text-[var(--accent)] bg-[var(--bg-hover)]">` for active. Same two-token,
    same-order override contract as #3b (navbar), #3c (settings tabs), #3g (history filter chips). Fourth + fifth
    consumers of the pattern (services has one filter, system-services has another). The current source shape is
    `preset-filled-primary-500` (active) vs `preset-tonal` (inactive); this migration _changes_ that — the ghost +
    accent-override replaces the filled primary → tonal toggle and is the intentional cross-surface alignment.
  - Keep filled-primary active style as `<Button variant="primary">`. Rejected — breaks navbar/tab/chip pattern;
    overweights a non-primary-action toggle.
  - Extract `<FilterChip>` primitive now. Deferred — still mechanical override; revisit after a sixth consumer emerges
    (#3g deferred at four; consistency in deferral rule).
- Reasoning: cross-surface consistency beats local semantic nuance; five consumers share one shape.

Filter chips on `/services` are a **capability** filter (`all | software_discovery | ssh_remote`, labels All Services /
Agents / SSH Agents). Filter chips on `/system-services` are a **status** filter
(`all | pending | approved | rejected | deactivated`). Both use the same migration shape — only the chip count + label
set differs.

**Q3 — Row action trigger (ellipsis menu button).**

- Options:
  - (chosen) `<Button variant="ghost" size="sm" ariaLabel={`Actions for
    ${service.friendly_name}`} leadingIcon={EllipsisIcon} onclick={(e) => toggleMenu(service.id, e.currentTarget)}>`.
    Icon-only trigger — no children; `ariaLabel` supplies the accessible name per #2c's ariaLabel prop contract.
    Preserves the existing `e.stopPropagation()` + `e.currentTarget` call (positioning source for the popover) — the
    Button primitive forwards the native event object unchanged.
  - Keep the raw `&#8943;` unicode character as children. Rejected — icon-only buttons need a real icon for theme tint;
    the unicode ellipsis doesn't pick up `--accent` and fails dark-mode contrast. An `EllipsisIcon` component (matching
    the PlayIcon / ChevronIcon shape used elsewhere) is the replacement.
- Reasoning: #2c's ariaLabel contract is the standard for icon-only buttons; same shape used on host-detail context
  trigger in #3h.

**Q4 — Retry button in error snippets.**

- Options:
  - (chosen) `<Button variant="primary" loading={isRetrying}>Retry </Button>` — same rationale as #3e Q2 / #3h Q1: Retry
    is the sole action in an error boundary, hence primary by context. Size remains default `md` (not `sm`). A new local
    `let isRetrying = $state(false)` flag wraps each file's `loadServices(...)` invocation with
    `try { ... } finally { isRetrying = false; }` so the spinner window is the actual fetch window.
  - Reuse the existing `submitting` flag. Rejected — `submitting` is shared across batch + confirm actions; binding
    Retry to it would surface spurious spinners on unrelated flows.
- Reasoning: Retry is the primary action of its error view; per- action loading flags prevent false spinners across
  unrelated state.

**Q5 — Modal footer buttons (Merge + Ping services; Ping system-services).**

- Options:
  - (chosen) Cancel → `<Button variant="secondary">`. Submit →
    `<Button variant="primary" loading={submitting} disabled={!mergeTargetId && kind==='merge'}>Merge</Button>` /
    `<Button variant="primary" loading={submitting}>Save</Button>`. Drop the `{submitting ? 'Merging...' : 'Merge'}` and
    `{submitting ? 'Saving...' : 'Save'}` text-swap expressions per #3c Q4 — the Button primitive's spinner preserves
    children text. `disabled` forwards `!mergeTargetId` on Merge (unchanged semantics; the `|| submitting` disjunct is
    dropped because `loading=true` already sets `disabled=true` via the primitive's #2c loading→disabled contract).
  - Keep the text swap. Rejected — #3c Q4 is the locked loading contract; five + sub-specs already converged on it.
- Reasoning: consumers converge on a single loading UI; preserves the established #3c Q4 pattern.

**Q6 — Shared component migration boundary.**

- Options:
  - (chosen) `<BatchActionBar>`, `<ContextMenuShell>`, `<ContextMenuItem>`, `<ConfirmDialog>`, `<BatchResultDialog>`,
    `<ModalShell>` all defer to #3k. This sub-spec does not touch their internals. The call sites in these two pages
    pass their existing props through unchanged.
  - Migrate any shared primitive touched by these pages. Rejected — #3k owns them end-to-end to avoid split-migration.
- Reasoning: scope discipline matching #3c–h precedent.

## Goals

1. Every **directly rendered** `<button>` in both files renders through `<Button>`. Actions rendered via shared
   components (context menu items, batch bar, confirm dialogs, batch result dialog, modal shell) remain untouched and
   migrate with their owning sub-spec (#3k).
2. Filter chips adopt ghost + active-override pattern (capability chips on services, status chips on system-services) —
   matching navbar / settings tabs / history.
3. Row action ellipsis trigger adopts `variant="ghost" size="sm"` + `ariaLabel` + `leadingIcon={EllipsisIcon}`
   (icon-only; no children).
4. Error-state Retry adopts `variant="primary"` + new local `isRetrying` loading flag.
5. Modal footer buttons (Merge + Ping modals) adopt `variant="secondary"` (Cancel) / `variant="primary"` (Submit) with
   `loading={submitting}`; `{submitting ? 'Saving…' : …}` text-swap expressions removed.

## Non-goals

- **Features that do not exist in the current source**: approve / reject / deactivate / reactivate / delete direct row
  buttons; "Run now"; "Register" launcher; "Assign to host" launcher. None of these exist in `services/+page.svelte` or
  `system-services/+page.svelte` today. Row lifecycle transitions are `ContextMenuItem` entries only, and adding direct
  row buttons is out of scope for this migration. An earlier draft of this spec listed these as in-scope; those claims
  were wrong against the current source and have been removed.
- Form-input migration (select, input, checkbox in modal bodies; table-header `<input type="checkbox">` for "Select all"
  and per-row "Select {name}" checkboxes) — deferred to #3i2 after #2b primitives land.
- Shared component internals — migrated by #3k.
- Service backend logic — out of scope.
- New system-service types or new row actions — feature work.

## Scope

Files migrated:

- `frontend/src/routes/services/+page.svelte`
  - 3 capability filter chips (All Services / Agents / SSH Agents) → ghost + active-override (Q2).
  - Row-level ellipsis trigger → ghost + sm + ariaLabel + icon (Q3).
  - Error snippet Retry → primary + new `isRetrying` flag (Q4).
  - Merge modal footer: Cancel → secondary; Merge submit → primary + `loading={submitting}` + static `Merge` children
    (Q5).
  - Edit Ping Interval modal footer: Cancel → secondary; Save submit → primary + `loading={submitting}` + static `Save`
    children (Q5).

- `frontend/src/routes/system-services/+page.svelte`
  - 5 status filter chips (All / Pending / Approved / Rejected / Deactivated) → ghost + active-override (Q2).
  - Row-level ellipsis trigger → ghost + sm + ariaLabel + icon (Q3).
  - Error snippet Retry → primary + new `isRetrying` flag (Q4).
  - Edit Ping Interval modal footer: Cancel → secondary; Save submit → primary + `loading={submitting}` + static `Save`
    children (Q5).

Explicitly not migrated here (listed to prevent spec bleed at implementation time):

- `frontend/src/lib/components/ContextMenuShell.svelte`, `ContextMenuItem.svelte`, `BatchActionBar.svelte`,
  `ConfirmDialog.svelte`, `BatchResultDialog.svelte`, `ModalShell.svelte` — all owned by #3k.
- The `confirmClass: 'preset-filled-success-500' | 'preset-filled-error-500'` inputs in the `confirmLabels` tables
  (services line 394, system-services line 370) and in the inline batch-confirm expression (services line 568,
  system-services line 552) — these are inputs to `<ConfirmDialog>`, migrated by #3k alongside the dialog's internal
  confirm button.

## Migration pattern

Per-attribute translation:

- `preset-filled-primary-500` (filter active) / `preset-tonal` (filter inactive) →
  `<Button variant="ghost" size="sm" class={active ? 'text-[var(--accent)] bg-[var(--bg-hover)]' : ''}>`.
- `preset-tonal` on the ellipsis trigger → `<Button variant="ghost" size="sm" ariaLabel={`Actions for
  ${service.friendly_name}`} leadingIcon={EllipsisIcon}>` (no children).
- `preset-filled-primary-500` on the error Retry button →
  `<Button variant="primary" loading={isRetrying}>Retry</Button>`.
- `preset-tonal-surface` on modal Cancel → `<Button variant="secondary">`.
- `preset-filled-primary-500` on modal Submit → `<Button variant="primary" loading={submitting}>` with static children
  (`Merge` / `Save`).

Async wiring:

- Retry: add `let isRetrying = $state(false)`. Snippet handler becomes
  `async () => { isRetrying = true; try { await loadServices(currentPage); } finally { isRetrying = false; } }`. This is
  strictly a flag wrapper — `loadServices` itself is unchanged.
- Modal submits: reuse the existing `submitting` flag (already shared between batch, confirm, merge, and ping flows).
  Because #2c's `loading=true` contract sets `disabled=true` internally, the Merge button's
  `disabled={!mergeTargetId || submitting}` expression collapses to `disabled={!mergeTargetId}` after migration; the
  Ping button's `disabled={submitting}` is dropped entirely (the primitive handles it via `loading`).
- Row ellipsis trigger: introduce an `EllipsisIcon` icon component in `frontend/src/lib/components/icons/` following the
  existing PlayIcon / ChevronIcon shape (static SVG in a Svelte file, no props). If an equivalent already exists at
  migration time, reuse it instead.

Filter chip scope note: the three capability filters on `/services` are not a superset of the five status filters on
`/system-services`. They are two distinct filter taxonomies; the migration applies the same _shape_ to both. Unit tests
enumerate each file's chip set independently.

## Data flow

Template-level only. No runtime behaviour changes. Existing filter handlers (`setFilter`, `setStatusFilter`), batch
selection, context- menu positioning, `ConfirmDialog` confirm-labels, and modal open/close pipelines all pass through
unchanged.

The only new local state is the per-file `isRetrying` flag (Q4); everything else reuses existing `submitting`,
`selectedIds`, `batchConfirmAction`, `confirmAction`, `mergeSource`, `editPingService` signals.

## Error handling

- Button primitive's discriminated union catches invalid prop combos at compile time (including the polymorphic
  href/onclick guard for the ellipsis trigger, which always takes the `onclick` branch).
- Retry error propagation: existing error-toast pipeline in `loadServices` unchanged — Button merely renders loading
  state.
- Modal submit errors surface via existing toast pipeline; `submitting` returns to `false` in the existing `finally`
  block, so the primitive's loading state clears automatically.

## Testing

### Unit tests

Extend `services.test.ts` and `system-services.test.ts`:

- **Filter chip matrix** (per file, one case per chip):
  - Inactive chip renders `variant="ghost" size="sm"` with no `text-[var(--accent)]` / `bg-[var(--bg-hover)]` fragments
    in its classlist.
  - Active chip renders both fragments simultaneously; assertion checks both, in either order (the primitive does not
    preserve consumer class order in the final DOM).
  - Switching the active chip updates both ARIA + class fragments in a single render.

- **Row ellipsis trigger**:
  - Renders `variant="ghost" size="sm"` with no children text.
  - `aria-label` matches `Actions for ${service.friendly_name}` (verify for a row where `friendly_name` contains a
    space).
  - `onclick` preserves `e.stopPropagation()` + `e.currentTarget` positioning — mount one row, click the ellipsis,
    assert `toggleMenu` was called with the row id and an `HTMLElement` (not `null`/`undefined`).

- **Retry button**:
  - Renders `variant="primary"` (no size override → default md).
  - On click, `loading=true` for the duration of the awaited `loadServices` call; flips back to `false` after both
    resolution and rejection paths (test both). Assert `aria-busy="true"` during the loading window.

- **Merge modal (services)**:
  - Cancel renders `variant="secondary"`.
  - Merge submit renders `variant="primary"` + `loading={submitting}`; children text stays `Merge` across the submit
    window (regression guard that the `Merging...` text-swap is gone).
  - `disabled` follows `!mergeTargetId` alone (submitting-disabled is covered by the primitive's loading→disabled
    contract; separate assertion that `loading=true` also sets `aria-disabled="true"`).

- **Ping modal (both files)**:
  - Cancel renders `variant="secondary"`.
  - Save submit renders `variant="primary"` + `loading={submitting}`; children text stays `Save` across the submit
    window.

- **Out-of-scope regression guard**: assert that row `ContextMenuItem` elements for Approve / Reject / Delete / Merge
  Into… / Edit Ping Interval are _not_ wrapped in `<Button>` — they remain `<ContextMenuItem>` instances and defer to
  #3k. Flip test: rendering a `pending` row still exposes `Approve` + `Reject` via the context-menu panel (menu-trigger
  click → menu opens → items present).

### Integration / e2e

- Playwright re-baseline `/services` (default capability = all, pending row present, selected-row batch bar visible) and
  `/system-services` (default status = all, filter switch to `pending`, induced error state), each in dark + light
  themes.
- Delta enumeration per parent §9 (split by size):
  - Filter chips + ellipsis trigger (`size="sm"`): `h-[19px]`, label `8.5px` uppercase; active chip renders `--bg-hover`
    background and `--accent` text.
  - Retry + modal submit/cancel (`size="md"`): `h-[23px]`, label `9px` uppercase; primary variant renders the default
    gradient, secondary renders the #2c secondary contract.
- Snapshot masking per parent §3 (total masked area under 15%):
  - Mask in-flight spinner rotation on every `<Button loading>` site (Retry during fetch window, modal submit during
    save window).
  - Mask `last_seen_at` timestamp cells (dynamic) and any toast banners raised by save / batch flows.
  - Mask batch-selection count text in `BatchActionBar` (`{selectedCount} selected`) — it moves per row and is not the
    button chrome under test.
- Smoke tests:
  - Capability filter click on `/services` (all → Agents → SSH Agents) — assert active-chip class fragments appear only
    on the currently selected chip.
  - Status filter click on `/system-services` (all → Pending → Approved) — same assertion.
  - Error-state Retry: induce fetch failure (mock a 500 on `loadServices`), assert Retry button renders, click it,
    assert `aria-busy="true"` during the refetch, then either a second failed render (spinner clears) or the table
    repopulates (spinner clears + rows appear).

### Out-of-scope in tests

- Context-menu item variant / loading assertions — those belong to #3k.
- `BatchActionBar` launcher / cancel button assertions — those belong to #3k.
- `ConfirmDialog` confirm/cancel button assertions — those belong to #3k (including the `confirmClass` preset-string
  input transition to a tone enum).

## Rollout

Single PR titled "feat(frontend): migrate services + system-services to Button primitive (sub-spec #3i)".

1. Add `EllipsisIcon` component under `frontend/src/lib/components/icons/` (or reuse if present).
2. Migrate `services/+page.svelte` — filter chips, ellipsis trigger, error Retry, Merge modal footer, Ping modal footer.
   Wire new `isRetrying` flag. Strip modal `Merging…` / `Saving…` text-swap.
3. Migrate `system-services/+page.svelte` — filter chips, ellipsis trigger, error Retry, Ping modal footer. Wire new
   `isRetrying` flag. Strip modal `Saving…` text-swap.
4. Extend unit tests per plan.
5. Re-baseline Playwright snapshots for both routes in both themes.
6. Full frontend gate.

### Risk + rollback

Revert of one PR restores preset classes across both routes. Moderate sensitivity — filter chip pattern changes visual
style across five consumer surfaces simultaneously once this PR lands; mitigated by the Playwright re-baseline covering
both routes' filter states and by unit-test assertions on the active-chip class fragments. The row-action context menu
is intentionally untouched, so the critical state-transition surface (approve/reject/delete/merge) has zero visual or
behavioural delta from this PR.

### Dependencies + ordering

- **Blocks on:** sub-spec #2 merged; sub-spec #2c merged (`ariaLabel` prop on Button, `--bg-hover` token on active
  filter chips, `loading→disabled` contract for modal submits); sub-spec #3b merged (navbar-pill baseline for the
  ghost+active-override pattern).
- **Blocks:** #3i2 form-input migration (modal selects, inputs, checkboxes; requires #2b + #2d Textarea primitive
  parity).
- **Coordinates with #3k:** this sub-spec intentionally leaves five shared components and the `confirmClass`
  preset-string API unmigrated; #3k absorbs them. Landing order is independent — whichever lands first leaves a
  temporary mixed-style call site (ghost chip + preset-classed context menu, or preset chip + migrated context menu),
  which is expected and not a regression.
- **Parallel-safe with:** sub-spec #3c–h, #3j, #4.
