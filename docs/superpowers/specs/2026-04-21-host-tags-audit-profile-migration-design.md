# Host-Tags + Audit-Logs + Profile Migration — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md` (§4.3 Buttons, §4.6 Loading State,
§4.7 Tables)

**Sub-spec #3j of the UI design-language rollout.** Depends on sub-spec #2 (Button primitive) merged and #2c merged
(`ariaLabel` prop, `--bg-hover` token, `loading → disabled` contract). Form-input sites (text inputs, checkboxes,
select, date-time, textarea, color picker) defer to a future #3j2 pass after sub-spec #2b primitives land.

## Overview

Migrate three lower-frequency admin routes:

- `/host-tags/+page.svelte` (537 lines) — tag CRUD list with per-row context-menu trigger, Create Tag header action,
  Create/Edit modal bodies with a color toggle, and error-state Retry.
- `/audit-logs/+page.svelte` (322 lines) — filter form with Apply/Clear buttons, a read-only results table, and
  error-state Retry. Scope toggle between tenant + system streams is rendered via the already- migrated shared
  `<TabStrip>` component (not a #3j site).
- `/profile/+page.svelte` (217 lines) — current user profile surface: static account fields (Name, Email), API token
  list with "New Token" launcher and per-row Revoke action, plus a "New API Token" modal whose footer swaps between
  create-form state and created-token state (Cancel + Create → Copy + Done).

Scope here is strictly the Skeleton preset buttons that live directly inside these three `+page.svelte` files. Shared
components (`ContextMenuShell`, `ContextMenuItem`, `BatchActionBar`, `ConfirmDialog`, `BatchResultDialog`, `ModalShell`,
`TabStrip`) are migrated end-to-end by #3k (or already through the `$lib/components/ui` surface in TabStrip's case) and
are out of scope — including the `confirmClass` `preset-filled-*-500` string inputs threaded into ConfirmDialog from
these pages.

## Design decisions

**Q1 — host-tags Create/Edit modal "color toggle" buttons.**

- Options:
  - (chosen) `<Button variant="secondary" size="sm">Auto</Button>` and
    `<Button variant="secondary" size="sm">Pick color</Button>` — the two buttons inside the Create/Edit modal that flip
    between "auto- assigned from palette" and a concrete hex color. They're reversible side actions on a form input, not
    the primary modal action. Children text stays identical to the current source; no icon.
  - `<Button variant="ghost">`. Rejected — ghost reads as navigation / tertiary; these flip form state and deserve
    secondary weight per #3c Q4 semantic.
- Reasoning: parent §4.3 secondary = reversible side action; both buttons toggle a form-input state and are reversible.

The `<input type="color">` itself + the hex text input (lines 466–471, 516–518) are form inputs — deferred to #3j2.

The row-level tag color indicator (host-tags lines 358–359) is a passive `<span>` swatch, not interactive. Not a button.
Not migrated. An earlier draft of this spec proposed a "color swatch launcher" Button here; that was stale against the
current source (no such interactive element exists) and has been removed.

**Q2 — host-tags row ellipsis (actions) trigger.**

- Options:
  - (chosen) Icon-only ghost trigger using `leadingIcon` snippet + `sr-only` children to satisfy `Button`'s required
    `children: Snippet` prop (see canonical form below). Same shape as #3h (host-detail) and #3i (services,
    system-services). The `leadingIcon` prop is a `Snippet` — it must be passed via
    `{#snippet leadingIcon()}…{/snippet}`, not as a component reference. `ariaLabel` supplies the accessible name per
    #2c's ariaLabel prop. The `sr-only` children satisfy the required `children: Snippet` (no `?`) without adding
    visible text. Preserves `e.stopPropagation()` and `e.currentTarget` (popover positioning source). The `EllipsisIcon`
    component is either reused from #3i or created as part of this PR (see Rollout step 1).
  - Keep the raw `&#8943;` unicode children. Rejected — theme tint fails on unicode glyphs (same rationale as #3i Q3).
  - Use a component reference `leadingIcon={EllipsisIcon}`. Rejected — `leadingIcon` is typed as `Snippet`, not a
    component constructor; this would be a compile error.
- Design decision — `children` is required: `Button.svelte` declares `children: Snippet` with no `?`, making it
  non-optional. Icon-only triggers must always include a `children` block. The canonical pattern is a
  `<span class="sr-only">` with a descriptive label; this doubles as the accessible fallback when `ariaLabel` is not
  read by all assistive technology stacks.
- Reasoning: fourth consumer (hosts, services, system-services, host-tags) of the icon-only ellipsis trigger; same
  primitive shape across all four.

Canonical ellipsis trigger form:

```svelte
<Button variant="ghost" size="sm" ariaLabel="Actions for {tag.name}" onclick={(e) => toggleMenu(tag.id, e.currentTarget)}>
  {#snippet leadingIcon()}<EllipsisIcon />{/snippet}
  <span class="sr-only">Actions for {tag.name}</span>
</Button>
```

The row-level Edit and Delete actions themselves are `<ContextMenuItem label="…" destructive?>` children inside
`<ContextMenuShell>` (host-tags lines 433, 436) — their variant / loading surface is owned by #3k. Not migrated here.

**Q3 — Error-state Retry (all three files).**

- Options:
  - (chosen) `<Button variant="primary" loading={isRetrying}>Retry </Button>` — same rationale as #3e Q2 / #3h Q1 / #3i
    Q4: Retry is the sole action in an error boundary, hence primary by context. Size remains default `md`. A new local
    `let isRetrying = $state(false)` per file wraps the existing loader invocation (`loadTags(currentPage)` /
    `load(currentPage)` / host-tags' / audit-logs' / profile's loader) with
    `try { ... } finally { isRetrying = false; }`.
  - Reuse an existing `submitting` / `loading` flag. Rejected — those flags span unrelated flows (batch, create,
    revoke); binding Retry to them would surface spurious spinners.
  - Note: profile has no error-state Retry button in the current source (DataTable's `emptyTitle`/`loading` props drive
    its error UI, not a Retry snippet). Profile therefore gets no `isRetrying` flag.
- Reasoning: Retry = primary per context; per-action loading flags prevent false spinners.

**Q4 — audit-logs Apply Filters / Clear Filters.**

- Options:
  - (chosen) Apply Filters → `<Button variant="primary" onclick={applyFilters}>Apply Filters</Button>`. Clear Filters →
    `<Button variant="secondary" onclick={clearFilters}>Clear Filters</Button>`. Neither is async at the click site
    (each triggers a fresh `load(1)` whose loading state surfaces in `DataTable`'s `loading` prop — no per-button
    `loading` binding). Default size `md`.
  - Wire `loading` on Apply. Rejected — the DataTable already renders the fetch spinner via its `loading` prop;
    double-signaling would split the user's attention between table chrome and button chrome.
- Reasoning: Apply = primary filter-commit; Clear = reversible reset. The table owns fetch-loading UI.

**Q5 — profile API token row Revoke.**

- Options:
  - (chosen)
    `<Button variant="danger" size="sm" onclick={() => (revokeConfirm = { id: token.id, name: token.name })}>Revoke </Button>`
    — launcher for the `<ConfirmDialog>` revoke confirmation. No `loading` here (click opens the modal instantly; async
    work happens inside the dialog). The `preset-tonal-error` source class maps to `variant="danger"` per parent §4.3 —
    revocation is destructive and irreversible from the actor's perspective.
  - `<Button variant="secondary">`. Rejected — Revoke is destructive; same semantic as Delete in other row surfaces
    (#3e, #3h, #3i).
- Reasoning: revocation is destructive + irreversible; danger variant.

**Q6 — profile "New Token" launcher + New API Token modal footer.**

- Options:
  - (chosen) New Token launcher → `<Button variant="primary" onclick={openCreateModal}>New Token</Button>` (preserve
    existing label — the current source label is "New Token", not "Generate"). Create state footer:
    `<Button variant="secondary" onclick={closeCreateModal}>Cancel</Button>` +
    `<Button variant="primary" loading={creating} disabled={!newTokenName.trim()} onclick={handleCreate}>Create</Button>`.
    Note: the Create submit in the New API Token modal is already migrated as of Wave 3; skip it. The `Button` import at
    line 10 already exists. Cancel (line 208) is the only remaining create-state site. The `{creating ? 'Creating...' :
    'Create'}` text-swap is already removed; the `disabled={submitting || !newTokenName.trim()}` collapse is already
    applied. Created state footer:
    `<Button variant="secondary" onclick={() => copyToken(createdToken!)}>Copy</Button>` +
    `<Button variant="primary" onclick={closeCreateModal}>Done</Button>`. Copy is a side-effect action (writes to
    clipboard); Done is the acknowledgement primary.
  - Copy as ghost with `leadingIcon={CopyIcon}`. Rejected — no `CopyIcon` exists in the codebase today; adding one is
    additive feature work outside a button-primitive migration. Copy as text-only secondary preserves the current
    affordance without sneaking in feature work.
  - Rename "New Token" to "Generate token". Rejected — relabel is feature work; spec is migration-only.
- Reasoning: primary reserved for the modal's acknowledge action (Done) and for the launcher; Copy is secondary because
  it is a side action adjacent to Done.

**Q7 — Shared component migration boundary.**

- Options:
  - (chosen) `<BatchActionBar>`, `<ContextMenuShell>`, `<ContextMenuItem>`, `<ConfirmDialog>`, `<BatchResultDialog>`,
    `<ModalShell>` all defer to #3k. `<TabStrip>` (audit-logs scope toggle) is already exported from
    `$lib/components/ui`; its internals are out of scope for this sub-spec. This sub-spec does not touch any of them.
    The `confirmClass` preset strings threaded into `<ConfirmDialog>` from host-tags (lines 417, 448) and profile
    (line 168) are likewise #3k's API surface.
  - Migrate any shared primitive touched by these pages. Rejected — #3k owns them end-to-end.
- Reasoning: scope discipline matching #3c–i precedent.

## Goals

1. Every **directly rendered** `<button>` in the three files renders through `<Button>`. Actions rendered via shared
   components (`<ContextMenuItem>`, `<BatchActionBar>`, `<ConfirmDialog>`, `<BatchResultDialog>`, `<TabStrip>`,
   `<ModalShell>`) remain untouched and migrate with their owning sub-specs (#3k or already-shared).
2. host-tags Create Tag header action adopts `variant="primary"`; row ellipsis trigger adopts icon-only ghost +
   `ariaLabel` + `{#snippet leadingIcon()}` + `sr-only` children (required — `children: Snippet` has no `?`);
   Create/Edit modal footers adopt secondary (Cancel) + primary + `loading` (Submit); Auto / Pick color
   modal-body toggles adopt `variant="secondary" size="sm"`.
3. audit-logs Apply Filters adopts `variant="primary"`; Clear Filters adopts `variant="secondary"`.
4. profile New Token launcher adopts `variant="primary"`; per-row Revoke adopts `variant="danger" size="sm"`; New API
   Token modal footer adopts secondary (Cancel / Copy) + primary (Create / Done) shape. Note: the Create submit in the
   New API Token modal is already migrated as of Wave 3; only the remaining unmigrated sites (New Token launcher, Revoke,
   Cancel, Copy, Done) are performed here.
5. Error-state Retry buttons (host-tags, audit-logs) adopt `variant="primary"` + new local `isRetrying` loading flag.

## Non-goals

- **Features that do not exist in the current source**: color swatch launcher button on host-tags rows, Export CSV on
  audit-logs, row-level view-details action on audit-logs, filter chip buttons on audit-logs, pagination custom buttons
  (both audit-logs and host-tags use shared `TableFooterBar`), Change password launcher on profile, profile "Generate
  new token" relabel. None of these exist in the current source; an earlier draft listed them and those claims have been
  removed.
- Form-input migration (text inputs, select, checkbox, date-time- local, color input, textarea on host-tags / audit-logs
  / profile modal bodies and filter form) — deferred to #3j2 after #2b + #2d primitives land.
- Shared component internals — migrated by #3k (or already through `$lib/components/ui` for `<TabStrip>`).
- Account / avatar / profile-section refactor — outside Button scope.
- API token backend, audit-log backend, tag backend — out of scope.

## Scope

Files migrated:

- `frontend/src/routes/host-tags/+page.svelte`
  - Header action: Create Tag (line 295) → primary (Q-implicit).
  - Row ellipsis trigger (lines 374–383) → ghost sm + ariaLabel + icon (Q2).
  - Error snippet Retry (line 390) → primary + new `isRetrying` flag (Q3).
  - Create modal body: Auto toggle (lines 472–476), Pick color toggle (lines 479–483) → secondary sm (Q1).
  - Create modal footer: Cancel (line 494) → secondary; Create submit (lines 495–501) → primary +
    `loading={submitting}` + `disabled={!createForm.name.trim()}` + static `Create` children (drop `Creating…` text-swap
    per #3c Q4).
  - Edit modal footer: Cancel (line 526) → secondary; Save submit (lines 527–533) → primary + `loading={submitting}` +
    `disabled={!editTag?.name.trim()}` + static `Save` children (drop `Saving…` text-swap).

- `frontend/src/routes/audit-logs/+page.svelte`
  - Filter actions snippet: Apply Filters (line 220) → primary (Q4); Clear Filters (line 221) → secondary (Q4).
  - Error snippet Retry (line 311) → primary + new `isRetrying` flag (Q3).

- `frontend/src/routes/profile/+page.svelte`
  - Section action: New Token launcher (line 116) → primary (Q6).
  - Row action: Revoke (lines 148–153) → danger sm (Q5).
  - New API Token modal footer (created state): Copy (line 205) → secondary; Done (line 206) → primary (Q6).
  - New API Token modal footer (create state): Cancel (line 208) → secondary (Q6).
  - **Already migrated (skip):** Create submit (line 209) — as of Wave 3, this site already reads
    `<Button variant="primary" onclick={handleCreate} disabled={!newTokenName.trim()} loading={creating}>Create</Button>`
    and the `Button` import at line 10 already exists. The implementation must NOT re-migrate this site.

Explicitly not migrated here (listed to prevent spec bleed at implementation time):

- `frontend/src/lib/components/ContextMenuShell.svelte`, `ContextMenuItem.svelte`, `BatchActionBar.svelte`,
  `ConfirmDialog.svelte`, `BatchResultDialog.svelte`, `ModalShell.svelte` — all owned by #3k.
- `frontend/src/lib/components/ui/TabStrip.svelte` — already exported through the shared UI surface.
- The `confirmClass: 'preset-filled-error-500'` inputs fed into `<ConfirmDialog>` from host-tags (lines 417, 448) and
  profile (line 168) — ConfirmDialog's external API and its internal confirm button are migrated together by #3k.

## Migration pattern

Per-attribute translation:

- `preset-filled-primary-500` (default button intent) → `<Button variant="primary">`.
- `preset-tonal` on row ellipsis trigger → `<Button variant="ghost" size="sm" ariaLabel="…">` with
  `{#snippet leadingIcon()}<EllipsisIcon />{/snippet}` and `<span class="sr-only">…</span>` children.
  `leadingIcon` is a `Snippet` — use `{#snippet leadingIcon()}…{/snippet}`, not a component reference.
  The `sr-only` children satisfy `children: Snippet` which is required with no `?`.
  See the canonical form under Q2 above.
- `preset-tonal-surface` on Cancel + Copy + in-modal side toggles → `<Button variant="secondary">` (with `size="sm"` on
  the in-modal color toggles per Q1).
- `preset-tonal-error` on row Revoke → `<Button variant="danger" size="sm">`.
- `preset-filled-primary-500` on error Retry → `<Button variant="primary" loading={isRetrying}>Retry</Button>`.

Async wiring:

- Retry (host-tags + audit-logs): add per file `let isRetrying = $state(false)`. Snippet handler becomes
  `async () => { isRetrying = true; try { await loadTags(currentPage); } finally { isRetrying = false; } }` (parallel
  form in audit-logs with `load(currentPage)`). `loadTags` / `load` themselves are unchanged.
- host-tags Create/Edit modal submits: reuse existing `submitting` flag. Because `loading=true` sets `disabled=true`
  internally per #2c, the `disabled={submitting || !…}` expressions collapse to `disabled={!…}`. Drop the `Creating…` /
  `Saving…` text-swap expressions; children render static `Create` / `Save`.
- profile Create submit: already migrated as of Wave 3 — `loading={creating}`, `disabled={!newTokenName.trim()}`,
  static `Create` children are already in place. No changes needed to this site.
- Row ellipsis trigger: `frontend/src/lib/components/icons/` does not exist in the codebase. The icon must be created
  unconditionally as part of this PR (see Rollout step 1). If #3i has already landed and created
  `frontend/src/lib/components/icons/EllipsisIcon.svelte`, reuse it; otherwise create it here as a static SVG with no
  props. Never use `leadingIcon={EllipsisIcon}` (component reference) — `leadingIcon` is typed as `Snippet` and must be
  passed as `{#snippet leadingIcon()}<EllipsisIcon />{/snippet}`.
- audit-logs Apply / Clear Filters: neither needs async wiring — `applyFilters()` and `clearFilters()` both delegate to
  `load(1)`, whose loading state surfaces via `DataTable`'s `loading` prop, not via button chrome.

## Data flow

Template-level only. No runtime behaviour changes. Existing tag CRUD, token CRUD, filter, clear, retry,
copy-to-clipboard, and modal open/close pipelines all pass through unchanged.

New local state introduced: `isRetrying` flag in host-tags and audit-logs. Nothing in profile (no Retry snippet in that
file).

## Error handling

- Button primitive's discriminated union catches invalid prop combos at compile time (polymorphic `href`/`onclick` guard
  defaults to the `onclick` branch in every site here).
- Retry / save / revoke / create error propagation: existing toast pipelines unchanged. Button renders loading state
  only; `finally` blocks already flip the upstream flag back to false.
- The created-token modal's Copy button catches clipboard failures via existing `copyToken(...)` try/catch (see profile
  source line 85–92); Button primitive does not alter that path.

## Testing

### Unit tests

Extend `host-tags/+page.test.ts`, `audit-logs/+page.test.ts`, `profile/+page.test.ts`:

- **host-tags**:
  - Create Tag header action renders `variant="primary"` (no `size` → default `md`).
  - Row ellipsis trigger renders `variant="ghost" size="sm"`, `aria-label="Actions for <name>"` (verify for a tag
    whose name contains a space); `leadingIcon` snippet renders `EllipsisIcon`; children is a `sr-only` span (required
    — `children: Snippet` has no `?`); `onclick` preserves `e.stopPropagation()` + `e.currentTarget` positioning
    (mount row, click ellipsis, assert `toggleMenu` called with `(tag.id, HTMLElement)`).
  - Error Retry renders `variant="primary"`; on click, `loading=true` for the duration of the awaited `loadTags`; flips
    back to `false` after both resolution and rejection (test both); `aria-busy="true"` during the loading window.
  - Auto toggle (color-present state) renders `variant="secondary" size="sm"`; Pick color toggle (color-empty state)
    same.
  - Create modal footer: Cancel renders `variant="secondary"`; Create submit renders `variant="primary"` +
    `loading={submitting}`; `disabled` follows `!createForm.name.trim()` alone; children stay `Create` across the submit
    window (regression guard that `Creating...` text-swap is gone); assert `loading=true` also flips `aria-busy="true"`.
  - Edit modal footer: mirrored assertions for Save (children stay `Save`).
  - Out-of-scope regression guard: Edit / Delete `ContextMenuItem` entries are _not_ wrapped in `<Button>` and remain
    deferred to #3k (mount a row, open menu, assert the two items render as `ContextMenuItem`).

- **audit-logs**:
  - Apply Filters renders `variant="primary"`; click triggers `load(1)` (mock the API).
  - Clear Filters renders `variant="secondary"`; click triggers `load(1)` after zeroing the filter state (assert
    `filterActorType`, `filterActionType`, etc. are empty).
  - Error Retry renders `variant="primary"` with loading wiring (same pattern as host-tags Retry).
  - Out-of-scope regression guard: `<TabStrip>` for tenant / system scope is not wrapped in `<Button>` and is not
    asserted on here (it's shared UI, unchanged).

- **profile**:
  - New Token launcher renders `variant="primary"` with children `New Token`.
  - Row Revoke renders `variant="danger" size="sm"`.
  - New API Token modal (create state): Cancel renders `variant="secondary"`. Create submit is already migrated as of
    Wave 3 — assert it renders `variant="primary"` + `loading={creating}` + `disabled` follows `!newTokenName.trim()`
    alone + children stay `Create` across the submit window (regression guard only; no new migration work here).
  - New API Token modal (created state): Copy renders `variant="secondary"` with children `Copy`; Done renders
    `variant="primary"` with children `Done`; clicking Copy invokes `navigator.clipboard.writeText` with the created
    token (mock clipboard) and surfaces the existing success toast.
  - Out-of-scope regression guard: `<ConfirmDialog>` revoke confirmation is not wrapped in `<Button>` and is not
    asserted on here (owned by #3k).

### Integration / e2e

- Playwright re-baseline `/host-tags`, `/audit-logs`, `/profile` in dark + light themes. Induce the error state on
  host-tags + audit-logs (mock 500 on the list API) and capture the Retry button. Open the host-tags Create modal, then
  the Edit modal, then the profile New Token modal (create state and created state); capture each footer state.
- Delta enumeration per parent §9 (split by size class):
  - `size="sm"` buttons (row ellipsis trigger, row Revoke, in-modal Auto / Pick color toggles): `h-[19px]`, label
    `8.5px` uppercase (label absent on icon-only ellipsis); `danger` variant renders error gradient on Revoke.
  - `size="md"` buttons (Create Tag launcher, Apply / Clear Filters, Retry, modal Cancel / Submit / Copy / Done):
    `h-[23px]`, label `9px` uppercase; `primary` renders default gradient; `secondary` renders #2c secondary contract.
- Snapshot masking per parent §3 (total masked area under 15%):
  - Mask in-flight spinner rotation on every `<Button loading>` site (Retry during fetch window, modal submit during
    save / create window).
  - Mask `formatDate(...)` outputs (created_at, occurred_at, revoked_at) in all three file tables — dynamic strings
    shift between snapshots.
  - Mask the generated token string inside the profile created- token `<pre>` (profile line 185) — token contents are
    deliberately randomised per run.
  - Mask transient toast banners raised by save / revoke / copy flows.
  - Mask batch-selection count text in `BatchActionBar` (`{selectedCount} selected`) when rendered on host-tags.
- Smoke tests:
  - host-tags Retry: induce `loadTags` failure, assert Retry button renders with `aria-busy="false"`, click, assert
    `aria-busy="true"` during refetch, clear after resolution.
  - audit-logs Apply / Clear round-trip: enter a filter, click Apply (assert `load(1)` called with filter), click Clear
    (assert filter state resets), tab visual delta captured.
  - profile Revoke flow: click Revoke row button → ConfirmDialog opens → ConfirmDialog's own confirm is #3k territory,
    assert only that the launcher triggers the dialog; do not assert on dialog chrome.
  - profile Create flow: open New Token, type a name, submit, assert Button `aria-busy="true"` during request (Create
    submit was already migrated in Wave 3 — this is a regression guard), assert children stays `Create`, assert
    created-token state renders with Copy + Done.

### Out-of-scope in tests

- `ContextMenuItem` variant / loading assertions — #3k.
- `BatchActionBar` launcher / cancel assertions — #3k.
- `ConfirmDialog` confirm / cancel assertions — #3k (including the `confirmClass` preset-string input transition to a
  tone enum).
- `<TabStrip>` variant / chip-fragment assertions — already-shared UI, out of scope for this sub-spec.
- `<input type="color">`, `<input type="text">`, `<textarea>`, `<select>`, `<input type="checkbox">`,
  `<input type="datetime-local">` assertions on modal bodies and the audit-logs filter form — all form inputs; #3j2
  territory after #2b + #2d primitives land.

## Rollout

Single PR titled "feat(frontend): migrate host-tags + audit-logs + profile to Button primitive (sub-spec #3j)".

1. Create `frontend/src/lib/components/icons/EllipsisIcon.svelte` as a static SVG with no props. The
   `frontend/src/lib/components/icons/` directory does not exist in the current codebase and must be created
   unconditionally. If sub-spec #3i has already landed and created this file, reuse it without recreating.
2. Migrate `host-tags/+page.svelte` — Create Tag launcher, row ellipsis trigger, error Retry (new `isRetrying` flag),
   Create/Edit modal footers (Cancel + Submit with loading + collapsed disabled + text-swap removal), Auto / Pick color
   in-modal toggles.
3. Migrate `audit-logs/+page.svelte` — Apply Filters, Clear Filters, error Retry (new `isRetrying` flag).
4. Migrate `profile/+page.svelte` — New Token launcher, row Revoke, New API Token modal footer remaining sites: Cancel,
   Copy, Done. The Create submit is already migrated as of Wave 3; skip it.
5. Extend unit tests per plan.
6. Re-baseline Playwright snapshots for all three routes in both themes.
7. Full frontend gate.

### Risk + rollback

Revert of one PR restores preset classes across three lower-traffic routes. Lowest aggregate blast radius of the #3c–j
sub-specs; the highest-value regression guard is the profile Revoke flow (irreversible side effect), mitigated by unit
test on the danger-variant + ConfirmDialog launcher linkage. host-tags row-action context menu is intentionally
untouched — #3k owns it — so the critical destructive surface (Delete) has zero visual or behavioural delta from this
PR.

### Dependencies + ordering

- **Blocks on:** sub-spec #2 merged; sub-spec #2c merged (`ariaLabel` prop, `loading→disabled` contract, secondary
  contract); sub-spec #3b merged (layout-chrome baseline).
- **Blocks:** #3j2 form-input migration (depends on #2b + #2d Textarea primitive — host-tags has 3 form-input sites;
  audit-logs has a 7-field filter form; profile has 1 input in the New Token modal).
- **Coordinates with #3k:** this sub-spec intentionally leaves six shared components and the `confirmClass`
  preset-string API unmigrated; #3k absorbs them. Landing order is independent — a mixed-style call site during the
  interim (migrated launcher button + preset-classed ConfirmDialog, or preset launcher + migrated dialog) is expected
  and not a regression.
- **Parallel-safe with:** sub-spec #3c–i, #3k, #4.
