# Release Notes Markdown Rendering

**Date:** 2026-04-27
**Status:** Approved

## Goal

Make the Confirm Update dialogue wider and render release notes as markdown
(supporting markdown, raw HTML, and plain text inputs) in both the Confirm Update
dialogue and the standalone Release Notes modal.

## Scope

Two routes are affected:

- `frontend/src/routes/software/[id]/+page.svelte` — contains `updateModal` (Confirm Update) and `releaseNotesModal` (standalone)
- `frontend/src/routes/software/+page.svelte` — contains `singleHostUpdateModal` (Confirm Update)

## Width Change

All three modals gain `maxWidth="max-w-3xl"` (768px, responsive):

- `updateModal` ModalShell: no current `maxWidth` → add `maxWidth="max-w-3xl"`
- `singleHostUpdateModal` ModalShell: no current `maxWidth` → add `maxWidth="max-w-3xl"`
- `releaseNotesModal` ModalShell: currently `max-w-2xl` → bump to `max-w-3xl`

## New Component: `ReleaseNotes.svelte`

**Location:** `frontend/src/lib/components/ReleaseNotes.svelte`

**Props:**

```typescript
{
  content: string;      // raw release notes string (markdown, HTML, or plain text)
  compact?: boolean;    // true inside <details> collapsibles (max-h-48 context)
}
```

**Rendering pipeline:**

1. `md.render(content)` — markdown-it with `html: true`, `linkify: true`, default preset
   (tables + strikethrough built-in), plus `markdown-it-task-lists` plugin
2. `DOMPurify.sanitize(html, ALLOW_LIST)` — strips XSS, retains prose structure
3. `{@html sanitized}` — Svelte HTML injection

The `html: true` option passes raw HTML blocks through markdown-it unchanged, satisfying
the "handle raw HTML" requirement. DOMPurify then sanitizes the combined output regardless
of input type. Plain text is valid markdown and renders as paragraphs.

**SSR safety:** DOMPurify is browser-only. The component uses `$derived` with a
`typeof window !== 'undefined'` guard — during SSR the raw content string is used as
fallback. The modals are never server-rendered in practice, so this is purely defensive.

**DOMPurify allowlist:**

Tags: `h1 h2 h3 h4 h5 h6 p ul ol li pre code blockquote table thead tbody tr th td del input hr a strong em br`

Attributes:

- `href` on `a` (external links in release notes)
- `checked`, `disabled` on `input` (task list checkboxes)
- `class` on `li` (markdown-it-task-lists adds `task-list-item`)

Blocked: all event handlers, `javascript:` hrefs, `data:` URIs, `style` attributes.

**Styling:** Scoped `<style>` block using design-system CSS variables. No Tailwind typography plugin.

- **`p`, `li`** — 13px, `var(--text-primary)`, `line-height: 1.6`
- **`h1`–`h3`** — `font-weight: 700`, `var(--text-primary)`, sizes 18/15/13px
- **`code` (inline)** — `var(--bg-surface)` background, `border-radius: 4px`, monospace 12px
- **`pre`** — `var(--bg-surface)` background, `border-radius: 6px`, `overflow-x: auto`
- **`blockquote`** — `border-left: 3px solid var(--border-subtle)`, muted text
- **`a`** — `var(--accent)` color, underline on hover
- **`table`** — `border-collapse: collapse`, `var(--border-subtle)` cell borders
- **`input[type=checkbox]`** — `pointer-events: none` (read-only task list items)

`compact` mode: heading sizes reduced one step, vertical margins halved.

**Export:** Added to `frontend/src/lib/components/ui/index.ts`.

## Dependencies

Added to `frontend/package.json` `dependencies`:

```json
"markdown-it": "^14.x",
"markdown-it-task-lists": "^2.x",
"dompurify": "^3.x"
```

Added to `devDependencies`:

```json
"@types/dompurify": "^3.x"
```

`markdown-it` ships its own TypeScript types — no `@types/markdown-it` needed.

## Touch Points

### `software/[id]/+page.svelte`

**`updateModal` (Confirm Update):**

- ModalShell: add `maxWidth="max-w-3xl"`
- Replace `<pre class="mt-2 max-h-48 ... font-mono">{meta.release_notes}</pre>`
  with `<ReleaseNotes content={meta.release_notes} compact />`

**`releaseNotesModal` (standalone):**

- ModalShell: change `maxWidth="max-w-2xl"` → `maxWidth="max-w-3xl"`
- Replace `<pre class="whitespace-pre-wrap ... font-mono">...</pre>`
  with `<ReleaseNotes content={releaseNotesModal.meta.release_notes} />`

### `software/+page.svelte`

**`singleHostUpdateModal` (Confirm Update):**

- ModalShell: add `maxWidth="max-w-3xl"`
- Replace `<pre class="mt-2 max-h-48 ... font-mono">{meta.release_notes}</pre>`
  with `<ReleaseNotes content={meta.release_notes} compact />`

## Out of Scope

- Syntax highlighting (Shiki) — deferred, own decision
- Markdown rendering in any other part of the app
- Changes to how `release_notes` is fetched or stored
