# Hosts Button Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate all non-shared-component button elements in `hosts/+page.svelte` and `hosts/[id]/+page.svelte` to the `<Button>` primitive, using
`variant="secondary"` for reversible actions and the inline-snippet pattern for the context-menu trigger.

**Architecture:** Template-level swap only. `BatchActionBar`, `ContextMenuShell`, and `ConfirmDialog` internals are out of scope —
those migrate in sub-spec \#3k. Loading prop eliminates `{flag ? 'Saving…' : 'Save'}` ternaries. Context-menu trigger uses
`{#snippet moreIcon()}` + `<span aria-hidden>⋮</span>` — no icon import needed.

**Tech Stack:** Svelte 5, TypeScript, @testing-library/svelte, Playwright

---

## Dependencies

> **Blocks on sub-spec #2c being merged first.** `variant="secondary"` and `ariaLabel` prop on `<Button>` are added by that sub-spec. If implementing
> without #2c merged, stub every `variant="secondary"` as `variant="ghost"` and pass `ariaLabel` as a plain attribute temporarily; update after #2c
> lands.

Also blocks on sub-spec #2 (Button + UpdateAllButton primitives).

---

## Migration rules (quick reference)

| Legacy class | `variant` |
| --- | --- |
| `preset-filled-primary-500` | `primary` |
| `preset-filled-error-500` / `preset-tonal-error` | `danger` |
| `preset-tonal-surface` | `secondary` |
| `preset-tonal` (non-surface) | `ghost` |
| `btn-sm` | `size="sm"` |

Loading: replace `{flag ? 'Saving…' : 'Save'}` ternary children with `loading={flag}` + static children. Button sets `disabled` internally when
`loading=true`; no separate `disabled={flag}` needed.

Href branch: `<a href="..." class="btn btn-sm preset-tonal">View</a>` → `<Button href="..." variant="ghost" size="sm">View</Button>`.

---

## Out of scope (leave unchanged)

- `<BatchActionBar>` and its internal buttons
- `<ContextMenuShell>` and menu item `<button>` elements inside it
- `<ConfirmDialog>` internals (Cancel/confirm inside the dialog component itself)
- `confirmLabel` / `confirmClass` props passed to `<ConfirmDialog>` — these stay as-is until #3k

---

## File structure

| Path | Change |
| --- | --- |
| `frontend/src/routes/hosts/+page.svelte` | Add `Button` import; swap 3 migration sites |
| `frontend/src/routes/hosts/[id]/+page.svelte` | Add `Button` import; swap 11 migration sites |
| hosts unit test file(s) | Extend with assertions for migrated buttons |
| hosts e2e spec file(s) | Re-baseline snapshots |

> **Before editing:** Read both route files to confirm exact line numbers and surrounding context. The line numbers listed below are approximate from
> the spec — verify against the live files.

---

## Task 1: Read the source files

Before writing any code, read these files:

- [ ] `frontend/src/routes/hosts/+page.svelte`
- [ ] `frontend/src/routes/hosts/[id]/+page.svelte`

Find the test files:

```bash
ls frontend/src/routes/hosts/
ls frontend/tests/
```

- [ ] Read the hosts unit test file(s) to understand existing mock patterns, fixtures, and assertion style.
- [ ] Read the hosts e2e spec file to understand mock session / route setup.

---

## Task 2: Migrate hosts/+page.svelte (3 sites)

**Files:**

- Modify: `frontend/src/routes/hosts/+page.svelte`

Three migration sites in this file:

1. Per-row context-menu trigger `<button class="btn btn-sm preset-tonal">⋮</button>` (≈line 468)
2. Error-state Retry button (≈line 484)
3. Edit Host Name modal footer Cancel + Save (≈lines 583–586)

- [ ] **Step 1: Add `Button` import**

In the `<script lang="ts">` block, add:

```ts
import Button from '$lib/components/Button.svelte';
```

- [ ] **Step 2: Migrate site 1 — context-menu trigger**

The `{#snippet moreIcon()}` must be declared inside the row-level `{#each}` loop (or `{#snippet row(...)}` block if the file uses snippet rows) —
directly before the `<Button>` that uses it. This gives each row its own snippet binding.

Before (approximate):

```svelte
<button
  class="btn btn-sm preset-tonal"
  aria-label="Actions for {host.friendly_name}"
  onclick={(e) => { e.stopPropagation(); toggleMenu(host.id, e.currentTarget); }}
>
  ⋮
</button>
```

