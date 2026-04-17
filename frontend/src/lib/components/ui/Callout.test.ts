import { cleanup, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import Callout from './Callout.svelte';

afterEach(() => {
	cleanup();
});

describe('Callout', () => {
	it('renders semantic tone metadata alongside the title and message', () => {
		const { container } = render(Callout, {
			tone: 'warning',
			title: 'Action unavailable',
			message: 'This provider is currently offline.'
		});

		const callout = container.querySelector('[data-ui="callout"]');
		expect(callout).toHaveAttribute('data-tone', 'warning');
		expect(screen.getByText('Action unavailable')).toBeInTheDocument();
		expect(screen.getByText('This provider is currently offline.')).toBeInTheDocument();
	});
});
