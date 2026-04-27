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

	it('does not render javascript: links from markdown syntax', () => {
		const { container } = render(ReleaseNotes, {
			props: { content: '[click](javascript:void(0))' }
		});
		// markdown-it's default validateLink blocks javascript: — no <a> rendered
		expect(container.querySelector('a')).toBeNull();
	});

	it('strips javascript: hrefs from raw HTML input', () => {
		const { container } = render(ReleaseNotes, {
			props: { content: '<a href="javascript:void(0)">click</a>' }
		});
		const link = container.querySelector('a');
		// DOMPurify removes or sanitizes javascript: href
		expect(link).not.toBeNull();
		const href = link!.getAttribute('href');
		expect(href === null || href === '' || !href.match(/javascript:/i)).toBe(true);
	});

	it('removes unallowed tags entirely (img not in allowlist)', () => {
		const { container } = render(ReleaseNotes, {
			content: '<img onerror="alert(1)" src="x">'
		});
		expect(container.querySelector('img')).not.toBeInTheDocument();
	});

	it('strips event handler attributes from allowed tags', () => {
		const { container } = render(ReleaseNotes, {
			content: '<p onclick="alert(1)">safe text</p>'
		});
		const p = container.querySelector('p');
		expect(p).toBeInTheDocument();
		expect(p?.getAttribute('onclick')).toBeNull();
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
