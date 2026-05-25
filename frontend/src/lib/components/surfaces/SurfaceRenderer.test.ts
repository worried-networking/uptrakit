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
				label: 'Submit form',
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

	it('renders shared callout and empty-state primitives for parity branches', () => {
		const node: SurfaceNode = {
			kind: 'section',
			children: [
				{
					kind: 'callout',
					level: 'warning',
					text: 'Provider response is delayed.'
				},
				{
					kind: 'empty_state',
					title: 'No hosts connected',
					description: 'Connect a host to continue.'
				}
			]
		};

		const { container } = render(SurfaceRenderer, {
			surfaceId: 'surface.page',
			node
		});

		expect(screen.getByText('Provider response is delayed.')).toBeInTheDocument();
		expect(screen.getByText('No hosts connected')).toBeInTheDocument();
		expect(container.querySelector('[data-ui="callout"]')).toBeInTheDocument();
		expect(container.querySelector('[data-ui="empty-state"]')).toBeInTheDocument();
	});

	it('uses human-facing fallback copy for missing trigger interactions', () => {
		const node: SurfaceNode = {
			kind: 'section',
			children: [
				{
					kind: 'modal_trigger',
					interaction_id: 'provider.action.launch'
				},
				{
					kind: 'workflow_trigger',
					interaction_id: 'provider.workflow.launch'
				}
			]
		};

		render(SurfaceRenderer, {
			surfaceId: 'surface.page',
			node
		});

		expect(screen.getAllByText('Action unavailable')).toHaveLength(2);
		expect(screen.queryByText(/provider\.action\.launch/i)).not.toBeInTheDocument();
		expect(screen.queryByText(/provider\.workflow\.launch/i)).not.toBeInTheDocument();
	});

	it('keeps labeled modal triggers on shared trigger and modal-shell treatment', async () => {
		const node: SurfaceNode = {
			kind: 'modal_trigger',
			interaction_id: 'provider.action.open',
			modal_nodes: [
				{
					kind: 'text_block',
					text: 'Modal details'
				}
			]
		};
		const interaction: InteractionDescriptor = {
			interaction_id: 'provider.action.open',
			kind: 'mutation_action',
			label: 'Open modal',
			transport: { mode: 'controller_local' }
		};

		render(SurfaceRenderer, {
			surfaceId: 'surface.page',
			node,
			interactions: [interaction]
		});

		const trigger = screen.getByRole('button', { name: 'Open modal' });
		expect(trigger).toBeInTheDocument();
		expect(trigger).toHaveAttribute('data-ui', 'modal-trigger');
		expect(screen.queryByText('provider.action.open')).not.toBeInTheDocument();

		await fireEvent.click(trigger);

		expect(screen.getByRole('heading', { name: 'Open modal' })).toBeInTheDocument();
		expect(screen.getByText('Modal details')).toBeInTheDocument();
		// Modal portals to <body>; query the document, not the test container.
		expect(document.body.querySelector('[data-ui="modal-shell"]')).toBeInTheDocument();
	});

	it('modal-trigger button is secondary variant — no preset-tonal-surface class', () => {
		const node: SurfaceNode = {
			kind: 'modal_trigger',
			interaction_id: 'provider.action.open',
			modal_nodes: []
		};
		const interaction: InteractionDescriptor = {
			interaction_id: 'provider.action.open',
			kind: 'mutation_action',
			label: 'Open modal',
			transport: { mode: 'controller_local' }
		};

		render(SurfaceRenderer, {
			surfaceId: 'surface.page',
			node,
			interactions: [interaction]
		});

		const trigger = screen.getByRole('button', { name: 'Open modal' });
		expect(trigger.className).not.toMatch(/preset-tonal|preset-filled/);
		// secondary variant uses bg-[var(--bg-raised)]
		expect(trigger.className).toContain('bg-[var(--bg-raised)]');
	});

	it('shows an unavailable callout for unlabeled modal triggers', () => {
		const node: SurfaceNode = {
			kind: 'modal_trigger',
			interaction_id: 'provider.action.open'
		};
		const interaction = {
			interaction_id: 'provider.action.open',
			kind: 'mutation_action',
			label: undefined,
			transport: { mode: 'controller_local' }
		} as unknown as InteractionDescriptor;

		render(SurfaceRenderer, {
			surfaceId: 'surface.page',
			node,
			interactions: [interaction]
		});

		expect(screen.getByText('Action unavailable')).toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Open details' })).not.toBeInTheDocument();
	});

	it('renders section with header_action_ids as SectionCard with header button', async () => {
		const node: SurfaceNode = {
			kind: 'section',
			title: 'My Section',
			header_action_ids: ['create-item'],
			children: []
		};
		const interactions: InteractionDescriptor[] = [
			{
				interaction_id: 'create-item',
				kind: 'mutation_action',
				label: 'Create Item',
				transport: { mode: 'controller_local' }
			}
		];
		const { container } = render(SurfaceRenderer, {
			surfaceId: 'test.surface',
			node,
			interactions
		});
		expect(container.querySelector('[data-ui="section-card"]')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Create Item' })).toBeInTheDocument();
	});

	it('renders section without header_action_ids as plain div (no SectionCard)', () => {
		const node: SurfaceNode = {
			kind: 'section',
			title: 'Plain Section',
			children: []
		};
		const { container } = render(SurfaceRenderer, {
			surfaceId: 'test.surface',
			node
		});
		expect(container.querySelector('[data-ui="section-card"]')).not.toBeInTheDocument();
		expect(screen.getByText('Plain Section')).toBeInTheDocument();
	});
});
