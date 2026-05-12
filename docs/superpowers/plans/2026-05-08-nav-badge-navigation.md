# Nav Badge Separate Navigation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the `/software` nav badge a standalone `<a>` that pre-applies the "updates available"
filter, while the label/icon link still navigates to the unfiltered view.

**Architecture:** Add optional `badgeHref`/`badgeAriaLabel` fields (discriminated union) to
`ShellNavItem`. When `badgeHref` is set, render the badge as a sibling `<a>` next to the label
anchor rather than inside it. The software item is the only callsite that sets these fields. Four
render sites in `+layout.svelte` need updating (3 × Variant A `<li>`-based; 1 × Variant B mobile
bottom nav `<div>`-wrapped).

**Tech Stack:** Svelte 5 (SvelteKit), TypeScript strict mode, `@testing-library/svelte`, Vitest,
Tailwind CSS

---

## File Map

| Action | Path                                                  | Responsibility                    |
| ------ | ----------------------------------------------------- | --------------------------------- |
| Create | `frontend/src/routes/software/constants.ts`           | `UPDATES_AVAILABLE_HREF` constant |
| Modify | `frontend/src/routes/+layout.svelte`                  | Type, assignment, 4 render sites  |
| Modify | `frontend/src/routes/layout-button-migration.test.ts` | New badge-navigation tests        |

---

### Task 1: Export `UPDATES_AVAILABLE_HREF` constant

**Files:**

- Create: `frontend/src/routes/software/constants.ts`

- [ ] **Step 1: Create the constants file**

```typescript
export const UPDATES_AVAILABLE_HREF = "/software?updatable=true";
```

- [ ] **Step 2: Verify TypeScript is happy**

```bash
cd frontend && npm run check 2>&1 | tail -5
```

Expected: no new errors.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/routes/software/constants.ts
git commit --only frontend/src/routes/software/constants.ts \
  -m "feat(frontend): export UPDATES_AVAILABLE_HREF constant from software route"
