import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import { buildSettingsTabsParityFixture } from '$lib/test-fixtures/ui-parity';
import { Permission } from '$lib/types';

vi.mock('$app/state', () => ({
	page: {
		url: new URL('http://localhost/settings?tab=notifications.email')
	}
}));

vi.mock('$app/navigation', () => ({
	goto: vi.fn()
}));

vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => ({
		id: 'user-1',
		email: 'user@example.com',
		first_name: 'Test',
		last_name: 'User',
		permissions: [Permission.ViewNotifications, Permission.UpdateSystemServices]
	}))
}));

vi.mock('$lib/api', () => ({
	getCombinedSettings: vi.fn(async () => ({
		registration: {},
		authentication: {},
		agent_certificates: {},
		enrollment_tokens: {},
		multi_tenancy_enabled: false
	})),
	getOidcProviders: vi.fn(async () => [])
}));

vi.mock('$lib/surfaces/registry.svelte', () => ({
	getSurfaceReadLoading: vi.fn(() => false),
	getSurfaceReadModel: vi.fn(() => undefined),
	getSurfaceReadRequested: vi.fn(() => false),
	getSurfaceRegistryLoaded: vi.fn(() => true),
	getSurfaceRuntimeStatus: vi.fn(() => ({ active: true })),
	getSurfacesBySlot: vi.fn((slot: string) =>
		slot === 'settings.tabs' ? buildSettingsTabsParityFixture().surfaceTabs : []
	),
	loadSurfaceReadModels: vi.fn(async () => {})
}));

import { goto } from '$app/navigation';
import { page } from '$app/state';
import { getUser } from '$lib/auth.svelte';
import { getSurfaceRegistryLoaded, getSurfaceRuntimeStatus, getSurfacesBySlot } from '$lib/surfaces/registry.svelte';
import SettingsPage from './+page.svelte';

describe('/settings shared-surface tabs', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		page.url = new URL('http://localhost/settings?tab=notifications.email') as typeof page.url;
		vi.mocked(getUser).mockReturnValue({
			id: 'user-1',
			email: 'user@example.com',
			first_name: 'Test',
			last_name: 'User',
			permissions: [Permission.ViewNotifications, Permission.UpdateSystemServices]
		});
		vi.mocked(getSurfaceRegistryLoaded).mockReturnValue(true);
		vi.mocked(getSurfaceRuntimeStatus).mockReturnValue({ active: true });
		vi.mocked(getSurfacesBySlot).mockImplementation((slot: string) =>
			slot === 'settings.tabs' ? buildSettingsTabsParityFixture().surfaceTabs : []
		);
	});

	afterEach(() => {
		vi.clearAllMocks();
	});

	it('shows surface-backed tabs while read models are still pending', () => {
		render(SettingsPage);

		expect(screen.getByRole('tab', { name: 'MQTT Clients' })).toBeInTheDocument();
		expect(screen.getByRole('tab', { name: 'Email Channels' })).toBeInTheDocument();
		expect(document.querySelector('[data-ui="page-shell"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="tab-strip"]')).toBeInTheDocument();
		expect(document.querySelector('[data-ui="section-card"]')).toBeInTheDocument();
	});

	it('renders settings forms with shared form rows when the general tab is active', async () => {
		page.url = new URL('http://localhost/settings?tab=general') as typeof page.url;
		vi.mocked(getUser).mockReturnValue({
			id: 'user-1',
			email: 'user@example.com',
			first_name: 'Test',
			last_name: 'User',
			permissions: [Permission.ManageAuthSettings, Permission.ManageEnrollmentTokens, Permission.ManageAgentCerts]
		});

		render(SettingsPage);

		await screen.findByText('Registration');
		expect(document.querySelector('[data-ui="form-field-row"]')).toBeInTheDocument();
	});

	it('does not reset a surface tab from the URL before the registry finishes loading', () => {
		vi.mocked(getSurfaceRegistryLoaded).mockReturnValue(false);
		vi.mocked(getSurfacesBySlot).mockReturnValue([]);

		render(SettingsPage);

		expect(vi.mocked(goto)).not.toHaveBeenCalledWith('/settings', expect.anything());
	});

	it('does not reset a surface tab from the URL while runtime status is still unresolved', () => {
		vi.mocked(getSurfaceRegistryLoaded).mockReturnValue(false);
		vi.mocked(getSurfaceRuntimeStatus).mockReturnValue({ active: false });
		vi.mocked(getSurfacesBySlot).mockReturnValue([]);

		render(SettingsPage);

		expect(vi.mocked(goto)).not.toHaveBeenCalledWith('/settings', expect.anything());
	});

	it('hides surface tabs when runtime rollout is inactive even if slot surfaces are present', () => {
		vi.mocked(getSurfaceRuntimeStatus).mockReturnValue({ active: false });
		vi.mocked(getSurfacesBySlot).mockImplementation((slot: string) =>
			slot === 'settings.tabs' ? buildSettingsTabsParityFixture().surfaceTabs : []
		);

		render(SettingsPage);

		expect(screen.queryByRole('tab', { name: 'MQTT Clients' })).not.toBeInTheDocument();
		expect(screen.getByRole('tab', { name: 'Notification Rules' })).toBeInTheDocument();
	});
});
