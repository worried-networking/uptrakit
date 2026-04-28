<!-- markdownlint-disable MD013 -->

# Website Phase 2 — Docs Hub

**Status:** Approved (design)
**Date:** 2026-04-28
**Scope:** Phase 2 of the public Uptrakit website at `https://uptrakit.org`. Adds a documentation
hub at `/docs/` built on the Phase 1 Zola foundation. Primarily targets end-user guides and
security documentation.

## Goal

Extend the Phase 1 marketing site into a documentation hub that:

- publishes `docs/end-user/` and `docs/security/` as browsable, searchable web docs at `/docs/`
- stays single-source — source files in `docs/` are authoritative; the website reads them via
  symlinks with no copy-on-publish step
- inherits the Phase 1 design language with no new token definitions
- adds Pagefind-powered client-side search
- is structured so remaining doc sections (`docs/api/`, `docs/architecture/`, `docs/development/`,
  `docs/hackme/`) can be published later without throwaway work

## Non-Goals

- Publishing `docs/api/`, `docs/architecture/`, `docs/development/`, or `docs/hackme/`. Phase 3+.
- Versioned docs. URL structure (`/docs/end-user/…`) reserves space for it; no implementation now.
- A blog, changelog, or tutorial series.
- New CSS tokens beyond those already defined in Phase 1.
- Server-side rendering or dynamic content.

## Decisions

### Content scope

| Section | Published | Notes |
| --- | --- | --- |
| `docs/end-user/` | Yes | Primary audience: homelab operators |
| `docs/security/` | Yes | Security posture matters to evaluators |
| `docs/api/` | No | Too narrow/contributor-facing for Phase 2 |
| `docs/architecture/` | No | Contributor-facing |
| `docs/development/` | No | Contributor-facing |
| `docs/hackme/` | No | Not appropriate for public site |

### Single-source via symlinks

`docs/end-user/` and `docs/security/` are the authoritative sources. The website reads them via
symlinks in `website/content/docs/`:

```text
website/content/docs/end-user/ → ../../../docs/end-user/   (symlink)
website/content/docs/security/ → ../../../docs/security/   (symlink)
```

Symlinks are committed to the repo. `actions/checkout@v4` preserves them on Linux runners.
Zola follows symlinks during build. No preprocessing, no copying.

### URL structure

URLs mirror the filesystem. Zola's `zola check` validates all internal links. Relative `.md`
cross-links between published pages are rewritten to page URLs automatically by Zola. Links
targeting unpublished sections (`docs/api/`, `docs/development/`, etc.) are rewritten to
`https://github.com/worried-networking/uptrakit/blob/main/docs/…` URLs at the time front
matter is added to each page. `zola check` enforces this — broken internal links block CI.

### Front matter

No existing `docs/end-user/` or `docs/security/` file has front matter. Zola requires front
matter on every page. YAML front matter (`---`) is added directly to source files — it is
colocated with content (no drift risk), and GitHub renders YAML front matter gracefully.

Minimum required fields:

```yaml
---
title: Page Title
weight: 10
description: One-sentence summary for search and OG.
---
```

Weights use steps of 10 to allow insertion without renumbering. Weight ordering derives from
the existing `README.md` contents lists in each section, which already reflect the intended
reading order.

### Section indexes

Zola requires `_index.md` for section index pages. Source files use `README.md` for GitHub
directory browsing. These coexist:

- `_index.md` — Zola-specific infrastructure only (front matter: `title`, `sort_by = "weight"`);
  no body content. Added to every section directory that is published.
- `README.md` — becomes a regular Zola page, titled "Overview", with `weight = 1` so it
  appears first in the sidebar. GitHub directory browsing continues to work unchanged.

### GFM alerts

`github_alerts = true` is added to `[markdown]` in `website/config.toml`. This feature landed
in Zola v0.21.0 (we use 0.22.1). Zola renders `> [!NOTE]` etc. as
`<blockquote class="markdown-alert-note">`.

