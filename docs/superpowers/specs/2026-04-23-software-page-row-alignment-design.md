<!-- markdownlint-disable MD013 -->

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

The current spec (§5.1) defines these two grids and the associated ASCII structure diagram.
**This design supersedes:**

- §5.1 column grid table
- §5.1 ASCII structure diagram
- §5.1 column-grid prose block (the `Col 1: caret / spacer`, `Col 2: …` description)
- §5.1 all other text describing the caret column
- §7 tablet grid sentence (`Software page column grid compresses to 16px 1fr 90px 88px`)

All other §5.1 and §7 rules remain in force.

## Solution

Unify all rows to `1fr 120px 88px`. Move the expand/collapse control off the dedicated column
and into the subtitle line as an inline pill button.

### Grid change

All software rows — header and host sub-rows, single- and multi-host — use:

```css
grid-template-columns: minmax(0,1fr) 120px 88px
```

The 16px caret column is removed entirely. No spacer or placeholder needed.

### Tablet compressed grid

§7 defines the tablet compressed grid as `16px 1fr 90px 88px`. With the caret column removed,
the tablet grid becomes:

```css
grid-template-columns: minmax(0,1fr) 90px 88px
```

Badge column (`88px`) is unchanged. Version column compresses from `120px` to `90px` on tablet,
as before.

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

This is an **interactive accent-tinted button**, not a §4.2 categorical pill. §4.2 categorical
pills (`12px`, `7.5px` bold uppercase, no border) label agent type, OS, and plugin type. The
expand pill is a clickable control at badge scale with an accent border.

| Property         | Value                                                                |
| ---------------- | -------------------------------------------------------------------- |
| Idle background  | `rgba(var(--accent-rgb), .08)`                                       |
| Idle border      | `rgba(var(--accent-rgb), .22)`                                       |
| Idle text        | `var(--accent)`                                                      |
| Hover background | `rgba(var(--accent-rgb), .18)`                                       |
| Hover border     | `rgba(var(--accent-rgb), .42)`                                       |
| Hover text       | `var(--accent-bright)`                                               |
| Height           | `14px`                                                               |
| Padding          | `0 5px`                                                              |
| Border radius    | `2px` (chips/badges rule, §2.3)                                      |
| Font size        | `9px`, weight `600`, `text-transform: none`                          |
| Internal gap     | `3px` between glyph and count text                                   |
| Overflow         | `hidden` (prevents glyph clip artifact)                              |
| Transition       | standard triplet per §2.5                                            |

Font weight `600` and no uppercase: the pill label is informational text (`4 hosts`), not a
command or category label. Button labels (§4.3) are `700` uppercase; this pill is not a §4.3
button, and forcing uppercase onto a number-and-noun string (`4 hosts`) is incorrect.

Glyph sizes inside the pill. Both glyphs use `line-height: 1` with `display: flex; align-items: center`
for vertical centering within the `14px` pill height:

- Expanded (`▼`): `font-size: 13px`
- Collapsed (`▶`): `font-size: 11px`

The pill is an interactive `<button>` element. It receives the standard focus ring from §2.6
on `:focus-visible`.

### Star contrast adjustment

The unfeatured star `☆` currently uses `text-surface-400` (Skeleton cerberus: `oklch(0.62 0 0)`
≈ `#878787` in dark). This is too low-contrast at small sizes.

New values — chosen for visual legibility at `13px`, positioned between `--text-muted` and
`--text-secondary` for clear inactive appearance without competing with the featured star:

| Theme | Value     | Range                                  |
| ----- | --------- | -------------------------------------- |
| Dark  | `#78788a` | `#52525b` muted – `#a1a1aa` secondary  |
| Light | `#8496a8` | `#64748b` secondary – `#94a3b8` muted  |

Note: in light theme, `--text-muted` (`#94a3b8`) is lighter than `--text-secondary` (`#64748b`),
which is the inverse of the dark theme relationship. The table ordering reflects each theme's
dark-to-light direction.

