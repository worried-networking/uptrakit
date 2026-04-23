import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@testing-library/svelte';

vi.mock('$lib/api', () => ({ updateAgentCertificateSettings: vi.fn() }));

import * as api from '$lib/api';
import type { AgentCertificateSettings } from '$lib/types';
import AgentCertificateSettingsComponent from './AgentCertificateSettings.svelte';

const mockSettings: AgentCertificateSettings = {
	lifetime_days: 7,
	renewal_window_hours_override: null,
	effective_renewal_window_hours: 24
};

const props = {
	settings: mockSettings,
	onSuccess: vi.fn(),
	onError: vi.fn()
};

afterEach(() => vi.clearAllMocks());

describe('AgentCertificateSettings Save button', () => {
	it('Save button has no raw preset-filled-primary-500 class', () => {
		const { container } = render(AgentCertificateSettingsComponent, props);
		expect(container.querySelector('button.preset-filled-primary-500')).toBeNull();
	});

	it('Save button has primary gradient class', () => {
		render(AgentCertificateSettingsComponent, props);
		const btn = screen.getByRole('button', { name: 'Save' });
		expect(btn.className).toContain('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
	});

	it('Save button carries aria-busy=true while saving', async () => {
		let resolve!: (v: AgentCertificateSettings) => void;
		vi.mocked(api.updateAgentCertificateSettings).mockReturnValue(
			new Promise<AgentCertificateSettings>((r) => {
				resolve = r;
			})
		);
		render(AgentCertificateSettingsComponent, props);
		const btn = screen.getByRole('button', { name: 'Save' });
		await fireEvent.click(btn);
		await waitFor(() => expect(btn).toHaveAttribute('aria-busy', 'true'));
		resolve(mockSettings);
		await waitFor(() => expect(btn).not.toHaveAttribute('aria-busy'));
	});

	it('Save button text is static "Save" during loading — no text swap', async () => {
		let resolve!: (v: AgentCertificateSettings) => void;
		vi.mocked(api.updateAgentCertificateSettings).mockReturnValue(
			new Promise<AgentCertificateSettings>((r) => {
				resolve = r;
			})
		);
		render(AgentCertificateSettingsComponent, props);
		const btn = screen.getByRole('button', { name: 'Save' });
		await fireEvent.click(btn);
		await waitFor(() => expect(btn).toHaveAttribute('aria-busy', 'true'));
		expect(btn).toHaveTextContent('Save');
		resolve(mockSettings);
	});
});
