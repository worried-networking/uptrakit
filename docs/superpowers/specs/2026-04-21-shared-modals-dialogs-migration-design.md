# Shared Modals + Dialogs Migration — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md` (§4.3 Buttons, §4.8 Modals, §4.5
Cards)

**Sub-spec #3k of the UI design-language rollout.** Depends on sub-spec #2 (Button primitive with `leadingIcon` /
`trailingIcon` already shipped) and sub-spec #2c merged (`variant="secondary"` + base-Button `ariaLabel` prop).
Form-input sites inside modals defer to a future #3k2 pass after sub-spec #2b primitives land.

## Overview

Migrate seven shared UI components referenced by every route: `ConfirmDialog.svelte` (64 lines),
`BatchResultDialog.svelte` (47), `BatchActionBar.svelte` (173), `Pagination.svelte` (83), `ToastNotifications.svelte`
(406), `AssignToHostModal.svelte` (550), `EditHostAssignmentModal.svelte` (~1365). These are the cross-cutting
primitives — a regression here hits every consumer at once.

Scope boundary callouts against current source:

- `AddSoftwareModal.svelte` and `SoftwareMergeWizard.svelte` have preset-
  - buttons but are owned by sub-spec **#3h (software area)** end-to-end. They are explicitly NOT migrated here — see
    `2026-04-21-software-area- migration-design.md` Scope section. Don't double-migrate.
- `Modal.svelte` and `ContextMenu.svelte` contain no interactive `<button>` elements (they are pure layout/keyboard
  shells). No migration work. Predecessor specs occasionally referenced these as `ModalShell.svelte` /
  `ContextMenuShell.svelte`; those are stale aliases, not separate files.
- `ui/ContextMenuItem.svelte` uses raw `<button>` styled with design tokens (`var(--bg-raised)`, `var(--color-error)`) —
  no preset-\* classes to migrate. Out of scope. Row-menu consumers in #3i / #3j / #3h invoke this component unchanged.

## Design decisions

**Q1 — ConfirmDialog confirm/cancel variants.**

- Options:
  - (chosen) Replace the current `confirmClass?: string` prop (source line 10, default `'preset-filled-error-500'`) with
    `confirmVariant?: 'primary' | 'danger'` (default `'danger'` to preserve today's visual default — the existing prop
    defaulted to the error preset, so every caller that did not override got a red confirm; flipping the default to
    `'primary'` would silently weaken destructive prompts). Cancel is always `variant="secondary"`. The existing
    `confirmDisabled` prop (source line 11) is preserved as-is and passed through to the Button's `disabled` prop —
    note the prop is named `confirmDisabled`, not `disabled`.
  - Always `primary` + `secondary`. Rejected — callers need danger semantics; hardcoding `primary` would silently weaken
    every destructive confirm.
- Reasoning: ConfirmDialog is the most-used primitive for destructive confirms across the app (Delete rule, Delete
  provider, Revoke token, Rotate CA, etc.). A single `confirmVariant` enum is the correct generalization; the `'danger'`
  default preserves current behavior for every caller that hadn't explicitly overridden `confirmClass`. Callers that DID
  override (set `confirmClass` to a non-error preset) must migrate to `confirmVariant="primary"` — see Consumer audit
  below.

**Q2 — BatchActionBar action buttons.**

- Options:
  - (chosen) Preserve the existing `actions: { id: string; label: string; destructive?: boolean }[]` prop shape and the
    single `onaction: (actionId: string) => void` callback contract. Extend the type to
    `actions: { id: string; label: string; destructive?: boolean; variant?: 'primary' | 'secondary' | 'danger'; loading?: boolean }[]`
    — both new fields optional. Internally, the `{#each}` render loop maps
    `action.variant ?? (action.destructive ? 'danger' : 'primary')` onto
    `<Button variant={...} size="sm" loading={action.loading} onclick={() => onaction(action.id)}>`. No consumer site
    changes are required; existing `{id, label, destructive}` rows keep working.
  - Reshape to `actions: Array<{label, variant, onclick, loading}>` (per-action `onclick`, drop `onaction`). Rejected —
    every consumer (software, hosts, services bulk flows) would need rewriting in the same PR. The `id`+`onaction`
    contract already correctly separates the action list from the dispatch function; no benefit to collapsing them
    beyond the loading-state gap, which `loading?` above fixes.
