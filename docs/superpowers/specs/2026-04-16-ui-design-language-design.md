# UI Design Language

**Date:** 2026-04-16
**Status:** Approved

## Overview

This document defines the visual design language for the uptrakit web UI.
It covers design tokens, component patterns, interactive conventions, and page-level layouts.
The goal is a coherent, dark-native interface that feels sharp and professional — not decorative.

This design language applies equally to built-in product UI and to the shared Surfaces runtime.
The old Extension Framework terminology is retired; any page, panel, tab, context-menu launcher,
or other slot-backed surface content contributed through the shared surface model must look,
behave, and feel native.
Surface-backed UI and built-in UI must be visually indistinguishable from one another unless a
difference is required by product meaning rather than implementation origin.

This document mixes **current contract coverage** and **target visual direction**. Where the current
runtime has not yet converged to the target shell behavior, the relevant section explicitly labels
its status as `Implemented`, `Transitional`, or `Target`.

Status meanings:

- `Implemented`: current required behavior; built-in and surface-backed UI must match it now
- `Transitional`: both built-in and surface-backed UI must stay visually matched to the currently
  shipped host pattern until a paired rollout moves both origins to the target pattern
- `Target`: approved future-state design; not required until promoted, but it must not contradict the
  current registered slot/runtime contract

Enforcement rule:

- CI parity gates in this document are mandatory for `Implemented` and `Transitional` sections now
- `Target` sections become mandatory only when they are promoted in the spec or called out by an
  implementation-linked rollout decision
- Any normative section or subsection without an explicit status label is `Implemented` by default

Allowed parity exceptions are narrow and explicit:

- Security or destructive-action affordances
- Permission-denied states
- Provider-selection controls required only for targeted surfaces
- Contract-limited transitional seams explicitly documented in this spec

Every exception must be documented in the spec section that uses it and covered by parity-focused
visual regression tests.

---

## 1. Themes

The UI supports dark and light themes. **Dark is the default** when system preference is
unavailable; otherwise the theme follows `prefers-color-scheme`.

A theme switcher is available in the UI for manual override.

Both themes are fully specified. Light is not an afterthought — it must be usable and visually
comparable to dark. Text on its intended background tokens must meet WCAG AA contrast requirements.

---

## 2. Design Tokens

### 2.1 Color — Dark Theme

| Role | Token | Value |
| --- | --- | --- |
| Page background | `--bg-base` | `#09090b` |
| Sidebar / card surface | `--bg-surface` | `#111113` |
| Elevated surface (header rows, hover) | `--bg-raised` | `#18181b` |
| Subtle border | `--border-subtle` | `#1c1c1f` |
| Standard border | `--border-default` | `#27272a` |
| Muted text | `--text-muted` | `#52525b` |
| Secondary text | `--text-secondary` | `#a1a1aa` |
| Primary text | `--text-primary` | `#e4e4e7` |
| Inverted (on accent fills) | `--text-inverted` | `#fafafa` |
| **Accent** | `--accent` | `#06b6d4` |
| Accent RGB components | `--accent-rgb` | `6 182 212` |
| Accent bright | `--accent-bright` | `#22d3ee` |
| Accent dark | `--accent-dark` | `#0891b2` |
| Accent deep | `--accent-deep` | `#0e7490` |
| Success | `--color-success` | `#4ade80` |
| Success background tint | `--color-success-bg` | `rgba(74,222,128,.10)` |
| Success border | `--color-success-border` | `rgba(74,222,128,.25)` |
| Warning | `--color-warning` | `#fbbf24` |
| Warning background tint | `--color-warning-bg` | `rgba(251,191,36,.12)` |
| Warning border | `--color-warning-border` | `rgba(251,191,36,.3)` |
| **Error** | `--color-error` | `#fdba74` |
| Error background tint | `--color-error-bg` | `rgba(234,88,12,.15)` |
| Error border | `--color-error-border` | `rgba(234,88,12,.35)` |
| In-progress / info | `--color-info` | `#67e8f9` |
| Info background tint | `--color-info-bg` | `rgba(6,182,212,.10)` |
| Info border | `--color-info-border` | `rgba(6,182,212,.22)` |

> **Note on error tokens:** `--color-error` (`#fdba74`, orange-300) is the display text color,
> chosen for readability on dark backgrounds. The `-bg` and `-border` tints use a deeper
> orange-600 base `rgb(234,88,12)` for the background wash. This is intentional — the two
> values are from different stops of the same orange scale.

### 2.2 Color — Light Theme

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
| Inverted (on accent fills) | `--text-inverted` | `#ffffff` |
| **Accent** | `--accent` | `#2563eb` |
| Accent RGB components | `--accent-rgb` | `37 99 235` |
| Accent bright | `--accent-bright` | `#3b82f6` |
| Accent dark | `--accent-dark` | `#1d4ed8` |
| Accent deep | `--accent-deep` | `#1e40af` |
| Success | `--color-success` | `#16a34a` |
| Success background tint | `--color-success-bg` | `rgba(22,163,74,.08)` |
| Success border | `--color-success-border` | `rgba(22,163,74,.3)` |
| Warning | `--color-warning` | `#d97706` |
| Warning background tint | `--color-warning-bg` | `rgba(217,119,6,.08)` |
| Warning border | `--color-warning-border` | `rgba(217,119,6,.28)` |
| **Error** | `--color-error` | `#dc2626` |
| Error background tint | `--color-error-bg` | `rgba(220,38,38,.07)` |
| Error border | `--color-error-border` | `rgba(220,38,38,.3)` |
| In-progress / info | `--color-info` | `#0891b2` |
| Info background tint | `--color-info-bg` | `rgba(8,145,178,.08)` |
| Info border | `--color-info-border` | `rgba(8,145,178,.22)` |

### 2.3 Border Radius

| Element | Radius |
| --- | --- |
| Page panels, modals, sidebar | `4px` |
| Terminal modal window | `6px` |
| Cards, table wrappers, buttons | `3px` |
| Badges, pills, small chips | `2px` |
| Traffic light dots | `50%` |
| Toggle track | `10px` |

### 2.4 Typography

- **Font stack:** `-apple-system, BlinkMacSystemFont, 'Segoe UI', 'Inter', sans-serif`
- **Monospace stack:** `'SF Mono', 'Roboto Mono', monospace` — versions, digests, terminal output
- **Heading scale:**
  - `h1`: `20px`, `700`, `--text-primary`
  - `h2`: `16px`, `700`, `--text-primary`
  - `h3`: `13px`, `700`, `--text-primary`
- No custom web font loading; system fonts only to keep load instant.

### 2.5 Transitions

Interactive elements use a single short transition for background and border:

```css
transition: background .12s, border-color .12s, color .12s;
```

No transforms, no shadows appearing on hover. State changes are flat and immediate.

Allowed animated properties and exceptions:

- Interactive controls: `background`, `border-color`, `color`
- Loading affordances: `opacity`, `transform`, `background-position`
- Toast progress bar: `transform: scaleX()`
- Terminal shell maximize: `width`, `height`

### 2.6 Focus States

