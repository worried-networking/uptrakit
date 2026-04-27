<!-- markdownlint-disable MD013 -->

# Website Phase 1 — Marketing Landing

**Status:** Approved (design)
**Date:** 2026-04-27
**Scope:** Phase 1 of the public Uptrakit website at `https://uptrakit.org`. Marketing landing
plus a thin install route. Phase 2 (docs hub) is intentionally out of scope and will be specified
separately.

## Goal

Stand up a minimal, design-language-aligned marketing site for Uptrakit on GitHub Pages with a
custom domain (`uptrakit.org`) that:

- communicates what Uptrakit is, what it deliberately is not, and the security stance
- gives an evaluator a small, honest first-contact install path
- is structured so phase 2 can layer a docs hub at `/docs/` without throwaway work
- visually matches the product UI design language (dark default + light, tokens, typography,
  flat hover, 120 ms transitions, focus ring)

## Non-Goals

- A docs hub. Phase 2.
- A blog or changelog.
- Marketing illustrations, screenshots of the product UI, or polished diagrams.
- Analytics, tracking, third-party JS.
- Coupling the marketing site to the SvelteKit `frontend/` build pipeline.

## Decisions

### Stack and location

| Concern | Decision |
| --- | --- |
| Generator | Zola (single Rust binary, version pinned, `0.22.1` at spec time) |
| Source location | New top-level `website/` directory in this monorepo |
| Pages source mode | GitHub Actions artifact (`actions/upload-pages-artifact` + `actions/deploy-pages`) |
| CI tool installer | `taiki-e/install-action@v2` with `tool: zola@<version>` |
| Bumps | Dependabot for `package-ecosystem: github-actions` (action versions). Zola version is bumped manually in PR; cadence is slow enough that automation is not justified |
| Custom domain | `uptrakit.org` (committed `website/static/CNAME`) |
| Fallback host | `<user>.github.io/uptrakit` accepted to have absolute-link drift relative to canonical |
| HTTPS | GitHub-provisioned Let's Encrypt; "Enforce HTTPS" enabled after DNS propagates |
| Analytics | None |
| Tailwind / Node | Not used. Plain CSS in `website/static/css/site.css` |

### Why Zola, not Jekyll / plain HTML / SvelteKit reuse

Considered and rejected during brainstorming:

- **Plain HTML/CSS now, migrate later:** migration tax to a real SSG when phase 2 lands is
  60–80 % of phase-1 markup work. Pick the destination once.
- **GitHub-native Jekyll:** built-in build, no Actions, but Liquid templating, locked Jekyll
  3.10 plugin allow-list, weak design control, and Ruby toolchain friction outweigh the
  zero-CI benefit. Escape hatch later is bounded but real.
- **SvelteKit `frontend/` reuse with `adapter-static`:** couples marketing lifecycle to the
  product UI, drags app dependencies into a static landing page, mixes contributor mental
  model. Reject unless route-split discipline is applied; not worth it for static content.
- **mdBook:** docs-shaped, fights the tool for landing.

Zola gives full design control, fast builds, single binary, Rust-aligned tooling, and a
phase-2 docs hub is a `content/docs/*.md` plus a section template — no rewrite.

### Why same repo, `website/` directory

- Monorepo is the existing pattern (`crates/`, `frontend/`, `docs/`).
- Phase 2 needs `docs/` markdown as input; same repo means no submodule sync glue.
- Single PR can update product code and landing copy together.
- Pages source = Actions artifact is the least locked-in mode; switching later to a branch
  or separate repo is a workflow change, not a content migration.

### Caveats accepted

- CI cost: workflows on `website/**` and on Rust paths must use `paths:` filters to avoid
  cross-noise. Cross-cutting required checks (if any are introduced later) need a
  skip-with-success sentinel job so website-only PRs do not stall on Rust gates and vice
  versa.
- `base_url` dual-host: build pins `base_url = "https://uptrakit.org"`. Internal navigation
  uses Zola's `get_url` (relative resolution). Absolute links in Open Graph and canonical
  tags are anchored at `uptrakit.org`. The fallback host (`<user>.github.io/uptrakit`)
  works for browsing but absolute links will point at the canonical domain. Acceptable.
- Directory naming: `website/`, not `web/` or `site/`, to avoid ambiguity with the existing
  `frontend/` (product UI).

## Architecture

