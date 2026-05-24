import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/svelte';
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
	});
});