All existing callouts in `docs/end-user/` and `docs/security/` are migrated to GFM alert
syntax as part of the front matter authoring pass:

| Old pattern | New syntax |
| --- | --- |
| `> **Note:**` | `> [!NOTE]` |
| `> **Tip:**` | `> [!TIP]` |
| `> **Important:**` | `> [!IMPORTANT]` |
| `> **Warning:**` | `> [!WARNING]` |
| `> **Security note:**` | `> [!CAUTION]` |

### Syntax highlighting

Built-in Syntect themes: `base16-ocean-dark` (dark), `base16-ocean-light` (light). CSS overrides
in `site.css` set `pre` background to `--bg-raised` and border to `--border-subtle` under the
appropriate `[data-theme]` selectors. A pixel-perfect token-matched theme is deferred.

### Versioning

No version selector. URL structure uses `/docs/end-user/…` (no version segment). Adding
versioning later can be done with redirects from `/docs/…` to `/docs/v2/…` without breaking
existing links.

### Phase 1 install page update

`/install/` currently links to the GitHub source for `docs/end-user/deployment/docker.md` as
the canonical deployment reference. Phase 2 updates this link to
`/docs/end-user/deployment/` (the deployment section index on the website). No redirect from
the old GitHub URL is added — backwards compatibility is not a concern here.

## Architecture

### Repository layout changes

```text
repo-root/
├── docs/
│   ├── end-user/              ← add YAML front matter + GFM alert migration
│   │   ├── _index.md          ← new: Zola section metadata only
│   │   ├── README.md          ← becomes "Overview" page (weight = 1)
│   │   ├── deployment/
│   │   │   ├── _index.md      ← new: Zola section metadata only
│   │   │   ├── README.md      ← becomes "Overview" page (weight = 1)
│   │   │   └── *.md           ← add front matter
│   │   ├── plugins/
│   │   │   ├── _index.md      ← new: Zola section metadata only
│   │   │   └── *.md           ← add front matter
│   │   └── *.md               ← add front matter
│   └── security/
│       ├── _index.md          ← new: Zola section metadata only
│       ├── README.md          ← becomes "Overview" page (weight = 1)
│       └── *.md               ← add front matter
└── website/
    ├── config.toml            ← add github_alerts = true; syntax highlight theme
    ├── content/
    │   └── docs/
    │       ├── _index.md      ← docs hub landing (links to sections)
    │       ├── end-user/      ← symlink → ../../../docs/end-user/
    │       └── security/      ← symlink → ../../../docs/security/
    ├── templates/
    │   ├── base.html          ← add "Docs" + "Install" to top bar
    │   └── docs.html          ← new: unified docs template
    └── static/
        ├── og.png             ← new: 1200×630 social card
        └── css/
            └── site.css       ← add docs layout, sidebar, GFM alert styles,
                               ←     syntax highlight overrides
```

### Routes

| Route | Source | Notes |
| --- | --- | --- |
| `/docs/` | `content/docs/_index.md` | Hub landing; links to sections |
| `/docs/end-user/` | `docs/end-user/_index.md` (via symlink) | Section index; alpha banner |
| `/docs/end-user/deployment/` | `docs/end-user/deployment/_index.md` | Subsection index; alpha banner |
| `/docs/end-user/<page>/` | `docs/end-user/<page>.md` | Leaf page |
| `/docs/security/` | `docs/security/_index.md` (via symlink) | Section index; alpha banner |
| `/docs/security/<page>/` | `docs/security/<page>.md` | Leaf page |

### `docs.html` template

Single unified template extending `base.html`. Handles both section indexes
(`{% if section %}`) and leaf pages (`{% if page %}`).

**Layout:** Two-column on desktop — sidebar `~240 px` fixed left, content area `max-width 880 px`.
Single column on mobile (≤ 768 px) with hamburger drawer.

**Breadcrumbs:** Rendered above the `h1` via `page.ancestors` loop. Example:
`Docs › End-user › Deployment › Docker Deployment`.

