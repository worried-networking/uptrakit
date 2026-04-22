# Layout Shell + Home Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate all raw Skeleton `preset-*`/`btn` button and link elements in `+layout.svelte`
and `home/+page.svelte` to the `<Button>` primitive; leave landmark nav `<a>` elements unchanged.

**Architecture:** Two files, task-per-file; existing nav anchors are already token-based and
require no change; session-expired banner and action buttons are the primary migration targets.

**Tech Stack:** Svelte 5, SvelteKit, TypeScript, Vitest, Playwright

---

## Dependency

**Blocks on:** sub-spec #2 merged (`Button` exported from `frontend/src/lib/components/Button.svelte`)
and sub-spec #2c merged (`ariaLabel?: string` prop available on `Button` for icon-only sites).

---

## Migration rules (quick reference)

| Legacy class | Button variant | Notes |
| --- | --- | --- |
| `preset-filled-primary-500` | `primary` | Retry button in `+page.svelte` |
| `preset-filled-error-500` | `danger` | Session-expired "Log in" link |
| `preset-tonal-surface` | `ghost` | Logout, theme toggle, sidebar toggle, Dismiss |
| `btn btn-sm preset-tonal` | `ghost` size="sm" | Action link `<a>` elements in `+page.svelte` |
| `btn-icon preset-tonal-surface` | `ghost` + icon-only + `ariaLabel` | Theme toggle, tablet sidebar toggle |

Nav `<a>` elements inside `<nav>` landmarks: **no change** — already token-based, no `preset-*`
classes present. Do NOT wrap in `<Button>`.

`href` assertion note: `<Button href="...">` renders `<a role="button">`. In tests, assert `href`
via CSS selector `document.querySelector('a[href="..."]')`, **not**
`getByRole('button').toHaveAttribute('href', ...)`.

Snippet syntax: icon content goes inside `{#snippet leadingIcon()}..{/snippet}` declared **inside**
the `<Button>` element — NOT as a prop value.

---

## Button site inventory

Read source files to verify exact line numbers before editing.
All line numbers reference the current state of the files.

**`frontend/src/routes/+layout.svelte`** — 6 button sites:

1. Line 371–387: Tablet sidebar toggle — `<button class="btn-icon preset-tonal-surface" ...>`
   with hamburger SVG → `<Button variant="ghost" ariaLabel="..." onclick>` with
   `{#snippet leadingIcon()}` containing the SVG.
2. Line 397–426: Theme toggle — `<button class="btn-icon preset-tonal-surface" ...>` with
   conditional SVG children → `<Button variant="ghost" ariaLabel="..." onclick>` with
   `{#snippet leadingIcon()}`.
3. Line 428: Logout — `<button class="btn preset-tonal-surface" onclick={handleLogout}>` →
   `<Button variant="ghost" onclick={handleLogout}>`.
4. Lines 429–431: Unauthenticated Login/Register links —
   `<a href="/login" class="btn preset-tonal-surface">` × 2 →
   `<Button variant="ghost" href="/login">` and `<Button variant="ghost" href="/register">`.
5. Lines 451–454: Session-expired "Log in" link —
   `<a href="..." class="btn btn-sm preset-filled-error-500">` →
   `<Button variant="danger" size="sm" href="...">`.
6. Lines 455–459: Session-expired "Dismiss" —
   `<button class="btn btn-sm preset-tonal-surface" ...>` →
   `<Button variant="ghost" size="sm" onclick={...}>`.

Out-of-scope elements (no `preset-*` classes, already token-based):

- Lines 497–504: Tablet sidebar backdrop `<button>` — plain overlay, no `preset-*`. Leave as-is.
- Lines 578–593: Mobile overflow toggle `<button>` — nav pill, token-based. Leave as-is per Q5.
- Lines 597–603: Mobile overflow backdrop `<button>` — overlay, no `preset-*`. Leave as-is.
- All `<a>` elements inside `<nav>` landmark regions — plain anchors, leave unchanged per Q5.

