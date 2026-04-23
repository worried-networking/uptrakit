# Software Page Row Alignment

**Date:** 2026-04-23  
**Status:** Approved

## Problem

The Software page mixes single-host compact rows and multi-host group rows in the same list.
The current implementation uses two different column grids:

- Multi-host: `16px 1fr 120px 88px` — 16px caret column present
- Single-host: `1fr 120px 88px` — no caret column

This causes a ~28px horizontal jump between row types (16px column + 12px gap). Stars, icons,
and names land at different x positions depending on row type. Visually jarring when both types
appear in the same list.

The current spec (§5.1) defines these two grids explicitly. This design supersedes that
definition.

## Solution

Unify all rows to `1fr 120px 88px`. Move the expand/collapse control off the dedicated column
and into the subtitle line as an inline pill button.

### Grid change

All software rows — header and host sub-rows, single- and multi-host — use:

```css
grid-template-columns: minmax(0,1fr) 120px 88px
```

The 16px caret column is removed entirely. No spacer or placeholder needed.

### Name cell layout

Both row types use the same name cell structure:

```text
[★/☆]  [icon?]  [software name]   ← name-line (flex row, gap 5px)
[expand-pill?]  [subtitle text]    ← sub-line  (flex row, gap 4px)
```

Stars and names are flush-left on every row. No invisible spacer.

### Expand/collapse pill (multi-host only)

Multi-host header rows show an interactive pill button in the sub-line:

```text
▼ 4 hosts  · up to date        ← expanded state
▶ 6 hosts  · 2 updates         ← collapsed state
```

The pill contains the glyph + host count. The trailing summary text (`· up to date`) lives
outside the pill as secondary text.

Single-host rows omit the pill entirely. Their sub-line shows host name + plugin pill only.

#### Pill token spec

| Property         | Value (both themes)                                          |
| ---------------- | ------------------------------------------------------------ |
| Idle background  | `rgba(var(--accent-rgb), .08)`                               |
| Idle border      | `rgba(var(--accent-rgb), .22)`                               |
| Idle text        | `var(--accent)`                                              |
| Hover background | `rgba(var(--accent-rgb), .18)`                               |
| Hover border     | `rgba(var(--accent-rgb), .42)`                               |
| Hover text       | `var(--accent-bright)`                                       |
| Height           | `14px`                                                       |
| Padding          | `0 5px`                                                      |
| Border radius    | `2px` (chips/badges rule, §2.3)                              |
| Font size        | `9px`, weight `600`                                          |
| Transition       | standard triplet per §2.5                                    |

Glyph sizes inside the pill:

- Expanded (`▼`): `font-size: 13px`
- Collapsed (`▶`): `font-size: 11px`

The pill is an interactive `<button>` element. It receives the standard focus ring from §2.6
on `:focus-visible`.

### Star contrast adjustment

The unfeatured star `☆` currently uses `text-surface-400` (Skeleton cerberus: `oklch(0.62 0 0)`
≈ `#878787` in dark). This is too low-contrast at small sizes.

New values — midpoint between `--text-muted` and `--text-secondary`:

| Theme | Value     | Between                                |
| ----- | --------- | -------------------------------------- |
| Dark  | `#78788a` | `#52525b` muted – `#a1a1aa` secondary  |
| Light | `#8496a8` | `#94a3b8` muted – `#64748b` secondary  |

No new semantic token is introduced. Applied via a scoped `<style>` block in `+page.svelte`:

```css
/* in +page.svelte <style> */
.star-unfeatured { color: #8496a8; }                         /* light */
:global(.dark) .star-unfeatured { color: #78788a; }          /* dark */
```

The existing `class:text-warning-500` / `class:text-surface-400` pair is replaced by
`class="star-unfeatured"` for the unfeatured state. The featured star `★` continues to use
`var(--color-warning)`.

### Host sub-row indent

Host sub-rows indent their name cell by `padding-left: 18px`. The leading dot `·` uses
`var(--border-default)` color. Unchanged from current implementation.

## Spec delta from §5.1

| Area                    | Current spec §5.1              | This design              |
| ----------------------- | ------------------------------ | ------------------------ |
| Multi-host grid         | `16px 1fr 120px 88px`          | `1fr 120px 88px`         |
| Single-host grid        | `1fr 120px 88px`               | `1fr 120px 88px`         |
| Caret/spacer column     | 16px dedicated grid column     | Removed                  |
| Expand/collapse trigger | `▾/▸` button in col 1          | Expand pill in sub-line  |
| Pill contains           | glyph only                     | glyph + host count       |
| Star-off color          | `text-surface-400` (~`#878787`)| `#78788a` / `#8496a8`    |

All other §5.1 rules remain in force: version column stack, badge spec, host sub-row
background, truncation pattern, single-host compact row rules, `↑ Update all` button.

## Unchanged

- Column widths `120px` (version) and `88px` (badge) — non-negotiable per §8
- Host sub-rows: transparent background until hover → `--bg-raised`
- Multi-host header rows: `--bg-raised` background
- `▸ N more` truncation row at 4+ hosts (col 2, `padding-left: 49px`)
- Single-host: no aggregate copy, no nested sub-row, singular action badge
- All badge, pill, button, and transition specs per §4.1–4.3 and §2.5

## Implementation scope

Confined to `frontend/src/routes/software/+page.svelte`:

1. Replace conditional `grid-cols-[16px_minmax(0,1fr)_120px_88px]` /
   `grid-cols-[minmax(0,1fr)_120px_88px]` with a single
   `grid-cols-[minmax(0,1fr)_120px_88px]` on all header and host-row elements
2. Remove the `<button>` chevron from col 1 of multi-host headers
3. Remove the `<div aria-hidden>` spacers from col 1 of single-host headers
4. Add the expand pill to the sub-line of multi-host header rows
5. Update star-off color from `text-surface-400` to scoped `.star-unfeatured` class
6. Update the loading skeleton row (also uses the conditional 4-col grid) to match

No other files change. No new components. No API changes.