```text
repo-root/
├── website/
│   ├── config.toml                # base_url, theme settings, taxonomies (none in phase 1)
│   ├── content/
│   │   ├── _index.md              # landing page (uses landing template)
│   │   └── install/
│   │       └── _index.md          # /install/ page
│   ├── templates/
│   │   ├── base.html              # <html>, <head>, top bar, footer, theme bootstrap
│   │   ├── landing.html           # extends base, renders the landing sections
│   │   ├── install.html           # extends base, renders install page
│   │   └── 404.html               # extends base
│   ├── static/
│   │   ├── CNAME                  # contains: uptrakit.org
│   │   ├── favicon.svg            # copied from frontend/static/favicon.svg
│   │   ├── og.png                 # 1200x630 social card (optional in phase 1; tree entry shown for layout, file may be absent)
│   │   ├── robots.txt             # User-agent: * / Allow: /
│   │   └── css/
│   │       └── site.css           # all styles, no preprocessor
│   └── README.md                  # local-dev + bump notes
└── .github/
    ├── dependabot.yml             # github-actions ecosystem entry
    └── workflows/
        └── website.yml            # build + deploy
```

### Routes

| Route | Source | Purpose |
| --- | --- | --- |
| `/` | `content/_index.md` + `landing.html` | Marketing landing (single page, scrollable) |
| `/install/` | `content/install/_index.md` + `install.html` | Thin alpha install page |
| `/404.html` | `templates/404.html` (Zola convention) | Custom 404, served by GitHub Pages on unmatched paths |
| `/sitemap.xml` | Zola built-in | Generated automatically |
| `/robots.txt` | `static/robots.txt` | Allow-all |

### Top bar (shared across routes)

- Wordmark `uptrakit` lowercase mono on the left, with `favicon.svg` rendered at ~24 px
  immediately to the left of the wordmark.
- Right side: GitHub icon link (external, opens in new tab), theme toggle.
- Theme toggle writes `data-theme="dark"` or `data-theme="light"` on `<html>` and persists
  the choice in `localStorage` under key `uptrakit-theme`. On first load, no override
  → follow `prefers-color-scheme`, defaulting to dark when the preference is unavailable.
- Top bar is non-sticky in phase 1 (sticky shows up only when scroll length warrants it;
  the landing is short).

### Landing page sections (in order)

1. **Hero** — `h1` (24 px / 600, `text-entry-title`-equivalent) tagline:
   *"Track upstream updates across your homelab. You decide when to apply them."*
   Sub-paragraph (1–2 sentences) on what Uptrakit is. Two CTAs: primary "Install"
   linking to `/install/`, secondary "View on GitHub" linking to the repo.
2. **Alpha banner** — warning callout: APIs may change, no formal security audit, use at
   your own risk.
3. **What it does** — four-tile grid:
   - Tracks installed software versions across multiple hosts
   - Plugin-based upstream checks (GitHub Releases, Proxmox VE Helper-Scripts, …)
   - Manual, user-triggered updates only
   - Home Assistant integration via MQTT `update` auto-discovery
4. **Will / Won't** — two-column block.
   - **Will:** track versions, run user-triggered updates, expose Web UI + API, integrate
     with Home Assistant.
   - **Won't:** auto-update, phone home, accept inbound connections on agents.
5. **Security stance** — bullet list:
   - Agents run unprivileged
   - Privileged operations gated by sudo allowlist (`NOPASSWD` for specific commands only)
   - Agents accept no inbound connections
   - All update actions require explicit user confirmation
   - Dual MIT / Apache-2.0 licensing
6. **Topology** — small ASCII or hand-drawn SVG diagram showing
   `Controller ⇄ MQTT ⇄ Agents` and `Controller → plugin checks → upstream sources`,
   plus `Controller ⇄ Home Assistant`. No marketing illustration.
7. **Get involved** — links to repo, `CONTRIBUTING.md`, `SECURITY.md`, license info.
8. **Footer** — © year, dual-license note, repo link, build commit short SHA (Zola
   surfaces the build context via env vars, set in workflow).

### `/install/` page

- Eyebrow: `tracking-eyebrow` uppercase "Alpha install"
- Heading: "Try Uptrakit locally"
- Prerequisite line: Docker Engine 24+ with Compose V2 (matches
  `docs/end-user/deployment/docker.md`).
- Quickstart (verbatim shape from `docker.md` "Quick Start", controller-only / SQLite
  default profile — no MQTT, no scheduler):

  ```bash
  git clone https://github.com/worried-networking/uptrakit.git
  cd uptrakit

  cp .env.example .env
  echo "UPTRAKIT_MASTER_KEY=$(openssl rand -hex 32)" >> .env

  docker compose up -d
  ```

  After start: `https://localhost:8443` (self-signed certificate warning expected).
  First-run registration token in logs:

  ```bash
  docker compose logs controller | grep "registration token"
  ```

- Warning callout: this is the default-profile evaluation stack only — controller +
  embedded scheduler + SQLite. Production deploys need reverse proxy, agent enrollment,
  optional MQTT/scheduler/PostgreSQL profiles, and sudo allowlist setup. Link to
  `docs/end-user/deployment/docker.md` on GitHub as the canonical deployment reference.
