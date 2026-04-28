<!-- markdownlint-disable MD013 MD031 MD032 -->

# Website Phase 2 — Docs Hub Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend the Phase 1 Zola site into a searchable documentation hub publishing `docs/end-user/` and `docs/security/` at `/docs/` via symlinks, with Pagefind search, GFM alerts, and the Phase 1 OG image asset.

**Architecture:** Single-source via symlinks in `website/content/docs/`; YAML front matter added to source files; unified `docs.html` Tera template with two-column sidebar layout; Pagefind index built in CI after `zola build`.

**Tech Stack:** Zola 0.22.1, Tera templates, Pagefind 1.x (npx), ImageMagick (og.png authoring), GitHub Actions Pages deploy.

---

## File Map

| Status | Path | Responsibility |
| --- | --- | --- |
| Modify | `website/config.toml` | Add `[markdown]` section |
| Create | `website/content/docs/_index.md` | Docs hub landing page |
| Create | `website/content/docs/end-user` | Symlink → `../../../docs/end-user/` |
| Create | `website/content/docs/security` | Symlink → `../../../docs/security/` |
| Create | `docs/end-user/_index.md` | Zola section metadata |
| Create | `docs/end-user/deployment/_index.md` | Zola section metadata |
| Create | `docs/end-user/plugins/_index.md` | Zola section metadata |
| Create | `docs/security/_index.md` | Zola section metadata |
| Create | `docs/end-user/plugins/README.md` | New authored overview page |
| Modify | `docs/end-user/*.md` (26 files) | Add YAML front matter |
| Modify | `docs/end-user/deployment/*.md` (10 files) | Add YAML front matter |
| Modify | `docs/end-user/plugins/*.md` (4 files) | Add YAML front matter |
| Modify | `docs/security/*.md` (19 files) | Add YAML front matter |
| Modify | All 60 published `.md` files | GFM alert migration |
| Modify | ~42 published `.md` files | Rewrite unpublished cross-links to GitHub `tree/` URLs |
| Create | `website/templates/docs.html` | Unified docs template |
| Modify | `website/templates/base.html` | Add Docs/Install nav links + og:image |
| Modify | `website/static/css/site.css` | Docs layout, sidebar, GFM alert, pre override |
| Create | `website/static/og.png` | 1200×630 social card |
| Modify | `website/templates/install.html` | Update deployment reference link |
| Modify | `.github/workflows/website.yml` | Paths filter, pagefind step, 10 MB guard |
| Modify | `website/README.md` | Document pagefind version bump procedure |

---

## Task 1: Add `[markdown]` section to `website/config.toml`

**Files:**
- Modify: `website/config.toml`

- [ ] **Step 1: Append `[markdown]` section**

Add after the `[extra]` block:

```toml
[markdown]
highlight_code = true
highlight_theme = "base16-ocean-dark"
github_alerts = true
```

Full file after edit:

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

[extra]
github_repo_url = "https://github.com/worried-networking/uptrakit"

[markdown]
highlight_code = true
highlight_theme = "base16-ocean-dark"
github_alerts = true
```

- [ ] **Step 2: Commit**

```bash
git add website/config.toml
git commit -m "feat(website): add [markdown] section with GFM alerts and syntax highlighting"
```

---

## Task 2: Create `website/content/docs/` structure

**Files:**
- Create: `website/content/docs/_index.md`
- Create: `website/content/docs/end-user` (symlink)
- Create: `website/content/docs/security` (symlink)

- [ ] **Step 1: Create the hub landing `_index.md`**

```bash
mkdir -p website/content/docs
```

Write `website/content/docs/_index.md`:

```markdown
---
title: Documentation
description: Uptrakit documentation hub — end-user guides, security, and deployment references.
sort_by: "weight"
---

Browse documentation for Uptrakit:

- [End-user guides](/docs/end-user/) — update workflows, deployment, plugins, and integrations.
- [Security](/docs/security/) — security architecture, cryptography, PKI, and hardening guides.
```

- [ ] **Step 2: Create symlinks**

```bash
cd website/content/docs
ln -s ../../../docs/end-user end-user
ln -s ../../../docs/security security
```

Verify they resolve correctly:

```bash
ls -la website/content/docs/
# end-user -> ../../../docs/end-user
# security -> ../../../docs/security
readlink -f website/content/docs/end-user
# → <repo-root>/docs/end-user
```

- [ ] **Step 3: Commit**

```bash
git add website/content/docs/_index.md website/content/docs/end-user website/content/docs/security
git commit -m "feat(website): add docs/ content root with hub landing and symlinks to source docs"
```

---

## Task 3: Create `_index.md` files in source doc directories

Zola requires an `_index.md` at each section level. These files contain metadata only — no body content — so GitHub directory browsing is unaffected (GitHub shows `README.md` not `_index.md`).

**Files:**
- Create: `docs/end-user/_index.md`
- Create: `docs/end-user/deployment/_index.md`
- Create: `docs/end-user/plugins/_index.md`
- Create: `docs/security/_index.md`

- [ ] **Step 1: Create all four `_index.md` files**

`docs/end-user/_index.md`:

```markdown
---
title: End-user Guides
description: User-facing guides for operating Uptrakit — update workflows, deployment, plugins, and integrations.
sort_by: "weight"
---
```

`docs/end-user/deployment/_index.md`:

```markdown
---
title: Deployment Guides
description: Deployment-specific references for running Uptrakit behind a reverse proxy.
sort_by: "weight"
---
```

`docs/end-user/plugins/_index.md`:

```markdown
---
title: Plugin Reference
description: Per-plugin configuration reference for the Uptrakit package-manager plugins.
sort_by: "weight"
---
```

`docs/security/_index.md`:

```markdown
---
title: Security
description: Security architecture, cryptography, PKI, authentication, and secure deployment guidance.
sort_by: "weight"
---
```

- [ ] **Step 2: Run `zola check` to confirm Zola sees the sections**

```bash
cd website && zola check
```

Expected: PASS (no pages have front matter yet so there are no leaf pages to check; only sections are registered).

If `zola check` fails with "unknown section path", the symlinks are not followed. Re-verify symlinks with `readlink -f`.

- [ ] **Step 3: Commit**

```bash
git add docs/end-user/_index.md docs/end-user/deployment/_index.md docs/end-user/plugins/_index.md docs/security/_index.md
git commit -m "feat(docs): add Zola _index.md section metadata files"
```

---

## Task 4: Create `docs/end-user/plugins/README.md`

This is the only new content file in Phase 2. The plugins subsection has no existing overview page.

**Files:**
- Create: `docs/end-user/plugins/README.md`

- [ ] **Step 1: Write the file**

```markdown
---
title: Plugin Reference
weight: 1
description: Per-plugin configuration reference for the Uptrakit package-manager plugins.
---

# Plugin Reference

Uptrakit ships built-in plugins for common Linux package managers. Each plugin page documents the configuration schema, supported options, and behavior notes specific to that package manager.

