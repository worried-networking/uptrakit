import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';

vi.mock('$lib/auth.svelte', () => ({ getUser: vi.fn(() => null) }));
vi.mock('$lib/notifications.svelte', () => ({
	showSuccess: vi.fn(),
	showError: vi.fn(),
	clearError: vi.fn()
}));
vi.mock('$lib/surfaces/registry.svelte', () => ({
	getSurfaceReadModel: vi.fn(() => undefined),
	getSurfacesBySlot: vi.fn(() => []),
	loadSurfaceReadModels: vi.fn()
}));
vi.mock('$lib/surfaces/read-model', () => ({
	filterSurfacesByPermission: vi.fn(() => []),
	shouldUseSurfaceRoute: vi.fn(() => false)
}));
vi.mock('$lib/api', () => ({
	getGitHubProviderSettings: vi.fn(),
	getSystemAlerts: vi.fn(),
	renewServerCertificate: vi.fn(),
	getNetworkSettings: vi.fn(),
	updateNetworkSettings: vi.fn(),
	getNatsSettings: vi.fn(),
	updateNatsSettings: vi.fn(),
	updateGitHubProviderSettings: vi.fn(),
	getZeroconfSettings: vi.fn(),
	updateZeroconfSettings: vi.fn(),
	rotateCA: vi.fn()
}));

import * as api from '$lib/api';
import GlobalSettingsTab from './GlobalSettingsTab.svelte';

function stubAllApis() {
	vi.mocked(api.getNetworkSettings).mockResolvedValue({
		trusted_proxies: [],
		real_ip_header: 'X-Forwarded-For',
		sans: [],
		https_addr: '[::]:8443'
	});
	vi.mocked(api.getSystemAlerts).mockResolvedValue({ alerts: [] });
	vi.mocked(api.getNatsSettings).mockResolvedValue({ url: 'nats://host:4222', has_url: true });
	vi.mocked(api.getZeroconfSettings).mockResolvedValue({
		enabled: false
	});
	vi.mocked(api.getGitHubProviderSettings).mockResolvedValue({
		has_auth_token: false
	});
}

describe('GlobalSettingsTab button variants', () => {
	beforeEach(() => stubAllApis());
	afterEach(() => vi.clearAllMocks());

	it('GitHub Provider Save button has no raw preset-filled-primary-500 class', async () => {
		const { container } = render(GlobalSettingsTab);
		await screen.findByText('Save GitHub Provider');
		const raw = container.querySelector('button.preset-filled-primary-500');
		expect(raw).toBeNull();
	});

	it('NATS Save button has no raw preset class', async () => {
		const { container } = render(GlobalSettingsTab);
		await waitFor(() => expect(screen.queryAllByText('Save').length).toBeGreaterThan(0));
		const raw = container.querySelector('button.preset-filled-primary-500');
		expect(raw).toBeNull();
	});

	it('NATS Clear button has no raw preset-tonal-error class', async () => {
		const { container } = render(GlobalSettingsTab);
		await screen.findByText('Clear');
		const raw = container.querySelector('button.preset-tonal-error');
		expect(raw).toBeNull();
	});

	it('CA Rotate button has no raw preset-filled-error-500 class', async () => {
		const { container } = render(GlobalSettingsTab);
		await screen.findByText('Rotate CA');
		const raw = container.querySelector('button.preset-filled-error-500');
		expect(raw).toBeNull();
	});

	it('Renew Server Certificate button has no raw preset-filled-primary-500 class', async () => {
		const { container } = render(GlobalSettingsTab);
		await screen.findByText('Renew Server Certificate');
		const raw = container.querySelector('button.preset-filled-primary-500');
		expect(raw).toBeNull();
	});

	it('NATS Clear button uses danger variant (has color-error class)', async () => {
		// natsCurrentUrl is non-null so Clear button renders — stub returns url
		render(GlobalSettingsTab);
		await screen.findByText('Clear');
		const clearBtn = screen.getByRole('button', { name: 'Clear' });
		expect(clearBtn.className).toContain('color-error');
		expect(clearBtn.className).not.toMatch(/preset-tonal-error/);
	});

	it('CA Rotate button uses danger variant (has color-error class)', async () => {
		render(GlobalSettingsTab);
		await screen.findByText('Rotate CA');
		const rotateBtn = screen.getByRole('button', { name: 'Rotate CA' });
		expect(rotateBtn.className).toContain('color-error');
		expect(rotateBtn.className).not.toMatch(/preset-filled-error-500/);
	});
});

describe('GlobalSettingsTab loading states', () => {
	beforeEach(() => stubAllApis());
	afterEach(() => vi.clearAllMocks());

	it('NATS Save button becomes aria-busy while updateNatsSettings is in-flight', async () => {
		render(GlobalSettingsTab);
		// Wait for NATS section to appear (natsAvailable = true after getNatsSettings resolves)
		await screen.findByText('NATS Configuration');
		const natsSection = screen
			.getByRole('heading', { name: 'NATS Configuration' })
			.closest('[data-ui="section-card"]') as HTMLElement;

		const input = within(natsSection).getByRole('textbox');

		// Enable the Save button by typing a URL
		fireEvent.input(input, { target: { value: 'nats://test:4222' } });

		const saveBtn = within(natsSection).getByRole('button', { name: 'Save' });
		await waitFor(() => expect(saveBtn).not.toBeDisabled());
		expect(saveBtn).not.toHaveAttribute('aria-busy');

		// Stall the save — aria-busy must appear while in-flight
		let resolveSave!: () => void;
		vi.mocked(api.updateNatsSettings).mockReturnValue(
			new Promise((res) => (resolveSave = () => res({ has_url: false })))
		);
		fireEvent.click(saveBtn);
		await waitFor(() => expect(saveBtn).toHaveAttribute('aria-busy', 'true'));
		expect(saveBtn.textContent?.trim()).toBe('Save'); // static label — no swap

		resolveSave();
		await waitFor(() => expect(saveBtn).not.toHaveAttribute('aria-busy'));
	});
});
