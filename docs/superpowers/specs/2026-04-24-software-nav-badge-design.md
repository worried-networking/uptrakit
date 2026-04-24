# Software Nav Badge

**Status:** Approved — pending implementation

## Overview

Show a count badge next to the "Software" nav item in all shell navigation surfaces
(desktop sidebar, tablet overlay, mobile primary row, mobile overflow sheet) when
there are software items with updates available. No badge when the count is zero or
not yet loaded.

---

## Behaviour

| Count | Badge |
| --- | --- |
| `null` (not yet fetched) | hidden |
| `0` | hidden |
| `1 – 99` | label = `String(count)` |
| `≥ 100` | label = `"99+"` |

Badge is fetched once on page load (after authentication) and does not update
again until the next full page load. Live SSE updates are out of scope for this
spec — the store is designed to support them without layout changes.

---

## Store — `frontend/src/lib/stores/software-updates.svelte.ts`

New file, parallel in shape to `events.svelte.ts`.

```typescript
// Public API
export function getUpdatableSoftwareCount(): number | null
export function fetchUpdatableSoftwareCount(): Promise<void>
```

- `getUpdatableSoftwareCount()` returns a `$state`-backed value: `null` before first
  fetch, `number` after.
- `fetchUpdatableSoftwareCount()` is **idempotent**: if count is already non-null,
  returns early without making a network request. This makes it safe to call from a
  `$effect` that may re-run on user object changes.
- When it does fetch, calls `getSoftwareItems(undefined, 1, undefined, undefined, true)`
  (perPage=1 to minimise payload) and reads `.total`.
  Silently swallows errors — the badge is non-critical.
- No subscriptions wired yet. Future SSE integration adds
  `subscribeToEvent('software_item_updated', ...)` and
  `subscribeToEvent('version_check_completed', ...)` inside this store only; no
  layout changes required.

---

## Layout changes — `frontend/src/routes/+layout.svelte`

### 1. Extend `ShellNavItem`

```typescript
type ShellNavItem = {
  href: string;
  label: string;
  priority: number;
  origin: NavItemOrigin;
  stableId: string;
  badge?: string;   // ← new
};
```

### 2. Fetch on auth

Add a `$effect` triggered by `getUser()`. The store's idempotency guard ensures the
network call happens only once even if the effect re-runs:

```typescript
$effect(() => {
  if (getUser()?.permissions.includes(Permission.ViewSoftware)) {
    void fetchUpdatableSoftwareCount();
  }
});
```

### 3. Inject badge into derived `navItems`

Helper function (local to layout):

```typescript
function formatBadge(count: number | null): string | undefined {
  if (count === null || count === 0) return undefined;
  return count >= 100 ? '99+' : String(count);
}
```

In the `.map()` over `builtInNavItems`, add `badge` for the Software item:

```typescript
.map((item): ShellNavItem => ({
  href: item.href,
  label: item.label,
  priority: item.priority,
  origin: 'built-in',
  stableId: item.href,
  badge: item.href === '/software'
    ? formatBadge(getUpdatableSoftwareCount())
    : undefined,
}))
```

`getUpdatableSoftwareCount()` is reactive (`$state`-backed), so `navItems` re-derives
automatically when the count changes. Surface-originated nav items never carry a badge.

### 4. Render badge in nav templates

There are **four** nav templates. They differ structurally so badge placement differs.

#### Desktop sidebar, tablet overlay sidebar, mobile overflow sheet

These are left-aligned flex rows. Use `ml-auto` to push badge to the trailing edge:

```svelte
<a class="flex h-7 items-center rounded-card px-2.5 text-nav-item font-medium tracking-nav transition-colors ...">
  {item.label}
  {#if item.badge}
    <span class="ml-auto pl-1.5">
      <StatusBadge tone="info" label={item.badge} />
    </span>
  {/if}
</a>
```

#### Mobile primary nav (bottom bar)

Mobile primary items use `justify-center` and wrap the label in
`<span class="truncate">`. Do **not** use `ml-auto` here — it conflicts with
centering. Instead let badge sit inline after the label:

```svelte
<a class="flex min-w-0 flex-1 items-center justify-center rounded-card px-1 py-1.5 ...">
  <span class="truncate">{item.label}</span>
  {#if item.badge}
    <span class="shrink-0 pl-1.5">
      <StatusBadge tone="info" label={item.badge} />
    </span>
  {/if}
</a>
```

`shrink-0` prevents the badge from being squeezed on narrow screens. The label
`truncate` span absorbs any overflow independently.

---

## Design-language compliance

| Rule | How satisfied |
| --- | --- |
| No hardcoded hex/rgb | `StatusBadge tone="info"` uses `--color-info` / `--color-info-bg` / `--color-info-border` |
| Badge radius | `rounded-badge` (2px) — via `StatusBadge` |
| Badge typography | `text-badge font-bold uppercase tracking-badge` — via `StatusBadge` |
| Badge min-height | `min-h-badge` (14px) — via `StatusBadge` |
| No new primitive | Reuses existing `StatusBadge` |

---

## Out of scope

- SSE live-update of count (future; store is already designed for it)
- Badge on any nav item other than Software
- Server-side fetch in `+layout.ts`
- Snapshot tests for nav badge (follow-on)