| Plugin | Description |
| --- | --- |
| [APT](apt.md) | Debian and Ubuntu package management via `apt`. |
| [DNF](dnf.md) | Fedora and RHEL package management via `dnf`. |
| [Docker](docker.md) | Docker image and container update tracking. |
| [Pacman](pacman.md) | Arch Linux package management via `pacman`. |

## Related Documentation

- [Plugin Configurations](../plugin-configs.md) — managing plugin configs, supported plugin types, and autodiscovery.
- [Manual Software Tracking](../manual-software-tracking.md) — setting up tracking for software that cannot be autodiscovered.
```

- [ ] **Step 2: Commit**

```bash
git add docs/end-user/plugins/README.md
git commit -m "docs(plugins): add Plugin Reference overview page"
```

---

## Task 5: Add YAML front matter — end-user top-level pages

Front matter is added to every `.md` file directly under `docs/end-user/` (not in subdirectories). The `description` field must be a one-sentence summary — write it from the file's own opening paragraph.

Weights derive from the README contents table order. Files absent from the README get sequential weights starting at 130.

**Files:**
- Modify: `docs/end-user/README.md` and 25 other `.md` files

- [ ] **Step 1: Add front matter to each file**

For each file below, prepend the front matter block shown. The `description` value must be authored from the file's own opening paragraph — read the file, write one sentence.

Front matter template:

```yaml
---
title: <Title from H1 heading>
weight: <weight from table below>
description: <One-sentence summary written from the file's opening paragraph>
---
```

**Weight table — end-user top-level:**

| File | Weight | Title (from H1) |
| --- | --- | --- |
| `README.md` | 1 | Overview |
| `system-overview.md` | 10 | System Overview |
| `cli-usage.md` | 20 | CLI Usage Guide |
| `plugin-configs.md` | 30 | Plugin Configurations |
| `manual-software-tracking.md` | 40 | Manual Software Tracking |
| `autodiscovery.md` | 50 | Autodiscovery |
| `update-workflow.md` | 60 | Update Workflow |
| `update-history.md` | 70 | Update History |
| `notifications.md` | 80 | Notifications |
| `profile-tokens.md` | 90 | Profile and API Tokens |
| `home-assistant-mqtt.md` | 100 | Home Assistant and MQTT |
| `deployment-map.md` | 110 | Deployment Map |
| `db-migration.md` | 120 | Database Data Migration |
| `audit-logs.md` | 130 | Audit Logs |
| `batch-actions.md` | 140 | Batch Actions |
| `dashboard-icons.md` | 150 | Dashboard Icons |
| `host-packages.md` | 160 | Host packages *(see note below)* |
| `interactive-updates.md` | 170 | Interactive Updates |
| `npm-plugin.md` | 180 | npm Plugin (`package_manager_npm`) |
| `proxmox.md` | 190 | Proxmox VE Integration |
| `snap-plugin.md` | 200 | Snap Package Manager Plugin |
| `ssh-agent-bootstrap.md` | 210 | SSH Agent Bootstrap |
| `ssh-agent-host-management.md` | 220 | SSH Agent Host Management |
| `surfaces.md` | 230 | Shared Surfaces |
| `user-management.md` | 240 | User Management |
| `zeroconf-discovery.md` | 250 | Zero-Configuration Service Discovery |

For `README.md` specifically:

```yaml
---
title: Overview
weight: 1
description: <one sentence from the file's opening paragraph>
---
```

For every other file, open it, read the H1 and opening paragraph, fill in `title` (matching H1 exactly, even if lowercase) and `description` (one sentence from the opening paragraph). Do not change the body of any file beyond prepending the front matter block.

**Special case — `host-packages.md`:** This file is a superseded stub with no substantive content (it redirects readers to `autodiscovery.md`). Add front matter with `draft = true` so Zola excludes it from the build. The file still needs front matter to satisfy the gate check; `draft = true` hides it from the sidebar and search index:

```yaml
---
title: Host packages
weight: 160
description: This page has been merged into the unified software tracking documentation.
draft: true
---
```

- [ ] **Step 2: Verify front matter count**

```bash
find docs/end-user -maxdepth 1 -name "*.md" | xargs grep -L "^---" | wc -l
```

Expected: `0`

- [ ] **Step 3: Commit**

```bash
git add docs/end-user/*.md
git commit -m "docs(end-user): add YAML front matter to top-level pages"
```

---

## Task 6: Add YAML front matter — end-user deployment pages

**Files:**
- Modify: `docs/end-user/deployment/README.md` and 9 other `.md` files

- [ ] **Step 1: Add front matter to each deployment file**

Use the same front matter template as Task 5. Open each file, read H1 and opening paragraph, fill in `title` and `description`.

**Weight table — deployment:**

| File | Weight | Title (from H1) |
| --- | --- | --- |
| `README.md` | 1 | Overview |
| `reverse-proxy.md` | 10 | Reverse Proxy Deployment |
| `nginx.md` | 20 | Nginx |
| `nginx-proxy-manager.md` | 30 | Nginx Proxy Manager |
| `traefik.md` | 40 | Traefik |
| `caddy.md` | 50 | Caddy |
| `envoy.md` | 60 | Envoy |
| `haproxy.md` | 70 | HAProxy |
| `docker.md` | 80 | Docker Deployment |
| `external-scheduler.md` | 90 | External Scheduler |
| `nats.md` | 100 | NATS |

- [ ] **Step 2: Verify front matter count**

```bash
find docs/end-user/deployment -name "*.md" | xargs grep -L "^---" | wc -l
```

Expected: `0`

- [ ] **Step 3: Commit**

```bash
git add docs/end-user/deployment/*.md
git commit -m "docs(end-user/deployment): add YAML front matter to deployment pages"
```

---

## Task 7: Add YAML front matter — end-user plugins pages

**Files:**
- Modify: `docs/end-user/plugins/apt.md`, `dnf.md`, `docker.md`, `pacman.md`
- `docs/end-user/plugins/README.md` already has front matter from Task 4.

- [ ] **Step 1: Add front matter to each plugins file**

**Weight table — plugins:**

| File | Weight | Title (from H1) |
| --- | --- | --- |
| `apt.md` | 10 | APT Plugin |
| `dnf.md` | 20 | DNF Plugin |
| `docker.md` | 30 | Docker Plugin |
| `pacman.md` | 40 | Pacman Plugin |

- [ ] **Step 2: Verify front matter count**

```bash
find docs/end-user/plugins -name "*.md" | xargs grep -L "^---" | wc -l
```

Expected: `0`

- [ ] **Step 3: Commit**

```bash
git add docs/end-user/plugins/apt.md docs/end-user/plugins/dnf.md docs/end-user/plugins/docker.md docs/end-user/plugins/pacman.md
git commit -m "docs(end-user/plugins): add YAML front matter to plugin pages"
```

---

## Task 8: Add YAML front matter — security pages

**Files:**
- Modify: `docs/security/README.md` and 18 other `.md` files

- [ ] **Step 1: Add front matter to each security file**

**Weight table — security:**

| File | Weight | Title (from H1) |
| --- | --- | --- |
| `README.md` | 1 | Overview |
| `security-architecture.md` | 10 | Security Architecture |
| `cryptography.md` | 20 | Cryptography |
| `pki-certificates.md` | 30 | PKI and Certificates |
| `auth-and-authorization.md` | 40 | Auth and Authorization |
| `secrets-and-encryption.md` | 50 | Secrets and Encryption |
| `tofu-tls.md` | 60 | TOFU and TLS |
| `filesystem-dependency-security.md` | 70 | Filesystem and Dependency Security |
| `secure-development.md` | 80 | Secure Development |
| `reverse-proxy-security.md` | 90 | Reverse Proxy Security |
| `ssh-agent-secrets.md` | 100 | SSH Agent Secrets |
| `sudoers-management.md` | 110 | Sudoers Management |
| `notifications-security.md` | 120 | Notification Subsystem Security |
| `audit-logs.md` | 130 | Audit Log Security |
| `github-attestation.md` | 140 | GitHub Actions Attestation Verification |
| `interactive-updates.md` | 150 | Interactive Updates Security |
| `key-rotation.md` | 160 | Master Key Rotation |
| `surfaces.md` | 170 | Shared Surface Security |
| `zeroconf-discovery.md` | 180 | Zero-Configuration Discovery Security |

- [ ] **Step 2: Verify front matter count**

```bash
find docs/security -name "*.md" | xargs grep -L "^---" | wc -l
```

Expected: `0`

- [ ] **Step 3: Run the global pre-merge gate**

```bash
find docs/end-user docs/security -name "*.md" | xargs grep -L "^---" | wc -l
```

Expected: `0`

- [ ] **Step 4: Commit**

```bash
git add docs/security/*.md
git commit -m "docs(security): add YAML front matter to security pages"
```

---

## Task 9: Migrate GFM alerts

Replace all `> **Label:**` callout patterns in `docs/end-user/` and `docs/security/` with GFM alert syntax (`> [!NOTE]` etc.). Zola renders these as `<blockquote class="markdown-alert-*">` when `github_alerts = true`.

**Mapping:**

| Old pattern | New first line | Notes |
| --- | --- | --- |
| `> **Note:**` | `> [!NOTE]` | Drop the label entirely |
| `> **Note**:` | `> [!NOTE]` | Variant spelling |
| `> **Tip:**` | `> [!TIP]` | Drop the label |
| `> **Important:**` | `> [!IMPORTANT]` | Drop the label |
| `> **Warning:**` | `> [!WARNING]` | Drop the label |
| `> **Security note:**` | `> [!CAUTION]` | Drop the label |
| Any other `> **Label:**` | `> [!NOTE]` | Fold label text into first sentence |

**Conversion example — standard label:**

Before:
```markdown
> **Note:** API tokens are scoped to a single tenant.
```

After:
```markdown
> [!NOTE]
> API tokens are scoped to a single tenant.
```

**Conversion example — multi-word label:**

Before:
```markdown
> **Output size limit:** Uptrakit stores up to 50 MB of output per update.
```

After:
```markdown
> [!NOTE]
> Output size limit: Uptrakit stores up to 50 MB of output per update.
```

**Conversion example — multi-line alert body:**

Before:
```markdown
> **Note:** First line of the callout.
> Continuation on the next line.
```

After:
```markdown
> [!NOTE]
> First line of the callout.
> Continuation on the next line.
```

The `> [!NOTE]` marker occupies its own line. All subsequent `>` continuation lines remain unchanged — no blank line is inserted between the marker and the body.

**Files:**
- Modify: all `docs/end-user/**/*.md` and `docs/security/*.md` that contain `> **`

- [ ] **Step 1: Find all instances**

```bash
grep -rn "^> \*\*" docs/end-user docs/security
```

Review the output. For each match: determine the correct target type using the mapping table above, apply the conversion.

- [ ] **Step 2: Convert each instance**

Work through the grep output file by file. For each occurrence:

1. Check the label word(s) against the mapping table.
2. Remove the `> **Label:**` prefix (or `> **Label**:`) and insert `> [!TYPE]` on a new line above the blockquote body.
3. For multi-word labels not in the table, use `> [!NOTE]` and fold the label into the first sentence (see example above).

- [ ] **Step 3: Verify none remain**

```bash
grep -r "^> \*\*" docs/end-user docs/security | wc -l
```

Expected: `0`

- [ ] **Step 4: Commit**

```bash
git add docs/end-user docs/security
git commit -m "docs: migrate all callouts to GFM alert syntax"
```

---

## Task 10: Rewrite cross-links to unpublished sections

42 files in `docs/end-user/` and `docs/security/` contain ~126 links to unpublished sections (`docs/api/`, `docs/architecture/`, `docs/development/`, `docs/hackme/`). These must be rewritten to GitHub directory URLs before `zola check` can pass.

Target URL pattern: `https://github.com/worried-networking/uptrakit/tree/main/docs/<section>/`

Do NOT link to individual file blob URLs (`blob/main/<file>`). Directory URLs survive file-level renames within a section.

**Files:**
- Modify: ~42 files across `docs/end-user/` and `docs/security/`

- [ ] **Step 1: Find all cross-links to unpublished sections**

```bash
grep -rn "../api/" docs/end-user docs/security
grep -rn "../architecture/" docs/end-user docs/security
grep -rn "../development/" docs/end-user docs/security
grep -rn "../hackme/" docs/end-user docs/security
grep -rn "../../api/" docs/end-user docs/security
grep -rn "../../architecture/" docs/end-user docs/security
grep -rn "../../development/" docs/end-user docs/security
```

Save this output — it is your work list.

- [ ] **Step 2: Rewrite each link**

For each match, replace the relative path with the corresponding GitHub directory URL:

| Relative path (any depth) | Replace with |
| --- | --- |
| `../api/` or `../../api/` (or deeper) | `https://github.com/worried-networking/uptrakit/tree/main/docs/api/` |
| `../architecture/` or deeper | `https://github.com/worried-networking/uptrakit/tree/main/docs/architecture/` |
| `../development/` or deeper | `https://github.com/worried-networking/uptrakit/tree/main/docs/development/` |
| `../hackme/` or deeper | `https://github.com/worried-networking/uptrakit/tree/main/docs/hackme/` |

If the link target is a specific file (e.g. `../api/README.md`), still replace the whole href with the directory URL (e.g. `.../tree/main/docs/api/`). The link text is unchanged.

Example:

Before:
```markdown
See the [API docs](../api/README.md) for endpoint reference.
```

After:
```markdown
See the [API docs](https://github.com/worried-networking/uptrakit/tree/main/docs/api/) for endpoint reference.
```

- [ ] **Step 3: Verify no relative cross-links remain**

```bash
grep -rn "\.\./api/" docs/end-user docs/security | wc -l
grep -rn "\.\./architecture/" docs/end-user docs/security | wc -l
grep -rn "\.\./development/" docs/end-user docs/security | wc -l
grep -rn "\.\./hackme/" docs/end-user docs/security | wc -l
```

Expected: all return `0`

- [ ] **Step 4: Run `zola check`**

```bash
cd website && zola check
```

Expected: PASS. If it fails with broken link errors, read the error output — the failing link is shown. Fix it and re-run.

**Cross-symlink links between published sections:** `docs/security/` pages link to `docs/end-user/` pages and vice versa via relative paths (e.g. `../end-user/deployment/traefik.md`, `../security/ssh-agent-secrets.md`). These cross a symlink boundary inside `website/content/docs/`. If `zola check` reports these as broken:

1. Find them:
   ```bash
   grep -rn "\.\./end-user/" docs/security
   grep -rn "\.\./security/" docs/end-user
   ```
2. Rewrite each relative cross-section link to an absolute site path:
   ```markdown
   <!-- before -->
   [Traefik guide](../end-user/deployment/traefik.md)

   <!-- after -->
   [Traefik guide](/docs/end-user/deployment/traefik/)
   ```
   Use the site URL path (trailing slash, no `.md` extension) — Zola renders pages to directories.
3. Re-run `zola check` after rewriting.

If `zola check` passes without these rewrites, Zola resolved the cross-symlink links correctly — no action needed.

- [ ] **Step 5: Commit**

```bash
git add docs/end-user docs/security
git commit -m "docs: rewrite cross-links to unpublished sections as GitHub tree/ URLs"
```

---

## Task 11: Run all pre-merge gates

- [ ] **Step 1: Front matter gate**

```bash
find docs/end-user docs/security -name "*.md" | xargs grep -L "^---" | wc -l
```

Expected: `0`. If non-zero, the output lists the files missing front matter. Add front matter and re-run.

- [ ] **Step 2: GFM alert gate**

```bash
grep -r "^> \*\*" docs/end-user docs/security | wc -l
```

Expected: `0`. If non-zero, the output shows the file and line. Migrate the remaining callout and re-run.

- [ ] **Step 3: Zola check**

```bash
cd website && zola check
```

Expected: PASS with no broken link warnings. If it fails:
- Broken internal link: find the file shown, fix the link.
- Missing section: verify `_index.md` exists in the referenced directory.
- Symlink not followed: check `readlink -f website/content/docs/end-user`.

All three gates must pass before proceeding to template work.

---

## Task 12: Create `website/templates/docs.html`

Unified template for all docs pages — both section indexes and leaf pages.

**Files:**
- Create: `website/templates/docs.html`

- [ ] **Step 1: Write the template**

```html
{% extends "base.html" %}

{% block title %}
  {%- if page -%}{{ page.title }} — {{ config.title }}
  {%- elif section -%}{{ section.title }} — {{ config.title }}
  {%- else -%}{{ config.title }}
  {%- endif -%}
{% endblock %}

{% block description %}
  {%- if page and page.description -%}{{ page.description }}
  {%- elif section and section.description -%}{{ section.description }}
  {%- else -%}{{ config.description }}
  {%- endif -%}
{% endblock %}

{% block og_title %}
  {%- if page -%}{{ page.title }}
  {%- elif section -%}{{ section.title }}
  {%- else -%}{{ config.title }}
  {%- endif -%}
{% endblock %}

{% block og_description %}
  {%- if page and page.description -%}{{ page.description }}
  {%- elif section and section.description -%}{{ section.description }}
  {%- else -%}{{ config.description }}
  {%- endif -%}
{% endblock %}

{% block content %}
<div class="docs-layout">

  <button class="docs-hamburger" type="button" aria-label="Open navigation" data-sidebar-toggle>
    <span aria-hidden="true">☰</span>
  </button>
  <div class="docs-overlay" aria-hidden="true" data-sidebar-overlay></div>

  <aside class="docs-sidebar" id="docs-sidebar">
    <div class="docs-sidebar__search">
      <link rel="stylesheet" href="/pagefind/pagefind-ui.css">
      <div id="search"></div>
      <script src="/pagefind/pagefind-ui.js"></script>
      <script>
        window.addEventListener('DOMContentLoaded', function () {
          new PagefindUI({ element: '#search', showSubResults: true });
        });
      </script>
    </div>

    <nav class="docs-sidebar__nav" aria-label="Documentation sections">
      {%- set docs_root = get_section(path="docs/_index.md") -%}
      {%- for sub_path in docs_root.subsections -%}
        {%- set sub = get_section(path=sub_path) -%}
        <div class="sidebar-section">
          <a class="sidebar-section__title" href="{{ sub.permalink }}">{{ sub.title }}</a>

          {%- if sub.pages -%}
          <ul class="sidebar-pages">
            {%- for p in sub.pages -%}
              {%- set is_active = current_path == p.path -%}
              <li>
                <a class="sidebar-page{% if is_active %} is-active{% endif %}"
                   href="{{ p.permalink }}">{{ p.title }}</a>
              </li>
            {%- endfor -%}
          </ul>
          {%- endif -%}

          {%- for nested_path in sub.subsections -%}
            {%- set nested = get_section(path=nested_path) -%}
            <details{% if (page and nested.relative_path in page.ancestors) or (section and (section.relative_path == nested.relative_path or nested.relative_path in section.ancestors)) %} open data-active{% endif %}>
              <summary class="sidebar-nested-title">
                <a href="{{ nested.permalink }}">{{ nested.title }}</a>
              </summary>
              {%- if nested.pages -%}
              <ul class="sidebar-pages sidebar-pages--nested">
                {%- for np in nested.pages -%}
                  {%- set is_active = current_path == np.path -%}
                  <li>
                    <a class="sidebar-page{% if is_active %} is-active{% endif %}"
                       href="{{ np.permalink }}">{{ np.title }}</a>
                  </li>
                {%- endfor -%}
              </ul>
              {%- endif -%}
            </details>
          {%- endfor -%}
        </div>
      {%- endfor -%}
    </nav>
  </aside>

  <div class="docs-content">

    <nav class="docs-breadcrumbs" aria-label="Breadcrumb">
      <a href="/docs/">Docs</a>
      {%- if page -%}
        {%- for anc_path in page.ancestors -%}
          {%- set anc = get_section(path=anc_path) -%}
          {%- if anc.relative_path != "docs/_index.md" -%}
            <span aria-hidden="true"> › </span><a href="{{ anc.permalink }}">{{ anc.title }}</a>
          {%- endif -%}
        {%- endfor -%}
        <span aria-hidden="true"> › </span><span>{{ page.title }}</span>
      {%- elif section and section.relative_path != "docs/_index.md" -%}
        {%- for anc_path in section.ancestors -%}
          {%- set anc = get_section(path=anc_path) -%}
          {%- if anc.relative_path != "docs/_index.md" -%}
            <span aria-hidden="true"> › </span><a href="{{ anc.permalink }}">{{ anc.title }}</a>
          {%- endif -%}
        {%- endfor -%}
        <span aria-hidden="true"> › </span><span>{{ section.title }}</span>
      {%- endif -%}
    </nav>

    {%- if section and section.relative_path != "docs/_index.md" -%}
    <div class="docs-alpha-banner" role="alert">
      <strong>Alpha documentation.</strong> Content may be incomplete or change without notice.
    </div>
    {%- endif -%}

    {%- if page -%}
    <article class="docs-article">
      <h1>{{ page.title }}</h1>
      {{ page.content | safe }}

      <nav class="docs-prevnext" aria-label="Page navigation">
        {%- if page.higher -%}
          <a class="docs-prevnext__prev" href="{{ page.higher.permalink }}">← {{ page.higher.title }}</a>
        {%- endif -%}
        {%- if page.lower -%}
          <a class="docs-prevnext__next" href="{{ page.lower.permalink }}">{{ page.lower.title }} →</a>
        {%- endif -%}
      </nav>

      <div class="docs-edit-link">
        <a href="{{ config.extra.github_repo_url }}/blob/main/{{ page.relative_path }}"
           rel="noopener" target="_blank">Edit this page on GitHub</a>
      </div>
    </article>

    {%- elif section -%}
    <article class="docs-article">
      <h1>{{ section.title }}</h1>
      {{ section.content | safe }}

      {%- if section.pages -%}
      <ul class="docs-section-index">
        {%- for p in section.pages -%}
          <li>
            <a href="{{ p.permalink }}">{{ p.title }}</a>
            {%- if p.description -%} — {{ p.description }}{%- endif -%}
          </li>
        {%- endfor -%}
      </ul>
      {%- endif -%}

      {%- if section.subsections -%}
      <ul class="docs-section-index">
        {%- for sp in section.subsections -%}
          {%- set s = get_section(path=sp) -%}
          <li>
            <a href="{{ s.permalink }}">{{ s.title }}</a>
            {%- if s.description -%} — {{ s.description }}{%- endif -%}
          </li>
        {%- endfor -%}
      </ul>
      {%- endif -%}
    </article>
    {%- endif -%}

  </div>
</div>

<script>
  (function () {
    var toggle = document.querySelector('[data-sidebar-toggle]');
    var overlay = document.querySelector('[data-sidebar-overlay]');
    var sidebar = document.getElementById('docs-sidebar');
    if (!toggle || !overlay || !sidebar) return;

    function open() {
      sidebar.setAttribute('data-open', '');
      overlay.setAttribute('data-open', '');
      toggle.setAttribute('aria-expanded', 'true');
    }
    function close() {
      sidebar.removeAttribute('data-open');
      overlay.removeAttribute('data-open');
      toggle.setAttribute('aria-expanded', 'false');
    }

    toggle.addEventListener('click', function () {
      sidebar.hasAttribute('data-open') ? close() : open();
    });
    overlay.addEventListener('click', close);

    document.addEventListener('keydown', function (e) {
      if (e.key === 'Escape') close();
    });
  })();
</script>
{% endblock %}
```

- [ ] **Step 2: Run `zola check`**

```bash
cd website && zola check
```

Expected: PASS. The template is used only if a section or page declares `template = "docs.html"` — at this stage no page does, so the template compiles but is not applied. If Zola reports a Tera syntax error, fix the error and re-run.

- [ ] **Step 3: Wire up the template**

**These are modifications to files already created in Tasks 2 and 3.** Replace the entire front matter block in each file — do not create new files. Add `template: "docs.html"` (and `page_template: "docs.html"` where applicable) to the existing front matter.

Files to modify: `website/content/docs/_index.md` (from Task 2) and the four source `_index.md` files from Task 3.

`website/content/docs/_index.md`:

```markdown
---
title: Documentation
description: Uptrakit documentation hub — end-user guides, security, and deployment references.
sort_by: "weight"
template: "docs.html"
---

Browse documentation for Uptrakit:

- [End-user guides](/docs/end-user/) — update workflows, deployment, plugins, and integrations.
- [Security](/docs/security/) — security architecture, cryptography, PKI, and hardening guides.
```

`docs/end-user/_index.md`:

```markdown
---
title: End-user Guides
description: User-facing guides for operating Uptrakit — update workflows, deployment, plugins, and integrations.
sort_by: "weight"
template: "docs.html"
---
```

`docs/end-user/deployment/_index.md`:

```markdown
---
title: Deployment Guides
description: Deployment-specific references for running Uptrakit behind a reverse proxy.
sort_by: "weight"
template: "docs.html"
---
```

`docs/end-user/plugins/_index.md`:

```markdown
---
title: Plugin Reference
description: Per-plugin configuration reference for the Uptrakit package-manager plugins.
sort_by: "weight"
template: "docs.html"
---
```

`docs/security/_index.md`:

```markdown
---
title: Security
description: Security architecture, cryptography, PKI, authentication, and secure deployment guidance.
sort_by: "weight"
template: "docs.html"
---
```

For leaf pages, Zola inherits the template from the parent section when the section's `_index.md` sets `page_template`. However, Zola does NOT propagate `template` to leaf pages — it uses `page_template` for that. Update each `_index.md` to add `page_template`:

Add `page_template: "docs.html"` to the four source `_index.md` files (not the hub landing — hub landing has no leaf pages of its own):

```markdown
---
title: End-user Guides
description: User-facing guides for operating Uptrakit — update workflows, deployment, plugins, and integrations.
sort_by: "weight"
template: "docs.html"
page_template: "docs.html"
---
```

Apply `page_template: "docs.html"` to all four source `_index.md` files: `docs/end-user/_index.md`, `docs/end-user/deployment/_index.md`, `docs/end-user/plugins/_index.md`, `docs/security/_index.md`.

- [ ] **Step 4: Run `zola check` again**

```bash
cd website && zola check
```

Expected: PASS. If Zola reports template variable errors (e.g., `page.ancestors` empty for symlinked pages), see the "Risks and open items" section of the spec and apply the fallback: build the sidebar by iterating `section.subsections` from the root docs section instead of using `get_section` with logical paths. Then re-run `zola check`.

- [ ] **Step 5: Local build smoke check**

```bash
cd website && zola build
python3 -m http.server 8080 --directory public
```

Navigate to `http://localhost:8080/docs/end-user/deployment/nginx/` (a nested leaf page) and verify:
- Sidebar renders with section headings and page list.
- Deployment `<details>` group is **open** and Nginx is highlighted as active.
- Breadcrumbs show `Docs › End-user Guides › Deployment Guides › Nginx`.
- Alpha banner visible.

This verifies that `page.ancestors in` check works through symlinks. If the `<details>` group is closed (not open), Zola is returning resolved paths in `page.ancestors` rather than logical content-relative paths. Fallback: replace the `nested.relative_path in page.ancestors` condition in `docs.html` with `current_path is starting_with(nested.permalink | replace(from=config.base_url, to="/"))`. Then rebuild and verify again.

Also navigate to `/docs/` hub landing:
- Breadcrumbs: absent or just "Docs" with no separator.
- Alpha banner: absent.
- Sidebar present.

- [ ] **Step 6: Commit**

```bash
git add website/templates/docs.html website/content/docs/_index.md docs/end-user/_index.md docs/end-user/deployment/_index.md docs/end-user/plugins/_index.md docs/security/_index.md
git commit -m "feat(website): add docs.html template; add template/page_template to section _index.md files"
```

---

## Task 13: Update `website/templates/base.html`

Add "Docs" and "Install" nav links and the og:image meta tag.

**Files:**
- Modify: `website/templates/base.html`

- [ ] **Step 1: Add nav links**

Current nav block (lines 40–45 of `website/templates/base.html`):

```html
    <nav class="topbar__nav">
      <a class="topbar__link" href="{{ config.extra.github_repo_url }}" rel="noopener" target="_blank">GitHub</a>
      <button class="topbar__theme-toggle" type="button" aria-label="Toggle theme" data-theme-toggle>
        <span aria-hidden="true">◐</span>
      </button>
    </nav>
```

Replace with:

```html
    <nav class="topbar__nav">
      <a class="topbar__link{% if current_path is starting_with('/docs/') %} topbar__link--active{% endif %}"
         href="/docs/">Docs</a>
      <a class="topbar__link{% if current_path is starting_with('/install/') %} topbar__link--active{% endif %}"
         href="/install/">Install</a>
      <a class="topbar__link" href="{{ config.extra.github_repo_url }}" rel="noopener" target="_blank">GitHub</a>
      <button class="topbar__theme-toggle" type="button" aria-label="Toggle theme" data-theme-toggle>
        <span aria-hidden="true">◐</span>
      </button>
    </nav>
```

`is starting_with(...)` is a built-in Tera test and works correctly here. Use literal `/docs/` and `/install/` hrefs — `get_url(path='/docs/')` treats the path as a static file lookup (not a content section) and would produce the wrong URL.

- [ ] **Step 2: Add og:image meta tag**

In the `<head>` section, after the existing OG tags (after `og:url`), add:

```html
  <meta property="og:image" content="{{ get_url(path='og.png') }}">
```

The full OG block after edit:

```html
  <!-- Open Graph -->
  <meta property="og:type" content="website">
  <meta property="og:title" content="{% block og_title %}{{ config.title }}{% endblock %}">
  <meta property="og:description" content="{% block og_description %}{{ config.description }}{% endblock %}">
  <meta property="og:url" content="{% block og_url %}{{ config.base_url }}{% endblock %}">
  <meta property="og:image" content="{{ get_url(path='og.png') }}">
```

- [ ] **Step 3: Run `zola check`**

```bash
cd website && zola check
```

Expected: PASS. If `is starting_with(...)` raises a Tera error, fall back to the `containing` test: `{% if current_path is containing('/docs/') %}`.

- [ ] **Step 4: Commit**

```bash
git add website/templates/base.html
git commit -m "feat(website): add Docs/Install nav links and og:image to base template"
```

---

## Task 14: Add docs CSS to `website/static/css/site.css`

Append to the end of `website/static/css/site.css`. Do not modify existing rules.

**Files:**
- Modify: `website/static/css/site.css`

- [ ] **Step 1: Append docs styles**

```css
/* ========== Syntax highlight override ========== */

