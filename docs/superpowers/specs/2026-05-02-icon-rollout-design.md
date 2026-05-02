# Icon Rollout — Design

## Goal

Extend lucide-svelte icon usage across the Dashboard beyond the single `Copy`
icon introduced on the Profile page. Primary target is the navigation shell;
secondary targets are the `Callout`, `EmptyState`, and toast/header chrome.

---

## Scope

| Area                            | Change                                                   |
| ------------------------------- | -------------------------------------------------------- |
| Nav sidebar (desktop + tablet)  | Icon + label inline on every nav item                    |
| Mobile bottom nav               | Icon stacked above label on primary items                |
| Mobile overflow sheet           | Icon + label inline (matches sidebar)                    |
| Inline SVGs in layout           | Replace hamburger and theme-toggle SVGs with lucide      |
| `Callout` component             | Always-on tone icon, no new API                          |
| `EmptyState` component          | New optional `icon` prop                                 |
| Toast dismiss button            | Replace text "Dismiss" with `X` icon                     |
| Header logout button            | Add `LogOut` leading icon                                |
| `SurfaceDescriptor` (Rust + TS) | New optional `nav_icon` field; renders via `<Icon name>` |

---

## Nav Icon Mapping

### Built-in items

| Nav item        | href               | lucide icon  |
| --------------- | ------------------ | ------------ |
| Home            | `/`                | `House`      |
| Services        | `/services`        | `Server`     |
| System Services | `/system-services` | `ServerCog`  |
| Hosts           | `/hosts`           | `HardDrive`  |
| Tags            | `/host-tags`       | `Tags`       |
| Software        | `/software`        | `Package`    |
| History         | `/history`         | `History`    |
| Audit Logs      | `/audit-logs`      | `ScrollText` |
| Settings        | `/settings`        | `Settings`   |

### Surface-supplied items

Surface items carry an optional `nav_icon` string (lucide icon name). When
present the Dashboard renders it via `<Icon name={nav_icon} size={16} />`. When
absent the Dashboard falls back to `Box`.

---

## Data Model Changes

### `ShellNavItem` (frontend, `+layout.svelte`)

Add an optional `icon` field. Built-in items carry a statically-imported
Svelte component; surface items carry the resolved icon name as a string to be
rendered with `<Icon name>`.

Because the two sources need different rendering paths, split `icon` into:

```ts
import type { Component } from "svelte";

type ShellNavIcon =
  | { kind: "component"; component: Component }
  | { kind: "named"; name: string };

type ShellNavItem = {
  href: string;
  label: string;
  priority: number;
  origin: NavItemOrigin;
  stableId: string;
  badge?: string;
  icon?: ShellNavIcon;
};
```

Built-in items set `icon: { kind: 'component', component: House }` etc.
Surface items set `icon: { kind: 'named', name: navIcon }` where `navIcon`
defaults to `'Box'` when `nav_icon` is absent on the descriptor.

A single `NavIcon` helper snippet renders both:

```svelte
{#snippet navIcon(icon: ShellNavIcon | undefined)}
  {#if icon?.kind === 'component'}
    <svelte:component this={icon.component} size={16} aria-hidden="true" />
  {:else if icon?.kind === 'named'}
    <Icon name={icon.name} size={16} aria-hidden="true" />
  {/if}
{/snippet}
```

`Icon` is imported from `lucide-svelte` (re-exported from the main bundle).

### `SurfacePageNavItem` (frontend, `registry.svelte.ts`)

Add `icon?: string` to the type. `resolveSurfacePageNavItems` populates it from
`surface.nav_icon`, defaulting to `'Box'` when absent:

```ts
export interface SurfacePageNavItem {
  id: string;
  href: string;
  label: string;
  priority: number;
  icon?: string;
}
```

```ts
navItems.push({
  id: surface.surface_id,
  href: `/surfaces/${surface.surface_id}`,
  label: surface.label,
  priority: surface.priority,
  icon: surface.nav_icon ?? "Box",
});
```

### `SurfaceDescriptor` TS interface (frontend, `contract.ts`)

```ts
export interface SurfaceDescriptor {
  // ... existing fields
  nav_icon?: string;
}
```

### `SurfaceDescriptor` Rust struct (`crates/shared/surfaces/src/surface.rs`)

Add the field after `context_selector` in the struct and builder:

```rust
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceDescriptor {
    // ... existing fields ...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_selector: Option<SurfaceContextSelectorDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nav_icon: Option<String>,
}
```

Add `nav_icon: Option<String>` to `SurfaceDescriptorBuilder`, a builder setter
`nav_icon(impl Into<String>) -> Self`, and wire it in `build()` as
`nav_icon: self.nav_icon`. The field is purely optional — `build()` does not
require it.

