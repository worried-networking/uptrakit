<!-- markdownlint-disable MD013 -->

# Design Tokens

**Status:** `Implemented` (all sections unless noted)

All styling uses semantic CSS custom properties. Never use raw color values or Tailwind palette
utilities (`text-zinc-500`, `bg-slate-900`, etc.) where a semantic token exists. Hardcoded hex
values are a bug.

---

## Dark Theme

| Role | Token | Value |
| --- | --- | --- |
| Page background | `--bg-base` | `#09090b` |
| Sidebar / card surface | `--bg-surface` | `#111113` |
| Elevated surface | `--bg-raised` | `#18181b` |
| Subtle border | `--border-subtle` | `#1c1c1f` |
| Standard border | `--border-default` | `#27272a` |
| Muted text | `--text-muted` | `#52525b` |
| Secondary text | `--text-secondary` | `#a1a1aa` |
| Primary text | `--text-primary` | `#e4e4e7` |
| Inverted text | `--text-inverted` | `#fafafa` |
| Accent | `--accent` | `#06b6d4` |
| Accent RGB (space-separated) | `--accent-rgb` | `6 182 212` |
| Accent bright | `--accent-bright` | `#22d3ee` |
| Accent dark | `--accent-dark` | `#0891b2` |
| Accent deep | `--accent-deep` | `#0e7490` |
| Success | `--color-success` | `#4ade80` |
| Success background tint | `--color-success-bg` | `rgba(74,222,128,.10)` |
| Success border | `--color-success-border` | `rgba(74,222,128,.25)` |
| Warning | `--color-warning` | `#fbbf24` |
| Warning background tint | `--color-warning-bg` | `rgba(251,191,36,.12)` |
| Warning border | `--color-warning-border` | `rgba(251,191,36,.3)` |
| Error | `--color-error` | `#fdba74` |
| Error background tint | `--color-error-bg` | `rgba(234,88,12,.15)` |
| Error border | `--color-error-border` | `rgba(234,88,12,.35)` |
| Info | `--color-info` | `#67e8f9` |
| Info background tint | `--color-info-bg` | `rgba(6,182,212,.10)` |
| Info border | `--color-info-border` | `rgba(6,182,212,.22)` |

---

## Light Theme

| Role | Token | Value |
| --- | --- | --- |
| Page background | `--bg-base` | `#f8fafc` |
| Sidebar / card surface | `--bg-surface` | `#ffffff` |
| Elevated surface | `--bg-raised` | `#f1f5f9` |
| Subtle border | `--border-subtle` | `#e2e8f0` |
| Standard border | `--border-default` | `#cbd5e1` |
| Muted text | `--text-muted` | `#94a3b8` |
| Secondary text | `--text-secondary` | `#64748b` |
| Primary text | `--text-primary` | `#0f172a` |
| Inverted text | `--text-inverted` | `#ffffff` |
| Accent | `--accent` | `#2563eb` |
| Accent RGB (space-separated) | `--accent-rgb` | `37 99 235` |
| Accent bright | `--accent-bright` | `#3b82f6` |
| Accent dark | `--accent-dark` | `#1d4ed8` |
| Accent deep | `--accent-deep` | `#1e40af` |
| Success | `--color-success` | `#16a34a` |
| Success background tint | `--color-success-bg` | `rgba(22,163,74,.08)` |
| Success border | `--color-success-border` | `rgba(22,163,74,.3)` |
| Warning | `--color-warning` | `#d97706` |
| Warning background tint | `--color-warning-bg` | `rgba(217,119,6,.08)` |
| Warning border | `--color-warning-border` | `rgba(217,119,6,.28)` |
| Error | `--color-error` | `#dc2626` |
| Error background tint | `--color-error-bg` | `rgba(220,38,38,.07)` |
| Error border | `--color-error-border` | `rgba(220,38,38,.3)` |
| Info | `--color-info` | `#0891b2` |
| Info background tint | `--color-info-bg` | `rgba(8,145,178,.08)` |
| Info border | `--color-info-border` | `rgba(8,145,178,.22)` |

---

## Typography

Font stacks:

- Sans-serif: `-apple-system, BlinkMacSystemFont, 'Segoe UI', 'Inter', sans-serif`
- Monospace: `'SF Mono', 'Roboto Mono', monospace`
- No custom web fonts loaded.

Heading scale:

| Element | Size | Weight | Color |
| --- | --- | --- | --- |
| `h1` (page title) | `20px` | `700` | `--text-primary` |
| `h2` (section heading) | `16px` | `700` | `--text-primary` |
| `h3` (subsection heading) | `13px` | `700` | `--text-primary` |

Do not use Skeleton/framework heading utility classes (`h3`, `h4`, etc.). Write explicit Tailwind
classes: `text-[13px] font-bold text-[var(--text-primary)]`.