/* base16-ocean-dark tokens are always dark regardless of theme.
   Override the pre container to use design tokens. */
pre {
  background: var(--bg-raised);
  border: 1px solid var(--border-subtle);
  border-radius: 3px;
  padding: 14px 16px;
  overflow-x: auto;
  line-height: 1.5;
  font-size: 12px;
}

/* ========== GFM alert styles ========== */

.markdown-alert-note,
.markdown-alert-tip,
.markdown-alert-important,
.markdown-alert-warning,
.markdown-alert-caution {
  border-left: 3px solid;
  border-radius: 0 3px 3px 0;
  padding: 10px 14px;
  margin: 16px 0;
}

.markdown-alert-note {
  background: var(--color-info-bg);
  border-color: var(--color-info-border);
}

.markdown-alert-tip {
  background: var(--color-success-bg);
  border-color: var(--color-success-border);
}

.markdown-alert-important {
  background: var(--color-warning-bg);
  border-color: var(--color-warning-border);
}

.markdown-alert-warning {
  background: var(--color-warning-bg);
  border-color: var(--color-warning-border);
}

.markdown-alert-caution {
  background: var(--color-danger-bg);
  border-color: var(--color-danger-border);
}

/* ========== Docs layout ========== */

.docs-layout {
  display: flex;
  align-items: flex-start;
  width: 100%;
  max-width: 1200px;
  margin: 0 auto;
  padding: 0;
  flex: 1 0 auto;
}