**`frontend/src/routes/+page.svelte`** — 4 button sites:

1. Line 162: Retry button — `<button class="btn preset-filled-primary-500 mt-3" onclick>` →
   `<Button variant="primary" class="mt-3" onclick>`.
2. Line 247: "Review" action link — `<a class="btn btn-sm preset-tonal" href="/services?status=pending">` →
   `<Button variant="ghost" size="sm" href="/services?status=pending">`.
3. Line 259: "Investigate" action link — `<a class="btn btn-sm preset-tonal" href="/history?status=failed">` →
   `<Button variant="ghost" size="sm" href="/history?status=failed">`.
4. Line 271: "View all" action link (inside `{#snippet actions()}`) —
   `<a href="/history" class="btn btn-sm preset-tonal">` →
   `<Button variant="ghost" size="sm" href="/history">`.

---

## Task 1: Write failing unit tests for +layout.svelte button sites

**Files:**

- Create: `frontend/src/routes/layout-button-migration.test.ts`

TDD: write tests that fail against the current `preset-*` markup, then pass after Task 2.

Read `frontend/src/routes/surface-migration.test.ts` in full before writing — copy its mock setup
for `$app/state`, `$app/navigation`, `$lib/auth.svelte`, `$lib/theme.svelte`, `$lib/api`,
`$lib/stores/network.svelte`, `$lib/surfaces/registry.svelte`, and the `createRawSnippet` children
pattern for rendering `Layout`.

- [ ] **Step 1: Create the test file with mock setup**

Mirror the mock block from `surface-migration.test.ts` exactly. The mock for `$lib/auth.svelte`
must include `getUser`, `getLoading`, `initialize`, `handleLogout`, `getSessionExpired`, and
`setSessionExpired`. `getSessionExpired` must return `true` for session-expired banner tests.

Types needed by tests (e.g. `RenderResult`) should be imported from `@testing-library/svelte`;
do NOT import component-internal types from the `.svelte` file.

- [ ] **Step 2: Add test — theme toggle renders as ghost Button with aria-label**

```ts
it('theme toggle renders as ghost Button with aria-label', () => {
  render(Layout, {
    children: createRawSnippet(() => ({ render: () => '<p>content</p>' }))
  });
  const toggle = document.querySelector(
    '[data-ui="app-shell-header"] button[aria-label*="mode"]'
  ) as HTMLElement;
  expect(toggle).not.toBeNull();
  expect(toggle.className).toContain('h-[23px]');
  expect(toggle.className).toContain('bg-transparent');
  expect(toggle).toHaveAttribute('aria-label');
});
```

This test fails on the current file (no `h-[23px]` class present) and passes after migration.

- [ ] **Step 3: Add test — tablet sidebar toggle renders as ghost Button with aria-label**

```ts
it('tablet sidebar toggle renders as ghost Button with aria-label', () => {
  render(Layout, {
    children: createRawSnippet(() => ({ render: () => '<p>content</p>' }))
  });
  const toggle = document.querySelector('[data-ui="app-shell-sidebar-toggle"]') as HTMLElement;
  // Toggle only renders in tablet viewport range; skip assertion if not rendered
  if (!toggle) return;
  expect(toggle.className).toContain('h-[23px]');
  expect(toggle.className).toContain('bg-transparent');
  expect(toggle).toHaveAttribute('aria-label');
  expect(toggle).not.toHaveAttribute('role', 'link');
});
```

- [ ] **Step 4: Add test — Logout renders as ghost Button**

```ts
it('logout button renders as ghost Button', () => {
  render(Layout, {
    children: createRawSnippet(() => ({ render: () => '<p>content</p>' }))
  });
  const logoutBtn = screen.getByRole('button', { name: /logout/i });
  expect(logoutBtn.className).toContain('h-[23px]');
  expect(logoutBtn.className).toContain('bg-transparent');
});
```

