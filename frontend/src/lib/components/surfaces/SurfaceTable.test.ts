import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import SurfaceTable from './SurfaceTable.svelte';
import type { DataSourceDescriptor, InteractionDescriptor, SurfaceNode } from '$lib/surfaces/contract';

vi.mock('$lib/api', async (importOriginal) => ({
	...(await importOriginal<typeof import('$lib/api')>()),
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
				data: {
					items: [{ id: 'chan-1', name: 'Alpha' }],
					total: 1,
					page: 1,
					per_page: 20,
					total_pages: 1
				}
			} as unknown as Awaited<ReturnType<typeof invokeSurfaceInteraction>>)
			.mockResolvedValueOnce({ data: {} } as unknown as Awaited<ReturnType<typeof invokeSurfaceInteraction>>)
			.mockResolvedValueOnce({
				data: {
					items: [{ id: 'chan-1', name: 'Alpha' }],
					total: 1,
					page: 1,
					per_page: 20,
					total_pages: 1
				}
			} as unknown as Awaited<ReturnType<typeof invokeSurfaceInteraction>>);
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

		const { container } = render(SurfaceTable, {
			surfaceId: 'notifications.email',
			node,
			dataSource,
			dataLoadInteraction: interactions[0],
			interactions,
			baseParams: { channel_type: 'email' }
		});

		expect(await screen.findByText('Alpha')).toBeInTheDocument();
		expect(container.querySelector('[data-ui="data-table"]')).toBeInTheDocument();
		expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenNthCalledWith(1, {
			path: { surface_id: 'notifications.email', interaction_id: 'list' },
			body: {
				params: {
					channel_type: 'email',
					page: 1,
					per_page: 20
				},
				target_provider_id: undefined,
				timeout_seconds: undefined
			}
		});

		await fireEvent.click(screen.getByRole('button', { name: 'Delete' }));

		await waitFor(() => {
			expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenNthCalledWith(2, {
				path: { surface_id: 'notifications.email', interaction_id: 'delete' },
				body: {
					params: {
						channel_type: 'email',
						id: 'chan-1',
						name: 'Alpha'
					},
					target_provider_id: undefined,
					timeout_seconds: undefined
				}
			});
		});
	});

	it('uses shared empty-state copy from the data source when no rows are available', async () => {
		const node: Extract<SurfaceNode, { kind: 'table' }> = {
			kind: 'table',
			data_source_id: 'data.primary',
			columns: [{ key: 'name', label: 'Name' }],
			row_actions: []
		};
		const dataSource: DataSourceDescriptor = {
			data_source_id: 'data.primary',
			kind: { kind: 'static', data: [] },
			result_schema: 'array',
			refresh_policy: { type: 'manual' },
			empty_state: {
				title: 'No channels found',
				description: 'Create a channel to populate this table.'
			}
		};

		const { container } = render(SurfaceTable, {
			surfaceId: 'notifications.email',
			node,
			dataSource,
			rows: []
		});

		expect(screen.getByText('No channels found')).toBeInTheDocument();
		expect(screen.getByText('Create a channel to populate this table.')).toBeInTheDocument();
		expect(container.querySelector('[data-ui="empty-state"]')).toBeInTheDocument();
		expect(container.querySelector('[data-ui="table-footer-bar"]')).not.toBeInTheDocument();
	});

	it('omits row-action treatment when configured actions are not resolvable from interactions', () => {
		const node: Extract<SurfaceNode, { kind: 'table' }> = {
			kind: 'table',
			data_source_id: 'data.primary',
			columns: [{ key: 'name', label: 'Name' }],
			row_actions: [{ interaction_id: 'missing-action' }]
		};
		const dataSource: DataSourceDescriptor = {
			data_source_id: 'data.primary',
			kind: { kind: 'static', data: [{ id: 'chan-1', name: 'Alpha' }] },
			result_schema: 'array',
			refresh_policy: { type: 'manual' }
		};

		render(SurfaceTable, {
			surfaceId: 'notifications.email',
			node,
			dataSource,
			interactions: []
		});

		expect(screen.getByRole('columnheader', { name: 'Name' })).toBeInTheDocument();
		expect(screen.queryByRole('columnheader', { name: 'Actions' })).not.toBeInTheDocument();
	});

	it('omits the shared table footer for static table data', () => {
		const node: Extract<SurfaceNode, { kind: 'table' }> = {
			kind: 'table',
			data_source_id: 'data.primary',
			columns: [{ key: 'name', label: 'Name' }],
			row_actions: []
		};
		const dataSource: DataSourceDescriptor = {
			data_source_id: 'data.primary',
			kind: { kind: 'static', data: [{ id: 'chan-1', name: 'Alpha' }] },
			result_schema: 'array',
			refresh_policy: { type: 'manual' }
		};

		const { container } = render(SurfaceTable, {
			surfaceId: 'notifications.email',
			node,
			dataSource
		});

		expect(screen.getByText('Alpha')).toBeInTheDocument();
		expect(container.querySelector('[data-ui="table-footer-bar"]')).not.toBeInTheDocument();
	});

	it('loads exactly once when pagination changes', async () => {
		vi.mocked(invokeSurfaceInteraction)
			.mockReset()
			.mockResolvedValueOnce({
				data: {
					items: [{ id: 'chan-1', name: 'Alpha' }],
					total: 2,
					page: 1,
					per_page: 20,
					total_pages: 2
				}
			} as unknown as Awaited<ReturnType<typeof invokeSurfaceInteraction>>)
			.mockResolvedValueOnce({
				data: {
					items: [{ id: 'chan-2', name: 'Beta' }],
					total: 2,
					page: 2,
					per_page: 20,
					total_pages: 2
				}
			} as unknown as Awaited<ReturnType<typeof invokeSurfaceInteraction>>);

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
		expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenNthCalledWith(2, {
			path: { surface_id: 'notifications.email', interaction_id: 'list' },
			body: {
				params: {
					page: 2,
					per_page: 20
				},
				target_provider_id: undefined,
				timeout_seconds: undefined
			}
		});
	});

	it('reloads the current page when the shared surface reload event fires', async () => {
		vi.mocked(invokeSurfaceInteraction)
			.mockReset()
			.mockResolvedValueOnce({
				data: {
					items: [{ id: 'chan-1', name: 'Alpha' }],
					total: 1,
					page: 1,
					per_page: 20,
					total_pages: 1
				}
			} as unknown as Awaited<ReturnType<typeof invokeSurfaceInteraction>>)
			.mockResolvedValueOnce({
				data: {
					items: [{ id: 'chan-1', name: 'Gamma' }],
					total: 1,
					page: 1,
					per_page: 20,
					total_pages: 1
				}
			} as unknown as Awaited<ReturnType<typeof invokeSurfaceInteraction>>);

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
		const olderReload = deferred<Awaited<ReturnType<typeof invokeSurfaceInteraction>>>();
		const newerReload = deferred<Awaited<ReturnType<typeof invokeSurfaceInteraction>>>();
		vi.mocked(invokeSurfaceInteraction)
			.mockResolvedValueOnce({
				data: {
					items: [{ id: 'chan-1', name: 'Alpha' }],
					total: 1,
					page: 1,
					per_page: 20,
					total_pages: 1
				}
			} as unknown as Awaited<ReturnType<typeof invokeSurfaceInteraction>>)
			.mockImplementationOnce(() => olderReload.promise as unknown as ReturnType<typeof invokeSurfaceInteraction>)
			.mockImplementationOnce(() => newerReload.promise as unknown as ReturnType<typeof invokeSurfaceInteraction>);

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
			data: {
				items: [{ id: 'chan-1', name: 'Gamma' }],
				total: 1,
				page: 1,
				per_page: 20,
				total_pages: 1
			}
		} as unknown as Awaited<ReturnType<typeof invokeSurfaceInteraction>>);
		expect(await screen.findByText('Gamma')).toBeInTheDocument();

		olderReload.resolve({
			data: {
				items: [{ id: 'chan-1', name: 'Beta' }],
				total: 1,
				page: 1,
				per_page: 20,
				total_pages: 1
			}
		} as unknown as Awaited<ReturnType<typeof invokeSurfaceInteraction>>);
		await olderReload.promise;
		await waitFor(() => {
			expect(screen.getByText('Gamma')).toBeInTheDocument();
		});
		expect(screen.queryByText('Beta')).not.toBeInTheDocument();
	});

	it('cancels in-flight provider-query loads when switching to static data', async () => {
		vi.mocked(invokeSurfaceInteraction).mockReset();
		const inFlightLoad = deferred<Awaited<ReturnType<typeof invokeSurfaceInteraction>>>();
		vi.mocked(invokeSurfaceInteraction).mockImplementationOnce(
			() => inFlightLoad.promise as unknown as ReturnType<typeof invokeSurfaceInteraction>
		);

		const node: Extract<SurfaceNode, { kind: 'table' }> = {
			kind: 'table',
			data_source_id: 'data.primary',
			columns: [{ key: 'name', label: 'Name' }],
			row_actions: []
		};
		const providerQueryDataSource: DataSourceDescriptor = {
			data_source_id: 'data.primary',
			kind: { kind: 'provider_query', operation_id: 'list' },
			result_schema: 'array',
			pagination: {
				default_page_size: 20,
				max_page_size: 200
			},
			refresh_policy: { type: 'manual' }
		};
		const staticDataSource: DataSourceDescriptor = {
			data_source_id: 'data.primary',
			kind: { kind: 'static', data: [{ id: 'chan-static', name: 'Static Row' }] },
			result_schema: 'array',
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
			dataSource: providerQueryDataSource,
			dataLoadInteraction: interactions[0],
			interactions
		});
		expect(await screen.findByText('Loading...')).toBeInTheDocument();

		await view.rerender({
			surfaceId: 'notifications.email',
			node,
			dataSource: staticDataSource,
			dataLoadInteraction: interactions[0],
			interactions
		});

		expect(screen.getByText('Static Row')).toBeInTheDocument();
		expect(screen.queryByText('Loading...')).not.toBeInTheDocument();

		inFlightLoad.resolve({
			data: {
				items: [{ id: 'chan-provider', name: 'Provider Row' }],
				total: 1,
				page: 1,
				per_page: 20,
				total_pages: 1
			}
		} as unknown as Awaited<ReturnType<typeof invokeSurfaceInteraction>>);
		await inFlightLoad.promise;

		await waitFor(() => {
			expect(screen.getByText('Static Row')).toBeInTheDocument();
		});
		expect(screen.queryByText('Provider Row')).not.toBeInTheDocument();
	});

	it('loads from initialPage prop when provided', async () => {
		vi.mocked(invokeSurfaceInteraction)
			.mockReset()
			.mockResolvedValueOnce({
				data: {
					items: [{ id: 'chan-1', name: 'Alpha' }],
					total: 40,
					page: 2,
					per_page: 20,
					total_pages: 2
				}
			} as unknown as Awaited<ReturnType<typeof invokeSurfaceInteraction>>);

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
			pagination: { default_page_size: 20, max_page_size: 200 },
			refresh_policy: { type: 'manual' }
		};
		const interactions: InteractionDescriptor[] = [
			{ interaction_id: 'list', kind: 'data_load', label: 'List', transport: { mode: 'controller_local' } }
		];

		render(SurfaceTable, {
			surfaceId: 'notifications.email',
			node,
			dataSource,
			dataLoadInteraction: interactions[0],
			interactions,
			initialPage: 2
		});

		expect(await screen.findByText('Alpha')).toBeInTheDocument();
		expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledOnce();
		expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledWith({
			path: { surface_id: 'notifications.email', interaction_id: 'list' },
			body: {
				params: { page: 2, per_page: 20 },
				target_provider_id: undefined,
				timeout_seconds: undefined
			}
		});
	});

	it('fires onPageChange callback with data_source_id and new page when page changes', async () => {
		vi.mocked(invokeSurfaceInteraction)
			.mockReset()
			.mockResolvedValueOnce({
				data: {
					items: [{ id: 'chan-1', name: 'Alpha' }],
					total: 40,
					page: 1,
					per_page: 20,
					total_pages: 2
				}
			} as unknown as Awaited<ReturnType<typeof invokeSurfaceInteraction>>)
			.mockResolvedValueOnce({
				data: {
					items: [{ id: 'chan-2', name: 'Beta' }],
					total: 40,
					page: 2,
					per_page: 20,
					total_pages: 2
				}
			} as unknown as Awaited<ReturnType<typeof invokeSurfaceInteraction>>);

		const onPageChange = vi.fn();
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
			pagination: { default_page_size: 20, max_page_size: 200 },
			refresh_policy: { type: 'manual' }
		};
		const interactions: InteractionDescriptor[] = [
			{ interaction_id: 'list', kind: 'data_load', label: 'List', transport: { mode: 'controller_local' } }
		];

		render(SurfaceTable, {
			surfaceId: 'notifications.email',
			node,
			dataSource,
			dataLoadInteraction: interactions[0],
			interactions,
			onPageChange
		});

		expect(await screen.findByText('Alpha')).toBeInTheDocument();
		await fireEvent.click(screen.getByRole('button', { name: 'Next' }));

		await waitFor(() => {
			expect(onPageChange).toHaveBeenCalledOnce();
			expect(onPageChange).toHaveBeenCalledWith('data.primary', 2);
		});
	});

	it('syncs currentPage from initialPage prop when it changes (browser back simulation)', async () => {
		vi.mocked(invokeSurfaceInteraction)
			.mockReset()
			.mockResolvedValue({
				data: {
					items: [{ id: 'chan-1', name: 'Alpha' }],
					total: 40,
					page: 1,
					per_page: 20,
					total_pages: 2
				}
			} as unknown as Awaited<ReturnType<typeof invokeSurfaceInteraction>>);

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
			pagination: { default_page_size: 20, max_page_size: 200 },
			refresh_policy: { type: 'manual' }
		};
		const interactions: InteractionDescriptor[] = [
			{ interaction_id: 'list', kind: 'data_load', label: 'List', transport: { mode: 'controller_local' } }
		];

		const view = render(SurfaceTable, {
			surfaceId: 'notifications.email',
			node,
			dataSource,
			dataLoadInteraction: interactions[0],
			interactions,
			initialPage: 2
		});

		await waitFor(() => {
			expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledWith({
				path: { surface_id: 'notifications.email', interaction_id: 'list' },
				body: expect.objectContaining({ params: expect.objectContaining({ page: 2 }) })
			});
		});

		vi.mocked(invokeSurfaceInteraction).mockClear();

		await view.rerender({
			surfaceId: 'notifications.email',
			node,
			dataSource,
			dataLoadInteraction: interactions[0],
			interactions,
			initialPage: 1
		});

		await waitFor(() => {
			expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledWith({
				path: { surface_id: 'notifications.email', interaction_id: 'list' },
				body: expect.objectContaining({ params: expect.objectContaining({ page: 1 }) })
			});
			expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledTimes(1);
		});
	});

	it('entity-link path: row-action wrapper uses flex flex-nowrap @container/buttons', async () => {
		vi.mocked(invokeSurfaceInteraction).mockReset();

		const node: Extract<SurfaceNode, { kind: 'table' }> = {
			kind: 'table',
			data_source_id: 'data.primary',
			columns: [{ key: 'name', label: 'Name', cell_type: { kind: 'entity_link', entity_type: 'host' } }],
			row_actions: [{ interaction_id: 'delete' }]
		};
		const dataSource: DataSourceDescriptor = {
			data_source_id: 'data.primary',
			kind: { kind: 'static', data: [{ id: 'row-1', name: { entity_id: 'h-1', label: 'Alpha', found: true } }] },
			result_schema: 'array',
			refresh_policy: { type: 'manual' }
		};
		const interactions: InteractionDescriptor[] = [
			{
				interaction_id: 'delete',
				kind: 'mutation_action',
				label: 'Delete',
				icon: 'trash-2',
				transport: { mode: 'controller_local' }
			}
		];

		const { container } = render(SurfaceTable, {
			surfaceId: 'test.surface',
			node,
			dataSource,
			interactions
		});

		expect(screen.getByText('Alpha')).toBeInTheDocument();
		const wrapper = container.querySelector('div.flex.flex-nowrap.\\@container\\/buttons');
		expect(wrapper).toBeInTheDocument();
		const td = container.querySelector('td.table-cell-pad.whitespace-nowrap');
		expect(td).toBeInTheDocument();
		expect(td?.classList.contains('whitespace-nowrap')).toBe(true);
	});

	it('entity-link path: icon-only labelDisplay hides label in sr-only when icon present', async () => {
		vi.mocked(invokeSurfaceInteraction).mockReset();

		const node: Extract<SurfaceNode, { kind: 'table' }> = {
			kind: 'table',
			data_source_id: 'data.primary',
			columns: [{ key: 'name', label: 'Name', cell_type: { kind: 'entity_link', entity_type: 'host' } }],
			row_actions: [{ interaction_id: 'delete' }]
		};
		const dataSource: DataSourceDescriptor = {
			data_source_id: 'data.primary',
			kind: { kind: 'static', data: [{ id: 'row-1', name: { entity_id: 'h-1', label: 'Alpha', found: true } }] },
			result_schema: 'array',
			refresh_policy: { type: 'manual' }
		};
		const interactions: InteractionDescriptor[] = [
			{
				interaction_id: 'delete',
				kind: 'mutation_action',
				label: 'Delete',
				icon: 'trash-2',
				transport: { mode: 'controller_local' }
			}
		];

		const { container } = render(SurfaceTable, {
			surfaceId: 'test.surface',
			node,
			dataSource,
			interactions
		});

		expect(screen.getByText('Alpha')).toBeInTheDocument();
		const srOnly = container.querySelector('span.sr-only');
		expect(srOnly).toBeInTheDocument();
		expect(srOnly?.textContent).toBe('Delete');
	});

	it('entity-link path: label is visible (no sr-only) when interaction has no icon', async () => {
		vi.mocked(invokeSurfaceInteraction).mockReset();

		const node: Extract<SurfaceNode, { kind: 'table' }> = {
			kind: 'table',
			data_source_id: 'data.primary',
			columns: [{ key: 'name', label: 'Name', cell_type: { kind: 'entity_link', entity_type: 'host' } }],
			row_actions: [{ interaction_id: 'act' }]
		};
		const dataSource: DataSourceDescriptor = {
			data_source_id: 'data.primary',
			kind: { kind: 'static', data: [{ id: 'row-1', name: { entity_id: 'h-1', label: 'Alpha', found: true } }] },
			result_schema: 'array',
			refresh_policy: { type: 'manual' }
		};
		const interactions: InteractionDescriptor[] = [
			{
				interaction_id: 'act',
				kind: 'mutation_action',
				label: 'Do It',
				transport: { mode: 'controller_local' }
			}
		];

		const { container } = render(SurfaceTable, {
			surfaceId: 'test.surface',
			node,
			dataSource,
			interactions
		});

		expect(screen.getByText('Alpha')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Do It' })).toBeInTheDocument();
		expect(container.querySelector('span.sr-only')).not.toBeInTheDocument();
	});

	it('rowActions snippet path: row-action wrapper uses flex flex-nowrap @container/buttons', async () => {
		vi.mocked(invokeSurfaceInteraction).mockReset();

		const node: Extract<SurfaceNode, { kind: 'table' }> = {
			kind: 'table',
			data_source_id: 'data.primary',
			columns: [{ key: 'name', label: 'Name' }],
			row_actions: [{ interaction_id: 'delete' }]
		};
		const dataSource: DataSourceDescriptor = {
			data_source_id: 'data.primary',
			kind: { kind: 'static', data: [{ id: 'row-1', name: 'Alpha' }] },
			result_schema: 'array',
			refresh_policy: { type: 'manual' }
		};
		const interactions: InteractionDescriptor[] = [
			{
				interaction_id: 'delete',
				kind: 'mutation_action',
				label: 'Delete',
				icon: 'trash-2',
				transport: { mode: 'controller_local' }
			}
		];

		const { container } = render(SurfaceTable, {
			surfaceId: 'test.surface',
			node,
			dataSource,
			interactions
		});

		expect(screen.getByText('Alpha')).toBeInTheDocument();
		const wrapper = container.querySelector('div.flex.flex-nowrap.\\@container\\/buttons');
		expect(wrapper).toBeInTheDocument();
	});

	it('keeps the footer visible for provider-query pagination when the current page has no rows', async () => {
		vi.mocked(invokeSurfaceInteraction)
			.mockReset()
			.mockResolvedValueOnce({
				data: {
					items: [],
					total: 60,
					page: 1,
					per_page: 20,
					total_pages: 3
				}
			} as unknown as Awaited<ReturnType<typeof invokeSurfaceInteraction>>)
			.mockResolvedValueOnce({
				data: {
					items: [],
					total: 60,
					page: 1,
					per_page: 20,
					total_pages: 3
				}
			} as unknown as Awaited<ReturnType<typeof invokeSurfaceInteraction>>);

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

		const { container } = render(SurfaceTable, {
			surfaceId: 'notifications.email',
			node,
			dataSource,
			dataLoadInteraction: interactions[0],
			interactions
		});

		expect(await screen.findByText('No rows available')).toBeInTheDocument();
		expect(container.querySelector('[data-ui="table-footer-bar"]')).toBeInTheDocument();
		expect(screen.getByText('60 total')).toBeInTheDocument();
	});
});
