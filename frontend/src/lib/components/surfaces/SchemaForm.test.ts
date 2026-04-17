import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/svelte';
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

		expect(await screen.findByRole('combobox', { name: 'Region *' })).toBeInTheDocument();
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
});