- [ ] **Step 5: Add test — session-expired banner buttons render with correct variants**

The mock for `getSessionExpired` must return `true` for this test. Use a nested `describe` with
its own `beforeEach` that overrides `getSessionExpired` to return `true`.

```ts
describe('session-expired banner', () => {
  beforeEach(() => {
    vi.mocked(auth.getSessionExpired).mockReturnValue(true);
  });

  it('"Log in" in session-expired banner renders as danger Button with href', () => {
    render(Layout, {
      children: createRawSnippet(() => ({ render: () => '<p>content</p>' }))
    });
    // Use CSS selector — Button href branch renders <a role="button">, not <a role="link">
    const loginAnchor = document.querySelector('a[href*="/login"]') as HTMLElement;
    expect(loginAnchor).not.toBeNull();
    expect(loginAnchor.className).toContain('h-[23px]');
    expect(loginAnchor.className).not.toContain('preset-filled-error');
  });

  it('"Dismiss" in session-expired banner renders as ghost Button', () => {
    render(Layout, {
      children: createRawSnippet(() => ({ render: () => '<p>content</p>' }))
    });
    const dismissBtn = screen.getByRole('button', { name: /dismiss/i });
    expect(dismissBtn.className).toContain('h-[23px]');
    expect(dismissBtn.className).toContain('bg-transparent');
  });
});
```

- [ ] **Step 6: Add test — nav anchors inside nav landmark are plain `<a>` without role="button"**

```ts
it('nav anchors inside nav landmark render as plain <a> without role="button"', () => {
  render(Layout, {
    children: createRawSnippet(() => ({ render: () => '<p>content</p>' }))
  });
  const nav = document.querySelector('[data-ui="app-shell-nav"]');
  if (!nav) return;
  const navLinks = nav.querySelectorAll('a');
  for (const link of navLinks) {
    expect(link.getAttribute('role')).not.toBe('button');
  }
});
```

- [ ] **Step 7: Add test — no preset-* classes remain in layout source**

```ts
import layoutSource from './+layout.svelte?raw';

it('layout source contains no preset-filled-* or preset-tonal-* class strings', () => {
  expect(layoutSource).not.toMatch(/preset-filled-/);
  expect(layoutSource).not.toMatch(/preset-tonal-/);
  expect(layoutSource).not.toMatch(/btn-icon/);
});
```

- [ ] **Step 8: Run tests to confirm they all fail**

```bash
cd frontend && npx vitest run src/routes/layout-button-migration.test.ts 2>&1 | tail -30
```

Expected: test failures on `h-[23px]` and `preset-*` assertions — confirms tests exercise the
right code paths.

- [ ] **Step 9: Commit**

```bash
git add frontend/src/routes/layout-button-migration.test.ts
git commit -m "test(layout): add failing Button migration tests for +layout.svelte (#3b)"
```

---

## Task 2: Migrate +layout.svelte (6 button sites)

**Files:**

- Modify: `frontend/src/routes/+layout.svelte`

Read the file before editing to confirm exact markup and surrounding context.

- [ ] **Step 1: Add `Button` import**

In the `<script lang="ts">` block, after the existing imports, add:

```ts
import Button from '$lib/components/Button.svelte';
```

No existing imports need to be removed — `+layout.svelte` does not import any `preset-*`
constants.

- [ ] **Step 2: Migrate site 1 — tablet sidebar toggle (lines ~371–387)**

Before:

```svelte
<button
  bind:this={tabletSidebarToggleEl}
  class="btn-icon preset-tonal-surface"
  type="button"
  aria-label={sidebarOverlayOpen ? 'Close navigation' : 'Open navigation'}
  aria-controls="app-shell-sidebar-tablet"
  aria-expanded={sidebarOverlayOpen}
  data-ui="app-shell-sidebar-toggle"
  onclick={() => (sidebarOverlayOpen = !sidebarOverlayOpen)}
>
  <svg ...>...</svg>
</button>
```

After:

