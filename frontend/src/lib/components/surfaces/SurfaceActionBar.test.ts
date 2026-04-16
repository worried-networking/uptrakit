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
});
