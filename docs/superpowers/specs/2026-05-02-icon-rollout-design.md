# Icon Rollout — Design

## Goal

Extend lucide-svelte icon usage across the Dashboard beyond the single `Copy`
icon introduced on the Profile page. Primary target is the navigation shell;
secondary targets are the `Callout`, `EmptyState`, and toast/header chrome.

---

## Scope

| Area                            | Change                                                             |
| ------------------------------- | ------------------------------------------------------------------ |
| Nav sidebar (desktop + tablet)  | Icon + label inline on every nav item                              |
| Mobile bottom nav               | Icon stacked above label on primary items                          |
| Mobile overflow sheet           | Icon + label inline (matches sidebar)                              |
| Inline SVGs in layout           | Replace hamburger and theme-toggle SVGs with lucide                |
| `Callout` component             | Always-on tone icon, no new API                                    |
| `EmptyState` component          | New optional `icon` prop                                           |
| Toast dismiss button            | Replace text "Dismiss" with `X` icon                               |
| Header logout button            | Add `LogOut` leading icon                                          |
| `SurfaceDescriptor` (Rust + TS) | New optional `nav_icon` field; resolved via frontend allowlist map |
| `frontend/src/lib/nav-icons.ts` | New: curated allowlist + `resolveNavIcon(name)` helper             |

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

Surface items carry an optional `nav_icon` string (a key from the `SURFACE_NAV_ICONS`
allowlist). At render time the string is resolved to a Svelte component via
`resolveNavIcon(name)` — falls back to `Box` for unknown or absent values.

> **Note:** `<Icon name="string">` from lucide-svelte only sets a CSS class and does
> **not** render SVG paths. All icon rendering — including surface items — must go
> through a statically-imported component, not the `Icon` wrapper's `name` prop.

---

## Data Model Changes

### `ShellNavItem` (frontend, `+layout.svelte`)

Add an optional `icon` field typed as `ComponentType<SvelteComponent>`. Both
built-in and surface items store a resolved component — no discriminated union needed.
Surface items are resolved via `resolveNavIcon` (see below) at the point they are
mapped into `ShellNavItem`.

```ts
import type { ComponentType, SvelteComponent } from "svelte";

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

Built-in items set `icon: House` etc. (static import).
Surface items set `icon: resolveNavIcon(item.icon)`.

A `navIcon` helper snippet renders any item icon. The parameter is capitalized so
Svelte treats it as a component reference, not an HTML element:

```svelte
{#snippet navIcon(NavIcon: ComponentType<SvelteComponent> | undefined)}
  {#if NavIcon}
    <NavIcon size={16} aria-hidden="true" />
  {/if}
{/snippet}
```

Two intermediate `.map()` calls in `+layout.svelte` must also be updated:

1. The `surfacePageNavItems` derived (lines ~182–186) maps `resolveSurfacePageNavItems(...)` results
   into `{ id, href, label, priority }` — add `icon: item.icon`. `item.icon` is always a
   non-optional `string` because `resolveSurfacePageNavItems` already applies the `?? "Box"` default.
2. The `navItems` derived (lines ~209–218) maps surface items into `ShellNavItem` — add
   `icon: resolveNavIcon(item.icon)` to resolve the string to a component.

### `nav-icons.ts` (new file, `frontend/src/lib/nav-icons.ts`)

A curated allowlist of icons plugin authors may declare as `nav_icon`. Exposes
a resolver function used at both render time (frontend) and optionally at
surface registration validation.

```ts
import type { ComponentType, SvelteComponent } from "svelte";
import {
  Box,
  Cpu,
  Database,
  FileText,
  Globe,
  HardDrive,
  History,
  Layers,
  Package,
  Puzzle,
  ScrollText,
  Server,
  ServerCog,
  Settings,
  Shield,
  Tag,
  Tags,
  Wrench,
} from "lucide-svelte";

export const SURFACE_NAV_ICONS: Record<
  string,
  ComponentType<SvelteComponent>
> = {
  Box,
  Cpu,
  Database,
  FileText,
  Globe,
  HardDrive,
  History,
  Layers,
  Package,
  Puzzle,
  ScrollText,
  Server,
  ServerCog,
  Settings,
  Shield,
  Tag,
  Tags,
  Wrench,
};

export function resolveNavIcon(name: string): ComponentType<SvelteComponent> {
  return SURFACE_NAV_ICONS[name] ?? Box;
}
```

The list is intentionally small (~18 icons). Expanding it is a non-breaking
frontend-only change. Plugin authors must pick from this list; unknown names
silently fall back to `Box`.

`SURFACE_NAV_ICONS` is not imported into the main nav bundle on every page — only
the icons actually referenced in the bundle are tree-shaken in. The import of the
full map is contained to `nav-icons.ts`; SvelteKit's bundler will include all listed
icons as a minor one-time cost.

`ComponentType<SvelteComponent>` is used because lucide-svelte 1.0.1 exports class-based
`SvelteComponentTyped` icons (Svelte 4 API). Both types are Svelte-deprecated in favour of
the Svelte 5 `Component` interface, but are the correct pragmatic choice for this library
version. When lucide-svelte ships runes-native exports, migrate the map type to
`Component<IconProps>`.

### `SurfacePageNavItem` (frontend, `registry.svelte.ts`)

Add `icon: string` to the type (non-optional — always provided). `resolveSurfacePageNavItems`
populates it from `surface.nav_icon`, defaulting to `'Box'` when absent:

```ts
export interface SurfacePageNavItem {
  id: string;
  href: string;
  label: string;
  priority: number;
  icon: string;
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

Add a wire validation rule in `crates/shared/wire/src/wire_validate_impls.rs` for
`SurfaceDescriptor::nav_icon`: when `Some`, reject empty strings and strings
longer than 64 characters. Full icon-name validation is intentionally kept in the
frontend allowlist; the Rust layer enforces only format constraints.

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

Add an optional `icon` prop typed as `ComponentType<SvelteComponent>` (from `'svelte'`). When
provided, the icon renders above the title at 32 px in the secondary text colour:

```svelte
import type { ComponentType, SvelteComponent } from 'svelte';

let { title, description, actions, icon: IconComponent }: {
  title: string;
  description?: string;
  actions?: Snippet;
  icon?: ComponentType<SvelteComponent>;
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
