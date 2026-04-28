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
cross-links between published pages are rewritten to page URLs automatically by Zola. Links targeting unpublished sections (`docs/api/`, `docs/development/`, etc.) are rewritten to
GitHub **directory** URLs (`https://github.com/worried-networking/uptrakit/tree/main/docs/<section>/`)
rather than individual file blob URLs. Directory URLs survive file-level renames within a
section; section-level renames still require a bulk URL update, but those are far rarer than
individual file moves. `zola check` enforces this — broken internal links block CI.

**Cross-link scope:** As of spec date, 42 files in `docs/end-user/` and `docs/security/`
contain links pointing to unpublished sections, totalling 126 individual link instances.
The cross-link rewrite pass is significant implementation work and must be completed before
`zola check` can pass.

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

Pre-merge gate: `find docs/end-user docs/security -name "*.md" | xargs grep -L "^---" | wc -l`
must return `0`. All 60 published markdown files require front matter.

### Section indexes

Zola requires `_index.md` for section index pages. Source files use `README.md` for GitHub
directory browsing. These coexist:

- `_index.md` — Zola section metadata only. Front matter: `title`, `sort_by = "weight"`. No
  body content. Added to **every** published section directory at all nesting levels
  (`end-user/`, `end-user/deployment/`, `end-user/plugins/`, `security/`).
- `README.md` — published as a regular Zola page, title "Overview", `weight = 1`. Appears
  first in the sidebar for its section. GitHub directory browsing continues to work unchanged.

`docs/end-user/plugins/` has no `README.md`. It gets an authored `README.md` as part of Phase 2
work: brief intro paragraph listing the four plugin pages (`apt`, `dnf`, `docker`, `pacman`),
`weight = 1`. This is the only new content file created in Phase 2.

### GFM alerts

`github_alerts = true` is added to `[markdown]` in `website/config.toml`. This feature landed
in Zola v0.21.0 (we use 0.22.1). Zola renders `> [!NOTE]` etc. as
`<blockquote class="markdown-alert-note">`.

All existing callouts in `docs/end-user/` and `docs/security/` are migrated to GFM alert
syntax as part of the front matter authoring pass. GFM alert syntax supports five types only:
`NOTE`, `TIP`, `IMPORTANT`, `WARNING`, `CAUTION`.

Standard mappings:

| Old pattern | New syntax |
| --- | --- |
| `> **Note:**` / `> **Note**:` | `> [!NOTE]` |
| `> **Tip:**` / `> **Tip**:` | `> [!TIP]` |
| `> **Important:**` / `> **Important**:` | `> [!IMPORTANT]` |
| `> **Warning:**` | `> [!WARNING]` |
| `> **Security note:**` | `> [!CAUTION]` |

Multi-word or topic-specific labels (e.g. `> **Sudoers note:**`, `> **Output size limit:**`,
`> **Removing a role:**`, `> **Note for existing hosts:**`, `> **Note on APT batch upgrade:**`)
are informational in nature and map to `> [!NOTE]`. The label text is folded into the first
sentence of the alert body.

Example conversion:

```markdown
<!-- before -->
> **Output size limit:** Uptrakit stores up to 50 MB of output per update.

<!-- after -->
> [!NOTE]
> Output size limit: Uptrakit stores up to 50 MB of output per update.
```

Pre-merge gate: `grep -r "^> \*\*" docs/end-user docs/security | wc -l` must return `0`.
`zola check` does not catch unmigrated callouts — they render as plain blockquotes without error.

### Syntax highlighting

Single dark theme only. Light-mode syntax token colors are deferred — on an alpha docs site the
cost of per-theme token fidelity outweighs the benefit.

```toml
# website/config.toml [markdown] section
highlight_code = true
highlight_theme = "base16-ocean-dark"
```

No additional `<link>` tag or JS changes needed. Token colors are dark-theme values in both
dark and light modes; light-mode users see a dark code block on a light page — acceptable for
alpha.

CSS overrides in `site.css` set the `pre` container to match design tokens regardless of theme:

```css
pre { background: var(--bg-raised); border: 1px solid var(--border-subtle); border-radius: 3px; }
```

Upgrade path to dual-theme (if ever needed): switch to `highlight_theme = "css"` +
`highlight_themes_css` with two entries, add `href`-swap to the theme toggle JS. No template
restructuring required.

### Versioning