```

---

### Task 2: Write failing tests for badge-as-link behavior

**Files:**

- Modify: `frontend/src/routes/layout-button-migration.test.ts`

The existing test file already mocks `getUpdatableSoftwareCount` to return `null`. Add a new
`describe` block that mocks it to return `5` and asserts the badge renders as a separate link.

- [ ] **Step 1: Add the test block** — append after the last existing test in
      `layout-button-migration.test.ts`:

```typescript
describe("software nav badge navigation", () => {
  beforeEach(() => {
    vi.mocked(softwareUpdates.getUpdatableSoftwareCount).mockReturnValue(5);
  });

  afterEach(() => {
    cleanup();
    vi.mocked(softwareUpdates.getUpdatableSoftwareCount).mockReturnValue(null);
  });

  it("renders badge as a separate link when update count is non-zero", () => {
    render(Layout, {
      children: createRawSnippet(() => ({ render: () => "<p>content</p>" })),
    });
    const badgeLink = document.querySelector(
      '[data-ui="app-shell-nav-badge"]',
    ) as HTMLAnchorElement | null;
    expect(badgeLink).not.toBeNull();
    expect(badgeLink?.tagName.toLowerCase()).toBe("a");
    expect(badgeLink?.getAttribute("href")).toBe("/software?updatable=true");
    expect(badgeLink?.getAttribute("aria-label")).toBe(
      "View software updates available",
    );
  });

  it("badge link text shows formatted count", () => {
    render(Layout, {
      children: createRawSnippet(() => ({ render: () => "<p>content</p>" })),
    });
    const badgeLink = document.querySelector(
      '[data-ui="app-shell-nav-badge"]',
    ) as HTMLElement | null;
    expect(badgeLink?.textContent?.trim()).toBe("5");
  });

  it("does not render badge link when update count is null", () => {
    vi.mocked(softwareUpdates.getUpdatableSoftwareCount).mockReturnValue(null);
    render(Layout, {
      children: createRawSnippet(() => ({ render: () => "<p>content</p>" })),
    });
    const badgeLink = document.querySelector('[data-ui="app-shell-nav-badge"]');
    expect(badgeLink).toBeNull();
  });

  it("does not render badge link when update count is zero", () => {
    vi.mocked(softwareUpdates.getUpdatableSoftwareCount).mockReturnValue(0);
    render(Layout, {
      children: createRawSnippet(() => ({ render: () => "<p>content</p>" })),
    });
    const badgeLink = document.querySelector('[data-ui="app-shell-nav-badge"]');
    expect(badgeLink).toBeNull();
  });
});
```

- [ ] **Step 2: Run the new tests and confirm they fail**

```bash
cd frontend && npm run test -- --reporter=verbose layout-button-migration 2>&1 | tail -30
```

Expected: the four new tests fail (`[data-ui="app-shell-nav-badge"]` not found yet).

---

### Task 3: Extend `ShellNavItem` type, import constant, convert `$derived` to `$derived.by`

**Files:**

- Modify: `frontend/src/routes/+layout.svelte` lines 27, 81–89, 254–283

- [ ] **Step 1: Add import** — after line 27
      (`import { getUpdatableSoftwareCount, fetchUpdatableSoftwareCount } ...`), add:

```typescript
import { UPDATES_AVAILABLE_HREF } from "./software/constants";
```

- [ ] **Step 2: Replace the `ShellNavItem` type** — replace lines 81–89:

_Before:_

```typescript
type ShellNavItem = {
  href: string;
  label: string;
  priority: number;
  origin: NavItemOrigin;
  stableId: string;
  badge?: string;
  icon?: ComponentType<SvelteComponent>;
};
```

_After:_

```typescript
type ShellNavItem = {
  href: string;
  label: string;
  priority: number;
  origin: NavItemOrigin;
  stableId: string;
  badge?: string;
  icon?: ComponentType<SvelteComponent>;
} & (
  | { badgeHref?: undefined; badgeAriaLabel?: undefined }
  | { badgeHref: string; badgeAriaLabel: string }
);
```

- [ ] **Step 3: Replace the `navItems` `$derived` block** — replace lines 254–283:

_Before:_

```typescript
const navItems = $derived(
  [
    ...builtInNavItems
      .filter((item) => {
        if (!item.permission) return true;
        const perms = Array.isArray(item.permission)
          ? item.permission
          : [item.permission];
        return perms.some((p) => getUser()?.permissions.includes(p));
      })
      .map(
        (item): ShellNavItem => ({
          href: item.href,
          label: item.label,
          priority: item.priority,
          origin: "built-in",
          stableId: item.href,
          icon: item.icon,
          badge:
            item.href === "/software"
              ? formatBadge(getUpdatableSoftwareCount())
              : undefined,
        }),
      ),
    ...surfacePageNavItems.map(
      (item): ShellNavItem => ({
        href: item.href,
        label: item.label,
        priority: item.priority,
        origin: "surface.page",
        stableId: item.id,
        icon: resolveIcon(item.icon).component,
      }),
    ),
  ].sort(compareShellNavItems),
);
```

_After:_

```typescript
const navItems = $derived.by(() => {
  const softwareUpdateCount = getUpdatableSoftwareCount();
  return [
    ...builtInNavItems
      .filter((item) => {
        if (!item.permission) return true;
        const perms = Array.isArray(item.permission)
          ? item.permission
          : [item.permission];
        return perms.some((p) => getUser()?.permissions.includes(p));
      })
      .map(
        (item): ShellNavItem => ({
          href: item.href,
          label: item.label,
          priority: item.priority,
          origin: "built-in",
          stableId: item.href,
          icon: item.icon,
          badge:
            item.href === "/software"
              ? formatBadge(softwareUpdateCount)
              : undefined,
          badgeHref:
            item.href === "/software" && softwareUpdateCount
              ? UPDATES_AVAILABLE_HREF
              : undefined,
          badgeAriaLabel:
            item.href === "/software" && softwareUpdateCount
              ? "View software updates available"
              : undefined,
        }),
      ),
    ...surfacePageNavItems.map(
      (item): ShellNavItem => ({
        href: item.href,
        label: item.label,
        priority: item.priority,
        origin: "surface.page",
        stableId: item.id,
        icon: resolveIcon(item.icon).component,
      }),
    ),
  ].sort(compareShellNavItems);
});
```

- [ ] **Step 4: Run the new tests — they should now pass**

```bash
cd frontend && npm run test -- --reporter=verbose layout-button-migration 2>&1 | tail -20
```

Expected: all four new tests pass.

- [ ] **Step 5: Run TypeScript check**

```bash
cd frontend && npm run check 2>&1 | tail -10
```

Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/routes/+layout.svelte \
        frontend/src/routes/layout-button-migration.test.ts
git commit --only frontend/src/routes/+layout.svelte \
           frontend/src/routes/layout-button-migration.test.ts \
  -m "feat(frontend): extend ShellNavItem with badgeHref discriminated union"
```

