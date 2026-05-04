import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render } from '@testing-library/svelte';
import SurfaceActionButton from './SurfaceActionButton.svelte';

afterEach(() => cleanup());

describe('SurfaceActionButton', () => {
	it('renders the visible label when labelDisplay is "always"', () => {
		const { container } = render(SurfaceActionButton, {
			label: 'Sync',
			icon: 'refresh-cw',
			labelDisplay: 'always',
			variant: 'primary'
		});
		expect(container.textContent).toContain('Sync');
		expect(container.querySelector('.sr-only')).toBeNull();
		expect(container.querySelector('.button-label-auto')).toBeNull();
		expect(container.querySelector('span[title]')).toBeNull();
	});

	it('renders sr-only label and tooltip wrapper for icon-only with icon', () => {
		const { container } = render(SurfaceActionButton, {
			label: 'Sync',
			icon: 'refresh-cw',
			labelDisplay: 'icon-only',
			variant: 'primary'
		});
		const srOnly = container.querySelector('.sr-only');
		expect(srOnly?.textContent).toBe('Sync');
		const tooltipWrap = container.querySelector('span[title="Sync"]');
		expect(tooltipWrap).not.toBeNull();
		expect(tooltipWrap?.classList.contains('inline-flex')).toBe(true);
		const button = container.querySelector('button');
		expect(button?.getAttribute('aria-label')).toBe('Sync');
	});

	it('renders button-label-auto span and tooltip wrapper for auto with icon', () => {
		const { container } = render(SurfaceActionButton, {
			label: 'Sync',
			icon: 'refresh-cw',
			labelDisplay: 'auto',
			variant: 'primary'
		});
		const labelSpan = container.querySelector('.button-label-auto');
		expect(labelSpan?.textContent).toBe('Sync');
		const tooltipWrap = container.querySelector('span[title="Sync"]');
		expect(tooltipWrap).not.toBeNull();
	});

	it('forces always when labelDisplay is icon-only but icon is missing', () => {
		const { container } = render(SurfaceActionButton, {
			label: 'Sync',
			icon: undefined,
			labelDisplay: 'icon-only',
			variant: 'primary'
		});
		expect(container.textContent).toContain('Sync');
		expect(container.querySelector('.sr-only')).toBeNull();
		expect(container.querySelector('span[title]')).toBeNull();
	});

	it('forces always when labelDisplay is auto but icon is missing', () => {
		const { container } = render(SurfaceActionButton, {
			label: 'Sync',
			icon: undefined,
			labelDisplay: 'auto',
			variant: 'primary'
		});
		expect(container.textContent).toContain('Sync');
		expect(container.querySelector('.button-label-auto')).toBeNull();
		expect(container.querySelector('span[title]')).toBeNull();
	});

	it('omits the tooltip wrapper when disabled is true even with icon-only', () => {
		const { container } = render(SurfaceActionButton, {
			label: 'Sync',
			icon: 'refresh-cw',
			labelDisplay: 'icon-only',
			disabled: true,
			variant: 'primary'
		});
		expect(container.querySelector('.sr-only')?.textContent).toBe('Sync');
		expect(container.querySelector('span[title]')).toBeNull();
		const button = container.querySelector('button');
		expect(button?.getAttribute('aria-label')).toBe('Sync');
		expect(button?.disabled).toBe(true);
	});

	it('renders the leading icon when the resolver succeeds', () => {
		const { container } = render(SurfaceActionButton, {
			label: 'Sync',
			icon: 'refresh-cw',
			variant: 'primary'
		});
		expect(container.querySelector('svg')).not.toBeNull();
	});

	it('does not render an icon when the resolver fails (unknown name)', () => {
		const consoleSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
		const { container } = render(SurfaceActionButton, {
			label: 'Sync',
			icon: 'NotARealIcon',
			labelDisplay: 'icon-only',
			variant: 'primary'
		});
		expect(consoleSpy).toHaveBeenCalled();
		expect(container.textContent).toContain('Sync');
		expect(container.querySelector('.sr-only')).toBeNull();
		consoleSpy.mockRestore();
	});
});