```svelte
<Button
  bind:this={tabletSidebarToggleEl}
  variant="ghost"
  ariaLabel={sidebarOverlayOpen ? 'Close navigation' : 'Open navigation'}
  aria-controls="app-shell-sidebar-tablet"
  aria-expanded={sidebarOverlayOpen}
  data-ui="app-shell-sidebar-toggle"
  onclick={() => (sidebarOverlayOpen = !sidebarOverlayOpen)}
>
  {#snippet leadingIcon()}
    <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor" class="h-4 w-4">
      <!-- hamburger path — copy verbatim from current markup -->
    </svg>
  {/snippet}
</Button>
```

Note: `bind:this` requires `Button` to expose a `ref` or forward `bind:this` to the underlying
element. Check `frontend/src/lib/components/Button.svelte` for the correct binding pattern before
applying. `tabletSidebarToggleEl` is used in `activateOverlayModal` for restore-focus; if
`Button` does not support `bind:this`, bind via `data-ui="app-shell-sidebar-toggle"` +
`querySelector` in the relevant `$effect`.

- [ ] **Step 3: Migrate site 2 — theme toggle (lines ~397–426)**

Before:

```svelte
<button
  class="btn-icon preset-tonal-surface"
  type="button"
  title={getThemeMode() === 'light' ? 'Light mode' : ...}
  onclick={cycleTheme}
>
  {#if getThemeMode() === 'light'}<svg .../>{:else if ...}<svg .../>{:else}<svg .../>{/if}
</button>
```

After:

```svelte
<Button
  variant="ghost"
  ariaLabel={
    getThemeMode() === 'light'
      ? 'Light mode — click to switch to dark'
      : getThemeMode() === 'dark'
        ? 'Dark mode — click to switch to system'
        : 'System mode — click to switch to light'
  }
  onclick={cycleTheme}
>
  {#snippet leadingIcon()}
    {#if getThemeMode() === 'light'}
      <svg ...><!-- sun path verbatim --></svg>
    {:else if getThemeMode() === 'dark'}
      <svg ...><!-- moon path verbatim --></svg>
    {:else}
      <svg ...><!-- monitor path verbatim --></svg>
    {/if}
  {/snippet}
</Button>
```

The `title` attribute is replaced by `ariaLabel` per sub-spec #2c / spec Q4. Copy SVG paths
verbatim — do not alter them.

- [ ] **Step 4: Migrate site 3 — Logout button (line ~428)**

Before:

```svelte
<button class="btn preset-tonal-surface" onclick={handleLogout}> Logout </button>
```

After:

```svelte
<Button variant="ghost" onclick={handleLogout}>Logout</Button>
```

- [ ] **Step 5: Migrate sites 4a and 4b — unauthenticated Login/Register links (lines ~429–431)**

Before:

```svelte
<a href="/login" class="btn preset-tonal-surface">Login</a>
<a href="/register" class="btn preset-tonal-surface">Register</a>
```

After:

```svelte
<Button variant="ghost" href="/login">Login</Button>
<Button variant="ghost" href="/register">Register</Button>
```

These appear in the header's unauthenticated branch, outside any `<nav>` landmark.
`<Button href>` is correct here — non-nav link-shaped CTAs per spec Q5.

- [ ] **Step 6: Migrate site 5 — session-expired "Log in" link (lines ~451–454)**

Before:

```svelte
<a
  href="/login?redirect={encodeURIComponent(page.url.pathname + page.url.search)}"
  class="btn btn-sm preset-filled-error-500">Log in</a
>
```

After:

```svelte
<Button
  variant="danger"
  size="sm"
  href="/login?redirect={encodeURIComponent(page.url.pathname + page.url.search)}"
>Log in</Button>
```

- [ ] **Step 7: Migrate site 6 — session-expired "Dismiss" button (lines ~455–459)**

Before:

```svelte
<button
  onclick={() => setSessionExpired(false)}
  class="btn btn-sm preset-tonal-surface"
  aria-label="Dismiss session expired notification">Dismiss</button
>
```