/* Override the default .content centering for docs pages */
main.content:has(.docs-layout) {
  max-width: none;
  padding: 0;
}

/* ---- Sidebar ---- */

.docs-sidebar {
  flex: 0 0 240px;
  width: 240px;
  min-height: calc(100vh - 52px);
  border-right: 1px solid var(--border-subtle);
  padding: 20px 0 40px;
  position: sticky;
  top: 52px; /* topbar height */
  max-height: calc(100vh - 52px);
  overflow-y: auto;
  background: var(--bg-base);
}

.docs-sidebar__search {
  padding: 0 16px 12px;
}

.docs-sidebar__nav {
  padding: 0 8px;
}

.sidebar-section {
  margin-bottom: 20px;
}

.sidebar-section__title {
  display: block;
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: var(--text-muted);
  padding: 0 8px 6px;
  text-decoration: none;
}

.sidebar-section__title:hover {
  color: var(--text-secondary);
}

.sidebar-pages {
  list-style: none;
  margin: 0;
  padding: 0;
}

.sidebar-pages--nested {
  padding-left: 12px;
}

.sidebar-page {
  display: block;
  padding: 4px 8px;
  border-radius: 3px;
  font-size: 13px;
  color: var(--text-secondary);
  text-decoration: none;
  line-height: 1.4;
}