**Sidebar:**

- Generated from `get_section()` tree.
- Nested subsections (`deployment/`, `plugins/`) are collapsible. Collapse is CSS-only via
  `:has()` + Zola's `current_path` check — no JS required.
- Active page is highlighted.
- Pagefind search widget rendered in the sidebar header.

**Prev/next:** `page.lower` / `page.higher` (weight-ordered) rendered at the bottom of each
leaf page. Not shown on section index pages.

**Edit-on-GitHub:** Rendered in the page footer on every page:
`https://github.com/worried-networking/uptrakit/blob/main/{{ page.relative_path }}`.

**Alpha banner:** Warning callout (`--color-warning-bg` / `--color-warning-border`) rendered
below breadcrumbs on section index pages only (`{% if section %}`). Not shown on leaf pages.

### Top bar changes (`base.html`)

"Docs" (→ `/docs/`) and "Install" (→ `/install/`) text links are added between the wordmark
and the GitHub icon. Active state applied to the link matching the current route prefix.

Order left to right: wordmark + favicon | "Docs" | "Install" | GitHub icon | theme toggle.

### CSS additions (`site.css`)

**Docs layout:**

- Two-column grid; sidebar `~240 px`, content `max-width 880 px`.
- Responsive breakpoint at `≤ 768 px`: single column, hamburger toggle shows.
- Sidebar drawer overlay uses `--bg-surface` background and `--border-subtle` border.
- Hamburger drawer requires ~30 lines of JS for open/close toggle.

**GFM alert styles:**

| CSS class | Background token | Border token |
| --- | --- | --- |
| `.markdown-alert-note` | `--color-info-bg` | `--color-info-border` |
| `.markdown-alert-tip` | `--color-success-bg` | `--color-success-border` |
| `.markdown-alert-important` | `--color-warning-bg` | `--color-warning-border` |
| `.markdown-alert-warning` | `--color-warning-bg` | `--color-warning-border` |
| `.markdown-alert-caution` | `--color-danger-bg` | `--color-danger-border` |

**Syntax highlighting overrides:**

```css
[data-theme="dark"] pre { background: var(--bg-raised); border: 1px solid var(--border-subtle); border-radius: 3px; }
[data-theme="light"] pre { background: var(--bg-raised); border: 1px solid var(--border-subtle); border-radius: 3px; }
```

Syntect theme `base16-ocean-dark` for dark, `base16-ocean-light` for light, set in
`website/config.toml` under `[markdown]`.

## OG Image

`website/static/og.png` — 1200×630 PNG:

- Background: `#0F172A`
- Favicon SVG (from `website/static/favicon.svg`) at 400×400 px, centered horizontally,
  top edge at 58 px from top. Rendered with the rounded plate (`rx="96"` rect, fill `#1e293b`)
  visible so the icon reads as a distinct element against the background.
- Wordmark `uptrakit` in `SF Mono`/`Roboto Mono` monospace, 48 px, weight 500, color `#e4e4e7`,
  centered horizontally at the vertical midpoint between the icon bottom edge and the card
  bottom edge (≈ 544 px from top).

`base.html` adds `<meta property="og:image" content="{{ get_url(path='og.png') }}">`.
The Phase 1 spec noted this asset was optional in Phase 1; Phase 2 ships it.

## Search

Pagefind (`pagefind@1`, pinned) provides client-side search. The Pagefind UI widget is embedded
in the sidebar header via a `<link>` to `/pagefind/pagefind-ui.css` and a `<script>` to
`/pagefind/pagefind-ui.js`, plus a small init call. The widget loads lazily — WASM is not
fetched until the user focuses the search input. Both the JS bundle and the CSS are emitted by
`pagefind --site public` into `public/pagefind/` and served as static assets.

## Build and Deploy

### Workflow changes

`zola check` and `zola build` steps are unchanged. Two additions:

1. Combined tool install (replaces the Phase 1 `zola`-only install):

   ```yaml
   - uses: taiki-e/install-action@v2
     with:
       tool: zola@0.22.1, pagefind@1
   ```