After:

```svelte
<Button
  variant="ghost"
  size="sm"
  ariaLabel="Dismiss session expired notification"
  onclick={() => setSessionExpired(false)}
>Dismiss</Button>
```

- [ ] **Step 8: Verify compilation**

```bash
cd frontend && npm run check 2>&1 | grep -E '(layout|error)'
```

Expected: no type errors on `+layout.svelte`.

- [ ] **Step 9: Run layout tests**

```bash
cd frontend && npx vitest run src/routes/layout-button-migration.test.ts 2>&1 | tail -30
```

Expected: all previously failing tests now pass. The source-scan test
(`no preset-filled-* or preset-tonal-*`) must pass.

- [ ] **Step 10: Commit**

```bash
git add frontend/src/routes/+layout.svelte
git commit -m "refactor(layout): migrate all 6 button sites to Button primitive (#3b)"
```

---

## Task 3: Write failing unit tests for +page.svelte button sites

**Files:**

- Modify: `frontend/src/routes/home.test.ts`

Read `frontend/src/routes/home.test.ts` in full before editing. Add to the existing
`describe('Dashboard Route', ...)` block; do not restructure existing tests.

- [ ] **Step 1: Add test — Retry button renders as primary Button**

The Retry button only appears when an error is set. Reject all API calls to trigger the error
state, then assert the button renders with Button primitive classes.

```ts
it('Retry button renders as primary Button with mt-3 class', async () => {
  vi.mocked(api.getHosts).mockRejectedValue(new Error('fail'));
  vi.mocked(api.getServices).mockRejectedValue(new Error('fail'));
  vi.mocked(api.getSoftwareItems).mockRejectedValue(new Error('fail'));
  vi.mocked(api.listUpdateHistory).mockRejectedValue(new Error('fail'));
  render(HomePage);
  await waitFor(() =>
    expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument()
  );

  const retryBtn = screen.getByRole('button', { name: /retry/i });
  expect(retryBtn.className).toContain('h-[23px]');
  expect(retryBtn.className).toContain(
    'bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]'
  );
  expect(retryBtn.className).toContain('mt-3');
});
```

- [ ] **Step 2: Add test — "Review" action link renders as ghost Button href**

```ts
it('"Review" action link renders as ghost Button href', async () => {
  render(HomePage);
  await waitFor(() => expect(screen.getByText('Attention Needed')).toBeInTheDocument());

  const reviewAnchor = document.querySelector(
    'a[href="/services?status=pending"]'
  ) as HTMLElement;
  expect(reviewAnchor).not.toBeNull();
  expect(reviewAnchor.className).toContain('h-[23px]');
  expect(reviewAnchor.className).toContain('bg-transparent');
  expect(reviewAnchor.className).not.toContain('preset-tonal');
});
```

- [ ] **Step 3: Add test — "Investigate" action link renders as ghost Button href**

```ts
it('"Investigate" action link renders as ghost Button href', async () => {
  render(HomePage);
  await waitFor(() => expect(screen.getByText('Attention Needed')).toBeInTheDocument());

  const investigateAnchor = document.querySelector(
    'a[href="/history?status=failed"]'
  ) as HTMLElement;
  expect(investigateAnchor).not.toBeNull();
  expect(investigateAnchor.className).toContain('h-[23px]');
  expect(investigateAnchor.className).toContain('bg-transparent');
});
```

- [ ] **Step 4: Add test — "View all" action link renders as ghost Button href**

