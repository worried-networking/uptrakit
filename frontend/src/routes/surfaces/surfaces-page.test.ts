import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import { buildParitySurfacePageFixture } from '$lib/test-fixtures/ui-parity';
import type { SurfaceReadResponse, SurfaceResponse } from '$lib/surfaces/contract';

function buildSurfacePageParity() {
	return buildParitySurfacePageFixture();
}

function buildSurface(overrides: Partial<SurfaceResponse> = {}): SurfaceResponse {
	return {
		...buildSurfacePageParity().surface,
		...overrides
	};
}

function buildRead(surface: SurfaceResponse): SurfaceReadResponse {
	const { provider_count: _providerCount, ...descriptor } = surface;
	return {
		descriptor,
		interactions: [],
		data_sources: []
	};
}

vi.mock('$app/state', () => ({
	page: {
		params: { id: 'surface.one' },
		url: new URL('http://localhost/surfaces/surface.one')
	}
}));

vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => ({
		id: 'user-1',
		email: 'user@example.com',
		first_name: 'Test',
		last_name: 'User',
		has_pending_email_change: false,
		permissions: []
	}))
}));

vi.mock('$lib/surfaces/registry.svelte', () => ({
	getSurfaceById: vi.fn(() => buildSurfacePageParity().surface),
	getSurfaceReadModel: vi.fn(() => undefined),
	getSurfaceReadRequested: vi.fn(() => false),
	getSurfaceReadLoading: vi.fn(() => false),
	getSurfaceRegistryLoaded: vi.fn(() => true),
	loadSurfaceReadModels: vi.fn(async () => {}),
	getSurfaceProviders: vi.fn(() => buildSurfacePageParity().providers)
}));

vi.mock('$lib/api', () => ({
	invokeSurfaceInteraction: vi.fn()
}));

vi.mock('$app/navigation', () => ({
	goto: vi.fn(async () => {})
}));

import SurfacesPage from './[id]/+page.svelte';
import { goto } from '$app/navigation';
import { getUser } from '$lib/auth.svelte';
import { invokeSurfaceInteraction } from '$lib/api';
import {
	getSurfaceById,
	getSurfaceReadModel,
	getSurfaceReadLoading,
	getSurfaceRegistryLoaded,
	getSurfaceReadRequested,
	loadSurfaceReadModels,
	getSurfaceProviders
} from '$lib/surfaces/registry.svelte';

