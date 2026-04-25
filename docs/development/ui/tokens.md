<!-- markdownlint-disable MD013 -->

# Design Tokens

**Status:** `Implemented` (all sections unless noted)

All styling uses semantic CSS custom properties and named Tailwind utilities. The following are
bugs, not style choices:

- Hardcoded hex or rgb color values anywhere in component or route files
- Tailwind palette utilities (`text-zinc-500`, `bg-slate-900`, etc.) where a semantic token exists
- Arbitrary pixel values (`text-[11px]`, `rounded-[4px]`, `tracking-[0.12em]`, `duration-[120ms]`,
  etc.) for any role that has a named utility in `app.css`
- Inventing a new hardcoded pixel value before checking whether a token already covers the role

**Before writing any hardcoded value:** check `app.css` `@theme` and `@utility` blocks and this
document. If a token exists, use it. If none exists and the value appears in more than one place,
add a token first. Isolated one-off values (e.g. a single modal's max-height) are acceptable
as-is and do not need a token.

---

## Dark Theme

| Role | Token | Value |
| --- | --- | --- |
| Page background | `--bg-base` | `#09090b` |
| Sidebar / card surface | `--bg-surface` | `#111113` |
| Elevated surface | `--bg-raised` | `#18181b` |
| Hover surface | `--bg-hover` | `#1e1e22` |
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
| Danger | `--color-danger` | `#fdba74` |
| Danger background tint | `--color-danger-bg` | `rgba(234,88,12,.15)` |
| Danger border | `--color-danger-border` | `rgba(234,88,12,.35)` |
| Danger background tint (hover) | `--color-danger-bg-hover` | `rgba(234,88,12,.22)` |
| Danger border (hover) | `--color-danger-border-hover` | `rgba(234,88,12,.50)` |
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
| Hover surface | `--bg-hover` | `#eef1f5` |
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
| Danger | `--color-danger` | `#dc2626` |
| Danger background tint | `--color-danger-bg` | `rgba(220,38,38,.07)` |
| Danger border | `--color-danger-border` | `rgba(220,38,38,.3)` |
| Danger background tint (hover) | `--color-danger-bg-hover` | `rgba(220,38,38,.14)` |
| Danger border (hover) | `--color-danger-border-hover` | `rgba(220,38,38,.45)` |
| Info | `--color-info` | `#0891b2` |
| Info background tint | `--color-info-bg` | `rgba(8,145,178,.08)` |
| Info border | `--color-info-border` | `rgba(8,145,178,.22)` |

---

## Typography

Font stacks:

- Sans-serif: `-apple-system, BlinkMacSystemFont, 'Segoe UI', 'Inter', sans-serif`
- Monospace: `'SF Mono', 'Roboto Mono', monospace`
- No custom web fonts loaded.

Heading scale (page content):

| Element | Size | Weight | Color | Example |
| --- | --- | --- | --- | --- |
| `h1` (page title) | `20px` | `700` | `--text-primary` | `PageShell` title |
| `h2` (section heading) | `18px` | `600` | `--text-primary` | `SectionCard` title, `EmptyState` title |
| `h3` (subsection heading) | `13px` | `700` | `--text-primary` | Inline card headings |

Public entry shell (`/login`, `/register`) uses `24px`/`600` for its `h1`. Do not replicate this
size in authenticated routes.

Do not use Skeleton/framework heading utility classes (`h3`, `h4`, etc.). Do not use Tailwind
scale classes (`text-lg`, `text-xs`, etc.) for any size where deviating from the spec is a visual
bug. Use the named typography utilities defined in `app.css` via `@theme` (see table below).

`text-sm` (14px) is acceptable for body copy, form labels, and descriptive prose — anywhere the
exact pixel value is not load-bearing.

Heading scale utilities:

| Element | Utility | Weight | Color |
| --- | --- | --- | --- |
| `h1` (page title) | `text-page-title` | `font-bold` | `text-[var(--text-primary)]` |
| `h2` (section heading) | `text-section-title` | `font-semibold` | `text-[var(--text-primary)]` |
| `h3` (subsection heading) | `text-subsection-title` | `font-bold` | `text-[var(--text-primary)]` |
| Public entry `h1` | `text-entry-title` | `font-semibold` | `text-[var(--text-primary)]` |

UI chrome uses a compressed scale:

| Use | Utility | Weight |
| --- | --- | --- |
| Nav section headers | `text-nav-section` | `font-bold` uppercase |
| Nav items | `text-nav-item` | `font-medium` |
| Table headers | `text-table-header` | `font-semibold` uppercase |
| Table body | `text-table-body` | `font-normal` |
| Badge / pill labels | `text-badge` | `font-bold` uppercase |
| Button labels | `text-button` (`sm`: `text-button-sm`) | `font-bold` uppercase |
| Top bar title | `text-topbar` | `font-bold` |
| Form labels | `text-sm` | `font-medium` |

All utilities are defined in `frontend/src/app.css` via `@theme`. Do not use raw `text-[Npx]`
arbitrary values for any of these roles.

---

## Border Radius

| Element | Utility | Value |
| --- | --- | --- |
| Page panels, modals, sidebar | `rounded-panel` | `4px` |
| Terminal modal window | `rounded-terminal` | `6px` |
| Cards, table wrappers, buttons, inputs | `rounded-card` | `3px` |
| Badges, pills, small chips | `rounded-badge` | `2px` |
| Traffic light dots | `rounded-full` | `50%` |
| Toggle track | `rounded-toggle` | `10px` (no dedicated toggle component; boolean settings use `Checkbox`) |

Do not use shorthand scale classes (`rounded-lg`, `rounded-md`, `rounded-2xl`, etc.) or raw
`rounded-[Npx]` arbitrary values. Use the named utilities above.

---

## Letter Spacing

| Use | Utility | Value |
| --- | --- | --- |
| Table headers | `tracking-table-header` | `0.12em` |
| Page / section eyebrows | `tracking-eyebrow` | `0.24em` |
| Badge / status labels | `tracking-badge` | `0.04em` |
| Pill labels | `tracking-pill` | `0.08em` |
| Sidebar nav items | `tracking-nav` | `0.01em` |

Do not use raw `tracking-[Nem]` arbitrary values for these roles.

---

## Spacing Utilities

Compound padding patterns defined via `@utility` in `app.css`. Use these instead of combining
raw padding classes for these specific contexts.

| Utility | Value | Use |
| --- | --- | --- |
| `content-padding` | `padding: 12px 14px` | Standard content areas, form sections |
| `content-padding-x` | `padding-left/right: 14px` | Horizontal only |
| `content-padding-y` | `padding-top/bottom: 12px` | Vertical only |
| `table-cell-pad` | `padding: 12px 10px` | `<td>` cells in DataTable custom rows |
| `card-padding` | `padding: 16px 20px` | SectionCard header and body sections |
| `min-h-badge` | `min-height: 14px` | StatusBadge, PillBadge, ActionBadge |

Standard Tailwind grid values (`py-3`, `px-4`, `gap-2`, etc.) remain as-is — no token needed
for on-grid values.

---

## Component Sizing

Named utilities for off-grid component dimensions:

| Utility | Value | Use |
| --- | --- | --- |
| `w-sidebar` | `180px` | Shell sidebar width |

Button heights (`h-[23px]` md, `h-[19px]` sm) and spinner size (`h-[9px] w-[9px]`) are
confined to `Button.svelte` — acceptable raw values within that single component file.

---

## Opacity

| Utility | Value | Use |
| --- | --- | --- |
| `opacity-pressed` | `88%` | Button active/pressed state |

---

## Transitions

All interactive controls use one flat transition triplet:

```css
transition: background 0.12s, border-color 0.12s, color 0.12s;
```

Tailwind: `transition-[background,border-color,color] duration-fast`

Use `duration-fast` (`120ms`) for all interactive control transitions. Do not write
`duration-[120ms]` or `duration-[0.12s]` directly.

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
| Toast stack | `920` | Toasts (floats above modals and terminal) |
| Modal backdrop | `900` | Dialog or terminal backdrop |
| Modal content | `910` | Dialog or terminal window |

Do not invent ad hoc z-index values when one of these applies.

---

## Runtime Token Adapter

**Status:** `Implemented`

Semantic tokens are the design contract. The runtime adapter is the enforcement layer.

| Artifact | Path |
| --- | --- |
| Token definitions | `frontend/src/theme/tokens.ts` |
| Value completeness tests | `frontend/src/lib/theme/design-token-values.test.ts`, `frontend/src/theme/tokens.test.ts` |
| CSS contract (z-index, transitions) | `frontend/src/lib/theme/css-contract.test.ts` |

Family-level mapping from Skeleton/framework utilities to semantic tokens:

| Framework utility family | Semantic token family |
| --- | --- |
| `bg-surface-*` | `--bg-base`, `--bg-surface`, `--bg-raised` |
| `border-surface-*` | `--border-subtle`, `--border-default` |
| `text-surface-*` | `--text-muted`, `--text-secondary`, `--text-primary` |
| `primary-*` preset/tonal utilities | `--accent`, `--accent-*`, `--accent-rgb` |
| `preset-filled-success-*` / `success-*` | `--color-success-*` |
| `preset-filled-warning-*` / `warning-*` | `--color-warning-*` |
| `preset-filled-error-*` / `error-*` | `--color-danger-*` |
| `info-*` / info preset utilities | `--color-info-*` |

**Adding a new token:** update `TokenName` union in `tokens.ts`, add dark/light values to the token
map in the same file, then add the token to both `design-token-values.test.ts` and `tokens.test.ts`.
All files must be updated together — `css-contract.test.ts` covers z-index and transitions only and
does not need to be changed for a new color or surface token.

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
