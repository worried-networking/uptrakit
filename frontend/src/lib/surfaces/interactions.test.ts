import { describe, expect, it, vi } from 'vitest';
import type { InteractionDescriptor, SurfaceResponse } from './contract';
import { buildSurfaceInteractionRequest, clampSurfaceTabIndex, getSurfaceDescriptorRenderKey } from './interactions';

function makeInteraction(overrides: Partial<InteractionDescriptor> = {}): InteractionDescriptor {
	return {
		interaction_id: 'surface.submit',
		kind: 'form_submit',
		label: 'Submit',
		transport: { mode: 'provider_proxied' },
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
