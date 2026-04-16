import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';

import SurfaceForm from './SurfaceForm.svelte';
import type { InteractionDescriptor } from '$lib/surfaces/contract';

vi.mock('$lib/api', () => ({
	invokeSurfaceInteraction: vi.fn()
}));

vi.mock('$lib/notifications.svelte', () => ({
	showError: vi.fn(),
	showSuccess: vi.fn()
}));

vi.mock('$lib/surfaces/interactions', () => ({
	buildSurfaceInteractionRequest: vi.fn(async (_interaction, params, options) => ({
		params,
		target_provider_id: options.targetProviderId,
		timeout_seconds: undefined
	}))
}));

import { invokeSurfaceInteraction } from '$lib/api';

describe('SurfaceForm', () => {
	beforeEach(() => {
		vi.mocked(invokeSurfaceInteraction).mockReset();
	});

	afterEach(() => {
		cleanup();
		vi.clearAllMocks();
	});

	it('renders schema fields and preloads values for form-backed surface interactions', async () => {
		vi.mocked(invokeSurfaceInteraction)
			.mockResolvedValueOnce({ new_image_ref: 'ghcr.io/example/app:1.2.3' })
			.mockResolvedValueOnce({});
		const interaction: InteractionDescriptor = {
			interaction_id: 'switch-tag',
			kind: 'form_submit',
			transport: { mode: 'controller_local' },
			form_ui: {
				fields: [
					{ key: 'software_item_id', label: '', field_type: 'hidden', required: true },
					{ key: 'host_id', label: '', field_type: 'hidden', required: true },
					{
						key: 'new_image_ref',
						label: 'New Image Reference',
						field_type: 'text',
						required: true
					}
				],
				pre_load_interaction_id: 'get-current-tag'
			}
		};
		const preLoadInteraction: InteractionDescriptor = {
			interaction_id: 'get-current-tag',
			kind: 'data_load',
			transport: { mode: 'controller_local' }
		};

		render(SurfaceForm, {
			surfaceId: 'docker.item-host-actions',
			interaction,
			preLoadInteraction,
			baseParams: {
				software_item_id: 'software-1',
				host_id: 'host-1'
			}
		});

		const input = await screen.findByRole('textbox', { name: /New Image Reference/i });
		await waitFor(() => {
			expect((input as HTMLInputElement).value).toBe('ghcr.io/example/app:1.2.3');
		});
		expect(screen.queryByLabelText('JSON Payload')).not.toBeInTheDocument();

		await fireEvent.input(input, {
			target: { value: 'ghcr.io/example/app:2.0.0' }
		});
		await fireEvent.submit(input.closest('form')!);

		await waitFor(() => {
			expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenNthCalledWith(
				1,
				'docker.item-host-actions',
				'get-current-tag',
				expect.objectContaining({
					params: {
						software_item_id: 'software-1',
						host_id: 'host-1'
					}
				})
			);
			expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenNthCalledWith(
				2,
				'docker.item-host-actions',
				'switch-tag',
				expect.objectContaining({
					params: {
						software_item_id: 'software-1',
						host_id: 'host-1',
						new_image_ref: 'ghcr.io/example/app:2.0.0'
					}
				})
			);
		});
	});

	it('loads action-backed select options via surface interactions', async () => {
		vi.mocked(invokeSurfaceInteraction)
			.mockResolvedValueOnce({
				options: [{ value: 'eu-west-1', label: 'EU West 1' }]
			})
			.mockResolvedValueOnce({});
		const interaction: InteractionDescriptor = {
			interaction_id: 'switch-region',
			kind: 'form_submit',
			transport: { mode: 'controller_local' },
			form_ui: {
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
				]
			}
		};
		const interactions: InteractionDescriptor[] = [
			{
				interaction_id: 'list-regions',
				kind: 'data_load',
				transport: { mode: 'controller_local' }
			},
			{
				interaction_id: 'switch-region',
				kind: 'form_submit',
				transport: { mode: 'controller_local' }
			}
		];

		render(SurfaceForm, {
			surfaceId: 'docker.item-host-actions',
			interaction,
			interactions,
			baseParams: {
				software_item_id: 'software-1',
				host_id: 'host-1'
			}
		});

		await waitFor(() => {
			expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenNthCalledWith(
				1,
				'docker.item-host-actions',
				'list-regions',
				expect.objectContaining({
					params: {
						software_item_id: 'software-1',
						host_id: 'host-1'
					}
				})
			);
		});
		expect(await screen.findByRole('option', { name: 'EU West 1' })).toBeInTheDocument();

		const select = screen.getByRole('combobox', { name: /Region/i });
		await fireEvent.change(select, {
			target: { value: 'eu-west-1' }
		});
		await fireEvent.submit(select.closest('form')!);

		await waitFor(() => {
			expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenNthCalledWith(
				2,
				'docker.item-host-actions',
				'switch-region',
				expect.objectContaining({
					params: {
						software_item_id: 'software-1',
						host_id: 'host-1',
						region: 'eu-west-1'
					}
				})
			);
		});
	});

	it('coerces schema number fields to JSON numbers before submit', async () => {
		vi.mocked(invokeSurfaceInteraction).mockResolvedValueOnce({});
		const interaction: InteractionDescriptor = {
			interaction_id: 'save-client',
			kind: 'form_submit',
			transport: { mode: 'controller_local' },
			form_ui: {
				fields: [
					{
						key: 'port',
						label: 'Broker Port',
						field_type: 'number',
						required: true
					}
				]
			}
		};

		render(SurfaceForm, {
			surfaceId: 'mqtt.clients',
			interaction
		});

		const input = screen.getByRole('spinbutton', { name: /Broker Port/i });
		await fireEvent.input(input, {
			target: { value: '1883' }
		});
		await fireEvent.submit(input.closest('form')!);

		await waitFor(() => {
			expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledWith(
				'mqtt.clients',
				'save-client',
				expect.objectContaining({
					params: {
						port: 1883
					}
				})
			);
		});
	});

	it('does not send synthetic _row helper in preload requests', async () => {
		vi.mocked(invokeSurfaceInteraction).mockResolvedValueOnce({ value: 'preloaded' });
		const interaction: InteractionDescriptor = {
			interaction_id: 'save',
			kind: 'form_submit',
			transport: { mode: 'controller_local' },
			form_ui: {
				fields: [{ key: 'value', label: 'Value', field_type: 'text', required: true }],
				pre_load_interaction_id: 'load'
			}
		};
		const preLoadInteraction: InteractionDescriptor = {
			interaction_id: 'load',
			kind: 'data_load',
			transport: { mode: 'controller_local' }
		};

		render(SurfaceForm, {
			surfaceId: 'example.surface',
			interaction,
			preLoadInteraction,
			baseParams: {
				host_id: 'host-1',
				_row: { id: 'row-1', name: 'Alpha' }
			}
		});

		await waitFor(() => {
			expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenNthCalledWith(
				1,
				'example.surface',
				'load',
				expect.objectContaining({
					params: {
						host_id: 'host-1'
					}
				})
			);
		});
	});
});
