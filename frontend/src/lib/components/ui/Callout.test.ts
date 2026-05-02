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

	it('renders a tone icon inside the callout', () => {
		const { container } = render(Callout, {
			tone: 'warning',
			title: 'Watch out',
			message: 'Something needs your attention.'
		});

		const callout = container.querySelector('[data-ui="callout"]');
		expect(callout?.querySelector('svg')).toBeInTheDocument();
	});

	it('renders a danger callout with its icon', () => {
		const { container } = render(Callout, {
			tone: 'danger',
			message: 'Critical error.'
		});
		const callout = container.querySelector('[data-ui="callout"]');
		expect(callout?.querySelector('svg')).toBeInTheDocument();
	});
});
