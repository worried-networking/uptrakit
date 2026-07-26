import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { InteractionDescriptor, SurfaceResponse } from './contract';
import {
	actionRefId,
	actionRefMethod,
	buildSurfaceInteractionRequest,
	clampSurfaceTabIndex,
	dispatchSurfaceInteraction,
	getSurfaceDescriptorRenderKey,
	resolveInteraction
} from './interactions';

vi.mock('$lib/api', async (importOriginal) => {
	const actual = await importOriginal<typeof import('$lib/api')>();
	return {
		...actual,
		readSurfaceInteraction: vi.fn(async () => ({ data: { ok: 'read' } })),
		readSurfaceInteractionItem: vi.fn(async () => ({ data: { ok: 'read-item' } })),
		updateSurfaceInteraction: vi.fn(async () => ({ data: { ok: 'update' } })),
		updateSurfaceInteractionItem: vi.fn(async () => ({ data: { ok: 'update-item' } })),
		deleteSurfaceInteraction: vi.fn(async () => ({ data: { ok: 'delete' } })),
		deleteSurfaceInteractionItem: vi.fn(async () => ({ data: { ok: 'delete-item' } })),
		invokeSurfaceInteraction: vi.fn(async () => ({ data: { ok: 'invoke' } })),
		sealedBoxEncrypt: vi.fn(async () => 'ciphertext')
	};
});

function makeInteraction(overrides: Partial<InteractionDescriptor> = {}): InteractionDescriptor {
	return {
		interaction_id: 'surface.submit',
		kind: 'form_submit',
		label: 'Submit',
		transport: { mode: 'provider_proxied' },
		http_method: 'post',
		...overrides
	};
}

function makeSurface(overrides: Partial<SurfaceResponse> = {}): SurfaceResponse {
	return {
		surface_id: 'surface.page',
		label: 'Surface',
		priority: 100,
		slot: 'surface.page',
		scope: 'tenant',
		targeting: 'universal',
		provider_kind: 'service',
		required_capabilities: [],
		root_node: { kind: 'text_block', text: 'hello' },
		provider_count: 1,
		...overrides
	};
}

describe('buildSurfaceInteractionRequest', () => {
	it('passes timeout_seconds and target_provider_id to request payload', async () => {
		const interaction = makeInteraction({ timeout_seconds: 42 });

		const request = await buildSurfaceInteractionRequest(
			interaction,
			{ foo: 'bar' },
			{ targetProviderId: 'provider-1' }
		);

		expect(request.timeout_seconds).toBe(42);
		expect(request.target_provider_id).toBe('provider-1');
		expect(request.params).toEqual({ foo: 'bar' });
	});

	it('splits sensitive fields and emits encrypted_sensitive_params', async () => {
		const interaction = makeInteraction({ sensitive_fields: ['password'] });
		const encrypt = vi.fn(async () => 'ciphertext');

		const request = await buildSurfaceInteractionRequest(
			interaction,
			{ username: 'admin', password: 'secret' },
			{
				encryption: {
					keyId: 'k1',
					algorithm: 'ecies_p256',
					publicKey: 'pub'
				},
				encryptSensitivePayload: encrypt
			}
		);

		expect(request.params).toEqual({ username: 'admin' });
		expect(request.encrypted_sensitive_params).toEqual({
			key_id: 'k1',
			algorithm: 'ecies_p256',
			ciphertext_b64: 'ciphertext'
		});
		expect(encrypt).toHaveBeenCalledWith(JSON.stringify({ password: 'secret' }), 'pub');
	});

	it('fails closed when sensitive_fields are declared but encryption metadata is missing', async () => {
		const interaction = makeInteraction({ sensitive_fields: ['token'] });

		await expect(buildSurfaceInteractionRequest(interaction, { token: 'abc' })).rejects.toThrow(
			'declares sensitive_fields'
		);
	});

	it('allows cleartext sensitive params for controller_local interactions without encryption metadata', async () => {
		const interaction = makeInteraction({
			sensitive_fields: ['bot_token'],
			transport: { mode: 'controller_local' }
		});

		await expect(buildSurfaceInteractionRequest(interaction, { bot_token: 'abc' })).resolves.toEqual({
			params: { bot_token: 'abc' },
			target_provider_id: undefined,
			timeout_seconds: undefined
		});
	});

	it('allows payloads without sensitive params even when sensitive_fields are declared', async () => {
		const interaction = makeInteraction({ sensitive_fields: ['token'] });

		await expect(buildSurfaceInteractionRequest(interaction, { username: 'alice' })).resolves.toEqual({
			params: { username: 'alice' },
			target_provider_id: undefined,
			timeout_seconds: undefined
		});
	});
});

