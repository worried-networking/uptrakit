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
			label: 'Provider',
			selectedId: 'provider-a',
			providers: [
				{ id: 'provider-a', label: 'Provider A', description: 'Connected locally' },
				{ id: 'provider-b', label: 'Provider B', description: 'Connected remotely' }
			],
			onSelect
		});

		const select = screen.getByLabelText('Provider');
		expect(screen.getByText('Connected locally')).toBeInTheDocument();

		await fireEvent.change(select, { target: { value: 'provider-b' } });

		expect(onSelect).toHaveBeenCalledWith('provider-b');
		await view.rerender({
			label: 'Provider',
			selectedId: 'provider-b',
			providers: [
				{ id: 'provider-a', label: 'Provider A', description: 'Connected locally' },
				{ id: 'provider-b', label: 'Provider B', description: 'Connected remotely' }
			],
			onSelect
		});
		expect(screen.getByText('Connected remotely')).toBeInTheDocument();
	});

	it('treats selectedId as authoritative when the parent rerenders with the same selection', async () => {
		const onSelect = vi.fn();
		const view = render(ProviderSelector, {
			label: 'Provider',
			selectedId: 'provider-a',
			providers: [
				{ id: 'provider-a', label: 'Provider A', description: 'Connected locally' },
				{ id: 'provider-b', label: 'Provider B', description: 'Connected remotely' }
			],
			onSelect
		});

		const select = screen.getByLabelText('Provider') as HTMLSelectElement;
		await fireEvent.change(select, { target: { value: 'provider-b' } });
		expect(onSelect).toHaveBeenCalledWith('provider-b');

		await view.rerender({
			label: 'Provider',
			selectedId: 'provider-a',
			providers: [
				{ id: 'provider-a', label: 'Provider A', description: 'Connected locally' },
				{ id: 'provider-b', label: 'Provider B', description: 'Connected remotely' }
			],
			onSelect
		});

		expect((screen.getByLabelText('Provider') as HTMLSelectElement).value).toBe('provider-a');
		expect(screen.getByText('Connected locally')).toBeInTheDocument();
	});

	it('supports uncontrolled selection changes when selectedId is omitted', async () => {
		render(ProviderSelector, {
			label: 'Provider',
			providers: [
				{ id: 'provider-a', label: 'Provider A', description: 'Connected locally' },
				{ id: 'provider-b', label: 'Provider B', description: 'Connected remotely' }
			]
		});

		const select = screen.getByLabelText('Provider') as HTMLSelectElement;
		expect(select.value).toBe('provider-a');
		expect(screen.getByText('Connected locally')).toBeInTheDocument();

		await fireEvent.change(select, { target: { value: 'provider-b' } });

		expect(select.value).toBe('provider-b');
		expect(screen.getByText('Connected remotely')).toBeInTheDocument();
	});
});
