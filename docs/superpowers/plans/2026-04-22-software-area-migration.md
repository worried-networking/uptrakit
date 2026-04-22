# Software Area Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate all raw Skeleton `preset-*`/`btn` button elements in the software routes and related shared
components to the `<Button>` and `<UpdateAllButton>` primitives.

**Architecture:** Five files, one task per file; Pagination.svelte deferred to #3k; per-host loading guard pattern
for the detail page trigger.

**Tech Stack:** Svelte 5, SvelteKit, TypeScript, Vitest, Playwright

---

## Dependency

**Blocks on:** sub-spec #2 merged (`Button` + `UpdateAllButton` primitives), sub-spec #2c merged
(`variant="secondary"` + `ariaLabel` + `--bg-hover` token), sub-spec #2b merged (Checkbox primitive for row +
header selection on software list).

---

## Migration rules (quick reference)

| Legacy class | Button primitive |
| --- | --- |
| `preset-filled-primary-500` | `variant="primary"` |
| `preset-filled-error-500` / `preset-tonal-error` | `variant="danger"` |
| `preset-tonal-surface` | `variant="secondary"` (modal/wizard footers) or `variant="ghost"` (row actions) |
| `preset-tonal` (context actions row) | `variant="ghost" size="sm"` |
| `btn-sm` row action | add `size="sm"` |
| default `btn` (modal footer) | default `size="md"` |

Aggregate header "Update all" → `<UpdateAllButton state count onclick ariaLabel>`.
Per-host single "Trigger update" → `<Button variant="primary" size="sm" loading={isTriggeringHostId === host.host_id}>`.

**Pagination.svelte is OUT OF SCOPE — do not touch it. It is owned by #3k.**

---

## Button site inventory (read each source file before editing to confirm exact lines)

**`software/+page.svelte`** — 7 button sites + context-menu actions button:

1. Header toolbar: "Add Software" → `variant="primary" size="sm"`
2. Error callout: "Retry" → `variant="primary" size="sm"`
3. Actions context-menu toggle (per row) → `variant="ghost" size="sm"` (three-dot button)
4. Trigger Update modal footer: Cancel → `variant="secondary"`,
   "Update N host(s)" → `variant="primary" loading={triggeringUpdate}`
5. Edit Software modal footer: Cancel → `variant="secondary"`, Save → `variant="primary" loading={editSubmitting}`
6. `<UpdateAllButton>` block at lines 1100–1105 already uses the primitive for multi-host groups —
   verify in source and leave untouched if already correct.
7. The "▸ N more" overflow-hosts toggle and the star/featured toggle have no `btn`/`preset-*` classes
   — **leave them as-is**, not in scope.

**`software/[id]/+page.svelte`** — 13 button sites:

1. Error callout: "Retry" → `variant="primary" size="sm"`
2. Header: "Update All" → `variant="primary"` (replaces `preset-filled-warning-500`)
3. Header: "Assign to Host" → `variant="secondary"`
4. Header: "Check All Versions" → `variant="secondary" loading={checkingAll}`
5. Header: "Merge..." → `variant="secondary"` (shown when `canMergeSoftware`)
6. Header: "Edit" → `variant="secondary"`
7. Header: "Delete" → `variant="danger" loading={deleteSubmitting}`
8. Host table: actions context-menu toggle → `variant="ghost" size="sm"`
9. Confirm Update modal footer: Cancel → `variant="secondary"`,
   "Trigger Update" → `variant="primary" loading={updateTriggering}`
10. Update All modal footer: Cancel → `variant="secondary"`,
    "Update N host(s)" → `variant="primary" loading={updateAllTriggering}`
11. Edit Software modal footer: Cancel → `variant="secondary"`, Save → `variant="primary" loading={editSubmitting}`
12. Release notes modal: "View on GitHub ↗" `<a>` with `btn btn-sm preset-tonal-surface`
    → `<Button variant="ghost" size="sm" href={url}>`
13. Release notes modal footer: Close → `variant="secondary"`

**`IgnoreRulesTab.svelte`** — 3 button sites:

1. Tab header: "Add Ignore Rule" → `variant="primary" size="sm"`
2. Table row: "Delete" (per-row) → `variant="danger" size="sm"` (replaces `btn-sm preset-tonal-error`)
3. Modal footer: Cancel → `variant="secondary"`,
   Create → `variant="primary" disabled={!ignoreForm.name.trim()}` (no loading state)

**`SoftwareMergeWizard.svelte`** — 6 button sites:

1. Step-1 search panel: "Search" → `variant="ghost"` (replaces `btn preset-tonal-surface`)
2. Step-1 candidates list: "Add" (per-result) → `variant="ghost" size="sm"`
3. Step-1 selected list: "Remove" (per-candidate) → `variant="ghost" size="sm"`
4. Wizard footer: Cancel → `variant="ghost"` (enabled throughout, even during submit)
5. Wizard footer: Back (step 2 only) → `variant="secondary"` (replaces `btn preset-tonal-surface`)
6. Wizard footer: Next (step 1) / Merge (step 2) → `variant="primary" loading={loading}`

**`AddSoftwareModal.svelte`** — 2 button sites:

1. Footer: Cancel → `variant="secondary"` (replaces `btn preset-tonal-surface`)
2. Footer: "Register Software" → `variant="primary" loading={submitting}` with static children
   "Register Software" — the existing `{submitting ? 'Registering...' : 'Register Software'}` ternary is removed.

---

