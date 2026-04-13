import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/svelte';
import SurfaceReadPanel from './SurfaceReadPanel.svelte';
import type { SurfaceReadResponse, SurfaceResponse } from '$lib/surfaces/contract';

vi.mock('$lib/surfaces/registry.svelte', () => ({
	getSurfaceProviders: vi.fn(() => [])
}));

function makeSurface(): SurfaceResponse {
	return {
		surface_id: 'surface.one',
		label: 'Surface One',
		priority: 100,
		slot: 'extension.page',
		scope: 'tenant',
		targeting: 'universal',
		provider_kind: 'service',
		required_capabilities: [],
		root_node: { kind: 'text_block', text: 'list descriptor node' },
		provider_count: 1
	};
}

function makeRead(surfaceId = 'surface.one'): SurfaceReadResponse {
	return {
		descriptor: {
			surface_id: surfaceId,
			label: 'Read Descriptor',
			priority: 100,
			slot: 'extension.page',
			scope: 'tenant',
			targeting: 'universal',
			provider_kind: 'service',
			required_capabilities: [],
			root_node: { kind: 'text_block', text: 'read descriptor node' }
		},
		interactions: [],
		data_sources: []
	};
}

describe('SurfaceReadPanel', () => {
	afterEach(() => {
		cleanup();
		vi.clearAllMocks();
	});

	it('renders from read.descriptor instead of the list descriptor when read is present', () => {
		render(SurfaceReadPanel, {
			surface: makeSurface(),
			read: makeRead()
		});

		expect(screen.getByText('read descriptor node')).toBeInTheDocument();
		expect(screen.queryByText('list descriptor node')).not.toBeInTheDocument();
	});

	it('rejects mismatched descriptors instead of mixing list and read metadata', () => {
		render(SurfaceReadPanel, {
			surface: makeSurface(),
			read: makeRead('surface.two')
		});

		expect(screen.getByText('Surface contract mismatch detected. Please refresh and try again.')).toBeInTheDocument();
		expect(screen.queryByText('read descriptor node')).not.toBeInTheDocument();
	});
});
