import { cleanup, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { createRawSnippet } from 'svelte';
import FormFieldRow from './FormFieldRow.svelte';

function makeSnippet(html: string) {
	return createRawSnippet(() => ({
		render() {
			return html;
		}
	}));
}

afterEach(() => {
	cleanup();
});

describe('FormFieldRow', () => {
	it('renders the label, hint, error, and associated field content', () => {
		render(FormFieldRow, {
			label: 'Provider name',
			hint: 'Shown in the surface header.',
			error: 'A name is required.',
			inputId: 'provider-name',
			required: true,
			children: makeSnippet('<input id="provider-name" type="text" />')
		});

		expect(screen.getByLabelText('Provider name')).toBeInTheDocument();
		expect(screen.getByText('Shown in the surface header.')).toBeInTheDocument();
		expect(screen.getByText('A name is required.')).toBeInTheDocument();
		expect(screen.getByText('*')).toBeInTheDocument();
	});
});