No version selector. URL structure uses `/docs/end-user/…` (no version segment). Adding
versioning later can be done with redirects from `/docs/…` to `/docs/v2/…` without breaking
existing links.

### Phase 1 install page update

`/install/` links to the GitHub source for `docs/end-user/deployment/docker.md` as the
canonical deployment reference. Phase 2 changes this to `/docs/end-user/deployment/` (the
deployment section index on the website, covering all reverse proxy and profile options, not
just Docker). The link lives in `website/templates/install.html` — update it there.
`website/content/install/_index.md` is front matter only and has no links. No redirect from
the old GitHub URL.

## Architecture

### Repository layout changes

```text
repo-root/
├── docs/
│   ├── end-user/              ← add YAML front matter + GFM alert migration
│   │   ├── _index.md          ← new: Zola section metadata only (sort_by = "weight")
│   │   ├── README.md          ← add front matter (weight = 1, title = "Overview")
│   │   ├── deployment/
│   │   │   ├── _index.md      ← new: Zola section metadata only (sort_by = "weight")
│   │   │   ├── README.md      ← add front matter (weight = 1, title = "Overview")
│   │   │   └── *.md           ← add front matter
│   │   ├── plugins/
│   │   │   ├── _index.md      ← new: Zola section metadata only (sort_by = "weight")
│   │   │   ├── README.md      ← new: authored intro page (weight = 1)
│   │   │   └── *.md           ← add front matter
│   │   └── *.md               ← add front matter
│   └── security/
│       ├── _index.md          ← new: Zola section metadata only (sort_by = "weight")
│       ├── README.md          ← add front matter (weight = 1, title = "Overview")
│       └── *.md               ← add front matter
└── website/
    ├── config.toml            ← add [markdown] section with github_alerts,
    │                          ←   highlight_code, highlight_themes_css
    ├── content/
    │   └── docs/
    │       ├── _index.md      ← new: docs hub landing (links to sections)
    │       ├── end-user/      ← symlink → ../../../docs/end-user/
    │       └── security/      ← symlink → ../../../docs/security/
    ├── templates/
    │   ├── base.html          ← add "Docs" + "Install" to top bar;
    │   │                      ←   add og:image meta tag; add hl-theme link tag
    │   └── docs.html          ← new: unified docs template
    └── static/
        ├── og.png             ← new: 1200×630 social card
        └── css/
            └── site.css       ← add docs layout, sidebar, GFM alert styles,
                               ←   syntax highlight pre overrides
```

### `config.toml` additions

The existing `config.toml` has no `[markdown]` section. Phase 2 adds:

```toml
[markdown]
highlight_code = true
highlight_theme = "base16-ocean-dark"
github_alerts = true
```

### Routes

| Route | Source | Notes |
| --- | --- | --- |
| `/docs/` | `content/docs/_index.md` | Hub landing; links to sections |
| `/docs/end-user/` | `docs/end-user/_index.md` (via symlink) | Section index; alpha banner |
| `/docs/end-user/deployment/` | `docs/end-user/deployment/_index.md` | Subsection index; alpha banner |
| `/docs/end-user/plugins/` | `docs/end-user/plugins/_index.md` | Subsection index; alpha banner |
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

- Generated via `get_section(path="docs/_index.md")`, then iterating `section.subsections` for
  `end-user` and `security`, and recursing one level into their `subsections` (deployment,
  plugins). Zola resolves `get_section` paths relative to `content/`; symlinked directories
  are accessed as `get_section(path="docs/end-user/_index.md")` etc.
- Nested subsections (`deployment/`, `plugins/`) use `<details>` for collapse/expand. Tera
  conditionally emits the `open` attribute on `<details>` when `current_path` matches a page
  within that subsection — this is server-side rendering, not CSS. CSS cannot set the `open`
  attribute. Tera also emits `data-active` on the active subsection's `<details>` element;
  CSS `:has([data-active])` drives visual highlighting (border, color). No JS required for
  collapse/expand because the open state is baked into the HTML at render time.
- Active leaf page is highlighted via a matching `is-active` class emitted by Tera.
- Pagefind search widget rendered in the sidebar header.

**Prev/next:** In Zola's weight-ascending sort, `page.higher` points to the page with a lower
weight (earlier in the reading order = "Previous"), and `page.lower` points to the page with a
higher weight (later in the reading order = "Next"). Render `page.higher` as the "← Previous"
link and `page.lower` as the "Next →" link. Not shown on section index pages.

