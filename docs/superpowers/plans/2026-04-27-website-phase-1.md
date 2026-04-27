<!-- markdownlint-disable MD013 -->

# Website Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a phase-1 marketing site for Uptrakit at `https://uptrakit.org` on GitHub Pages — a landing page plus a thin install page — built by Zola and deployed by GitHub Actions, visually aligned with the product UI design tokens.

**Architecture:** New top-level `website/` directory containing a Zola project with hand-written CSS (no Node, no Tailwind). Pages source mode is "GitHub Actions"; a single workflow `.github/workflows/website.yml` builds the artifact and deploys via `actions/deploy-pages`. CNAME committed in `website/static/`. `.github/dependabot.yml` already covers `github-actions` at `/`, so no Dependabot edits are needed.

**Tech Stack:**

- [Zola](https://www.getzola.org/) v0.22.1 (single-binary Rust SSG, [Tera](https://keats.github.io/tera/docs/) templates)
- GitHub Pages (Actions artifact source)
- GitHub Actions (`taiki-e/install-action@v2`, `actions/upload-pages-artifact@v3`, `actions/deploy-pages@v4`)
- Plain CSS, system fonts only — no preprocessor, no Node

**Reference:** [Spec — `docs/superpowers/specs/2026-04-27-website-phase-1-design.md`](../specs/2026-04-27-website-phase-1-design.md)

---

## File Structure

Files to be created (all under repo root unless noted):

```text
website/
├── config.toml                 # Zola config: base_url, title, description, language
├── content/
│   ├── _index.md               # Landing page (uses landing.html)
│   └── install/
│       └── _index.md           # /install/ page (uses install.html)
├── templates/
│   ├── base.html               # Shared chrome: <html>, <head>, top bar, footer
│   ├── landing.html            # Extends base.html — landing sections
│   ├── install.html            # Extends base.html — install page
│   ├── 404.html                # Extends base.html — 404 page
│   └── macros/
│       └── ui.html             # Tera macros for callouts, icons
├── static/
│   ├── CNAME                   # Contents: "uptrakit.org" (no trailing newline-required, but harmless)
│   ├── favicon.svg             # Copied from frontend/static/favicon.svg
│   ├── robots.txt              # Allow-all + sitemap reference
│   └── css/
│       └── site.css            # All site styles (tokens, typography, layout, components)
└── README.md                   # Local-dev instructions, Zola version-bump procedure

.github/workflows/website.yml   # Build + deploy workflow (new file)
```

No edits to the existing `.github/dependabot.yml` are required — its `github-actions` entry at `/` already discovers the new workflow.

---

## Conventions for this plan

- **Working directory:** all `zola` commands run from `website/` unless stated. All `git` commands run from repo root.
- **Local Zola install:** `cargo install zola --version 0.22.1` (one-time; the engineer's local box only — CI installs via `taiki-e/install-action`). On macOS, `brew install zola` works but the version may lag.
- **TDD-shaped verification:** Zola itself acts as the test runner — `zola check` validates internal links and template syntax, `zola build` is the end-to-end gate. After each task, the engineer runs `zola check` and `zola build`; both must succeed before commit.
- **Commit per task** at the end of every task.
- **No code comments unless WHY is non-obvious** (per repo CLAUDE.md rules).
- **Markdownlint** runs in pre-commit on `*.md` files. Keep lines ≤ 120 chars where practical; `<!-- markdownlint-disable MD013 -->` only when truly needed (it's used in the design-language docs but should be avoided in generated content).

---

## Task 1: Scaffold the Zola project

**Files:**

- Create: `website/config.toml`
- Create: `website/content/_index.md` (placeholder, full content in Task 7)
- Create: `website/.gitignore`

- [ ] **Step 1: Create `website/config.toml`**

```toml
# Zola configuration. Reference: https://www.getzola.org/documentation/getting-started/configuration/

base_url = "https://uptrakit.org"
title = "Uptrakit"
description = "Self-hosted update tracking toolkit for Linux homelabs and small fleets. Manual updates only — you decide when."
default_language = "en"
output_dir = "public"
compile_sass = false
build_search_index = false
generate_feeds = false
minify_html = true

taxonomies = []

[markdown]
highlight_code = true
highlight_theme = "base16-ocean-dark"

[extra]
github_repo_url = "https://github.com/worried-networking/uptrakit"
```

- [ ] **Step 2: Create a minimal placeholder landing page so Zola can build**

`website/content/_index.md`:

```markdown
+++
title = "Uptrakit"
template = "landing.html"
+++

Placeholder. Real content arrives in a later task.
```

- [ ] **Step 3: Create `website/.gitignore`**

```gitignore
public/
.zola-cache/
```

- [ ] **Step 4: Verify directory layout**

Run from repo root:

```bash
ls website/
```

Expected output (in any order):

```text
.gitignore
config.toml
content
```

- [ ] **Step 5: Stage and commit**

```bash
git add website/.gitignore website/config.toml website/content/_index.md
git commit -m "feat(website): scaffold Zola project skeleton"
```

---

## Task 2: Static assets — CNAME, favicon, robots.txt

**Files:**

- Create: `website/static/CNAME`
- Create: `website/static/favicon.svg` (copied from `frontend/static/favicon.svg`)
- Create: `website/static/robots.txt`

- [ ] **Step 1: Create `website/static/CNAME`**

Single line, no trailing whitespace beyond the file's natural newline:

```text
uptrakit.org
```

- [ ] **Step 2: Copy the favicon**

```bash
mkdir -p website/static
cp frontend/static/favicon.svg website/static/favicon.svg
```

Verify:

```bash
diff frontend/static/favicon.svg website/static/favicon.svg
```

Expected: no output (files identical).

- [ ] **Step 3: Create `website/static/robots.txt`**

```text
User-agent: *
Allow: /

Sitemap: https://uptrakit.org/sitemap.xml
```

- [ ] **Step 4: Stage and commit**

```bash
git add website/static/CNAME website/static/favicon.svg website/static/robots.txt
git commit -m "feat(website): add CNAME, favicon, robots.txt static assets"
```

---

## Task 3: Base template + theme bootstrap

**Files:**

- Create: `website/templates/base.html`
- Create: `website/templates/landing.html` (minimal extends; full content Task 7)

This task gets a build passing with a bare-bones page so subsequent visual work has something to render.

- [ ] **Step 1: Create `website/templates/base.html`**

```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{% block title %}{{ config.title }}{% endblock %}</title>
  <meta name="description" content="{% block description %}{{ config.description }}{% endblock %}">
  <link rel="icon" type="image/svg+xml" href="{{ get_url(path='favicon.svg') }}">
  <link rel="stylesheet" href="{{ get_url(path='css/site.css') }}">

  <!-- Open Graph (og:image deferred to phase 2 per spec) -->
  <meta property="og:type" content="website">
  <meta property="og:title" content="{% block og_title %}{{ config.title }}{% endblock %}">
  <meta property="og:description" content="{% block og_description %}{{ config.description }}{% endblock %}">
  <meta property="og:url" content="{% block og_url %}{{ config.base_url }}{% endblock %}">

  <!-- Theme bootstrap: set data-theme before first paint to avoid flash. -->
  <script>
    (function () {
      try {
        var stored = localStorage.getItem('uptrakit-theme');
        if (stored === 'dark' || stored === 'light') {
          document.documentElement.setAttribute('data-theme', stored);
          return;
        }
        var prefers = window.matchMedia && window.matchMedia('(prefers-color-scheme: light)').matches;
        document.documentElement.setAttribute('data-theme', prefers ? 'light' : 'dark');
      } catch (e) {
        document.documentElement.setAttribute('data-theme', 'dark');
      }
    })();
  </script>
</head>
<body>
  <header class="topbar">
    <a class="topbar__brand" href="{{ get_url(path='/') }}">
      <img class="topbar__logo" src="{{ get_url(path='favicon.svg') }}" alt="" width="24" height="24">
      <span class="topbar__wordmark">uptrakit</span>
    </a>
    <nav class="topbar__nav">
      <a class="topbar__link" href="{{ config.extra.github_repo_url }}" rel="noopener" target="_blank">GitHub</a>
      <button class="topbar__theme-toggle" type="button" aria-label="Toggle theme" data-theme-toggle>
        <span aria-hidden="true">◐</span>
      </button>
    </nav>
  </header>

  <main class="content">
    {% block content %}{% endblock %}
  </main>

  <footer class="footer">
    <p>
      © {{ now() | date(format="%Y") }} Uptrakit contributors —
      Dual-licensed
      <a href="{{ config.extra.github_repo_url }}/blob/main/LICENSE-MIT">MIT</a>
      /
      <a href="{{ config.extra.github_repo_url }}/blob/main/LICENSE-APACHE">Apache-2.0</a>
      —
      <a href="{{ config.extra.github_repo_url }}">Source</a>
    </p>
  </footer>

  <script>
    (function () {
      var btn = document.querySelector('[data-theme-toggle]');
      if (!btn) return;
      btn.addEventListener('click', function () {
        var current = document.documentElement.getAttribute('data-theme') === 'light' ? 'light' : 'dark';
        var next = current === 'light' ? 'dark' : 'light';
        document.documentElement.setAttribute('data-theme', next);
        try { localStorage.setItem('uptrakit-theme', next); } catch (e) {}
      });
    })();
  </script>
</body>
</html>
```

- [ ] **Step 2: Create `website/templates/landing.html`**

```html
{% extends "base.html" %}

{% block content %}
  <p>Phase-1 placeholder. Task 7 fills this in.</p>
{% endblock %}
```

- [ ] **Step 3: Create empty `website/static/css/site.css`**

```css
/* Populated in Task 4. */
```

- [ ] **Step 4: Run a build to verify the skeleton compiles**

```bash
cd website
zola check
zola build
```

Expected: both commands exit 0. `zola build` writes to `website/public/`.

- [ ] **Step 5: Smoke-load the site**

```bash
zola serve --port 1111
```

Open `http://127.0.0.1:1111/` — the placeholder page renders, top bar shows the wordmark, theme toggle is clickable. No console errors. Stop the server with Ctrl-C.

- [ ] **Step 6: Stage and commit**

```bash
cd ..
git add website/templates/base.html website/templates/landing.html website/static/css/site.css
git commit -m "feat(website): add base template with theme bootstrap"
```

---

## Task 4: Design tokens + typography + layout primitives in CSS

**Files:**

- Modify: `website/static/css/site.css` (full content below; replaces the placeholder)

This is one large file written end-to-end so its parts cohere. Read it as one unit; do not split commits.

- [ ] **Step 1: Replace `website/static/css/site.css` with the full stylesheet**

```css
/*
 * site.css — Uptrakit website phase 1.
 * Mirrors the product design tokens from docs/development/ui/tokens.md.
 * No Tailwind, no preprocessor.
 */

/* ---------- Tokens (dark default) ---------- */

:root,
[data-theme="dark"] {
  --bg-base: #09090b;
  --bg-surface: #111113;
  --bg-raised: #18181b;
  --bg-hover: #1e1e22;
  --border-subtle: #1c1c1f;
  --border-default: #27272a;
  --text-muted: #52525b;
  --text-secondary: #a1a1aa;
  --text-primary: #e4e4e7;
  --text-inverted: #fafafa;
  --accent: #06b6d4;
  --accent-rgb: 6 182 212;
  --accent-bright: #22d3ee;
  --accent-dark: #0891b2;
  --accent-deep: #0e7490;
  --color-success: #4ade80;
  --color-success-bg: rgba(74, 222, 128, 0.10);
  --color-success-border: rgba(74, 222, 128, 0.25);
  --color-warning: #fbbf24;
  --color-warning-bg: rgba(251, 191, 36, 0.12);
  --color-warning-border: rgba(251, 191, 36, 0.30);
  --color-danger: #fdba74;
  --color-danger-bg: rgba(234, 88, 12, 0.15);
  --color-danger-border: rgba(234, 88, 12, 0.35);
  --color-info: #67e8f9;
  --color-info-bg: rgba(6, 182, 212, 0.10);
  --color-info-border: rgba(6, 182, 212, 0.22);
}

[data-theme="light"] {
  --bg-base: #f8fafc;
  --bg-surface: #ffffff;
  --bg-raised: #f1f5f9;
  --bg-hover: #eef1f5;
  --border-subtle: #e2e8f0;
  --border-default: #cbd5e1;
  --text-muted: #94a3b8;
  --text-secondary: #64748b;
  --text-primary: #0f172a;
  --text-inverted: #ffffff;
  --accent: #2563eb;
  --accent-rgb: 37 99 235;
  --accent-bright: #3b82f6;
  --accent-dark: #1d4ed8;
  --accent-deep: #1e40af;
  --color-success: #16a34a;
  --color-success-bg: rgba(22, 163, 74, 0.08);
  --color-success-border: rgba(22, 163, 74, 0.30);
  --color-warning: #d97706;
  --color-warning-bg: rgba(217, 119, 6, 0.08);
  --color-warning-border: rgba(217, 119, 6, 0.28);
  --color-danger: #dc2626;
  --color-danger-bg: rgba(220, 38, 38, 0.07);
  --color-danger-border: rgba(220, 38, 38, 0.30);
  --color-info: #0891b2;
  --color-info-bg: rgba(8, 145, 178, 0.08);
  --color-info-border: rgba(8, 145, 178, 0.22);
}

/* ---------- Reset + base ---------- */

*,
*::before,
*::after {
  box-sizing: border-box;
}

html {
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', 'Inter', sans-serif;
  font-size: 14px;
  line-height: 1.55;
  color: var(--text-primary);
  background: var(--bg-base);
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
  scroll-behavior: smooth;
}

body {
  margin: 0;
  min-height: 100vh;
  display: flex;
  flex-direction: column;
}

main.content {
  flex: 1 0 auto;
  width: 100%;
  max-width: 880px;
  padding: 32px 20px 64px;
  margin: 0 auto;
}

a {
  color: var(--accent-bright);
  text-decoration: none;
  border-bottom: 1px solid transparent;
  transition: background 0.12s, border-color 0.12s, color 0.12s;
}

a:hover {
  border-bottom-color: var(--accent);
}

a:focus-visible {
  outline: none;
  box-shadow: 0 0 0 3px rgba(var(--accent-rgb), 0.25);
  border-radius: 2px;
}

code {
  font-family: 'SF Mono', 'Roboto Mono', monospace;
  font-size: 0.92em;
  background: var(--bg-raised);
  padding: 1px 6px;
  border-radius: 2px;
}

pre {
  font-family: 'SF Mono', 'Roboto Mono', monospace;
  background: var(--bg-surface);
  border: 1px solid var(--border-default);
  border-radius: 3px;
  padding: 14px 16px;
  overflow-x: auto;
  font-size: 13px;
  line-height: 1.5;
}

pre code {
  background: transparent;
  padding: 0;
  border-radius: 0;
}

/* ---------- Typography ---------- */

h1, h2, h3 {
  margin: 0 0 12px;
  color: var(--text-primary);
}

h1 {
  font-size: 24px;
  font-weight: 600;
  line-height: 1.25;
}

h2 {
  font-size: 18px;
  font-weight: 600;
  line-height: 1.35;
}

h3 {
  font-size: 13px;
  font-weight: 700;
  line-height: 1.4;
}

p {
  margin: 0 0 14px;
  color: var(--text-secondary);
}

p strong {
  color: var(--text-primary);
}

.eyebrow {
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.24em;
  text-transform: uppercase;
  color: var(--text-muted);
  margin: 0 0 8px;
}

ul, ol {
  margin: 0 0 14px;
  padding-left: 22px;
  color: var(--text-secondary);
}

li + li {
  margin-top: 4px;
}

hr {
  border: none;
  border-top: 1px solid var(--border-subtle);
  margin: 32px 0;
}

/* ---------- Top bar ---------- */

.topbar {
  position: sticky;
  top: 0;
  z-index: 10;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 20px;
  background: var(--bg-base);
  border-bottom: 1px solid var(--border-subtle);
}

.topbar__brand {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  color: var(--text-primary);
  border-bottom: none;
}

.topbar__brand:hover {
  border-bottom: none;
  color: var(--text-primary);
}

.topbar__logo {
  display: block;
  width: 24px;
  height: 24px;
}

.topbar__wordmark {
  font-family: 'SF Mono', 'Roboto Mono', monospace;
  font-size: 14px;
  font-weight: 600;
  letter-spacing: 0.01em;
  color: var(--text-primary);
}

.topbar__nav {
  display: inline-flex;
  align-items: center;
  gap: 14px;
}

.topbar__link {
  font-size: 13px;
  color: var(--text-secondary);
  border-bottom: none;
}

.topbar__link:hover {
  color: var(--text-primary);
  border-bottom: none;
}

.topbar__theme-toggle {
  font-family: inherit;
  font-size: 14px;
  background: var(--bg-surface);
  color: var(--text-secondary);
  border: 1px solid var(--border-default);
  border-radius: 3px;
  padding: 4px 10px;
  cursor: pointer;
  transition: background 0.12s, border-color 0.12s, color 0.12s;
}

.topbar__theme-toggle:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.topbar__theme-toggle:focus-visible {
  outline: none;
  box-shadow: 0 0 0 3px rgba(var(--accent-rgb), 0.25);
}

/* ---------- Footer ---------- */

.footer {
  flex: 0 0 auto;
  padding: 24px 20px 32px;
  border-top: 1px solid var(--border-subtle);
  font-size: 12px;
  color: var(--text-muted);
  text-align: center;
}

.footer p {
  margin: 0;
  color: var(--text-muted);
}

/* ---------- Layout primitives ---------- */

.section {
  margin: 40px 0;
}

.section__heading {
  margin-bottom: 16px;
}

.grid {
  display: grid;
  gap: 14px;
}

.grid--2 { grid-template-columns: repeat(2, minmax(0, 1fr)); }
.grid--4 { grid-template-columns: repeat(4, minmax(0, 1fr)); }

@media (max-width: 640px) {
  .grid--2,
  .grid--4 { grid-template-columns: 1fr; }
}

.card {
  background: var(--bg-surface);
  border: 1px solid var(--border-default);
  border-radius: 3px;
  padding: 16px 18px;
}

.card__title {
  margin: 0 0 6px;
  font-size: 13px;
  font-weight: 700;
  color: var(--text-primary);
}

.card__body {
  margin: 0;
  font-size: 13px;
  color: var(--text-secondary);
}

/* ---------- Hero ---------- */

.hero {
  padding: 24px 0 12px;
}

.hero__title {
  font-size: 24px;
  font-weight: 600;
  margin: 0 0 12px;
}

.hero__sub {
  font-size: 15px;
  margin: 0 0 24px;
  color: var(--text-secondary);
}

.hero__ctas {
  display: inline-flex;
  flex-wrap: wrap;
  gap: 10px;
}

/* ---------- Buttons ---------- */

.btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-family: inherit;
  font-size: 11px;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  padding: 8px 14px;
  border-radius: 3px;
  border: 1px solid transparent;
  cursor: pointer;
  transition: background 0.12s, border-color 0.12s, color 0.12s;
  border-bottom: 1px solid transparent;
}

.btn:focus-visible {
  outline: none;
  box-shadow: 0 0 0 3px rgba(var(--accent-rgb), 0.25);
}

.btn--primary {
  background: var(--accent);
  color: var(--text-inverted);
  border-color: var(--accent);
}

.btn--primary:hover {
  background: var(--accent-dark);
  border-color: var(--accent-dark);
  color: var(--text-inverted);
}

.btn--secondary {
  background: var(--bg-surface);
  color: var(--text-primary);
  border-color: var(--border-default);
}

.btn--secondary:hover {
  background: var(--bg-hover);
  border-color: var(--border-default);
  color: var(--text-primary);
}

/* ---------- Callouts ---------- */

.callout {
  padding: 12px 14px;
  border-radius: 3px;
  border: 1px solid var(--border-default);
  background: var(--bg-surface);
  margin: 16px 0;
  font-size: 13px;
  color: var(--text-secondary);
}

.callout--warning {
  background: var(--color-warning-bg);
  border-color: var(--color-warning-border);
  color: var(--text-primary);
}

.callout--info {
  background: var(--color-info-bg);
  border-color: var(--color-info-border);
  color: var(--text-primary);
}

.callout--success {
  background: var(--color-success-bg);
  border-color: var(--color-success-border);
  color: var(--text-primary);
}

.callout--danger {
  background: var(--color-danger-bg);
  border-color: var(--color-danger-border);
  color: var(--text-primary);
}

.callout__title {
  display: block;
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  margin-bottom: 4px;
  color: var(--text-primary);
}

/* ---------- Will / Won't block ---------- */

.willwont {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 14px;
}

@media (max-width: 640px) {
  .willwont { grid-template-columns: 1fr; }
}

.willwont__col {
  background: var(--bg-surface);
  border: 1px solid var(--border-default);
  border-radius: 3px;
  padding: 16px 18px;
}

.willwont__col h3 {
  margin: 0 0 10px;
  letter-spacing: 0.04em;
  text-transform: uppercase;
}

.willwont__col--will h3 { color: var(--color-success); }
.willwont__col--wont h3 { color: var(--color-danger); }

.willwont__col ul {
  margin: 0;
  padding-left: 18px;
}

/* ---------- Topology block ---------- */

.topology {
  background: var(--bg-surface);
  border: 1px solid var(--border-default);
  border-radius: 3px;
  padding: 16px 18px;
  font-family: 'SF Mono', 'Roboto Mono', monospace;
  font-size: 12px;
  line-height: 1.5;
  color: var(--text-secondary);
  overflow-x: auto;
  white-space: pre;
}

/* ---------- Install page extras ---------- */

.install-step {
  display: flex;
  gap: 12px;
  align-items: flex-start;
  margin-bottom: 18px;
}

.install-step__num {
  flex: 0 0 auto;
  width: 22px;
  height: 22px;
  border-radius: 2px;
  background: var(--bg-raised);
  border: 1px solid var(--border-default);
  font-family: 'SF Mono', 'Roboto Mono', monospace;
  font-size: 12px;
  font-weight: 700;
  color: var(--accent-bright);
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

.install-step__body {
  flex: 1 1 auto;
  min-width: 0;
}

.install-step__body p {
  margin: 0 0 6px;
}
```

- [ ] **Step 2: Verify the build still passes**

```bash
cd website
zola check
zola build
```

Both must exit 0.

- [ ] **Step 3: Smoke the styles**

```bash
zola serve --port 1111
```

Open `http://127.0.0.1:1111/`. Confirm:

- Body background is dark slate (token `--bg-base`)
- Top bar is sticky with thin border underneath
- Wordmark renders in monospace
- Theme toggle flips the page light, persists across reloads
- `prefers-color-scheme: light` on a fresh profile starts in light mode

Stop with Ctrl-C.

- [ ] **Step 4: Stage and commit**

```bash
cd ..
git add website/static/css/site.css
git commit -m "feat(website): add design-token CSS for landing chrome"
```

---

## Task 5: Tera macros — callout

**Files:**

- Create: `website/templates/macros/ui.html`

A small macro keeps callout markup consistent across pages.

- [ ] **Step 1: Create `website/templates/macros/ui.html`**

```html
{% macro callout(kind, title, body) %}
  <aside class="callout callout--{{ kind }}" role="note">
    {% if title %}<span class="callout__title">{{ title }}</span>{% endif %}
    <div>{{ body | safe }}</div>
  </aside>
{% endmacro callout %}
```

- [ ] **Step 2: Verify it parses**

```bash
cd website
zola check
zola build
```

(The macro is unused at this point; both commands still pass because Tera does not parse macros until imported.)

- [ ] **Step 3: Stage and commit**

```bash
cd ..
git add website/templates/macros/ui.html
git commit -m "feat(website): add callout Tera macro"
```

---

## Task 6: Landing page content + template

**Files:**

- Modify: `website/templates/landing.html` (full content)
- Modify: `website/content/_index.md` (front-matter only; sections live in the template)

The landing page is mostly static markup, so the template carries the structure and the content file only configures the page.

- [ ] **Step 1: Replace `website/templates/landing.html` with the full landing page**

```html
{% extends "base.html" %}
{% import "macros/ui.html" as ui %}

{% block title %}Uptrakit — track upstream updates across your homelab{% endblock %}
{% block description %}Self-hosted update tracking toolkit for Linux homelabs and small fleets. Tracks installed software versions, checks upstream releases, and runs manual updates only.{% endblock %}
{% block og_title %}Uptrakit — track upstream updates across your homelab{% endblock %}
{% block og_description %}Self-hosted update tracker for homelabs. Manual updates only — you decide when.{% endblock %}

{% block content %}

<section class="hero">
  <h1 class="hero__title">Track upstream updates across your homelab. You decide when to apply them.</h1>
  <p class="hero__sub">
    Uptrakit is a self-hosted update tracking toolkit for Linux homelabs and small fleets.
    It checks upstream sources on a schedule, surfaces what's new, and runs only the
    updates you confirm.
  </p>
  <div class="hero__ctas">
    <a class="btn btn--primary" href="{{ get_url(path='install/') }}">Install</a>
    <a class="btn btn--secondary" href="{{ config.extra.github_repo_url }}" rel="noopener" target="_blank">View on GitHub</a>
  </div>
</section>

{{ ui::callout(
  kind="warning",
  title="Alpha",
  body="<p>APIs may change. No formal third-party security audit yet. Use at your own risk.</p>"
) }}

<section class="section">
  <h2 class="section__heading">What it does</h2>
  <div class="grid grid--4">
    <article class="card">
      <h3 class="card__title">Track versions</h3>
      <p class="card__body">Records the installed version of every tracked item across multiple hosts.</p>
    </article>
    <article class="card">
      <h3 class="card__title">Plugin upstream checks</h3>
      <p class="card__body">Pluggable sources — GitHub Releases, Proxmox VE Helper-Scripts, package managers.</p>
    </article>
    <article class="card">
      <h3 class="card__title">Manual updates only</h3>
      <p class="card__body">Every update action requires explicit user confirmation. No silent automatic upgrades.</p>
    </article>
    <article class="card">
      <h3 class="card__title">Home Assistant ready</h3>
      <p class="card__body">Each tracked item appears as an <code>update</code> entity via MQTT auto-discovery.</p>
    </article>
  </div>
</section>

<section class="section">
  <h2 class="section__heading">Will and won't</h2>
  <div class="willwont">
    <div class="willwont__col willwont__col--will">
      <h3>Will</h3>
      <ul>
        <li>Track versions across hosts</li>
        <li>Run user-triggered updates</li>
        <li>Expose a Web UI and HTTP API</li>
        <li>Integrate with Home Assistant over MQTT</li>
      </ul>
    </div>
    <div class="willwont__col willwont__col--wont">
      <h3>Won't</h3>
      <ul>
        <li>Auto-update without confirmation</li>
        <li>Phone home or report telemetry</li>
        <li>Accept inbound connections on agents</li>
        <li>Run privileged operations beyond the sudo allowlist</li>
      </ul>
    </div>
  </div>
</section>

<section class="section">
  <h2 class="section__heading">Security stance</h2>
  <ul>
    <li>Agents run unprivileged (e.g. <code>uptrakit</code>)</li>
    <li>Privileged operations are constrained via a sudo allowlist (<code>NOPASSWD</code> for specific commands only)</li>
    <li>Agents accept no inbound connections</li>
    <li>All update actions require explicit user confirmation</li>
    <li>Dual-licensed under MIT and Apache-2.0</li>
  </ul>
</section>

<section class="section">
  <h2 class="section__heading">Topology</h2>
  <pre class="topology">
   upstream sources
   (GitHub Releases, PHS, package managers, …)
            │
            ▼
   ┌──────────────────┐         ┌──────────────┐
   │   Controller     │ ◀─────▶ │  Home Asst.  │
   │  (Web UI + API)  │   MQTT  │   (updates)  │
   └────────┬─────────┘         └──────────────┘
            │
            │  WebSocket (controller-initiated)
            ▼
   ┌──────────────────┐
   │     Agents       │
   │  (no inbound)    │
   └──────────────────┘
  </pre>
</section>

<section class="section">
  <h2 class="section__heading">Get involved</h2>
  <p>
    Issues, ideas, and pull requests are welcome.
    See <a href="{{ config.extra.github_repo_url }}/blob/main/CONTRIBUTING.md">CONTRIBUTING.md</a>
    for project conventions, and <a href="{{ config.extra.github_repo_url }}/blob/main/SECURITY.md">SECURITY.md</a>
    for the disclosure policy.
  </p>
</section>

{% endblock %}
```

- [ ] **Step 2: Update `website/content/_index.md` to drive the landing template**

```markdown
+++
title = "Uptrakit"
template = "landing.html"
+++
```

(The body of the markdown file is unused because all landing content lives in the template;
keeping the file minimal avoids drift.)

- [ ] **Step 3: Verify build**

```bash
cd website
zola check
zola build
```

Both exit 0.

- [ ] **Step 4: Smoke check**

```bash
zola serve --port 1111
```

Open `http://127.0.0.1:1111/`. Verify:

- Hero, alpha banner, four feature cards, will/won't, security bullets, topology block, get-involved, footer all render in order
- "Install" CTA links to `/install/` (404 in this task — that's expected, install page comes in Task 7)
- "View on GitHub" opens the repo in a new tab
- Theme toggle still works

Stop the server.

- [ ] **Step 5: Stage and commit**

```bash
cd ..
git add website/templates/landing.html website/content/_index.md
git commit -m "feat(website): build landing page sections"
```

---

## Task 7: Install page

**Files:**

- Create: `website/templates/install.html`
- Create: `website/content/install/_index.md`

The install page mirrors the canonical `docs/end-user/deployment/docker.md` Quick Start
verbatim. Before authoring, the engineer must confirm the canonical commands have not
drifted.

- [ ] **Step 1: Confirm the canonical install commands**

Open `docs/end-user/deployment/docker.md` and read the "Quick Start" section. The five
commands below must match exactly. If they have drifted in the canonical doc, update this
task to match the doc (the doc is the source of truth) before proceeding.

Expected canonical commands (as of 2026-04-27):

```bash
git clone https://github.com/worried-networking/uptrakit.git
cd uptrakit
cp .env.example .env
echo "UPTRAKIT_MASTER_KEY=$(openssl rand -hex 32)" >> .env
docker compose up -d
```

- [ ] **Step 2: Create `website/templates/install.html`**

```html
{% extends "base.html" %}
{% import "macros/ui.html" as ui %}

{% block title %}Install Uptrakit (alpha){% endblock %}
{% block description %}Local evaluation install for Uptrakit using Docker Compose. Production deploys live in the canonical deployment guide.{% endblock %}
{% block og_title %}Install Uptrakit (alpha){% endblock %}
{% block og_description %}Local evaluation install for Uptrakit using Docker Compose.{% endblock %}

{% block content %}

<p class="eyebrow">Alpha install</p>
<h1>Try Uptrakit locally</h1>

<p>
  This is a local evaluation flow. It starts the controller with the embedded scheduler
  and SQLite — no MQTT, no external scheduler, no agents. Production deploys need a
  reverse proxy, agent enrollment, and (depending on profile) MQTT or PostgreSQL.
</p>

<h2>Prerequisites</h2>
<ul>
  <li>Docker Engine 24+ with Compose V2</li>
  <li><code>openssl</code> available on <code>$PATH</code> (or any other source of 64 hex characters)</li>
</ul>

<h2>Quickstart</h2>

<div class="install-step">
  <span class="install-step__num">1</span>
  <div class="install-step__body">
    <p>Clone the repository:</p>
<pre><code>git clone https://github.com/worried-networking/uptrakit.git
cd uptrakit</code></pre>
  </div>
</div>

<div class="install-step">
  <span class="install-step__num">2</span>
  <div class="install-step__body">
    <p>Create the environment file and a master encryption key:</p>
<pre><code>cp .env.example .env
echo "UPTRAKIT_MASTER_KEY=$(openssl rand -hex 32)" &gt;&gt; .env</code></pre>
  </div>
</div>

<div class="install-step">
  <span class="install-step__num">3</span>
  <div class="install-step__body">
    <p>Start the controller (default profile — controller + SQLite + embedded scheduler):</p>
<pre><code>docker compose up -d</code></pre>
    <p>The controller is reachable at <code>https://localhost:8443</code>. Expect a self-signed certificate warning on first load.</p>
    <p>The first-run registration token is printed to the controller logs:</p>
<pre><code>docker compose logs controller | grep "registration token"</code></pre>
  </div>
</div>

{{ ui::callout(
  kind="warning",
  title="Production deploys need more",
  body="<p>This snippet runs the controller alone. Production setups need a reverse proxy in front of the controller, an enrollment flow for agents, and depending on profile (mqtt, scheduler, postgres, full) one or more support services. The canonical deployment guide is in the repo.</p>"
) }}

<p>
  Full reference:
  <a href="{{ config.extra.github_repo_url }}/blob/main/docs/end-user/deployment/docker.md">docs/end-user/deployment/docker.md</a>
  on GitHub.
</p>

<p><a href="{{ get_url(path='/') }}">← Back to home</a></p>

{% endblock %}
```

- [ ] **Step 3: Create `website/content/install/_index.md`**

```markdown
+++
title = "Install"
template = "install.html"
+++
```

- [ ] **Step 4: Verify the build and link the routes**

```bash
cd website
zola check
zola build
```

Both exit 0. `zola check` validates that the landing page's "Install" CTA points at a
real route now that `/install/` exists.

- [ ] **Step 5: Smoke**

```bash
zola serve --port 1111
```

Open `http://127.0.0.1:1111/install/`. Verify:

- Eyebrow + heading visible
- Three numbered steps render with code blocks
- Warning callout sits below step 3
- Back-to-home link returns to `/`
- The hero "Install" CTA on `/` jumps here

Stop the server.

- [ ] **Step 6: Stage and commit**

```bash
cd ..
git add website/templates/install.html website/content/install/_index.md
git commit -m "feat(website): add /install/ alpha-evaluation page"
```

---

## Task 8: 404 template

**Files:**

- Create: `website/templates/404.html`

In Zola, `templates/404.html` is rendered automatically to `public/404.html`, which is
what GitHub Pages serves for unmatched paths.

- [ ] **Step 1: Create `website/templates/404.html`**

```html
{% extends "base.html" %}

{% block title %}Not found — Uptrakit{% endblock %}

{% block content %}
  <section class="hero">
    <p class="eyebrow">404</p>
    <h1 class="hero__title">Page not found</h1>
    <p class="hero__sub">The page you asked for does not exist (or has moved).</p>
    <div class="hero__ctas">
      <a class="btn btn--primary" href="{{ get_url(path='/') }}">Back to home</a>
    </div>
  </section>
{% endblock %}
```

- [ ] **Step 2: Verify build emits `public/404.html`**

```bash
cd website
zola check
zola build
test -f public/404.html && echo OK
```

Expected: `OK`.

- [ ] **Step 3: Smoke 404 in `zola serve`**

```bash
zola serve --port 1111
```

Visit `http://127.0.0.1:1111/no-such-page` — Zola serves the 404 template (the dev
server intercepts unmatched paths). Confirm the chrome and back-to-home button render.

Stop the server.

- [ ] **Step 4: Stage and commit**

```bash
cd ..
git add website/templates/404.html
git commit -m "feat(website): add 404 page"
```

---

## Task 9: GitHub Actions workflow

**Files:**

- Create: `.github/workflows/website.yml`

- [ ] **Step 1: Create the workflow**

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
            echo "Artifact size $size exceeds 5MB limit"
            exit 1
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

- [ ] **Step 2: Lint the workflow YAML**

If `actionlint` is installed locally:

```bash
actionlint .github/workflows/website.yml
```

Expected: no errors. If `actionlint` is not installed, skip this step — CI runs the
workflow itself on push, which is the real validation.

- [ ] **Step 3: Stage and commit**

```bash
git add .github/workflows/website.yml
git commit -m "feat(website): add GitHub Actions build + Pages deploy workflow"
```

---

## Task 10: `website/README.md` — local-dev and bump notes

**Files:**

- Create: `website/README.md`

- [ ] **Step 1: Create the README**

````markdown
<!-- markdownlint-disable MD013 -->

# Website (`website/`)

Public marketing site for Uptrakit, served at `https://uptrakit.org`.

## Stack

- [Zola](https://www.getzola.org/) static site generator (single Rust binary, version pinned in CI).
- Plain CSS, system fonts. No Node, no preprocessor.
- Built and deployed by `.github/workflows/website.yml` to GitHub Pages.

See `docs/superpowers/specs/2026-04-27-website-phase-1-design.md` for the design contract.

## Local development

Install Zola once:

```bash
cargo install zola --version 0.22.1
# or, on macOS (version may lag):
brew install zola
```

Serve with auto-reload:

```bash
cd website
zola serve --port 1111
```

Browse `http://127.0.0.1:1111/`.

Validate templates and internal links:

```bash
zola check
```

Produce a production-shaped artifact:

```bash
zola build --output-dir ../public
```

## Deployment

Pushes to `main` that touch `website/**` or `.github/workflows/website.yml` build the
site and deploy the artifact to GitHub Pages via Actions. Pull requests on the same paths
build but do not deploy.

The custom domain (`uptrakit.org`) is set in the GitHub UI:

1. Settings → Pages → Build and deployment → Source = "GitHub Actions".
2. Settings → Pages → Custom domain = `uptrakit.org` (after DNS records are live).
3. Wait for Let's Encrypt provisioning, then enable "Enforce HTTPS".

DNS records on `uptrakit.org`:

- `A` apex → `185.199.108.153`, `185.199.109.153`, `185.199.110.153`, `185.199.111.153`
- `AAAA` apex → `2606:50c0:8000::153`, `2606:50c0:8001::153`, `2606:50c0:8002::153`, `2606:50c0:8003::153`
- `CNAME` `www` → `worried-networking.github.io`

(Verify against
<https://docs.github.com/en/pages/configuring-a-custom-domain-for-your-github-pages-site>
before committing — GitHub publishes anycast IP changes there.)

The CNAME file is committed in `website/static/CNAME`; Zola copies `static/` verbatim,
so the artifact preserves it.

## Bumping Zola

Dependabot bumps `taiki-e/install-action` itself but does not parse the
`tool: zola@<version>` string. To bump Zola:

1. Check the latest release: <https://github.com/getzola/zola/releases>.
2. Edit the `tool:` line in `.github/workflows/website.yml`.
3. Run `zola build` locally to confirm no template/syntax regressions.
4. Open a PR.

## What lives here

| Path | Purpose |
| --- | --- |
| `config.toml` | Zola configuration |
| `content/` | Page content + per-page front-matter |
| `templates/` | Tera templates |
| `templates/macros/ui.html` | Shared callout macro |
| `static/` | Files copied verbatim into the build (CNAME, favicon, robots.txt, css/) |

For phase-2 plans (docs hub at `/docs/`), see follow-up specs in
`docs/superpowers/specs/`.
````

- [ ] **Step 2: Verify markdownlint passes on the file**

```bash
markdownlint --config .markdownlint.json website/README.md
```

Expected: no output (clean).

- [ ] **Step 3: Stage and commit**

```bash
git add website/README.md
git commit -m "docs(website): add local-dev README and bump notes"
```

---

## Task 11: Mention the website from the root README

**Files:**

- Modify: `README.md`

- [ ] **Step 1: Read the current root README to confirm where the new pointer fits**

The "Documentation" section already lists architecture, security, and audience docs.
Add the website entry under "Core overviews" or "Audience docs" — the engineer chooses
the spot that reads most naturally; "Audience docs" is the better fit because the
website is end-user-facing.

- [ ] **Step 2: Add a single-line entry under "Audience docs"**

Insert this line into `README.md` in the audience-docs bullet list, alphabetically
between existing entries:

```markdown
- [website/](website/) — public marketing site at <https://uptrakit.org>
```

- [ ] **Step 3: Verify markdownlint passes**

```bash
markdownlint --config .markdownlint.json README.md
```

Expected: clean.

- [ ] **Step 4: Stage and commit**

```bash
git add README.md
git commit -m "docs(readme): link to website/ directory"
```

---

## Task 12: Open the PR

**Files:**

- (No new files — this task pushes the branch and opens the PR.)

- [ ] **Step 1: Sanity-check the full diff**

```bash
git log --oneline main..HEAD
git diff main...HEAD --stat
```

Expected: a small set of feat/docs commits and a focused diff under `website/`,
`.github/workflows/`, and `README.md` only.

- [ ] **Step 2: Push the branch**

```bash
git push -u origin HEAD
```

- [ ] **Step 3: Open the PR**

```bash
gh pr create --title "feat(website): phase 1 marketing landing on GitHub Pages" --body "$(cat <<'EOF'
## Summary
- Adds a Zola-built marketing site under `website/`, deployed to GitHub Pages at `uptrakit.org`.
- Phase 1 only: landing page (`/`) plus thin alpha-install page (`/install/`). Phase 2 (docs hub at `/docs/`) is deferred to a separate spec.
- Visual design mirrors the product UI tokens (dark default + light, system fonts, flat hover, 120 ms transitions, focus ring).

## Spec
- `docs/superpowers/specs/2026-04-27-website-phase-1-design.md`

## Plan
- `docs/superpowers/plans/2026-04-27-website-phase-1.md`

## Test plan
- [ ] CI build passes (zola check + zola build + artifact size guard).
- [ ] After merge: GitHub Pages deploys; `https://uptrakit.org/` and `https://uptrakit.org/install/` load over HTTPS.
- [ ] Theme toggle persists across reloads in both Chromium and Firefox.
- [ ] Tab navigation shows the focus ring; mouse click does not.
- [ ] Mobile viewport at 320 px wide does not horizontal-scroll.
- [ ] `https://uptrakit.org/no-such-path` serves the custom 404 page.
- [ ] Lighthouse on `/` reports accessibility ≥ 95, performance ≥ 95, best-practices ≥ 95, SEO ≥ 95.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 4: Note the PR URL in the implementation log**

The URL printed by `gh pr create` is the handoff to the human reviewer. Do not merge
until the post-merge GitHub UI configuration steps in `website/README.md` are also
followed (Pages source = Actions; custom domain; HTTPS enforcement; DNS records).

---

## Post-merge manual steps (humans only — out of plan scope)

These cannot be automated from a PR. After the workflow's first successful main-branch
deploy:

1. GitHub repo → Settings → Pages → Build and deployment → Source = "GitHub Actions".
2. Add custom domain `uptrakit.org`.
3. Configure DNS at the registrar per the records listed in `website/README.md`.
4. Wait for HTTPS provisioning, then check "Enforce HTTPS".
5. Confirm `https://uptrakit.org/` and `https://uptrakit.org/install/` load.
6. Run Lighthouse and record scores in the PR for follow-up.

---

## Verification checklist (rolls up the per-task smoke checks)

- [ ] `cd website && zola check && zola build` exits 0
- [ ] `website/public/index.html`, `install/index.html`, `404.html` all exist after build
- [ ] `website/public/CNAME` exists and contains `uptrakit.org`
- [ ] `website/public/sitemap.xml` exists (Zola auto-generates)
- [ ] `website/public/robots.txt` exists
- [ ] `du -sb website/public` returns < 5 MB
- [ ] `markdownlint --config .markdownlint.json website/README.md README.md` clean
- [ ] CI workflow on PR shows the build job passing; deploy job is skipped on PR (only runs on main)
