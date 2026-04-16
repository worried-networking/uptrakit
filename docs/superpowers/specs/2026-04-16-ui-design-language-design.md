# UI Design Language

**Date:** 2026-04-16
**Status:** Approved

## Overview

This document defines the visual design language for the uptrakit web UI.
It covers design tokens, component patterns, interactive conventions, and page-level layouts.
The goal is a coherent, dark-native interface that feels sharp and professional — not decorative.

---

## 1. Themes

The UI supports dark and light themes. **Dark is the default** when system preference is
unavailable; otherwise the theme follows `prefers-color-scheme`.

A theme switcher is available in the UI for manual override.

Both themes are fully specified. Light is not an afterthought — it must be usable and visually
comparable to dark.

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
| Accent bright | `--accent-bright` | `#22d3ee` |
| Accent dark | `--accent-dark` | `#0891b2` |
| Accent deep | `--accent-deep` | `#0e7490` |
| Success | `--color-success` | `#4ade80` |
| Warning | `--color-warning` | `#fbbf24` |
| **Error** | `--color-error` | `#fdba74` |
| Error background tint | `--color-error-bg` | `rgba(234,88,12,.15)` |
| Error border | `--color-error-border` | `rgba(234,88,12,.35)` |
| In-progress / info | `--color-info` | `#67e8f9` |

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
| Accent bright | `--accent-bright` | `#3b82f6` |
| Accent dark | `--accent-dark` | `#1d4ed8` |
| Accent deep | `--accent-deep` | `#1e40af` |
| Success | `--color-success` | `#16a34a` |
| Warning | `--color-warning` | `#d97706` |
| **Error** | `--color-error` | `#dc2626` |
| Error background tint | `--color-error-bg` | `rgba(220,38,38,.07)` |
| Error border | `--color-error-border` | `rgba(220,38,38,.3)` |
| In-progress / info | `--color-info` | `#0891b2` |

### 2.3 Border Radius

| Element | Radius |
| --- | --- |
| Page panels, modals, sidebar | `4px` |
| Cards, table wrappers, buttons | `3px` |
| Badges, pills, small chips | `2px` |
| Traffic light dots | `50%` |
| Toggle track | `10px` |

### 2.4 Typography

- **Font stack:** `-apple-system, BlinkMacSystemFont, 'Segoe UI', 'Inter', sans-serif`
- **Monospace stack:** `'SF Mono', 'Roboto Mono', monospace` — versions, digests, terminal output
- No custom web font loading; system fonts only to keep load instant.

### 2.5 Transitions

Interactive elements use a single short transition for background and border:

```css
transition: background .12s, border-color .12s, color .12s;
```

No transforms, no shadows appearing on hover. State changes are flat and immediate.
The only exception is the terminal modal maximize animation which uses `0.18s ease` on
`width` and `height`.

### 2.6 Focus States

Keyboard-navigable elements use a visible focus ring that does not rely on the default
browser outline:

```css
outline: none;
box-shadow: 0 0 0 3px rgba(var(--accent-rgb), .25);
```

Dark theme: `--accent-rgb` resolves to `6 182 212`. Light theme: `37 99 235`.
Focus rings appear only on `:focus-visible` (keyboard navigation), not on click.

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

---

## 3. Layout Shell

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
  - Active: `rgba(accent, .1)` background, `--accent-bright` text, colored nav icon

### Top Bar

- Height: `40px`, bottom border `--border-subtle`
- Page title (bold, `12px`) + optional chip (item count)
- Right side: search input + primary action button

### Content Area

- Padding: `12px 14px`
- Scrollable independently of the shell

---

## 4. Components

### 4.1 Badges

Badges are `14px` tall, `2px` radius, `7.5px` bold uppercase text with `letter-spacing: .04em`.
They always have both a background tint and a 1px border.

