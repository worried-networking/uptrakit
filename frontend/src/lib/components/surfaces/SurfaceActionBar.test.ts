import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import SurfaceActionBar from './SurfaceActionBar.svelte';
import type { InteractionDescriptor } from '$lib/surfaces/contract';

vi.mock('$lib/api', () => ({
	invokeSurfaceInteraction: vi.fn(),
	sealedBoxEncrypt: vi.fn()
}));

vi.mock('$lib/notifications.svelte', () => ({
	showError: vi.fn(),
	showSuccess: vi.fn()
}));

import { invokeSurfaceInteraction, sealedBoxEncrypt } from '$lib/api';

describe('SurfaceActionBar', () => {
	beforeEach(() => {
		vi.mocked(sealedBoxEncrypt).mockResolvedValue('ciphertext');
		vi.mocked(invokeSurfaceInteraction).mockResolvedValue({});
	});

	afterEach(() => {
		cleanup();
		vi.clearAllMocks();
	});

	it('dispatches a reload event after a form action completes', async () => {
		const interaction: InteractionDescriptor = {
			interaction_id: 'create',
			kind: 'form_submit',
			label: 'Create Channel',
			transport: { mode: 'controller_local' },
			form_ui: {
				fields: [
					{
						key: 'name',
						label: 'Name',
						field_type: 'text',
						required: true
					}
				]
			}
		};
		const reloadListener = vi.fn();
		window.addEventListener('surface:reload', reloadListener);

		render(SurfaceActionBar, {
			surfaceId: 'notifications.email',
			actionIds: ['create'],
			interactions: [interaction]
		});

		await fireEvent.click(screen.getByRole('button', { name: 'Create Channel' }));
		await fireEvent.input(screen.getByRole('textbox'), {
			target: { value: 'Alerts' }
		});
		const buttons = screen.getAllByRole('button', { name: 'Create Channel' });
		await fireEvent.click(buttons[buttons.length - 1]);

		await waitFor(() => {
			expect(reloadListener).toHaveBeenCalledTimes(1);
		});
		window.removeEventListener('surface:reload', reloadListener);
	});

	it('renders a shared empty state when no actions are configured', () => {
		const { container } = render(SurfaceActionBar, {
			surfaceId: 'notifications.email',
			actionIds: [],
			interactions: []
		});

		expect(screen.getByText('No actions available')).toBeInTheDocument();
		expect(container.querySelector('[data-ui="empty-state"]')).toBeInTheDocument();
	});

	it('renders action buttons in the shared right-aligned action-row layout', () => {
		const interaction: InteractionDescriptor = {
			interaction_id: 'delete',
			kind: 'mutation_action',
			label: 'Delete',
			transport: { mode: 'controller_local' }
		};

		const { container } = render(SurfaceActionBar, {
			surfaceId: 'notifications.email',
			actionIds: ['delete'],
			interactions: [interaction]
		});

		expect(screen.getByRole('button', { name: 'Delete' })).toBeInTheDocument();
		const actionRow = container.querySelector('[data-ui="surface-action-bar"]');
		expect(actionRow).toBeInTheDocument();
		expect(actionRow?.className).toContain('justify-end');
		expect(actionRow?.className).toContain('gap-2');
		expect(actionRow?.className).toContain('flex-wrap');
	});

	it('passes requiredContextParam to button whose id is in requiredForInteractionIds', async () => {
		const discoverInteraction: InteractionDescriptor = {
			interaction_id: 'discover',
			kind: 'mutation_action',
			label: 'Discover',
			transport: { mode: 'controller_local' }
		};
		const testInteraction: InteractionDescriptor = {
			interaction_id: 'test-connection',
			kind: 'mutation_action',
			label: 'Test Connection',
			transport: { mode: 'controller_local' }
		};

		render(SurfaceActionBar, {
			surfaceId: 'proxmox.hosts',
			actionIds: ['discover', 'test-connection'],
			interactions: [discoverInteraction, testInteraction],
			baseParams: {},
			requiredContextParam: 'plugin_config_id',
			requiredForInteractionIds: ['discover', 'test-connection']
		});

		const discoverBtn = screen.getByRole('button', { name: 'Discover' });
		const testBtn = screen.getByRole('button', { name: 'Test Connection' });
		expect(discoverBtn).toBeDisabled();
		expect(testBtn).toBeDisabled();
	});
});
