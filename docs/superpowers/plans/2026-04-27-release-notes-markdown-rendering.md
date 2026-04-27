# Release Notes Markdown Rendering Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render release notes as markdown in all three modals (Confirm Update ×2, standalone
Release Notes) and widen those modals from their defaults to `max-w-3xl`.

**Architecture:** New `ReleaseNotes.svelte` component wraps a markdown-it→DOMPurify pipeline
and exposes a `compact` prop for reduced spacing inside `<details>` collapsibles. All three
modal sites import it from the shared `ui` barrel and replace their `<pre>` tags.

**Tech Stack:** `markdown-it@^14`, `markdown-it-task-lists@^2`, `dompurify@^3`,
`@types/markdown-it@^14`, Svelte 5, Tailwind CSS v4, Vitest + Testing Library

---

## File Map

| Action | Path |
| ------ | ---- |
| Create | `frontend/src/types/markdown-it-task-lists.d.ts` |
| Create | `frontend/src/lib/components/ReleaseNotes.svelte` |
| Create | `frontend/src/lib/components/ReleaseNotes.test.ts` |
| Modify | `frontend/src/lib/components/ui/index.ts` |
| Modify | `frontend/src/routes/software/[id]/+page.svelte` |
| Modify | `frontend/src/routes/software/+page.svelte` |

---

## Task 1: Install Dependencies and Create Type Stub

**Files:**

- Modify: `frontend/package.json`
- Create: `frontend/src/types/markdown-it-task-lists.d.ts`

- [ ] **Step 1.1: Install packages**

```bash
cd frontend
npm install markdown-it@^14 markdown-it-task-lists@^2 dompurify@^3
npm install --save-dev @types/markdown-it@^14
```

Expected: packages appear in `node_modules/`, `package.json` updated.

- [ ] **Step 1.2: Verify markdown-it-task-lists loads on markdown-it 14**

```bash
cd frontend
node -e "
  const md = require('markdown-it')({ html: true });
  const tl = require('markdown-it-task-lists');
  md.use(tl);
  const out = md.render('- [x] done\n- [ ] todo\n');
  console.log(out);
  if (!out.includes('input')) process.exit(1);
  console.log('OK');
"
```

Expected output includes `<input` and prints `OK`. If it throws or exits 1,
`markdown-it-task-lists` is incompatible with md v14 — stop and report to user
before continuing. The fallback is to skip the `.use(taskLists)` line and omit
task list support.

- [ ] **Step 1.3: Create the type stub**

Create `frontend/src/types/markdown-it-task-lists.d.ts`:

```typescript
declare module 'markdown-it-task-lists' {
	import type MarkdownIt from 'markdown-it';
	const plugin: (md: MarkdownIt, options?: { enabled?: boolean; label?: boolean; labelAfter?: boolean }) => void;
	export default plugin;
}
```

- [ ] **Step 1.4: Verify TypeScript picks up the stub**

```bash
cd frontend
npx svelte-check --tsconfig ./tsconfig.json 2>&1 | grep -i "task-lists" || echo "no errors for task-lists"
```

Expected: no TypeScript error mentioning `markdown-it-task-lists`.

- [ ] **Step 1.5: Commit**

```bash
git add frontend/package.json frontend/package-lock.json \
        frontend/src/types/markdown-it-task-lists.d.ts
git commit -m "chore(frontend): add markdown-it, dompurify, and task-lists deps"
```

---

## Task 2: Build `ReleaseNotes.svelte` with TDD

**Files:**

- Create: `frontend/src/lib/components/ReleaseNotes.test.ts`
- Create: `frontend/src/lib/components/ReleaseNotes.svelte`

### Step 2.1 — Write failing tests

- [ ] **Step 2.1: Create `ReleaseNotes.test.ts`**