```ts
it('"View all" action link renders as ghost Button href', async () => {
  // total > 5 is required for View all to render
  vi.mocked(api.listUpdateHistory).mockResolvedValue({
    items: Array.from({ length: 5 }, (_, i) => ({
      id: `hist-${i}`,
      software_item_name: `pkg-${i}`,
      host_name: 'host',
      status: 'completed',
      created_at: '2026-01-01T10:00:00Z'
    })) as unknown as UpdateHistoryResponse[],
    total: 10,
    page: 1,
    per_page: 5,
    total_pages: 2
  });
  render(HomePage);
  await waitFor(() => expect(screen.getByText('Recent Updates')).toBeInTheDocument());
  await waitFor(() =>
    expect(document.querySelector('a[href="/history"]')).not.toBeNull()
  );

  const viewAllAnchor = document.querySelector('a[href="/history"]') as HTMLElement;
  expect(viewAllAnchor).not.toBeNull();
  expect(viewAllAnchor.className).toContain('h-[23px]');
  expect(viewAllAnchor.className).toContain('bg-transparent');
});
```

- [ ] **Step 5: Add test — no preset-* classes remain in home page source**

```ts
import homeSource from './+page.svelte?raw';

it('home page source contains no preset-filled-* or preset-tonal-* class strings', () => {
  expect(homeSource).not.toMatch(/preset-filled-/);
  expect(homeSource).not.toMatch(/preset-tonal-/);
});
```

- [ ] **Step 6: Run tests to confirm they fail**

```bash
cd frontend && npx vitest run src/routes/home.test.ts 2>&1 | tail -30
```

Expected: failures on `h-[23px]`, `bg-transparent`, and `no preset-*` assertions — confirms
tests exercise the right code paths.

- [ ] **Step 7: Commit**

```bash
git add frontend/src/routes/home.test.ts
git commit -m "test(home): add failing Button migration tests for +page.svelte (#3b)"
```

---

## Task 4: Migrate +page.svelte (4 button sites)

**Files:**

- Modify: `frontend/src/routes/+page.svelte`

Read the file before editing to confirm exact markup and surrounding context.

- [ ] **Step 1: Add `Button` import**

In the `<script lang="ts">` block, add after existing imports:

```ts
import Button from '$lib/components/Button.svelte';
```

- [ ] **Step 2: Migrate site 1 — Retry button (line ~162)**

Before:

```svelte
<button class="btn preset-filled-primary-500 mt-3" onclick={() => loadDashboard()}>Retry</button>
```

After:

```svelte
<Button variant="primary" class="mt-3" onclick={() => loadDashboard()}>Retry</Button>
```

The `mt-3` spacing is passed via the `class` prop so Button merges it with its own class list.

- [ ] **Step 3: Migrate site 2 — "Review" action link (line ~247)**

Before:

```svelte
<a class="btn btn-sm preset-tonal" href="/services?status=pending">Review</a>
```

After:

```svelte
<Button variant="ghost" size="sm" href="/services?status=pending">Review</Button>
```

- [ ] **Step 4: Migrate site 3 — "Investigate" action link (line ~259)**

Before:

```svelte
<a class="btn btn-sm preset-tonal" href="/history?status=failed">Investigate</a>
```

After:

```svelte
<Button variant="ghost" size="sm" href="/history?status=failed">Investigate</Button>
```

- [ ] **Step 5: Migrate site 4 — "View all" action link (line ~271, inside `{#snippet actions()}`)**

Before:

```svelte
{#snippet actions()}
  {#if totalRecentUpdates > 5}
    <a href="/history" class="btn btn-sm preset-tonal">View all</a>
  {/if}
{/snippet}
```

After:

```svelte
{#snippet actions()}
  {#if totalRecentUpdates > 5}
    <Button variant="ghost" size="sm" href="/history">View all</Button>
  {/if}
{/snippet}
```

- [ ] **Step 6: Verify compilation**

```bash
cd frontend && npm run check 2>&1 | grep -E '(\+page|home)'
```

Expected: no type errors on `+page.svelte`.

- [ ] **Step 7: Run home tests**

```bash
cd frontend && npx vitest run src/routes/home.test.ts 2>&1 | tail -30
```

Expected: all previously failing tests now pass. The source-scan test must pass.

- [ ] **Step 8: Commit**

