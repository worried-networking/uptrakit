import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type {
	InvokeSurfaceInteractionRequest,
	SurfaceProviderInfo,
	SurfaceReadResponse,
	SurfaceResponse
} from '../surfaces/contract';

// client.ts registers an onTokenChange listener at module-init; provide a no-op stub.
vi.mock('../token-store.svelte', () => ({
	getAccessToken: vi.fn().mockReturnValue('tok'),
	setAccessToken: vi.fn(),
	onTokenChange: vi.fn(() => () => {}),
	getSessionExpired: vi.fn().mockReturnValue(false),
	setSessionExpired: vi.fn()
}));

import { listSurfaces, listSurfaceProviders, getSurfaceRead, invokeSurfaceInteraction } from './surfaces';

// ── Fixtures ──────────────────────────────────────────────────────────────────

const mockSurface: SurfaceResponse = {
	surface_id: 'home.overview',
	label: 'Overview',
	priority: 10,
	slot: 'home',
	scope: 'global',
	targeting: 'universal',
	provider_kind: 'built_in',
	required_capabilities: [],
	root_node: { kind: 'section' },
	provider_count: 2
};

const mockProvider: SurfaceProviderInfo = {
	provider_id: 'prov-1',
	display_label: 'Built-in Provider',
	availability: 'available'
};

const mockReadResponse: SurfaceReadResponse = {
	descriptor: {
		surface_id: 'home.overview',
		label: 'Overview',
		priority: 10,
		slot: 'home',
		scope: 'global',
		targeting: 'universal',
		provider_kind: 'built_in',
		required_capabilities: [],
		root_node: { kind: 'section' }
	},
	interactions: [],
	data_sources: []
};

function jsonResponse(payload: unknown, status = 200): Response {
	return new Response(JSON.stringify(payload), {
		status,
		headers: { 'Content-Type': 'application/json' }
	});
}

// ── listSurfaces ──────────────────────────────────────────────────────────────

describe('listSurfaces', () => {
	afterEach(() => {
		vi.restoreAllMocks();
	});

	it('GETs /surfaces with no query when called with no options', async () => {
		const spy = vi.spyOn(globalThis, 'fetch').mockResolvedValue(jsonResponse([mockSurface]));

		const result = await listSurfaces();

		const req = spy.mock.calls[0][0] as Request;
		expect(req.method).toBe('GET');
		expect(req.url).toContain('/surfaces');
		expect(req.url).not.toContain('slot=');
		expect(req.url).not.toContain('page=');
		expect(result).toEqual([mockSurface]);
	});

	it('appends slot query param when slot option is provided', async () => {
		const spy = vi.spyOn(globalThis, 'fetch').mockResolvedValue(jsonResponse([mockSurface]));

		await listSurfaces({ slot: 'home' });

		const req = spy.mock.calls[0][0] as Request;
		expect(req.url).toContain('slot=home');
	});

	it('appends page query param when page option is provided', async () => {
		const spy = vi.spyOn(globalThis, 'fetch').mockResolvedValue(jsonResponse([]));

		await listSurfaces({ page: '3' });

		const req = spy.mock.calls[0][0] as Request;
		expect(req.url).toContain('page=3');
	});

	it('appends both slot and page when both options are provided', async () => {
		const spy = vi.spyOn(globalThis, 'fetch').mockResolvedValue(jsonResponse([mockSurface]));

		const result = await listSurfaces({ slot: 'dashboard', page: '2' });

		const req = spy.mock.calls[0][0] as Request;
		expect(req.url).toContain('slot=dashboard');
		expect(req.url).toContain('page=2');
		expect(result).toEqual([mockSurface]);
	});
});

// ── listSurfaceProviders ──────────────────────────────────────────────────────

describe('listSurfaceProviders', () => {
	afterEach(() => {
		vi.restoreAllMocks();
	});

	it('GETs /surfaces/:id/providers and returns the provider list', async () => {
		const spy = vi.spyOn(globalThis, 'fetch').mockResolvedValue(jsonResponse([mockProvider]));

		const result = await listSurfaceProviders('home.overview');

		const req = spy.mock.calls[0][0] as Request;
		expect(req.method).toBe('GET');
		expect(req.url).toContain('/surfaces/home.overview/providers');
		expect(result).toEqual([mockProvider]);
	});

	it('percent-encodes surfaceId with special characters', async () => {
		vi.spyOn(globalThis, 'fetch').mockResolvedValue(jsonResponse([]));

		await listSurfaceProviders('host/detail');

		const spy = vi.mocked(fetch);
		const req = spy.mock.calls[0][0] as Request;
		expect(req.url).toContain('host%2Fdetail');
		expect(req.url).not.toContain('host/detail/providers');
	});
});

// ── getSurfaceRead ────────────────────────────────────────────────────────────

describe('getSurfaceRead', () => {
	afterEach(() => {
		vi.restoreAllMocks();
	});

	it('GETs /surfaces/:id/read and returns the read response', async () => {
		const spy = vi.spyOn(globalThis, 'fetch').mockResolvedValue(jsonResponse(mockReadResponse));

		const result = await getSurfaceRead('home.overview');

		const req = spy.mock.calls[0][0] as Request;
		expect(req.method).toBe('GET');
		expect(req.url).toContain('/surfaces/home.overview/read');
		expect(result).toEqual(mockReadResponse);
	});
});

// ── invokeSurfaceInteraction ──────────────────────────────────────────────────

describe('invokeSurfaceInteraction', () => {
	beforeEach(() => {
		vi.restoreAllMocks();
	});

	it('POSTs to /surfaces/:id/interactions/:iid with the serialized body', async () => {
		const responsePayload = { result: 'ok', value: 42 };
		const spy = vi.spyOn(globalThis, 'fetch').mockResolvedValue(jsonResponse(responsePayload));

		const body: InvokeSurfaceInteractionRequest = {
			params: { key: 'value' },
			target_provider_id: 'prov-1'
		};
		const result = await invokeSurfaceInteraction('home.overview', 'refresh', body);

		const req = spy.mock.calls[0][0] as Request;
		expect(req.method).toBe('POST');
		expect(req.url).toContain('/surfaces/home.overview/interactions/refresh');
		const bodyText = await req.clone().text();
		expect(JSON.parse(bodyText)).toEqual(body);
		expect(result).toEqual(responsePayload);
	});

	it('percent-encodes both surfaceId and interactionId', async () => {
		vi.spyOn(globalThis, 'fetch').mockResolvedValue(jsonResponse({}));

		await invokeSurfaceInteraction('my surface', 'my action', {});

		const req = vi.mocked(fetch).mock.calls[0][0] as Request;
		expect(req.url).toContain('my%20surface');
		expect(req.url).toContain('my%20action');
	});
});
