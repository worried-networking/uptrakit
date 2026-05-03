import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import ProviderSelector from './ProviderSelector.svelte';

afterEach(() => {
	cleanup();
	vi.clearAllMocks();
});

describe('ProviderSelector', () => {
	it('renders providers with semantic labels and reports selection changes', async () => {
		const onSelect = vi.fn();

		const view = render(ProviderSelector, {
			id: 'test-provider',
			label: 'Provider',
			selectedId: 'provider-a',
			providers: [
				{ id: 'provider-a', label: 'Provider A' },
				{ id: 'provider-b', label: 'Provider B' }
			],
			onSelect
		});

		const select = screen.getByLabelText('Provider');

		await fireEvent.change(select, { target: { value: 'provider-b' } });

		expect(onSelect).toHaveBeenCalledWith('provider-b');
		await view.rerender({
			id: 'test-provider',
			label: 'Provider',
			selectedId: 'provider-b',
			providers: [
				{ id: 'provider-a', label: 'Provider A' },
				{ id: 'provider-b', label: 'Provider B' }
			],
			onSelect
		});
		expect((screen.getByLabelText('Provider') as HTMLSelectElement).value).toBe('provider-b');
	});

	it('renders disabled providers with the disabled attribute on their option', () => {
		render(ProviderSelector, {
			id: 'test-provider',
			label: 'Provider',
			selectedId: 'provider-a',
			providers: [
				{ id: 'provider-a', label: 'Provider A' },
				{ id: 'provider-b', label: 'Provider B', disabled: true }
			]
		});
		const opts = (screen.getByLabelText('Provider') as HTMLSelectElement).options;
		expect(opts[0].disabled).toBe(false);
		expect(opts[1].disabled).toBe(true);
	});
});