```bash
git add frontend/src/routes/+page.svelte
git commit -m "refactor(home): migrate all 4 button sites to Button primitive (#3b)"
```

---

## Task 5: Full unit test suite

- [ ] **Step 1: Run the full test suite**

```bash
cd frontend && npx vitest run 2>&1 | tail -30
```

Expected: all tests pass, including `surface-migration.test.ts` (its source-scan must still pass
since `+layout.svelte` no longer contains `preset-*`), `home.test.ts`, and
`layout-button-migration.test.ts`.

If `surface-migration.test.ts` fails on a different assertion about layout source content (e.g.
it positively asserts something now removed), read the test and update the expected value to match
the migrated file. Do not weaken coverage — only update expected values to reflect the migration.

- [ ] **Step 2: Commit test fixes if needed**

```bash
git add frontend/src/routes/surface-migration.test.ts
git commit -m "test(surface-migration): update layout source assertions after #3b Button migration"
```

---

## Task 6: Re-baseline Playwright snapshots

**Files:**

- Modify or create: `frontend/tests/e2e/layout-shell.spec.ts`

Check for an existing layout/shell e2e spec before creating. Read
`frontend/tests/e2e/button-primitive.spec.ts` for the exact `mockAuthApi` / session mock
patterns to copy.

- [ ] **Step 1: Check for existing layout e2e spec**

```bash
ls frontend/tests/e2e/
```

If a spec covering `/` (dashboard) and layout chrome already exists, read it and re-baseline.
If not, create `frontend/tests/e2e/layout-shell.spec.ts`.

- [ ] **Step 2: Write the spec (if creating new)**

The spec must snapshot the following routes × 2 themes (dark + light) using `toHaveScreenshot`
with `threshold: 0.005`:

- `/` (dashboard home) — full chrome + stat cards
- `/hosts` — inactive nav pill, desktop sidebar visible
- `/services` — inactive nav pill in a different group
- `/settings` — deep route, tests nav-pill inactive-other-group state
- `/` in both themes to cover theme-toggle effect on chrome tokens

If the layout supports tablet-width sidebar at 640–1023 px, also test at that viewport width
to catch the sidebar toggle button rendering.

Per spec §Testing, all non-chrome route content must stay within 0.5% threshold — use per-snapshot
`threshold: 0.005`. Any drift on deep-route non-chrome content is a spec bug and blocks merge.

- [ ] **Step 3: Generate baselines**

```bash
cd frontend && npx playwright test tests/e2e/layout-shell.spec.ts --update-snapshots
```

- [ ] **Step 4: Re-run to confirm stability**

```bash
cd frontend && npx playwright test tests/e2e/layout-shell.spec.ts
```

Expected: all pass with 0 failures and 0 visual diffs.

- [ ] **Step 5: Commit**

```bash
git add frontend/tests/e2e/layout-shell.spec.ts "frontend/tests/e2e/layout-shell.spec.ts-snapshots"
git commit -m "test(e2e): add/re-baseline layout shell snapshots after Button primitive migration (#3b)"
```

---

## Task 7: Full frontend gate

- [ ] **Step 1: Run full gate**

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

Expected: every command exits 0.

- [ ] **Step 2: Run full e2e suite**

```bash
cd frontend && npx playwright test 2>&1 | tail -20
```

Expected: 0 failures. All existing snapshot suites (public-entry, button-primitive,
form-primitive) must be unaffected — this migration only touches `+layout.svelte` and
`+page.svelte` and their tests.

---

## Commit summary

| # | Commit | Files |
| --- | --- | --- |
| 1 | Failing layout tests | `layout-button-migration.test.ts` (new) |
| 2 | Migrate 6 sites | `+layout.svelte` |
| 3 | Failing home tests | `home.test.ts` |
| 4 | Migrate 4 sites | `+page.svelte` |
| 5 | Fix surface-migration test if needed | `surface-migration.test.ts` |
| 6 | E2e baselines | `layout-shell.spec.ts` + PNGs |