```typescript
import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render } from '@testing-library/svelte';
import ReleaseNotes from './ReleaseNotes.svelte';

afterEach(cleanup);

describe('ReleaseNotes', () => {
	it('renders markdown headings as h2/h3 elements', () => {
		const { container } = render(ReleaseNotes, { content: '## Heading\n### Sub\n' });
		expect(container.querySelector('h2')).toBeInTheDocument();
		expect(container.querySelector('h3')).toBeInTheDocument();
	});

	it('renders bold and italic inline formatting', () => {
		const { container } = render(ReleaseNotes, { content: '**bold** and _italic_' });
		expect(container.querySelector('strong')).toBeInTheDocument();
		expect(container.querySelector('em')).toBeInTheDocument();
	});

	it('renders unordered lists', () => {
		const { container } = render(ReleaseNotes, { content: '- item one\n- item two\n' });
		const items = container.querySelectorAll('li');
		expect(items).toHaveLength(2);
	});

	it('renders strikethrough', () => {
		const { container } = render(ReleaseNotes, { content: '~~removed~~' });
		expect(container.querySelector('del')).toBeInTheDocument();
	});

	it('renders inline code', () => {
		const { container } = render(ReleaseNotes, { content: 'run `npm install`' });
		expect(container.querySelector('code')).toBeInTheDocument();
	});

	it('renders fenced code blocks as pre > code', () => {
		const { container } = render(ReleaseNotes, { content: '```\necho hello\n```\n' });
		expect(container.querySelector('pre > code')).toBeInTheDocument();
	});

	it('renders plain text as a paragraph', () => {
		const { container } = render(ReleaseNotes, { content: 'just plain text' });
		expect(container.querySelector('p')).toBeInTheDocument();
		expect(container.querySelector('p')!.textContent).toContain('just plain text');
	});

	it('renders raw HTML input (sanitized)', () => {
		const { container } = render(ReleaseNotes, {
			content: '<p>raw <strong>html</strong></p>'
		});
		expect(container.querySelector('strong')).toBeInTheDocument();
	});

	it('strips script tags from raw HTML input', () => {
		const { container } = render(ReleaseNotes, {
			content: '<script>alert(1)</script><p>safe</p>'
		});
		expect(container.querySelector('script')).not.toBeInTheDocument();
		expect(container.querySelector('p')!.textContent).toContain('safe');
	});

	it('strips javascript: hrefs', () => {
		const { container } = render(ReleaseNotes, {
			content: '[click](javascript:alert(1))'
		});
		const link = container.querySelector('a');
		expect(link?.getAttribute('href')).not.toMatch(/^javascript:/i);
	});

	it('strips event handler attributes', () => {
		const { container } = render(ReleaseNotes, {
			content: '<img onerror="alert(1)" src="x">'
		});
		const img = container.querySelector('img');
		expect(img).not.toBeInTheDocument();
	});

	it('renders task list checkboxes as disabled inputs', () => {
		const { container } = render(ReleaseNotes, {
			content: '- [x] done\n- [ ] todo\n'
		});
		const checkboxes = container.querySelectorAll('input[type="checkbox"]');
		expect(checkboxes).toHaveLength(2);
		checkboxes.forEach((cb) => {
			expect(cb).toHaveAttribute('disabled');
		});
	});

	it('renders GFM tables', () => {
		const { container } = render(ReleaseNotes, {
			content: '| A | B |\n|---|---|\n| 1 | 2 |\n'
		});
		expect(container.querySelector('table')).toBeInTheDocument();
		expect(container.querySelector('thead')).toBeInTheDocument();
	});

	it('auto-links bare URLs when linkify is true', () => {
		const { container } = render(ReleaseNotes, {
			content: 'See https://example.com for details'
		});
		const link = container.querySelector('a[href="https://example.com"]');
		expect(link).toBeInTheDocument();
	});

	it('applies release-notes class to wrapper div', () => {
		const { container } = render(ReleaseNotes, { content: 'text' });
		expect(container.querySelector('.release-notes')).toBeInTheDocument();
	});

	it('applies compact class when compact prop is true', () => {
		const { container } = render(ReleaseNotes, { content: 'text', compact: true });
		expect(container.querySelector('.release-notes.compact')).toBeInTheDocument();
	});

	it('does not apply compact class by default', () => {
		const { container } = render(ReleaseNotes, { content: 'text' });
		expect(container.querySelector('.release-notes.compact')).not.toBeInTheDocument();
	});
});
```