.sidebar-page:hover {
  color: var(--text-primary);
  background: var(--bg-hover);
}

.sidebar-page.is-active {
  color: var(--accent);
  background: var(--bg-raised);
  font-weight: 500;
}

.sidebar-nested-title {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px 8px;
  border-radius: 3px;
  font-size: 13px;
  color: var(--text-secondary);
  cursor: pointer;
  list-style: none;
  user-select: none;
}

.sidebar-nested-title:hover {
  color: var(--text-primary);
  background: var(--bg-hover);
}

.sidebar-nested-title a {
  color: inherit;
  text-decoration: none;
}

details:has([data-active]) > .sidebar-nested-title {
  color: var(--accent);
}

details > summary { list-style: none; }
details > summary::-webkit-details-marker { display: none; }

/* ---- Content area ---- */

.docs-content {
  flex: 1 1 auto;
  min-width: 0;
  max-width: 880px;
  padding: 32px 40px 64px;
}

/* ---- Breadcrumbs ---- */

.docs-breadcrumbs {
  font-size: 12px;
  color: var(--text-muted);
  margin-bottom: 16px;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 2px;
}

.docs-breadcrumbs a {
  color: var(--text-secondary);
  text-decoration: none;
}

.docs-breadcrumbs a:hover {
  color: var(--accent);
}

