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

	it('renders "Update all" as default children', () => {
		const { container } = render(UpdateAllButton, { state: 'idle', onclick: noop });
		expect(container.querySelector('button')!.textContent).toContain('Update all');
	});

	it('renders custom children when provided', () => {
		const children = createRawSnippet(() => ({ render: () => '<span>CUSTOM</span>' }));
		const { container } = render(UpdateAllButton, { state: 'idle', onclick: noop, children });
		expect(container.querySelector('button')!.textContent).toContain('CUSTOM');
	});

	it('applies idle-state accent classes matching ActionBadge', () => {
		const { container } = render(UpdateAllButton, { state: 'idle', onclick: noop });
		const cls = container.querySelector('button')!.className;
		expect(cls).toContain('bg-[color:rgb(var(--accent-rgb)/0.12)]');
		expect(cls).toContain('text-[var(--accent-bright)]');
		expect(cls).toContain('rounded-badge');
		expect(cls).not.toContain('opacity-40');
	});

	it('applies dim-state classes including pointer-events-none and opacity-40', () => {
		const { container } = render(UpdateAllButton, { state: 'dim', onclick: noop });
		const cls = container.querySelector('button')!.className;
		expect(cls).toContain('opacity-40');
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
		expect(cls).toContain('rounded-badge');
	});
});
