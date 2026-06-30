import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@testing-library/svelte';

vi.mock('$lib/api', async (importOriginal) => ({
	...(await importOriginal<typeof import('$lib/api')>()),
	updateAgentCertificateSettings: vi.fn()
}));

import * as api from '$lib/api';
import type { AgentCertificateSettingsResponse } from '$lib/api';
import AgentCertificateSettingsComponent from './AgentCertificateSettings.svelte';

const mockSettings: AgentCertificateSettingsResponse = {
	lifetime_hours: 168,
	renewal_window_hours_override: null,
	effective_renewal_window_hours: 24
};

const props = {
	settings: mockSettings,
	onSuccess: vi.fn(),
	onError: vi.fn()
};

afterEach(() => vi.clearAllMocks());

async function makeFormDirty() {
	const lifetimeInput = screen.getByRole('spinbutton', { name: /certificate lifetime/i });
	await fireEvent.input(lifetimeInput, { target: { value: '180' } });
}

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

	it('Save button is disabled when form is not dirty', () => {
		render(AgentCertificateSettingsComponent, props);
		const btn = screen.getByRole('button', { name: 'Save' });
		expect(btn).toBeDisabled();
	});

	it('Save button is enabled when form is dirty', async () => {
		render(AgentCertificateSettingsComponent, props);
		await makeFormDirty();
		const btn = screen.getByRole('button', { name: 'Save' });
		expect(btn).not.toBeDisabled();
	});

	it('Save button carries aria-busy=true while saving', async () => {
		let resolve!: (v: unknown) => void;
		vi.mocked(api.updateAgentCertificateSettings).mockReturnValue(
			new Promise((r) => {
				resolve = (v) => r(v as unknown as Awaited<ReturnType<typeof api.updateAgentCertificateSettings>>);
			}) as unknown as ReturnType<typeof api.updateAgentCertificateSettings>
		);
		render(AgentCertificateSettingsComponent, props);
		await makeFormDirty();
		const btn = screen.getByRole('button', { name: 'Save' });
		await fireEvent.click(btn);
		await waitFor(() => expect(btn).toHaveAttribute('aria-busy', 'true'));
		resolve({ data: mockSettings });
		await waitFor(() => expect(btn).not.toHaveAttribute('aria-busy'));
	});

	it('Save button text is static "Save" during loading — no text swap', async () => {
		let resolve!: (v: unknown) => void;
		vi.mocked(api.updateAgentCertificateSettings).mockReturnValue(
			new Promise((r) => {
				resolve = (v) => r(v as unknown as Awaited<ReturnType<typeof api.updateAgentCertificateSettings>>);
			}) as unknown as ReturnType<typeof api.updateAgentCertificateSettings>
		);
		render(AgentCertificateSettingsComponent, props);
		await makeFormDirty();
		const btn = screen.getByRole('button', { name: 'Save' });
		await fireEvent.click(btn);
		await waitFor(() => expect(btn).toHaveAttribute('aria-busy', 'true'));
		expect(btn).toHaveTextContent('Save');
		resolve({ data: mockSettings });
	});
});

describe('AgentCertificateSettings Discard button', () => {
	it('Discard button is absent when form is clean', () => {
		render(AgentCertificateSettingsComponent, props);
		expect(screen.queryByRole('button', { name: 'Discard' })).toBeNull();
	});

	it('Discard button appears when form is dirty', async () => {
		render(AgentCertificateSettingsComponent, props);
		await makeFormDirty();
		expect(screen.getByRole('button', { name: 'Discard' })).not.toBeNull();
	});

	it('Discard button resets the form and hides itself', async () => {
		render(AgentCertificateSettingsComponent, props);
		await makeFormDirty();
		const discard = screen.getByRole('button', { name: 'Discard' });
		await fireEvent.click(discard);
		expect(screen.queryByRole('button', { name: 'Discard' })).toBeNull();
		const btn = screen.getByRole('button', { name: 'Save' });
		expect(btn).toBeDisabled();
	});
});
