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
| **Accent** | `--accent` | `#2563eb` |
| Accent bright | `--accent-bright` | `#3b82f6` |
| Accent dark | `--accent-dark` | `#1d4ed8` |
| **Error** | `--color-error` | `#dc2626` |
| Error background tint | `--color-error-bg` | `rgba(220,38,38,.08)` |
| Error border | `--color-error-border` | `rgba(220,38,38,.3)` |

### 2.3 Border Radius

| Element | Radius |
| --- | --- |
| Page panels, modals, sidebar | `4px` |
| Cards, table wrappers, buttons | `3px` |
| Badges, pills, small chips | `2px` |
| Traffic light dots | `50%` |

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

#### Clickable badges (interactive variant)

Used where a badge doubles as an action trigger. The text swaps on hover; the column it lives in
has a **fixed width** so the swap never causes layout reflow.

Pattern: two sibling spans `.idle` / `.hov` inside the badge element.
CSS hides `.hov` by default and swaps on `:hover`.

On hover: background and border brighten (`rgba` opacity increases ~2×).

Examples in use:

| Page | Idle text | Hover text | Action |
| --- | --- | --- | --- |
| Software — host row | `Update Avail` | `↑ Update` | Trigger update for this host |
| Hosts — software column | `N updates` | `→ Software` | Navigate to Software filtered for this host |
| Hosts — software column | `X error` | `→ History` | Navigate to History filtered for this host |

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

For **in-progress** items: a `▶ view log` hint appears in the meta line.
Clicking the item opens the terminal modal.

### 5.4 Settings Page

Two-column layout: narrow nav (120px) + form body. Nav items follow the same active/hover pattern
as sidebar nav. Form uses label + input rows at `110px` fixed label width.

---

## 6. Terminal Modal (Xterm.js)

Used for live and historical update output. Opens as a centred modal over a
`rgba(0,0,0,.78)` backdrop.

### Opening / Closing

- Opened by clicking an in-progress or completed history item
- Closed by: clicking the red traffic light, pressing `Escape`, or clicking the backdrop

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
- Hover **any** of the three dots: icons appear on all three simultaneously
  - Red: `✕`, Yellow: `_`, Green: `+` (normal) / `⊡` (when maximized)
- Icons render in a dark semi-transparent color appropriate to each button's background

### Maximized State

Clicking the green dot expands the window to `92vw × 88vh` with a `0.18s ease` transition.
The terminal body grows to fill available height (`flex: 1`). Border radius reduces to `4px`.
Clicking the green dot again restores to the default `580px` fixed width.
Closing the modal always resets to normal size.

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

## 7. Interaction Conventions

- **No layout reflow on hover.** Any element that changes text on hover must live in a
  fixed-width container.
- **Consistent hover pattern.** All clickable badge-style elements (status badges, `↑ Update all`,
  navigable host badges) use the same treatment: background and border opacity increase,
  no shadow or transform.
- **Flat transitions only.** `background`, `border-color`, `color` — nothing else.
- **Dim = disabled, not hidden.** Inactive controls (e.g. `↑ Update all` when nothing to update,
  yellow traffic light) are visible but visually receded and `pointer-events: none`.
- **Destructive actions** use the danger button variant and are segregated in a
  "Danger Zone" settings section.
