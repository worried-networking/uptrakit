# Software Nav Badge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show a live count badge next to "Software" in all shell nav surfaces, hidden when zero/null, capped at 99+.

**Architecture:** New dedicated store (`software-updates.svelte.ts`) fetches the updatable count once on auth
via an idempotent function; `+layout.svelte` reads the store reactively in its `navItems` derived expression
and renders a `StatusBadge tone="info"` in all four nav templates. Mobile primary nav uses inline placement
(no `ml-auto`) to avoid conflicting with `justify-center`.

**Tech Stack:** Svelte 5 runes (`$state`, `$derived`, `$effect`), Testing Library + Vitest, `getSoftwareItems` API call with `updatable=true, perPage=1`.

---

## File map

| Action | Path | Responsibility |
| --- | --- | --- |
| Create | `frontend/src/lib/stores/software-updates.svelte.ts` | Module-level `$state` count, idempotent fetch, public getter |
| Create | `frontend/src/lib/stores/software-updates.test.ts` | Unit tests for store behaviour |
| Modify | `frontend/src/routes/+layout.svelte` | Import store + StatusBadge, extend type, add effect + formatBadge, inject badge into navItems, render badge in all 4 templates |
| Modify | `frontend/src/routes/layout-button-migration.test.ts` | Add badge rendering tests |

---

## Task 1: Store unit tests (write first — they will fail)

**Files:**

- Create: `frontend/src/lib/stores/software-updates.test.ts`

- [ ] **Step 1: Create the test file**

```typescript
// frontend/src/lib/stores/software-updates.test.ts
import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { PaginatedResponse, SoftwareItemResponse } from '$lib/types';

function makeResponse(total: number): PaginatedResponse<SoftwareItemResponse> {
	return { items: [], total, page: 1, per_page: 1, total_pages: total };
}

describe('software-updates store', () => {
	beforeEach(() => {
		vi.resetModules();
	});

	it('getUpdatableSoftwareCount is null before any fetch', async () => {
		vi.doMock('$lib/api', () => ({ getSoftwareItems: vi.fn() }));
		const { getUpdatableSoftwareCount } = await import('$lib/stores/software-updates.svelte');
		expect(getUpdatableSoftwareCount()).toBeNull();
	});

	it('fetchUpdatableSoftwareCount sets count from response total', async () => {
		const getSoftwareItems = vi.fn().mockResolvedValue(makeResponse(42));
		vi.doMock('$lib/api', () => ({ getSoftwareItems }));
		const { getUpdatableSoftwareCount, fetchUpdatableSoftwareCount } =
			await import('$lib/stores/software-updates.svelte');
		await fetchUpdatableSoftwareCount();
		expect(getUpdatableSoftwareCount()).toBe(42);
	});

	it('fetchUpdatableSoftwareCount calls getSoftwareItems with correct args', async () => {
		const getSoftwareItems = vi.fn().mockResolvedValue(makeResponse(3));
		vi.doMock('$lib/api', () => ({ getSoftwareItems }));
		const { fetchUpdatableSoftwareCount } = await import('$lib/stores/software-updates.svelte');
		await fetchUpdatableSoftwareCount();
		expect(getSoftwareItems).toHaveBeenCalledWith(undefined, 1, undefined, undefined, true);
	});

	it('fetchUpdatableSoftwareCount is idempotent — second call skips network', async () => {
		const getSoftwareItems = vi.fn().mockResolvedValue(makeResponse(5));
		vi.doMock('$lib/api', () => ({ getSoftwareItems }));
		const { fetchUpdatableSoftwareCount } = await import('$lib/stores/software-updates.svelte');
		await fetchUpdatableSoftwareCount();
		await fetchUpdatableSoftwareCount();
		expect(getSoftwareItems).toHaveBeenCalledTimes(1);
	});

	it('fetchUpdatableSoftwareCount silently swallows errors, count stays null', async () => {
		const getSoftwareItems = vi.fn().mockRejectedValue(new Error('network error'));
		vi.doMock('$lib/api', () => ({ getSoftwareItems }));
		const { getUpdatableSoftwareCount, fetchUpdatableSoftwareCount } =
			await import('$lib/stores/software-updates.svelte');
		await expect(fetchUpdatableSoftwareCount()).resolves.toBeUndefined();
		expect(getUpdatableSoftwareCount()).toBeNull();
	});
});
```