2. Pagefind index build (after `zola build`, before size guard):

   ```yaml
   - name: Build search index
     run: pagefind --site public
     working-directory: website
   ```

3. Size guard limit bumped from 5 MB to 10 MB to accommodate the Pagefind index and WASM.

### `website/README.md` updates

The bump procedure section is updated to cover both `zola` and `pagefind` version pins.
Both are bumped in the same manual PR when a new release warrants it.

## Verification

Extends the Phase 1 smoke checklist:

- `zola check` catches broken internal links (including any unpublished-target cross-links
  not yet rewritten to GitHub URLs) — CI enforcement.
- `pagefind --site public` completes without error — CI enforcement.
- Manual smoke post-deploy:
  - `/docs/end-user/` and `/docs/security/` load with sidebar, breadcrumbs, and alpha banner.
  - Sidebar nested sections (`deployment/`, `plugins/`) collapse and expand via CSS `:has()`;
    active page is highlighted.
  - Hamburger drawer opens/closes at 320 px viewport width.
  - Pagefind search widget returns results; WASM loads on first focus (not on page load).
  - Prev/next links navigate in weight order on leaf pages; absent on section index pages.
  - Edit-on-GitHub links resolve to correct `blob/main` paths on GitHub.
  - GFM alerts render with correct token colors for each type (note/tip/important/warning/caution).
  - Syntax-highlighted code blocks use `--bg-raised` background in both dark and light themes.
  - OG image served at `https://uptrakit.org/og.png`; `og:image` meta tag present in `<head>`.
  - `/install/` "canonical deployment reference" link resolves to `/docs/end-user/deployment/`.
- Lighthouse targets unchanged: accessibility ≥ 95, performance ≥ 95, best-practices ≥ 95,
  SEO ≥ 95. Pagefind WASM is lazy — no impact on initial load score.

## Risks and open items

- **Symlink support in Zola:** Zola follows symlinks for content directories. This is established
  behavior but not explicitly documented. If a future Zola version changes this, the fallback
  is a CI copy step (rejected in design but trivially addable).
- **`zola check` and symlink paths:** Internal link checking traverses symlinked directories.
  Verify `zola check` passes locally before the first merge.
- **OG image authoring:** `og.png` must be created as a raster asset. The spec above defines
  the exact layout; it can be produced with any raster tool (Figma, Inkscape, ImageMagick).
  The favicon SVG is the source for the icon. Current favicon is not finalized but ships as-is.
- **GFM alert migration scope:** All callout patterns in `docs/end-user/` and `docs/security/`
  must be migrated. A grep for `> \*\*` and `> \[!` before merge confirms completeness.
  `zola check` does not catch unmigrated callouts — they render as plain blockquotes, not errors.
- **Unpublished cross-links:** Any link in published pages pointing to `docs/api/`,
  `docs/architecture/`, `docs/development/`, or `docs/hackme/` must be rewritten to a GitHub
  `blob/main` URL. `zola check` catches these as broken internal links if the target section
  is not in `website/content/docs/`.
- **CSS `:has()` sidebar support:** `:has()` has broad support (Chrome 105+, Firefox 121+,
  Safari 15.4+). No fallback needed for the target audience.
- **`pagefind` in `taiki-e/install-action`:** `install-action` supports tools published via
  GitHub releases with standard binary naming. Pagefind publishes such releases, but support
  is not explicitly listed in install-action's tool registry. If install fails, the fallback
  is a direct curl-based download from `https://github.com/CloudCannon/pagefind/releases`
  pinned to the same version. Verify at implementation time before committing the workflow.

## Phase 3 (out of scope here)

Phase 3 will add `docs/api/`, `docs/architecture/`, and `docs/development/` to the published
hub. The symlink + `_index.md` + front matter pattern established in Phase 2 applies directly.
No structural changes to the website are required — only new symlinks, `_index.md` files, and
front matter on the newly published files.