- Reasoning: data-driven shape already exists in source; the only real gap is a per-action loading flag. Add it
  additively without churning consumer shape. The `variant` override exists for the rare case a consumer needs a
  non-destructive secondary action (e.g. a bulk "Mark as read" button sitting alongside a destructive "Delete
  selected").

**Q3 — Pagination buttons (size + shape).**

- Options:
  - (chosen) Migrate to `<Button>` but preserve the component's existing custom sizing via `class` override:
    `<Button variant="ghost" size="sm" class="h-8 min-h-8 px-3 text-[10px]" leadingIcon={ChevronLeft} disabled={currentPage <= 1} onclick={...}>Previous</Button>`
    and the mirror `trailingIcon={ChevronRight}` for Next. Page-number buttons share the same sizing override; the
    current (active) page uses `variant="ghost"` with the ghost+active class contract
    `class="h-8 min-h-8 min-w-8 px-2.5 text-[10px] text-[var(--accent)] bg-[var(--bg-hover)]"` — same override as #3b
    navbar pills, #3c settings-tab pills, #3g history filter chips, #3i filter chips. Inactive page numbers carry the
    same sizing override with no active class.
  - Drop the custom sizing and adopt standard `size="sm"` (`h-[19px]`). Rejected — Pagination renders at 32px row height
    today (`h-8`) for touch-target reasons; collapsing to 19px is a UX regression. The class override is explicitly the
    right vehicle for pagination- specific sizing without inventing a new size primitive.
  - Use `<Link>` primitive (#2b). Rejected — pagination mutates filter state only; no URL change, no navigation.
- Reasoning: Pagination is action-shaped (mutates state, not URL). Ghost variant matches. The custom height is a
  component contract, not a design-language regression — preserve it via `class` override. All three Pagination button
  kinds (Prev, page-number, Next) share the same sizing override for visual coherence.

**Q4 — ToastNotifications dismiss button.**

- Options:
  - (chosen) Preserve the current text label. Migrate source line 379
    `<button class="btn btn-sm preset-tonal-surface">Dismiss</button>` to
    `<Button variant="ghost" size="sm" onclick={() => dismissToast( item)}>Dismiss</Button>`. No icon change, no shape
    change — straight variant translation. Rationale: #3k is a migration-only sub-spec; swapping a text label for an
    icon-only affordance is a design change that belongs in its own design pass, not a preset-cleanup PR.
  - Swap to icon-only
    `<Button variant="ghost" size="sm" class="p-0 w-5 h-5" leadingIcon={CloseIcon} ariaLabel="Dismiss">`. Rejected —
    introduces a net-new UX (icon close), changes accessible name text from user-facing "Dismiss" to the (visually
    identical) aria-label string, and conflicts with the migration-only principle every other #3\* sub-spec follows. If
    parent §4 later wants icon-close toasts, that's a dedicated design change with its own baseline.
- Reasoning: migration-only purity + consumer visual stability. The toast dismiss is keyboard/screen-reader-reachable
  today with a visible label; preserving that is cheaper than a separate a11y regression test pass.

**Q4b — ToastNotifications `<a>` Call-to-Action anchor.**

- Source line 386: `<a href="/settings/global" class="btn btn-sm preset-tonal">Go to Global Settings</a>` — an anchor
  styled as a button. This is a route navigation (href present), not an action.
- Options:
  - (chosen) Defer to sub-spec **#2b** (`Link` primitive). #2b owns anchor-shaped styled affordances. The raw class
    `btn btn-sm preset- tonal` stays in the source file until #2b's migration pass. This sub-spec (#3k) migrates the
    dismiss `<button>` on line 379 only and leaves line 386 untouched. Unit tests on `ToastNotifications` must NOT
    assert a `<Button>` render for the anchor.
  - Wrap the anchor in `<Button href={...}>`. Rejected — Button primitive's `href` branch exists (#2 discriminated
    union) but the link-style token contract (underline hover, a11y focus ring) is #2b Link's responsibility. Avoid
    premature consolidation.
- Reasoning: the scope line for each sub-spec is "migrate preset-\* classes on `<button>` sites." `<a class="btn …">` is
  a Link concern by project convention; #3k does not reach into #2b territory.

**Q5 — AssignToHostModal / EditHostAssignmentModal scope.**

- Options:
  - (chosen) Migrate every `<button>` in both modals (enumerated in Scope below). Form-input sites (search box, select,
    textarea) defer to #3k2 after #2b + #2d primitives land. Modal shell (backdrop, focus trap) stays untouched — not a
    Button concern.
  - Defer modal migration entirely. Rejected — these two components render across every host/service management flow;
    leaving them on preset-\* classes creates visual inconsistency with migrated chrome.
- Reasoning: button-level migration is mechanical and essential for cross-surface consistency; form inputs are a
  separate concern tied to #2b + #2d.

**Q6 — Button primitive API surface (audit, not add).**

- `trailingIcon?: Snippet` already ships in the #2 Button primitive design (see
  `2026-04-21-shared-button-terminal-theme-design.md` line 75). No API add needed here. Earlier drafts of this spec
  listed a "flag primitive follow-up"; that was stale.