- [ ] **Step 2.2: Run tests — verify they all fail**

```bash
cd frontend
npx vitest run src/lib/components/ReleaseNotes.test.ts
```

Expected: all tests fail with "Cannot find module './ReleaseNotes.svelte'" or similar.

### Step 2.3 — Implement the component

- [ ] **Step 2.3: Create `ReleaseNotes.svelte`**

```svelte
<script lang="ts">
	import markdownit from 'markdown-it';
	import taskLists from 'markdown-it-task-lists';
	import DOMPurify from 'dompurify';

	let { content, compact = false }: { content: string; compact?: boolean } = $props();

	const md = markdownit({ html: true, linkify: true }).use(taskLists);

	const ALLOW_LIST: DOMPurify.Config = {
		ALLOWED_TAGS: [
			'h1', 'h2', 'h3', 'h4', 'h5', 'h6',
			'p', 'ul', 'ol', 'li',
			'pre', 'code',
			'blockquote',
			'table', 'thead', 'tbody', 'tr', 'th', 'td',
			'del', 'input', 'hr',
			'a', 'strong', 'em', 'br', 'span'
		],
		ALLOWED_ATTR: ['href', 'target', 'rel', 'checked', 'disabled', 'type', 'class']
	};

	const rendered = $derived(DOMPurify.sanitize(md.render(content), ALLOW_LIST));
</script>

<div class="release-notes" class:compact>
	{@html rendered}
</div>

<style>
	.release-notes :global(p) {
		font-size: 0.8125rem;
		color: var(--text-primary);
		line-height: 1.6;
		margin-bottom: 8px;
		margin-top: 0;
	}

	.release-notes :global(li) {
		font-size: 0.8125rem;
		color: var(--text-primary);
		line-height: 1.6;
		margin-bottom: 2px;
	}

	.release-notes :global(h1) {
		font-size: 1.125rem; /* 18px */
		font-weight: 700;
		color: var(--text-primary);
		margin-top: 12px;
		margin-bottom: 4px;
	}

	.release-notes :global(h2) {
		font-size: 0.9375rem; /* 15px */
		font-weight: 700;
		color: var(--text-primary);
		margin-top: 12px;
		margin-bottom: 4px;
	}

	.release-notes :global(h3) {
		font-size: 0.8125rem; /* 13px */
		font-weight: 700;
		color: var(--text-primary);
		margin-top: 12px;
		margin-bottom: 4px;
	}

	.release-notes :global(code) {
		background: var(--bg-surface);
		border-radius: 4px;
		padding: 1px 4px;
		font-family: monospace;
		font-size: 0.75rem; /* 12px */
	}

	.release-notes :global(pre) {
		background: var(--bg-surface);
		border-radius: 6px;
		padding: 10px;
		overflow-x: auto;
		margin: 8px 0;
	}

	.release-notes :global(pre code) {
		background: none;
		padding: 0;
		font-size: 0.75rem;
	}

	.release-notes :global(blockquote) {
		border-left: 3px solid var(--border-subtle);
		margin: 8px 0;
		padding-left: 12px;
		color: var(--text-muted);
		font-style: italic;
	}

	.release-notes :global(a) {
		color: var(--accent);
		text-decoration: none;
	}

	.release-notes :global(a:hover) {
		text-decoration: underline;
	}

	.release-notes :global(table) {
		border-collapse: collapse;
		width: 100%;
		font-size: 0.8125rem;
		margin: 8px 0;
	}

	.release-notes :global(th),
	.release-notes :global(td) {
		border: 1px solid var(--border-subtle);
		padding: 6px 10px;
		color: var(--text-primary);
	}

	.release-notes :global(th) {
		font-weight: 700;
	}

	.release-notes :global(input[type='checkbox']) {
		pointer-events: none;
		margin-right: 4px;
	}

	.release-notes :global(ul),
	.release-notes :global(ol) {
		padding-left: 1.25rem;
		margin: 4px 0;
	}

	.release-notes :global(hr) {
		border: none;
		border-top: 1px solid var(--border-subtle);
		margin: 12px 0;
	}

	/* compact mode — for use inside <details> collapsibles */
	.release-notes.compact :global(h1) {
		font-size: 0.8125rem; /* 13px */
		margin-top: 6px;
	}

	.release-notes.compact :global(h2) {
		font-size: 0.75rem; /* 12px */
		margin-top: 6px;
	}

	.release-notes.compact :global(h3) {
		font-size: 0.6875rem; /* 11px */
		margin-top: 6px;
	}

	.release-notes.compact :global(p) {
		margin-bottom: 4px;
	}

	.release-notes.compact :global(li) {
		margin-bottom: 1px;
	}
</style>
```