**Edit-on-GitHub:** Rendered in the page footer on every page:
`https://github.com/worried-networking/uptrakit/blob/main/{{ page.relative_path }}`.

**Alpha banner:** Warning callout (`--color-warning-bg` / `--color-warning-border`) rendered
below breadcrumbs on section index pages only. The hub landing at `/docs/` is also a section
page but does **not** show the banner. The Tera condition:

```jinja2
{% if section and section.relative_path != "docs/_index.md" %}
  {# alpha banner #}
{% endif %}
```

`section.relative_path` is the path relative to `content/` — for the hub landing it is
`"docs/_index.md"`. Section index pages for published content (e.g. `"docs/end-user/_index.md"`)
show the banner.

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

**Syntax highlighting:**

```css
pre { background: var(--bg-raised); border: 1px solid var(--border-subtle); border-radius: 3px; }
```

Token colors are `base16-ocean-dark` (Syntect inline). The `pre` override ensures the container
matches design tokens in both themes. Token colors remain dark in light mode — acceptable for alpha.

## OG Image

`website/static/og.png` — 1200×630 PNG:

- Background: `#0F172A`
- Favicon SVG (from `website/static/favicon.svg`) at 400×400 px, centered horizontally,
  top edge at 58 px from top. Rendered with the rounded plate (`rx="96"` rect, fill `#1e293b`)
  visible so the icon reads as a distinct element against the background.
- Wordmark `uptrakit` in `SF Mono`/`Roboto Mono` monospace, 48 px, weight 500, color `#e4e4e7`
  (dark-theme `--text-primary`), centered horizontally at the vertical midpoint between the
  icon bottom edge and the card bottom edge (≈ 544 px from top).

Note: Phase 1 spec described the wordmark color as "light-theme `--text-primary`" (`#0f172a`),
which is identical to the background and would render invisibly. Phase 2 supersedes that
description: the correct color on a dark background is dark-theme `--text-primary` (`#e4e4e7`).

`base.html` adds `<meta property="og:image" content="{{ get_url(path='og.png') }}">`.
The Phase 1 spec deferred this asset; Phase 2 ships it.

## Search

Pagefind (`pagefind@1`, pinned by major version) provides client-side search. The Pagefind UI
widget is embedded in the sidebar header via a `<link>` to `/pagefind/pagefind-ui.css` and a
`<script>` to `/pagefind/pagefind-ui.js`, plus a small init call. The widget loads lazily —
WASM is not fetched until the user focuses the search input. Both the JS bundle and the CSS are
emitted by `npx pagefind` into `public/pagefind/` and served as static assets.

## Build and Deploy

### Workflow changes

`zola check` and `zola build` steps are unchanged. Changes:

1. `taiki-e/install-action` continues to install `zola@0.22.1` unchanged. Pagefind is **not**
   in install-action's tool registry. It is invoked via `npx` — Ubuntu runners ship Node 20+,
   so no extra setup step is needed.

2. Pagefind index build step added after `zola build`, before the size guard:

   ```yaml
   - name: Build search index
     run: npx -y pagefind@1 --site public
     working-directory: website
   ```

   `@1` pins the major version. Bump is a one-line PR to this workflow file when a new major
   Pagefind version warrants it.

3. The `push.paths` and `pull_request.paths` filters in `website.yml` are extended to trigger
   on source doc changes:

   ```yaml
   paths:
     - 'website/**'
     - 'docs/end-user/**'
     - 'docs/security/**'
     - '.github/workflows/website.yml'
   ```

   Without this, editing `docs/end-user/deployment/docker.md` would not trigger a site rebuild.
   The `branches: [main]` constraint on `push` is retained unchanged — only the `paths` list
   is extended.

4. Size guard limit bumped from 5 MB to 10 MB to accommodate the Pagefind index and WASM.

### `website/README.md` updates

The bump procedure section is updated to document both the `zola@<version>` pin in
`taiki-e/install-action` and the `pagefind@<major>` pin in the `npx` invocation. Each is
bumped in its own manual PR when a new release warrants it.

## Verification

Extends the Phase 1 smoke checklist.

**Pre-merge gates (local, before opening PR):**