- Back-to-home link.
- Authoring rule: install page commands must be cross-checked against the current
  `docker-compose.yml` and `docker.md` "Quick Start" before merge. If the canonical
  doc changes, the install page is updated in the same PR. If `docker-compose.yml` is
  ever not evaluator-ready, the install page must say so explicitly and link to the
  canonical deployment doc only.

### 404 page

- Shared chrome (top bar + footer)
- "Not found." + link back to `/`.

## Visual Design

The site adopts the product UI design language documented at
`docs/development/ui/tokens.md`. Implementation notes:

- **Tokens:** the dark and light token tables are mirrored verbatim into `site.css` as CSS
  custom properties on `[data-theme="dark"]` and `[data-theme="light"]` selectors. Subset
  used by the site (enumerated, no globs):
  `--bg-base`, `--bg-surface`, `--bg-raised`, `--bg-hover`, `--border-subtle`,
  `--border-default`, `--text-muted`, `--text-secondary`, `--text-primary`,
  `--text-inverted`, `--accent`, `--accent-rgb`, `--accent-bright`, `--accent-dark`,
  `--accent-deep`,
  `--color-warning`, `--color-warning-bg`, `--color-warning-border`,
  `--color-info`, `--color-info-bg`, `--color-info-border`,
  `--color-success`, `--color-success-bg`, `--color-success-border`,
  `--color-danger`, `--color-danger-bg`, `--color-danger-border`. Hover variants of
  callout tokens (`--color-danger-bg-hover`, `--color-danger-border-hover`) are not used
  on the site (no danger callouts are interactive in phase 1) and may be omitted.
  Tokens not used are not duplicated.
- **Theme bootstrap:** an inline script in `<head>` reads `localStorage` and
  `prefers-color-scheme` and sets `data-theme` before first paint to avoid theme flash.
- **Fonts:** system stack only. Sans:
  `-apple-system, BlinkMacSystemFont, 'Segoe UI', 'Inter', sans-serif`. Mono:
  `'SF Mono', 'Roboto Mono', monospace`. No web fonts.
- **Type scale:** hero `h1` = 24 px / 600 (same value as the product's public-entry
  shell). Section `h2` = 18 px / 600. Sub `h3` = 13 px / 700. Body `text-sm` (14 px).
  Eyebrows use `tracking-eyebrow` (0.24 em) uppercase. The "do not replicate the 24 px
  size in authenticated routes" rule from `tokens.md` is an intra-app constraint and
  does not apply to this marketing site.
- **Border radius:** page panels 4 px, cards 3 px, badges 2 px. No shorthand classes.
- **Transitions:** `background, border-color, color` only, `120 ms`. No transforms on
  hover. Flat at rest and on hover.
- **Focus ring:** `outline: none; box-shadow: 0 0 0 3px rgba(var(--accent-rgb), .25)`,
  on `:focus-visible` only.
- **Callouts:** alpha banner uses warning treatment
  (`--color-warning-bg` / `--color-warning-border`); install-page production-deploy
  warning uses the same treatment.
- **Layout:** single column, max content width ≈ 880 px, generous gutters, no sidebar.
- **No Tailwind, no Node.** Tokens and utilities are hand-written in `site.css`.

## Build and Deploy

### Workflow `.github/workflows/website.yml`

```yaml
name: website
on:
  push:
    branches: [main]
    paths:
      - 'website/**'
      - '.github/workflows/website.yml'
  pull_request:
    paths:
      - 'website/**'
      - '.github/workflows/website.yml'
  workflow_dispatch:
permissions:
  contents: read
  pages: write
  id-token: write
concurrency:
  group: pages
  cancel-in-progress: false
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: taiki-e/install-action@v2
        with:
          tool: zola@0.22.1
      - name: Validate site
        run: zola check
        working-directory: website
      - name: Build site
        # zola is invoked with working-directory: website so the SSG sees the project
        # root correctly; --output-dir ../public emits the artifact at repo root,
        # which the next two steps reference without any working-directory of their own.
        run: zola build --output-dir ../public
        working-directory: website
      - name: Guard artifact size
        run: |
          size=$(du -sb public | cut -f1)
          limit=$((5 * 1024 * 1024))
          if [ "$size" -gt "$limit" ]; then
            echo "Artifact size $size exceeds 5MB limit"; exit 1
          fi
      - uses: actions/upload-pages-artifact@v3
        with:
          path: public
  deploy:
    needs: build
    if: github.ref == 'refs/heads/main'
    runs-on: ubuntu-latest
    environment:
      name: github-pages
      url: ${{ steps.deployment.outputs.page_url }}
    steps:
      - id: deployment
        uses: actions/deploy-pages@v4
```