describe('/surfaces/[id] canonical surface page', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		vi.mocked(getUser).mockReturnValue({
			id: 'user-1',
			email: 'user@example.com',
			first_name: 'Test',
			last_name: 'User',
			has_pending_email_change: false,
			permissions: []
		});
		vi.mocked(getSurfaceRegistryLoaded).mockReturnValue(true);
		vi.mocked(getSurfaceById).mockReturnValue(buildSurfacePageParity().surface);
		vi.mocked(getSurfaceReadModel).mockReturnValue(undefined);
		vi.mocked(getSurfaceReadRequested).mockReturnValue(false);
		vi.mocked(getSurfaceReadLoading).mockReturnValue(false);
		vi.mocked(getSurfaceProviders).mockReturnValue(buildSurfacePageParity().providers);
		vi.mocked(invokeSurfaceInteraction).mockResolvedValue({});
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('keeps a loading state while the surface read payload is still pending', () => {
		render(SurfacesPage);

		expect(screen.getByText('Loading...')).toBeInTheDocument();
		expect(screen.queryByText('Surface not found')).not.toBeInTheDocument();
		expect(document.querySelector('[data-ui="page-shell"]')).toBeInTheDocument();
		expect(document.querySelector('[data-parity-region="surface.page"]')).toBeInTheDocument();
		expect(vi.mocked(loadSurfaceReadModels)).toHaveBeenCalledTimes(1);
	});

	it('keeps loading while the surface registry is still loading', () => {
		vi.mocked(getSurfaceRegistryLoaded).mockReturnValue(false);
		vi.mocked(getSurfaceById).mockReturnValue(undefined);

		render(SurfacesPage);

		expect(screen.getByText('Loading...')).toBeInTheDocument();
		expect(screen.queryByText('Surface not found')).not.toBeInTheDocument();
	});

	it('shows surface access denied without falling back to removed compatibility content', () => {
		vi.mocked(getSurfaceById).mockReturnValue({
			...buildSurfacePageParity().surface,
			required_permission: 'view_settings'
		});
		vi.mocked(getUser).mockReturnValue({
			id: 'user-1',
			email: 'user@example.com',
			first_name: 'Test',
			last_name: 'User',
			has_pending_email_change: false,
			permissions: []
		});

		render(SurfacesPage);

		expect(screen.getByText('Access denied')).toBeInTheDocument();
		expect(screen.getByText('You do not have permission to access this surface.')).toBeInTheDocument();
		const parityRegion = document.querySelector('[data-parity-region="surface.page"]');
		expect(parityRegion).toBeInTheDocument();
		expect(screen.queryByText('Compatibility Fallback')).not.toBeInTheDocument();
		expect(vi.mocked(loadSurfaceReadModels)).not.toHaveBeenCalled();
	});

	it('renders the loaded surface.page state inside the canonical host container', () => {
		const surface = buildSurface({
			root_node: { kind: 'text_block', text: 'list descriptor node' }
		});
		const read = buildRead(surface);
		vi.mocked(getSurfaceById).mockReturnValue(surface);
		vi.mocked(getSurfaceReadModel).mockReturnValue({
			...read,
			descriptor: {
				...read.descriptor,
				root_node: { kind: 'text_block', text: 'loaded descriptor node' }
			}
		});

		render(SurfacesPage);

		expect(screen.getByRole('heading', { name: 'Surface One' })).toBeInTheDocument();
		expect(screen.getByText('loaded descriptor node')).toBeInTheDocument();
		expect(document.querySelector('[data-parity-region="surface.page"]')).toBeInTheDocument();
		expect(screen.queryByText('Loading...')).not.toBeInTheDocument();
	});

	it('keeps the surface shell visible once loading has settled, even without a read model yet', () => {
		vi.mocked(getSurfaceReadRequested).mockReturnValue(true);
		vi.mocked(getSurfaceReadLoading).mockReturnValue(false);

		render(SurfacesPage);

		expect(screen.getByRole('heading', { name: 'Surface One' })).toBeInTheDocument();
		expect(screen.getByText('Surface contract mismatch detected. Please refresh and try again.')).toBeInTheDocument();
		expect(document.querySelector('[data-parity-region="surface.page"]')).toBeInTheDocument();
		expect(
			screen.queryByText('This surface is currently unavailable because its read contract cannot be rendered.')
		).not.toBeInTheDocument();
		expect(vi.mocked(loadSurfaceReadModels)).not.toHaveBeenCalled();
	});

	it('does not request the read model again while an existing request is still loading', () => {
		vi.mocked(getSurfaceReadRequested).mockReturnValue(true);
		vi.mocked(getSurfaceReadLoading).mockReturnValue(true);

		render(SurfacesPage);

		expect(screen.getByText('Loading...')).toBeInTheDocument();
		expect(vi.mocked(loadSurfaceReadModels)).not.toHaveBeenCalled();
	});

	it('renders the targeted no_compatible_provider state with canonical empty-state copy', () => {
		const targetedSurface = buildSurface({
			targeting: 'targeted',
			provider_kind: 'plugin'
		});
		vi.mocked(getSurfaceById).mockReturnValue(targetedSurface);
		vi.mocked(getSurfaceReadModel).mockReturnValue(buildRead(targetedSurface));
		vi.mocked(getSurfaceProviders).mockReturnValue([
			{
				provider_id: 'provider.disconnected',
				display_label: 'Provider Disconnected',
				availability: 'disconnected'
			},
			{
				provider_id: 'provider.incompatible',
				display_label: 'Provider Incompatible',
				availability: 'incompatible_tenant'
			}
		]);

		render(SurfacesPage);

		expect(screen.getByRole('heading', { name: 'Surface One' })).toBeInTheDocument();
		expect(screen.getByText('No provider connected')).toBeInTheDocument();
		expect(screen.getByText('Connect a compatible service to use this surface.')).toBeInTheDocument();
		expect(screen.queryByText('No compatible provider connected')).not.toBeInTheDocument();
		expect(vi.mocked(invokeSurfaceInteraction)).not.toHaveBeenCalled();
	});

	it('renders the contract_mismatch state when read metadata does not match the route surface', () => {
		const surface = buildSurface();
		const read = buildRead(surface);
		vi.mocked(getSurfaceById).mockReturnValue(surface);
		vi.mocked(getSurfaceReadModel).mockReturnValue({
			...read,
			descriptor: {
				...read.descriptor,
				surface_id: 'surface.mismatch'
			}
		});

		render(SurfacesPage);

		expect(screen.getByRole('heading', { name: 'Surface One' })).toBeInTheDocument();
		expect(screen.getByText('Surface contract mismatch detected. Please refresh and try again.')).toBeInTheDocument();
		expect(screen.queryByText('Surface contract is not available yet.')).not.toBeInTheDocument();
		expect(document.querySelector('[data-parity-region="surface.page"]')).toBeInTheDocument();
	});

	it('renders the hydration_action_failure state when runtime hydration fails', async () => {
		const surface = buildSurface();
		const read = buildRead(surface);
		vi.mocked(getSurfaceById).mockReturnValue(surface);
		vi.mocked(getSurfaceReadModel).mockReturnValue({
			...read,
			descriptor: {
				...read.descriptor,
				root_node: {
					kind: 'key_value',
					data_source_id: 'data.remote'
				}
			},
			interactions: [
				{
					interaction_id: 'surface.load',
					kind: 'data_load',
					label: 'Load Surface',
					transport: { mode: 'controller_local' }
				}
			],
			data_sources: [
				{
					data_source_id: 'data.remote',
					kind: { kind: 'provider_query', operation_id: 'surface.load' },
					result_schema: 'object',
					refresh_policy: { type: 'manual' }
				}
			]
		});
		vi.mocked(invokeSurfaceInteraction).mockRejectedValue(new Error('boom'));

		render(SurfacesPage);

		expect(screen.getByRole('heading', { name: 'Surface One' })).toBeInTheDocument();
		expect(await screen.findByText('Unable to load surface data')).toBeInTheDocument();
		expect(screen.getByText('Failed to load surface data. Please try again.')).toBeInTheDocument();
		expect(vi.mocked(invokeSurfaceInteraction)).toHaveBeenCalledWith(
			'surface.one',
			'surface.load',
			expect.objectContaining({
				params: {}
			})
		);
	});

	it('renders normally when the page component is mounted (goto not called on mount)', () => {
		// The $app/state mock at the top of this file has a URL without page params.
		// This verifies the route component does not spontaneously call goto on initial render.
		const surface = buildSurface({
			root_node: { kind: 'text_block', text: 'surface content' }
		});
		const read = buildRead(surface);
		vi.mocked(getSurfaceById).mockReturnValue(surface);
		vi.mocked(getSurfaceReadModel).mockReturnValue(read);

		render(SurfacesPage);

		expect(screen.getByText('surface content')).toBeInTheDocument();
		expect(vi.mocked(goto)).not.toHaveBeenCalled();
	});
});
