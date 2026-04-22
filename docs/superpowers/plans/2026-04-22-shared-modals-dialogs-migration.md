# Shared Modals + Dialogs Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate all raw Skeleton `preset-*`/`btn` button elements in the seven shared component files to the
`<Button>` primitive; rename `ConfirmDialog`'s `confirmClass` prop to `confirmVariant`; migrate `Pagination.svelte`
button elements.

**Architecture:** Seven component files plus a consumer audit for `confirmClass` removal; ConfirmDialog prop rename
requires auditing all call sites; Pagination migration affects every page that uses it.

**Tech Stack:** Svelte 5, SvelteKit, TypeScript, Vitest, Playwright

---

## Dependencies

**Blocks on:**

- Sub-spec #2 merged — `Button` primitive exported from `frontend/src/lib/components/Button.svelte` with
  `leadingIcon` / `trailingIcon` snippet props and `loading`.
- Sub-spec #2c merged — `variant="secondary"` available on `Button`; `--bg-hover` token in CSS contract;
  base-Button `ariaLabel` prop.

**Blocks:** #3k2 form-input migration inside modals (depends on #2b Input / Checkbox + #2d Textarea).

**Parallel-safe with:** sub-specs #3c–j, #4 (surface layer).

---

## Migration rules (quick reference)

| Legacy class | Button variant |
| --- | --- |
| `preset-filled-primary-500` | `primary` |
| `preset-tonal-primary` | `primary` |
| `preset-tonal-surface` | `secondary` |
| `preset-tonal` (row neutral side action) | `secondary` |
| `preset-tonal` (pagination) | `ghost` — Q3 governs all Pagination buttons |
| `preset-tonal-error` / `preset-filled-error-500` | `danger` |

**Snippet icon syntax:** Always `{#snippet leadingIcon()}<svg …/>{/snippet}` passed via `leadingIcon={snippetRef}`.
Never `leadingIcon={Component}`.

**Loading / text-swap contract:** Bind submitting/saving flags to `loading` prop; remove
`{flag ? 'Saving…' : 'Save'}` ternaries; children are static label text. The primitive handles spinner +
visual disabled state.

**`confirmDisabled` passthrough:** The prop on `ConfirmDialog` is named `confirmDisabled` (not `disabled`).
It passes through to Button's `disabled` prop unchanged.

---

## Button site inventory

Verify exact line numbers by reading each file before editing — the numbers below are approximate.

### `ConfirmDialog.svelte` (frontend/src/lib/components/ConfirmDialog.svelte)

- Line 60: Cancel — **change** `variant="ghost"` to `variant="secondary"` (already a `<Button>`, variant update
  only).
- Lines 61–63: Confirm — raw `<button class="btn {confirmClass}" disabled={confirmDisabled} onclick={onconfirm}>`
  migrates to `<Button variant={confirmVariant} disabled={confirmDisabled} onclick={onconfirm}>{confirmLabel}</Button>`.
- Props: remove `confirmClass?: string` (default `'preset-filled-error-500'`); add
  `confirmVariant?: 'primary' | 'danger' = 'danger'`. Remove `resolveConfirmTone` helper and the `confirmTone`
  derived (both only exist to map `confirmClass` to `StatusBadge` tone; replace with a direct variant map).

**StatusBadge tone mapping after prop rename:**

```svelte
const confirmTone = $derived(confirmVariant === 'danger' ? 'danger' : 'info');
```

### `BatchResultDialog.svelte` (frontend/src/lib/components/BatchResultDialog.svelte)

- Line 45: Close — `<button class="btn preset-filled-primary-500" onclick={onclose}>Close</button>`
  migrates to `<Button variant="primary" onclick={onclose}>Close</Button>`. Add `Button` import.

### `BatchActionBar.svelte` (frontend/src/lib/components/BatchActionBar.svelte)

- Line ~126: primary actions loop — raw `<button class="btn btn-sm preset-filled-primary-500">` migrates to
  `<Button variant={a.variant ?? (a.destructive ? 'danger' : 'primary')} size="sm" loading={a.loading}
  onclick={() => onaction(a.id)}>{a.label}</Button>`.
- Line ~132–133: More-menu trigger — `<button class="btn btn-sm preset-tonal-surface">` migrates to
  `<Button variant="secondary" size="sm">` with existing `aria-label`, `aria-haspopup`, `aria-expanded` preserved.
- Line ~170: Deselect all — `<button class="btn btn-sm preset-tonal-surface" onclick={oncancel}>Deselect all</button>`
  migrates to `<Button variant="secondary" size="sm" onclick={oncancel}>Deselect all</Button>`.
- Prop type extension on `actions` (additive, no consumer changes required):

```ts
actions: {
  id: string;
  label: string;
  destructive?: boolean;
  variant?: 'primary' | 'secondary' | 'danger';
  loading?: boolean;
}[]
```

### `Pagination.svelte` (frontend/src/lib/components/Pagination.svelte)

All three button kinds use `variant="ghost" size="sm"` with a `class` override for the 32px height contract.
No icon library exists in this project; use inline SVG snippets for the chevron affordances.

Previous button (line ~52):

```svelte
<Button variant="ghost" size="sm" class="h-8 min-h-8 px-3 text-[10px]"
  leadingIcon={prevIcon} disabled={currentPage <= 1}
  onclick={() => onPageChange(currentPage - 1)}>Previous</Button>
```

Page-number buttons (lines ~63–72) — active page gets accent/bg-hover classes, inactive gets neither:

```svelte
<Button
  variant="ghost"
  size="sm"
  class={['h-8 min-h-8 min-w-8 px-2.5 text-[10px]',
    p === currentPage ? 'text-[var(--accent)] bg-[var(--bg-hover)]' : ''].join(' ').trim()}
  aria-current={p === currentPage ? 'page' : undefined}
  onclick={() => onPageChange(p)}
>{p}</Button>
```

Next button (line ~74):

```svelte
<Button variant="ghost" size="sm" class="h-8 min-h-8 px-3 text-[10px]"
  trailingIcon={nextIcon} disabled={currentPage >= totalPages}
  onclick={() => onPageChange(currentPage + 1)}>Next</Button>
```

### `ToastNotifications.svelte` (frontend/src/lib/components/ToastNotifications.svelte)

- Line ~379: Dismiss — `<button class="btn btn-sm preset-tonal-surface" onclick={() => dismissToast(item)}>Dismiss</button>`
  migrates to `<Button variant="ghost" size="sm" onclick={() => dismissToast(item)}>Dismiss</Button>`.