- [ ] **Step 2: Run tests — confirm they fail with "cannot find module"**

```bash
cd frontend && npx vitest run src/lib/stores/software-updates.test.ts
```

Expected: all tests fail with `Cannot find module '$lib/stores/software-updates.svelte'`

---

## Task 2: Implement the store

**Files:**

- Create: `frontend/src/lib/stores/software-updates.svelte.ts`

- [ ] **Step 3: Create the store**

```typescript
// frontend/src/lib/stores/software-updates.svelte.ts

import { getSoftwareItems } from '$lib/api';

let count: number | null = $state(null);

/** Reactive getter — null before first successful fetch. */
export function getUpdatableSoftwareCount(): number | null {
	return count;
}

/**
 * Fetch the number of software items with updates available.
 *
 * Idempotent: if count is already set, returns immediately without a network
 * request. Silently swallows errors — the badge is non-critical.
 */
export async function fetchUpdatableSoftwareCount(): Promise<void> {
	if (count !== null) return;
	try {
		const res = await getSoftwareItems(undefined, 1, undefined, undefined, true);
		count = res.total;
	} catch {
		// Non-critical — badge stays hidden on error.
	}
}
```

- [ ] **Step 4: Run tests — confirm they pass**

```bash
cd frontend && npx vitest run src/lib/stores/software-updates.test.ts
```

Expected: 5 passed, 0 failed

- [ ] **Step 5: Commit**

```bash
git add frontend/src/lib/stores/software-updates.svelte.ts \
        frontend/src/lib/stores/software-updates.test.ts
git commit -m "feat(frontend): add software-updates store with updatable count"
```

---

## Task 3: Layout badge tests (write first — they will fail)

**Files:**

- Modify: `frontend/src/routes/layout-button-migration.test.ts`

- [ ] **Step 6: Add store mock and badge test cases to the layout test file**

Open `frontend/src/routes/layout-button-migration.test.ts`.

After the existing `vi.mock('$lib/stores/network.svelte', ...)` block (around line 65), add:

```typescript
vi.mock('$lib/stores/software-updates.svelte', () => ({
	getUpdatableSoftwareCount: vi.fn(() => null),
	fetchUpdatableSoftwareCount: vi.fn(async () => {})
}));
```

The existing imports in `layout-button-migration.test.ts` (lines 1-3) are:

```typescript
import { createRawSnippet } from 'svelte';
import { render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi, beforeEach } from 'vitest';
```

Replace those three import lines with:

```typescript
import { createRawSnippet } from 'svelte';
import { cleanup, render, screen } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
```

After the import block at the top of the file, add:

```typescript
import * as softwareUpdates from '$lib/stores/software-updates.svelte';
```

At the end of the file (before the closing `});` of the outermost `describe`), add:

```typescript
describe('software nav badge', () => {
	afterEach(() => {
		cleanup();
	});

	// Desktop sidebar and tablet sidebar/overflow nav items carry data-ui="app-shell-nav-item".
	// Mobile primary nav uses data-ui="app-shell-mobile-nav-item" and only renders
	// when viewportWidth < 640 — not the default in jsdom (starts at 1024), so mobile
	// primary badge rendering is verified by code inspection, not these tests.

	it('shows info StatusBadge with count when updates available', () => {
		vi.mocked(softwareUpdates.getUpdatableSoftwareCount).mockReturnValue(5);
		render(Layout, {
			children: createRawSnippet(() => ({ render: () => '<p>content</p>' }))
		});
		const softwareLink = document.querySelector(
			'[data-ui="app-shell-nav-item"][href="/software"]'
		);
		expect(softwareLink).not.toBeNull();
		const badge = softwareLink?.querySelector('[data-ui="status-badge"]');
		expect(badge).not.toBeNull();
		expect(badge?.getAttribute('data-tone')).toBe('info');
		expect(badge?.textContent?.trim()).toBe('5');
	});

	it('shows 99+ when count is 100 or more', () => {
		vi.mocked(softwareUpdates.getUpdatableSoftwareCount).mockReturnValue(150);
		render(Layout, {
			children: createRawSnippet(() => ({ render: () => '<p>content</p>' }))
		});
		const softwareLink = document.querySelector(
			'[data-ui="app-shell-nav-item"][href="/software"]'
		);
		const badge = softwareLink?.querySelector('[data-ui="status-badge"]');
		expect(badge?.textContent?.trim()).toBe('99+');
	});

	it('hides badge when count is 0', () => {
		vi.mocked(softwareUpdates.getUpdatableSoftwareCount).mockReturnValue(0);
		render(Layout, {
			children: createRawSnippet(() => ({ render: () => '<p>content</p>' }))
		});
		const softwareLink = document.querySelector(
			'[data-ui="app-shell-nav-item"][href="/software"]'
		);
		const badge = softwareLink?.querySelector('[data-ui="status-badge"]');
		expect(badge).toBeNull();
	});

	it('hides badge when count is null', () => {
		vi.mocked(softwareUpdates.getUpdatableSoftwareCount).mockReturnValue(null);
		render(Layout, {
			children: createRawSnippet(() => ({ render: () => '<p>content</p>' }))
		});
		const softwareLink = document.querySelector(
			'[data-ui="app-shell-nav-item"][href="/software"]'
		);
		const badge = softwareLink?.querySelector('[data-ui="status-badge"]');
		expect(badge).toBeNull();
	});
});
```