**Token exception:** `tokens.md` prohibits hardcoded hex values "where a semantic token exists."
No semantic token exists for unfeatured-star color. One is not introduced here because this
color serves a single element in one component and adding it to the global token manifest would
be disproportionate. Applied via a scoped `<style>` block in `+page.svelte` — Svelte scoped
styles are the canonical component-level mechanism for per-component values that do not belong
in the global namespace. This is not a Tailwind palette utility class and does not violate the
tokens.md rule against raw Tailwind palette utilities.

```css
/* in +page.svelte <style> */
.star-unfeatured { color: #8496a8; }               /* light */
:global(.dark) .star-unfeatured { color: #78788a; } /* dark */
```

`:global(.dark)` targets the `.dark` class that Skeleton cerberus applies to the root element
when dark mode is active.

The existing `class:text-warning-500` / `class:text-surface-400` pair is replaced by
`class="star-unfeatured"` for the unfeatured state. The featured star `★` continues to use
`var(--color-warning)`.

### Host sub-row indent

Host sub-rows indent their name cell by `padding-left: 18px`. The leading dot `·` uses
`var(--border-default)` color. Unchanged from current implementation.

## Spec delta from §5.1

| Area                    | Current spec §5.1                           | This design              |
| ----------------------- | ------------------------------------------- | ------------------------ |
| Multi-host desktop grid | `16px 1fr 120px 88px`                       | `1fr 120px 88px`         |
| Single-host desktop     | `1fr 120px 88px`                            | `1fr 120px 88px`         |
| Multi-host tablet grid  | `16px 1fr 90px 88px`                        | `1fr 90px 88px`          |
| Caret/spacer column     | 16px dedicated grid column                  | Removed                  |
| Expand/collapse trigger | `▾/▸` button in col 1                       | Expand pill in sub-line  |
| Pill contains           | glyph only                                  | glyph + host count       |
| Star-off color          | `text-surface-400` (`oklch(0.62 0 0)`)      | `#78788a` / `#8496a8`    |
| §5.1 ASCII diagram      | Canonical                                   | Superseded               |
| §5.1 column prose block | Canonical                                   | Superseded               |

All other §5.1 rules remain in force: version column stack, badge spec, host sub-row
background, truncation pattern, single-host compact row rules, `↑ Update all` button.

## Unchanged

- Column widths `120px` (version, desktop) and `88px` (badge) — non-negotiable per §8
- Tablet version column `90px` — non-negotiable per §7
- Host sub-rows: transparent background until hover → `--bg-raised`
- Multi-host header rows: `--bg-raised` background
- `▸ N more` truncation row at 4+ hosts — `padding-left: 49px` within the name cell
  (1fr column). This padding is relative to the left edge of the name cell, not the outer
  row. In both old and new grids, host sub-rows indent `18px` within the same name cell and
  `▸ N more` indents `49px` — the `31px` difference is intentional and unchanged.
- Single-host: no aggregate copy, no nested sub-row, singular action badge
- All badge, pill, button, and transition specs per §4.1–4.3 and §2.5

## Implementation scope

Confined to `frontend/src/routes/software/+page.svelte`:

1. Replace conditional `grid-cols-[16px_minmax(0,1fr)_120px_88px]` /
   `grid-cols-[minmax(0,1fr)_120px_88px]` with a single
   `grid-cols-[minmax(0,1fr)_120px_88px]` on all header and host-row elements
2. Apply same unification to tablet breakpoint: replace
   `grid-cols-[16px_minmax(0,1fr)_90px_88px]` with `grid-cols-[minmax(0,1fr)_90px_88px]`
3. Remove the `<button>` chevron from col 1 of multi-host headers
4. Remove the `<div aria-hidden>` spacers from col 1 of single-host headers
5. Add the expand pill to the sub-line of multi-host header rows
6. Update star-off color from `text-surface-400` to scoped `.star-unfeatured` class
7. Update the loading skeleton row (uses `--bg-raised` shimmer, same `1fr 120px 88px` grid)
   to match the unified grid

No other files change. No new components. No API changes. No `tokens.ts` updates required.
