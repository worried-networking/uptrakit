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

	it('routes workflow interactions through their step submit interactions', async () => {
		const workflowInteraction: InteractionDescriptor = {
			interaction_id: 'bootstrap',
			kind: 'workflow',
			label: 'Bootstrap Host',
			transport: { mode: 'provider_proxied' },
			workflow_steps: [
				{
					step_id: 'connect',
					label: 'Connect',
					input_schema: 'object',
					result_schema: 'any',
					submit_interaction_id: 'bootstrap-connect',
					form_ui: {
						fields: [{ key: 'target', label: 'Target', field_type: 'text', required: true }]
					}
				}
			]
		};
		const stepInteraction: InteractionDescriptor = {
			interaction_id: 'bootstrap-connect',
			kind: 'mutation_action',
			label: 'Bootstrap Connect',
			transport: { mode: 'provider_proxied' }
		};

		render(SurfaceInteractionButton, {
			surfaceId: 'ssh-agent.hosts',
			interaction: workflowInteraction,
			interactions: [workflowInteraction, stepInteraction],
			baseParams: { id: 'host-1' }
		});

		await fireEvent.click(screen.getByRole('button', { name: 'Bootstrap Host' }));
		await fireEvent.input(screen.getByRole('textbox', { name: /Target/i }), {
			target: { value: 'root@example:22' }
		});
		await fireEvent.click(screen.getByRole('button', { name: 'Run' }));

		await waitFor(() => {
			expect(invokeSurfaceInteraction).toHaveBeenCalledWith(
				'ssh-agent.hosts',
				'bootstrap-connect',
				expect.objectContaining({
					params: {
						id: 'host-1',
						target: 'root@example:22'
					}
				})
			);
		});
	});

	it('uses the interaction label for provider-authored forms instead of fallback copy', async () => {
		const interaction: InteractionDescriptor = {
			interaction_id: 'provider.action.reset',
			kind: 'form_submit',
			label: 'Reset Provider',
			transport: { mode: 'controller_local' },
			form_ui: {
				fields: [{ key: 'name', label: 'Name', field_type: 'text', required: true }]
			}
		};

		render(SurfaceInteractionButton, {
			surfaceId: 'provider.surface',
			interaction,
			interactions: [interaction]
		});

		await fireEvent.click(screen.getByRole('button', { name: 'Reset Provider' }));
		expect(screen.getByRole('heading', { name: 'Reset Provider' })).toBeInTheDocument();
		expect(screen.queryByText('provider.action.reset')).not.toBeInTheDocument();

		await fireEvent.input(screen.getByRole('textbox', { name: /Name/i }), {
			target: { value: 'alpha' }
		});
		const submitButtons = screen.getAllByRole('button', { name: 'Reset Provider' });
		await fireEvent.click(submitButtons[submitButtons.length - 1]);

		await waitFor(() => {
			expect(invokeSurfaceInteraction).toHaveBeenCalledWith(
				'provider.surface',
				'provider.action.reset',
				expect.objectContaining({
					params: {
						name: 'alpha'
					}
				})
			);
		});
	});

	it('uses the interaction label as confirmation fallback copy', async () => {
		const interaction: InteractionDescriptor = {
			interaction_id: 'provider.action.delete',
			kind: 'mutation_action',
			label: 'Delete Provider',
			transport: { mode: 'controller_local' },
			confirmation: {
				title: 'Confirm action',
				message: 'Run',
				severity: 'danger'
			}
		};

		render(SurfaceInteractionButton, {
			surfaceId: 'provider.surface',
			interaction
		});

		await fireEvent.click(screen.getByRole('button', { name: 'Delete Provider' }));
		expect(screen.getAllByRole('button', { name: 'Delete Provider' }).length).toBeGreaterThanOrEqual(1);
		expect(screen.queryByText('provider.action.delete')).not.toBeInTheDocument();
	});

	it('shows an unavailable callout instead of rendering unlabeled actions', () => {
		const interaction = {
			interaction_id: 'provider.action.invalid',
			kind: 'mutation_action',
			label: undefined,
			transport: { mode: 'controller_local' }
		} as unknown as InteractionDescriptor;

		render(SurfaceInteractionButton, {
			surfaceId: 'provider.surface',
			interaction
		});

		expect(screen.getByText('Action unavailable')).toBeInTheDocument();
		expect(screen.queryByRole('button')).not.toBeInTheDocument();
	});

	it('renders a primary Button for non-danger interactions', () => {
		const interaction: InteractionDescriptor = {
			interaction_id: 'do-thing',
			kind: 'mutation_action',
			label: 'Do Thing',
			transport: { mode: 'controller_local' }
		};

		render(SurfaceInteractionButton, {
			surfaceId: 'test.surface',
			interaction
		});

		const btn = screen.getByRole('button', { name: 'Do Thing' });
		// Button primitive applies h-[23px] for size=md
		expect(btn.className).toContain('h-[23px]');
		// No loading spinner initially
		expect(btn).not.toHaveAttribute('aria-busy');
	});

	it('renders a danger Button when severity is danger', () => {
		const interaction: InteractionDescriptor = {
			interaction_id: 'delete-thing',
			kind: 'mutation_action',
			label: 'Delete Thing',
			transport: { mode: 'controller_local' },
			confirmation: {
				title: 'Confirm',
				message: 'Are you sure?',
				severity: 'danger'
			}
		};

		render(SurfaceInteractionButton, {
			surfaceId: 'test.surface',
			interaction
		});

		const btn = screen.getByRole('button', { name: 'Delete Thing' });
		// Button primitive's danger variant contains error-bg token
		expect(btn.className).toContain('color-danger');
	});

	it('sets aria-busy and preserves label text during loading', async () => {
		vi.mocked(invokeSurfaceInteraction).mockImplementation(
			() => new Promise(() => {}) // never resolves — keeps loading=true
		);
		const interaction: InteractionDescriptor = {
			interaction_id: 'slow-thing',
			kind: 'mutation_action',
			label: 'Slow Thing',
			transport: { mode: 'controller_local' }
		};

		render(SurfaceInteractionButton, {
			surfaceId: 'test.surface',
			interaction
		});

		const btn = screen.getByRole('button', { name: 'Slow Thing' });
		await fireEvent.click(btn);

		await waitFor(() => {
			expect(btn).toHaveAttribute('aria-busy', 'true');
		});
		// Children text is preserved during load — NOT replaced with 'Processing...'
		expect(btn.textContent).not.toContain('Processing');
		expect(screen.getByText('Slow Thing')).toBeInTheDocument();
	});

	it('renders size=sm Button with h-[19px]', () => {
		const interaction: InteractionDescriptor = {
			interaction_id: 'sm-thing',
			kind: 'mutation_action',
			label: 'Sm Thing',
			transport: { mode: 'controller_local' }
		};

		render(SurfaceInteractionButton, {
			surfaceId: 'test.surface',
			interaction,
			size: 'sm'
		});

		expect(screen.getByRole('button', { name: 'Sm Thing' }).className).toContain('h-[19px]');
	});

	it('no raw preset-filled-* or preset-tonal-* classes on any button', () => {
		const interaction: InteractionDescriptor = {
			interaction_id: 'any-thing',
			kind: 'mutation_action',
			label: 'Any Thing',
			transport: { mode: 'controller_local' }
		};

		const { container } = render(SurfaceInteractionButton, {
			surfaceId: 'test.surface',
			interaction
		});

		const buttons = container.querySelectorAll('button');
		buttons.forEach((b) => {
			expect(b.className).not.toMatch(/preset-filled|preset-tonal/);
		});
	});
});