Keyboard-navigable elements use a visible focus ring that does not rely on the default
browser outline:

```css
outline: none;
box-shadow: 0 0 0 3px rgba(var(--accent-rgb), .25);
```

`--accent-rgb` values: dark theme `6 182 212`, light theme `37 99 235` (matching token table).

Focus rings appear only on `:focus-visible` (keyboard navigation), not on click.

When a field is in **error state**, the accent focus ring still applies on focus; it appears
alongside the error border rather than replacing it.

### 2.7 Z-Index Scale

| Layer | Value | Use |
| --- | --- | --- |
| Base content | `0` | Normal page content |
| Sticky top bar | `10` | Top bar in layout shell |
| Sidebar | `20` | Sidebar overlay on tablet |
| Dropdown / tooltip | `100` | Inline popovers |
| Toast stack | `500` | Toast notifications |
| Modal backdrop | `900` | Dialog/terminal backdrop |
| Modal content | `910` | Dialog/terminal window |

### 2.8 Runtime Token Adapter

The semantic tokens in this document are the design contract. The current frontend runtime uses the
shared Skeleton/Tailwind theme stack, so implementations must expose these semantics through a
shared adapter layer consumed by both built-in and surface-backed components.

The family-level mapping table below is orientation only. Conformance requires a checked-in adapter
manifest in the frontend that pins every semantic token in this spec to one exact runtime token,
utility, or preset per theme. Family-level mapping alone is not sufficient for parity CI.

Adapter manifest requirements:

- Canonical path: `frontend/src/theme/adapter-manifest.json` or a checked-in generated equivalent
- Minimum shape: array of `{ token, theme, maps_to }` records
- CI must fail if any token from Sections 2.1–2.2 is missing from the manifest

| Spec semantic token family | Current runtime theme family |
| --- | --- |
| `--bg-base`, `--bg-surface`, `--bg-raised` | `surface-*` background utilities |
| `--border-subtle`, `--border-default` | `surface-*` border utilities |
| `--text-primary`, `--text-secondary`, `--text-muted` | `text-surface-*` utilities |
| `--accent`, `--accent-*`, `--accent-rgb` | `primary-*` theme utilities and preset variants |
| `--color-success-*` | `success-*` theme utilities and preset variants |
| `--color-warning-*` | `warning-*` theme utilities and preset variants |
| `--color-error-*` | `error-*` theme utilities and preset variants |
| `--color-info-*` | `info-*` semantic preset utilities exposed through the shared adapter |

Conformance rule:

- Built-in and surface-backed UI must consume the same adapter layer
- No component may bypass the shared adapter with one-off raw color classes when an equivalent
  semantic token exists
- Any future theme-system change must preserve this semantic mapping rather than rewriting
  built-in and surface-backed UI separately

Current implementation anchor:

- The current shared theme entrypoint is rooted in `frontend/src/app.css` plus shared preset/component wrappers
- Built-in and surface-backed UI must route through that same theme entrypoint until a dedicated adapter module is extracted

> **Note on info tokens:** info and accent may sit on the same hue family in some themes, but they
> remain separate semantic roles. Accent communicates primary interaction; info communicates status.

---

## 3. Layout Shell

**Status:** Target for shared-shell measurements. The sidebar/top-bar/content metrics in this section
define the convergence target for the shell. The `Shared Surfaces` subsection below remains
`Implemented` for parity rules, slot governance, and CI requirements.

Every page shares the same chrome:

```text
┌─────────────┬──────────────────────────────────┐
│  Sidebar    │  Top bar (title + actions)        │
│  180px      ├──────────────────────────────────┤
│             │  Content area (scrollable)        │
│             │                                  │
└─────────────┴──────────────────────────────────┘
```

### Sidebar

- Width: `180px`, background `--bg-surface`, right border `--border-subtle`
- Logo row at top (22×22 gradient mark + app name + version)
- Nav sections with uppercase label headers (`7.5px`, `letter-spacing: .12em`)
- Nav items: `28px` tall, `3px` radius, `10px` font
  - Default: `--text-secondary`
  - Hover: `--bg-raised` background, `--text-primary`
  - Active: `rgba(var(--accent-rgb), .1)` background, `--accent-bright` text, colored nav icon

### Top Bar

- Height: `40px`, bottom border `--border-subtle`
- Page title (bold, `12px`) + optional chip (item count)
- Right side: search input + primary action button

### Content Area

- Padding: `12px 14px`
- Scrollable independently of the shell

### Shared Surfaces

**Status:** Implemented.

Surfaces are not a second visual system. They inherit the same shell, tokens, spacing, typography,
hover states, and interaction patterns as built-in pages.

Visual parity rules:

- Surface-backed UI and built-in UI must use the same component library, tokens, spacing scale,
  typography scale, interaction states, and container patterns
- A user must not be able to tell whether a page, tab, inline settings panel, host-detail panel,
  or single context-menu launcher is surface-backed or built-in by visual treatment alone
- Origin-specific chrome is forbidden: no "plugin style", "service style", alternate card shells,
  alternate tab treatments, or alternate button systems
- If built-in UI uses a standard component for a pattern, surfaces use that same component rather
  than a visual approximation
- New primitives added for the Surfaces runtime must be promoted into the shared design system and
  become available to built-in UI as well; they must not remain surface-only visual widgets
- User-visible implementation leakage is forbidden: no raw interaction IDs, data-source IDs,
  fallback contract identifiers, or surface-only placeholder/error phrasing

- `surface.page` entries are first-class navigation items in the main sidebar
- Priority sorts ascending; lower values rank first
- `surface.page` remains a single-entry slot per provider registration; multiple providers may each
  contribute one top-level page, but one provider must not register multiple `surface.page` entries
  unless the shared slot contract changes
- The rendered UI exposes at most one visible top-level nav/page entry per `surface_id`; parity
  fixtures must use one canonical visible entry per `surface_id`, and descriptor-collision handling
  remains a registry concern outside this visual spec
- Mixed built-in and `surface.page` nav sorts by `priority`, then `label`, then origin
  (`built-in` before `surface.page`), then stable ID (`href` for built-in items,
  `surface_id` for surface items)
- Surface-backed full pages use the canonical `/surfaces/<surface_id>` route and the same page title
  and content framing as built-in pages
- Slot-backed surfaces injected into built-in routes must inherit the host route's container pattern
  rather than inventing their own chrome

Current slot coverage after the Surfaces migration:

| Slot ID | Host container | Visual rule |
| --- | --- | --- |
| `surface.page` | Top-level nav page | Behaves like a built-in top-level page inside the standard shell |
| `settings.tabs` | Settings tab strip | Uses the route-owned tab strip and tab-body container |
| `settings.below.global` | Global settings body | Renders as an inline card/section stack below the built-in global settings content |
| `software.tabs` | Software tab strip | Uses the route-owned tab strip and tab-body container |
| `host_detail.tabs` | Host detail body | Renders as an inline card stack inside the host detail route |
| `software_item.host_context_menu` | Software-item host context menu | Standard menu launcher rows; each launcher opens the standard modal shell |