- [ ] **Step 2.4: Run tests — verify all pass**

```bash
cd frontend
npx vitest run src/lib/components/ReleaseNotes.test.ts
```

Expected: all tests pass.

- [ ] **Step 2.5: Commit**

```bash
git add frontend/src/lib/components/ReleaseNotes.svelte \
        frontend/src/lib/components/ReleaseNotes.test.ts
git commit -m "feat(frontend): add ReleaseNotes markdown rendering component"
```

---

## Task 3: Export and Integrate — `software/[id]/+page.svelte`

**Files:**

- Modify: `frontend/src/lib/components/ui/index.ts`
- Modify: `frontend/src/routes/software/[id]/+page.svelte`

- [ ] **Step 3.1: Add export to the ui barrel**

In `frontend/src/lib/components/ui/index.ts`, add after line 18
(`export { default as ContextMenuShell } from '../ContextMenu.svelte';`):

```typescript
export { default as ReleaseNotes } from '../ReleaseNotes.svelte';
```

- [ ] **Step 3.2: Import ReleaseNotes in `[id]/+page.svelte`**

In `frontend/src/routes/software/[id]/+page.svelte`, the import block starting at line 43
imports named exports from `$lib/components/ui`. Add `ReleaseNotes` to that destructured
import. The block currently looks like:

```svelte
import {
    ActionBadge,
    Callout,
    ContextMenuItem,
    ContextMenuShell,
    DataTable,
    FormFieldRow,
    ModalShell,
```

Add `ReleaseNotes,` in alphabetical order:

```svelte
import {
    ActionBadge,
    Callout,
    ContextMenuItem,
    ContextMenuShell,
    DataTable,
    FormFieldRow,
    ModalShell,
    ReleaseNotes,
```

- [ ] **Step 3.3: Widen `updateModal` ModalShell**

At line 1037, change:

```svelte
	<ModalShell title="Confirm Update" onclose={() => (updateModal = null)}>
```

to:

```svelte
	<ModalShell title="Confirm Update" onclose={() => (updateModal = null)} maxWidth="max-w-3xl">
```

- [ ] **Step 3.4: Replace `<pre>` in `updateModal` release notes**

At lines 1078–1079, replace:

```svelte
					<pre
						class="mt-2 max-h-48 overflow-y-auto whitespace-pre-wrap text-table-body text-[var(--text-primary)] font-mono">{meta.release_notes}</pre>
```

with:

```svelte
					<div class="mt-2 max-h-48 overflow-y-auto">
						<ReleaseNotes content={meta.release_notes} compact />
					</div>
```

- [ ] **Step 3.5: Widen `releaseNotesModal` ModalShell and replace `<pre>`**

At line 1103, change `maxWidth="max-w-2xl"` to `maxWidth="max-w-3xl"`:

```svelte
	<ModalShell onclose={() => (releaseNotesModal = null)} maxWidth="max-w-3xl">
```

At lines 1128–1130, replace:

```svelte
				<pre
					class="whitespace-pre-wrap text-table-body text-[var(--text-primary)] font-mono leading-relaxed">{releaseNotesModal
						.meta.release_notes}</pre>
```

with:

```svelte
				<ReleaseNotes content={releaseNotesModal.meta.release_notes} />
```

- [ ] **Step 3.6: Run the type check**