- [ ] **Step 7: Run badge tests — confirm they fail**

```bash
cd frontend && npx vitest run src/routes/layout-button-migration.test.ts
```

Expected: new badge tests fail, existing tests still pass

---

## Task 4: Implement layout changes

**Files:**

- Modify: `frontend/src/routes/+layout.svelte`

### Step 8 — Add imports

- [ ] **Step 8: Add StatusBadge and store imports**

In `frontend/src/routes/+layout.svelte`, find the existing import from `$lib/components/ui` (line ~25):

```svelte
import { Callout } from '$lib/components/ui';
```

Replace with:

```svelte
import { Callout, StatusBadge } from '$lib/components/ui';
```

After the last `import` statement in the `<script>` block (around line 28, before `let { children }`), add:

```typescript
import {
	getUpdatableSoftwareCount,
	fetchUpdatableSoftwareCount
} from '$lib/stores/software-updates.svelte';
```

### Step 9 — Extend ShellNavItem and add formatBadge

- [ ] **Step 9: Extend type and add helper**

Find the `ShellNavItem` type definition (around line 61):

```typescript
type ShellNavItem = {
	href: string;
	label: string;
	priority: number;
	origin: NavItemOrigin;
	stableId: string;
};
```

Replace with:

```typescript
type ShellNavItem = {
	href: string;
	label: string;
	priority: number;
	origin: NavItemOrigin;
	stableId: string;
	badge?: string;
};

function formatBadge(count: number | null): string | undefined {
	if (count === null || count === 0) return undefined;
	return count >= 100 ? '99+' : String(count);
}
```

### Step 10 — Add fetch effect

- [ ] **Step 10: Add $effect to trigger fetch on auth**

Find the `$effect` block that loads the surface registry (around line 126):

```typescript
// Load surface registry when authenticated, clear on logout.
$effect(() => {
	if (getUser()) {
		loadSurfaceRegistry();
	} else {
		clearSurfaceRegistry();
	}
});
```

After that block, add:

```typescript
$effect(() => {
	if (getUser()?.permissions.includes(Permission.ViewSoftware)) {
		void fetchUpdatableSoftwareCount();
	}
});
```

### Step 11 — Inject badge into navItems

- [ ] **Step 11: Add badge field to the builtInNavItems map**

Find the `.map()` call over `builtInNavItems` inside the `navItems` derived (around line 186):

```typescript
.map(
	(item): ShellNavItem => ({
		href: item.href,
		label: item.label,
		priority: item.priority,
		origin: 'built-in',
		stableId: item.href
	})
),
```

Replace with:

```typescript
.map(
	(item): ShellNavItem => ({
		href: item.href,
		label: item.label,
		priority: item.priority,
		origin: 'built-in',
		stableId: item.href,
		badge: item.href === '/software'
			? formatBadge(getUpdatableSoftwareCount())
			: undefined
	})
),
```

### Step 12 — Render badge: desktop sidebar

- [ ] **Step 12: Add badge to desktop sidebar nav items**

Find the desktop sidebar nav item `<a>` (around line 490). Include the `aria-current` line in the old_string for unambiguous matching:

```svelte
									aria-current={isNavItemActive(item) ? 'page' : undefined}
									data-ui="app-shell-nav-item"
								>
									{item.label}
								</a>
```