Built-in route table-column **slot injection** is not part of the current Surfaces runtime contract
after migration. This does **not** remove support for `Table.columns` inside the `Table` surface
primitive itself. If host-table column injection is added later, it must follow the same visual parity
rules and be added to the canonical slot registry before it is treated as supported.
Any future slot added to the registry inherits the same parity gates and enforcement rules by default.

Registration cardinality:

- `settings.tabs`, `software.tabs`, `host_detail.tabs`, and `settings.below.global` are multi-entry
  slots in the current registry contract
- `surface.page` and `software_item.host_context_menu` are single-entry per provider registration,
  but runtime aggregation may still yield multiple visible entries across providers

Aggregated rendering order:

- When aggregation yields multiple visible entries for a slot, surface-backed entries render in
  `priority`, then `label`, then `surface_id` order
- This applies to `settings.tabs`, `software.tabs`, `host_detail.tabs`, `settings.below.global`,
  and aggregated `software_item.host_context_menu` launcher rows

Slot-specific parity gates:

- `surface.page`: same shell, page heading pattern, content container, and nav item treatment as
  currently deployed built-in pages during shell convergence; it promotes to the Section 3 target
  shell metrics when this section is promoted from `Target`
- `settings.tabs`, `software.tabs`: same tab-strip component, active state, overflow behavior, and body container as built-in tabs
- `host_detail.tabs`: same inline card shell, heading rhythm, spacing, and empty/error treatment as
  the route-owned host detail slot container
- `settings.below.global`: same card, section spacing, and heading rhythm as built-in inline settings panels
- `software_item.host_context_menu`: same context-menu row styling as built-in actions and the same
  modal shell once invoked; multiple launcher rows, when present, follow the deterministic ordering rule

Persistent surface page state uses query parameters on `/surfaces/<surface_id>`.
Surface pages must not create origin-specific sub-routes or fragment-only navigation for durable UI state.

Targeted surfaces show a provider selector above the rendered content.
That selector uses the standard field colors, borders, and focus styling from Section 4.10, but
uses a compact stacked-label layout rather than the `110px` fixed-label row. It sits inside the
content column rather than a separate toolbar and has a compact max width of `280px`.

Provider-selector ownership:

- The host route or shared slot-host wrapper owns the targeted-provider selector and the no-provider
  empty state
- Surface primitives render provider-specific content only; they do not invent a second selector inside
  the surface body

For targeted `surface.page` routes, the provider selector appears directly below the page heading
and above the primary content stack. If no compatible provider is connected, the page body uses the
global empty-state pattern from Section 4.7 with the title `No provider connected` and the
description `Connect a compatible service to use this surface.`

Parity enforcement:

- Shared components are mandatory for tabs, tables, forms, callouts, empty states, modals, menus, and action buttons
- Visual regression coverage must compare built-in and surface-backed instances of the same host pattern
- DOM/component checks must ensure no surface-only fallback labels or contract IDs leak into user-facing UI

Minimum CI parity gates:

- Required paired snapshots:
  - built-in settings tab vs `settings.tabs` surface tab
  - built-in software tab vs `software.tabs` surface tab
  - built-in inline settings card vs `settings.below.global` surface panel
  - route-owned host detail slot container vs `host_detail.tabs` surface card shell
  - standard form field row vs targeted-surface provider selector
  - built-in context-menu item vs `software_item.host_context_menu` launcher
  - built-in action modal vs `software_item.host_context_menu` opened modal
  - built-in top-level nav item vs `surface.page` nav item
  - built-in page shell/body vs `surface.page` page shell/body in loaded,
    empty/no-provider, and contract-mismatch states
- Required shared-primitive parity coverage:
  - `Table`: header row, body row, empty row, and row-action treatment
  - `Callout`: info, warning, and danger variants (`danger` uses error visual tokens)
  - `ModalTrigger`: trigger row/button and opened modal shell/body
  - `WorkflowTrigger`: trigger row/button, opened modal shell, and step-indicator states
- Required fixture-backed slot/state matrix:
  - `surface.page`: loaded, permission-denied, targeted no-compatible-provider,
    contract-mismatch, and hydration/action-failure
  - `settings.tabs` and `software.tabs`: loaded, permission-denied, targeted
    no-compatible-provider, contract-mismatch, and structural no-surface-content
    host-chrome check
  - `settings.below.global` and `host_detail.tabs`: loaded, permission-denied,
    targeted no-compatible-provider, contract-mismatch, hydration/action-failure,
    and omitted-state
  - `software_item.host_context_menu`: launcher row, opened modal, permission-denied/contract-mismatch fallback, and omitted-state
- Fixture rule:
  - each mandatory slot/state pair must have a checked-in deterministic fixture or story that names the trigger inputs for that state
- Required matrix:
  - light + dark themes for every required pair
  - desktop for every required pair now
  - mobile coverage promotes with Section 7; until then, parity CI treats slots as desktop-only
- Fail conditions:
  - any leaked contract ID, missing-label fallback, or raw renderer error text
  - component mismatch without a checked-in design waiver in `docs/superpowers/ui-parity-waivers.json`
  - visual diff above `0.5%` after masking only approved dynamic regions

Approved dynamic masking:

- Allowed categories: relative timestamps, version strings, SHA digests, animated spinners, and live log text
- Allowed mechanism: checked-in selector list or explicit `data-visual-dynamic` markers
- Maximum masked area per snapshot: `15%`
- If a snapshot legitimately exceeds `15%`, a waiver entry must narrow the capture region and document why the wider view is too dynamic for parity comparison
- CI must use Playwright screenshot comparison with mismatch ratio computed as mismatched pixels divided by total snapshot pixels
- CI capture profile must be deterministic: pinned browser channel, fixed DPR, fixed viewport presets,
  locked font package, reduced-motion mode, and fixed locale/timezone
- Waiver file entries must include scope, owner, expiry date, capture region, justification, and linked review/PR reference

---

## 4. Components

### 4.1 Badges

Badges are `14px` tall, `2px` radius, `7.5px` bold uppercase text with `letter-spacing: .04em`.
They always have both a background tint and a 1px border.

#### Dark theme badge values

| Variant | Background | Text | Border |
| --- | --- | --- | --- |
| Green (up to date / success) | `rgba(74,222,128,.10)` | `#4ade80` | `rgba(74,222,128,.20)` |
| Teal (update / in-progress) | `rgba(6,182,212,.10)` | `#67e8f9` | `rgba(6,182,212,.22)` |
| Violet (input required / interactive attention) | `rgba(168,85,247,.12)` | `#c4b5fd` | `rgba(168,85,247,.28)` |
| Orange (error / failed) | `rgba(234,88,12,.15)` | `#fdba74` | `rgba(234,88,12,.35)` |
| Amber (warning) | `rgba(251,191,36,.12)` | `#fcd34d` | `rgba(251,191,36,.30)` |
| Dim (unknown / offline) | `rgba(148,163,184,.08)` | `#71717a` | `--border-default` |

#### Light theme badge values

Light theme badges follow the same structure with adjusted tint strengths for a white surface:

