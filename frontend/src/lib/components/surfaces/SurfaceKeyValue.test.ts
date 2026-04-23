import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render, screen } from '@testing-library/svelte';
import SurfaceKeyValue from './SurfaceKeyValue.svelte';

describe('SurfaceKeyValue', () => {
	afterEach(() => {
		cleanup();
	});

	it('uses shared key/value typography and tone parity for labels and values', () => {
		const { container } = render(SurfaceKeyValue, {
			data: {
				Hostname: 'node-01'
			}
		});

		expect(screen.getByText('Hostname')).toBeInTheDocument();
		expect(screen.getByText('node-01')).toBeInTheDocument();

		const keyElement = container.querySelector('dt');
		const valueElement = container.querySelector('dd');
		expect(keyElement).toBeInTheDocument();
		expect(valueElement).toBeInTheDocument();
		expect(keyElement?.className).toContain('text-nav-item');
		expect(keyElement?.className).toContain('text-[var(--text-secondary)]');
		expect(valueElement?.className).toContain('text-table-header');
		expect(valueElement?.className).toContain('text-[var(--text-primary)]');
	});
});