---

### Task 4: Update desktop sidebar render site (Variant A, no dismiss handler)

**Files:**

- Modify: `frontend/src/routes/+layout.svelte` lines 545–564 (inside `{#each navItems as item}` of
  the desktop `<aside>`)

- [ ] **Step 1: Replace the `<li>` block** — replace lines 545–564:

_Before:_

```svelte
								<li>
									<a
										href={item.href}
										class={`flex h-7 items-center gap-2 rounded-card px-2.5 text-nav-item font-medium tracking-nav transition-colors ${
											isNavItemActive(item)
												? 'bg-[rgba(var(--accent-rgb),0.12)] text-[var(--accent-bright)]'
												: 'text-[var(--text-secondary)] hover:bg-[var(--bg-raised)] hover:text-[var(--text-primary)]'
										}`}
										aria-current={isNavItemActive(item) ? 'page' : undefined}
										data-ui="app-shell-nav-item"
									>
										{#if NavIcon}<NavIcon size={16} aria-hidden="true" />{/if}
										<span>{item.label}</span>
										{#if item.badge}
											<span class="ml-auto pl-1.5">
												<StatusBadge tone="info" label={item.badge} />
											</span>
										{/if}
									</a>
								</li>
```

_After:_

```svelte
								<li class={item.badgeHref ? 'flex items-center' : ''}>
									<a
										href={item.href}
										class={`flex h-7 items-center gap-2 rounded-card px-2.5 text-nav-item font-medium tracking-nav transition-colors ${
											isNavItemActive(item) &&
											!(item.badgeHref && page.url.pathname + page.url.search === item.badgeHref)
												? 'bg-[rgba(var(--accent-rgb),0.12)] text-[var(--accent-bright)]'
												: 'text-[var(--text-secondary)] hover:bg-[var(--bg-raised)] hover:text-[var(--text-primary)]'
										} ${item.badgeHref ? 'flex-1' : ''}`}
										aria-current={isNavItemActive(item) &&
										!(item.badgeHref && page.url.pathname + page.url.search === item.badgeHref)
											? 'page'
											: undefined}
										data-ui="app-shell-nav-item"
									>
										{#if NavIcon}<NavIcon size={16} aria-hidden="true" />{/if}
										<span>{item.label}</span>
										{#if item.badge && !item.badgeHref}
											<span class="ml-auto pl-1.5">
												<StatusBadge tone="info" label={item.badge} />
											</span>
										{/if}
									</a>
									{#if item.badge && item.badgeHref}
										<a
											href={item.badgeHref}
											aria-label={item.badgeAriaLabel}
											aria-current={page.url.pathname + page.url.search === item.badgeHref
												? 'page'
												: undefined}
											class="pl-1.5 shrink-0"
											data-ui="app-shell-nav-badge"
										>
											<StatusBadge tone="info" label={item.badge} />
										</a>
									{/if}
								</li>
```

- [ ] **Step 2: Check TypeScript + run tests**

```bash
cd frontend && npm run check 2>&1 | tail -5 && npm run test -- layout-button-migration 2>&1 | tail -10
```

Expected: no errors, all tests pass.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/routes/+layout.svelte
git commit --only frontend/src/routes/+layout.svelte \
  -m "feat(frontend): update desktop sidebar nav to render badge as separate link"
