# Nav Badge Separate Navigation

**Date:** 2026-05-08 **Status:** Approved

## Problem

The `/software` nav item shows a badge with available update count. Clicking the badge navigates to `/software` with no filter applied. Users expect
clicking the badge to open the software list pre-filtered to updates available.

## Solution

Extend `ShellNavItem` with optional `badgeHref` and `badgeAriaLabel` fields using a discriminated union so TypeScript enforces that `badgeAriaLabel`
is required when `badgeHref` is set. When `badgeHref` is set, the badge renders as its own `<a>` element alongside the label anchor. When absent,
badge renders inside the parent `<a>` unchanged.

## Data Model

Add to `ShellNavItem` type in `frontend/src/routes/+layout.svelte`:

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

The discriminated union enforces that `badgeAriaLabel` is always provided when `badgeHref` is set.

## Software Item Assignment

The `navItems` `$derived` block builds the item list with a `.filter().map()` chain (~line 255). Call `getUpdatableSoftwareCount()` once **before**
the `.map()` call so the three badge fields read a consistent value:

```typescript
const softwareUpdateCount = getUpdatableSoftwareCount(); // declared before the .map() call, inside the $derived block
// then inside the .map() callback:
badge: item.href === '/software' ? formatBadge(softwareUpdateCount) : undefined,
badgeHref: item.href === '/software' && softwareUpdateCount ? '/software?updatable=true' : undefined,
badgeAriaLabel: item.href === '/software' && softwareUpdateCount ? 'View software updates available' : undefined,
```

`badgeHref` and `badgeAriaLabel` use the same truthiness guard as `formatBadge` so the three fields stay in sync. If `softwareUpdateCount` is `null`
or `0`, `badge` is `undefined` and `badgeHref`/`badgeAriaLabel` are also `undefined`.

**Query param coupling**: The string `'/software?updatable=true'` is hardcoded in the layout. To prevent silent breakage if the software route's
filter param is ever renamed, export a constant from the software route (e.g. `export const UPDATES_AVAILABLE_HREF = '/software?updatable=true'` in
`frontend/src/routes/software/+page.svelte` or a co-located constants file) and import it here. This makes the coupling explicit and compiler-visible.
This adds one additional affected file to the change.

## Render Pattern

There are two structural variants depending on the render site.

### Variant A — Desktop sidebar, Tablet overlay, Mobile overflow sheet (all use `<li>`)

**Without `badgeHref`** (no change):

```svelte
<li>
  <a href={item.href} class="flex h-7 items-center gap-2 …" …>
    {#if NavIcon}<NavIcon … />{/if}
    <span>{item.label}</span>
    {#if item.badge}
      <span class="ml-auto pl-1.5">
        <StatusBadge tone="info" label={item.badge} />
      </span>
    {/if}
  </a>
</li>
```

**With `badgeHref`** (`<li>` becomes flex, badge extracted as sibling link):

```svelte
<li class={item.badgeHref ? 'flex items-center' : ''}>
  <a
    href={item.href}
    class={`flex h-7 items-center gap-2 … ${item.badgeHref ? 'flex-1' : ''}`}
    aria-current={isNavItemActive(item) && !(item.badgeHref && page.url.pathname + page.url.search === item.badgeHref) ? 'page' : undefined}
    onclick={/* site-specific handler — see Dismiss Handlers below */}
    …
  >
    {#if NavIcon}<NavIcon … />{/if}
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
      aria-current={page.url.pathname + page.url.search === item.badgeHref ? 'page' : undefined}
      class="pl-1.5 shrink-0"
      data-ui="app-shell-nav-badge"
      onclick={/* same site-specific handler as label <a> — see Dismiss Handlers below */}
    >
      <StatusBadge tone="info" label={item.badge} />
    </a>
  {/if}
</li>
```

### Variant B — Mobile bottom nav (bare `<a>` flex children, no `<li>`)

The mobile bottom nav flex container (`<div class="mx-auto flex …">`) uses bare `<a>` as direct `flex-1` children. No `<li>` wrapper exists or should
be introduced.

Note: The bottom nav shows only the first 4 items by priority. `/software` has priority 500 (6th after Home/Services/System Services/Hosts/Tags), so
for full-permission users it renders in the overflow sheet (Variant A), not the bottom nav. Variant B activates for permission subsets where fewer
than 4 higher-priority items are visible.