| Variant | Background | Text | Border |
| --- | --- | --- | --- |
| Green (up to date / success) | `rgba(22,163,74,.08)` | `#16a34a` | `rgba(22,163,74,.25)` |
| Info (update / in-progress) | `rgba(8,145,178,.08)` | `#0891b2` | `rgba(8,145,178,.22)` |
| Violet (input required / interactive attention) | `rgba(124,58,237,.10)` | `#7c3aed` | `rgba(124,58,237,.25)` |
| Red (error / failed) | `rgba(220,38,38,.08)` | `#dc2626` | `rgba(220,38,38,.28)` |
| Amber (warning) | `rgba(217,119,6,.10)` | `#d97706` | `rgba(217,119,6,.28)` |
| Dim (unknown / offline) | `rgba(148,163,184,.08)` | `#94a3b8` | `--border-default` |

#### Clickable badges (interactive variant)

Used where a badge doubles as an action trigger. The text swaps on hover; the column it lives in
has a **fixed width** so the swap never causes layout reflow.

Pattern: two sibling spans `.idle` / `.hov` inside the badge element.
CSS hides `.hov` by default and swaps on `:hover`.

**Hover opacity values — dark theme** (exact values):

| Variant | Idle bg | Hover bg | Idle border | Hover border |
| --- | --- | --- | --- | --- |
| Green | `.10` | `.20` | `.20` | `.40` |
| Teal | `.10` | `.20` | `.22` | `.44` |
| Orange | `.15` | `.28` | `.35` | `.60` |
| Amber | `.12` | `.22` | `.30` | `.55` |

**Hover opacity values — light theme** (exact values):

| Variant | Idle bg | Hover bg | Idle border | Hover border |
| --- | --- | --- | --- | --- |
| Green | `.08` | `.16` | `.25` | `.45` |
| Info | `.08` | `.16` | `.22` | `.42` |
| Red | `.08` | `.16` | `.28` | `.50` |
| Amber | `.10` | `.20` | `.28` | `.50` |

> Update / in-progress badges use the info semantic family in both themes. Accent remains reserved
> for primary interaction emphasis rather than status signaling.

Examples in use:

| Page | Idle text | Hover text | Action |
| --- | --- | --- | --- |
| Software — host row | `Update Avail` | `↑ Update` | Trigger update for this host |
| Hosts — software column | `N updates` | `→ Software` | Navigate to Software filtered for this host |
| Hosts — software column | `X error` | `→ History` | Navigate to History filtered for this host |

The hover text must never be wider than the idle text. Both texts are measured at design time;
the badge column width is fixed to the wider of the two (in practice the idle text is always chosen
to be at least as wide). `min-width: max-content` and `justify-content: center` on the badge
prevents any reflow.

Violet badges are static-only. They do not use hover text swap and must not be treated as clickable badges.
Dim badges are also static-only. They do not participate in the hover-swap pattern.

### 4.2 Pills

Pills are `12px` tall, `2px` radius, `7px` bold uppercase, no border.
Used for categorical labels (agent type, OS, plugin type).

#### Dark theme pill values

| Variant | Background | Text | Use |
| --- | --- | --- | --- |
| Purple | `rgba(139,92,246,.12)` | `#a78bfa` | SSH agent |
| Teal | `rgba(6,182,212,.12)` | `#67e8f9` | Local agent, Docker plugin |
| Green | `rgba(74,222,128,.12)` | `#4ade80` | Linux OS |
| Grey | `rgba(148,163,184,.10)` | `#a1a1aa` | macOS, GitHub plugin |
| Yellow | `rgba(251,191,36,.12)` | `#fcd34d` | Homebrew plugin |

#### Light theme pill values

| Variant | Background | Text | Use |
| --- | --- | --- | --- |
| Purple | `rgba(124,58,237,.09)` | `#7c3aed` | SSH agent |
| Teal | `rgba(8,145,178,.09)` | `#0891b2` | Local agent, Docker plugin |
| Green | `rgba(22,163,74,.09)` | `#16a34a` | Linux OS |
| Grey | `rgba(100,116,139,.09)` | `#64748b` | macOS, GitHub plugin |
| Yellow | `rgba(180,83,9,.09)` | `#b45309` | Homebrew plugin |

### 4.3 Buttons

Standard button height: `23px`, `3px` radius, `9px` bold text.

| Variant | Idle style | Hover style | Active style |
| --- | --- | --- | --- |
| Primary (dark) | `linear-gradient(90deg, #0e7490, #06b6d4)`, white text | `linear-gradient(90deg, #0891b2, #22d3ee)` | `opacity: .88` |
| Primary (light) | `linear-gradient(90deg, #1d4ed8, #2563eb)`, white text | `linear-gradient(90deg, #2563eb, #3b82f6)` | `opacity: .88` |
| Ghost | Transparent, `--border-default` border, `--text-primary` text | `--bg-raised` background, border stays `--border-default` | `opacity: .88` |
| Danger (dark) | `rgba(234,88,12,.15)` bg, `rgba(234,88,12,.35)` border, `--color-error` text | bg `rgba(234,88,12,.22)`, border `rgba(234,88,12,.50)` | `opacity: .88` |
| Danger (light) | `rgba(220,38,38,.07)` bg, `rgba(220,38,38,.3)` border, `--color-error` text | bg `rgba(220,38,38,.14)`, border `rgba(220,38,38,.45)` | `opacity: .88` |

All variants inherit the standard `transition: background .12s, border-color .12s, color .12s` from Section 2.5.

**Disabled state:** All variants use `opacity: 0.4` when `disabled`. `pointer-events: none`.
No border or background change — the opacity communicates the state clearly without a
separate disabled color set.

#### `↑ Update all` button

Appears on software header rows. Uses the same interaction pattern as clickable badges rather than
the standard button style — it reads as a badge-level control, not a page-level action.

Exact values per theme:

| State | Background | Border | Text |
| --- | --- | --- | --- |
| Dark idle | `rgba(6,182,212,.06)` | `rgba(6,182,212,.20)` | `--accent` (`#06b6d4`) |
| Dark hover | `rgba(6,182,212,.18)` | `rgba(6,182,212,.45)` | `--accent-bright` (`#22d3ee`) |
| Light idle | `rgba(37,99,235,.06)` | `rgba(37,99,235,.20)` | `--accent` (`#2563eb`) |
| Light hover | `rgba(37,99,235,.18)` | `rgba(37,99,235,.45)` | `--accent-bright` (`#3b82f6`) |
| Dim (nothing to update) | transparent | `--border-default` | `--text-muted` — `pointer-events: none` |

### 4.4 Toggles

`28×15px` track, `10px` radius. Thumb: `11×11px` circle, `50%` radius, `#ffffff` fill
(same in both on and off states; the track color conveys state).
Off: `--border-default` track background, thumb at `left: 2px`.
On: `rgba(var(--accent-rgb), .5)` track background with `1px solid var(--accent)`, thumb at `left: 15px`
(= `track-width 28 - thumb-width 11 - right-offset 2`).

Disabled: `opacity: 0.4; pointer-events: none` — same approach as buttons.

### 4.5 Stat Cards