## Task 1: Migrate `software/+page.svelte`

**Files:**

- Modify: `frontend/src/routes/software/+page.svelte`

Read the file in full before editing. The `UpdateAllButton` import is already present at line 61.

- [ ] **Step 1: Add `Button` import**

In the `<script lang="ts">` block, add after the `UpdateAllButton` import:

```ts
import Button from '$lib/components/Button.svelte';
```

- [ ] **Step 2: Migrate "Add Software" button (header toolbar, line ~917)**

Before:

```svelte
<button class="btn preset-filled-primary-500" onclick={() => (showAddModal = true)}>Add Software</button>
```

After:

```svelte
<Button variant="primary" size="sm" onclick={() => (showAddModal = true)}>Add Software</Button>
```

- [ ] **Step 3: Migrate "Retry" button (error callout, line ~925)**

Before:

```svelte
<button class="btn preset-filled-primary-500 mt-3" onclick={() => loadAll(currentPage)}>Retry</button>
```

After:

```svelte
<Button variant="primary" size="sm" class="mt-3" onclick={() => loadAll(currentPage)}>Retry</Button>
```

- [ ] **Step 4: Migrate the context-menu actions toggle (per-row three-dot, lines ~1124–1134)**

Before:

```svelte
<button
  class="btn btn-sm preset-tonal"
  aria-label={'Actions for ' + item.name}
  onclick={(e) => {
    e.stopPropagation();
    toggleMenu(item.id, e.currentTarget);
  }}
>
  &#8943;
</button>
```

After:

```svelte
<Button
  variant="ghost"
  size="sm"
  ariaLabel={'Actions for ' + item.name}
  onclick={(e) => {
    e.stopPropagation();
    toggleMenu(item.id, e.currentTarget);
  }}
>&#8943;</Button>
```

Note: `e.currentTarget` is used in `toggleMenu` to position the menu. The `Button` primitive's `onclick`
prop passes the native `MouseEvent`, so `e.currentTarget` is the underlying `<button>` element. Verify
after migration.

- [ ] **Step 5: Migrate "Trigger Update" modal footer (lines ~1475–1484)**

Before (`{#snippet footer()}`):

```svelte
<button class="btn preset-tonal-surface" onclick={() => (updateModalItem = null)}> Cancel </button>
<button
  class="btn preset-filled-primary-500"
  disabled={selectedHostIds.size === 0 || triggeringUpdate}
  onclick={executeUpdate}
>
  {triggeringUpdate ? 'Triggering...' : `Update ${selectedHostIds.size} host(s)`}
</button>
```

After:

```svelte
<Button variant="secondary" onclick={() => (updateModalItem = null)}>Cancel</Button>
<Button
  variant="primary"
  loading={triggeringUpdate}
  disabled={selectedHostIds.size === 0}
  onclick={executeUpdate}
>Update {selectedHostIds.size} host(s)</Button>
```

`disabled={selectedHostIds.size === 0}` guards empty selection. `loading={triggeringUpdate}` replaces the
ternary text with a spinner + static label.

- [ ] **Step 6: Migrate "Edit Software" modal footer (lines ~1508–1513)**

Before:

```svelte
<button class="btn preset-tonal-surface" onclick={() => (editItem = null)}>Cancel</button>
<button class="btn preset-filled-primary-500" onclick={executeEdit} disabled={editSubmitting}>
  {editSubmitting ? 'Saving...' : 'Save'}
</button>
```

After:

```svelte
<Button variant="secondary" onclick={() => (editItem = null)}>Cancel</Button>
<Button variant="primary" loading={editSubmitting} onclick={executeEdit}>Save</Button>
```

- [ ] **Step 7: Verify UpdateAllButton and ActionBadge call sites are untouched**

Confirm lines ~1099–1106 (the `UpdateAllButton` block for multi-host groups) already use the primitive
with correct props and require no changes.