/* ---- Alpha banner ---- */

.docs-alpha-banner {
  background: var(--color-warning-bg);
  border: 1px solid var(--color-warning-border);
  border-radius: 3px;
  padding: 8px 12px;
  font-size: 13px;
  color: var(--text-secondary);
  margin-bottom: 24px;
}

/* ---- Article ---- */

.docs-article h1 {
  margin-top: 0;
  font-size: 26px;
  font-weight: 700;
  color: var(--text-primary);
}

.docs-article h2 { font-size: 18px; font-weight: 600; margin-top: 32px; }
.docs-article h3 { font-size: 15px; font-weight: 600; margin-top: 24px; }

.docs-article p, .docs-article li {
  color: var(--text-secondary);
  line-height: 1.7;
}

.docs-article a { color: var(--accent); }
.docs-article a:hover { color: var(--accent-bright); }

.docs-article table {
  border-collapse: collapse;
  width: 100%;
  margin: 16px 0;
  font-size: 13px;
}

.docs-article th,
.docs-article td {
  text-align: left;
  padding: 8px 12px;
  border: 1px solid var(--border-default);
}

.docs-article th {
  background: var(--bg-raised);
  font-weight: 600;
  color: var(--text-primary);
}

.docs-article code {
  font-family: 'SF Mono', 'Roboto Mono', monospace;
  font-size: 12px;
  background: var(--bg-raised);
  border: 1px solid var(--border-subtle);
  border-radius: 2px;
  padding: 1px 4px;
}