- `ariaLabel?: string` on the base Button ships in #2c (`2026-04-21- button-primitive-updates-design.md` line 20+).
  Required by Q4 icon- only toast variant — but since Q4 (chosen) preserves text and does NOT go icon-only, #3k has no
  surface that REQUIRES `ariaLabel`. #2c is still a dependency for `variant="secondary"` (used by Cancel buttons across
  all seven files) and the active-pill hover contract through `--bg-hover`.
- Reasoning: no primitive additions in this sub-spec. Remove the earlier "add trailingIcon" claim — it was internally
  contradictory (same spec also said "already shipped in #2").

## Goals

1. Every interactive `<button>` in the seven listed files renders through `<Button>`.
2. `ConfirmDialog` exposes `confirmVariant?: 'primary' | 'danger'` (default `'danger'`) and passes through the existing
   `confirmDisabled` prop (not `disabled`) to the Button primitive. The legacy `confirmClass?: string` prop is removed.
   Consumer call sites audit + update.
3. `BatchActionBar` accepts an extended `actions[]` shape with optional `variant` + `loading`; the `id`+`onaction`
   dispatch contract is preserved. No consumer migration required beyond adopting the new optional fields where useful.
4. `Pagination` renders through `<Button variant="ghost" size="sm">` with `class` sizing override + `leadingIcon` /
   `trailingIcon` / the standard ghost active-pill contract for the current page.
