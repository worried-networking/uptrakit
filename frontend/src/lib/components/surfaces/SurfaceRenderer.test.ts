import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import SurfaceRenderer from './SurfaceRenderer.svelte';

vi.mock('$lib/api', () => ({
	invokeSurfaceInteraction: vi.fn(),
	sealedBoxEncrypt: vi.fn()
}));

vi.mock('$lib/notifications.svelte', () => ({
	showError: vi.fn(),
	showSuccess: vi.fn()
}));

import { invokeSurfaceInteraction, sealedBoxEncrypt } from '$lib/api';
import type { InteractionDescriptor, SurfaceNode } from '$lib/surfaces/contract';

describe('SurfaceRenderer', () => {
	beforeEach(() => {
		vi.mocked(sealedBoxEncrypt).mockResolvedValue('ciphertext');
		vi.mocked(invokeSurfaceInteraction).mockResolvedValue({});
	});

	afterEach(() => {
		cleanup();
		vi.clearAllMocks();
	});

	it('propagates encryption context through section recursion to nested forms', async () => {
		const node: SurfaceNode = {
			kind: 'section',
			children: [
				{
					kind: 'form',
					interaction_id: 'surface.submit'
				}
			]
		};

		const interactions: InteractionDescriptor[] = [
			{
				interaction_id: 'surface.submit',
				kind: 'form_submit',
				transport: { mode: 'provider_proxied' },
				sensitive_fields: ['password']
			}
		];

		render(SurfaceRenderer, {
			surfaceId: 'surface.page',
			node,
			interactions,
			targetProviderId: 'provider.a',
			encryptionContext: {
				keyId: 'enc-key',
				algorithm: 'ecies_p256',
				publicKey: 'public-key'
			}
		});

		const payload = screen.getByRole('textbox', { name: 'JSON Payload' });
		await fireEvent.input(payload, {
			target: { value: '{"username":"admin","password":"secret"}' }
		});
		await fireEvent.submit(payload.closest('form')!);

		await waitFor(() => {
			expect(invokeSurfaceInteraction).toHaveBeenCalledTimes(1);
		});

		expect(sealedBoxEncrypt).toHaveBeenCalledWith(JSON.stringify({ password: 'secret' }), 'public-key');
		expect(invokeSurfaceInteraction).toHaveBeenCalledWith('surface.page', 'surface.submit', {
			params: { username: 'admin' },
			target_provider_id: 'provider.a',
			timeout_seconds: undefined,
			encrypted_sensitive_params: {
				key_id: 'enc-key',
				algorithm: 'ecies_p256',
				ciphertext_b64: 'ciphertext'
			}
		});
	});
});
