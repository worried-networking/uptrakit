import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import SchemaForm from './SchemaForm.svelte';
import type { FormField } from '$lib/types';

vi.mock('$lib/api', () => ({
	apiGet: vi.fn()
}));

vi.mock('$lib/notifications.svelte', () => ({
	showError: vi.fn()
}));

import { apiGet } from '$lib/api';

describe('SchemaForm', () => {
	afterEach(() => {
		cleanup();
		vi.clearAllMocks();
	});

	it('reloads select options when a reused field key gets a new action source', async () => {
		const loadSelectOptions = vi
			.fn()
			.mockResolvedValueOnce([{ value: 'eu-west-1', label: 'EU West 1' }])
			.mockResolvedValueOnce([{ value: 'us-east-1', label: 'US East 1' }]);

		const fields: FormField[] = [
			{
				key: 'region',
				label: 'Region',
				field_type: 'select',
				required: true,
				select_source: {
					type: 'action',
					action_id: 'list-regions-a'
				}
			}
		];

		const view = render(SchemaForm, {
			fields,
			onsubmit: vi.fn().mockResolvedValue(undefined),
			loadSelectOptions
		});

		expect(document.querySelectorAll('[data-ui="form-field-row"]')).toHaveLength(1);
		expect(await screen.findByRole('option', { name: 'EU West 1' })).toBeInTheDocument();
		expect(loadSelectOptions).toHaveBeenCalledWith('list-regions-a');

		await view.rerender({
			fields: [
				{
					key: 'region',
					label: 'Region',
					field_type: 'select',
					required: true,
					select_source: {
						type: 'action',
						action_id: 'list-regions-b'
					}
				}
			] satisfies FormField[],
			onsubmit: vi.fn().mockResolvedValue(undefined),
			loadSelectOptions
		});

		expect(await screen.findByRole('option', { name: 'US East 1' })).toBeInTheDocument();
		expect(screen.queryByRole('option', { name: 'EU West 1' })).not.toBeInTheDocument();
		expect(loadSelectOptions).toHaveBeenNthCalledWith(2, 'list-regions-b');
	});

	it('reloads REST-backed options when a reused field key gets a new endpoint', async () => {
		vi.mocked(apiGet)
			.mockResolvedValueOnce([{ id: 'team-a', name: 'Team A' }])
			.mockResolvedValueOnce([{ id: 'team-b', name: 'Team B' }]);

		const view = render(SchemaForm, {
			fields: [
				{
					key: 'team',
					label: 'Team',
					field_type: 'select',
					required: true,
					select_source: {
						type: 'rest_api',
						path: '/api/v1/teams-a',
						value_field: 'id',
						label_field: 'name'
					}
				}
			] satisfies FormField[],
			onsubmit: vi.fn().mockResolvedValue(undefined)
		});

		expect(await screen.findByRole('option', { name: 'Team A' })).toBeInTheDocument();
		expect(apiGet).toHaveBeenCalledWith('/api/v1/teams-a?page=1&per_page=1000');

		await view.rerender({
			fields: [
				{
					key: 'team',
					label: 'Team',
					field_type: 'select',
					required: true,
					select_source: {
						type: 'rest_api',
						path: '/api/v1/teams-b',
						value_field: 'id',
						label_field: 'name'
					}
				}
			] satisfies FormField[],
			onsubmit: vi.fn().mockResolvedValue(undefined)
		});

		expect(await screen.findByRole('option', { name: 'Team B' })).toBeInTheDocument();
		expect(screen.queryByRole('option', { name: 'Team A' })).not.toBeInTheDocument();
		expect(apiGet).toHaveBeenNthCalledWith(2, '/api/v1/teams-b?page=1&per_page=1000');
	});

	it('retries the same dynamic action source after a transient load failure', async () => {
		const loadSelectOptions = vi
			.fn()
			.mockRejectedValueOnce(new Error('temporary failure'))
			.mockResolvedValueOnce([{ value: 'eu-west-1', label: 'EU West 1' }]);

		const fields: FormField[] = [
			{
				key: 'region',
				label: 'Region',
				field_type: 'select',
				required: true,
				select_source: {
					type: 'action',
					action_id: 'list-regions'
				}
			}
		];

		const view = render(SchemaForm, {
			fields,
			onsubmit: vi.fn().mockResolvedValue(undefined),
			loadSelectOptions
		});

		expect(await screen.findByRole('combobox', { name: /Region/i })).toBeInTheDocument();
		expect(loadSelectOptions).toHaveBeenCalledTimes(1);

		await view.rerender({
			fields: [
				{
					key: 'region',
					label: 'Region',
					field_type: 'select',
					required: true,
					select_source: {
						type: 'action',
						action_id: 'list-regions'
					}
				}
			] satisfies FormField[],
			onsubmit: vi.fn().mockResolvedValue(undefined),
			loadSelectOptions
		});

		expect(await screen.findByRole('option', { name: 'EU West 1' })).toBeInTheDocument();
		expect(loadSelectOptions).toHaveBeenCalledTimes(2);
	});

	it('blocks empty required multi-select submission with inline field validation', async () => {
		const onsubmit = vi.fn().mockResolvedValue(undefined);
		render(SchemaForm, {
			fields: [
				{
					key: 'regions',
					label: 'Regions',
					field_type: 'multi_select',
					required: true,
					options: [
						{ value: 'eu-west-1', label: 'EU West 1' },
						{ value: 'us-east-1', label: 'US East 1' }
					]
				}
			] satisfies FormField[],
			onsubmit
		});

		const submitButton = screen.getByRole('button', { name: 'Submit' });
		await fireEvent.submit(submitButton.closest('form')!);

		expect(onsubmit).not.toHaveBeenCalled();
		expect(screen.getByText('Regions is required.')).toBeInTheDocument();

		const regionOption = screen.getByRole('checkbox', { name: 'EU West 1' });
		await fireEvent.click(regionOption);

		expect(screen.queryByText('Regions is required.')).not.toBeInTheDocument();

		await fireEvent.submit(submitButton.closest('form')!);

		await waitFor(() => {
			expect(onsubmit).toHaveBeenCalledWith({
				regions: '["eu-west-1"]'
			});
		});
	});

	it('loads initial multi-select values from _row and preload payload', async () => {
		const loadInitialValues = vi.fn().mockResolvedValue({
			regions: ['us-east-1']
		});

		render(SchemaForm, {
			fields: [
				{
					key: 'regions',
					label: 'Regions',
					field_type: 'multi_select',
					required: false,
					options: [
						{ value: 'eu-west-1', label: 'EU West 1' },
						{ value: 'us-east-1', label: 'US East 1' }
					]
				}
			] satisfies FormField[],
			extraParams: {
				_row: {
					regions: ['eu-west-1']
				}
			},
			loadInitialValues,
			onsubmit: vi.fn().mockResolvedValue(undefined)
		});

		await waitFor(() => {
			expect(screen.getByRole('checkbox', { name: 'US East 1' })).toBeChecked();
		});
		expect(screen.getByRole('checkbox', { name: 'EU West 1' })).not.toBeChecked();
		expect(loadInitialValues).toHaveBeenCalledTimes(1);
	});
});