5. `ToastNotifications` dismiss button renders through `<Button variant="ghost" size="sm">Dismiss</Button>` with the
   existing text label preserved. The CTA `<a>` on source line 386 is untouched here (belongs to #2b).
6. `AssignToHostModal` + `EditHostAssignmentModal` have every `<button>` migrated to `<Button>`; form inputs unchanged.
7. No primitive API additions in this sub-spec.

## Non-goals

- Form-input migration inside modals — deferred to #3k2 (depends on #2b
  - #2d Textarea; `EditHostAssignmentModal` has 42 textarea sites).
- Modal shell (backdrop, focus trap) refactor — outside Button scope.
- Icon-only toast dismiss redesign — deferred to a future design pass.
- `ToastNotifications` line 386 CTA anchor — belongs to #2b Link.
- `AddSoftwareModal` + `SoftwareMergeWizard` — owned by #3h (software area). Confirm by reading #3h Scope before
  touching either file.
- `Modal.svelte` / `ContextMenu.svelte` — no buttons to migrate.
- `ui/ContextMenuItem.svelte` — already token-styled, no preset classes.
- ToastNotifications store architecture — untouched.

## Scope

Button sites enumerated exhaustively against current source. Adding buttons not listed here is out of scope.

### `frontend/src/lib/components/ConfirmDialog.svelte`

- Line 60: Cancel — **already migrated** to `<Button variant="ghost" onclick={oncancel}>Cancel</Button>`. The remaining
  step is to change `variant="ghost"` to `variant="secondary"` on this line (not a raw-button migration).
- Lines 61–63: Confirm (currently raw `<button class="btn {confirmClass}" disabled={confirmDisabled} onclick={onconfirm}>`)
  → `<Button variant={confirmVariant} disabled={confirmDisabled} onclick={onconfirm}>{confirmLabel}</Button>`.
- Prop change: drop `confirmClass?: string`, add `confirmVariant?: 'primary' | 'danger' = 'danger'`.

### `frontend/src/lib/components/BatchResultDialog.svelte`

- Line 45: Close (`preset-filled-primary-500`) → `<Button variant="primary">Close</Button>`.

### `frontend/src/lib/components/BatchActionBar.svelte`

- Line ~126: primary action render in `{#each primaryActions}` loop (`btn btn-sm preset-filled-primary-500`) →
  `<Button variant={ a.variant ?? (a.destructive ? 'danger' : 'primary')} size="sm" loading={a.loading} onclick={() => onaction(a.id)}>{a.label}</Button>`.
- Line ~132-133: secondary / destructive More-menu trigger (`btn btn-sm preset-tonal-surface`) →
  `<Button variant="secondary" size="sm">`.
- Line ~170: Deselect all (`btn btn-sm preset-tonal-surface`) → `<Button variant="secondary" size="sm">`.
- Prop type extension: extend existing `actions` to
  `{ id: string; label: string; destructive?: boolean; variant?: 'primary' | 'secondary' | 'danger'; loading?: boolean }[]`.
  `onaction` callback contract unchanged.

### `frontend/src/lib/components/Pagination.svelte`

- Line 52 (Previous): `<Button variant="ghost" size="sm" class="h-8 min-h-8 px-3 text-[10px]" leadingIcon={ChevronLeft}
  disabled={ currentPage <= 1} onclick={() => onPageChange(currentPage - 1)}
  > Previous</Button>`.
- Line 63 (page numbers):

  ```svelte
  <Button
    variant="ghost"
    size="sm"
    class={[
      'h-8 min-h-8 min-w-8 px-2.5 text-[10px]',
      p === currentPage ? 'text-[var(--accent)] bg-[var(--bg-hover)]' : ''
    ].join(' ')}
    aria-current={p === currentPage ? 'page' : undefined}
    onclick={() => onPageChange(p)}
  >
    {p}
  </Button>
  ```

- Line 74 (Next): `<Button variant="ghost" size="sm" class="h-8 min-h-8 px-3 text-[10px]" trailingIcon={ChevronRight}
  disabled={currentPage >= totalPages} onclick={() => onPageChange(currentPage + 1)}>Next</Button>`.
- No new primitive props introduced; `class` override is how every migrated consumer handles non-standard sizing.

### `frontend/src/lib/components/ToastNotifications.svelte`

- Line 379 (Dismiss): `<Button variant="ghost" size="sm" onclick={() => dismissToast(item)}>Dismiss</Button>`.
- Line 386 (`<a href="/settings/global">Go to Global Settings</a>`): **NOT migrated in this sub-spec.** Owned by #2b
  Link. Leave untouched including its current preset classes.

### `frontend/src/lib/components/AssignToHostModal.svelte`

- Line 371 + line 498 (addHook launcher, `btn btn-sm preset-tonal- surface text-xs`):
  `<Button variant="secondary" size="sm" type="button" onclick={() => addHook(hookRole)}>`. Drop the inline `text-xs` —
  the primitive's `size="sm"` label typography is the contract.
- Line 398 + line 525 (remove hook row-action, `btn btn-sm preset- tonal-error text-xs shrink-0`):
  `<Button variant="danger" size="sm" class="shrink-0">`. Keep the `shrink-0` class override (layout concern, not
  typography).
- Line 545 (Cancel, `btn preset-tonal-surface`): `<Button variant="secondary" onclick={onclose}>Cancel</Button>`.
- Line 546 (Submit, `btn preset-filled-primary-500` with `disabled={submitting || loading || !!loadError}`):
  `<Button variant="primary" loading={submitting} disabled={loading || !!loadError} onclick={submit}>Save</Button>`
  (children text static per #3c Q4 loading contract — no `{submitting ? 'Saving…' : …}` text-swap). Note the disabled
  passthrough splits off `submitting` → `loading` (primitive sets `disabled=true` while loading per #2c) and leaves the
  remaining reasons on `disabled`.

### `frontend/src/lib/components/EditHostAssignmentModal.svelte`

Twelve button sites verified in source. Group by role:

- Eight JSON view-mode toggle buttons (all `btn btn-sm preset-tonal text-xs`) — each flips a config editor between
  form-field view and raw-JSON `<textarea>` view. Source-verified children text per line:
  - Line 817: `Edit as JSON` (standard role override, form → JSON)
  - Line 841: `Back to Form` (standard role override, JSON → form)
  - Line 943: `Advanced: Edit as JSON` (standard role advanced toggle)
  - Line 969: `Back to Form` (standard role advanced, JSON → form)
  - Line 1137: `Edit as JSON` (hook entry, form → JSON)
  - Line 1163: `Back to Form` (hook entry, JSON → form)
  - Line 1270: `Advanced: Edit as JSON` (hook entry advanced)
  - Line 1298: `Back to Form` (hook entry advanced, JSON → form)

  Each migrates to `<Button variant="secondary" size="sm" type="button">` with the existing children text preserved and
  the existing `onclick` handler unchanged. Drop inline `text-xs`. Preserve any other utility classes (e.g. `shrink-0`)
  via the primitive's `class` prop. The blanket `secondary` assignment is verified against source: none of these eight
  buttons are destructive, none are the row's primary action — they are all reversible view-mode toggles, which is
  exactly the `secondary` semantic per parent §4.3.

- Line 1015 (`btn btn-sm preset-tonal-primary text-xs shrink-0`, children `+ Add` — hook-row primary action that appends
  a new hook entry):
  `<Button variant="primary" size="sm" class="shrink-0" type="button" onclick={() => addHook(hookRole)}>+ Add</Button>`.
- Line 1036 (`btn btn-sm preset-tonal-error text-xs`, children `Remove` — hook-entry destructive action):
  `<Button variant="danger" size="sm" type="button" onclick={() => requestHookRemoval(hookRole, entry.localKey)}>Remove</Button>`.
- Line 1346 (Cancel footer, `btn preset-tonal-surface`):
  `<Button variant="secondary" onclick={onclose}>Cancel</Button>`.
- Line 1347 (Save footer, `btn preset-filled-primary-500` with `disabled={submitting || loading || !!loadError}`): same
  pattern as AssignToHostModal line 546 —
  `<Button variant="primary" loading={submitting} disabled={loading || !!loadError} onclick={save}>Save Changes</Button>`,
  static children per #3c Q4. **Note:** the button label is "Save Changes" (with text-swap "Saving…" currently in
  source) — NOT "Save". Do not default to "Save" by analogy with AssignToHostModal.

The implementer MUST re-grep this file during execution; the line numbers above are approximate against the ~1365-line
source and may drift slightly. The canonical signal is the `btn preset-*` class substring.

## Migration pattern

Standard preset-→variant translation rules:

- `preset-filled-primary-500` → `<Button variant="primary">`
- `preset-tonal-primary` → `<Button variant="primary">` (row-level non-destructive primary actions; same variant —
  gradient/fill handled by the primitive)
- `preset-tonal-surface` → `<Button variant="secondary">`
- `preset-tonal` (row neutral) → `<Button variant="secondary">` (when context is row-level side action) or
  `<Button variant="ghost">` (when context is pagination — see Q3). Pick based on context, not class.
- `preset-tonal-error` / `preset-filled-error-500` → `<Button variant="danger">`

**Pagination exception — Q3 governs all page-number buttons:** The generic `preset-filled-primary-500 →
variant="primary"` rule does NOT apply to Pagination active page number buttons. Those must follow Q3: all page
buttons (active or inactive) use `variant="ghost"` with the class override described in the Pagination scope section.
The active page applies the additional `class="... text-[var(--accent)] bg-[rgba(var(--accent-rgb),0.12)]"` (or
`bg-[var(--bg-hover)]` per the Q3 ghost+active-pill contract) — not `variant="primary"`. Q3 is the governing rule for
every button inside `Pagination.svelte`.

Async wiring (#3c Q4 loading contract):

- Any button bound to a submitting/saving/loading state flag binds the flag to the primitive's `loading` prop; text-swap
  expressions (`{saving ? 'Saving…' : 'Save'}`) are removed; children render the static label.
- The primitive's spinner + preserved-children contract (parent §4.6 + #2c) handles the loading visual.

Consumer audit — `ConfirmDialog`:

- Grep every `<ConfirmDialog` call site. For each, inspect the `confirmClass` prop value (if any) and map:
  - Not set, or set to `'preset-filled-error-500'` → remove the prop (the new default `confirmVariant='danger'` produces
    identical visuals).
  - Set to `'preset-filled-primary-500'` or any non-error preset → replace with `confirmVariant="primary"`.
  - Set to any other string → flag in the PR description and resolve case-by-case.
- Update in the same PR. No consumer may continue to pass `confirmClass` after this sub-spec lands (the prop is
  removed).

Consumer audit — `BatchActionBar`:

- No migration required (additive extension). Existing `actions` prop shape continues to render; new `variant` /
  `loading` fields are optional. Consumers wanting per-action loading UI update their `actions` array at will, outside
  this PR.

## Data flow

Template-level + one API-removing + one API-additive change. `ConfirmDialog` removes `confirmClass`, adds
`confirmVariant`. `BatchActionBar` extends its `actions` prop type non-breakingly. No runtime behavior changes.
`Pagination` state flow unchanged. Toast store unchanged.

## Error handling

Button discriminated union catches invalid prop combos at compile time. Modal focus trap + keyboard handling unchanged.
`ConfirmDialog` prop rename is a compile-time break — every consumer gets a TypeScript error until migrated in the same
PR; this is the acceptance gate.

## Testing

### Unit tests

Extend existing / create new spec files:

- `ConfirmDialog.test.ts` — confirm button renders with configured variant: default call (no prop) → `variant="danger"`;
  `confirmVariant="primary"` → `variant="primary"`; cancel always `variant="secondary"`; `confirmDisabled` prop
  passthrough to Button `disabled` unchanged (assert the Button receives `disabled={true}` when `confirmDisabled={true}`).
- `BatchResultDialog.test.ts` (new or extended) — Close button renders `variant="primary"`.
- `BatchActionBar.test.ts` (new) — matrix:
  - Two actions, one `destructive: false` + one `destructive: true`, no `variant` override → renders `primary` +
    `danger`.
  - One action with `variant: 'secondary'` override → renders `secondary` regardless of `destructive` value.
  - One action with `loading: true` → `aria-busy="true"` on that button only (others' `aria-busy` absent).
  - `onaction(actionId)` fires with the correct `id` on click.
  - "Deselect all" button renders `variant="secondary" size="sm"`.
- `Pagination.test.ts` — Prev renders `variant="ghost" size="sm"` with `leadingIcon` present (assert DOM `data-icon`
  presence or the icon component's role); Next renders `trailingIcon`; current page carries both `text-[var(--accent)]`
  and `bg-[var(--bg-hover)]` class fragments; inactive pages carry neither; disabled passthrough on Prev/Next at page
  bounds; custom height class `h-8` present on all three.
- `ToastNotifications.test.ts` — Dismiss button renders `<Button variant="ghost" size="sm">` with children text
  `Dismiss` (regression guard that children text wasn't replaced with an icon). Line 386 anchor
  (`<a href="/settings/global">`) is NOT asserted as a `<Button>` render (that's #2b territory).
- `AssignToHostModal.test.ts` — addHook launcher renders `variant="secondary" size="sm"`; remove-hook renders
  `variant="danger" size="sm"`; Cancel renders `variant="secondary"`; Save renders `variant="primary"` +
  `loading={submitting}` wired through + static children `Save` across the submit window (regression guard that any
  pre-existing text-swap is gone) + `disabled` composition (`disabled={loading || !!loadError}` — NOT including
  `submitting`, which is now expressed through `loading`).
- `EditHostAssignmentModal.test.ts` — same pattern as `AssignToHostModal.test.ts` plus row-level Edit / Save / Delete
  row button variants (`secondary` / `primary` / `danger` respectively); `shrink-0` class preserved where applicable
  (row-flex layout).
- Consumer-site grep test: a repo-wide assertion (run as part of the frontend test suite — can be a simple `rg`
  invocation inside a `describe.todo` block or a lint rule) that no remaining Svelte file passes a `confirmClass` prop
  to `<ConfirmDialog`. This is the acceptance gate for the ConfirmDialog prop rename.

### Integration / e2e

- Playwright re-baseline: every route that renders one of these seven components — approximately `/software`, `/hosts`,
  `/services`, `/settings` (including danger-zone confirm flow), `/host-tags`, `/history`, `/profile` — each in both
  dark and light themes. The blast radius here is cross-route; the re-baseline set is large by design.
- Smoke test a destructive confirm flow (Revoke token on `/profile`, Delete rule on `/settings` notifications) —
  danger-variant confirm button visually confirmed, focus trap unchanged, Cancel returns to baseline without mutation.
- Smoke test BatchActionBar on `/software` (bulk merge candidate surface) or `/hosts` — primary + destructive action
  variants render, loading state surfaces the primitive spinner, `onaction(id)` dispatch reaches the expected handler.
- Delta enumeration per parent §9 (separated by size class):
  - Standard `size="md"` buttons (ConfirmDialog, BatchResultDialog, Cancel/Save modal footers): `h-[23px]`, label `9px`
    uppercase.
  - Standard `size="sm"` buttons (BatchActionBar actions, row-level actions, Toast Dismiss, addHook / remove-hook in
    AssignToHostModal): `h-[19px]`, label `8.5px` uppercase.
  - Pagination buttons: `h-8` (32px) via class override, label `text-[10px]` via class override — explicitly
    non-standard and documented here as the Pagination contract.
- Snapshot masking (required to stabilise the visual gate):
  - Mask in-flight spinner rotation on every `<Button loading>` site.
  - Mask transient toast banners raised inside the snapshot window.
  - Mask `ToastNotifications` progress bar `<span data-ui="toast- progress">` (source line 392) — it animates
    `transform: scaleX(...)` and would otherwise churn every snapshot.
  - Mask dynamic id / timestamp cells inside EditHostAssignmentModal rows.

## Rollout

Single PR titled "feat(frontend): migrate shared modals + dialogs to Button primitive (sub-spec #3k)".

Prereq: Button primitive (#2) merged; #2c merged (`variant="secondary"`

- base-Button `ariaLabel` for consumers that want it — #3k does not add any `ariaLabel`-requiring sites itself, but peer
  sub-specs do).

1. `ConfirmDialog.svelte` — add `confirmVariant` prop; remove `confirmClass` prop; migrate confirm + cancel buttons.
2. Consumer sweep — every `<ConfirmDialog>` call site migrated according to the Consumer audit rules above. This is the
   TypeScript-error-driven step; every caller must compile.
3. `BatchResultDialog.svelte` — migrate Close button.
4. `BatchActionBar.svelte` — extend `actions` prop type; migrate primary / More-menu / Deselect-all render paths.
5. `Pagination.svelte` — migrate Previous / Next / page-number buttons with class-override sizing preserved.
6. `ToastNotifications.svelte` — migrate line 379 Dismiss only. Leave line 386 CTA anchor untouched.
7. `AssignToHostModal.svelte` — migrate every enumerated button.
8. `EditHostAssignmentModal.svelte` — migrate every enumerated button.
9. Extend unit tests per plan.
10. Re-baseline Playwright snapshots across the listed routes.
11. Full frontend gate.

### Risk + rollback

Revert of one PR restores preset classes across the shared primitives, reverting their effect app-wide.
Highest-blast-radius sub-spec — mitigated by the broadest Playwright coverage (every authenticated route that renders
one of the seven components), the compile-time gate from the `ConfirmDialog` prop rename (no silent-miss consumer), and
focused unit tests on each primitive's variant matrix.

### Dependencies + ordering

- **Blocks on:** sub-spec #2 merged (base Button with `leadingIcon` + `trailingIcon` snippets); sub-spec #2c merged
  (`variant="secondary"`
  - `--bg-hover` token + base-Button `ariaLabel`).
- **Blocks:** #3k2 form-input migration inside modals (depends on #2b Input / Checkbox + #2d Textarea —
  `EditHostAssignmentModal` has 42 textarea sites).
- **Parallel-safe with:** sub-spec #3c–j, #4 (surface layer).
- **Scope-boundary note:** `AddSoftwareModal` + `SoftwareMergeWizard` are #3h's responsibility; the
  `<a href="/settings/global">` anchor inside `ToastNotifications.svelte` is #2b's responsibility. Both are documented
  above under Non-goals and Scope.