Used at the top of list pages (Hosts). `3px` radius, `--bg-surface` background,
`--border-subtle` border. Label in `7.5px` (`text-transform: uppercase` in CSS, stored as normal case),
value in `14px` bold.

Value color mapping:

| State | Text color token |
| --- | --- |
| Healthy / online | `--color-success` |
| Needs attention / updates pending | `--color-info` |
| Error | `--color-error` |
| Offline / unknown | `--text-muted` |

On the Hosts page, the four stat cards map as:
Online → success (green), Offline → muted (dim), Updates pending → info (teal/blue),
Errors → error (orange/red).

### 4.6 Loading States

Three patterns are used depending on context:

**Skeleton placeholders** — used when the page or a list section is loading its initial data.
Skeleton elements mimic the shape of the content they replace (rows, badges, text lines).

- Background: `--bg-raised`
- Animation: `opacity` pulses between `0.35` and `0.70` over `1.4s ease-in-out infinite`
  (slower than the spinner to convey passive waiting rather than active in-flight work)
- Radius matches the element being replaced (e.g. `2px` for badges, `3px` for rows)

**Spinner** — used for in-flight actions (button loading state, individual item refresh).

- `16px` circle, `2px` border, base color `--border-default`, sweep color `--accent`
- Rotates `360°` over `0.7s linear infinite`
- Sizes: `sm` 12px / default 16px / `lg` 24px

**Indeterminate loading bar** — used at the top of the content area during page-level
navigation or background polling.

- `2px` height, full page width, background `--border-subtle`
- Animated sweep: gradient `transparent → --accent → transparent` moving left to right
- Animation duration `1.4s ease-in-out infinite`

Deterministic usage rules:

- Use skeletons when the loading region has a known content shape
- Use a spinner only for user-triggered in-flight actions or single-item refresh states
- Use centered muted `Loading...` copy only when the layout is intentionally unconstrained or highly variable

### 4.7 Empty States

Used when a list page has no items to display, or when a filtered view returns no results.

Structure: centred block within the content area.

- Icon: `32×32` neutral icon in `--text-muted`
- Title: `13px` bold, `--text-primary`
- Description: `11px`, `--text-secondary`, max `320px` width, centered
- Optional action button: ghost variant, displayed below the description

Two variants:

- **Global empty** (no data at all): icon + title + description + action (e.g. "No hosts enrolled yet. Add a host to get started. [Enroll Host]")
- **Filtered empty** (filter/search returned nothing): icon + "No results" title + "Try adjusting your search or filter." description, no action button

### 4.8 Toasts

Toast notifications appear in the **top-right** corner, stacking downward. New toasts appear
at the top of the stack.

**Dimensions and positioning:**

- Width: `300px` fixed
- Offset from viewport edges: `16px` top, `16px` right
- Gap between stacked toasts: `6px`

**Dismissal:**

- Click anywhere on the toast body (entire card is clickable; `cursor: pointer`)
- Swipe right on **tablet** touch devices (threshold `80px`)
- Auto-dismiss after timeout: 4s for success/info, 8s for error/warning
- Explicit close button (`✕`) visible on the right edge

All error and warning toasts use the 8s timeout regardless of message urgency.
Hovering over a toast pauses the auto-dismiss timer and progress bar; the timer resumes on mouse leave.

A **progress bar** depletes along the bottom of the toast over the auto-dismiss duration,
giving a visual countdown. Height `2px`, full toast width, color uses the variant's main color
token (e.g. `--color-success` for success toasts). The bar shrinks from right to left over the
auto-dismiss duration using `transform: scaleX()` with `transform-origin: left`.

**Structure:** icon square + body (title + description) + close button.
Icon square: `20×20px`, `2px` radius, icon centered at `9px`. Colors per variant:

| Variant | Dark bg | Dark icon | Light bg | Light icon |
| --- | --- | --- | --- | --- |
| Success | `rgba(74,222,128,.12)` | `#4ade80` | `--color-success-bg` | `--color-success` |
| Error | `rgba(234,88,12,.15)` | `#fdba74` | `--color-error-bg` | `--color-error` |
| Info | `rgba(6,182,212,.12)` | `#67e8f9` | `--color-info-bg` | `--color-info` |
| Warning | `rgba(251,191,36,.12)` | `#fcd34d` | `--color-warning-bg` | `--color-warning` |

Toast body has a subtle background shift on hover (`--bg-raised`).

**Variants:**

| Variant | Use | Color token |
| --- | --- | --- |
| Success | Update triggered, operation completed | `--color-success` |
| Error | Update failed, connection lost | `--color-error` |
| Info | Updates available, background event | `--color-info` |
| Warning | Host offline, configuration issue | `--color-warning` |

Target mobile behavior: on mobile (< 640px), toasts appear at **bottom-center** instead of
top-right. This promotes together with Section 7. See Section 7 for the mobile swipe direction
change.

### 4.9 Confirmation Dialogs

Used for destructive or irreversible actions (delete host, remove plugin config, revoke token).

- Centred modal over a `rgba(0,0,0,.55)` backdrop (lighter than the terminal modal)
- Width: `380px` fixed, `4px` radius
- Title: `13px` bold
- Body: `11px` `--text-secondary`, describes what will happen
- Actions row: right-aligned, cancel (ghost) + confirm (danger); both use standard button
  dimensions from Section 4.3 (`23px` height, `9px` bold text, `3px` radius)
- Close on backdrop click or `Escape`

The confirm button uses the danger variant and is labeled with the specific action
(e.g. "Delete host", "Revoke token") rather than a generic "Confirm".

### 4.10 Form Validation

Validation is inline — errors appear immediately below their field, not in a summary block.

**Default field state:**

- Height: `32px` for text inputs/selects, `72px` minimum for textareas
- Background: `--bg-surface`
- Border: `1px solid var(--border-default)`
- Padding: `0 10px`
- Text color: `--text-primary`
- Placeholder color: `--text-muted`

**Error state:**

- Input border: `--color-error-border`
- Input background: `--color-error-bg`
- Error message: `10px`, `--color-error`, appears below the input with a small `✕` icon prefix
- No red outline ring — border color change alone is sufficient

**Success state (optional):**

- Used only for fields with meaningful validation (e.g. hostname format check)
- Input border: `--color-success-border` (token defined in Section 2)
- Small `✓` icon at input right edge

**Focus state:**

- `box-shadow: 0 0 0 3px rgba(var(--accent-rgb), .25)` (matches Section 2.6)
- Border color: `--accent` only when the field is not already in error state
- Applies on `:focus-visible` only
- On error-state fields: keep the error border and add the accent focus ring; the focus state must
  not replace the error border

**Label layout:** `110px` fixed label width, input takes remaining space.
Labels are `10px` bold, `--text-secondary`.

### 4.11 Tab Strip

Used by built-in route tabs and slot-backed tab surfaces, except `host_detail.tabs`, which is
currently rendered as an inline card stack rather than a tab strip.