- [ ] **Step 8: Compile check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npm run check 2>&1 | grep -i 'software/+page'
```

Expected: no type errors on this file.

- [ ] **Step 9: Commit**

```bash
git add frontend/src/routes/software/+page.svelte
git commit -m "refactor(software): migrate +page.svelte button sites to Button primitive (#3f)"
```

---

## Task 2: Migrate `software/[id]/+page.svelte`

**Files:**

- Modify: `frontend/src/routes/software/[id]/+page.svelte`

Read the file in full before editing. The detail page uses `openUpdateModal(host)` to launch a confirm
modal, then `executeUpdate()` does the actual trigger via modal footer. Lines 916–930 use `ActionBadge`
for the status/update-available column — these are not `<Button>` sites. There is no inline per-host row
trigger button to wire up with `isTriggeringHostId`. All "Trigger update" loading is in the confirm modal
footer via `loading={updateTriggering}`.

- [ ] **Step 1: Add `Button` import**

```ts
import Button from '$lib/components/Button.svelte';
```

- [ ] **Step 2: Migrate "Retry" button (error callout, line ~733)**

Before:

```svelte
<button class="btn preset-filled-primary-500 mt-2" onclick={() => loadItem()}>Retry</button>
```

After:

```svelte
<Button variant="primary" size="sm" class="mt-2" onclick={() => loadItem()}>Retry</Button>
```

- [ ] **Step 3: Migrate header action buttons (lines ~786–806)**

Before:

```svelte
{#if canTriggerUpdates && item.update_available}
  <button class="btn preset-filled-warning-500" onclick={openUpdateAllModal}> Update All </button>
{/if}
<button class="btn preset-tonal-surface" onclick={() => (showAssignModal = true)}>
  Assign to Host
</button>
<button class="btn preset-tonal-surface" onclick={checkAllVersions} disabled={checkingAll}>
  {checkingAll ? 'Checking...' : 'Check All Versions'}
</button>
{#if canMergeSoftware}
  <button class="btn preset-tonal-surface" onclick={openMergeModal}>Merge...</button>
{/if}
<button class="btn preset-tonal-surface" onclick={openEditModal}>Edit</button>
<button
  class="btn preset-filled-error-500"
  onclick={() => (confirmDelete = true)}
  disabled={deleteSubmitting}
>
  Delete
</button>
```

After:

```svelte
{#if canTriggerUpdates && item.update_available}
  <Button variant="primary" onclick={openUpdateAllModal}>Update All</Button>
{/if}
<Button variant="secondary" onclick={() => (showAssignModal = true)}>Assign to Host</Button>
<Button variant="secondary" loading={checkingAll} onclick={checkAllVersions}>Check All Versions</Button>
{#if canMergeSoftware}
  <Button variant="secondary" onclick={openMergeModal}>Merge...</Button>
{/if}
<Button variant="secondary" onclick={openEditModal}>Edit</Button>
<Button variant="danger" loading={deleteSubmitting} onclick={() => (confirmDelete = true)}>Delete</Button>
```

`loading={checkingAll}` replaces the `{checkingAll ? 'Checking...' : 'Check All Versions'}` ternary.
`loading={deleteSubmitting}` replaces `disabled={deleteSubmitting}` — Button sets `disabled` internally
when `loading`, and `executeDelete` already guards against double-submit.

- [ ] **Step 4: Migrate host table actions context-menu toggle (lines ~939–949)**

Before:

```svelte
<button
  class="btn btn-sm preset-tonal"
  aria-label="Actions for {host.hostname}"
  onclick={(e) => {
    e.stopPropagation();
    toggleMenu(host.id, e.currentTarget);
  }}
>
  &#8943;
</button>
```

After:

```svelte
<Button
  variant="ghost"
  size="sm"
  ariaLabel="Actions for {host.hostname}"
  onclick={(e) => {
    e.stopPropagation();
    toggleMenu(host.id, e.currentTarget);
  }}
>&#8943;</Button>
```

- [ ] **Step 5: Migrate "View on GitHub ↗" link in release notes modal (lines ~1070–1074)**

Before:

```svelte
<a
  href={releaseNotesModal.meta.release_url}
  target="_blank"
  rel="noopener noreferrer"
  class="btn btn-sm preset-tonal-surface shrink-0">View on GitHub ↗</a
>
```

After:

```svelte
<Button
  variant="ghost"
  size="sm"
  href={releaseNotesModal.meta.release_url}
  class="shrink-0"
>View on GitHub ↗</Button>
```

Note: `<Button href="...">` renders `<a role="button">`. The `target="_blank"` and `rel="noopener noreferrer"`
attributes are not in the Button primitive's current contract — if Button does not forward arbitrary anchor
attributes, keep this as a plain `<a>` with manually applied Button classes and note as deferred. Do NOT
remove security attributes.

- [ ] **Step 6: Migrate release notes modal footer (line ~1089)**

Before:

```svelte
<button class="btn preset-tonal-surface" onclick={() => (releaseNotesModal = null)}>Close</button>
```

After:

```svelte
<Button variant="secondary" onclick={() => (releaseNotesModal = null)}>Close</Button>
```

- [ ] **Step 7: Migrate "Confirm Update" modal footer (lines ~1050–1054)**

Before:

```svelte
<button class="btn preset-tonal-surface" onclick={() => (updateModal = null)}>Cancel</button>
<button class="btn preset-filled-warning-500" onclick={executeUpdate} disabled={updateTriggering}>
  {updateTriggering ? 'Triggering...' : 'Trigger Update'}
</button>
```

After:

```svelte
<Button variant="secondary" onclick={() => (updateModal = null)}>Cancel</Button>
<Button variant="primary" loading={updateTriggering} onclick={executeUpdate}>Trigger Update</Button>
```

- [ ] **Step 8: Migrate "Update All" modal footer (lines ~1188–1196)**

Before:

```svelte
<button class="btn preset-tonal-surface" onclick={() => (updateAllModal = false)}> Cancel </button>
<button
  class="btn preset-filled-primary-500"
  disabled={updateAllSelectedHostIds.size === 0 || updateAllTriggering}
  onclick={executeUpdateAll}
>
  {updateAllTriggering ? 'Triggering...' : `Update ${updateAllSelectedHostIds.size} host(s)`}
</button>
```

After:

```svelte
<Button variant="secondary" onclick={() => (updateAllModal = false)}>Cancel</Button>
<Button
  variant="primary"
  loading={updateAllTriggering}
  disabled={updateAllSelectedHostIds.size === 0}
  onclick={executeUpdateAll}
>Update {updateAllSelectedHostIds.size} host(s)</Button>
```

- [ ] **Step 9: Migrate "Edit Software" modal footer (lines ~1143–1148)**

Before:

```svelte
<button class="btn preset-tonal-surface" onclick={() => (editItem = false)}>Cancel</button>
<button class="btn preset-filled-primary-500" onclick={executeEdit} disabled={editSubmitting}>
  {editSubmitting ? 'Saving...' : 'Save'}
</button>
```

After:

```svelte
<Button variant="secondary" onclick={() => (editItem = false)}>Cancel</Button>
<Button variant="primary" loading={editSubmitting} onclick={executeEdit}>Save</Button>
```

- [ ] **Step 10: Compile check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npm run check 2>&1 | grep -i 'software/\[id\]'
```

Expected: no type errors on this file.

- [ ] **Step 11: Commit**

```bash
git add "frontend/src/routes/software/[id]/+page.svelte"
git commit -m "refactor(software): migrate [id]/+page.svelte button sites to Button primitive (#3f)"
```

---

## Task 3: Migrate `IgnoreRulesTab.svelte`

**Files:**

- Modify: `frontend/src/routes/software/IgnoreRulesTab.svelte`

Read the full file before editing (271 lines). There is no `isSaving` state and no "Saving…" ternary — the
modal "Create" button uses `disabled={!ignoreForm.name.trim()}` only. Do NOT add a loading state.

- [ ] **Step 1: Add `Button` import**

```ts
import Button from '$lib/components/Button.svelte';
```

- [ ] **Step 2: Migrate "Add Ignore Rule" header button (line ~145)**

Before:

```svelte
<button class="btn preset-filled-primary-500" onclick={openCreateIgnore}>Add Ignore Rule</button>
```

After:

```svelte
<Button variant="primary" size="sm" onclick={openCreateIgnore}>Add Ignore Rule</Button>
```

- [ ] **Step 3: Migrate per-row "Delete" button (lines ~196–200)**

Before:

```svelte
<button
  class="btn btn-sm preset-tonal-error"
  onclick={() => (ignoreDeleteConfirm = { id: ignore.id, name: ignore.name })}
>
  Delete
</button>
```

After:

```svelte
<Button
  variant="danger"
  size="sm"
  onclick={() => (ignoreDeleteConfirm = { id: ignore.id, name: ignore.name })}
>Delete</Button>
```

- [ ] **Step 4: Migrate modal footer Cancel and Create buttons (lines ~252–257)**

Before:

```svelte
<button class="btn preset-tonal-surface" onclick={closeIgnoreModal}>Cancel</button>
<button class="btn preset-filled-primary-500" onclick={saveIgnore} disabled={!ignoreForm.name.trim()}>
  Create
</button>
```

After:

```svelte
<Button variant="secondary" onclick={closeIgnoreModal}>Cancel</Button>
<Button variant="primary" disabled={!ignoreForm.name.trim()} onclick={saveIgnore}>Create</Button>
```

Regression guard: no `loading` prop, no `isSaving` state — intentional. The `disabled` guard on
`!ignoreForm.name.trim()` is preserved verbatim.

- [ ] **Step 5: Compile check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npm run check 2>&1 | grep -i 'IgnoreRulesTab'
```

- [ ] **Step 6: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit && git add frontend/src/routes/software/IgnoreRulesTab.svelte
git commit -m "refactor(software): migrate IgnoreRulesTab.svelte button sites to Button primitive (#3f)"
```

---

## Task 4: Migrate `SoftwareMergeWizard.svelte`

**Files:**

- Modify: `frontend/src/lib/components/SoftwareMergeWizard.svelte`

Read the full file before editing (466 lines). The wizard uses a `Modal` wrapper (not `ModalShell`). The
state variable for loading is `loading` (not `isSubmitting`). Step-2 action label is "Merge" (not "Finish").
Cancel must remain enabled throughout the submit window.

- [ ] **Step 1: Add `Button` import**

```ts
import Button from '$lib/components/Button.svelte';
```

- [ ] **Step 2: Migrate "Search" button in step-1 search panel (line ~270)**

Before:

```svelte
<button class="btn preset-tonal-surface" type="button" disabled={loading} onclick={runSearch}>
  Search
</button>
```

After:

```svelte
<Button variant="ghost" type="button" disabled={loading} onclick={runSearch}>Search</Button>
```

- [ ] **Step 3: Migrate "Add" per-result button (lines ~293–300)**

Before:

```svelte
<button
  class="btn btn-sm preset-tonal-surface"
  type="button"
  disabled={loading}
  aria-label={`Add ${candidate.name}`}
  onclick={() => addCandidate(candidate)}
>
  Add
</button>
```

After:

```svelte
<Button
  variant="ghost"
  size="sm"
  type="button"
  disabled={loading}
  ariaLabel={`Add ${candidate.name}`}
  onclick={() => addCandidate(candidate)}
>Add</Button>
```

- [ ] **Step 4: Migrate "Remove" per-candidate button (lines ~331–340)**

Before:

```svelte
<button
  class="btn btn-sm preset-tonal-surface"
  type="button"
  disabled={loading}
  aria-label={`Remove ${candidate.name}`}
  onclick={() => removeCandidate(candidate.id)}
>
  Remove
</button>
```

After:

```svelte
<Button
  variant="ghost"
  size="sm"
  type="button"
  disabled={loading}
  ariaLabel={`Remove ${candidate.name}`}
  onclick={() => removeCandidate(candidate.id)}
>Remove</Button>
```

- [ ] **Step 5: Migrate wizard footer buttons (lines ~427–448)**

Before:

```svelte
{#snippet footer()}
  <button class="btn preset-tonal-surface" onclick={onclose} disabled={loading}>Cancel</button>
  {#if step === 2}
    <button
      class="btn preset-tonal-surface"
      onclick={() => {
        showMergeConfirm = false;
        step = 1;
      }}
      disabled={loading}
    >
      Back
    </button>
  {/if}
  <button class="btn preset-filled-primary-500" onclick={step === 1 ? goToPreview : requestMerge} disabled={loading}>
    {#if step === 1}
      {loading ? 'Loading preview...' : 'Next'}
    {:else}
      {loading ? 'Merging...' : 'Merge'}
    {/if}
  </button>
{/snippet}
```

After:

```svelte
{#snippet footer()}
  <Button variant="ghost" onclick={onclose}>Cancel</Button>
  {#if step === 2}
    <Button
      variant="secondary"
      onclick={() => {
        showMergeConfirm = false;
        step = 1;
      }}
      disabled={loading}
    >Back</Button>
  {/if}
  {#if step === 1}
    <Button variant="primary" loading={loading} onclick={goToPreview}>Next</Button>
  {:else}
    <Button variant="primary" loading={loading} onclick={requestMerge}>Merge</Button>
  {/if}
{/snippet}
```

Key changes:

- Cancel: `variant="ghost"` with **no** `disabled={loading}` — remains enabled throughout so the user can
  always back out.
- Back: `variant="secondary"` with `disabled={loading}` — blocked while submitting. `loading` resets to
  `false` in the catch path of `goToPreview` / `merge`, so Back returns to enabled after a failed step.
- Next (step 1): `variant="primary" loading={loading}` — replaces `'Loading preview...'` ternary text.
- Merge (step 2): `variant="primary" loading={loading}` — replaces `'Merging...'` ternary text.

- [ ] **Step 6: Compile check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npm run check 2>&1 | grep -i 'SoftwareMergeWizard'
```

- [ ] **Step 7: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit && git add frontend/src/lib/components/SoftwareMergeWizard.svelte
git commit -m "refactor(software): migrate SoftwareMergeWizard.svelte nav buttons to Button primitive (#3f)"
```

---

## Task 5: Migrate `AddSoftwareModal.svelte`

**Files:**

- Modify: `frontend/src/lib/components/AddSoftwareModal.svelte`

Read the full file before editing (109 lines). The state variable is `submitting` (not `isSubmitting`).
Remove the `{submitting ? 'Registering...' : 'Register Software'}` ternary — spinner + static label is
the #2 §4.6 pattern.

- [ ] **Step 1: Add `Button` import**

```ts
import Button from '$lib/components/Button.svelte';
```

- [ ] **Step 2: Migrate modal footer (lines ~103–108)**

Before:

```svelte
<button class="btn preset-tonal-surface" onclick={onclose}>Cancel</button>
<button class="btn preset-filled-primary-500" disabled={submitting} onclick={submit}>
  {submitting ? 'Registering...' : 'Register Software'}
</button>
```

After:

```svelte
<Button variant="secondary" onclick={onclose}>Cancel</Button>
<Button variant="primary" loading={submitting} onclick={submit}>Register Software</Button>
```

The `{submitting ? 'Registering...' : 'Register Software'}` ternary is completely removed. The spinner is
rendered by the `loading` prop. Children stays as static text "Register Software" at all times.

- [ ] **Step 3: Compile check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npm run check 2>&1 | grep -i 'AddSoftwareModal'
```

- [ ] **Step 4: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit && git add frontend/src/lib/components/AddSoftwareModal.svelte
git commit -m "refactor(software): migrate AddSoftwareModal.svelte buttons to Button primitive (#3f)"
```

---

## Task 6: Extend unit tests

**Files:**

- Modify: `frontend/src/routes/software/software-trigger-status.test.ts`
- Modify: `frontend/src/routes/software/[id]/software-detail-update-trigger.test.ts`
- Modify: `frontend/src/routes/software/ignore-rules-tab.test.ts`
- Create: `frontend/src/lib/components/software-merge-wizard.test.ts`
- Create: `frontend/src/lib/components/add-software-modal.test.ts`

Read each existing test file in full before modifying. The test files define their own local type fixtures
and mock setup — mirror that pattern for new test files.

### 6a — software list tests

Read `frontend/src/routes/software/software-trigger-status.test.ts` to understand the mock setup and extend:

- [ ] **Step 1: Assert "Add Software" button renders `variant="primary" size="sm"`**

```ts
it('"Add Software" button renders with primary variant and sm size', async () => {
  // Use the existing admin user fixture and mock setup from the file.
  // Render the page with canManage=true (admin with CreateSoftware permission).
  const addBtn = screen.getByRole('button', { name: 'Add Software' });
  expect(addBtn.className).toContain('h-[19px]'); // size="sm"
  expect(addBtn.className).toContain('bg-[linear-gradient'); // variant="primary"
});
```

- [ ] **Step 2: Assert actions context-menu toggle renders `variant="ghost" size="sm"`**

```ts
it('row context-menu toggle renders ghost sm button', async () => {
  const actionsBtn = screen.getByRole('button', { name: /^Actions for /i });
  expect(actionsBtn.className).toContain('h-[19px]');
  expect(actionsBtn.className).toContain('bg-transparent'); // ghost
});
```

- [ ] **Step 3: Assert `<UpdateAllButton>` renders for multi-host items (not raw `<Button>`)**

```ts
it('header row aggregate trigger renders UpdateAllButton, not a raw Button', async () => {
  // Confirm the trigger renders UpdateAllButton — it uses rgba(var(--accent-rgb)...)
  // bg in idle state and does NOT emit aria-busy.
  const updateAllBtn = screen.getByRole('button', { name: /update all/i });
  expect(updateAllBtn).not.toHaveAttribute('aria-busy');
});
```

### 6b — software detail tests

Read `frontend/src/routes/software/[id]/software-detail-update-trigger.test.ts` in full, then extend:

**ActionBadge scope note:** The "Update Avail" status column renders `<ActionBadge>` (a specialised UI
component, not a `btn preset-*` element). `ActionBadge` is NOT in scope for this migration — it is not
a raw `<button class="btn ...">`. The click handler on `ActionBadge` calls `openUpdateModal(host)`,
which opens the Confirm Update modal whose footer buttons ARE migrated in steps below.

- [ ] **Step 4: Assert plugin-link buttons render ghost sm**

Plugin-link buttons in the host plugin table render as `variant="ghost" size="sm"`. The current source
uses `btn btn-sm preset-tonal-surface` (or similar) for these links.

```ts
it('plugin-link buttons render variant="ghost" size="sm"', async () => {
  // Use a host fixture with at least one plugin entry.
  // The plugin table renders one link-like button per plugin role.
  render(SoftwareDetailPage, { /* mock props */ });
  await waitFor(() =>
    expect(screen.getByRole('button', { name: /configure/i })).toBeInTheDocument()
  );
  const pluginBtn = screen.getByRole('button', { name: /configure/i });
  expect(pluginBtn.className).toContain('h-[19px]'); // size="sm"
  expect(pluginBtn.className).toContain('bg-transparent'); // ghost
});
```

Adjust the button label to match the actual plugin-link text in the source (e.g. "Configure Plugins"
or the role short name). Read `software/[id]/+page.svelte` to confirm the exact label before writing
this assertion.

- [ ] **Step 5: Assert Confirm Update modal footer uses Button primitives**

```ts
it('Confirm Update modal Trigger Update renders primary loading during submit', async () => {
  vi.mocked(api.triggerSoftwareUpdate).mockReturnValue(new Promise(() => {}));
  // ... render + open confirm modal via ActionBadge click as per existing test pattern
  const triggerBtn = screen.getByRole('button', { name: 'Trigger Update' });
  expect(triggerBtn).not.toHaveAttribute('aria-busy');
  await fireEvent.click(triggerBtn);
  await waitFor(() => expect(triggerBtn).toHaveAttribute('aria-busy', 'true'));
});
```

- [ ] **Step 6: Assert header "Delete" button renders `variant="danger"`**

```ts
it('Delete header button renders danger variant', async () => {
  const deleteBtn = screen.getByRole('button', { name: 'Delete' });
  expect(deleteBtn.className).toContain('var(--color-error-bg)');
});
```

### 6c — IgnoreRulesTab tests

Read `frontend/src/routes/software/ignore-rules-tab.test.ts` in full, then extend:

- [ ] **Step 6: Assert "Add Ignore Rule" renders `primary size="sm"`**

```ts
it('"Add Ignore Rule" renders with primary variant and sm size', async () => {
  render(IgnoreRulesTab);
  await waitFor(() => expect(screen.getByText('Plex')).toBeInTheDocument());
  const addBtn = screen.getByRole('button', { name: 'Add Ignore Rule' });
  expect(addBtn.className).toContain('h-[19px]');
  expect(addBtn.className).toContain('bg-[linear-gradient');
});
```

- [ ] **Step 7: Assert per-row "Delete" renders `danger size="sm"`**

```ts
it('per-row Delete button renders danger variant and sm size', async () => {
  render(IgnoreRulesTab);
  await waitFor(() => expect(screen.getByText('Plex')).toBeInTheDocument());
  const deleteBtn = screen.getByRole('button', { name: 'Delete' });
  expect(deleteBtn.className).toContain('h-[19px]');
  expect(deleteBtn.className).toContain('var(--color-error-bg)');
});
```

- [ ] **Step 8: Assert modal "Create" is disabled when name empty, no loading state**

```ts
it('modal Create button is disabled when name field is empty', async () => {
  render(IgnoreRulesTab);
  await waitFor(() => expect(screen.getByText('Plex')).toBeInTheDocument());
  await fireEvent.click(screen.getByRole('button', { name: 'Add Ignore Rule' }));
  const createBtn = await screen.findByRole('button', { name: 'Create' });
  expect(createBtn).toBeDisabled();
  expect(createBtn).not.toHaveAttribute('aria-busy'); // no loading state
});

it('modal Create button is enabled when name field has content', async () => {
  render(IgnoreRulesTab);
  await waitFor(() => expect(screen.getByText('Plex')).toBeInTheDocument());
  await fireEvent.click(screen.getByRole('button', { name: 'Add Ignore Rule' }));
  await fireEvent.input(screen.getByPlaceholderText(/FreshRSS/i), { target: { value: 'SomeApp' } });
  const createBtn = screen.getByRole('button', { name: 'Create' });
  expect(createBtn).not.toBeDisabled();
});
```

### 6d — SoftwareMergeWizard tests

Create `frontend/src/lib/components/software-merge-wizard.test.ts`.

- [ ] **Step 9: Create the test file with boilerplate**

```ts
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';

// Local type definitions (per .test.ts convention)
type Candidate = { id: string; name: string; host_count: number; plugins: string[] };

vi.mock('$lib/notifications.svelte', () => ({
  showError: vi.fn(),
  showSuccess: vi.fn()
}));

import SoftwareMergeWizard from './SoftwareMergeWizard.svelte';
import * as notifications from '$lib/notifications.svelte';

const candidateA: Candidate = { id: 'a', name: 'Firefox', host_count: 3, plugins: ['apt'] };
const candidateB: Candidate = { id: 'b', name: 'Firefox ESR', host_count: 1, plugins: ['apt'] };

function makeProps(overrides = {}) {
  return {
    candidates: [candidateA, candidateB],
    seedItemId: 'a',
    searchCandidates: null,
    initialSearchQuery: '',
    onclose: vi.fn(),
    onsuccess: vi.fn(),
    previewMerge: vi.fn().mockResolvedValue({
      candidate_count: 2,
      moved_link_count: 1,
      skipped_duplicate_link_count: 0,
      survivor: { id: 'a', name: 'Firefox', host_count: 3 },
      losers: [{ id: 'b', name: 'Firefox ESR', host_count: 1 }],
      moved_links: [],
      skipped_duplicate_links: []
    }),
    executeMerge: vi.fn().mockResolvedValue({ merged_item_id: 'a' }),
    ...overrides
  };
}
```

- [ ] **Step 10: Back renders secondary, Next/Merge renders primary with loading**

```ts
it('step-1 Next button renders primary variant', async () => {
  render(SoftwareMergeWizard, makeProps());
  const nextBtn = screen.getByRole('button', { name: 'Next' });
  expect(nextBtn.className).toContain('bg-[linear-gradient');
  expect(nextBtn).not.toHaveAttribute('aria-busy');
});

it('step-2 Back button renders secondary variant', async () => {
  render(SoftwareMergeWizard, makeProps());
  await fireEvent.click(screen.getByRole('button', { name: 'Next' }));
  const backBtn = await screen.findByRole('button', { name: 'Back' });
  expect(backBtn.className).toContain('var(--bg-raised)'); // secondary
});

it('step-2 Merge button renders primary variant', async () => {
  render(SoftwareMergeWizard, makeProps());
  await fireEvent.click(screen.getByRole('button', { name: 'Next' }));
  const mergeBtn = await screen.findByRole('button', { name: 'Merge' });
  expect(mergeBtn.className).toContain('bg-[linear-gradient');
});
```

- [ ] **Step 11: Cancel stays enabled during submit**

```ts
it('Cancel button remains enabled during merge submit', async () => {
  const executeMerge = vi.fn().mockReturnValue(new Promise(() => {}));
  render(SoftwareMergeWizard, makeProps({ executeMerge }));

  await fireEvent.click(screen.getByRole('button', { name: 'Next' }));
  const mergeBtn = await screen.findByRole('button', { name: 'Merge' });
  await fireEvent.click(mergeBtn);

  // Confirm in the ConfirmDialog that surfaces after clicking Merge
  const confirmMergeBtn = await screen.findByRole('button', { name: 'Merge Items' });
  await fireEvent.click(confirmMergeBtn);

  await waitFor(() => expect(mergeBtn).toHaveAttribute('aria-busy', 'true'));

  const cancelBtn = screen.getByRole('button', { name: 'Cancel' });
  expect(cancelBtn).not.toBeDisabled();
  expect(cancelBtn).not.toHaveAttribute('aria-busy');
});
```

- [ ] **Step 12: On step-submit error, loading resets**

```ts
it('Next loading resets to false on preview error, showError called', async () => {
  const previewMerge = vi.fn().mockRejectedValue(new Error('Preview failed'));
  render(SoftwareMergeWizard, makeProps({ previewMerge }));

  await fireEvent.click(screen.getByRole('button', { name: 'Next' }));
  await waitFor(() =>
    expect(vi.mocked(notifications.showError)).toHaveBeenCalledWith('Preview failed')
  );

  const nextBtn = screen.getByRole('button', { name: 'Next' });
  expect(nextBtn).not.toHaveAttribute('aria-busy');
});
```

### 6e — AddSoftwareModal tests

Create `frontend/src/lib/components/add-software-modal.test.ts`.

- [ ] **Step 13: Create the test file with boilerplate**

```ts
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';

vi.mock('$lib/api', () => ({ createSoftwareItem: vi.fn() }));
vi.mock('$lib/notifications.svelte', () => ({ showSuccess: vi.fn(), showError: vi.fn() }));
vi.mock('$lib/utils', () => ({
  isValidLogoUrl: vi.fn((url: string) => url.startsWith('https://'))
}));

import AddSoftwareModal from './AddSoftwareModal.svelte';
import * as api from '$lib/api';
```

- [ ] **Step 14: Assert Register Software renders primary with loading, no text-swap**

```ts
it('"Register Software" button renders primary variant', () => {
  render(AddSoftwareModal, { onclose: vi.fn(), onsuccess: vi.fn() });
  const submitBtn = screen.getByRole('button', { name: 'Register Software' });
  expect(submitBtn.className).toContain('bg-[linear-gradient');
  expect(submitBtn).not.toHaveAttribute('aria-busy');
});

it('shows aria-busy and no "Registering..." text during submit', async () => {
  vi.mocked(api.createSoftwareItem).mockReturnValue(new Promise(() => {}));
  render(AddSoftwareModal, { onclose: vi.fn(), onsuccess: vi.fn() });

  await fireEvent.input(screen.getByRole('textbox', { name: /name/i }), {
    target: { value: 'Firefox' }
  });
  const submitBtn = screen.getByRole('button', { name: 'Register Software' });
  await fireEvent.click(submitBtn);

  await waitFor(() => expect(submitBtn).toHaveAttribute('aria-busy', 'true'));
  expect(document.body.textContent).not.toContain('Registering...');
  expect(submitBtn.textContent?.trim()).toContain('Register Software');
});

it('Cancel renders secondary variant', () => {
  render(AddSoftwareModal, { onclose: vi.fn(), onsuccess: vi.fn() });
  const cancelBtn = screen.getByRole('button', { name: 'Cancel' });
  expect(cancelBtn.className).toContain('var(--bg-raised)'); // secondary
});
```

- [ ] **Step 15: Run all software-area unit tests**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run \
  src/routes/software \
  src/lib/components/software-merge-wizard.test.ts \
  src/lib/components/add-software-modal.test.ts
```

Expected: all tests pass.

- [ ] **Step 16: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit && git add \
  frontend/src/routes/software/software-trigger-status.test.ts \
  frontend/src/routes/software/ignore-rules-tab.test.ts \
  "frontend/src/routes/software/[id]/software-detail-update-trigger.test.ts" \
  frontend/src/lib/components/software-merge-wizard.test.ts \
  frontend/src/lib/components/add-software-modal.test.ts
git commit -m "test(software): add Button primitive contract tests for #3f migration"
```

---

## Task 7: Re-baseline Playwright snapshots

**Files:**

- Create or modify: `frontend/tests/e2e/software-area.spec.ts`

Read `frontend/tests/e2e/button-primitive.spec.ts` for the `mockAuthApi` + `setTheme` helper patterns to copy.

- [ ] **Step 1: Check for existing software e2e spec**

```bash
ls /Users/andreyyantsen/Development/uptrakit/frontend/tests/e2e/
```

If no `software*` spec exists, create `frontend/tests/e2e/software-area.spec.ts`.

- [ ] **Step 2: Wire up mock API handlers**

Extend the `mockAuthApi` pattern from `button-primitive.spec.ts` to also mock:

- `GET /api/v1/software` → paginated list of 1–2 software items
- `GET /api/v1/software/:id` → software detail with 2 hosts (one with `update_available=true`)
- `GET /api/v1/plugin-types` → empty array
- `GET /api/v1/software/ignores` → empty page
- Mock user must have all software-area permissions: `ViewSoftware`, `CreateSoftware`, `UpdateSoftware`,
  `DeleteSoftware`, `TriggerChecks`, `TriggerUpdates`, `ManageIgnores`

- [ ] **Step 3: Define snapshot targets**

```ts
const SNAPSHOTS = [
  { name: 'software-list-dark', route: '/software?tab=all', theme: 'dark' as const },
  { name: 'software-list-light', route: '/software?tab=all', theme: 'light' as const },
  { name: 'software-ignores-dark', route: '/software?tab=ignores', theme: 'dark' as const },
  { name: 'software-ignores-light', route: '/software?tab=ignores', theme: 'light' as const },
  { name: 'software-detail-dark', route: '/software/test-item-id', theme: 'dark' as const },
  { name: 'software-detail-light', route: '/software/test-item-id', theme: 'light' as const }
];
```

- [ ] **Step 4: Mask dynamic content per spec §3**

Apply masks for:

- All `[aria-busy="true"]` spinners (prefer no in-flight requests during snapshot capture)
- Version digest strings: `page.locator('td.font-mono')` or similar
- Relative timestamps
- Toast banners

Per parent §3, total masked area stays under 15% per snapshot.

- [ ] **Step 5: Generate baselines**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx playwright test tests/e2e/software-area.spec.ts --update-snapshots
```

- [ ] **Step 6: Re-run to confirm stability**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx playwright test tests/e2e/software-area.spec.ts
```

Expected: all pass with 0 failures.

- [ ] **Step 7: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit && \
  git add frontend/tests/e2e/software-area.spec.ts \
  "frontend/tests/e2e/software-area.spec.ts-snapshots"
git commit -m "test(e2e): add/re-baseline software-area snapshots after Button primitive migration (#3f)"
```

---

## Task 8: Full frontend gate

- [ ] **Step 1: Run full gate**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && \
  npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

Expected: every command exits 0.

- [ ] **Step 2: Run full e2e suite**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx playwright test 2>&1 | tail -20
```

Expected: 0 failures. Existing snapshot suites (public-entry, button-primitive, form-primitive) are unaffected.

---

## Commit summary

| # | Commit message | Files |
| --- | --- | --- |
| 1 | `refactor(software): migrate +page.svelte button sites to Button primitive (#3f)` | `software/+page.svelte` |
| 2 | `refactor(software): migrate [id]/+page.svelte button sites to Button primitive (#3f)` | `software/[id]/+page.svelte` |
| 3 | `refactor(software): migrate IgnoreRulesTab.svelte button sites to Button primitive (#3f)` | `IgnoreRulesTab.svelte` |
| 4 | `refactor(software): migrate SoftwareMergeWizard.svelte nav buttons to Button primitive (#3f)` | `SoftwareMergeWizard.svelte` |
| 5 | `refactor(software): migrate AddSoftwareModal.svelte buttons to Button primitive (#3f)` | `AddSoftwareModal.svelte` |
| 6 | `test(software): add Button primitive contract tests for #3f migration` | test files |
| 7 | `test(e2e): add/re-baseline software-area snapshots after Button primitive migration (#3f)` | e2e spec + PNGs |