```bash
# All published files have front matter
find docs/end-user docs/security -name "*.md" | xargs grep -L "^---" | wc -l
# → must be 0

# No unmigrated callouts remain
grep -r "^> \*\*" docs/end-user docs/security | wc -l
# → must be 0

# zola check passes (catches broken cross-links and template errors)
cd website && zola check
```

**CI gates:**

- `zola check` — broken internal links fail the build.
- `npx -y pagefind@1 --site public` — index build failure fails the build.

**Manual smoke post-deploy:**

- `/docs/end-user/` and `/docs/security/` load with sidebar, breadcrumbs, and alpha banner.
- Alpha banner absent on `/docs/` hub landing.
- Sidebar nested sections (`deployment/`, `plugins/`) collapse via CSS `:has()`;
  active page is highlighted; section auto-opens when navigating inside it.
- Hamburger drawer opens/closes at 320 px viewport width.
- Pagefind search widget returns results; WASM loads on first focus (not on page load).
- Prev/next links navigate in weight order on leaf pages; absent on section index pages.
- Edit-on-GitHub links resolve to correct `blob/main` paths on GitHub.
- GFM alerts render with correct token colors (note/tip/important/warning/caution).
- Syntax-highlighted code blocks use `--bg-raised` background in both themes.
- OG image served at `https://uptrakit.org/og.png`; `og:image` meta tag present in `<head>`.
- `/install/` "canonical deployment reference" link resolves to `/docs/end-user/deployment/`.
- Lighthouse targets unchanged: accessibility ≥ 95, performance ≥ 95, best-practices ≥ 95,
  SEO ≥ 95. Pagefind WASM is lazy — no impact on initial load score.

## Risks and open items

- **Symlink support in Zola:** Zola follows symlinks for content directories. This is established
  behavior but not explicitly documented. If a future Zola version changes this, the fallback
  is a CI copy step (rejected in design but trivially addable).
- **Cross-symlink relative link rewriting:** `docs/security/` pages link to `docs/end-user/`
  pages via relative paths (e.g. `../end-user/deployment/traefik.md`). Whether Zola's internal
  link resolver traverses symlink boundaries for `../` paths is not documented. If it fails,
  these render as dead anchor tags. Verify with `zola check` locally before first merge. If
  broken, rewrite the specific cross-section links to absolute `/docs/end-user/…` site paths.
- **OG image authoring:** `og.png` must be created as a raster asset. The spec above defines
  the exact layout; it can be produced with any raster tool (Figma, Inkscape, ImageMagick).
  The favicon SVG is the source for the icon. Current favicon is not finalized but ships as-is.
- **GFM alert migration scope:** 27 distinct `> **Label:**` callout patterns exist across
  published files. The migration table maps standard labels; multi-word labels fold into
  `[!NOTE]`. Pre-merge grep confirms completeness. `zola check` does not catch unmigrated
  callouts.
- **Unpublished cross-links scope:** 42 files, 126 link instances point to unpublished sections.
  This is significant rewrite work. `zola check` will fail on these until all are rewritten
  to GitHub `tree/main/docs/<section>/` directory URLs. Directory URLs survive file-level
  renames within a section; section-level renames still require a bulk URL update, but those
  are far rarer. Do not use individual `blob/main/<file>` links.
- **CSS `:has()` sidebar support:** `:has()` has broad support (Chrome 105+, Firefox 121+,
  Safari 15.4+). No fallback needed for the target audience.
- **`get_section()` with symlinks:** Zola's `get_section(path="docs/end-user/_index.md")`
  uses the logical content-relative path through the symlink. Whether Zola registers the
  section key by logical or resolved filesystem path is not documented. Verify locally that
  `get_section(path="docs/end-user/_index.md")` returns a populated section before relying on
  it in the sidebar template. Fallback: build the sidebar by iterating `section.subsections`
  from the root docs section instead.
- **`page.ancestors` with symlinks:** Breadcrumbs rely on `page.ancestors` being populated for
  pages under symlinked directories. If Zola cannot trace ancestry through a symlink boundary,
  `page.ancestors` may be empty. Verify with a locally built site before relying on breadcrumbs.

## Phase 3 (out of scope here)

Phase 3 will add `docs/api/`, `docs/architecture/`, and `docs/development/` to the published
hub. The symlink + `_index.md` + front matter pattern established in Phase 2 applies directly.
No structural changes to the website are required — only new symlinks, `_index.md` files, and
front matter on the newly published files.
