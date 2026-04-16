import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import SurfaceTable from './SurfaceTable.svelte';
import type { DataSourceDescriptor, InteractionDescriptor, SurfaceNode } from '$lib/surfaces/contract';

vi.mock('$lib/api', () => ({
	invokeSurfaceInteraction: vi.fn(),
	sealedBoxEncrypt: vi.fn()
}));

vi.mock('$lib/notifications.svelte', () => ({
	showError: vi.fn(),
	showSuccess: vi.fn()
}));

import { invokeSurfaceInteraction, sealedBoxEncrypt } from '$lib/api';

function deferred<T>() {
	let resolve!: (value: T | PromiseLike<T>) => void;
	let reject!: (reason?: unknown) => void;
	const promise = new Promise<T>((res, rej) => {
		resolve = res;
		reject = rej;
	});
	return { promise, resolve, reject };
}

describe('SurfaceTable', () => {
	beforeEach(() => {
		vi.mocked(sealedBoxEncrypt).mockResolvedValue('ciphertext');
		vi.mocked(invokeSurfaceInteraction)
			.mockResolvedValueOnce({
				items: [{ id: 'chan-1', name: 'Alpha' }],
				total: 1,
				page: 1,
				per_page: 20,
				total_pages: 1
			})
			.mockResolvedValueOnce({})
			.mockResolvedValueOnce({
				items: [{ id: 'chan-1', name: 'Alpha' }],
				total: 1,
				page: 1,
				per_page: 20,
				total_pages: 1
			});
	});

	afterEach(() => {
		cleanup();
		vi.clearAllMocks();
	});

	it('hydrates provider-query rows and invokes row actions with merged row params', async () => {
		const node: Extract<SurfaceNode, { kind: 'table' }> = {
			kind: 'table',
			data_source_id: 'data.primary',
			columns: [{ key: 'name', label: 'Name' }],
			row_actions: [{ interaction_id: 'delete' }]
		};
		const dataSource: DataSourceDescriptor = {
			data_source_id: 'data.primary',
			kind: { kind: 'provider_query', operation_id: 'list' },
			result_schema: 'array',
			pagination: {
				default_page_size: 20,
				max_page_size: 200
			},
			refresh_policy: { type: 'manual' }
		};
		const interactions: InteractionDescriptor[] = [
			{
				interaction_id: 'list',
				kind: 'data_load',
				label: 'List',
				transport: { mode: 'controller_local' }
			},
			{
				interaction_id: 'delete',
				kind: 'mutation_action',
				label: 'Delete',
				transport: { mode: 'controller_local' }
			}
		];

		render(SurfaceTable, {
			surfaceId: 'notifications.email',
			node,
			dataSource,
			dataLoadInteraction: interactions[0],
			interactions,
			baseParams: { channel_type: 'email' }
		});

		expect(await screen.findByText('Alpha')).toBeInTheDocument();
		expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenNthCalledWith(1, 'notifications.email', 'list', {
			params: {
				channel_type: 'email',
				page: 1,
				per_page: 20
			},
			target_provider_id: undefined,
			timeout_seconds: undefined
		});

		await fireEvent.click(screen.getByRole('button', { name: 'Delete' }));

		await waitFor(() => {
			expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenNthCalledWith(2, 'notifications.email', 'delete', {
				params: {
					channel_type: 'email',
					id: 'chan-1',
					name: 'Alpha'
				},
				target_provider_id: undefined,
				timeout_seconds: undefined
			});
		});
	});

	it('loads exactly once when pagination changes', async () => {
		vi.mocked(invokeSurfaceInteraction)
			.mockReset()
			.mockResolvedValueOnce({
				items: [{ id: 'chan-1', name: 'Alpha' }],
				total: 2,
				page: 1,
				per_page: 20,
				total_pages: 2
			})
			.mockResolvedValueOnce({
				items: [{ id: 'chan-2', name: 'Beta' }],
				total: 2,
				page: 2,
				per_page: 20,
				total_pages: 2
			});

		const node: Extract<SurfaceNode, { kind: 'table' }> = {
			kind: 'table',
			data_source_id: 'data.primary',
			columns: [{ key: 'name', label: 'Name' }],
			row_actions: []
		};
		const dataSource: DataSourceDescriptor = {
			data_source_id: 'data.primary',
			kind: { kind: 'provider_query', operation_id: 'list' },
			result_schema: 'array',
			pagination: {
				default_page_size: 20,
				max_page_size: 200
			},
			refresh_policy: { type: 'manual' }
		};
		const interactions: InteractionDescriptor[] = [
			{
				interaction_id: 'list',
				kind: 'data_load',
				label: 'List',
				transport: { mode: 'controller_local' }
			}
		];

		render(SurfaceTable, {
			surfaceId: 'notifications.email',
			node,
			dataSource,
			dataLoadInteraction: interactions[0],
			interactions
		});

		expect(await screen.findByText('Alpha')).toBeInTheDocument();
		await fireEvent.click(screen.getByRole('button', { name: 'Next' }));

		expect(await screen.findByText('Beta')).toBeInTheDocument();
		expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledTimes(2);
		expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenNthCalledWith(2, 'notifications.email', 'list', {
			params: {
				page: 2,
				per_page: 20
			},
			target_provider_id: undefined,
			timeout_seconds: undefined
		});
	});

	it('reloads the current page when the shared surface reload event fires', async () => {
		vi.mocked(invokeSurfaceInteraction)
			.mockReset()
			.mockResolvedValueOnce({
				items: [{ id: 'chan-1', name: 'Alpha' }],
				total: 1,
				page: 1,
				per_page: 20,
				total_pages: 1
			})
			.mockResolvedValueOnce({
				items: [{ id: 'chan-1', name: 'Gamma' }],
				total: 1,
				page: 1,
				per_page: 20,
				total_pages: 1
			});

		const node: Extract<SurfaceNode, { kind: 'table' }> = {
			kind: 'table',
			data_source_id: 'data.primary',
			columns: [{ key: 'name', label: 'Name' }],
			row_actions: []
		};
		const dataSource: DataSourceDescriptor = {
			data_source_id: 'data.primary',
			kind: { kind: 'provider_query', operation_id: 'list' },
			result_schema: 'array',
			pagination: {
				default_page_size: 20,
				max_page_size: 200
			},
			refresh_policy: { type: 'manual' }
		};
		const interactions: InteractionDescriptor[] = [
			{
				interaction_id: 'list',
				kind: 'data_load',
				label: 'List',
				transport: { mode: 'controller_local' }
			}
		];

		render(SurfaceTable, {
			surfaceId: 'notifications.email',
			node,
			dataSource,
			dataLoadInteraction: interactions[0],
			interactions
		});

		expect(await screen.findByText('Alpha')).toBeInTheDocument();
		window.dispatchEvent(
			new CustomEvent('surface:reload', {
				detail: {
					surfaceId: 'notifications.email',
					targetProviderId: null
				}
			})
		);

		expect(await screen.findByText('Gamma')).toBeInTheDocument();
		expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledTimes(2);
	});

	it('ignores stale provider-query responses when newer loads finish first', async () => {
		vi.mocked(invokeSurfaceInteraction).mockReset();
		const olderReload = deferred<Record<string, unknown>>();
		const newerReload = deferred<Record<string, unknown>>();
		vi.mocked(invokeSurfaceInteraction)
			.mockResolvedValueOnce({
				items: [{ id: 'chan-1', name: 'Alpha' }],
				total: 1,
				page: 1,
				per_page: 20,
				total_pages: 1
			})
			.mockImplementationOnce(() => olderReload.promise)
			.mockImplementationOnce(() => newerReload.promise);

		const node: Extract<SurfaceNode, { kind: 'table' }> = {
			kind: 'table',
			data_source_id: 'data.primary',
			columns: [{ key: 'name', label: 'Name' }],
			row_actions: []
		};
		const dataSource: DataSourceDescriptor = {
			data_source_id: 'data.primary',
			kind: { kind: 'provider_query', operation_id: 'list' },
			result_schema: 'array',
			pagination: {
				default_page_size: 20,
				max_page_size: 200
			},
			refresh_policy: { type: 'manual' }
		};
		const interactions: InteractionDescriptor[] = [
			{
				interaction_id: 'list',
				kind: 'data_load',
				label: 'List',
				transport: { mode: 'controller_local' }
			}
		];

		const view = render(SurfaceTable, {
			surfaceId: 'notifications.email',
			node,
			dataSource,
			dataLoadInteraction: interactions[0],
			interactions
		});

		expect(await screen.findByText('Alpha')).toBeInTheDocument();
		await view.rerender({
			surfaceId: 'notifications.email',
			node,
			dataSource: { ...dataSource },
			dataLoadInteraction: interactions[0],
			interactions
		});
		await view.rerender({
			surfaceId: 'notifications.email',
			node,
			dataSource: { ...dataSource },
			dataLoadInteraction: interactions[0],
			interactions
		});

		await waitFor(() => {
			expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledTimes(3);
		});

		newerReload.resolve({
			items: [{ id: 'chan-1', name: 'Gamma' }],
			total: 1,
			page: 1,
			per_page: 20,
			total_pages: 1
		});
		expect(await screen.findByText('Gamma')).toBeInTheDocument();

		olderReload.resolve({
			items: [{ id: 'chan-1', name: 'Beta' }],
			total: 1,
			page: 1,
			per_page: 20,
			total_pages: 1
		});
		await olderReload.promise;
		await waitFor(() => {
			expect(screen.getByText('Gamma')).toBeInTheDocument();
		});
		expect(screen.queryByText('Beta')).not.toBeInTheDocument();
	});
});
