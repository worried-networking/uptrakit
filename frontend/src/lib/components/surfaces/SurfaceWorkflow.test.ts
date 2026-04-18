import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import SurfaceWorkflow from './SurfaceWorkflow.svelte';
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
import { showError } from '$lib/notifications.svelte';

describe('SurfaceWorkflow', () => {
	beforeEach(() => {
		vi.mocked(sealedBoxEncrypt).mockResolvedValue('ciphertext');
		vi.mocked(invokeSurfaceInteraction)
			.mockResolvedValueOnce({
				host_info: { hostname: 'example-host' },
				actions: [{ id: 'sudoers', label: 'Install sudoers', description: 'Configure sudo', skippable: false }]
			})
			.mockResolvedValueOnce({ host_id: 'host-1' });
	});

	afterEach(() => {
		cleanup();
		vi.clearAllMocks();
	});

	it('executes wizard steps via submit interaction ids without invoking the workflow id directly', async () => {
		const interaction: InteractionDescriptor = {
			interaction_id: 'bootstrap',
			kind: 'workflow',
			label: 'Bootstrap Host',
			transport: { mode: 'provider_proxied' },
			workflow_steps: [
				{
					step_id: 'connect',
					label: 'Connection',
					input_schema: 'object',
					result_schema: 'any',
					submit_interaction_id: 'bootstrap-connect',
					form_ui: {
						fields: [
							{ key: 'target', label: 'SSH Target', field_type: 'text', required: true },
							{
								key: 'auth_password',
								label: 'SSH Password',
								field_type: 'password',
								required: true,
								sensitive: true
							}
						]
					}
				},
				{
					step_id: 'review',
					label: 'Review',
					input_schema: 'object',
					result_schema: 'any',
					render_previous_response: true,
					form_ui: {
						fields: []
					}
				},
				{
					step_id: 'execute',
					label: 'Execute',
					input_schema: 'object',
					result_schema: 'any',
					submit_interaction_id: 'bootstrap-execute',
					form_ui: {
						fields: []
					}
				}
			]
		};

		const interactions: InteractionDescriptor[] = [
			interaction,
			{
				interaction_id: 'bootstrap-connect',
				kind: 'mutation_action',
				label: 'Bootstrap Connect',
				transport: { mode: 'provider_proxied' },
				sensitive_fields: ['auth_password']
			},
			{
				interaction_id: 'bootstrap-execute',
				kind: 'mutation_action',
				label: 'Bootstrap Execute',
				transport: { mode: 'provider_proxied' },
				sensitive_fields: ['auth_password']
			}
		];

		const { container } = render(SurfaceWorkflow, {
			surfaceId: 'ssh-agent.hosts',
			interaction,
			interactions,
			targetProviderId: 'provider-1',
			encryptionContext: {
				keyId: 'enc-key',
				algorithm: 'ecies_p256',
				publicKey: 'public-key'
			},
			baseParams: { host_id: 'host-1' }
		});

		await fireEvent.click(screen.getByRole('button', { name: 'Bootstrap Host' }));
		expect(document.querySelectorAll('[data-ui="form-field-row"]')).toHaveLength(2);
		expect(container.querySelector('[data-ui="modal-shell"]')).toBeInTheDocument();
		await fireEvent.input(screen.getByRole('textbox', { name: /SSH Target/i }), {
			target: { value: 'root@example:22' }
		});
		await fireEvent.input(screen.getByLabelText(/SSH Password/i), {
			target: { value: 'super-secret' }
		});
		await fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

		await waitFor(() => {
			expect(invokeSurfaceInteraction).toHaveBeenNthCalledWith(
				1,
				'ssh-agent.hosts',
				'bootstrap-connect',
				expect.objectContaining({
					params: {
						host_id: 'host-1',
						target: 'root@example:22'
					},
					target_provider_id: 'provider-1',
					encrypted_sensitive_params: {
						key_id: 'enc-key',
						algorithm: 'ecies_p256',
						ciphertext_b64: 'ciphertext'
					}
				})
			);
		});

		await fireEvent.click(screen.getByRole('button', { name: 'Execute' }));

		await waitFor(() => {
			expect(invokeSurfaceInteraction).toHaveBeenNthCalledWith(
				2,
				'ssh-agent.hosts',
				'bootstrap-execute',
				expect.objectContaining({
					params: {
						host_id: 'host-1',
						target: 'root@example:22'
					},
					target_provider_id: 'provider-1',
					encrypted_sensitive_params: {
						key_id: 'enc-key',
						algorithm: 'ecies_p256',
						ciphertext_b64: 'ciphertext'
					}
				})
			);
		});

		expect(vi.mocked(invokeSurfaceInteraction).mock.calls.map((call) => call[1])).toEqual([
			'bootstrap-connect',
			'bootstrap-execute'
		]);
		expect(sealedBoxEncrypt).toHaveBeenCalledTimes(2);
	});

	it('auto mode skips review and auto-submits execute step when execute has no fields', async () => {
		const interaction: InteractionDescriptor = {
			interaction_id: 'bootstrap',
			kind: 'workflow',
			label: 'Bootstrap Host',
			transport: { mode: 'provider_proxied' },
			workflow_steps: [
				{
					step_id: 'connect',
					label: 'Connection',
					input_schema: 'object',
					result_schema: 'any',
					submit_interaction_id: 'bootstrap-connect',
					form_ui: {
						fields: [
							{ key: 'target', label: 'SSH Target', field_type: 'text', required: true },
							{ key: 'auto', label: 'Auto', field_type: 'toggle', required: false }
						]
					}
				},
				{
					step_id: 'review',
					label: 'Review',
					input_schema: 'object',
					result_schema: 'any',
					render_previous_response: true,
					form_ui: { fields: [] }
				},
				{
					step_id: 'execute',
					label: 'Execute',
					input_schema: 'object',
					result_schema: 'any',
					submit_interaction_id: 'bootstrap-execute',
					form_ui: { fields: [] }
				}
			]
		};

		const interactions: InteractionDescriptor[] = [
			interaction,
			{
				interaction_id: 'bootstrap-connect',
				kind: 'mutation_action',
				label: 'Bootstrap Connect',
				transport: { mode: 'provider_proxied' }
			},
			{
				interaction_id: 'bootstrap-execute',
				kind: 'mutation_action',
				label: 'Bootstrap Execute',
				transport: { mode: 'provider_proxied' }
			}
		];

		render(SurfaceWorkflow, {
			surfaceId: 'ssh-agent.hosts',
			interaction,
			interactions,
			baseParams: { host_id: 'host-1' }
		});

		await fireEvent.click(screen.getByRole('button', { name: 'Bootstrap Host' }));
		await fireEvent.input(screen.getByRole('textbox', { name: /SSH Target/i }), {
			target: { value: 'root@example:22' }
		});
		await fireEvent.click(screen.getByLabelText(/Auto/i));
		await fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

		await waitFor(() => {
			expect(vi.mocked(invokeSurfaceInteraction).mock.calls.map((call) => call[1])).toEqual([
				'bootstrap-connect',
				'bootstrap-execute'
			]);
		});

		expect(screen.queryByRole('button', { name: 'Execute' })).toBeNull();
	});

	it('keeps execute step active when auto-submitted execute fails', async () => {
		vi.mocked(invokeSurfaceInteraction).mockReset();
		vi.mocked(invokeSurfaceInteraction)
			.mockResolvedValueOnce({
				host_info: { hostname: 'example-host' },
				actions: [{ id: 'sudoers', label: 'Install sudoers', description: 'Configure sudo', skippable: false }]
			})
			.mockRejectedValueOnce(new Error('execute failed'));

		const interaction: InteractionDescriptor = {
			interaction_id: 'bootstrap',
			kind: 'workflow',
			label: 'Bootstrap Host',
			transport: { mode: 'provider_proxied' },
			workflow_steps: [
				{
					step_id: 'connect',
					label: 'Connection',
					input_schema: 'object',
					result_schema: 'any',
					submit_interaction_id: 'bootstrap-connect',
					form_ui: {
						fields: [
							{ key: 'target', label: 'SSH Target', field_type: 'text', required: true },
							{ key: 'auto', label: 'Auto', field_type: 'toggle', required: false }
						]
					}
				},
				{
					step_id: 'review',
					label: 'Review',
					input_schema: 'object',
					result_schema: 'any',
					render_previous_response: true,
					form_ui: { fields: [] }
				},
				{
					step_id: 'execute',
					label: 'Execute',
					input_schema: 'object',
					result_schema: 'any',
					submit_interaction_id: 'bootstrap-execute',
					form_ui: { fields: [] }
				}
			]
		};

		const interactions: InteractionDescriptor[] = [
			interaction,
			{
				interaction_id: 'bootstrap-connect',
				kind: 'mutation_action',
				label: 'Bootstrap Connect',
				transport: { mode: 'provider_proxied' }
			},
			{
				interaction_id: 'bootstrap-execute',
				kind: 'mutation_action',
				label: 'Bootstrap Execute',
				transport: { mode: 'provider_proxied' }
			}
		];

		render(SurfaceWorkflow, {
			surfaceId: 'ssh-agent.hosts',
			interaction,
			interactions,
			baseParams: { host_id: 'host-1' }
		});

		await fireEvent.click(screen.getByRole('button', { name: 'Bootstrap Host' }));
		await fireEvent.input(screen.getByRole('textbox', { name: /SSH Target/i }), {
			target: { value: 'root@example:22' }
		});
		await fireEvent.click(screen.getByLabelText(/Auto/i));
		await fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

		await waitFor(() => {
			expect(vi.mocked(invokeSurfaceInteraction).mock.calls.map((call) => call[1])).toEqual([
				'bootstrap-connect',
				'bootstrap-execute'
			]);
		});
		await waitFor(() => {
			expect(vi.mocked(showError)).toHaveBeenCalledWith('execute failed');
		});

		expect(screen.queryByRole('button', { name: 'Run' })).not.toBeNull();
		expect(screen.queryByRole('textbox', { name: /SSH Target/i })).toBeNull();
	});

	it('uses workflow trigger and step-indicator parity treatment in the shared modal shell', async () => {
		const interaction: InteractionDescriptor = {
			interaction_id: 'bootstrap',
			kind: 'workflow',
			label: 'Bootstrap Host',
			transport: { mode: 'provider_proxied' },
			workflow_steps: [
				{
					step_id: 'connect',
					label: 'Connection',
					input_schema: 'object',
					result_schema: 'any',
					submit_interaction_id: 'bootstrap-connect',
					form_ui: {
						fields: [{ key: 'target', label: 'SSH Target', field_type: 'text', required: true }]
					}
				},
				{
					step_id: 'execute',
					label: 'Execute',
					input_schema: 'object',
					result_schema: 'any',
					submit_interaction_id: 'bootstrap-execute',
					form_ui: { fields: [] }
				}
			]
		};
		const interactions: InteractionDescriptor[] = [
			interaction,
			{
				interaction_id: 'bootstrap-connect',
				kind: 'mutation_action',
				label: 'Bootstrap Connect',
				transport: { mode: 'provider_proxied' }
			},
			{
				interaction_id: 'bootstrap-execute',
				kind: 'mutation_action',
				label: 'Bootstrap Execute',
				transport: { mode: 'provider_proxied' }
			}
		];

		const { container } = render(SurfaceWorkflow, {
			surfaceId: 'ssh-agent.hosts',
			interaction,
			interactions
		});

		const trigger = screen.getByRole('button', { name: 'Bootstrap Host' });
		expect(trigger).toHaveAttribute('data-ui', 'workflow-trigger');

		await fireEvent.click(trigger);

		const modalShell = container.querySelector('[data-ui="modal-shell"]');
		expect(modalShell).toBeInTheDocument();
		const indicator = modalShell?.querySelector('[data-ui="workflow-step-indicator"]');
		expect(indicator).toBeInTheDocument();

		const chips = modalShell?.querySelectorAll('[data-ui="workflow-step-chip"]');
		expect(chips).toHaveLength(2);
		expect(chips?.[0]).toHaveAttribute('data-state', 'active');
		expect(chips?.[1]).toHaveAttribute('data-state', 'upcoming');
	});

	it('renders review-state and security-impact treatment using shared parity markers', async () => {
		vi.mocked(invokeSurfaceInteraction).mockReset();
		vi.mocked(invokeSurfaceInteraction)
			.mockResolvedValueOnce({
				host_info: { hostname: 'example-host' },
				actions: [
					{
						id: 'sudoers',
						label: 'Install sudoers',
						description: 'Configure sudo',
						skippable: false,
						security_impact: 'high'
					}
				]
			})
			.mockResolvedValueOnce({ host_id: 'host-1' });

		const interaction: InteractionDescriptor = {
			interaction_id: 'bootstrap',
			kind: 'workflow',
			label: 'Bootstrap Host',
			transport: { mode: 'provider_proxied' },
			workflow_steps: [
				{
					step_id: 'connect',
					label: 'Connection',
					input_schema: 'object',
					result_schema: 'any',
					submit_interaction_id: 'bootstrap-connect',
					form_ui: {
						fields: [{ key: 'target', label: 'SSH Target', field_type: 'text', required: true }]
					}
				},
				{
					step_id: 'review',
					label: 'Review',
					input_schema: 'object',
					result_schema: 'any',
					render_previous_response: true,
					form_ui: { fields: [] }
				},
				{
					step_id: 'execute',
					label: 'Execute',
					input_schema: 'object',
					result_schema: 'any',
					submit_interaction_id: 'bootstrap-execute',
					form_ui: { fields: [] }
				}
			]
		};
		const interactions: InteractionDescriptor[] = [
			interaction,
			{
				interaction_id: 'bootstrap-connect',
				kind: 'mutation_action',
				label: 'Bootstrap Connect',
				transport: { mode: 'provider_proxied' }
			},
			{
				interaction_id: 'bootstrap-execute',
				kind: 'mutation_action',
				label: 'Bootstrap Execute',
				transport: { mode: 'provider_proxied' }
			}
		];

		const { container } = render(SurfaceWorkflow, {
			surfaceId: 'ssh-agent.hosts',
			interaction,
			interactions
		});

		await fireEvent.click(screen.getByRole('button', { name: 'Bootstrap Host' }));
		await fireEvent.input(screen.getByRole('textbox', { name: /SSH Target/i }), {
			target: { value: 'root@example:22' }
		});
		await fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

		await waitFor(() => {
			expect(screen.getByText('Planned Actions')).toBeInTheDocument();
		});

		const modalShell = container.querySelector('[data-ui="modal-shell"]');
		expect(modalShell?.querySelector('[data-ui="workflow-review-state"]')).toBeInTheDocument();
		expect(modalShell?.querySelector('[data-ui="workflow-security-impact"]')).toBeInTheDocument();
	});
});
