# History Migration — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md` (§4.3 Buttons, §4.5 Cards, §4.7
Tables, §4.6 Loading)

**Sub-spec #3g of the UI design-language rollout.** Depends on sub-spec #2 (Button primitive) merged. Form-input sites
(date pickers, search) defer to a future #3g2 pass after sub-spec #2b primitives land.

## Overview

Migrate `/history/+page.svelte` (722 lines) — the update history list view, its filter chips, per-row expand-log action,
the "Trigger Update" header launcher and its modal (Cancel + Trigger Update submit), and the "Input Required" badge
adjacency for in-progress interactive updates (per memory note on `update_history.interactive`). The `<Pagination>`
shared component renders raw preset buttons today but is owned by sub- spec #3k (shared modals + dialogs) end-to-end;
this sub-spec does not touch `Pagination.svelte` internals. `<TerminalOutput>` action-slot buttons (Ctrl+C etc.) are
likewise delegated to TerminalOutput's own migration and are out of scope here.

## Design decisions

**Q1 — Per-row expand-log action shape.**

- Options:
  - (chosen) Keep the existing expand/collapse semantic (click toggles inline terminal panel — matches current
    `toggleExpand` + `expandedId` state; no behavior change). Render as
    `<Button variant="ghost" size="sm" leadingIcon={ChevronIcon}>` whose children text reflects four states: idle-closed
    "View logs", idle-open "Hide logs", interactive-closed "Attach terminal", interactive-open "Close terminal". Bind
    `loading` to `expandedId === item.id && wsState === 'connecting'` so the spinner appears during the WebSocket attach
    window. `aria-label` mirrors children via the primitive's default (children is the accessible name) — no separate
    `ariaLabel` override.
  - Use a link primitive. Rejected — this is not a navigation link; it's an action that opens an in-view panel.
- Reasoning: consistent with #3f's row-action shape; "ghost + icon + text label" is the emerging row-level action
  standard. The four- state children text removes the previous ambiguity between label- swap and expand/collapse; both
  are preserved.

**Q2 — Filter chip buttons.**

