import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/svelte';

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
		permissions: []
	}))
}));

vi.mock('$lib/surfaces/registry.svelte', () => ({
	getSurfaceById: vi.fn(() => ({
		surface_id: 'surface.one',
		label: 'Surface One',
		priority: 100,
		slot: 'extension.page',
		scope: 'tenant',
		targeting: 'universal',
		provider_kind: 'service',
		required_capabilities: [],
		root_node: { kind: 'text_block', text: 'surface' },
		provider_count: 1
	})),
	getSurfaceReadModel: vi.fn(() => undefined),
	getSurfaceReadRequested: vi.fn(() => false),
	getSurfaceReadLoading: vi.fn(() => false),
	getSurfaceRegistryLoaded: vi.fn(() => true),
	getSurfaceRuntimeStatus: vi.fn(() => ({ active: true })),
	loadSurfaceReadModels: vi.fn(async () => {})
}));

import SurfacesPage from './[id]/+page.svelte';
import { getUser } from '$lib/auth.svelte';
import {
	getSurfaceById,
	getSurfaceRegistryLoaded,
	getSurfaceRuntimeStatus,
	loadSurfaceReadModels
} from '$lib/surfaces/registry.svelte';

describe('/surfaces/[id] canonical surface page', () => {
	beforeEach(() => {
		vi.mocked(getSurfaceRuntimeStatus).mockReturnValue({ active: true });
		vi.mocked(getSurfaceRegistryLoaded).mockReturnValue(true);
		vi.mocked(getSurfaceById).mockReturnValue({
			surface_id: 'surface.one',
			label: 'Surface One',
			priority: 100,
			slot: 'extension.page',
			scope: 'tenant',
			targeting: 'universal',
			provider_kind: 'service',
			required_capabilities: [],
			root_node: { kind: 'text_block', text: 'surface' },
			provider_count: 1
		});
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('keeps a loading state while the surface read payload is still pending', () => {
		render(SurfacesPage);

		expect(screen.getByText('Loading...')).toBeInTheDocument();
		expect(screen.queryByText('Surface not found')).not.toBeInTheDocument();
	});

	it('keeps loading while the surface registry is still loading even when rollout is inactive', () => {
		vi.mocked(getSurfaceRuntimeStatus).mockReturnValue({ active: false });
		vi.mocked(getSurfaceRegistryLoaded).mockReturnValue(false);
		vi.mocked(getSurfaceById).mockReturnValue(undefined);

		render(SurfacesPage);

		expect(screen.getByText('Loading...')).toBeInTheDocument();
		expect(screen.queryByText('Surface not found')).not.toBeInTheDocument();
	});

	it('shows surface access denied without falling back to legacy extension content', () => {
		vi.mocked(getSurfaceById).mockReturnValue({
			surface_id: 'surface.one',
			label: 'Surface One',
			priority: 100,
			slot: 'extension.page',
			scope: 'tenant',
			targeting: 'universal',
			required_permission: 'view_settings',
			provider_kind: 'service',
			required_capabilities: [],
			root_node: { kind: 'text_block', text: 'surface' },
			provider_count: 1
		});
		vi.mocked(getUser).mockReturnValue({
			id: 'user-1',
			email: 'user@example.com',
			first_name: 'Test',
			last_name: 'User',
			permissions: []
		});

		render(SurfacesPage);

		expect(screen.getByText('Access denied')).toBeInTheDocument();
		expect(screen.getByText('You do not have permission to access this surface.')).toBeInTheDocument();
		expect(screen.queryByText('Legacy Extension')).not.toBeInTheDocument();
		expect(vi.mocked(loadSurfaceReadModels)).not.toHaveBeenCalled();
	});
});