| Variant | Background | Text | Border |
| --- | --- | --- | --- |
| Green (up to date / success) | `rgba(74,222,128,.10)` | `#4ade80` | `rgba(74,222,128,.2)` |
| Teal (update / in-progress) | `rgba(6,182,212,.10)` | `#67e8f9` | `rgba(6,182,212,.22)` |
| Orange (error / failed) | `rgba(234,88,12,.15)` | `#fdba74` | `rgba(234,88,12,.35)` |
| Amber (warning) | `rgba(251,191,36,.12)` | `#fcd34d` | `rgba(251,191,36,.3)` |
| Dim (unknown / offline) | `rgba(148,163,184,.08)` | `#71717a` | `#27272a` |

These values are for the dark theme. Light theme badges use the same structural pattern with
`--color-success`, `--accent`, `--color-error`, `--color-warning`, `--text-muted` token values
respectively.

#### Clickable badges (interactive variant)

Used where a badge doubles as an action trigger. The text swaps on hover; the column it lives in
has a **fixed width** so the swap never causes layout reflow.

Pattern: two sibling spans `.idle` / `.hov` inside the badge element.
CSS hides `.hov` by default and swaps on `:hover`.

On hover: background and border opacity increases approximately 2×.

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

### 4.2 Pills

Pills are `12px` tall, `2px` radius, `7px` bold uppercase, no border.
Used for categorical labels (agent type, OS, plugin type).

| Variant | Use |
| --- | --- |
| Purple tint | SSH agent |
| Teal tint | Local agent, Docker plugin |
| Green tint | Linux OS |
| Grey tint | macOS, GitHub plugin |
| Yellow tint | Homebrew plugin |

### 4.3 Buttons

Standard button height: `23px`, `3px` radius, `9px` bold text.

| Variant | Style |
| --- | --- |
| Primary | Teal gradient (`#0e7490` → `#06b6d4`), white text, no border |
| Ghost | Transparent, `--border-default` border, `--text-primary` text |
| Danger | Orange tint background + border, `--color-error` text |

**Disabled state:** All variants use `opacity: 0.4` when `disabled`. `pointer-events: none`.
No border or background change — the opacity communicates the state clearly without a
separate disabled color set.

#### `↑ Update all` button

Appears on software header rows. Uses the same interaction pattern as clickable badges rather than
the standard button style — it reads as a badge-level control, not a page-level action.

- Idle: `rgba(accent, .06)` background, dim accent border, accent text
- Hover: `rgba(accent, .18)` background, brighter border, brighter text
- Dim (nothing to update): transparent background, `--border-default` border,
  `--text-muted` text — `pointer-events: none`

### 4.4 Toggles

`28×15px`, `10px` radius pill. Off: `--border-default` background.
On: `rgba(accent, .5)` background with accent border.
Thumb moves from `left: 2px` to `left: 15px`.

### 4.5 Stat Cards

Used at the top of list pages (Hosts). `3px` radius, `--bg-surface` background,
`--border-subtle` border. Label in `7.5px` uppercase, value in `14px` bold.
Values are color-coded: green = healthy, amber = needs attention, orange = error,
dim = offline/unknown.

### 4.6 Loading States

Three patterns are used depending on context:

**Skeleton placeholders** — used when the page or a list section is loading its initial data.
Skeleton elements mimic the shape of the content they replace (rows, badges, text lines).

- Background: `--bg-raised` tinted with subtle opacity pulse
- Animation: `opacity` pulses between `0.35` and `0.70` over `1.4s ease-in-out infinite`
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

**Dismissal:**

- Click anywhere on the toast body (entire card is clickable)
- Swipe right on touch devices
- Auto-dismiss after timeout (success/info: 4s, error/warning: 8s)
- Explicit close button (`✕`) visible on the right edge

A **progress bar** depletes along the bottom of the toast over the auto-dismiss duration,
giving a visual countdown. The bar is color-matched to the toast variant.

**Structure:** icon square + body (title + description) + close button.
Toast body has `cursor: pointer` and a subtle background shift on hover (`--bg-raised`).

