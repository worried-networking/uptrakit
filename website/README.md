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

Produce a production-shaped artifact (emits to `website/public/`):

```bash
zola build
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

## Bumping Pagefind

Pagefind is invoked via `npx -y pagefind@1 --site public` in `.github/workflows/website.yml`.
The `@1` pins the major version. Dependabot does not parse this; bump is manual.

To bump Pagefind to a new major version:

1. Check the latest release: <https://github.com/CloudCannon/pagefind/releases>.
2. Edit the `npx -y pagefind@<major>` line in `.github/workflows/website.yml`.
3. Run `npx -y pagefind@<new-major> --site public` locally against a fresh `zola build` output.
4. Confirm the search index builds without errors and the widget loads in a browser.
5. Open a PR.

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
