import { cleanup, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import PillBadge from './PillBadge.svelte';

afterEach(() => {
	cleanup();
});

describe('PillBadge', () => {
	it('renders the shared pill shell for neutral taxonomy labels', () => {
		render(PillBadge, { label: 'SSH Agent' });

		const pill = screen.getByText('SSH Agent');
		expect(pill).toHaveAttribute('data-ui', 'pill-badge');
		expect(pill.className).toContain('rounded-full');
		expect(pill.className).toContain('text-[7.5px]');
		expect(pill.className).toContain('border-[var(--border-default)]');
	});
});
