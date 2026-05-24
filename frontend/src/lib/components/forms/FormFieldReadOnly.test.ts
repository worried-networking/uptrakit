import { cleanup, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { createRawSnippet } from 'svelte';
import FormFieldReadOnly from './FormFieldReadOnly.svelte';
import ModalLayoutFixture from '$lib/test-mocks/form-field-read-only-with-modal-layout-fixture.svelte';

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

describe('FormFieldReadOnly', () => {
	it('renders label and value as plain text by default', () => {
		const { container } = render(FormFieldReadOnly, {
			label: 'Current URL',
			value: 'nats://localhost:4222'
		});

		expect(screen.getByText('Current URL')).toBeInTheDocument();
		const valueNode = screen.getByText('nats://localhost:4222');
		expect(valueNode).toBeInTheDocument();
		expect(valueNode.className).not.toContain('font-mono');

		const row = container.querySelector('[data-ui="form-field-read-only"]');
		expect(row).not.toBeNull();
	});

	it('applies font-mono when mono prop is set', () => {
		render(FormFieldReadOnly, {
			label: 'CA Fingerprint',
			value: 'e3676c6137dada24f4',
			mono: true
		});

		const valueNode = screen.getByText('e3676c6137dada24f4');
		expect(valueNode.className).toContain('font-mono');
	});

	it('renders children snippet instead of value when provided', () => {
		render(FormFieldReadOnly, {
			label: 'Status',
			value: 'should-not-appear',
			children: makeSnippet('<span>custom rendered</span>')
		});

		expect(screen.getByText('custom rendered')).toBeInTheDocument();
		expect(screen.queryByText('should-not-appear')).toBeNull();
	});

	it('renders hint when supplied', () => {
		render(FormFieldReadOnly, {
			label: 'Current URL',
			hint: 'Configured in environment.',
			value: 'nats://localhost:4222'
		});

		expect(screen.getByText('Configured in environment.')).toBeInTheDocument();
	});

	it('uses page-context label column by default', () => {
		const { container } = render(FormFieldReadOnly, {
			label: 'Current URL',
			value: 'nats://localhost:4222'
		});

		const row = container.querySelector('[data-ui="form-field-read-only-grid"]');
		expect(row?.className).toContain('@[32rem]:grid-cols-[minmax(0,20rem)_minmax(0,1fr)]');
	});

	it('uses modal-context label column when FormLayout.Modal is set in context', () => {
		const { container } = render(ModalLayoutFixture, {
			label: 'Current URL',
			value: 'nats://localhost:4222'
		});

		const row = container.querySelector('[data-ui="form-field-read-only-grid"]');
		expect(row?.className).toContain('@[24rem]:grid-cols-[minmax(0,11rem)_minmax(0,1fr)]');
	});
});
