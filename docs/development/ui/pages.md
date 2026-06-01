<!-- markdownlint-disable MD013 -->

# Page Patterns

Feature-specific conventions for individual product pages — column grids, badge navigation, terminal modals, slot-backed panels. These rules sit on
top of the shell measurements (`layout.md`) and the shared primitives (`primitives.md`, `forms.md`) and apply to specific routes only.

If a rule applies to every page (e.g. focus rings, accent tints, button placement), it belongs in `tokens.md` or `primitives.md`. If a rule is about a
single route's layout decisions (which columns the Software table has, what the Hosts navigable badge does), it belongs here.

---

## Filter Bar Convention

Table pages in scope of the 2026-05-26 unified-filter-bars work share one filter-shell pattern, regardless of which page-section below documents them:

- `FilterBar` lives inside the table card header (`SectionCard` with `filterBar?: Snippet` prop, or a custom card wrapper).
- Filter values are URL-reactive via `createUrlParam` — the URL is the source of truth.
- Inline text search uses `ExpandableSearch`.
- Enum filters use `<Select>` (no accent button groups).

Pages in scope: Software, Hosts, History, Services, System Services, Host Tags. Each page section below cross-references this convention rather than
repeating it. Full primitive docs are in `primitives.md` under "Filter Primitives".

---

## Software Page

**Status:** `Implemented`

![Software group row showing item name, version, and update badge](../../../frontend/tests/e2e/ui-parity.test.ts-snapshots/ui-parity-software-group-row-chromium.png)

- Software items are top-level groups; hosts are sub-rows.
- Featured/unfeatured/all selection is a `<Select>` in the filter bar — driven by `?featured=` (default `featured`). Surface-backed `software.tabs`
  render as a `TabStrip` only when at least one surface is registered, and the first surface is auto-selected client-side (no URL persistence).
- Ignore Rules render as a collapsible `<details>` card below the main software card. The body is lazy-mounted via `{#if open}` so its API calls are
  deferred until the user expands it.
- Column grid: `16px 1fr 120px 88px`.
- Header-row version column is always empty; version column on host rows is a two-line installed/latest stack.
- Host-row background is transparent until hover.
- Truncation row uses `▸ N more`.
- Filters follow the Filter Bar Convention above.

---

## Hosts Page

**Status:** `Implemented`

- Standard table layout.
- Software-status badge uses the navigable badge pattern (see `ActionBadge` in `primitives.md`).
- `N updates` navigates to Software; `X error` navigates to History.
- `Up to date` and `Unknown` are static `StatusBadge` instances.
- Filters follow the Filter Bar Convention above.

---

## History Page

**Status:** `Implemented`

![History feed row with status icon, software name, version change, and metadata](../../../frontend/tests/e2e/ui-parity.test.ts-snapshots/ui-parity-history-feed-row-chromium.png)

- Chronological feed grouped by date.
- Icon square + body + right meta per row.
- Row-level "view log" actions open the shared terminal modal.
- Waiting/no-output, truncation, recovery, and actor details render as terminal callouts inside the modal.
- Interactive sessions expose live controls (e.g. `Ctrl+C`) inside terminal status actions.
- Filters follow the Filter Bar Convention above.

---

## Settings Page

**Status:** `Implemented`

![Settings tab strip with built-in tabs and a surface-contributed tab active](../../../frontend/tests/e2e/ui-parity.test.ts-snapshots/ui-parity-settings-tabs-chromium.png)

- Built-in settings sections and `settings.tabs` share one tab strip.
- Active tab persists in `?tab=<tab-id>`.
- Form-heavy views derive their label-column width from the `FormLayout` context — `11rem` inside modals, `20rem` on pages. See `forms.md` FormLayout
  Context for the canonical explanation. No manual width override is needed.
- Destructive actions live in a danger zone at the bottom of the page.
- `settings.below.global` renders below built-in global settings content.

---

## Slot-Backed Detail Panels

**Status:** `Implemented`

- `host_detail.tabs` is an inline card stack (not a tab strip).
- `settings.below.global` is an inline panel stack.
- Targeted surfaces keep their provider selector inside host-owned panel chrome, above rendered nodes.
- No-provider state uses the shared `EmptyState` component.
- Parity capture regions use stable host markers such as `data-parity-region`.

For the underlying slot contract, see the Slot Registry in `surfaces.md`.