describe('getSurfaceDescriptorRenderKey', () => {
	it('distinguishes descriptors with the same surface_id', () => {
		const first = makeSurface({ surface_id: 'surface.dup', label: 'A' });
		const second = makeSurface({ surface_id: 'surface.dup', label: 'B' });

		expect(getSurfaceDescriptorRenderKey(first)).not.toBe(getSurfaceDescriptorRenderKey(second));
	});
});

describe('clampSurfaceTabIndex', () => {
	it('clamps to a valid index range', () => {
		expect(clampSurfaceTabIndex(-1, 3)).toBe(0);
		expect(clampSurfaceTabIndex(1, 3)).toBe(1);
		expect(clampSurfaceTabIndex(10, 3)).toBe(2);
		expect(clampSurfaceTabIndex(0, 0)).toBe(0);
	});
});

describe('resolveInteraction', () => {
	it('resolves an exact (id, method) pair when both descriptors share an interaction_id', () => {
		const readVariant = makeInteraction({
			interaction_id: 'items',
			http_method: 'get',
			label: 'View items'
		});
		const writeVariant = makeInteraction({
			interaction_id: 'items',
			http_method: 'put',
			label: 'Edit items'
		});
		const interactions = [readVariant, writeVariant];

		expect(resolveInteraction(interactions, 'items', 'get')?.label).toBe('View items');
		expect(resolveInteraction(interactions, 'items', 'put')?.label).toBe('Edit items');
		expect(resolveInteraction(interactions, 'items')).toBeUndefined();
	});

	it('resolves a bare lookup when exactly one candidate matches the id', () => {
		const interaction = makeInteraction({ interaction_id: 'unique.action', http_method: 'post' });

		expect(resolveInteraction([interaction], 'unique.action')?.label).toBe('Submit');
		expect(resolveInteraction([interaction], 'missing.action')).toBeUndefined();
	});
});

describe('actionRefId / actionRefMethod', () => {
	it('extracts id and method from a bare string ref', () => {
		expect(actionRefId('create')).toBe('create');
		expect(actionRefMethod('create')).toBeUndefined();
	});

	it('extracts id and method from an object ref', () => {
		const ref = { interaction_id: 'delete-item', http_method: 'delete' as const };

		expect(actionRefId(ref)).toBe('delete-item');
		expect(actionRefMethod(ref)).toBe('delete');
	});
});