(No `onclick` attribute — this distinguishes the desktop block from tablet and overflow, both of which have an `onclick` between `data-ui` and `>`.)

Replace with:

```svelte
									aria-current={isNavItemActive(item) ? 'page' : undefined}
									data-ui="app-shell-nav-item"
								>
									{item.label}
									{#if item.badge}
										<span class="ml-auto pl-1.5">
											<StatusBadge tone="info" label={item.badge} />
										</span>
									{/if}
								</a>
```

### Step 13 — Render badge: tablet sidebar

- [ ] **Step 13: Add badge to tablet sidebar nav items**

Find the tablet sidebar nav item `<a>` (around line 535). The closing section is:

```svelte
								data-ui="app-shell-nav-item"
								onclick={() => (sidebarOverlayOpen = false)}
							>
								{item.label}
							</a>
```

Replace with:

```svelte
								data-ui="app-shell-nav-item"
								onclick={() => (sidebarOverlayOpen = false)}
							>
								{item.label}
								{#if item.badge}
									<span class="ml-auto pl-1.5">
										<StatusBadge tone="info" label={item.badge} />
									</span>
								{/if}
							</a>
```

### Step 14 — Render badge: mobile primary nav

- [ ] **Step 14: Add badge to mobile primary nav items**

Find the mobile primary nav `<a>` (around line 568). The current template body is:

```svelte
>
	<span class="truncate">{item.label}</span>
</a>
```

Replace with:

```svelte
>
	<span class="truncate">{item.label}</span>
	{#if item.badge}
		<span class="shrink-0 pl-1.5">
			<StatusBadge tone="info" label={item.badge} />
		</span>
	{/if}
</a>
```

Note: no `ml-auto` here — the parent uses `justify-center`, so the badge sits inline after the label.

### Step 15 — Render badge: mobile overflow sheet

- [ ] **Step 15: Add badge to mobile overflow sheet nav items**

Find the mobile overflow nav item `<a>` (around line 628). The closing section is:

```svelte
								data-ui="app-shell-nav-item"
								onclick={() => (mobileOverflowOpen = false)}
							>
								{item.label}
							</a>
```

Replace with:

```svelte
								data-ui="app-shell-nav-item"
								onclick={() => (mobileOverflowOpen = false)}
							>
								{item.label}
								{#if item.badge}
									<span class="ml-auto pl-1.5">
										<StatusBadge tone="info" label={item.badge} />
									</span>
								{/if}
							</a>
```

---

## Task 5: Verify and commit

- [ ] **Step 16: Run badge tests — confirm they pass**

```bash
cd frontend && npx vitest run src/routes/layout-button-migration.test.ts
```

Expected: all tests pass including the 4 new badge tests

- [ ] **Step 17: Run full frontend test suite**

```bash
cd frontend && npm run test
```

Expected: all tests pass, 0 failures

- [ ] **Step 18: Run type check and lint**

```bash
cd frontend && npm run check && npm run lint
```

Expected: no type errors, no lint warnings

- [ ] **Step 19: Commit**

```bash
git add frontend/src/routes/+layout.svelte \
        frontend/src/routes/layout-button-migration.test.ts
git commit -m "feat(frontend): render updatable-software badge in all nav templates"
```

---

## Self-review

**Spec coverage:**

| Spec requirement | Task |
| --- | --- |
| Store with idempotent fetch | Task 2 |
| `getUpdatableSoftwareCount()` returns null before fetch | Task 2, verified in Task 1 tests |
| `null` and `0` → no badge | `formatBadge`, tested in Task 3 |
| `1–99` → count string | `formatBadge`, tested in Task 3 |
| `≥100` → "99+" | `formatBadge`, tested in Task 3 |
| `$effect` on ViewSoftware permission | Step 10 |
| badge in navItems derived | Step 11 |
| Desktop sidebar badge (`ml-auto`) | Step 12 |
| Tablet sidebar badge (`ml-auto`) | Step 13 |
| Mobile primary badge (no `ml-auto`, `shrink-0`) | Step 14 |
| Mobile overflow badge (`ml-auto`) | Step 15 |
| `StatusBadge tone="info"` | Steps 12–15 |
| SSE wiring deferred | ✅ out of scope, store is ready for it |

All spec requirements covered. No gaps.
