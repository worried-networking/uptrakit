import { describe, expect, it, vi, afterEach } from 'vitest';
import { tick } from 'svelte';
import { render, screen, fireEvent } from '@testing-library/svelte';
import Tooltip from './Tooltip.svelte';

describe('Tooltip', () => {
	it('renders nothing when content is empty string', () => {
		const { container } = render(Tooltip, { content: '' });
		expect(container.querySelector('button')).toBeNull();
		// use:portal appends to document.body — verify no bubble portaled either
		expect(document.body.querySelector('[role="tooltip"]')).toBeNull();
	});

	it('renders trigger button when content is provided', () => {
		render(Tooltip, { content: 'Hello world' });
		expect(screen.getByRole('button', { name: 'More information' })).toBeTruthy();
	});

	it('trigger aria-describedby matches tooltip id', () => {
		render(Tooltip, { content: 'Hello world' });
		const trigger = screen.getByRole('button', { name: 'More information' });
		const tooltipId = trigger.getAttribute('aria-describedby');
		expect(tooltipId).toBeTruthy();
		const tooltipEl = document.getElementById(tooltipId!);
		expect(tooltipEl).toBeTruthy();
		expect(tooltipEl?.getAttribute('role')).toBe('tooltip');
	});

	it('tooltip bubble always in DOM (not hidden with {#if visible})', () => {
		render(Tooltip, { content: 'Hello world' });
		const tooltip = document.querySelector('[role="tooltip"]');
		// bubble present in DOM even when not triggered (CSS-only hiding)
		expect(tooltip).toBeTruthy();
		expect(tooltip?.classList.contains('invisible')).toBe(true);
	});

	it('accepts explicit id prop', () => {
		render(Tooltip, { content: 'Hello', id: 'my-tip' });
		expect(document.getElementById('my-tip')).toBeTruthy();
		expect(document.getElementById('my-tip')?.getAttribute('role')).toBe('tooltip');
		const trigger = screen.getByRole('button', { name: 'More information' });
		expect(trigger.getAttribute('aria-describedby')).toBe('my-tip');
	});

	describe('show/hide behavior', () => {
		afterEach(() => {
			vi.useRealTimers();
		});

		it('bubble is invisible when trigger is not hovered or focused', () => {
			render(Tooltip, { content: 'Hello' });
			const bubble = document.querySelector('[role="tooltip"]')!;
			expect(bubble.classList.contains('invisible')).toBe(true);
		});

		it('bubble becomes visible on trigger mouseenter', async () => {
			vi.useFakeTimers();
			render(Tooltip, { content: 'Hello' });
			const trigger = screen.getByRole('button', { name: 'More information' });
			await fireEvent.mouseEnter(trigger);
			const bubble = document.querySelector('[role="tooltip"]')!;
			expect(bubble.classList.contains('invisible')).toBe(false);
		});

		it('bubble becomes visible on trigger focus', async () => {
			vi.useFakeTimers();
			render(Tooltip, { content: 'Hello' });
			const trigger = screen.getByRole('button', { name: 'More information' });
			await fireEvent.focus(trigger);
			const bubble = document.querySelector('[role="tooltip"]')!;
			expect(bubble.classList.contains('invisible')).toBe(false);
		});

		it('bubble hides after mouseleave debounce (100ms)', async () => {
			vi.useFakeTimers();
			render(Tooltip, { content: 'Hello' });
			const trigger = screen.getByRole('button', { name: 'More information' });
			await fireEvent.mouseEnter(trigger);
			await fireEvent.mouseLeave(trigger);
			// Still visible immediately after mouseleave
			const bubble = document.querySelector('[role="tooltip"]')!;
			expect(bubble.classList.contains('invisible')).toBe(false);
			// Hidden after debounce expires — advance timer then flush Svelte microtask queue
			vi.advanceTimersByTime(150);
			await tick();
			expect(bubble.classList.contains('invisible')).toBe(true);
		});

		it('bubble hides after blur debounce (100ms)', async () => {
			vi.useFakeTimers();
			render(Tooltip, { content: 'Hello' });
			const trigger = screen.getByRole('button', { name: 'More information' });
			await fireEvent.focus(trigger);
			await fireEvent.blur(trigger);
			vi.advanceTimersByTime(150);
			await tick();
			const bubble = document.querySelector('[role="tooltip"]')!;
			expect(bubble.classList.contains('invisible')).toBe(true);
		});

		it('hover-bridge: mouseleave trigger + mouseenter bubble keeps tooltip visible', async () => {
			vi.useFakeTimers();
			render(Tooltip, { content: 'Hello' });
			const trigger = screen.getByRole('button', { name: 'More information' });
			const bubble = document.querySelector('[role="tooltip"]')!;
			await fireEvent.mouseEnter(trigger);
			await fireEvent.mouseLeave(trigger);
			await fireEvent.mouseEnter(bubble);
			vi.advanceTimersByTime(150);
			await tick();
			expect(bubble.classList.contains('invisible')).toBe(false);
		});

		it('Escape hides the tooltip immediately without debounce', async () => {
			vi.useFakeTimers();
			render(Tooltip, { content: 'Hello' });
			const trigger = screen.getByRole('button', { name: 'More information' });
			await fireEvent.mouseEnter(trigger);
			await fireEvent.keyDown(trigger, { key: 'Escape' });
			const bubble = document.querySelector('[role="tooltip"]')!;
			expect(bubble.classList.contains('invisible')).toBe(true);
		});

		it('bubble remains visible after positioning effect runs', async () => {
			vi.useFakeTimers();
			render(Tooltip, { content: 'Hello' });
			const trigger = screen.getByRole('button', { name: 'More information' });
			await fireEvent.mouseEnter(trigger);
			const bubble = document.querySelector('[role="tooltip"]')!;
			// getBoundingClientRect returns zeros in JSDOM — effect must not crash
			expect(bubble.classList.contains('invisible')).toBe(false);
		});
	});
});