**Variants:**

| Variant | Use | Color |
| --- | --- | --- |
| Success | Update triggered, operation completed | `--color-success` |
| Error | Update failed, connection lost | `--color-error` |
| Info | Updates available, background event | `--color-info` |
| Warning | Host offline, configuration issue | `--color-warning` |

**Swipe-to-dismiss:** on touch devices, a right-swipe gesture (threshold `80px`) triggers
a slide-out animation followed by removal. This is a JS behavior, not CSS-only.

### 4.9 Confirmation Dialogs

Used for destructive or irreversible actions (delete host, remove plugin config, revoke token).

- Centred modal over a `rgba(0,0,0,.55)` backdrop (lighter than the terminal modal)
- Width: `380px` fixed, `4px` radius
- Title: `13px` bold
- Body: `11px` `--text-secondary`, describes what will happen
- Actions row: right-aligned, cancel (ghost) + confirm (danger)
- Close on backdrop click or `Escape`

The confirm button uses the danger variant and is labeled with the specific action
(e.g. "Delete host", "Revoke token") rather than a generic "Confirm".

### 4.10 Form Validation

Validation is inline — errors appear immediately below their field, not in a summary block.

**Error state:**

- Input border: `--color-error-border`
- Input background: `--color-error-bg`
- Error message: `10px`, `--color-error`, appears below the input with a small `✕` icon prefix
- No red outline ring — border color change alone is sufficient

**Success state (optional):**

- Used only for fields with meaningful validation (e.g. hostname format check)
- Input border: `rgba(--color-success, .35)`
- Small `✓` icon at input right edge

**Focus state:**

- `box-shadow: 0 0 0 3px rgba(var(--accent-rgb), .2)`
- Border color: `--accent`
- Applies on `:focus-visible` only

**Label layout:** `110px` fixed label width, input takes remaining space.
Labels are `10px` bold, `--text-secondary`.

---

## 5. Page Patterns

### 5.1 Software Page

The central view. Software items are the top-level grouping; hosts are sub-rows.

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
- Col 3: (empty on header) / version — fixed `120px`, right-aligned
- Col 4: `↑ Update all` / status badge — fixed `88px`, right-aligned

Fixed column widths are non-negotiable — they prevent layout reflow when badge text changes
on hover.

#### Version column

Single column showing stacked values:

- Line 1: current installed version (`--text-secondary`, monospace)
- Line 2: `↓ new-version` in `--accent-bright` (only when update is available)

#### Truncation

When a software item has more than 3 hosts, only 3 are shown. A `▸ N more` row follows,
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

Stats cards above the table: Online, Offline, Updates pending, Errors.

### 5.3 History Page

Chronological feed of update events, grouped by date with separator labels.

Each item:

- Left: colored icon square (✓ success, ✕ failed, ↑ in-progress, · pending)
- Body: `software on host`, version change (`old → new` in monospace, new in teal), plugin type
- Right: status badge + relative timestamp

Icon square colors:

| State | Background | Icon color |
| --- | --- | --- |
| Success | `rgba(74,222,128,.12)` | `#4ade80` |
| Failed | `rgba(234,88,12,.15)` | `#fdba74` |
| In-progress | `rgba(6,182,212,.12)` | `#67e8f9` |
| Pending | `rgba(148,163,184,.08)` | `#71717a` |

For **in-progress** items: a `▶ view log` hint appears in the meta line.
Clicking the item opens the terminal modal.

### 5.4 Settings Page

Two-column layout: narrow nav (120px) + form body. Nav items follow the same active/hover pattern
as sidebar nav. Form uses label + input rows at `110px` fixed label width (see Section 4.10).

Destructive actions (delete account, revoke all tokens) are grouped in a "Danger Zone" section
with a danger-variant button and confirmation dialog (see Section 4.9).

---

## 6. Terminal Modal (Xterm.js)