**Without `badgeHref`** (no change):

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

**With `badgeHref`** (`flex-1` moves to wrapper `<div>`; badge becomes sibling `<a>` inside):

```svelte
{#if item.badgeHref && item.badge}
  <div class="flex min-w-0 flex-1 flex-col items-center">
    <a
      href={item.href}
      class={`flex w-full min-h-[2rem] flex-col items-center gap-0.5 justify-center rounded-card px-1 py-1.5 text-center text-nav-item font-medium leading-tight transition-colors ${
        isNavItemActive(item) && !(page.url.pathname + page.url.search === item.badgeHref)
          ? 'bg-[rgba(var(--accent-rgb),0.12)] text-[var(--accent-bright)]'
          : 'text-[var(--text-secondary)] hover:bg-[var(--bg-raised)] hover:text-[var(--text-primary)]'
      }`}
      aria-current={isNavItemActive(item) && !(page.url.pathname + page.url.search === item.badgeHref) ? 'page' : undefined}
      data-ui="app-shell-mobile-nav-item"
      onclick={closeTransientNavigation}
    >
      {#if NavIcon}<NavIcon size={20} aria-hidden="true" />{/if}
      <span class="truncate">{item.label}</span>
    </a>
    <a
      href={item.badgeHref}
      aria-label={item.badgeAriaLabel}
      aria-current={page.url.pathname + page.url.search === item.badgeHref ? 'page' : undefined}
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
    class={`flex min-w-0 flex-1 flex-col … (unchanged)`}
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

The `{#if item.badgeHref && item.badge}` guard ensures TypeScript narrows `item.badge` to `string` before it is passed to `StatusBadge`'s
`label: string` prop.

## Dismiss Handlers

Each render site where the nav is transient (overlay or sheet) has a dismiss handler that must be applied to **both** the label `<a>` and the badge
`<a>`:

| Render site           | Handler                              |
| --------------------- | ------------------------------------ |
| Desktop sidebar       | none (sidebar is persistent)         |
| Tablet overlay        | `() => (sidebarOverlayOpen = false)` |
| Mobile bottom nav     | `closeTransientNavigation`           |
| Mobile overflow sheet | `() => (mobileOverflowOpen = false)` |

## Behavior

- Clicking label/icon navigates to `/software` (no filter)
- Clicking badge navigates to `/software?updatable=true` (filter pre-applied)
- `isNavItemActive` unchanged — checks `pathname` only; active state correct in both cases
- Overlay/sheet closes on badge tap at all transient-nav sites

## Accessibility

- Badge link carries `aria-label="View software updates available"` — screen reader announces destination, not just count
- Badge link carries `aria-current="page"` when `page.url.pathname + page.url.search === item.badgeHref`
- Label `<a>` suppresses `aria-current` when the badge URL is active — prevents two sibling anchors simultaneously claiming `aria-current="page"` in
  the same nav group
- `StatusBadge` unchanged (remains a `<span>`)
- All badge `<a>` elements carry `data-ui="app-shell-nav-badge"`; label `<a>` keeps its existing `data-ui` attribute — treat as distinct selectors in
  tests

## Affected Files

- `frontend/src/routes/+layout.svelte` — type extension + badge assignment + 4 render sites (3 Variant A, 1 Variant B)
- `frontend/src/routes/software/+page.svelte` (or co-located constants file) — exports `UPDATES_AVAILABLE_HREF` constant

## Notes

**Variant B height**: The bottom nav wrapper `<div>` stacks the label `<a>` and badge `<a>` in a column. The label `<a>` gets `min-h-[2rem]` to
maintain a minimum touch target. On narrow viewports (≤375px), verify the badge row does not push the nav bar taller than its single-item siblings and
the label touch target remains ≥24px. A screenshot test against a 375px viewport is recommended.

**Discriminated union scope**: The union enforces `badgeAriaLabel` presence at the type level when `badgeHref` is set. The current software item
callsite happens to use a single ternary that sets both together, so the union is not exercised by this callsite. Its value is forward-looking: it
prevents future callers from setting `badgeHref` without `badgeAriaLabel`.

## Documentation Impact

No externally observable API, config, or architecture change. No doc updates required.