/* ---- Section index page ---- */

.docs-section-index {
  list-style: none;
  padding: 0;
  margin: 16px 0;
}

.docs-section-index li {
  padding: 8px 0;
  border-bottom: 1px solid var(--border-subtle);
  font-size: 14px;
}

.docs-section-index li:last-child { border-bottom: none; }

/* ---- Prev/Next navigation ---- */

.docs-prevnext {
  display: flex;
  justify-content: space-between;
  margin-top: 48px;
  padding-top: 20px;
  border-top: 1px solid var(--border-subtle);
  gap: 16px;
}

.docs-prevnext__prev,
.docs-prevnext__next {
  color: var(--accent);
  text-decoration: none;
  font-size: 13px;
}

.docs-prevnext__next { margin-left: auto; }

.docs-prevnext__prev:hover,
.docs-prevnext__next:hover { color: var(--accent-bright); }

/* ---- Edit link ---- */

.docs-edit-link {
  margin-top: 24px;
  font-size: 12px;
}

.docs-edit-link a {
  color: var(--text-muted);
  text-decoration: none;
}

.docs-edit-link a:hover { color: var(--text-secondary); }

/* ---- Hamburger (mobile only) ---- */

.docs-hamburger {
  display: none;
  position: fixed;
  bottom: 20px;
  right: 20px;
  z-index: 200;
  background: var(--accent);
  color: var(--text-inverted);
  border: none;
  border-radius: 50%;
  width: 44px;
  height: 44px;
  font-size: 18px;
  cursor: pointer;
  align-items: center;
  justify-content: center;
  box-shadow: 0 2px 8px rgba(0,0,0,0.4);
}

.docs-overlay {
  display: none;
  position: fixed;
  inset: 0;
  background: rgba(0,0,0,0.5);
  z-index: 100;
}

@media (max-width: 768px) {
  .docs-layout {
    display: block;
  }

  .docs-sidebar {
    position: fixed;
    top: 0;
    left: 0;
    bottom: 0;
    z-index: 150;
    transform: translateX(-100%);
    transition: transform 0.2s ease;
    max-height: 100vh;
    width: 280px;
    flex: none;
    border-right: 1px solid var(--border-default);
    background: var(--bg-surface);
  }

  .docs-sidebar[data-open] {
    transform: translateX(0);
  }

  .docs-overlay[data-open] {
    display: block;
  }

  .docs-hamburger {
    display: flex;
  }

  .docs-content {
    padding: 20px 16px 64px;
  }
}

/* ---- Topbar active link ---- */

.topbar__link--active {
  color: var(--accent);
}
```

- [ ] **Step 2: Build and visually verify locally**

```bash
cd website && zola build
python3 -m http.server 8080 --directory public
```

Navigate to `http://localhost:8080/docs/end-user/`. Check:
- Two-column layout visible (sidebar left, content right).
- GFM alert blocks styled (if any on first page).
- `pre` blocks use `--bg-raised` background.
- Pagefind widget area renders (widget loads lazily — no WASM until focused).

- [ ] **Step 3: Commit**

```bash
git add website/static/css/site.css
git commit -m "feat(website): add docs layout, sidebar, GFM alert, and syntax highlight CSS"
```

---

## Task 15: Create `website/static/og.png`

1200×630 PNG: dark background, favicon icon at top, wordmark below center.

**Files:**
- Create: `website/static/og.png`

- [ ] **Step 1: Render the OG image**

Requires ImageMagick (`magick` / `convert`). Run from the repository root:

```bash
magick -size 1200x630 xc:'#0F172A' \
  \( website/static/favicon.svg -resize 400x400 \) \
  -gravity None -geometry +400+58 -composite \
  -gravity Center \
  -font 'Roboto-Mono' -pointsize 48 -fill '#e4e4e7' \
  -annotate +0+229 'uptrakit' \
  website/static/og.png
```

Geometry notes:
- Icon: 400×400, top-left at (400, 58) → centered horizontally on 1200px, top edge 58px from top, bottom edge at 458px.
- Wordmark: `-gravity Center` places origin at (600, 315). The wordmark vertical center must be at y=544. Offset from image center: 544 − 315 = 229px down → `-annotate +0+229`.

If `Roboto-Mono` is not in ImageMagick's font list, substitute with an available monospace font:

```bash
magick -list font | grep -i mono
```

Use whatever name appears (e.g. `DejaVu-Sans-Mono`, `Courier`). The exact font name varies by system.

If ImageMagick is unavailable, produce the image using Figma, Inkscape, or any raster editor to match the spec exactly:
- Canvas: 1200×630, background `#0F172A`
- Icon: favicon SVG at 400×400 px, centered horizontally, top edge 58px from top
- Wordmark: "uptrakit", monospace, 48px, weight 500, `#e4e4e7`, horizontally centered, vertical center at 544px from top

- [ ] **Step 2: Verify the output**

```bash
magick identify website/static/og.png
# → website/static/og.png PNG 1200x630 ...
```

Open `website/static/og.png` and visually confirm:
- Dark `#0F172A` background.
- Favicon icon visible at top-center with the slate plate (`#1e293b`) and gradient chevrons.
- "uptrakit" text readable in light color, positioned in the lower half.
- Icon sits in the upper portion of the card (58 px gap above); wordmark sits in the lower half (~86 px gap below). The gap below the wordmark is intentionally larger than the gap above the icon — this matches the spec geometry and is correct.