- Tab button height: `28px`
- Horizontal padding: `10px`
- Radius: `3px`
- Text: `10px`, `600`
- Inactive state: transparent background, `--text-secondary`
- Hover state: `--bg-raised` background, `--text-primary`
- Active state: `rgba(var(--accent-rgb), .12)` background, `--accent-bright` text
- Gap between tab buttons: `4px`
- Overflow behavior: horizontally scrollable row on narrow widths; no alternate surface-only overflow styling

### 4.12 Data Tables

This is the canonical table treatment for built-in list pages and the `Table` surface primitive.

- Header row height: `28px`
- Body row minimum height: `32px`
- Cell horizontal padding: `10px`
- Header background: `--bg-raised`
- Header text: `9px`, `700`, uppercase, `letter-spacing: .04em`, `--text-muted`
- Body text: `10px`, `--text-primary`
- Row hover: `--bg-raised`
- Empty/loading rows: centered muted copy or skeletons using the rules in Section 4.6
- Mobile fallback: card-stack layout using the same label/value spacing as built-in mobile list cards

### 4.13 Context Menus

This is the canonical menu shell for built-in actions and for `software_item.host_context_menu`.

- Background: `--bg-surface`
- Border: `1px solid var(--border-default)`
- Radius: `4px`
- Menu item row height: `32px`
- Horizontal padding: `12px`
- Item text: `10px`, `--text-primary`
- Hover fill: `--bg-raised`
- Group label: uppercase `7.5px`, `letter-spacing: .08em`, `--text-muted`
- Destructive items use the error text token but keep the same row shell

Current slot limitation:

- `software_item.host_context_menu` currently contributes launcher entries, not nested action groups
- Until grouped context-menu actions are added to the slot contract, each launcher entry must use the
  same menu-item shell as built-in actions and open the standard modal shell used by `ModalTrigger`

### 4.14 Workflow / Wizard Shell

`WorkflowTrigger` uses the standard modal shell plus an explicit shared step indicator.

- Step indicator row appears below the modal title
- Step chip height: `18px`, `2px` radius, `8px` semibold text
- Completed step: success tint + success text
- Active step: accent tint + accent text
- Upcoming step: `--bg-raised` fill + `--text-secondary`
- Step content body uses the same form and action-row rules as built-in wizards

### 4.15 Shared Surface Primitives

The Surfaces renderer exposes a fixed set of primitives. Each maps onto an existing built-in pattern;
surface-backed implementations MUST NOT introduce bespoke visual language.

These primitives are renderer-level contract shapes, not permission to invent separate UI widgets.
Their implementation must come from the same shared component set used by built-in pages wherever
that component exists already.

| Surface primitive | Design treatment |
| --- | --- |
| `Section` | Vertical stack with `16px` gap; optional section title uses the `h3` style from Section 2.4 |
| `TextBlock` | Standard body copy; `11px`–`12px`, `--text-secondary`, `white-space: pre-wrap` when needed |
| `KeyValue` | Same label/value rhythm as settings and detail views; labels muted, values primary; `10px` labels, `11px` values |
| `Table` | Reuses the data-table component from Section 4.12 |
| `Form` | Uses the standard field layout, validation, focus ring, and action-row spacing from Section 4.10 |
| `ActionBar` | Right-aligned row of buttons with `8px` gap; wraps on narrow widths |
| `Tabs` | Uses the tab-strip component from Section 4.11 |
| `Callout` | Uses semantic info/warning/danger variants; danger uses error visual tokens and no custom illustrations or banners |
| `EmptyState` | Uses the empty-state pattern from Section 4.7 |
| `ModalTrigger` | Opens a standard modal shell, not a custom drawer or full-screen takeover |
| `WorkflowTrigger` | Opens the workflow shell from Section 4.14 |

If a built-in analogue does not exist yet for a needed primitive, the component MUST be designed as
a shared design-system component first and then consumed by both built-in and surface-backed UI.
No surface-only primitive may ship without shared design-system component availability.

Surface-provided context-menu launchers inherit the host menu shell:

- Menu items: same row height, padding, hover fill, and destructive-color treatment as built-in items
- Icons, if present, are single-color and align to the same leading column as built-in actions

Surface-provided content must never expose raw contract internals:

- Interactive controls must have human-authored labels
- Empty/error states must use shared design-system copy patterns
- Contract validation failures must surface through shared warning/error callouts, never raw IDs or missing-symbol text

### 4.16 Shared Surface Runtime States

Surface runtime states use the existing empty/callout/loading language instead of ad hoc placeholders.

A structural slot is one whose host UI container still renders without any surface entries because
the host route owns that structure. In the current contract, `settings.tabs` and `software.tabs`
are structural; `settings.below.global`, `host_detail.tabs`, and `software_item.host_context_menu`
are omitted when they have no surface content.

Canonical state IDs for fixtures and CI:

- `loading`: registry or read payload loading
- `permission_denied`: registered surface cannot render because the user lacks permission
- `no_compatible_provider`: targeted surface has no compatible connected provider
- `contract_mismatch`: unsupported, invalid, or mismatched surface contract payload
- `hydration_action_failure`: read hydration or action execution failed after the slot was otherwise renderable
- `no_surface_content`: no registered surface content exists for the slot

| State | Treatment |
| --- | --- |
| `loading` | Skeletons when shape is known; centered muted `Loading...` only when shape is intentionally unconstrained |
| `permission_denied` | Empty state / callout with muted explanation; no broken-shell fallback |
| `no_compatible_provider` | Neutral empty state in the content body, not a toast. Use title `No provider connected` and description `Connect a compatible service to use this surface.` Optional secondary action may be shown only when the host route already has an explicit service-connection destination outside the surface payload contract |
| `contract_mismatch` | Warning callout using warning tokens |
| `hydration_action_failure` | Inline error callout using error tokens; keep surrounding layout intact |
| `no_surface_content` | Keep structural tab strips with built-in tabs only and no synthetic placeholder tab. Omit `settings.below.global`, `host_detail.tabs`, and `software_item.host_context_menu` entirely when absent |

A slot is `no_surface_content` only when no registered surface content exists for it. A
`permission_denied` response from a registered surface is not absence; it renders the permission
callout inside the host container.

---

## 5. Page Patterns

### 5.1 Software Page

The central view. Software items are the top-level grouping; hosts are sub-rows.

#### Tabs

The Software route owns the tab state. Built-in tabs and surface-backed `software.tabs` entries share
one tab strip and one content body.

- Surface tabs use the same button treatment as built-in tabs
- Active tab state persists in the route URL via `?tab=<tab-id>`
- While a surface tab is still loading, the tab remains visible and the body shows a loading state
- Surface tabs do not get their own nested page chrome inside the Software route

#### Structure

```text
[▾] nginx          3 hosts · 2 updates · 1 error      [↑ Update all]
      prod-01  Docker   sha256:a3f1…  ↓ sha256:9de2…  [Update Avail]
      prod-02  APT      1.24.0        ↓ 1.25.0         [Update Avail]
      monitoring Docker sha256:b2e9…                   [Error]
[▾] postgresql     2 hosts · up to date                [↑ Update all] dim
      ...
[▾] node           4 hosts · 1 update                  [↑ Update all]
      dev-mac  Homebrew 22.11.0       ↓ 22.14.0        [Update Avail]
      staging  APT      22.14.0                        [Up to date]
      ▸ 2 more — all up to date
```