UI chrome uses a compressed scale:

| Use | Size | Weight |
| --- | --- | --- |
| Nav section headers | `7.5px` | `700` uppercase |
| Nav items | `10px` | `500` |
| Table headers | `11px` | `600` uppercase |
| Table body | `10–12px` | `400` |
| Badge / pill labels | `7.5px` | `700` uppercase |
| Button labels | `9px` (`sm`: `8.5px`) | `700` uppercase |
| Top bar title | `12px` | `700` |
| Form labels | `14px` (`text-sm`) | `500` |

---

## Border Radius

| Element | Radius |
| --- | --- |
| Page panels, modals, sidebar | `4px` |
| Terminal modal window | `6px` |
| Cards, table wrappers, buttons | `3px` |
| Badges, pills, small chips | `2px` |
| Traffic light dots | `50%` |
| Toggle track | `10px` |

Use `rounded-[4px]`, `rounded-[3px]`, `rounded-[2px]` in Tailwind. Do not use shorthand
`rounded-lg`, `rounded-md`, or other scale classes.

---

## Transitions

All interactive controls use one flat transition triplet:

```css
transition: background 0.12s, border-color 0.12s, color 0.12s;
```

Tailwind: `transition-[background,border-color,color] duration-[120ms]`

Allowed animated properties:

| Category | Properties |
| --- | --- |
| Interactive controls | `background`, `border-color`, `color` |
| Loading affordances | `opacity`, `transform`, `background-position` |
| Toast progress bar | `transform: scaleX()` |
| Terminal maximize | `width`, `height` |

Rules:

- No hover transforms on ordinary controls.
- No hover shadows on ordinary controls.
- Controls remain visually flat at rest and on hover.
- Shared shell links use the same triplet; transform transitions are forbidden there.

---

## Focus States

All focusable controls must suppress the browser default outline and use this focus ring:

```css
outline: none;
box-shadow: 0 0 0 3px rgba(var(--accent-rgb), .25);
```

Tailwind: `focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]`

Rules:

- Show focus ring on `:focus-visible` only — never on mouse click.
- Error-state fields keep the error border and also gain the accent focus ring.

---

## Z-Index Scale

| Layer | Value | Use |
| --- | --- | --- |
| Base content | `0` | Normal page content |
| Sticky top bar | `10` | Shell top bar |
| Sidebar | `20` | Tablet overlay sidebar |
| Dropdown / tooltip | `100` | Inline popovers |
| Toast stack | `500` | Toasts |
| Modal backdrop | `900` | Dialog or terminal backdrop |
| Modal content | `910` | Dialog or terminal window |

Do not invent ad hoc z-index values when one of these applies.

---

## Runtime Token Adapter

**Status:** `Implemented`

Semantic tokens are the design contract. The runtime adapter is the enforcement layer.

| Artifact | Path |
| --- | --- |
| Manifest | `frontend/src/theme/adapter-manifest.json` |
| Completeness test | `frontend/src/lib/theme/adapter-manifest.test.ts` |

Family-level mapping from Skeleton/framework utilities to semantic tokens:

| Framework utility family | Semantic token family |
| --- | --- |
| `bg-surface-*` | `--bg-base`, `--bg-surface`, `--bg-raised` |
| `border-surface-*` | `--border-subtle`, `--border-default` |
| `text-surface-*` | `--text-muted`, `--text-secondary`, `--text-primary` |
| `primary-*` preset/tonal utilities | `--accent`, `--accent-*`, `--accent-rgb` |
| `preset-filled-success-*` / `success-*` | `--color-success-*` |
| `preset-filled-warning-*` / `warning-*` | `--color-warning-*` |
| `preset-filled-error-*` / `error-*` | `--color-error-*` |
| `info-*` / info preset utilities | `--color-info-*` |

Conformance rules:

- Every semantic token from the dark and light tables above must exist in the manifest.
- CI must fail if any required token is missing.
- Built-in and surface-backed UI must consume the same adapter.
- No one-off raw color classes where an equivalent semantic token exists.
- `preset-filled-*`, `preset-tonal-*`, `text-surface-*`, `bg-surface-*`, `border-surface-*` Skeleton
  utilities are forbidden in component and route files — replace with semantic token classes.

---

## Accent Tint Patterns

When you need accent-tinted backgrounds (e.g. active tab, highlight), use the `--accent-rgb`
space-separated value with the `rgba()` function:

```html
<!-- Active state background -->
<div class="bg-[rgba(var(--accent-rgb),0.12)] text-[var(--accent-bright)]"></div>

<!-- Subtle info card border -->
<div class="border border-[rgba(var(--accent-rgb),0.15)]"></div>
```

Do not use `bg-primary-100`, `text-primary-500`, or similar Tailwind palette variants.