- Line ~386: `<a href="/settings/global" class="btn btn-sm preset-tonal">Go to Global Settings</a>` —
  **leave entirely untouched** (belongs to sub-spec #2b Link). Do not wrap in `<Button>` or change its classes.

### `AssignToHostModal.svelte` (frontend/src/lib/components/AssignToHostModal.svelte)

Six button sites:

- Line ~371 (pre_update_hook Add): `<button type="button" class="btn btn-sm preset-tonal-surface text-xs">`
  migrates to `<Button variant="secondary" size="sm" type="button">`. Drop `text-xs`.
- Line ~498 (post_update_hook Add): same pattern as ~371.
- Line ~398 (pre hook remove): `<button type="button" class="btn btn-sm preset-tonal-error text-xs shrink-0">`
  migrates to `<Button variant="danger" size="sm" class="shrink-0" type="button">`. Keep `shrink-0`, drop `text-xs`.
- Line ~525 (post hook remove): same pattern as ~398.
- Line ~545 (Cancel footer): `<button class="btn preset-tonal-surface" onclick={onclose}>Cancel</button>`
  migrates to `<Button variant="secondary" onclick={onclose}>Cancel</Button>`.
- Line ~546 (Save footer): `<button class="btn preset-filled-primary-500" disabled={submitting || loading || !!loadError}>`
  migrates to `<Button variant="primary" loading={submitting} disabled={loading || !!loadError} onclick={submit}>Save</Button>`.
  Remove text-swap ternary; static children `Save`.

### `EditHostAssignmentModal.svelte` (frontend/src/lib/components/EditHostAssignmentModal.svelte)

Twelve button sites. Re-grep with `grep -n "btn preset" frontend/src/lib/components/EditHostAssignmentModal.svelte`
before editing — the numbers below are approximate against the ~1365-line source.

**Eight JSON view-mode toggle buttons** (all `btn btn-sm preset-tonal text-xs`):

| Approx line | Children text | Context |
| --- | --- | --- |
| ~817 | `Edit as JSON` | standard role, form→JSON |
| ~841 | `Back to Form` | standard role, JSON→form |
| ~943 | `Advanced: Edit as JSON` | standard role advanced toggle |
| ~969 | `Back to Form` | standard role advanced, JSON→form |
| ~1137 | `Edit as JSON` | hook entry, form→JSON |
| ~1163 | `Back to Form` | hook entry, JSON→form |
| ~1270 | `Advanced: Edit as JSON` | hook entry advanced |
| ~1298 | `Back to Form` | hook entry advanced, JSON→form |

Each migrates to `<Button variant="secondary" size="sm" type="button">children text</Button>`. Drop `text-xs`.
Preserve any `shrink-0` or other layout classes via the `class` prop. Existing `onclick` handlers unchanged.

**Two row-action buttons:**

- Line ~1015 (`btn btn-sm preset-tonal-primary text-xs shrink-0`, `+ Add` hook-row primary): migrates to
  `<Button variant="primary" size="sm" class="shrink-0" type="button" onclick={() => addHook(hookRole)}>+ Add</Button>`.
- Line ~1036 (`btn btn-sm preset-tonal-error text-xs`, `Remove` hook-entry destructive): migrates to
  `<Button variant="danger" size="sm" type="button" onclick={() => requestHookRemoval(hookRole, entry.localKey)}>Remove</Button>`.

**Two footer buttons:**

- Line ~1346 (Cancel): `<button class="btn preset-tonal-surface" onclick={onclose}>Cancel</button>` migrates to
  `<Button variant="secondary" onclick={onclose}>Cancel</Button>`.
- Line ~1347 (Save Changes): `<button class="btn preset-filled-primary-500" onclick={save} disabled={submitting || loading || !!loadError}>`
  migrates to `<Button variant="primary" loading={submitting} disabled={loading || !!loadError} onclick={save}>Save Changes</Button>`.
  Static children `Save Changes` — NOT `Save`. Remove the `Saving…` text-swap ternary.

**Inline ConfirmDialog at line ~1354:** remove `confirmClass="preset-filled-error-500"` prop — the new default
`confirmVariant='danger'` is identical. Handle as part of Task 2 (consumer audit), applied here.

---

## Consumer audit — `confirmClass` removal

Twenty-one `confirmClass=` call sites exist across the codebase. TypeScript errors after Task 1 serve as the
acceptance gate. Mapping rules:

| Current `confirmClass` value | Action |
| --- | --- |
| `"preset-filled-error-500"` | Remove the prop (default `confirmVariant='danger'` is identical) |
| `"preset-filled-success-500"` | Replace with `confirmVariant="primary"` |
| `"preset-filled-warning-500"` | Replace with `confirmVariant="primary"` |
| ternary `success-500` or `error-500` | Bind ternary result to `confirmVariant` using `'primary'` and `'danger'` |
| `{labels.btnClass}` dynamic | Migrate `confirmLabels` object to use `variant` field; bind to `confirmVariant` |

**Note on `success-500` / `warning-500`:** The Button primitive has no `success` or `warning` variant.
`primary` is the nearest non-destructive positive-action variant. Flag this substitution in the PR description
for product review.

**Files requiring consumer updates:**

- `frontend/src/routes/services/+page.svelte` — two sites: batch confirm ternary + dynamic `labels.btnClass`
- `frontend/src/routes/system-services/+page.svelte` — two sites: same patterns as services
- `frontend/src/routes/software/+page.svelte` — two sites: batch confirm ternary + one error preset
- `frontend/src/routes/hosts/[id]/+page.svelte` — one site: error preset
- `frontend/src/routes/hosts/+page.svelte` — one site: error preset
- `frontend/src/routes/profile/+page.svelte` — one site: error preset
- `frontend/src/routes/software/IgnoreRulesTab.svelte` — two sites: both error preset
- `frontend/src/routes/software/[id]/+page.svelte` — two sites: both error preset
- `frontend/src/routes/host-tags/+page.svelte` — two sites: both error preset
- `frontend/src/routes/settings/GlobalSettingsTab.svelte` — one site: error preset
- `frontend/src/routes/settings/PluginConfigsTab.svelte` — four sites: all error preset
- `frontend/src/lib/components/EditHostAssignmentModal.svelte` — one site: error preset (migrated in Task 8)
- `frontend/src/lib/components/SoftwareMergeWizard.svelte` — one site: error preset (**NOTE:** SoftwareMergeWizard
  is owned by sub-spec #3h; remove `confirmClass` from the `<ConfirmDialog>` call only — do NOT migrate any
  other buttons in this file)

---

## Task 1: Migrate `ConfirmDialog.svelte`

**Files:**

- Modify: `frontend/src/lib/components/ConfirmDialog.svelte`

Read the file before editing to confirm exact line numbers. The file is 66 lines.

- [ ] **Step 1: Replace `confirmClass` prop with `confirmVariant` prop**

In the `$props()` destructure, change the default and type:

Before:

```svelte
confirmClass = 'preset-filled-error-500',
...
confirmClass?: string;
```

After:

```svelte
confirmVariant = 'danger' as 'primary' | 'danger',
...
confirmVariant?: 'primary' | 'danger';
```

- [ ] **Step 2: Replace `resolveConfirmTone` helper and `confirmTone` derived**

Remove the `resolveConfirmTone` function and the `const confirmTone = $derived(resolveConfirmTone(confirmClass))`
line. Add:

```ts
const confirmTone = $derived(confirmVariant === 'danger' ? 'danger' : 'info');
```

(Type is `StatusBadgeTone` — the existing import covers this.)

- [ ] **Step 3: Update the Cancel button variant**

Change:

```svelte
<Button variant="ghost" onclick={oncancel}>Cancel</Button>
```

To:

```svelte
<Button variant="secondary" onclick={oncancel}>Cancel</Button>
```

- [ ] **Step 4: Migrate the raw Confirm button**

Change (lines 61–63):

```svelte
<button class="btn {confirmClass}" disabled={confirmDisabled} onclick={onconfirm}>
  {confirmLabel}
</button>
```

To:

```svelte
<Button variant={confirmVariant} disabled={confirmDisabled} onclick={onconfirm}>
  {confirmLabel}
</Button>
```

- [ ] **Step 5: Verify TypeScript compilation**

```bash
cd frontend && npm run check 2>&1 | grep -i 'ConfirmDialog'
```

Expected: TypeScript errors at every `confirmClass=` call site (these are the acceptance-gate errors to fix in
Task 2). No errors on the component file itself.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/lib/components/ConfirmDialog.svelte
git commit -m "refactor(confirm-dialog): replace confirmClass with confirmVariant prop, migrate buttons to Button primitive (#3k)"
```

---

## Task 2: Consumer audit — remove all `confirmClass=` props

**Files:** All 13 files identified in the consumer audit section above.

Read each file before editing. The TypeScript errors from Task 1 Step 5 identify the exact locations.

- [ ] **Step 1: Migrate all `preset-filled-error-500` sites (simple removal)**

For each of these files, find every `confirmClass="preset-filled-error-500"` and delete that prop line:

- `frontend/src/routes/hosts/[id]/+page.svelte`
- `frontend/src/routes/hosts/+page.svelte`
- `frontend/src/routes/profile/+page.svelte`
- `frontend/src/routes/software/IgnoreRulesTab.svelte` (both sites)
- `frontend/src/routes/software/[id]/+page.svelte` (both sites)
- `frontend/src/routes/host-tags/+page.svelte` (both sites)
- `frontend/src/routes/settings/GlobalSettingsTab.svelte`
- `frontend/src/routes/settings/PluginConfigsTab.svelte` (all four sites)
- `frontend/src/lib/components/SoftwareMergeWizard.svelte` (remove `confirmClass` prop only — do NOT migrate any
  other buttons in this file; those are #3h scope)

- [ ] **Step 2: Migrate dynamic ternary sites**

`frontend/src/routes/services/+page.svelte` — batch confirm site (line ~568):

Before:

```svelte
confirmClass={batchConfirmAction === 'approve' ? 'preset-filled-success-500' : 'preset-filled-error-500'}
```

After:

```svelte
confirmVariant={batchConfirmAction === 'approve' ? 'primary' : 'danger'}
```

`frontend/src/routes/system-services/+page.svelte` — batch confirm site (line ~552): same pattern as services.

`frontend/src/routes/software/+page.svelte` — batch confirm site (line ~1292):

Before:

```svelte
confirmClass={batchConfirmAction === 'update-all' || batchConfirmAction === 'feature' ||
  batchConfirmAction === 'unfeature' ? 'preset-filled-warning-500' : 'preset-filled-error-500'}
```

After:

```svelte
confirmVariant={batchConfirmAction === 'update-all' || batchConfirmAction === 'feature' ||
  batchConfirmAction === 'unfeature' ? 'primary' : 'danger'}
```

Also remove the second `confirmClass="preset-filled-error-500"` site in this file (line ~1356) via Step 1.

- [ ] **Step 3: Migrate `confirmLabels`-driven dynamic sites**

`frontend/src/routes/services/+page.svelte` — single confirm site (line ~625). The `confirmLabels` object at
line ~394 currently uses `btnClass`. Migrate it to `variant`:

Before:

```ts
const confirmLabels = {
  approve: { title: 'Approve Service', verb: 'approve', btnClass: 'preset-filled-success-500' },
  reject:  { title: 'Reject Service',  verb: 'reject',  btnClass: 'preset-filled-error-500' },
  delete:  { title: 'Delete Service',  verb: '…',       btnClass: 'preset-filled-error-500' }
} as const;
```

After:

```ts
const confirmLabels = {
  approve: { title: 'Approve Service', verb: 'approve', variant: 'primary' as const },
  reject:  { title: 'Reject Service',  verb: 'reject',  variant: 'danger'  as const },
  delete:  { title: 'Delete Service',  verb: '…',       variant: 'danger'  as const }
} as const;
```

Then change the call site from `confirmClass={labels.btnClass}` to `confirmVariant={labels.variant}`.

`frontend/src/routes/system-services/+page.svelte` — single confirm site (line ~616): same pattern as services.
Migrate `confirmLabels` similarly, changing `btnClass` to `variant`.

- [ ] **Step 4: Verify TypeScript compilation — full gate**

```bash
cd frontend && npm run check 2>&1 | grep -i 'confirmClass\|confirmVariant'
```

Expected: zero errors.

- [ ] **Step 5: Verify no remaining `confirmClass=` usages**

```bash
cd frontend && grep -r "confirmClass=" src/ --include="*.svelte" --include="*.ts"
```

Expected: no output (zero matches).

- [ ] **Step 6: Commit**

```bash
git add \
  frontend/src/routes/services/+page.svelte \
  frontend/src/routes/system-services/+page.svelte \
  frontend/src/routes/software/+page.svelte \
  frontend/src/routes/software/IgnoreRulesTab.svelte \
  "frontend/src/routes/software/[id]/+page.svelte" \
  "frontend/src/routes/hosts/[id]/+page.svelte" \
  frontend/src/routes/hosts/+page.svelte \
  frontend/src/routes/profile/+page.svelte \
  frontend/src/routes/host-tags/+page.svelte \
  frontend/src/routes/settings/GlobalSettingsTab.svelte \
  frontend/src/routes/settings/PluginConfigsTab.svelte \
  frontend/src/lib/components/SoftwareMergeWizard.svelte
git commit -m "refactor(confirm-dialog): migrate all confirmClass= call sites to confirmVariant= (#3k)"
```

---

## Task 3: Migrate `BatchResultDialog.svelte`

**Files:**

- Modify: `frontend/src/lib/components/BatchResultDialog.svelte`

Read the file before editing. It is 47 lines.

- [ ] **Step 1: Add `Button` import**

In the `<script lang="ts">` block, add after the existing `Modal` import:

```ts
import Button from './Button.svelte';
```

- [ ] **Step 2: Migrate the Close button**

Before (line ~45):

```svelte
<button class="btn preset-filled-primary-500" onclick={onclose}>Close</button>
```

After:

```svelte
<Button variant="primary" onclick={onclose}>Close</Button>
```

- [ ] **Step 3: Verify**

```bash
cd frontend && npm run check 2>&1 | grep -i 'BatchResultDialog'
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/lib/components/BatchResultDialog.svelte
git commit -m "refactor(batch-result-dialog): migrate Close button to Button primitive (#3k)"
```

---

## Task 4: Migrate `BatchActionBar.svelte`

**Files:**

- Modify: `frontend/src/lib/components/BatchActionBar.svelte`

Read the file before editing. It is 173 lines.

- [ ] **Step 1: Add `Button` import**

After the existing `ContextMenuItem` import:

```ts
import Button from './Button.svelte';
```

- [ ] **Step 2: Extend `actions` prop type**

Change the type annotation in the `$props()` destructure:

Before:

```ts
actions: { id: string; label: string; destructive?: boolean }[];
```

After:

```ts
actions: { id: string; label: string; destructive?: boolean; variant?: 'primary' | 'secondary' | 'danger'; loading?: boolean }[];
```

- [ ] **Step 3: Migrate the primary actions loop (line ~126)**

Before:

```svelte
{#each primaryActions as action (action.id)}
  <button class="btn btn-sm preset-filled-primary-500" onclick={() => onaction(action.id)}>
    {action.label}
  </button>
{/each}
```

After:

```svelte
{#each primaryActions as action (action.id)}
  <Button
    variant={action.variant ?? (action.destructive ? 'danger' : 'primary')}
    size="sm"
    loading={action.loading}
    onclick={() => onaction(action.id)}
  >{action.label}</Button>
{/each}
```

- [ ] **Step 4: Migrate the More-menu trigger (line ~132)**

**`aria-haspopup`/`aria-expanded` forwarding:** `Button.svelte` does NOT use a rest-props spread —
it only renders the props defined in `ButtonProps`. Passing `aria-haspopup` or `aria-expanded` to
`<Button>` will produce a TypeScript error and the attributes will NOT reach the underlying `<button>`
element.

**Resolution:** Keep the More-menu trigger as a raw `<button>` styled to match the `secondary sm`
contract manually, until Button.svelte adds rest-prop forwarding:

Before:

```svelte
<button
  class="btn btn-sm preset-tonal-surface"
  onclick={toggleMoreMenu}
  aria-label="More actions"
  aria-haspopup="menu"
  aria-expanded={showMoreMenu}
>
  &hellip; More
</button>
```

After (raw `<button>` with Button-equivalent classes, preserving ARIA attributes):

```svelte
<button
  class="inline-flex items-center gap-1.5 rounded-[3px] font-bold uppercase tracking-wide
    transition-[background,border-color,color] duration-[0.12s]
    disabled:opacity-40 disabled:pointer-events-none active:opacity-[0.88]
    focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]
    h-[19px] px-2 text-[8.5px]
    bg-[var(--bg-raised)] border border-[var(--border-default)] text-[var(--text-primary)]
    hover:bg-[var(--bg-hover)] active:opacity-[0.88]"
  onclick={toggleMoreMenu}
  aria-label="More actions"
  aria-haspopup="menu"
  aria-expanded={showMoreMenu}
>
  &hellip; More
</button>
```

Document this in the PR description as a known limitation pending Button.svelte rest-prop forwarding.
This preserves the full keyboard-accessible dropdown contract without requiring a Button primitive change
in this PR. If Button.svelte gains rest-prop forwarding in a future spec, this site can be migrated
then.

- [ ] **Step 5: Migrate the Deselect all button (line ~170)**

Before:

```svelte
<button class="btn btn-sm preset-tonal-surface" onclick={oncancel}>Deselect all</button>
```

After:

```svelte
<Button variant="secondary" size="sm" onclick={oncancel}>Deselect all</Button>
```

- [ ] **Step 6: Verify**

```bash
cd frontend && npm run check 2>&1 | grep -i 'BatchActionBar'
```

Expected: no errors.

- [ ] **Step 7: Commit**

```bash
git add frontend/src/lib/components/BatchActionBar.svelte
git commit -m "refactor(batch-action-bar): migrate to Button primitive, extend actions type with variant/loading (#3k)"
```

---

## Task 5: Migrate `Pagination.svelte`

**Files:**

- Modify: `frontend/src/lib/components/Pagination.svelte`

Read the file before editing. It is 83 lines.

- [ ] **Step 1: Add `Button` import**

In the `<script lang="ts">` block:

```ts
import Button from './Button.svelte';
```

- [ ] **Step 2: Declare chevron icon snippets**

In the template, before the `{#if totalPages > 1}` block, declare both snippets:

```svelte
{#snippet prevIcon()}
  <svg aria-hidden="true" width="10" height="10" viewBox="0 0 10 10" fill="none">
    <path d="M6.5 2.5L3.5 5l3 2.5" stroke="currentColor" stroke-width="1.5"
      stroke-linecap="round" stroke-linejoin="round"/>
  </svg>
{/snippet}

{#snippet nextIcon()}
  <svg aria-hidden="true" width="10" height="10" viewBox="0 0 10 10" fill="none">
    <path d="M3.5 2.5l3 2.5-3 2.5" stroke="currentColor" stroke-width="1.5"
      stroke-linecap="round" stroke-linejoin="round"/>
  </svg>
{/snippet}
```

- [ ] **Step 3: Migrate the Previous button (line ~52)**

Before:

```svelte
<button
  class="btn btn-sm preset-tonal h-8 min-h-8 px-3 text-[10px]"
  disabled={currentPage <= 1}
  onclick={() => onPageChange(currentPage - 1)}
>
  Previous
</button>
```

After:

```svelte
<Button
  variant="ghost"
  size="sm"
  class="h-8 min-h-8 px-3 text-[10px]"
  leadingIcon={prevIcon}
  disabled={currentPage <= 1}
  onclick={() => onPageChange(currentPage - 1)}
>Previous</Button>
```

- [ ] **Step 4: Migrate page-number buttons (lines ~63–72)**

Before:

```svelte
<button
  class={`btn btn-sm h-8 min-h-8 min-w-8 px-2.5 text-[10px] ${
    p === currentPage ? 'preset-filled-primary-500' : 'preset-tonal'
  }`}
  onclick={() => onPageChange(p)}
  aria-current={p === currentPage ? 'page' : undefined}
>
  {p}
</button>
```

After:

```svelte
<Button
  variant="ghost"
  size="sm"
  class={[
    'h-8 min-h-8 min-w-8 px-2.5 text-[10px]',
    p === currentPage ? 'text-[var(--accent)] bg-[var(--bg-hover)]' : ''
  ].join(' ').trim()}
  aria-current={p === currentPage ? 'page' : undefined}
  onclick={() => onPageChange(p)}
>{p}</Button>
```

- [ ] **Step 5: Migrate the Next button (line ~74)**

Before:

```svelte
<button
  class="btn btn-sm preset-tonal h-8 min-h-8 px-3 text-[10px]"
  disabled={currentPage >= totalPages}
  onclick={() => onPageChange(currentPage + 1)}
>
  Next
</button>
```

After:

```svelte
<Button
  variant="ghost"
  size="sm"
  class="h-8 min-h-8 px-3 text-[10px]"
  trailingIcon={nextIcon}
  disabled={currentPage >= totalPages}
  onclick={() => onPageChange(currentPage + 1)}
>Next</Button>
```

- [ ] **Step 6: Verify**

```bash
cd frontend && npm run check 2>&1 | grep -i 'Pagination'
```

Expected: no errors.

- [ ] **Step 7: Commit**

```bash
git add frontend/src/lib/components/Pagination.svelte
git commit -m "refactor(pagination): migrate all page buttons to Button primitive with ghost+size-override contract (#3k)"
```

---

## Task 6: Migrate `ToastNotifications.svelte`

**Files:**

- Modify: `frontend/src/lib/components/ToastNotifications.svelte`

Read around lines 379 and 386 before editing. The file is ~406 lines.

- [ ] **Step 1: Add `Button` import**

Check if `Button` is already imported. Add only if missing:

```ts
import Button from './Button.svelte';
```

- [ ] **Step 2: Migrate the Dismiss button (line ~379)**

Before:

```svelte
<button class="btn btn-sm preset-tonal-surface" onclick={() => dismissToast(item)}>Dismiss</button>
```

After:

```svelte
<Button variant="ghost" size="sm" onclick={() => dismissToast(item)}>Dismiss</Button>
```

- [ ] **Step 3: Verify line ~386 is untouched**

Confirm `<a href="/settings/global" class="btn btn-sm preset-tonal">Go to Global Settings</a>` is unchanged.
Do NOT modify it.

- [ ] **Step 4: Verify**

```bash
cd frontend && npm run check 2>&1 | grep -i 'ToastNotifications'
```

Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/lib/components/ToastNotifications.svelte
git commit -m "refactor(toast-notifications): migrate Dismiss button to Button primitive (#3k)"
```

---

## Task 7: Migrate `AssignToHostModal.svelte`

**Files:**

- Modify: `frontend/src/lib/components/AssignToHostModal.svelte`

Read lines ~360–550 before editing. The file is ~550 lines.

- [ ] **Step 1: Verify `Button` import**

Check if `Button` is already imported. Add if missing:

```ts
import Button from './Button.svelte';
```

- [ ] **Step 2: Migrate addHook buttons (~371 and ~498)**

Both pre_update_hook and post_update_hook `+ Add` buttons follow this pattern.

Before:

```svelte
<button type="button" class="btn btn-sm preset-tonal-surface text-xs" onclick={() => addHook(hookRole)}>
  + Add
</button>
```

After:

```svelte
<Button variant="secondary" size="sm" type="button" onclick={() => addHook(hookRole)}>+ Add</Button>
```

Drop `text-xs` — `size="sm"` governs typography.

- [ ] **Step 3: Migrate removeHook buttons (~398 and ~525)**

Before:

```svelte
<button type="button" class="btn btn-sm preset-tonal-error text-xs shrink-0"
  onclick={() => removeHook(hookRole, entry.localKey)}>
  Remove
</button>
```

After:

```svelte
<Button variant="danger" size="sm" class="shrink-0" type="button"
  onclick={() => removeHook(hookRole, entry.localKey)}>Remove</Button>
```

Keep `shrink-0` (layout concern); drop `text-xs`.

- [ ] **Step 4: Migrate Cancel footer button (~545)**

Before:

```svelte
<button class="btn preset-tonal-surface" onclick={onclose}>Cancel</button>
```

After:

```svelte
<Button variant="secondary" onclick={onclose}>Cancel</Button>
```

- [ ] **Step 5: Migrate Save footer button (~546)**

Before:

```svelte
<button class="btn preset-filled-primary-500" disabled={submitting || loading || !!loadError} onclick={submit}>
  {submitting ? 'Saving...' : 'Save'}
</button>
```

After:

```svelte
<Button variant="primary" loading={submitting} disabled={loading || !!loadError} onclick={submit}>Save</Button>
```

`submitting` moves to `loading` (the primitive sets `disabled` internally while loading). `loading || !!loadError`
stays on `disabled` (data-not-ready cases unrelated to submitting). Children are static `Save`.

- [ ] **Step 6: Verify**

```bash
cd frontend && npm run check 2>&1 | grep -i 'AssignToHostModal'
```

Expected: no errors.

- [ ] **Step 7: Commit**

```bash
git add frontend/src/lib/components/AssignToHostModal.svelte
git commit -m "refactor(assign-to-host-modal): migrate all 6 button sites to Button primitive (#3k)"
```

---

## Task 8: Migrate `EditHostAssignmentModal.svelte`

**Files:**

- Modify: `frontend/src/lib/components/EditHostAssignmentModal.svelte`

This file is ~1365 lines. Re-grep before editing to get exact line numbers:

```bash
grep -n "btn preset\|btn btn-sm preset" frontend/src/lib/components/EditHostAssignmentModal.svelte
```

- [ ] **Step 1: Verify `Button` import**

Check if `Button` is already imported. Add if missing:

```ts
import Button from './Button.svelte';
```

- [ ] **Step 2: Migrate the eight JSON view-mode toggle buttons**

All eight currently match `class="btn btn-sm preset-tonal text-xs"`. Each migrates to the same pattern —
only `onclick` handler and children text differ:

```svelte
<Button variant="secondary" size="sm" type="button" onclick={…}>children text</Button>
```

Locations and children text (verify against live source):

- `Edit as JSON` (standard role, form→JSON, ~line 817)
- `Back to Form` (standard role, JSON→form, ~line 841)
- `Advanced: Edit as JSON` (standard role advanced, ~line 943)
- `Back to Form` (standard role advanced, ~line 969)
- `Edit as JSON` (hook entry, form→JSON, ~line 1137)
- `Back to Form` (hook entry, JSON→form, ~line 1163)
- `Advanced: Edit as JSON` (hook entry advanced, ~line 1270)
- `Back to Form` (hook entry advanced, ~line 1298)

Drop `text-xs` from all. Move any layout classes (e.g. `shrink-0`) to the `class` prop.

- [ ] **Step 3: Migrate the `+ Add` hook-row primary button (~1015)**

Before:

```svelte
<button type="button" class="btn btn-sm preset-tonal-primary text-xs shrink-0" onclick={() => addHook(hookRole)}>
  + Add
</button>
```

After:

```svelte
<Button variant="primary" size="sm" class="shrink-0" type="button" onclick={() => addHook(hookRole)}>+ Add</Button>
```

- [ ] **Step 4: Migrate the `Remove` hook-entry destructive button (~1036)**

Before:

```svelte
<button type="button" class="btn btn-sm preset-tonal-error text-xs"
  onclick={() => requestHookRemoval(hookRole, entry.localKey)}>
  Remove
</button>
```

After:

```svelte
<Button variant="danger" size="sm" type="button"
  onclick={() => requestHookRemoval(hookRole, entry.localKey)}>Remove</Button>
```

- [ ] **Step 5: Migrate Cancel footer button (~1346)**

Before:

```svelte
<button class="btn preset-tonal-surface" onclick={onclose}>Cancel</button>
```

After:

```svelte
<Button variant="secondary" onclick={onclose}>Cancel</Button>
```

- [ ] **Step 6: Migrate Save Changes footer button (~1347)**

Before:

```svelte
<button class="btn preset-filled-primary-500" onclick={save} disabled={submitting || loading || !!loadError}>
  {submitting ? 'Saving…' : 'Save Changes'}
</button>
```

After:

```svelte
<Button variant="primary" loading={submitting} disabled={loading || !!loadError} onclick={save}>Save Changes</Button>
```

Static children `Save Changes` — NOT `Save`. Remove the `Saving…` text-swap ternary.

- [ ] **Step 7: Remove `confirmClass` from the inline `<ConfirmDialog>` (~1359)**

Remove the `confirmClass="preset-filled-error-500"` prop line. The new default `confirmVariant='danger'` is
identical.

- [ ] **Step 8: Verify**

```bash
cd frontend && npm run check 2>&1 | grep -i 'EditHostAssignmentModal'
```

Expected: no errors.

- [ ] **Step 9: Commit**

```bash
git add frontend/src/lib/components/EditHostAssignmentModal.svelte
git commit -m "refactor(edit-host-assignment-modal): migrate all 12 button sites to Button primitive (#3k)"
```

---

## Task 9: Extend unit tests

**Files:**

- Modify: `frontend/src/lib/components/ConfirmDialog.test.ts`
- Modify: `frontend/src/lib/components/Pagination.test.ts`
- Modify: `frontend/src/lib/components/ToastNotifications.test.ts`
- Modify: `frontend/src/lib/components/AssignToHostModal.test.ts`
- Modify: `frontend/src/lib/components/EditHostAssignmentModal.test.ts`
- Create: `frontend/src/lib/components/BatchResultDialog.test.ts`
- Create: `frontend/src/lib/components/BatchActionBar.test.ts`

Read each existing test file in full before editing. Types in `.test.ts` files are defined locally.

### Step 1: Extend `ConfirmDialog.test.ts`

Add these test cases to the existing `describe('ConfirmDialog', ...)` block:

- [ ] **Confirm button renders danger variant by default:**

```ts
it('confirm button renders variant="danger" by default', () => {
  render(ConfirmDialog, defaultProps);
  const confirmBtn = screen.getByRole('button', { name: 'Delete' });
  expect(confirmBtn.className).toMatch(/danger|error/);
});
```

- [ ] **confirmVariant="primary" renders primary variant:**

```ts
it('confirm button renders variant="primary" when confirmVariant="primary"', () => {
  render(ConfirmDialog, { ...defaultProps, confirmVariant: 'primary' });
  const confirmBtn = screen.getByRole('button', { name: 'Delete' });
  expect(confirmBtn.className).not.toMatch(/danger|error/);
});
```

- [ ] **Cancel always renders secondary:**

```ts
it('cancel button renders variant="secondary"', () => {
  render(ConfirmDialog, defaultProps);
  const cancelBtn = screen.getByRole('button', { name: 'Cancel' });
  expect(cancelBtn.className).toContain('border');
});
```

- [ ] **confirmDisabled passthrough — false (default):**

```ts
it('confirm button is NOT disabled when confirmDisabled=false (default)', () => {
  render(ConfirmDialog, defaultProps);
  expect(screen.getByRole('button', { name: 'Delete' })).not.toBeDisabled();
});
```

- [ ] **confirmDisabled passthrough — true:**

```ts
it('confirm button IS disabled when confirmDisabled=true', () => {
  render(ConfirmDialog, { ...defaultProps, confirmDisabled: true });
  expect(screen.getByRole('button', { name: 'Delete' })).toBeDisabled();
});
```

### Step 2: Create `BatchResultDialog.test.ts`

```ts
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import BatchResultDialog from './BatchResultDialog.svelte';
import type { BatchActionResponse } from '$lib/types';

afterEach(() => { cleanup(); vi.clearAllMocks(); });

type LocalResponse = BatchActionResponse;
const makeResponse = (succeeded: string[], failed: { id: string; error: string }[]): LocalResponse =>
  ({ succeeded, failed });

describe('BatchResultDialog', () => {
  it('Close button renders variant="primary"', () => {
    render(BatchResultDialog, { title: 'Results', response: makeResponse(['id-1'], []), onclose: vi.fn() });
    const closeBtn = screen.getByRole('button', { name: 'Close' });
    expect(closeBtn.className).toMatch(/bg-\[linear-gradient/);
  });

  it('calls onclose when Close is clicked', () => {
    const onclose = vi.fn();
    render(BatchResultDialog, { title: 'Results', response: makeResponse(['id-1'], []), onclose });
    fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(onclose).toHaveBeenCalledOnce();
  });
});
```

### Step 3: Create `BatchActionBar.test.ts`

```ts
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import BatchActionBar from './BatchActionBar.svelte';

afterEach(() => { cleanup(); vi.clearAllMocks(); });

type Action = {
  id: string;
  label: string;
  destructive?: boolean;
  variant?: 'primary' | 'secondary' | 'danger';
  loading?: boolean;
};

describe('BatchActionBar', () => {
  it('non-destructive action renders variant="primary"', () => {
    const actions: Action[] = [{ id: 'do', label: 'Do It', destructive: false }];
    render(BatchActionBar, { selectedCount: 2, actions, onaction: vi.fn(), oncancel: vi.fn() });
    const btn = screen.getByRole('button', { name: 'Do It' });
    expect(btn.className).toMatch(/bg-\[linear-gradient/);
  });

  it('destructive action renders variant="danger"', () => {
    const actions: Action[] = [{ id: 'del', label: 'Delete', destructive: true }];
    render(BatchActionBar, { selectedCount: 1, actions, onaction: vi.fn(), oncancel: vi.fn() });
    expect(screen.getByRole('button', { name: 'Delete' }).className).toMatch(/danger|error/);
  });

  it('explicit variant override wins over destructive flag', () => {
    const actions: Action[] = [{ id: 'act', label: 'Mark', destructive: true, variant: 'secondary' }];
    render(BatchActionBar, { selectedCount: 1, actions, onaction: vi.fn(), oncancel: vi.fn() });
    const btn = screen.getByRole('button', { name: 'Mark' });
    expect(btn.className).toContain('border');
    expect(btn.className).not.toMatch(/danger|error/);
  });

  it('action with loading=true has aria-busy="true"', () => {
    const actions: Action[] = [
      { id: 'a', label: 'Action A', loading: true },
      { id: 'b', label: 'Action B', loading: false }
    ];
    render(BatchActionBar, { selectedCount: 2, actions, onaction: vi.fn(), oncancel: vi.fn() });
    expect(screen.getByRole('button', { name: 'Action A' })).toHaveAttribute('aria-busy', 'true');
    expect(screen.getByRole('button', { name: 'Action B' })).not.toHaveAttribute('aria-busy');
  });

  it('onaction fires with the correct id on click', () => {
    const onaction = vi.fn();
    const actions: Action[] = [{ id: 'my-action', label: 'Run' }];
    render(BatchActionBar, { selectedCount: 1, actions, onaction, oncancel: vi.fn() });
    fireEvent.click(screen.getByRole('button', { name: 'Run' }));
    expect(onaction).toHaveBeenCalledWith('my-action');
  });

  it('Deselect all button renders variant="secondary" size="sm"', () => {
    render(BatchActionBar, { selectedCount: 2, actions: [{ id: 'x', label: 'X' }], onaction: vi.fn(), oncancel: vi.fn() });
    const btn = screen.getByRole('button', { name: 'Deselect all' });
    expect(btn.className).toContain('border');
  });
});
```

### Step 4: Extend `Pagination.test.ts`

- [ ] **Replace the stale `preset-filled-primary-500` assertion (line ~47)**

Before:

```ts
expect(currentBtn.className).toContain('preset-filled-primary-500');
```

After:

```ts
expect(currentBtn.className).toContain('text-[var(--accent)]');
expect(currentBtn.className).toContain('bg-[var(--bg-hover)]');
```

- [ ] **Add new assertions:**

```ts
it('Previous and Next buttons carry h-8 height override class', () => {
  render(Pagination, { currentPage: 2, totalPages: 5, onPageChange: vi.fn() });
  expect(screen.getByRole('button', { name: /previous/i }).className).toContain('h-8');
  expect(screen.getByRole('button', { name: /next/i }).className).toContain('h-8');
});

it('page-number buttons carry h-8 height override class', () => {
  render(Pagination, { currentPage: 2, totalPages: 5, onPageChange: vi.fn() });
  expect(screen.getByRole('button', { name: '3' }).className).toContain('h-8');
});

it('inactive page-number buttons do not carry active accent/bg-hover classes', () => {
  render(Pagination, { currentPage: 2, totalPages: 5, onPageChange: vi.fn() });
  const inactiveBtn = screen.getByRole('button', { name: '3' });
  expect(inactiveBtn.className).not.toContain('text-[var(--accent)]');
  expect(inactiveBtn.className).not.toContain('bg-[var(--bg-hover)]');
});

it('Previous button has a leadingIcon SVG in the DOM', () => {
  render(Pagination, { currentPage: 2, totalPages: 5, onPageChange: vi.fn() });
  const prevBtn = screen.getByRole('button', { name: /previous/i });
  expect(prevBtn.querySelector('svg')).not.toBeNull();
});

it('Next button has a trailingIcon SVG in the DOM', () => {
  render(Pagination, { currentPage: 2, totalPages: 5, onPageChange: vi.fn() });
  const nextBtn = screen.getByRole('button', { name: /next/i });
  expect(nextBtn.querySelector('svg')).not.toBeNull();
});
```

### Step 5: Extend `ToastNotifications.test.ts`

Read the file in full before editing. It mocks `$lib/notifications.svelte` and uses fake timers.

- [ ] **Add assertions for Dismiss Button migration:**

```ts
it('Dismiss renders as Button variant="ghost" size="sm" with text "Dismiss"', () => {
  notificationState.errorMessage = 'Something failed';
  render(ToastNotifications, { alerts: [], onDismiss: vi.fn() });
  const dismissBtn = screen.getByRole('button', { name: 'Dismiss' });
  expect(dismissBtn).not.toHaveAttribute('aria-busy');
  expect(dismissBtn.className).toContain('bg-transparent');
});

it('Go to Global Settings anchor is NOT a Button (belongs to #2b)', () => {
  const alerts: SystemAlert[] = [{
    id: 'cert-alert',
    severity: 'warning',
    title: 'Cert renewal',
    message: 'Certificate needs renewal',
    action: 'renew_server_certificate'
  } as SystemAlert];
  const { container } = render(ToastNotifications, { alerts, onDismiss: vi.fn() });
  const cta = container.querySelector('a[href="/settings/global"]') as HTMLElement;
  expect(cta).not.toBeNull();
  expect(cta.tagName.toLowerCase()).toBe('a');
});
```

Note: check the `SystemAlert` type definition to confirm the `action` field name and shape before writing the
fixture. Adjust the cast or type annotation to match the actual type.

### Step 6: Extend `AssignToHostModal.test.ts`

Read the existing test file in full. It mocks `$lib/api` and `$lib/notifications.svelte`.

- [ ] **Add Button primitive contract tests after the existing tests:**

```ts
it('addHook launcher renders variant="secondary" size="sm"', async () => {
  // Use the existing fixture that loads the modal with plugin configs.
  const addBtn = screen.getAllByRole('button', { name: '+ Add' })[0];
  expect(addBtn.className).toContain('border');
});

it('removeHook button renders variant="danger" size="sm" with shrink-0 class', async () => {
  const removeBtn = screen.getByRole('button', { name: 'Remove' });
  expect(removeBtn.className).toMatch(/danger|error/);
  expect(removeBtn.className).toContain('shrink-0');
});

it('Cancel button renders variant="secondary"', async () => {
  expect(screen.getByRole('button', { name: 'Cancel' }).className).toContain('border');
});

it('Save button renders variant="primary"', async () => {
  expect(screen.getByRole('button', { name: 'Save' }).className).toMatch(/bg-\[linear-gradient/);
});

it('Save button has no "Saving..." text in any rendered state', () => {
  expect(document.body.textContent).not.toContain('Saving...');
});
```

Read the existing render setup in the test file to determine which `describe` block and what test fixture to
use for each assertion — the above shows the assertion shape only.

### Step 7: Extend `EditHostAssignmentModal.test.ts`

Read the existing test file in full before editing.

- [ ] **Add Button primitive contract tests:**

```ts
it('view-mode toggle buttons render variant="secondary" size="sm"', async () => {
  const editJsonBtns = screen.getAllByRole('button', { name: /edit as json/i });
  expect(editJsonBtns.length).toBeGreaterThan(0);
  editJsonBtns.forEach((btn) => { expect(btn.className).toContain('border'); });
});

it('+ Add hook-row button renders variant="primary" with shrink-0', async () => {
  const addBtn = screen.getByRole('button', { name: '+ Add' });
  expect(addBtn.className).toMatch(/bg-\[linear-gradient/);
  expect(addBtn.className).toContain('shrink-0');
});

it('Remove hook-entry button renders variant="danger"', async () => {
  const removeBtn = screen.getByRole('button', { name: 'Remove' });
  expect(removeBtn.className).toMatch(/danger|error/);
});

it('Cancel button renders variant="secondary"', async () => {
  expect(screen.getByRole('button', { name: 'Cancel' }).className).toContain('border');
});

it('Save Changes label is "Save Changes" not "Save"', async () => {
  expect(screen.getByRole('button', { name: 'Save Changes' })).toBeInTheDocument();
  expect(screen.queryByRole('button', { name: /^save$/i })).not.toBeInTheDocument();
});

it('Save Changes button has no "Saving…" text in any rendered state', () => {
  expect(document.body.textContent).not.toContain('Saving…');
});
```

- [ ] **Step 8: Run unit test suite**

```bash
cd frontend && npx vitest run \
  src/lib/components/ConfirmDialog.test.ts \
  src/lib/components/BatchResultDialog.test.ts \
  src/lib/components/BatchActionBar.test.ts \
  src/lib/components/Pagination.test.ts \
  src/lib/components/ToastNotifications.test.ts \
  src/lib/components/AssignToHostModal.test.ts \
  src/lib/components/EditHostAssignmentModal.test.ts
```

Expected: all tests pass.

- [ ] **Step 9: Commit**

```bash
git add \
  frontend/src/lib/components/ConfirmDialog.test.ts \
  frontend/src/lib/components/BatchResultDialog.test.ts \
  frontend/src/lib/components/BatchActionBar.test.ts \
  frontend/src/lib/components/Pagination.test.ts \
  frontend/src/lib/components/ToastNotifications.test.ts \
  frontend/src/lib/components/AssignToHostModal.test.ts \
  frontend/src/lib/components/EditHostAssignmentModal.test.ts
git commit -m "test(shared-modals): add Button primitive contract tests for all seven migrated components (#3k)"
```

---

## Task 10: Re-baseline Playwright snapshots

**Files:**

- Modify or create: relevant e2e spec files

- [ ] **Step 1: Identify affected e2e specs**

```bash
ls frontend/tests/e2e/
```

Find specs covering `/software`, `/hosts`, `/services`, `/system-services`, `/settings`, `/host-tags`,
`/history`, `/profile`.

- [ ] **Step 2: Re-baseline each affected route's snapshots**

```bash
cd frontend && npx playwright test <spec> --update-snapshots
```

Routes that render the seven migrated components:

- `/software` — BatchActionBar, Pagination, BatchResultDialog, AssignToHostModal, EditHostAssignmentModal,
  ConfirmDialog
- `/hosts` — BatchActionBar, Pagination, ConfirmDialog
- `/services` — BatchActionBar, Pagination, ConfirmDialog, BatchResultDialog, ToastNotifications
- `/system-services` — same as services
- `/settings` — ConfirmDialog, ToastNotifications (notifications danger zone)
- `/host-tags` — ConfirmDialog, Pagination
- `/profile` — ConfirmDialog (revoke token)
- Any route that shows ToastNotifications (globally visible)

- [ ] **Step 3: Apply required snapshot masks**

For each spec being re-baselined, ensure these masks are configured:

- Mask `<Button loading>` spinner rotation: `{ selector: '[aria-busy="true"] svg' }` or the spinner class
- Mask `[data-ui="toast-progress"]` spans (progress bar animates `transform: scaleX(…)`)
- Mask transient toast banners raised inside the snapshot window
- Mask dynamic id / timestamp cells inside EditHostAssignmentModal rows

- [ ] **Step 4: Verify delta sizes match spec**

| Button kind | Expected height |
| --- | --- |
| `size="md"` (ConfirmDialog, BatchResultDialog, modal footers) | `h-[23px]` |
| `size="sm"` (BatchActionBar, row actions, Toast Dismiss, modal hook buttons) | `h-[19px]` |
| Pagination (class override) | `h-8` (32px) |

- [ ] **Step 5: Re-run all specs to confirm stability**

```bash
cd frontend && npx playwright test 2>&1 | tail -30
```

Expected: 0 failures.

- [ ] **Step 6: Commit**

```bash
git add "frontend/tests/e2e/"
git commit -m "test(e2e): re-baseline snapshots after shared-modals Button primitive migration (#3k)"
```

---

## Task 11: Full frontend gate

- [ ] **Step 1: Run full gate**

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

Expected: every command exits 0.

- [ ] **Step 2: Confirm zero `confirmClass=` occurrences**

```bash
cd frontend && grep -r "confirmClass=" src/ --include="*.svelte" --include="*.ts"
```

Expected: no output.

- [ ] **Step 3: Confirm zero raw `btn preset-` in the seven component files**

```bash
grep -n "btn preset\|btn btn-sm preset" \
  frontend/src/lib/components/ConfirmDialog.svelte \
  frontend/src/lib/components/BatchResultDialog.svelte \
  frontend/src/lib/components/BatchActionBar.svelte \
  frontend/src/lib/components/Pagination.svelte \
  frontend/src/lib/components/ToastNotifications.svelte \
  frontend/src/lib/components/AssignToHostModal.svelte \
  frontend/src/lib/components/EditHostAssignmentModal.svelte
```

Expected: no matches.

---

## Commit summary

| # | Commit message | Files |
| --- | --- | --- |
| 1 | refactor(confirm-dialog): replace confirmClass with confirmVariant prop, migrate buttons | `ConfirmDialog.svelte` |
| 2 | refactor(confirm-dialog): migrate all confirmClass= call sites to confirmVariant= | 12 consumer files |
| 3 | refactor(batch-result-dialog): migrate Close button | `BatchResultDialog.svelte` |
| 4 | refactor(batch-action-bar): migrate to Button primitive, extend actions type | `BatchActionBar.svelte` |
| 5 | refactor(pagination): migrate all page buttons with ghost+size-override contract | `Pagination.svelte` |
| 6 | refactor(toast-notifications): migrate Dismiss button | `ToastNotifications.svelte` |
| 7 | refactor(assign-to-host-modal): migrate all 6 button sites | `AssignToHostModal.svelte` |
| 8 | refactor(edit-host-assignment-modal): migrate all 12 button sites | `EditHostAssignmentModal.svelte` |
| 9 | test(shared-modals): add Button primitive contract tests for seven components | 7 test files |
| 10 | test(e2e): re-baseline snapshots after shared-modals migration | e2e spec + PNGs |
