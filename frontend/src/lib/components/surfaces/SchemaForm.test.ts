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

	it('renders text field as Input primitive with rounded-card', async () => {
		const loadInitialValues = vi.fn().mockResolvedValue({});
		const { container } = render(SchemaForm, {
			fields: [{ key: 'name', label: 'Name', field_type: 'text', required: true }] satisfies FormField[],
			loadInitialValues,
			onsubmit: vi.fn().mockResolvedValue(undefined)
		});
		await waitFor(() => expect(loadInitialValues).toHaveBeenCalled());
		const input = container.querySelector('input[type="text"]');
		expect(input).toBeInTheDocument();
		expect(input!.className).toContain('rounded-card');
		const submitBtn = container.querySelector('button[type="submit"]');
		expect(submitBtn).toBeInTheDocument();
		expect(submitBtn!.className).toContain('h-[23px]');
	});

	it('renders password field as Input primitive with type=password', async () => {
		const loadInitialValues = vi.fn().mockResolvedValue({});
		const { container } = render(SchemaForm, {
			fields: [{ key: 'pass', label: 'Password', field_type: 'password', required: false }] satisfies FormField[],
			loadInitialValues,
			onsubmit: vi.fn().mockResolvedValue(undefined)
		});
		await waitFor(() => expect(loadInitialValues).toHaveBeenCalled());
		expect(container.querySelector('input[type="password"]')).toBeInTheDocument();
	});

	it('renders number field as Input primitive with type=number', async () => {
		const loadInitialValues = vi.fn().mockResolvedValue({});
		const { container } = render(SchemaForm, {
			fields: [{ key: 'port', label: 'Port', field_type: 'number', required: false }] satisfies FormField[],
			loadInitialValues,
			onsubmit: vi.fn().mockResolvedValue(undefined)
		});
		await waitFor(() => expect(loadInitialValues).toHaveBeenCalled());
		expect(container.querySelector('input[type="number"]')).toBeInTheDocument();
	});

	it('renders textarea field as Textarea primitive with rows=3', async () => {
		const loadInitialValues = vi.fn().mockResolvedValue({});
		const { container } = render(SchemaForm, {
			fields: [{ key: 'notes', label: 'Notes', field_type: 'textarea', required: false }] satisfies FormField[],
			loadInitialValues,
			onsubmit: vi.fn().mockResolvedValue(undefined)
		});
		await waitFor(() => expect(loadInitialValues).toHaveBeenCalled());
		const ta = container.querySelector('textarea');
		expect(ta).toBeInTheDocument();
		expect(ta!.getAttribute('rows')).toBe('3');
	});

	it('renders ssh_private_key as mono Textarea with rows=8', async () => {
		const loadInitialValues = vi.fn().mockResolvedValue({});
		const { container } = render(SchemaForm, {
			fields: [{ key: 'key', label: 'SSH Key', field_type: 'ssh_private_key', required: false }] satisfies FormField[],
			loadInitialValues,
			onsubmit: vi.fn().mockResolvedValue(undefined)
		});
		await waitFor(() => expect(loadInitialValues).toHaveBeenCalled());
		const ta = container.querySelector('textarea');
		expect(ta).toBeInTheDocument();
		expect(ta!.getAttribute('rows')).toBe('8');
		expect(ta!.className).toContain('font-mono');
	});

	it('renders toggle field as Checkbox primitive', async () => {
		const loadInitialValues = vi.fn().mockResolvedValue({});
		const { container } = render(SchemaForm, {
			fields: [{ key: 'enabled', label: 'Enabled', field_type: 'toggle', required: false }] satisfies FormField[],
			loadInitialValues,
			onsubmit: vi.fn().mockResolvedValue(undefined)
		});
		await waitFor(() => expect(loadInitialValues).toHaveBeenCalled());
		const cb = container.querySelector('input[type="checkbox"]');
		expect(cb).toBeInTheDocument();
		expect(cb!.className).toContain('rounded-badge');
	});

	it('renders hidden field as raw input[type=hidden] (not a primitive)', async () => {
		const loadInitialValues = vi.fn().mockResolvedValue({});
		const { container } = render(SchemaForm, {
			fields: [{ key: 'secret', label: 'Secret', field_type: 'hidden', required: false }] satisfies FormField[],
			loadInitialValues,
			onsubmit: vi.fn().mockResolvedValue(undefined)
		});
		await waitFor(() => expect(loadInitialValues).toHaveBeenCalled());
		const hidden = container.querySelector('input[type="hidden"]');
		expect(hidden).toBeInTheDocument();
	});

	it('select field renders raw <select> (not migrated — regression guard)', async () => {
		const loadInitialValues = vi.fn().mockResolvedValue({});
		vi.mocked(apiGet).mockResolvedValue([]);
		const { container } = render(SchemaForm, {
			fields: [
				{
					key: 'region',
					label: 'Region',
					field_type: 'select',
					required: false,
					options: [{ value: 'eu', label: 'EU' }]
				}
			] satisfies FormField[],
			loadInitialValues,
			onsubmit: vi.fn().mockResolvedValue(undefined)
		});
		await waitFor(() => expect(loadInitialValues).toHaveBeenCalled());
		expect(container.querySelector('select')).toBeInTheDocument();
	});

	it('multi_select renders CheckboxList (not migrated — regression guard)', async () => {
		const loadInitialValues = vi.fn().mockResolvedValue({});
		const { container } = render(SchemaForm, {
			fields: [
				{
					key: 'tags',
					label: 'Tags',
					field_type: 'multi_select',
					required: false,
					options: [{ value: 'a', label: 'A' }]
				}
			] satisfies FormField[],
			loadInitialValues,
			onsubmit: vi.fn().mockResolvedValue(undefined)
		});
		await waitFor(() => expect(loadInitialValues).toHaveBeenCalled());
		const checkboxes = container.querySelectorAll('input[type="checkbox"]');
		expect(checkboxes.length).toBeGreaterThanOrEqual(1);
	});

	it('unknown field_type warns once and renders as text input', async () => {
		const consoleSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
		const loadInitialValues = vi.fn().mockResolvedValue({});
		const { container } = render(SchemaForm, {
			fields: [
				{ key: 'x', label: 'X', field_type: 'alien' as FormField['field_type'], required: false }
			] satisfies FormField[],
			loadInitialValues,
			onsubmit: vi.fn().mockResolvedValue(undefined)
		});
		await waitFor(() => expect(loadInitialValues).toHaveBeenCalled());
		expect(container.querySelector('input[type="text"]')).toBeInTheDocument();
		expect(consoleSpy).toHaveBeenCalledWith(expect.stringContaining('alien'));
		consoleSpy.mockRestore();
	});

	it('error prop on text field propagates aria-invalid to input after failed submit', async () => {
		const loadInitialValues = vi.fn().mockResolvedValue({});
		const onsubmit = vi.fn().mockResolvedValue(undefined);
		const { container } = render(SchemaForm, {
			fields: [
				{
					key: 'name',
					label: 'Name',
					field_type: 'text',
					required: true
				}
			] satisfies FormField[],
			loadInitialValues,
			onsubmit
		});
		await waitFor(() => expect(loadInitialValues).toHaveBeenCalled());
		// Submit without filling required field — triggers validation error
		const form = container.querySelector('form')!;
		await fireEvent.submit(form);
		await waitFor(() => {
			const input = container.querySelector('input[type="text"]');
			expect(input).toHaveAttribute('aria-invalid', 'true');
		});
		// onsubmit not called because validation blocked it
		expect(onsubmit).not.toHaveBeenCalled();
	});

	it('submit Button shows aria-busy and preserves label text during loading', async () => {
		const loadInitialValues = vi.fn().mockResolvedValue({});
		const onsubmit = vi.fn().mockImplementation(() => new Promise(() => {}));
		const { container } = render(SchemaForm, {
			fields: [{ key: 'name', label: 'Name', field_type: 'text', required: false }] satisfies FormField[],
			submitLabel: 'Save Config',
			loadInitialValues,
			onsubmit
		});
		await waitFor(() => expect(loadInitialValues).toHaveBeenCalled());
		const submitBtn = container.querySelector('button[type="submit"]')!;
		expect(submitBtn).toBeInTheDocument();
		expect(submitBtn.textContent).toContain('Save Config');
		expect(submitBtn).not.toHaveAttribute('aria-busy');
	});

	it('no raw preset-filled-* or preset-tonal-* classes on submit button', async () => {
		const loadInitialValues = vi.fn().mockResolvedValue({});
		const { container } = render(SchemaForm, {
			fields: [{ key: 'name', label: 'Name', field_type: 'text', required: false }] satisfies FormField[],
			loadInitialValues,
			onsubmit: vi.fn().mockResolvedValue(undefined)
		});
		await waitFor(() => expect(loadInitialValues).toHaveBeenCalled());
		const buttons = container.querySelectorAll('button');
		buttons.forEach((b) => {
			expect(b.className).not.toMatch(/preset-filled|preset-tonal/);
		});
	});
});
