import { describe, expect, it, vi } from 'vitest';
import { render, fireEvent } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import UpdateAllButton from './UpdateAllButton.svelte';

function noop() {}

describe('UpdateAllButton primitive', () => {
	it('renders <button type="button">', () => {
		const { container } = render(UpdateAllButton, { state: 'idle', onclick: noop });
		const btn = container.querySelector('button');
		expect(btn).not.toBeNull();
		expect(btn?.getAttribute('type')).toBe('button');
	});

	it('renders "↑ Update all" as default children', () => {
		const { container } = render(UpdateAllButton, { state: 'idle', onclick: noop });
		expect(container.querySelector('button')!.textContent).toContain('↑ Update all');
	});

	it('appends " · {count}" when count is provided', () => {
		const { container } = render(UpdateAllButton, { state: 'idle', onclick: noop, count: 7 });
		const text = container
			.querySelector('button')!
			.textContent!.replace(/[\s\u00a0]+/g, ' ')
			.trim();
		expect(text).toBe('↑ Update all · 7');
	});

	it('renders custom children when provided', () => {
		const children = createRawSnippet(() => ({ render: () => '<span>CUSTOM</span>' }));
		const { container } = render(UpdateAllButton, { state: 'idle', onclick: noop, children });
		expect(container.querySelector('button')!.textContent).toContain('CUSTOM');
	});

	it('applies idle-state classes including accent-rgb backgrounds', () => {
		const { container } = render(UpdateAllButton, { state: 'idle', onclick: noop });
		const cls = container.querySelector('button')!.className;
		expect(cls).toContain('bg-[rgba(var(--accent-rgb),0.06)]');
		expect(cls).toContain('border-[rgba(var(--accent-rgb),0.20)]');
		expect(cls).toContain('text-[var(--accent)]');
		expect(cls).toContain('hover:bg-[rgba(var(--accent-rgb),0.18)]');
		expect(cls).toContain('hover:text-[var(--accent-bright)]');
	});

	it('applies dim-state classes including pointer-events-none', () => {
		const { container } = render(UpdateAllButton, { state: 'dim', onclick: noop });
		const cls = container.querySelector('button')!.className;
		expect(cls).toContain('bg-transparent');
		expect(cls).toContain('border-[var(--border-default)]');
		expect(cls).toContain('text-[var(--text-muted)]');
		expect(cls).toContain('pointer-events-none');
	});

	it('sets aria-disabled="true" in dim state', () => {
		const { container } = render(UpdateAllButton, { state: 'dim', onclick: noop });
		expect(container.querySelector('button')!.getAttribute('aria-disabled')).toBe('true');
	});

	it('omits aria-disabled attr in idle state', () => {
		const { container } = render(UpdateAllButton, { state: 'idle', onclick: noop });
		expect(container.querySelector('button')!.hasAttribute('aria-disabled')).toBe(false);
	});

	it('passes ariaLabel through when provided', () => {
		const { container } = render(UpdateAllButton, {
			state: 'dim',
			onclick: noop,
			ariaLabel: 'No updates available'
		});
		expect(container.querySelector('button')!.getAttribute('aria-label')).toBe('No updates available');
	});

	it('does not fire onclick in dim state under pointer activation', async () => {
		const onclick = vi.fn();
		const { container } = render(UpdateAllButton, { state: 'dim', onclick });
		await fireEvent.click(container.querySelector('button')!);
		expect(onclick).not.toHaveBeenCalled();
	});

	it('preventDefaults Enter/Space keydown in dim state', () => {
		const { container } = render(UpdateAllButton, { state: 'dim', onclick: noop });
		const btn = container.querySelector('button')!;
		const enter = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true });
		btn.dispatchEvent(enter);
		expect(enter.defaultPrevented).toBe(true);
		const space = new KeyboardEvent('keydown', { key: ' ', bubbles: true, cancelable: true });
		btn.dispatchEvent(space);
		expect(space.defaultPrevented).toBe(true);
	});

	it('fires onclick in idle state', async () => {
		const onclick = vi.fn();
		const { container } = render(UpdateAllButton, { state: 'idle', onclick });
		await fireEvent.click(container.querySelector('button')!);
		expect(onclick).toHaveBeenCalledTimes(1);
	});

	it('concatenates consumer class after internal classes', () => {
		const { container } = render(UpdateAllButton, {
			state: 'idle',
			onclick: noop,
			class: 'extra-marker'
		});
		const cls = container.querySelector('button')!.className;
		expect(cls).toContain('extra-marker');
		expect(cls).toContain('h-[19px]');
	});
});
