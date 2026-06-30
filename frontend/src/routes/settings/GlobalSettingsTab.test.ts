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
vi.mock('$lib/api', async (importOriginal) => ({
	...(await importOriginal<typeof import('$lib/api')>()),
	getGithubProviderSettings: vi.fn(),
	getSystemAlerts: vi.fn(),
	renewServerCertificate: vi.fn(),
	getNetworkSettings: vi.fn(),
	updateNetworkSettings: vi.fn(),
	getNatsSettings: vi.fn(),
	updateNatsSettings: vi.fn(),
	updateGithubProviderSettings: vi.fn(),
	getZeroconfSettings: vi.fn(),
	updateZeroconfSettings: vi.fn(),
	rotateCa: vi.fn()
}));

import * as api from '$lib/api';
import GlobalSettingsTab from './GlobalSettingsTab.svelte';

function stubAllApis() {
	vi.mocked(api.getNetworkSettings).mockResolvedValue({
		data: {
			trusted_proxies: [],
			real_ip_header: 'X-Forwarded-For',
			sans: [],
			https_addr: '[::]:8443'
		}
	} as unknown as Awaited<ReturnType<typeof api.getNetworkSettings>>);
	vi.mocked(api.getSystemAlerts).mockResolvedValue({ data: { alerts: [] } } as unknown as Awaited<
		ReturnType<typeof api.getSystemAlerts>
	>);
	vi.mocked(api.getNatsSettings).mockResolvedValue({
		data: { url: 'nats://host:4222', has_url: true }
	} as unknown as Awaited<ReturnType<typeof api.getNatsSettings>>);
	vi.mocked(api.getZeroconfSettings).mockResolvedValue({
		data: {
			enabled: false
		}
	} as unknown as Awaited<ReturnType<typeof api.getZeroconfSettings>>);
	vi.mocked(api.getGithubProviderSettings).mockResolvedValue({
		data: {
			has_auth_token: false
		}
	} as unknown as Awaited<ReturnType<typeof api.getGithubProviderSettings>>);
}

describe('GlobalSettingsTab button variants', () => {
	beforeEach(() => stubAllApis());
	afterEach(() => vi.clearAllMocks());

	it('GitHub Provider Save button has no raw preset-filled-primary-500 class', async () => {
		const { container } = render(GlobalSettingsTab);
		await screen.findAllByRole('button', { name: 'Save' });
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

	it('NATS Clear button uses danger variant (has color-danger class)', async () => {
		// natsCurrentUrl is non-null so Clear button renders — stub returns url
		render(GlobalSettingsTab);
		await screen.findByText('Clear');
		const clearBtn = screen.getByRole('button', { name: 'Clear' });
		expect(clearBtn.className).toContain('color-danger');
		expect(clearBtn.className).not.toMatch(/preset-tonal-error/);
	});

	it('CA Rotate button uses danger variant (has color-danger class)', async () => {
		render(GlobalSettingsTab);
		await screen.findByText('Rotate CA');
		const rotateBtn = screen.getByRole('button', { name: 'Rotate CA' });
		expect(rotateBtn.className).toContain('color-danger');
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
			new Promise(
				(res) =>
					(resolveSave = () =>
						res({ data: { has_url: false } } as unknown as Awaited<ReturnType<typeof api.updateNatsSettings>>))
			) as unknown as ReturnType<typeof api.updateNatsSettings>
		);
		fireEvent.click(saveBtn);
		await waitFor(() => expect(saveBtn).toHaveAttribute('aria-busy', 'true'));
		expect(saveBtn.textContent?.trim()).toBe('Save'); // static label — no swap

		resolveSave();
		await waitFor(() => expect(saveBtn).not.toHaveAttribute('aria-busy'));
	});
});