---

## Nav Rendering

### Desktop sidebar + tablet overlay sidebar

Current: `<a>` contains `{item.label}` only.

After: `<a>` contains `{@render navIcon(item.icon)}<span>{item.label}</span>`.
Icon and label are side-by-side with a `gap-2` flex row. Icon size 16 px.
No change to `h-7` row height.

### Mobile primary nav

Current: `<a>` contains `<span class="truncate">{item.label}</span>`.

After: stack icon above label using a `flex flex-col items-center gap-0.5`
wrapper. Icon size 20 px. When a badge is present it renders in a relative
wrapper below the label — `<span class="mt-0.5"><StatusBadge .../></span>` — so
the column order is icon → label → badge from top to bottom.

### Mobile overflow sheet

Same pattern as sidebar (icon + label inline, 16 px).

### "More" overflow toggle

The "More" button has no corresponding nav item icon — it stays text-only
(no change).

---

## Inline SVG Replacement

Replace the two raw SVG blobs in `+layout.svelte` with lucide imports:

| Current                        | Replacement                                |
| ------------------------------ | ------------------------------------------ |
| Hamburger SVG (sidebar toggle) | `<Menu size={16} aria-hidden="true" />`    |
| Sun SVG (`light` theme)        | `<Sun size={20} aria-hidden="true" />`     |
| Moon SVG (`dark` theme)        | `<Moon size={20} aria-hidden="true" />`    |
| Monitor SVG (`system` theme)   | `<Monitor size={20} aria-hidden="true" />` |

All four icons are statically imported from `lucide-svelte`.

---

## `Callout` Component

Add a tone-to-icon map and render the icon to the left of the text content.
No new props — icon is derived from the existing `tone` prop.

| Tone      | Icon            | Rationale                      |
| --------- | --------------- | ------------------------------ |
| `info`    | `Info`          | Standard information indicator |
| `success` | `CircleCheck`   | Affirming check in a circle    |
| `warning` | `TriangleAlert` | Standard caution indicator     |
| `danger`  | `OctagonAlert`  | High-severity stop/alert       |

Layout: the `<aside>` body becomes a flex row (`flex items-start gap-3`). Icon
column is `shrink-0 mt-0.5` at 16 px to optically align with the first line of
text. Text column is `flex-1 min-w-0`.

The `role="alert"` / `role="status"` assignment is unchanged.

---

## `EmptyState` Component

Add an optional `icon` prop typed as `Component` (from `'svelte'`). When
provided, the icon renders above the title at 32 px in the secondary text colour:

```svelte
import type { Component } from 'svelte';

let { title, description, actions, icon: IconComponent }: {
  title: string;
  description?: string;
  actions?: Snippet;
  icon?: Component;
} = $props();
```

```svelte
{#if IconComponent}
  <div class="mb-3 flex justify-center">
    <IconComponent size={32} class="text-[var(--text-muted)]" aria-hidden="true" />
  </div>
{/if}
```

Callers opt in with the most contextually appropriate icon (e.g. `Inbox` for
"no messages", `SearchX` for "no results", `Package` for "no software items").
No default icon is applied when the prop is omitted — the current dashed-border
card renders as before.

No existing call sites are required to be updated; icon enrichment is additive.

---

## Toast Dismiss Button

Replace `<Button variant="ghost" size="sm" onclick={...}>Dismiss</Button>` with
an icon-only button:

```svelte
<Button variant="ghost" size="sm" ariaLabel="Dismiss" onclick={...}>
  {#snippet leadingIcon()}
    <X size={14} aria-hidden="true" />
  {/snippet}
</Button>
```

The `Button` component already renders `leadingIcon`-only buttons correctly.

---

## Header Logout Button

Add `LogOut` as a leading icon on the existing logout button in the header:

```svelte
<Button variant="danger" onclick={handleLogout}>
  {#snippet leadingIcon()}
    <LogOut size={14} aria-hidden="true" />
  {/snippet}
  Logout
</Button>
```

---

## Accessibility

- All decorative icons carry `aria-hidden="true"`.
- Icon-only buttons (dismiss) carry `ariaLabel` on the `Button` wrapper.
- Nav icons are decorative (the `<a>` label and `aria-current` convey state);
  no additional `aria-label` is needed on icon elements.

---

## Out of Scope

- Dynamic icon picker for Operators (surface icon is set by plugin authors, not
  by Operators at runtime).
- Icon support in slots other than `surface.page` (e.g. entity-tab surfaces).
- Updating existing `EmptyState` call sites to pass icons (additive — callers
  can adopt over time).