Used for live and historical update output. Opens as a centred modal over a
`rgba(0,0,0,.78)` backdrop.

### Opening / Closing

- Opened by clicking an in-progress or completed history item
- Closed by: clicking the red traffic light, pressing `Escape`, or clicking the backdrop
- Modal state is managed via JS `classList.toggle('open')` on the modal element, not CSS `:target`
- Closing always resets to non-maximized size

### Window Chrome

The terminal window uses macOS-style traffic light controls in the title bar.

Traffic light states:

| Button | Color | Always? | Function |
| --- | --- | --- | --- |
| Red (close) | `#ff5f57` | Always colored | Closes the modal |
| Yellow (minimize) | `#3f3f46` grey | Always grey | No-op — minimize is meaningless for a modal |
| Green (maximize) | `#27c840` | Always colored | Toggles maximized state |

Interaction:

- Default: icons are invisible (`color: transparent`)
- Hover **any** of the three dots: icons appear on all three simultaneously using
  `.xterm-dots:hover .xterm-dot` CSS selector
  - Red: `✕`, Yellow: `_`, Green: `+` (normal) / `⊡` (when maximized)
- Icons render in a dark semi-transparent color appropriate to each button's background

### Maximized State

Clicking the green dot expands the window to `92vw × 88vh` with a `0.18s ease` transition
on `width` and `height`. The terminal body grows to fill available height (`flex: 1`).
Border radius reduces to `4px`. Clicking the green dot again restores to the default `580px`
fixed width. Closing the modal always resets to normal size.

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

Terminal body uses `white-space: pre` to preserve output formatting.
The status bar shows the update status badge (same variants as history items) and metadata
(host name, start time, duration).

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

Three breakpoints:

| Breakpoint | Range | Layout |
| --- | --- | --- |
| Desktop | ≥ 1024px | Full sidebar + top bar + content area |
| Tablet | 640–1023px | Sidebar hidden by default, slides in as overlay drawer on toggle |
| Mobile | < 640px | No sidebar; bottom navigation bar replaces sidebar nav |

### Tablet

- Sidebar collapses off-screen (`transform: translateX(-180px)`)
- Hamburger icon in top bar opens the sidebar as an overlay (`z-index: 20`) with a
  semi-transparent backdrop
- Content area spans full width
- Stat cards reflow to 2-column grid
- Software page column grid compresses: version column drops to `90px`

### Mobile

- Bottom navigation bar: `56px` tall, `--bg-surface` background, top border `--border-subtle`
- Icons + labels for the 4 main sections (Software, Hosts, History, Settings)
- Active item: `--accent` icon color
- Top bar retains title only; search and action button collapse into a full-width bar
  below the title when the search icon is tapped
- Tables adapt to card-stack layout: each row becomes a card with label/value pairs
- Software page: software items show name + aggregate badge only; tap to expand into detail view

### Toast position on mobile

On mobile, toasts appear at the **bottom-center** instead of top-right to avoid overlapping
the top navigation area. Swipe-down to dismiss on mobile (instead of swipe-right on desktop).

---

## 8. Interaction Conventions

- **No layout reflow on hover.** Any element that changes text on hover must live in a
  fixed-width container.
- **Consistent hover pattern.** All clickable badge-style elements (status badges, `↑ Update all`,
  navigable host badges) use the same treatment: background and border opacity increase,
  no shadow or transform.
- **Flat transitions only.** `background`, `border-color`, `color` — nothing else. The sole
  exception is the terminal maximize transition which animates `width` and `height`.
- **Dim = disabled, not hidden.** Inactive controls (e.g. `↑ Update all` when nothing to update,
  yellow traffic light) are visible but visually receded and `pointer-events: none`.
- **Destructive actions** use the danger button variant and are segregated in a
  "Danger Zone" settings section, always gated by a confirmation dialog.
- **Focus visible only.** Focus rings appear on `:focus-visible` (keyboard navigation),
  not on mouse click.