```

---

### Task 5: Update tablet overlay render site (Variant A, dismiss `sidebarOverlayOpen`)

**Files:**

- Modify: `frontend/src/routes/+layout.svelte` lines 596–616 (inside the tablet `<aside>` overlay)

- [ ] **Step 1: Replace the `<li>` block** — replace lines 596–616:

_Before:_

```svelte
								<li>
									<a
										href={item.href}
										class={`flex h-7 items-center gap-2 rounded-card px-2.5 text-nav-item font-medium tracking-nav transition-colors ${
											isNavItemActive(item)
												? 'bg-[rgba(var(--accent-rgb),0.12)] text-[var(--accent-bright)]'
												: 'text-[var(--text-secondary)] hover:bg-[var(--bg-raised)] hover:text-[var(--text-primary)]'
										}`}
										aria-current={isNavItemActive(item) ? 'page' : undefined}
										data-ui="app-shell-nav-item"
										onclick={() => (sidebarOverlayOpen = false)}
									>
										{#if NavIcon}<NavIcon size={16} aria-hidden="true" />{/if}
										<span>{item.label}</span>
										{#if item.badge}
											<span class="ml-auto pl-1.5">
												<StatusBadge tone="info" label={item.badge} />
											</span>
										{/if}
									</a>
								</li>
```

_After:_

```svelte
								<li class={item.badgeHref ? 'flex items-center' : ''}>
									<a
										href={item.href}
										class={`flex h-7 items-center gap-2 rounded-card px-2.5 text-nav-item font-medium tracking-nav transition-colors ${
											isNavItemActive(item) &&
											!(item.badgeHref && page.url.pathname + page.url.search === item.badgeHref)
												? 'bg-[rgba(var(--accent-rgb),0.12)] text-[var(--accent-bright)]'
												: 'text-[var(--text-secondary)] hover:bg-[var(--bg-raised)] hover:text-[var(--text-primary)]'
										} ${item.badgeHref ? 'flex-1' : ''}`}
										aria-current={isNavItemActive(item) &&
										!(item.badgeHref && page.url.pathname + page.url.search === item.badgeHref)
											? 'page'
											: undefined}
										data-ui="app-shell-nav-item"
										onclick={() => (sidebarOverlayOpen = false)}
									>
										{#if NavIcon}<NavIcon size={16} aria-hidden="true" />{/if}
										<span>{item.label}</span>
										{#if item.badge && !item.badgeHref}
											<span class="ml-auto pl-1.5">
												<StatusBadge tone="info" label={item.badge} />
											</span>
										{/if}
									</a>
									{#if item.badge && item.badgeHref}
										<a
											href={item.badgeHref}
											aria-label={item.badgeAriaLabel}
											aria-current={page.url.pathname + page.url.search === item.badgeHref
												? 'page'
												: undefined}
											class="pl-1.5 shrink-0"
											data-ui="app-shell-nav-badge"
											onclick={() => (sidebarOverlayOpen = false)}
										>
											<StatusBadge tone="info" label={item.badge} />
										</a>
									{/if}
								</li>
```

- [ ] **Step 2: Check TypeScript + run tests**

```bash
cd frontend && npm run check 2>&1 | tail -5 && npm run test -- layout-button-migration 2>&1 | tail -10
```

Expected: no errors, all tests pass.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/routes/+layout.svelte
git commit --only frontend/src/routes/+layout.svelte \
  -m "feat(frontend): update tablet overlay nav to render badge as separate link"
```

---

### Task 6: Update mobile overflow sheet render site (Variant A, dismiss `mobileOverflowOpen`)

**Files:**

- Modify: `frontend/src/routes/+layout.svelte` lines 703–723 (inside the mobile overflow sheet
  `<ul>`)

- [ ] **Step 1: Replace the `<li>` block** — replace lines 703–723:

_Before:_

```svelte
								<li>
									<a
										href={item.href}
										class={`flex h-7 items-center gap-2 rounded-card px-2.5 text-nav-item font-medium tracking-nav transition-colors ${
											isNavItemActive(item)
												? 'bg-[rgba(var(--accent-rgb),0.12)] text-[var(--accent-bright)]'
												: 'text-[var(--text-secondary)] hover:bg-[var(--bg-raised)] hover:text-[var(--text-primary)]'
										}`}
										aria-current={isNavItemActive(item) ? 'page' : undefined}
										data-ui="app-shell-nav-item"
										onclick={() => (mobileOverflowOpen = false)}
									>
										{#if NavIcon}<NavIcon size={16} aria-hidden="true" />{/if}
										<span>{item.label}</span>
										{#if item.badge}
											<span class="ml-auto pl-1.5">
												<StatusBadge tone="info" label={item.badge} />
											</span>
										{/if}
									</a>
								</li>
```

_After:_

```svelte
								<li class={item.badgeHref ? 'flex items-center' : ''}>
									<a
										href={item.href}
										class={`flex h-7 items-center gap-2 rounded-card px-2.5 text-nav-item font-medium tracking-nav transition-colors ${
											isNavItemActive(item) &&
											!(item.badgeHref && page.url.pathname + page.url.search === item.badgeHref)
												? 'bg-[rgba(var(--accent-rgb),0.12)] text-[var(--accent-bright)]'
												: 'text-[var(--text-secondary)] hover:bg-[var(--bg-raised)] hover:text-[var(--text-primary)]'
										} ${item.badgeHref ? 'flex-1' : ''}`}
										aria-current={isNavItemActive(item) &&
										!(item.badgeHref && page.url.pathname + page.url.search === item.badgeHref)
											? 'page'
											: undefined}
										data-ui="app-shell-nav-item"
										onclick={() => (mobileOverflowOpen = false)}
									>
										{#if NavIcon}<NavIcon size={16} aria-hidden="true" />{/if}
										<span>{item.label}</span>
										{#if item.badge && !item.badgeHref}
											<span class="ml-auto pl-1.5">
												<StatusBadge tone="info" label={item.badge} />
											</span>
										{/if}
									</a>
									{#if item.badge && item.badgeHref}
										<a
											href={item.badgeHref}
											aria-label={item.badgeAriaLabel}
											aria-current={page.url.pathname + page.url.search === item.badgeHref
												? 'page'
												: undefined}
											class="pl-1.5 shrink-0"
											data-ui="app-shell-nav-badge"
											onclick={() => (mobileOverflowOpen = false)}
										>
											<StatusBadge tone="info" label={item.badge} />
										</a>
									{/if}
								</li>
```

- [ ] **Step 2: Check TypeScript + run tests**

```bash
cd frontend && npm run check 2>&1 | tail -5 && npm run test -- layout-button-migration 2>&1 | tail -10
```

Expected: no errors, all tests pass.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/routes/+layout.svelte
git commit --only frontend/src/routes/+layout.svelte \
  -m "feat(frontend): update mobile overflow sheet nav to render badge as separate link"
```

---

### Task 7: Update mobile bottom nav render site (Variant B, `<div>` wrapper)

**Files:**

- Modify: `frontend/src/routes/+layout.svelte` lines 645–663 (bare `<a>` inside
  `<div class="mx-auto flex …">`)

The mobile bottom nav has no `<li>`. When `badgeHref` is set, wrap in a `<div class="flex-1 …">` so
`flex-1` moves from the `<a>` to the wrapper.

Note: `/software` has priority 500 and appears 6th in the full nav order — most users see it in the
overflow sheet (Task 6), not here. This Variant B path activates only for permission subsets where
fewer than 4 higher-priority items are visible.

- [ ] **Step 1: Replace the bare `<a>` block** — replace lines 645–663:

_Before:_

```svelte
						<a
							href={item.href}
							class={`flex min-w-0 flex-1 flex-col items-center gap-0.5 justify-center rounded-card px-1 py-1.5 text-center text-nav-item font-medium leading-tight transition-colors ${
								isNavItemActive(item)
									? 'bg-[rgba(var(--accent-rgb),0.12)] text-[var(--accent-bright)]'
									: 'text-[var(--text-secondary)] hover:bg-[var(--bg-raised)] hover:text-[var(--text-primary)]'
							}`}
							aria-current={isNavItemActive(item) ? 'page' : undefined}
							data-ui="app-shell-mobile-nav-item"
							onclick={closeTransientNavigation}
						>
							{#if NavIcon}<NavIcon size={20} aria-hidden="true" />{/if}
							<span class="truncate">{item.label}</span>
							{#if item.badge}
								<span class="mt-0.5 shrink-0 pl-1.5">
									<StatusBadge tone="info" label={item.badge} />
								</span>
							{/if}
						</a>
```

_After:_

```svelte
						{#if item.badgeHref && item.badge}
							<div class="flex min-w-0 flex-1 flex-col items-center">
								<a
									href={item.href}
									class={`flex w-full min-h-[2rem] flex-col items-center gap-0.5 justify-center rounded-card px-1 py-1.5 text-center text-nav-item font-medium leading-tight transition-colors ${
										isNavItemActive(item) &&
										!(page.url.pathname + page.url.search === item.badgeHref)
											? 'bg-[rgba(var(--accent-rgb),0.12)] text-[var(--accent-bright)]'
											: 'text-[var(--text-secondary)] hover:bg-[var(--bg-raised)] hover:text-[var(--text-primary)]'
									}`}
									aria-current={isNavItemActive(item) &&
									!(page.url.pathname + page.url.search === item.badgeHref)
										? 'page'
										: undefined}
									data-ui="app-shell-mobile-nav-item"
									onclick={closeTransientNavigation}
								>
									{#if NavIcon}<NavIcon size={20} aria-hidden="true" />{/if}
									<span class="truncate">{item.label}</span>
								</a>
								<a
									href={item.badgeHref}
									aria-label={item.badgeAriaLabel}
									aria-current={page.url.pathname + page.url.search === item.badgeHref
										? 'page'
										: undefined}
									class="mt-0.5 shrink-0"
									data-ui="app-shell-nav-badge"
									onclick={closeTransientNavigation}
								>
									<StatusBadge tone="info" label={item.badge} />
								</a>
							</div>
						{:else}
							<a
								href={item.href}
								class={`flex min-w-0 flex-1 flex-col items-center gap-0.5 justify-center rounded-card px-1 py-1.5 text-center text-nav-item font-medium leading-tight transition-colors ${
									isNavItemActive(item)
										? 'bg-[rgba(var(--accent-rgb),0.12)] text-[var(--accent-bright)]'
										: 'text-[var(--text-secondary)] hover:bg-[var(--bg-raised)] hover:text-[var(--text-primary)]'
								}`}
								aria-current={isNavItemActive(item) ? 'page' : undefined}
								data-ui="app-shell-mobile-nav-item"
								onclick={closeTransientNavigation}
							>
								{#if NavIcon}<NavIcon size={20} aria-hidden="true" />{/if}
								<span class="truncate">{item.label}</span>
								{#if item.badge}
									<span class="mt-0.5 shrink-0 pl-1.5">
										<StatusBadge tone="info" label={item.badge} />
									</span>
								{/if}
							</a>
						{/if}
```

- [ ] **Step 2: Check TypeScript + run tests**

```bash
cd frontend && npm run check 2>&1 | tail -5 && npm run test -- layout-button-migration 2>&1 | tail -10
```

Expected: no errors, all tests pass.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/routes/+layout.svelte
git commit --only frontend/src/routes/+layout.svelte \
  -m "feat(frontend): update mobile bottom nav to render badge as separate link (Variant B)"
```

---

### Task 8: Full quality gate

**Files:** none (verification only)

- [ ] **Step 1: Run full frontend quality gate**

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build 2>&1 | tail -20
```

Expected: all pass with no errors or warnings.

- [ ] **Step 2: Manual smoke test** — start the dev server and verify in a browser:

```bash
cd frontend && npm run dev
```

Navigate to the Dashboard. With software updates available (or mock the count by temporarily setting
`getUpdatableSoftwareCount` to return `5` in `software-updates.svelte.ts`):

1. The `/software` nav badge renders as a clickable badge separate from the menu label.
2. Clicking the label navigates to `/software` (no filter).
3. Clicking the badge navigates to `/software?updatable=true` (filter checkbox pre-ticked).
4. On `/software?updatable=true`, the badge link shows `aria-current="page"` and the label link does
   not.
5. On mobile (<640px), resize to a viewport where `/software` appears in the bottom nav — badge
   column-stacks below the label without pushing the nav bar height.

## Documentation Impact

No externally observable API, config, or architecture change. No doc updates required (per spec).