describe('dispatchSurfaceInteraction', () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it('routes a get-method interaction to readSurfaceInteraction with flattened string query params', async () => {
		const api = await import('$lib/api');
		const interaction = makeInteraction({ interaction_id: 'list', http_method: 'get' });

		const result = await dispatchSurfaceInteraction(
			'surface.page',
			interaction,
			{ params: { page: 1, per_page: 25 }, target_provider_id: 'provider-1', timeout_seconds: 30 },
			undefined
		);

		expect(api.readSurfaceInteraction).toHaveBeenCalledWith({
			path: { surface_id: 'surface.page', interaction_id: 'list' },
			query: {
				target_provider_id: 'provider-1',
				timeout_seconds: 30,
				page: '1',
				per_page: '25'
			}
		});
		expect(api.invokeSurfaceInteraction).not.toHaveBeenCalled();
		expect(result).toEqual({ ok: 'read' });
	});

	it('omits null/undefined param values from the query instead of stringifying them', async () => {
		const api = await import('$lib/api');
		const interaction = makeInteraction({ interaction_id: 'list', http_method: 'get' });

		await dispatchSurfaceInteraction('surface.page', interaction, {
			params: { keep: 'yes', dropNull: null, dropUndefined: undefined }
		});

		const call = vi.mocked(api.readSurfaceInteraction).mock.calls[0][0];
		expect(call.query).toEqual({
			target_provider_id: undefined,
			timeout_seconds: undefined,
			keep: 'yes'
		});
		expect(call.query).not.toHaveProperty('dropNull');
		expect(call.query).not.toHaveProperty('dropUndefined');
	});

	it('routes a get-method interaction with an itemId to readSurfaceInteractionItem', async () => {
		const api = await import('$lib/api');
		const interaction = makeInteraction({ interaction_id: 'item', http_method: 'get' });

		await dispatchSurfaceInteraction('surface.page', interaction, { params: {} }, { itemId: 'host-1' });

		expect(api.readSurfaceInteractionItem).toHaveBeenCalledWith({
			path: { surface_id: 'surface.page', interaction_id: 'item', item_id: 'host-1' },
			query: { target_provider_id: undefined, timeout_seconds: undefined }
		});
	});

	it('routes a put-method interaction with an itemId to updateSurfaceInteractionItem with the id in path', async () => {
		const api = await import('$lib/api');
		const interaction = makeInteraction({ interaction_id: 'item', http_method: 'put' });
		const request = { params: { name: 'renamed' } };

		await dispatchSurfaceInteraction('surface.page', interaction, request, { itemId: 'host-1' });

		expect(api.updateSurfaceInteractionItem).toHaveBeenCalledWith({
			path: { surface_id: 'surface.page', interaction_id: 'item', item_id: 'host-1' },
			body: request
		});
		expect(api.updateSurfaceInteraction).not.toHaveBeenCalled();
	});

	it('routes a put-method interaction without an itemId to updateSurfaceInteraction', async () => {
		const api = await import('$lib/api');
		const interaction = makeInteraction({ interaction_id: 'bulk', http_method: 'put' });
		const request = { params: { name: 'renamed' } };

		await dispatchSurfaceInteraction('surface.page', interaction, request);

		expect(api.updateSurfaceInteraction).toHaveBeenCalledWith({
			path: { surface_id: 'surface.page', interaction_id: 'bulk' },
			body: request
		});
	});

	it('routes a delete-method interaction with an itemId to deleteSurfaceInteractionItem', async () => {
		const api = await import('$lib/api');
		const interaction = makeInteraction({ interaction_id: 'item', http_method: 'delete' });
		const request = { params: {} };

		await dispatchSurfaceInteraction('surface.page', interaction, request, { itemId: 'host-1' });

		expect(api.deleteSurfaceInteractionItem).toHaveBeenCalledWith({
			path: { surface_id: 'surface.page', interaction_id: 'item', item_id: 'host-1' },
			body: request
		});
	});

	it('routes a delete-method interaction without an itemId to deleteSurfaceInteraction', async () => {
		const api = await import('$lib/api');
		const interaction = makeInteraction({ interaction_id: 'bulk', http_method: 'delete' });
		const request = { params: {} };

		await dispatchSurfaceInteraction('surface.page', interaction, request);

		expect(api.deleteSurfaceInteraction).toHaveBeenCalledWith({
			path: { surface_id: 'surface.page', interaction_id: 'bulk' },
			body: request
		});
	});

	it('routes a post-method (or default) interaction to invokeSurfaceInteraction unchanged', async () => {
		const api = await import('$lib/api');
		const interaction = makeInteraction({ interaction_id: 'surface.submit', http_method: 'post' });
		const request = { params: { name: 'value' } };

		const result = await dispatchSurfaceInteraction('surface.page', interaction, request);

		expect(api.invokeSurfaceInteraction).toHaveBeenCalledWith({
			path: { surface_id: 'surface.page', interaction_id: 'surface.submit' },
			body: request
		});
		expect(api.readSurfaceInteraction).not.toHaveBeenCalled();
		expect(result).toEqual({ ok: 'invoke' });
	});
});
