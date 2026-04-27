<script lang="ts">
	import markdownit from 'markdown-it';
	import taskLists from 'markdown-it-task-lists';
	import githubAlerts from 'markdown-it-github-alerts';
	import footnote from 'markdown-it-footnote';
	import { full as emoji } from 'markdown-it-emoji';
	import DOMPurify, { type Config } from 'dompurify';

	let { content, compact = false }: { content: string; compact?: boolean } = $props();

	const md = markdownit({ html: true, linkify: true });
	// Render ~~text~~ as <del> (GFM) instead of markdown-it's default <s>
	md.renderer.rules.s_open = () => '<del>';
	md.renderer.rules.s_close = () => '</del>';
	md.use(taskLists);
	// GitHub-style alerts (> [!NOTE], [!TIP], [!IMPORTANT], [!WARNING], [!CAUTION]).
	// Disable embedded SVG icons so DOMPurify allowlist stays free of svg/path tags;
	// titles fall back to plain text and are styled via CSS.
	md.use(githubAlerts, {
		icons: { NOTE: '', TIP: '', IMPORTANT: '', WARNING: '', CAUTION: '' }
	});
	md.use(footnote);
	md.use(emoji);

	const ALLOW_LIST: Config = {
		ALLOWED_TAGS: [
			'h1',
			'h2',
			'h3',
			'h4',
			'h5',
			'h6',
			'p',
			'ul',
			'ol',
			'li',
			'pre',
			'code',
			'blockquote',
			'table',
			'thead',
			'tbody',
			'tr',
			'th',
			'td',
			'del',
			'input',
			'hr',
			'a',
			'strong',
			'em',
			'br',
			'span',
			'div',
			'sup',
			'section'
		],
		ALLOWED_ATTR: ['href', 'rel', 'checked', 'disabled', 'type', 'class', 'id', 'dir']
	};

	// sanitize() returns string | TrustedHTML; cast to string for {@html}
	const rendered = $derived(DOMPurify.sanitize(md.render(content), ALLOW_LIST) as string);
</script>

<div class="release-notes" class:compact>
	<!-- eslint-disable-next-line svelte/no-at-html-tags -->
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

	.release-notes :global(ul) {
		list-style-type: disc;
		padding-left: 1.25rem;
		margin: 4px 0;
	}

	.release-notes :global(ul ul) {
		list-style-type: circle;
	}

	.release-notes :global(ul ul ul) {
		list-style-type: square;
	}

	.release-notes :global(ol) {
		list-style-type: decimal;
		padding-left: 1.25rem;
		margin: 4px 0;
	}

	.release-notes :global(ol ol) {
		list-style-type: lower-alpha;
	}

	.release-notes :global(ol ol ol) {
		list-style-type: lower-roman;
	}

	.release-notes :global(hr) {
		border: none;
		border-top: 1px solid var(--border-subtle);
		margin: 12px 0;
	}

	/* GitHub-style alert blocks (> [!NOTE] / [!TIP] / [!IMPORTANT] / [!WARNING] / [!CAUTION]) */
	.release-notes :global(.markdown-alert) {
		border-left: 3px solid var(--color-info-border);
		background: var(--color-info-bg);
		border-radius: 4px;
		padding: 8px 12px;
		margin: 8px 0;
	}

	.release-notes :global(.markdown-alert > p) {
		margin: 0;
		font-style: normal;
		color: var(--text-primary);
	}

	.release-notes :global(.markdown-alert > p + p) {
		margin-top: 4px;
	}

	.release-notes :global(.markdown-alert-title) {
		font-weight: 700;
		font-size: 0.75rem;
		text-transform: uppercase;
		letter-spacing: 0.04em;
		color: var(--color-info);
	}

	.release-notes :global(.markdown-alert-note) {
		border-left-color: var(--color-info-border);
		background: var(--color-info-bg);
	}

	.release-notes :global(.markdown-alert-note .markdown-alert-title) {
		color: var(--color-info);
	}

	.release-notes :global(.markdown-alert-tip) {
		border-left-color: var(--color-success-border);
		background: var(--color-success-bg);
	}

	.release-notes :global(.markdown-alert-tip .markdown-alert-title) {
		color: var(--color-success);
	}

	.release-notes :global(.markdown-alert-important) {
		border-left-color: var(--accent);
		background: rgba(var(--accent-rgb), 0.1);
	}

	.release-notes :global(.markdown-alert-important .markdown-alert-title) {
		color: var(--accent);
	}

	.release-notes :global(.markdown-alert-warning) {
		border-left-color: var(--color-warning-border);
		background: var(--color-warning-bg);
	}

	.release-notes :global(.markdown-alert-warning .markdown-alert-title) {
		color: var(--color-warning);
	}

	.release-notes :global(.markdown-alert-caution) {
		border-left-color: var(--color-danger-border);
		background: var(--color-danger-bg);
	}

	.release-notes :global(.markdown-alert-caution .markdown-alert-title) {
		color: var(--color-danger);
	}

	/* Footnotes */
	.release-notes :global(sup.footnote-ref) {
		font-size: 0.6875rem;
	}

	.release-notes :global(sup.footnote-ref a) {
		text-decoration: none;
	}

	.release-notes :global(section.footnotes) {
		margin-top: 16px;
		padding-top: 8px;
		border-top: 1px solid var(--border-subtle);
		font-size: 0.75rem;
		color: var(--text-muted);
	}

	.release-notes :global(section.footnotes ol) {
		padding-left: 1.25rem;
	}

	.release-notes :global(.footnote-backref) {
		text-decoration: none;
		margin-left: 4px;
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