#### Column grid

Both header rows and host sub-rows use the same 4-column grid: `16px 1fr 120px 88px`

- Col 1: caret / spacer
- Col 2: name + summary / host name + plugin pill
- Col 3: **always empty on header rows** (summary text lives in col 2 `1fr`) / version on host rows — fixed `120px`, right-aligned
- Col 4: `↑ Update all` / status badge — fixed `88px`, right-aligned

**Row backgrounds:** software header rows use `--bg-raised`; host sub-rows use transparent
(the list container's `--bg-surface` background shows through). Sub-rows hover to `--bg-raised`.

Fixed column widths are non-negotiable — they prevent layout reflow when badge text changes
on hover.

#### Version column

Single column showing stacked values:

- Line 1: current installed version — `10px`, `--text-secondary`, monospace
- Line 2: `↓ new-version` — `9px`, `--accent-bright`, monospace (only when update is available)

Docker image digests are truncated to `sha256:` + first 12 hex characters + `…`
(e.g. `sha256:a3f19cb2e3…`). Truncation is applied at render time; full digest is shown
in the terminal modal and on hover via a native `title` attribute.

#### Truncation

When a software item has 4 or more hosts, only the first 3 are shown. A `▸ N more` row follows,
left-aligned with the host name column (`padding-left: 49px`). The row is clickable to expand.

The summary text shows an aggregate: `▸ 2 more — all up to date` or `▸ 3 more — 1 with updates`.

### 5.2 Hosts Page

Standard table. Columns: online dot, host name + IP, agent type pill, OS pill, software status
badge, last-seen time.

The software status badge uses the **navigable badge** pattern:

- `N updates` (teal) → hover shows `→ Software` → click navigates to Software for this host
- `X error` (orange) → hover shows `→ History` → click navigates to History for this host
- `Up to date` (green): static, not clickable
- `Unknown` (dim): static, not clickable

Stat cards above the table (see Section 4.5 for color mapping):

| Card | Value color |
| --- | --- |
| Online | `--color-success` |
| Offline | `--text-muted` |
| Updates pending | `--color-info` |
| Errors | `--color-error` |

### 5.3 History Page

Chronological feed of update events, grouped by date with separator labels.

**Status:** Transitional. The current runtime still renders some output inline on the History page.
The terminal styling rules in Section 6 are the convergence target. Inline history output must reuse
the same terminal body colors, typography, badges, and metadata rhythm until the shell is unified.
Exit criteria are defined in Section 6.

Each item:

- Left: `24×24px` colored icon square, `3px` radius (✓ success, ✕ failed, ↑ in-progress, · pending)
- Body: `software on host`, version change (`old → new` in monospace, new in teal), plugin type
- Right: status badge + relative timestamp
- Interactive entries may additionally show an `Input Required` violet badge when operator input is pending

Icon square colors — dark theme:

| State | Background | Icon color |
| --- | --- | --- |
| Success | `rgba(74,222,128,.12)` | `#4ade80` |
| Failed | `rgba(234,88,12,.15)` | `#fdba74` |
| In-progress | `rgba(6,182,212,.12)` | `#67e8f9` |
| Pending | `rgba(148,163,184,.08)` | `#71717a` |

Icon square colors — light theme (semantic tokens):

| State | Background | Icon color |
| --- | --- | --- |
| Success | `--color-success-bg` | `--color-success` |
| Failed | `--color-error-bg` | `--color-error` |
| In-progress | `--color-info-bg` | `--color-info` |
| Pending | `--bg-raised` | `--text-muted` |

For **in-progress** items: a `▶ view log` hint appears in the meta line.
Clicking the item opens the update-output view for that entry. When rendered inline during the
transition period, it must still use the same terminal inner styling as the modal target.

### 5.4 Settings Page

The Settings route owns the tab state. Built-in settings sections and any surface-backed
`settings.tabs` entries share the same top-level tab strip.

- Tab buttons use the same active/hover pattern across built-in and surface tabs
- Active tab state persists in the route URL via `?tab=<tab-id>`
- A selected tab may render a form-heavy two-column body (`120px` narrow nav + form body) where the
  content calls for it, but the tab shell itself is shared
- Form-heavy settings content uses the standard `110px` label width from Section 4.10
- Surface tabs render in the same body container and must not add nested page headers or duplicate tab bars

Destructive actions (delete account, revoke all tokens) are grouped in a "Danger Zone" section
with a danger-variant button and confirmation dialog (see Section 4.9).

Inline surfaces mounted through `settings.below.global` appear below the built-in global settings
sections and use the same panel spacing and heading rhythm as built-in inline settings cards.

### 5.5 Slot-Backed Detail Panels

Built-in detail routes can host registered slot-backed panels.

- `host_detail.tabs` currently renders as an inline card stack inside the host detail route body
- `settings.below.global` uses the route-owned inline panel container below built-in global settings content
- The surface label is used as the tab or panel title; providers do not add a second title inside
  the rendered body unless content structure genuinely needs a subsection heading
- Targeted surfaces keep the provider selector inside the tab/panel body, above the rendered nodes,
  with the `280px` max width from Section 3
- When no provider is available, the tab/panel body shows the neutral empty-state treatment from
  Section 4.16 rather than collapsing unpredictably
- Where parity CI checks host chrome without surface content, routes should expose stable capture
  regions such as `data-parity-region` markers on the host container being compared

---

## 6. Terminal Output Shell (Xterm.js)

**Status:** Transitional. The current runtime still uses both inline history expansion and modal
presentation in different routes. The target shell below is the canonical modal treatment, and the
inline variant must mirror its inner visual language until convergence is complete.

Exit criteria:

- History route and software-item detail route use the same terminal-shell component
- Legacy inline-only styling is removed
- Screenshot parity exists for the final terminal shell in both themes at `<= 0.5%` visual diff
  using the masking rules from Section 3
- Terminal parity snapshots compare named chrome regions (frame, titlebar, status bar); the live
  terminal body is excluded from visual diff capture unless a waiver narrows the capture region

Used for live and historical update output. Opens as a centred modal over a
`rgba(0,0,0,.78)` backdrop.

### Opening / Closing

- Opened by clicking an in-progress or completed history item when output is rendered as a modal
- Closed by: clicking the red traffic light, pressing `Escape`, or clicking the backdrop
- Modal state is managed via JS `classList.toggle('open')` on the modal element, not CSS `:target`
- Closing always resets to non-maximized size

### Window Chrome

The terminal window uses macOS-style traffic light controls in the title bar.

Traffic light states:

| Button | Color | Always? | Function |
| --- | --- | --- | --- |
| Red (close) | `#ff5f57` | Always colored | Closes the modal |
| Yellow (minimize) | `#3f3f46` grey | Always grey | Disabled / non-interactive — minimize is meaningless for a modal |
| Green (maximize) | `#27c840` | Always colored | Toggles maximized state |

Interaction:

- Default: icons are invisible (`color: transparent`)
- Hover **any** of the three dots: icons appear on all three simultaneously using
  `.xterm-dots:hover .xterm-dot` CSS selector
  - Red: `✕`, Yellow: `_`, Green: `+` (normal) / `⊡` (when maximized)
- Icons render in a dark semi-transparent color appropriate to each button's background

### Maximized State

Default border radius: `6px`. Clicking the green dot expands the window to `92vw × 88vh`
with a `0.18s ease` transition on `width` and `height`. The terminal body grows to fill
available height (`flex: 1`). Border radius reduces to `4px` when maximized.
Clicking the green dot again restores to the default size. Closing the modal always resets
to normal size.

### Layout

```text
┌─ titlebar (36px) ─────────────────────────────┐
│  🔴 🟡 🟢          title (monospace, centered) │
├───────────────────────────────────────────────┤
│  terminal body (scrollable, flex:1 when max)  │
│  bg #0c0c0e · 9px monospace · 1.6 line-height │
├───────────────────────────────────────────────┤
│  status bar (28px): badge + metadata          │
└───────────────────────────────────────────────┘
```

**Default (non-maximized) size:** `580px` wide × `380px` tall.
Terminal body height in default mode: `316px` (`380 - 36px titlebar - 28px status bar`).
Maximized: `92vw × 88vh`; terminal body fills remaining height via `flex: 1`.

### Responsive behavior

- Tablet: cap default size at `92vw × 70vh`
- Mobile: modal becomes full-screen with `100vw × 100vh`, no maximize affordance, and `0px` radius
- Inline history expansion must reuse the same terminal body colors, status badges, and metadata treatment even when it does not use the modal shell

**Title text:** `<software-name> on <hostname>` in the monospace font stack, centered.

The terminal uses the Xterm.js `fit` addon to auto-fit columns to the container width.
No fixed column count is specified — the terminal fills the available width.
Terminal body uses `white-space: pre` to preserve output formatting.

**Status bar layout:** status badge left-aligned, metadata right-aligned, single line, vertically
centered. Metadata format: `<hostname> · started <relative-time> · <duration>`.
The status bar shows the update status badge (same variants as history items).

Colour conventions in terminal output:

| Colour | Use |
| --- | --- |
| `#d4d4d8` | Default output |
| `#52525b` | Timestamps, layer IDs (dim) |
| `#22d3ee` | uptrakit annotations |
| `#fafafa` | Docker status lines |
| `#4ade80` | Success lines |
| `#fcd34d` | Progress / in-flight layers |
| `#fdba74` | Warnings / errors |

---

## 7. Responsive Layout

**Status:** Target shell behavior. The current implementation still uses a persistent sidebar layout
across breakpoints; this section defines the convergence target for the shared shell.

Exit criteria:

- Tablet overlay sidebar behavior passes smoke tests and parity snapshots
- Mobile bottom navigation and overflow sheet render built-in and `surface.page` entries in the same
  priority order and visual shell
- Responsive parity snapshots pass for every required slot/body pair at their defined breakpoints
- Legacy persistent-sidebar-only responsive behavior is removed

Three breakpoints:

| Breakpoint | Range | Layout |
| --- | --- | --- |
| Desktop | ≥ 1024px | Full sidebar + top bar + content area |
| Tablet | 640–1023px | Sidebar hidden by default, slides in as overlay drawer on toggle |
| Mobile | < 640px | No sidebar; bottom navigation bar replaces sidebar nav |

### Tablet

- Sidebar collapses off-screen (`transform: translateX(-180px)`)
- Hamburger icon in top bar opens the sidebar as an overlay (`z-index: 20`) with a
  `rgba(0,0,0,.4)` backdrop
- Content area spans full width
- Stat cards reflow to 2-column grid
- Software page column grid compresses to `16px 1fr 90px 88px` (caret and badge columns unchanged)

### Mobile

- Bottom navigation bar: `56px` tall, `--bg-surface` background, top border `--border-subtle`
- Bottom bar shows the 4 highest-priority top-level nav items regardless of whether they are built-in
  or `surface.page` entries
- The same `priority`, then `label`, then origin (`built-in` before `surface.page`), then stable ID
  comparator used in the sidebar also applies on mobile
- Active item: `--accent` icon color and label text color (both change together)
- Remaining top-level nav items, regardless of origin, move into a shared overflow sheet or menu
- Top bar retains title only; search and action button collapse into a full-width bar
  below the title when the search icon is tapped
- Tables adapt to card-stack layout: each row becomes a card with label/value pairs
- Software page: software items show name + aggregate badge only; tap to inline-expand host rows
  (same page, no modal or separate view)

### Mobile overflow sheet

- Trigger: final bottom-nav slot labeled `More` when overflow exists
- Sheet: anchored to bottom, `--bg-surface`, top border `--border-subtle`, top radius `12px`
- Backdrop: `rgba(0,0,0,.4)`
- Contents: same nav-item typography, hover, and active treatment as the sidebar
- Built-in and surface page entries share the same sorting and rendering rules inside the sheet

### Built-in nav priorities

Default built-in top-level nav priorities are:

| Item | Priority |
| --- | --- |
| Home | `100` |
| Services | `200` |
| System Services | `300` |
| Hosts | `400` |
| Tags | `450` |
| Software | `500` |
| History | `800` |
| Audit Logs | `900` |
| Settings | `1000` |

### Toast position on mobile

**Status:** Target. This behavior promotes together with the rest of the mobile shell in Section 7.

On mobile, toasts appear at **bottom-center** instead of top-right to avoid overlapping
the top navigation area. Swipe-down to dismiss (threshold `80px`; swipe-right is used on tablet).

---

## 8. Interaction Conventions

- **No layout reflow on hover.** Any element that changes text on hover must live in a
  fixed-width container.
- **Consistent hover pattern.** All clickable badge-style elements (status badges, `↑ Update all`,
  navigable host badges) use the same treatment: background and border opacity increase,
  no shadow or transform.
- **Animation exceptions are centralized in Section 2.5.** Interactive controls stay flat by default,
  and any non-flat motion must use one of the explicitly allowed exception categories from that section.
- **Dim = disabled, not hidden.** Inactive controls (e.g. `↑ Update all` when nothing to update,
  yellow traffic light) are visible but visually receded and `pointer-events: none`.
- **Destructive actions** use the danger button variant and are segregated in a
  "Danger Zone" settings section, always gated by a confirmation dialog.
- **Focus visible only.** Focus rings appear on `:focus-visible` (keyboard navigation),
  not on mouse click.

---

## 9. Waiver Schema

`docs/superpowers/ui-parity-waivers.json` is a JSON array of waiver objects.

Each entry must contain:

- `scope`: string
- `owner`: string
- `expiry_date`: ISO `YYYY-MM-DD`
- `capture_region`: selector string or named capture preset
- `justification`: string
- `review_ref`: string

Example:

```json
[
  {
    "scope": "host_context_modal/mobile",
    "owner": "frontend-owner",
    "expiry_date": "2026-06-30",
    "capture_region": "[data-parity-region='host-context-modal']",
    "justification": "Temporary mobile overflow mismatch during shell convergence.",
    "review_ref": "PR-1234"
  }
]
```