- Options:
  - (chosen) `<Button variant="ghost" size="sm">` for both inactive and active chips; active state adds the
    consumer-level override class `class="text-[var(--accent)] bg-[var(--bg-hover)]"` (same order and same two tokens as
    #3b navbar pills and #3c settings tab pills). The `--bg-hover` token specifically (not `--bg-raised`) is the
    contract, introduced by #2c. `variant="secondary"` is not used here because secondary encodes a "reversible side
    action" intent (e.g. Cancel, Back, Edit) — filter chips are state toggles, not reversible actions in the verb sense.
  - Use `<Button variant="secondary">` for active chips. Rejected — semantic mismatch per above; also differs from the
    established navbar/tab-pill pattern.
  - Introduce a `<FilterChip>` primitive. Rejected — third consumer of the same ghost+override pattern; still YAGNI
    until a fourth consumer emerges.
- Reasoning: cross-surface consistency (navbar, tab, chip all share the same ghost + accent-override) beats semantic
  nuance at three consumers.

**Q3 — "Input Required" badge interaction.**

- Options:
  - (chosen) Leave the existing badge untouched (it's a status indicator, not a button). The row's "View logs" button is
    where the interactive terminal session attaches — rename label to "Attach terminal" for rows where
    `interactive && in_progress`, no variant change.
  - Promote badge to a button itself. Rejected — semantic mismatch; status vs. action.
- Reasoning: status badges remain badges; the button adjacent to them owns the attach action.

**Q4 — "Rerun" action scope (if present on history rows).**

- Options:
  - (chosen) Migrate if the action exists today. If not rendered in current markup, skip — scope limited to existing
    buttons.
  - Add rerun action as part of migration. Rejected — feature work, not style work.
- Reasoning: sub-spec is migration-only; functional changes belong elsewhere.

## Goals

1. Every interactive button in `/history/+page.svelte` renders through `<Button>`.
2. Filter chips adopt ghost + active-override pattern.
3. Row-level "View logs" / "Attach terminal" adopt `variant="ghost" size="sm" leadingIcon={...}`.
4. "Input Required" badge remains a status indicator (no button).

## Non-goals

- Form-input migration (date pickers, search input) — deferred to #3g2.
- History backend / SSE schema changes — out of scope.
- Log viewer panel refactor — outside Button scope.
- Rerun functionality — feature work, separate spec.

## Scope

Files migrated:

- `frontend/src/routes/history/+page.svelte` — filter chips (Q2), per-row expand-log action (Q1), the "Trigger Update"
  header launcher that opens the trigger modal (`variant="primary"` `size="sm"` on the launcher; no `loading` — it only
  opens a modal), and the trigger- modal action row inside the same file: Cancel (`variant="secondary"`) and Trigger
  Update submit (`variant="primary" loading={triggering}` with the existing `triggering` flag and the text-swap removed
  per the #3c Q4 contract).

Explicitly not migrated here:

- `frontend/src/lib/components/Pagination.svelte` — shared component, owned end-to-end by #3k.
- `frontend/src/lib/components/TerminalOutput.svelte` — shared component; its `actions`-slot buttons (Ctrl+C signal,
  etc.) defined via `terminalActionsFor()` in `+page.svelte` are a TerminalOutput migration concern (same sub-spec that
  migrates TerminalOutput itself).
- The trigger-modal's form-input bodies (select + checkbox fields) — deferred to #3g2 after #2b primitives land.

## Migration pattern

Standard translation rules (preset-filled-primary → primary, preset-filled-error → danger, preset-tonal-\* →
secondary/ghost).

Special:

- Filter chips:

  ```svelte
  <Button
    variant="ghost"
    size="sm"
    onclick={() => toggleFilter(...)}
    class={activeFilters.has(filter) ? 'text-[var(--accent)] bg-[var(--bg-hover)]' : ''}
  >
  ```

- Per-row expand action (Q1):

  ```svelte
  <Button
    variant="ghost"
    size="sm"
    leadingIcon={expandedId === item.id ? ChevronDown : ChevronRight}
    loading={expandedId === item.id && wsState === 'connecting'}
    onclick={() => toggleExpand(item)}
  >
    {#if item.interactive && item.status === 'in_progress'}
      {expandedId === item.id ? 'Close terminal' : 'Attach terminal'}
    {:else}
      {expandedId === item.id ? 'Hide logs' : 'View logs'}
    {/if}
  </Button>
  ```

  The `interactive` flag mutates via the existing SSE `update_started` handler (wire unchanged); the children text
  re-renders automatically. On WebSocket connection failure the existing error pipeline surfaces the toast and `wsState`
  returns to its idle state — the Button primitive's `loading` prop follows suit (no stuck spinner).

- "Trigger Update" header launcher →
  `<Button variant="primary" size="sm" onclick={openTriggerModal}> Trigger Update</Button>`.
- Trigger modal Cancel → `<Button variant="secondary">`; Submit →
  `<Button variant="primary" loading={triggering}>Trigger Update</Button>` with any existing
  `{triggering ? 'Triggering…' : 'Trigger Update'}` text-swap expression replaced by a static children label (the
  primitive's spinner + preserved children contract handles loading UI, per #3c Q4).

## Data flow

Template-level only. No runtime changes to SSE subscription, filter state, or pagination.

## Error handling

Button discriminated union catches invalid prop combos at compile time. Existing error pipelines (failed row, failed SSE
reconnect) unchanged.

## Testing

### Unit tests

Extend `history/+page.test.ts`:

- Filter chip inactive renders `variant="ghost" size="sm"` with no override class fragments; active chip renders both
  `text-[var(--accent)]` AND `bg-[var(--bg-hover)]` fragments.
- Per-row expand action variant matrix (all rendered via the single `<Button>` call; rows set as props permutation):
  - `interactive=false, status=completed, expandedId=null` → children text `View logs`, no `loading`.
  - `interactive=false, status=completed, expandedId=item.id` → children text `Hide logs`, icon ChevronDown.
  - `interactive=true,  status=in_progress, expandedId=null` → children text `Attach terminal`.
  - `interactive=true,  status=in_progress, expandedId=item.id, wsState='connecting'` → `loading=true` +
    `aria-busy="true"`.
  - `interactive=true,  status=in_progress, expandedId=item.id, wsState='connected'` → `loading=false`, children
    `Close terminal`.
- SSE-driven transition test: mock `connectEventStream` to emit `update_started` with `{interactive: true}` for a
  pending row; assert the row's action button re-renders from `View logs` to `Attach terminal`. Emit `update_completed`;
  assert children revert to `View logs` and `wsState` resets to idle.
- Trigger Update header launcher renders `variant="primary" size="sm"` (no `loading`). Modal Cancel renders
  `variant="secondary"`. Modal submit renders `variant="primary" loading={triggering}` and keeps static children
  `Trigger Update` across the submit window (regression guard that the `Triggering…` text-swap expression is gone).

### Integration / e2e

- Playwright re-baseline `/history` default + filter-active + in-progress-interactive row permutations, in dark + light
  themes.
- Delta enumeration per parent §9 (split by size):
  - Filter chips + per-row actions + Trigger Update launcher (all `size="sm"`): `h-[19px]`, label `8.5px` uppercase.
    Active filter chips render with `--bg-hover` background and `--accent` text.
  - Trigger modal submit + Cancel (`size="md"`): `h-[23px]`, label `9px` uppercase.
- Snapshot masking per parent §3 approved dynamic categories:
  - Mask `formatRelativeTime` spans (started_at / completed_at) — they shift between snapshots.
  - Mask `terminalDurationLabel` spans on in-progress rows (duration ticks during the snapshot window).
  - Mask in-flight spinner rotation on every `<Button loading>` site (including the per-row action during
    `wsState='connecting'`).
  - Mask transient toast banners raised by trigger or WS errors.
  - Total masked area stays under 15% per parent §3.
- Smoke test SSE flow: pending → in_progress (interactive=true) → completed. Assert the per-row button's
  `aria-busy="true"` during `wsState='connecting'` and the children text transitions above.

## Rollout

Single PR titled "feat(frontend): migrate history to Button primitive (sub-spec #3g)".

1. `history/+page.svelte` — migrate filters + row actions + pagination triggers.
2. Extend unit tests per plan.
3. Re-baseline Playwright snapshots.
4. Full frontend gate.

### Risk + rollback

Revert of one PR restores preset classes across history. Moderate sensitivity — SSE-driven live updates are the primary
regression concern, mitigated by existing in-progress-row unit tests and Playwright live-update smoke.

### Dependencies + ordering

- **Blocks on:** sub-spec #2 merged, sub-spec #2c merged (`--bg-hover` for active-filter override), sub-spec #3b merged
  (navbar baseline).
- **Blocks:** #3g2 form-input migration.
- **Parallel-safe with:** sub-spec #3c–f, #3h–k, #4.
