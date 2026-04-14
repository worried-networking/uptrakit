import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import SurfaceInteractionButton from './SurfaceInteractionButton.svelte';
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

describe('SurfaceInteractionButton', () => {
	beforeEach(() => {
		vi.mocked(sealedBoxEncrypt).mockResolvedValue('ciphertext');
		vi.mocked(invokeSurfaceInteraction).mockResolvedValue({});
	});

	afterEach(() => {
		cleanup();
		vi.clearAllMocks();
	});

	it('opens form interactions with their label and submits merged params', async () => {
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

		render(SurfaceInteractionButton, {
			surfaceId: 'notifications.email',
			interaction,
			interactions: [interaction],
			baseParams: { channel_type: 'email' }
		});

		await fireEvent.click(screen.getByRole('button', { name: 'Create Channel' }));
		await fireEvent.input(screen.getByRole('textbox'), {
			target: { value: 'Alerts' }
		});
		const buttons = screen.getAllByRole('button', { name: 'Create Channel' });
		await fireEvent.click(buttons[buttons.length - 1]);

		await waitFor(() => {
			expect(invokeSurfaceInteraction).toHaveBeenCalledWith('notifications.email', 'create', {
				params: {
					channel_type: 'email',
					name: 'Alerts'
				},
				target_provider_id: undefined,
				timeout_seconds: undefined
			});
		});
	});
});