```bash
cd frontend
npx svelte-check --tsconfig ./tsconfig.json 2>&1 | tail -5
```

Expected: `0 errors` in the summary line.

- [ ] **Step 3.7: Run the full test suite**

```bash
cd frontend
npx vitest run
```

Expected: all tests pass (no regressions).

- [ ] **Step 3.8: Commit**

```bash
git add frontend/src/lib/components/ui/index.ts \
        frontend/src/routes/software/[id]/+page.svelte
git commit -m "feat(frontend): render release notes as markdown in [id] modals, widen to max-w-3xl"
```

---

## Task 4: Integrate — `software/+page.svelte`

**Files:**

- Modify: `frontend/src/routes/software/+page.svelte`

- [ ] **Step 4.1: Import ReleaseNotes in `+page.svelte`**

In `frontend/src/routes/software/+page.svelte`, find the import block at lines 60–72
that imports named exports from `$lib/components/ui`:

```svelte
import {
    Callout,
    ContextMenuItem,
    ContextMenuShell,
    EmptyState,
    FormFieldRow,
    ModalShell,
    PageShell,
    SectionCard,
    SoftwareGroupList,
    StatusBadge,
    TabStrip,
    type TabStripItem
} from '$lib/components/ui';
```

Add `ReleaseNotes,` in alphabetical position:

```svelte
import {
    Callout,
    ContextMenuItem,
    ContextMenuShell,
    EmptyState,
    FormFieldRow,
    ModalShell,
    PageShell,
    ReleaseNotes,
    SectionCard,
    SoftwareGroupList,
    StatusBadge,
    TabStrip,
    type TabStripItem
} from '$lib/components/ui';
```

- [ ] **Step 4.2: Widen `singleHostUpdateModal` ModalShell**

At line 1340, change:

```svelte
	<ModalShell title="Confirm Update" onclose={() => (singleHostUpdateModal = null)}>
```

to:

```svelte
	<ModalShell title="Confirm Update" onclose={() => (singleHostUpdateModal = null)} maxWidth="max-w-3xl">
```

- [ ] **Step 4.3: Replace `<pre>` in `singleHostUpdateModal` release notes**

At lines 1385–1386, replace:

```svelte
				<pre
					class="mt-2 max-h-48 overflow-y-auto whitespace-pre-wrap text-table-body text-[var(--text-primary)] font-mono">{meta.release_notes}</pre>
```

with:

```svelte
				<div class="mt-2 max-h-48 overflow-y-auto">
					<ReleaseNotes content={meta.release_notes} compact />
				</div>
```

- [ ] **Step 4.4: Run the type check**

```bash
cd frontend
npx svelte-check --tsconfig ./tsconfig.json 2>&1 | tail -5
```

Expected: `0 errors`.

- [ ] **Step 4.5: Run the full test suite**

```bash
cd frontend
npx vitest run
```

Expected: all tests pass.

- [ ] **Step 4.6: Commit**

```bash
git add frontend/src/routes/software/+page.svelte
git commit -m "feat(frontend): render release notes as markdown in software list modal, widen to max-w-3xl"
```

---

## Task 5: Quality Gates

- [ ] **Step 5.1: Lint and format check**

```bash
cd frontend
npm run lint && npm run format:check
```

Expected: no errors, no unformatted files.

- [ ] **Step 5.2: Full type check**

```bash
cd frontend
npm run check
```

Expected: `0 errors, 0 warnings`.

- [ ] **Step 5.3: Full build**

```bash
cd frontend
npm run build
```

Expected: build succeeds with no errors.

- [ ] **Step 5.4: Manual smoke test**

Start the dev server. Open a software item that has a host with a `latest_release_metadata`
containing `release_notes`. Verify:

1. Confirm Update dialogue is `max-w-3xl` wide (wider than before).
2. "Release notes" `<details>` section expands and renders markdown (headings, bullets,
   code blocks) — not a raw `<pre>` dump.
3. Standalone Release Notes modal (if accessible) also renders markdown.
4. No JavaScript errors in browser console.
5. Confirm Update dialogue still functions — "Trigger Update" button works.