After:

```svelte
{#snippet moreIcon()}<span aria-hidden="true" class="leading-none">⋮</span>{/snippet}
<Button
  variant="ghost"
  size="sm"
  leadingIcon={moreIcon}
  ariaLabel="Actions for {host.friendly_name}"
  onclick={(e) => { e.stopPropagation(); toggleMenu(host.id, e.currentTarget); }}
></Button>
```

> **`children` is required.** `Button.svelte` declares `children: Snippet` (not optional). A self-closing tag `<Button />` produces no children
> snippet and TypeScript will error. Use a non-self-closing empty tag `<Button ...></Button>` — Svelte 5 compiles empty tags into an empty (no-op)
> children snippet that satisfies the `Snippet` type. If the TypeScript compiler still objects, add `<span class="sr-only">Actions for
> {host.friendly_name}</span>` as children; the `ariaLabel` prop provides the accessible name and the sr-only span is harmless.

Note: `ariaLabel` prop requires sub-spec #2c. If #2c not yet merged, use `aria-label="Actions for {host.friendly_name}"` as a plain attribute
temporarily (Button does not yet accept it but the HTML attribute still renders).

- [ ] **Step 3: Migrate site 2 — error-state Retry**

Before (inside `{#snippet errorActions()}` or error container):

```svelte
<button class="btn preset-filled-primary-500 ..." onclick={() => loadHosts(currentPage)}>Retry</button>
```

After:

```svelte
<Button variant="primary" onclick={() => loadHosts(currentPage)}>Retry</Button>
```

Preserve any `class` utility (e.g. `mt-3`) via the `class` prop if present in the original.

- [ ] **Step 4: Migrate site 3 — Edit Host Name modal footer**