Notes:

- `zola check` runs before build; broken internal links fail the workflow.
- The 5 MB artifact guard is a sanity bound for an alpha marketing site, not a real
  optimization target.
- PRs build but do not deploy; deploy is gated on `refs/heads/main`.
- No required-check entry should be added for the `website` workflow until / unless the
  repo introduces them — paths-filtering keeps Rust-only PRs from triggering this
  workflow at all, which is the simpler answer than skip-with-success sentinels.

### Dependabot `.github/dependabot.yml`

```yaml
version: 2
updates:
  - package-ecosystem: github-actions
    directory: /
    schedule:
      interval: weekly
```

This bumps `taiki-e/install-action`, `actions/checkout`, `actions/upload-pages-artifact`,
`actions/deploy-pages`, and any other `uses:` lines. It does not bump the
`tool: zola@<version>` string — that is a manual PR when a new Zola release warrants it.
The bump procedure: edit the `tool:` line in `.github/workflows/website.yml`, run
`zola build` locally to confirm no regressions, open a PR. `website/README.md` documents
this procedure and links to the Zola releases page
(`https://github.com/getzola/zola/releases`).

If `.github/dependabot.yml` already exists at implementation time (other ecosystems —
cargo, npm, etc.), the new entry is **merged** into the existing `updates:` list rather
than overwriting the file.

### One-time GitHub UI configuration

Documented in `website/README.md`:

1. Settings → Pages → Build and deployment → Source = "GitHub Actions".
2. Settings → Pages → Custom domain = `uptrakit.org` after DNS records are live.
3. Wait for Let's Encrypt provisioning, then enable "Enforce HTTPS".
4. DNS records on `uptrakit.org`:
   - `A` apex → `185.199.108.153`, `185.199.109.153`, `185.199.110.153`,
     `185.199.111.153`
   - `AAAA` apex → `2606:50c0:8000::153`, `2606:50c0:8001::153`,
     `2606:50c0:8002::153`, `2606:50c0:8003::153`
   - `CNAME` `www` → `<user>.github.io`
   (IPs reflect GitHub's published Pages anycast set; verify against
   `https://docs.github.com/en/pages/configuring-a-custom-domain-for-your-github-pages-site`
   at deploy time before committing.)

## Verification

- **`zola check`** in CI catches broken internal links and template errors.
- **Manual smoke** post-deploy:
  - `https://uptrakit.org/` and `https://uptrakit.org/install/` load over HTTPS.
  - Theme toggle flips dark ↔ light without flash on subsequent loads; persists across
    reloads.
  - Tab navigation shows the focus ring on interactive elements; mouse click does not.
  - Mobile viewport at 320 px wide does not horizontal-scroll.
  - 404 page renders with chrome (e.g. visit `/does-not-exist`).
- **Lighthouse** target on `/`: accessibility ≥ 95, performance ≥ 95, best-practices ≥ 95,
  SEO ≥ 95. With no JS bundles and system fonts, this is easily achievable.
- **Contrast** is inherited from the product token tables (already validated WCAG-AA per
  `docs/development/ui/README.md`); not re-verified here.
- **CNAME persistence**: the CNAME file is committed in `website/static/`. Confirm post-
  deploy that the GitHub UI does not silently rewrite it — Zola copies `static/`
  verbatim, so as long as we deploy via Actions and the file is in the artifact, this
  holds.

## Risks and open items

- **Absolute-link drift** under the fallback host. Accepted; canonical is `uptrakit.org`.
- **`docker compose up` honesty** — the install page must be checked against repo state
  when authored. If the compose file is not actually a working evaluator quickstart
  today, the page must explicitly say so and direct readers to the canonical doc only.
- **GitHub Pages anycast IP set** can change. Verify the IP list at deploy time against
  GitHub's published documentation before committing DNS.
- **Open Graph image** (`static/og.png`) is optional in phase 1. Default decision:
  **skip** in phase 1; Open Graph meta tags are emitted with `og:title` and
  `og:description` only, no `og:image`. Adding the image is a phase-2 task; when added,
  spec content is: 1200×630 PNG, dark slate (`#0F172A`) background matching the favicon
  plate, favicon SVG centered at ~512 px wide, wordmark "uptrakit" lowercase mono
  beneath in the `--text-primary` light-theme value. If a phase-1 contributor wants to
  ship the image early, that spec is the contract.

## Phase 2 (out of scope here)

Phase 2 will add a docs hub at `/docs/` rendering the existing `docs/end-user/`
markdown via Zola sections, plus navigation, search, and a versioned-content story.
That spec will be authored separately when phase 1 has shipped and the design language
ports cleanly to long-form docs.