- [ ] **Step 3: Commit**

```bash
git add website/static/og.png
git commit -m "feat(website): add og.png social card"
```

---

## Task 16: Update `website/templates/install.html`

Change the "canonical deployment reference" link from the GitHub blob URL to the published `/docs/end-user/deployment/` section.

**Files:**
- Modify: `website/templates/install.html`

- [ ] **Step 1: Replace the deployment reference link**

Current (line 66 of `website/templates/install.html`):

```html
<p>
  Full reference:
  <a href="{{ config.extra.github_repo_url }}/blob/main/docs/end-user/deployment/docker.md">docs/end-user/deployment/docker.md</a>
  on GitHub.
</p>
```

Replace with:

```html
<p>
  Full reference: <a href="/docs/end-user/deployment/">Deployment Guides</a>
  — covering all reverse proxy options, enrollment, and profiles.
</p>
```

- [ ] **Step 2: Run `zola check`**

```bash
cd website && zola check
```

Expected: PASS. If the `get_url(path='/docs/end-user/deployment/')` raises "unknown path", the section is not yet registered — verify Task 3 and Task 12 completed correctly.

- [ ] **Step 3: Commit**

```bash
git add website/templates/install.html
git commit -m "feat(website): update install page deployment link to /docs/end-user/deployment/"
```

---

## Task 17: Update `.github/workflows/website.yml`

Extend paths filter, add Pagefind index step, bump size guard to 10 MB.

**Files:**
- Modify: `.github/workflows/website.yml`

- [ ] **Step 1: Apply all three changes**

Replace the current file with:

```yaml
name: website

on:
  push:
    branches: [main]
    paths:
      - 'website/**'
      - 'docs/end-user/**'
      - 'docs/security/**'
      - '.github/workflows/website.yml'
  pull_request:
    paths:
      - 'website/**'
      - 'docs/end-user/**'
      - 'docs/security/**'
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
        # GITHUB_SHA is auto-set by Actions runners; mapping it explicitly here makes
        # the contract with base.html (`get_env(name="GITHUB_SHA")`) self-documenting.
        # zola emits to its default output dir `public/` inside `website/`, so every
        # subsequent step that touches the artifact runs with `working-directory: website`
        # (or references `website/public` from repo root) for consistency.
        run: zola build
        working-directory: website
        env:
          GITHUB_SHA: ${{ github.sha }}

      - name: Build search index
        # pagefind is not in taiki-e/install-action's registry; invoke via npx.
        # Ubuntu runners ship Node 20+, no extra setup step needed.
        # @1 pins the major version; bump is a one-line PR when pagefind v2 warrants it.
        run: npx -y pagefind@1 --site public
        working-directory: website

      - name: Guard artifact size
        working-directory: website
        run: |
          size=$(du -sb public | cut -f1)
          limit=$((10 * 1024 * 1024))
          if [ "$size" -gt "$limit" ]; then
            echo "Artifact size $size exceeds 10MB limit"
            exit 1
          fi

      - uses: actions/upload-pages-artifact@v3
        with:
          path: website/public

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

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/website.yml
git commit -m "ci(website): add docs/ paths trigger, pagefind index step, bump size guard to 10MB"
```

---

## Task 18: Final smoke verification

- [ ] **Step 1: Full local build with Pagefind**

```bash
cd website
zola build
npx -y pagefind@1 --site public
python3 -m http.server 8080 --directory public
```

- [ ] **Step 2: Smoke checklist**

Visit `http://localhost:8080` and verify each item:

| Check | Where | Expected |
| --- | --- | --- |
| Alpha banner present | `/docs/end-user/` | Visible below breadcrumbs |
| Alpha banner absent | `/docs/` | Not shown |
| Sidebar renders | `/docs/end-user/system-overview/` | Section headings + pages listed |
| Deployment group collapsed | `/docs/end-user/system-overview/` | `<details>` closed |
| Deployment group open | `/docs/end-user/deployment/nginx/` | `<details>` open, active page highlighted |
| Breadcrumbs | `/docs/end-user/deployment/nginx/` | `Docs › End-user Guides › Deployment Guides › Nginx` |
| Prev/next links | any leaf page | Previous/Next in weight order |
| No prev/next | `/docs/end-user/` section index | Links absent |
| Edit-on-GitHub | any leaf page | Links to `github.com/.../blob/main/docs/...` |
| Pagefind widget | sidebar header | Search input rendered; WASM not loaded yet |
| Pagefind search | type "update" | Results appear |
| GFM note alert | any page with `[!NOTE]` | Blue-tinted callout |
| GFM warning alert | any page with `[!WARNING]` | Yellow-tinted callout |
| Syntax highlight | any page with code fence | Dark code block, `--bg-raised` background |
| Top bar "Docs" active | `/docs/end-user/` | "Docs" link highlighted |
| Top bar "Install" active | `/install/` | "Install" link highlighted |
| OG image | view source of any page | `<meta property="og:image" content="https://uptrakit.org/og.png">` |
| Install deployment link | `/install/` | "Deployment Guides" link → `/docs/end-user/deployment/` |
| Mobile drawer (320px) | resize browser to 320px | Hamburger button visible; tap opens sidebar |

- [ ] **Step 3: Run pre-merge gates one final time**

```bash
find docs/end-user docs/security -name "*.md" | xargs grep -L "^---" | wc -l
# → 0

grep -r "^> \*\*" docs/end-user docs/security | wc -l
# → 0

cd website && zola check
# → no errors
```

- [ ] **Step 4: Commit any fixes, then open PR**

```bash
git push origin <branch>
# Open PR against main
```

CI will run `zola check`, `zola build`, `npx -y pagefind@1 --site public`, and the 10 MB size guard. All must pass before merge.

---

## Task 19: Update `website/README.md` — Pagefind bump procedure

The spec requires documenting the pagefind version pin alongside the existing Zola bump procedure.

**Files:**
- Modify: `website/README.md`

- [ ] **Step 1: Add Pagefind bump section**

The existing `website/README.md` has a "## Bumping Zola" section (lines 71–79). Add a "## Bumping Pagefind" section directly after it:

```markdown
## Bumping Pagefind

Pagefind is invoked via `npx -y pagefind@1 --site public` in `.github/workflows/website.yml`.
The `@1` pins the major version. Dependabot does not parse this; bump is manual.

To bump Pagefind to a new major version:

1. Check the latest release: <https://github.com/CloudCannon/pagefind/releases>.
2. Edit the `npx -y pagefind@<major>` line in `.github/workflows/website.yml`.
3. Run `npx -y pagefind@<new-major> --site public` locally against a fresh `zola build` output.
4. Confirm the search index builds without errors and the widget loads in a browser.
5. Open a PR.
```

- [ ] **Step 2: Commit**

```bash
git add website/README.md
git commit -m "docs(website): document pagefind version bump procedure"
```