Before (in the modal's `{#snippet footer()}`):

```svelte
<button class="btn preset-tonal-surface" onclick={cancelEdit}>Cancel</button>
<button class="btn preset-filled-primary-500" disabled={submitting} onclick={executeEdit}>
  {submitting ? 'Saving...' : 'Save'}
</button>
```

After:

```svelte
<Button variant="secondary" onclick={cancelEdit}>Cancel</Button>
<Button variant="primary" loading={submitting} onclick={executeEdit}>Save</Button>
```

- [ ] **Step 5: Verify compilation**

```bash
cd frontend && npm run check 2>&1 | grep -i 'hosts/\+page'
```

Expected: no errors. If `ariaLabel` or `variant="secondary"` produce type errors, sub-spec #2c has not landed — stub per the dependency note.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/routes/hosts/+page.svelte
git commit -m "feat(ui): migrate hosts/+page.svelte buttons to Button primitive (#3h)"
```

---

## Task 3: Migrate hosts/[id]/+page.svelte (11 sites)

**Files:**

- Modify: `frontend/src/routes/hosts/[id]/+page.svelte`

11 migration sites in this file:

1. Error-state Retry (≈line 398)
2. Header Edit Name (≈line 410)
3. Header Deactivate launcher (≈lines 411–414)
4. Header Trigger Discovery (≈lines 420–422)
5. Set Tags launcher (≈line 487)
6. Assigned Software row "View" link (≈line 581)
7. Discovery Allowlist "Add Plugin Type" (≈line 614)
8. Discovery Allowlist row Remove launcher (≈lines 646–649)
9. Edit Host Name modal footer Cancel + Save (≈lines 729–733)
10. Add Discovery Plugin Type modal footer Cancel + Add (≈lines 754–759)
11. Set Tags modal footer Cancel + Save (≈lines 777–780)

- [ ] **Step 1: Add `Button` import**

```ts
import Button from '$lib/components/Button.svelte';
```

- [ ] **Step 2: Migrate site 1 — error-state Retry (≈line 398)**

Before:

```svelte
<button class="btn preset-filled-primary-500 mt-2" onclick={() => loadData()}>Retry</button>
```

After:

```svelte
<Button variant="primary" class="mt-2" onclick={() => loadData()}>Retry</Button>
```

- [ ] **Step 3: Migrate sites 2–4 — header cluster (≈lines 409–423)**

Before (approximate):

```svelte
{#if canManage}
  <button class="btn preset-tonal-surface" onclick={openEditDialog}>Edit Name</button>
  <button class="btn preset-filled-error-500" onclick={() => (confirmDeactivate = true)} disabled={submitting}>
    Deactivate
  </button>
{/if}
{#if canManageSoftware}
  <button class="btn preset-tonal-surface" onclick={triggerDiscovery} disabled={discovering}>
    {discovering ? 'Triggering…' : 'Trigger Discovery'}
  </button>
{/if}
```

After:

```svelte
{#if canManage}
  <Button variant="secondary" onclick={openEditDialog}>Edit Name</Button>
  <Button variant="danger" disabled={submitting} onclick={() => (confirmDeactivate = true)}>Deactivate</Button>
{/if}
{#if canManageSoftware}
  <Button variant="secondary" loading={discovering} onclick={triggerDiscovery}>
    Trigger Discovery
  </Button>
{/if}
```

`loading={discovering}` replaces both `disabled={discovering}` and the `{discovering ? 'Triggering…' : ...}` ternary.

- [ ] **Step 4: Migrate site 5 — Set Tags launcher (≈line 487)**

Before:

```svelte
<button class="btn btn-sm preset-tonal-surface" onclick={openSetTagsModal}>Set Tags</button>
```

After:

```svelte
<Button variant="secondary" size="sm" onclick={openSetTagsModal}>Set Tags</Button>
```

- [ ] **Step 5: Migrate site 6 — Assigned Software "View" link (≈line 581)**

Before:

```svelte
<a href="/software/{item.id}" class="btn btn-sm preset-tonal">View</a>
```

After:

```svelte
<Button href="/software/{item.id}" variant="ghost" size="sm">View</Button>
```

- [ ] **Step 6: Migrate site 7 — Allowlist "Add Plugin Type" (≈line 614)**

Before:

```svelte
<button class="btn btn-sm preset-filled-primary-500" onclick={openAddAllowlistEntry}>
  Add Plugin Type
</button>
```

After:

```svelte
<Button variant="primary" size="sm" onclick={openAddAllowlistEntry}>Add Plugin Type</Button>
```

- [ ] **Step 7: Migrate site 8 — Allowlist row Remove launcher (≈lines 646–649)**

Before:

```svelte
<button
  class="btn btn-sm preset-tonal-error"
  onclick={() => (allowlistDeleteConfirm = { id: entry.id, plugin_type: entry.plugin_type })}
>
  Remove
</button>
```

After:

```svelte
<Button
  variant="danger"
  size="sm"
  onclick={() => (allowlistDeleteConfirm = { id: entry.id, plugin_type: entry.plugin_type })}
>
  Remove
</Button>
```

- [ ] **Step 8: Migrate site 9 — Edit Host Name modal footer (≈lines 729–733)**

Before:

```svelte
{#snippet footer()}
  <button class="btn preset-tonal-surface" onclick={cancelEdit}>Cancel</button>
  <button class="btn preset-filled-primary-500" disabled={submitting} onclick={executeEdit}>
    {submitting ? 'Saving...' : 'Save'}
  </button>
{/snippet}
```

After:

```svelte
{#snippet footer()}
  <Button variant="secondary" onclick={cancelEdit}>Cancel</Button>
  <Button variant="primary" loading={submitting} onclick={executeEdit}>Save</Button>
{/snippet}
```

- [ ] **Step 9: Migrate site 10 — Add Discovery Plugin Type modal footer (≈lines 754–759)**

The action button here is labeled "Add", not "Save". It is guarded by `disabled={!allowlistForm.plugin_type.trim()}` — no loading state.

Before:

```svelte
{#snippet footer()}
  <button class="btn preset-tonal-surface" onclick={closeAllowlistModal}>Cancel</button>
  <button class="btn preset-filled-primary-500" disabled={!allowlistForm.plugin_type.trim()} onclick={saveAllowlistEntry}>
    Add
  </button>
{/snippet}
```

After:

```svelte
{#snippet footer()}
  <Button variant="secondary" onclick={closeAllowlistModal}>Cancel</Button>
  <Button variant="primary" disabled={!allowlistForm.plugin_type.trim()} onclick={saveAllowlistEntry}>
    Add
  </Button>
{/snippet}
```

- [ ] **Step 10: Migrate site 11 — Set Tags modal footer (≈lines 777–780)**

Before:

```svelte
{#snippet footer()}
  <button class="btn preset-tonal-surface" onclick={() => (showSetTagsModal = false)}>Cancel</button>
  <button class="btn preset-filled-primary-500" disabled={submitting} onclick={executeSetTags}>
    {submitting ? 'Saving...' : 'Save'}
  </button>
{/snippet}
```

After:

```svelte
{#snippet footer()}
  <Button variant="secondary" onclick={() => (showSetTagsModal = false)}>Cancel</Button>
  <Button variant="primary" loading={submitting} onclick={executeSetTags}>Save</Button>
{/snippet}
```

- [ ] **Step 11: Verify compilation**

```bash
cd frontend && npm run check 2>&1 | grep -i 'hosts/\[id\]'
```

Expected: no errors.

- [ ] **Step 12: Commit**

```bash
git add "frontend/src/routes/hosts/[id]/+page.svelte"
git commit -m "feat(ui): migrate hosts/[id]/+page.svelte buttons to Button primitive (#3h)"
```

---

## Task 4: Extend hosts unit tests

**Files:**

- Modify: hosts unit test file(s) (locate by reading from Task 1)

Read the existing test files to understand: import aliases, mock setup, fixture objects (host name, IDs), how state is triggered (fireEvent, prop
injection, etc.), and how components are rendered. Add the tests below using the same patterns.

- [ ] **Step 1: Write tests for hosts/+page.svelte migrations**

Add to the hosts list-page test file:

```ts
it('context-menu trigger renders as ghost-variant sm Button with aria-label containing host name', async () => {
  vi.mocked(api.getHosts).mockResolvedValue(makePage([sampleHost]));
  render(HostsPage);
  await waitFor(() => expect(screen.getByText('Production Server')).toBeInTheDocument());

  const trigger = screen.getByRole('button', { name: /actions for production server/i });
  // ghost variant: bg-transparent is unique to ghost (secondary/danger don't have it)
  expect(trigger.className).toContain('bg-transparent');
  expect(trigger.className).toContain('border-[var(--border-default)]');
  // sm size class fragment:
  expect(trigger.className).toContain('h-[19px]');
  // aria-hidden glyph in leadingIcon slot:
  const glyph = trigger.querySelector('span[aria-hidden="true"]');
  expect(glyph).not.toBeNull();
  expect(glyph!.textContent).toContain('⋮');
});

it('error-state Retry button renders as primary variant', async () => {
  vi.mocked(api.getHosts).mockRejectedValue(new Error('Server unavailable'));
  render(HostsPage);
  await waitFor(() => expect(screen.getByText('Server unavailable')).toBeInTheDocument());

  const retry = screen.getByRole('button', { name: /retry/i });
  expect(retry.className).toContain('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
});

it('Edit Host Name modal Save shows aria-busy while submitting and children stay static "Save"', async () => {
  vi.mocked(api.getHosts).mockResolvedValue(makePage([sampleHost]));
  vi.mocked(api.updateHost).mockReturnValue(new Promise(() => {})); // never resolves
  render(HostsPage);
  await waitFor(() => expect(screen.getByText('Production Server')).toBeInTheDocument());

  // Open context menu → click Edit Name
  fireEvent.click(screen.getByRole('button', { name: /actions for production server/i }));
  await waitFor(() => expect(screen.getByRole('menuitem', { name: 'Edit Name' })).toBeInTheDocument());
  fireEvent.click(screen.getByRole('menuitem', { name: 'Edit Name' }));

  // Modal opens — type a new name to enable Save, then click
  await waitFor(() => expect(screen.getByRole('dialog')).toBeInTheDocument());
  const nameInput = screen.getByRole('textbox');
  fireEvent.input(nameInput, { target: { value: 'New Name' } });
  const saveBtn = screen.getByRole('button', { name: /^save$/i });
  await fireEvent.click(saveBtn);

  await waitFor(() => expect(saveBtn).toHaveAttribute('aria-busy', 'true'));
  expect(saveBtn.textContent).toContain('Save');
  expect(saveBtn.textContent).not.toContain('Saving');
});
```

- [ ] **Step 2: Write tests for hosts/[id]/+page.svelte migrations**

Add to the host detail test file:

```ts
// Read the host-detail test file (found in Task 1) to get the sampleHost fixture,
// vi.mock() setup, and render(HostDetailPage, { params: { id: sampleHost.id } }) pattern.
// The tests below use the same mock/fixture conventions — fill in the exact import
// of HostDetailPage, the mock for getHost/updateHost/triggerHostDiscovery/setHostTags,
// and any beforeEach wiring as shown in that file.

it('error-state Retry button renders as primary variant', async () => {
  vi.mocked(api.getHost).mockRejectedValue(new Error('Not found'));
  render(HostDetailPage, { params: { id: 'host-001' } });
  await waitFor(() => expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument());

  const retry = screen.getByRole('button', { name: /retry/i });
  expect(retry.className).toContain('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
});

it('header Deactivate button renders as danger variant', async () => {
  vi.mocked(api.getHost).mockResolvedValue(sampleHost);
  render(HostDetailPage, { params: { id: sampleHost.id } });
  await waitFor(() => expect(screen.getByText(sampleHost.friendly_name)).toBeInTheDocument());

  const btn = screen.getByRole('button', { name: /deactivate/i });
  expect(btn.className).toContain('text-[var(--color-error)]');
});

it('header Edit Name button renders as secondary variant', async () => {
  vi.mocked(api.getHost).mockResolvedValue(sampleHost);
  render(HostDetailPage, { params: { id: sampleHost.id } });
  await waitFor(() => expect(screen.getByText(sampleHost.friendly_name)).toBeInTheDocument());

  const btn = screen.getByRole('button', { name: /edit name/i });
  expect(btn.className).toContain('bg-[var(--bg-raised)]'); // secondary
  expect(btn.className).toContain('border-[var(--border-default)]');
});

it('Trigger Discovery shows aria-busy while discovering and children stay static', async () => {
  vi.mocked(api.getHost).mockResolvedValue(sampleHost);
  vi.mocked(api.triggerHostDiscovery).mockReturnValue(new Promise(() => {})); // never resolves
  render(HostDetailPage, { params: { id: sampleHost.id } });
  await waitFor(() => expect(screen.getByText(sampleHost.friendly_name)).toBeInTheDocument());

  const btn = screen.getByRole('button', { name: /trigger discovery/i });
  await fireEvent.click(btn);
  await waitFor(() => expect(btn).toHaveAttribute('aria-busy', 'true'));
  expect(btn.textContent).toContain('Trigger Discovery');
  expect(btn.textContent).not.toContain('Triggering');
});

it('Set Tags launcher renders as secondary variant with sm size', async () => {
  vi.mocked(api.getHost).mockResolvedValue(sampleHost);
  render(HostDetailPage, { params: { id: sampleHost.id } });
  await waitFor(() => expect(screen.getByText(sampleHost.friendly_name)).toBeInTheDocument());

  const btn = screen.getByRole('button', { name: /set tags/i });
  expect(btn.className).toContain('bg-[var(--bg-raised)]'); // secondary
  expect(btn.className).toContain('h-[19px]'); // sm
});

it('Allowlist Add Plugin Type renders as primary variant with sm size', async () => {
  vi.mocked(api.getHost).mockResolvedValue(sampleHost);
  render(HostDetailPage, { params: { id: sampleHost.id } });
  await waitFor(() => expect(screen.getByText(sampleHost.friendly_name)).toBeInTheDocument());

  const btn = screen.getByRole('button', { name: /add plugin type/i });
  expect(btn.className).toContain('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]'); // primary
  expect(btn.className).toContain('h-[19px]'); // sm
});

it('Allowlist row Remove renders as danger variant with sm size', async () => {
  vi.mocked(api.getHost).mockResolvedValue(sampleHostWithAllowlist); // fixture with at least one allowlist entry
  render(HostDetailPage, { params: { id: sampleHost.id } });
  await waitFor(() => expect(screen.getByText(sampleHost.friendly_name)).toBeInTheDocument());

  const btn = screen.getByRole('button', { name: /remove/i });
  expect(btn.className).toContain('text-[var(--color-error)]'); // danger
  expect(btn.className).toContain('h-[19px]'); // sm
});

it('Assigned Software "View" link renders as <a> with ghost variant and sm size', async () => {
  vi.mocked(api.getHost).mockResolvedValue(sampleHostWithSoftware); // fixture with software item id
  render(HostDetailPage, { params: { id: sampleHost.id } });
  await waitFor(() => expect(screen.getByText(sampleHost.friendly_name)).toBeInTheDocument());

  // Button href branch renders <a role="button"> — use CSS selector for href:
  const link = document.querySelector('a[href^="/software/"]') as HTMLElement;
  expect(link).not.toBeNull();
  expect(link.className).toContain('bg-transparent'); // ghost (unique to ghost)
  expect(link.className).toContain('h-[19px]'); // sm
});

it('Edit Host Name modal Save shows aria-busy while submitting and children stay static "Save"', async () => {
  vi.mocked(api.getHost).mockResolvedValue(sampleHost);
  vi.mocked(api.updateHost).mockReturnValue(new Promise(() => {})); // never resolves
  render(HostDetailPage, { params: { id: sampleHost.id } });
  await waitFor(() => expect(screen.getByText(sampleHost.friendly_name)).toBeInTheDocument());

  fireEvent.click(screen.getByRole('button', { name: /edit name/i }));
  await waitFor(() => expect(screen.getByRole('dialog')).toBeInTheDocument());
  const nameInput = screen.getByRole('textbox');
  fireEvent.input(nameInput, { target: { value: 'Updated Name' } });

  const saveBtn = screen.getByRole('button', { name: /^save$/i });
  await fireEvent.click(saveBtn);
  await waitFor(() => expect(saveBtn).toHaveAttribute('aria-busy', 'true'));
  expect(saveBtn.textContent).toContain('Save');
  expect(saveBtn.textContent).not.toContain('Saving');
});

it('Set Tags modal Save shows aria-busy while submitting and children stay static "Save"', async () => {
  vi.mocked(api.getHost).mockResolvedValue(sampleHost);
  vi.mocked(api.setHostTags).mockReturnValue(new Promise(() => {})); // never resolves
  render(HostDetailPage, { params: { id: sampleHost.id } });
  await waitFor(() => expect(screen.getByText(sampleHost.friendly_name)).toBeInTheDocument());

  fireEvent.click(screen.getByRole('button', { name: /set tags/i }));
  await waitFor(() => expect(screen.getByRole('dialog')).toBeInTheDocument());

  const saveBtn = screen.getByRole('button', { name: /^save$/i });
  await fireEvent.click(saveBtn);
  await waitFor(() => expect(saveBtn).toHaveAttribute('aria-busy', 'true'));
  expect(saveBtn.textContent).toContain('Save');
  expect(saveBtn.textContent).not.toContain('Saving');
});
```

> **Note on fixtures:** Construct these fixtures from the shape of `HostResponse` as used in the existing test file:
>
> - `sampleHostWithSoftware`: at least one entry in the software/assigned-software field; check what field name the host response uses.
> - `sampleHostWithAllowlist`: at least one discovery allowlist entry; check what field name the host detail response uses for allowlist data.
> If the existing test file already has these fixtures, use them directly. If not, define them inline following the same pattern as `sampleHost`.

- [ ] **Step 3: Run all hosts tests**

```bash
cd frontend && npx vitest run src/routes/hosts/
```

Expected: all pass, including new tests.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/routes/hosts/
git commit -m "test(ui): extend hosts unit tests for Button primitive migration (#3h)"
```

---

## Task 5: Re-baseline Playwright snapshots

**Files:**

- Modify: hosts e2e spec file (locate by reading from Task 1)

Read the existing hosts e2e spec to understand the mock session pattern, how the host fixture is structured, and which selectors are used to wait for
content.

- [ ] **Step 1: Add or update snapshot tests for `/hosts` and `/hosts/[id]`**

The spec should snapshot both pages in dark + light themes. For `/hosts/[id]`, also snapshot:

- Default view
- Edit Host Name modal open (if the existing spec doesn't cover this)
- Discovery Allowlist section expanded (if accessible without a backend)

Mask volatile columns per the project's snapshot masking conventions (read existing snapshots/masks in the spec file).

- [ ] **Step 2: Re-baseline**

```bash
cd frontend && npx playwright test <hosts-spec-file> --update-snapshots
```

Expected: updated PNGs capturing the new Button-styled elements.

- [ ] **Step 3: Re-run for stability**

```bash
cd frontend && npx playwright test <hosts-spec-file>
```

Expected: all pass.

- [ ] **Step 4: Commit**

```bash
git add <hosts-e2e-spec> <snapshot-dir>
git commit -m "test(e2e): re-baseline hosts snapshots after Button primitive migration (#3h)"
```

---

## Task 6: Full frontend gate

- [ ] **Step 1: Run full gate**

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

Expected: every command exits 0.

- [ ] **Step 2: Run full e2e suite**

```bash
cd frontend && npx playwright test 2>&1 | tail -20
```

Expected: 0 failures. All other snapshot suites unaffected.

---

## Commit summary

| # | Files | Message |
| --- | --- | --- |
| 1 | `hosts/+page.svelte` | Migrate 3 button sites |
| 2 | `hosts/[id]/+page.svelte` | Migrate 11 button sites |
| 3 | hosts unit test file(s) | Extend assertions |
| 4 | hosts e2e spec + snapshot PNGs | Re-baseline |
