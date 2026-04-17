import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';
import type { SurfaceReadResponse, SurfaceResponse } from '../../src/lib/surfaces/contract';
import {
	buildParityProvider,
	buildParitySurfacePageFixture,
	buildParitySurfaceTab,
	buildSettingsTabsParityFixture,
	buildSoftwareTabsParityFixture
} from '../../src/lib/test-fixtures/ui-parity';

test.use({
	viewport: { width: 1440, height: 900 },
	colorScheme: 'light',
	locale: 'en-US',
	timezoneId: 'UTC'
});

const isCanonicalUiParityHost = process.platform === 'darwin';
const canonicalUiParityReason =
	'ui parity screenshot baselines are canonicalized on macOS Chromium to avoid cross-OS rasterization drift';

type MockScenario = {
	runtimeActive?: boolean;
	surfaces?: SurfaceResponse[];
	readModels?: Record<string, SurfaceReadResponse>;
};

type MockParityApiResult = {
	readRequests: string[];
};

const mockUser = {
	id: '00000000-0000-0000-0000-000000000111',
	email: 'admin@example.com',
	first_name: 'Admin',
	last_name: 'User',
	permissions: [
		'view_hosts',
		'manage_hosts',
		'update_hosts',
		'deactivate_hosts',
		'view_software',
		'create_software',
		'update_software',
		'delete_software',
		'trigger_checks',
		'trigger_updates',
		'manage_scheduler',
		'view_settings',
		'manage_auth_settings',
		'manage_enrollment_tokens',
		'manage_agent_certs',
		'manage_global_settings',
		'view_notifications',
		'update_system_services',
		'view_system_services',
		'view_audit_logs'
	]
};

const settingsTabsParity = buildSettingsTabsParityFixture();
const softwareTabsParity = buildSoftwareTabsParityFixture();
const surfacePageParity = buildParitySurfacePageFixture();

const paritySurfaces: SurfaceResponse[] = [
	...settingsTabsParity.surfaceTabs,
	...softwareTabsParity.surfaceTabs,
	surfacePageParity.surface,
	buildParitySurfaceTab('settings.global.audit', 'Global Audit Extension', {
		priority: 650,
		slot: 'settings.below.global',
		scope: 'global',
		required_permission: 'manage_global_settings',
		provider_kind: 'service',
		root_node: { kind: 'text_block', text: 'global-audit' }
	}),
	buildParitySurfaceTab('proxmox.host-info', 'Proxmox VE Info', {
		priority: 700,
		slot: 'host_detail.tabs',
		provider_kind: 'plugin',
		root_node: { kind: 'text_block', text: 'host-info' }
	}),
	buildParitySurfaceTab('software.host.context', 'Host Context Surface', {
		priority: 800,
		slot: 'software_item.host_context_menu',
		provider_kind: 'plugin',
		root_node: { kind: 'text_block', text: 'host-context' }
	})
];

const softwareListItem = {
	id: 'sw-001',
	name: 'Nginx',
	featured: true,
	icon_url: null,
	host_count: 1,
	plugins: ['package_manager_apt'],
	update_available: true,
	last_checked_at: '2024-06-01T12:00:00Z',
	latest_version: '1.27.0',
	latest_release_metadata: { display_version: '1.27.0' },
	created_at: '2024-01-01T00:00:00Z',
	updated_at: '2024-01-01T00:00:00Z'
};

const softwareDetailItem = {
	id: 'sw-001',
	name: 'Nginx',
	featured: true,
	icon_url: null,
	host_count: 1,
	plugins: ['package_manager_apt'],
	latest_version: '1.27.0',
	last_checked_at: '2024-06-01T12:00:00Z',
	hosts: [
		{
			id: 'assignment-001',
			host_id: 'host-001',
			hostname: 'prod-server',
			friendly_name: 'Production Server',
			qualifier: null,
			plugins: [{ role: 'detect_version', plugin_type: 'package_manager_apt', plugin_config_name: 'APT Default' }],
			installed_version: '1.24.0',
			installed_display_version: '1.24.0',
			latest_version: '1.27.0',
			latest_release_metadata: { display_version: '1.27.0' },
			installed_version_detected_at: '2024-06-01T10:00:00Z',
			update_available: true,
			active_update_history_id: null
		}
	]
};

const hostDetail = {
	id: 'host-001',
	machine_id: 'machine-abc',
	hostname: 'prod-server',
	friendly_name: 'Production Server',
	os_type: 'Linux',
	os_version: 'Ubuntu 24.04',
	architecture: 'x86_64',
	ip_address: '10.0.0.5',
	last_seen_at: '2024-06-01T12:00:00Z',
	created_at: '2024-01-01T00:00:00Z',
	updated_at: '2024-01-01T00:00:00Z',
	agents: [{ id: 'agent-001', friendly_name: 'Main Agent', status: 'approved' }],
	tags: []
};

function buildSurfaceRead(surface: SurfaceResponse, text: string): SurfaceReadResponse {
	const { provider_count: _providerCount, ...descriptor } = surface;
	return {
		descriptor: {
			...descriptor,
			root_node: {
				kind: 'text_block',
				text
			}
		},
		interactions: [],
		data_sources: []
	};
}

function buildDefaultReadModels(surfaces: SurfaceResponse[]): Record<string, SurfaceReadResponse> {
	return Object.fromEntries(
		surfaces.map((surface) => [surface.surface_id, buildSurfaceRead(surface, `${surface.label} Loaded Content`)])
	);
}

async function freezeParityInputs(page: Page) {
	await page.addInitScript(() => {
		localStorage.setItem('theme-mode', 'light');
	});
	await page.emulateMedia({ colorScheme: 'light', reducedMotion: 'reduce' });
}

async function mockParityApi(page: Page, scenario: MockScenario = {}): Promise<MockParityApiResult> {
	const surfaces = scenario.surfaces ?? paritySurfaces;
	const readModels = scenario.readModels ?? buildDefaultReadModels(surfaces);
	const runtimeActive = scenario.runtimeActive ?? true;
	const readRequests: string[] = [];

	await page.route('**/api/v1/**', async (route) => {
		const url = new URL(route.request().url());
		const path = url.pathname;
		const method = route.request().method();

		const json = (body: unknown, status = 200) =>
			route.fulfill({
				status,
				contentType: 'application/json',
				body: JSON.stringify(body)
			});

		if (method === 'POST' && path === '/api/v1/auth/refresh') {
			return json({
				access_token: 'test-token',
				refresh_token: 'test-refresh-token',
				expires_in: 3600,
				token_type: 'Bearer',
				user: mockUser
			});
		}
		if (method === 'GET' && path === '/api/v1/auth/me') {
			return json(mockUser);
		}
		if (method === 'GET' && path === '/api/v1/system/alerts') {
			return json({ alerts: [] });
		}
		if (method === 'GET' && path === '/api/v1/surfaces/runtime-status') {
			return json({ active: runtimeActive });
		}
		if (method === 'GET' && path === '/api/v1/surfaces') {
			const slot = url.searchParams.get('slot');
			if (!slot) {
				return json(surfaces);
			}
			return json(surfaces.filter((surface) => surface.slot === slot));
		}
		if (method === 'GET' && /^\/api\/v1\/surfaces\/[^/]+\/providers$/.test(path)) {
			return json([buildParityProvider()]);
		}
		const readMatch = path.match(/^\/api\/v1\/surfaces\/([^/]+)\/read$/);
		if (method === 'GET' && readMatch) {
			const surfaceId = decodeURIComponent(readMatch[1] ?? '');
			readRequests.push(surfaceId);
			const readModel = readModels[surfaceId];
			if (!readModel) {
				return json({ error: `Missing read model fixture for ${surfaceId}` }, 404);
			}
			return json(readModel);
		}

		if (method === 'GET' && path === '/api/v1/settings') {
			return json({
				registration: {},
				authentication: {},
				agent_certificates: {},
				enrollment_tokens: {},
				multi_tenancy_enabled: false
			});
		}
		if (method === 'GET' && path === '/api/v1/settings/oidc-providers') {
			return json([]);
		}
		if (method === 'GET' && path === '/api/v1/global-settings/network') {
			return json({
				trusted_proxies: ['10.0.0.0/8'],
				real_ip_header: 'X-Forwarded-For',
				sans: ['controller.local'],
				https_addr: '[::]:8443'
			});
		}
		if (method === 'GET' && path === '/api/v1/global-settings/nats') {
			return json({ url: 'nats://nats.local:4222' });
		}
		if (method === 'GET' && path === '/api/v1/global-settings/zeroconf') {
			return json({
				enabled: true,
				ca_fingerprint: 'AA:BB:CC:DD',
				url: 'https://controller.local',
				pki_addr: 'http://controller.local:8080'
			});
		}
		if (method === 'GET' && path === '/api/v1/system-enrollment-tokens') {
			return json({
				items: [],
				total: 0,
				page: 1,
				per_page: 25,
				total_pages: 1
			});
		}
		if (method === 'GET' && path === '/api/v1/plugin-types') {
			return json([
				{
					plugin_type: 'package_manager_apt',
					display_name: 'APT',
					plugin_role: 'detect_version',
					capabilities: ['discover_local_software']
				}
			]);
		}
		if (method === 'GET' && path === '/api/v1/software-items') {
			const requestedPage = Number(url.searchParams.get('page') ?? '1');
			const requestedPerPage = Number(url.searchParams.get('per_page') ?? '50');
			const hostId = url.searchParams.get('host_id');
			const items = hostId === 'host-001' ? [softwareListItem] : [softwareListItem];
			return json({
				items,
				total: items.length,
				page: requestedPage,
				per_page: requestedPerPage,
				total_pages: 1
			});
		}
		const softwareItemMatch = path.match(/^\/api\/v1\/software-items\/([^/]+)$/);
		if (method === 'GET' && softwareItemMatch) {
			const softwareItemId = decodeURIComponent(softwareItemMatch[1] ?? '');
			if (softwareItemId === softwareDetailItem.id) {
				return json(softwareDetailItem);
			}
			return json({ error: `Software item fixture not found: ${softwareItemId}` }, 404);
		}
		const hostMatch = path.match(/^\/api\/v1\/hosts\/([^/]+)$/);
		if (method === 'GET' && hostMatch) {
			const hostId = decodeURIComponent(hostMatch[1] ?? '');
			if (hostId === hostDetail.id) {
				return json(hostDetail);
			}
			return json({ error: `Host fixture not found: ${hostId}` }, 404);
		}
		if (method === 'GET' && path === '/api/v1/update-history') {
			return json({
				items: [],
				total: 0,
				page: 1,
				per_page: 5,
				total_pages: 1
			});
		}
		const hostAllowlistMatch = path.match(/^\/api\/v1\/hosts\/([^/]+)\/discovery-allowlist$/);
		if (method === 'GET' && hostAllowlistMatch) {
			return json([]);
		}
		if (method === 'GET' && path === '/api/v1/host-tags') {
			return json({
				items: [],
				total: 0,
				page: 1,
				per_page: 100,
				total_pages: 1
			});
		}

		return json({ error: `Unhandled mock: ${method} ${path}` }, 500);
	});

	return { readRequests };
}

test.beforeEach(async ({ page }) => {
	test.skip(!isCanonicalUiParityHost, canonicalUiParityReason);
	await freezeParityInputs(page);
});

test('settings tabs ui parity: built-in settings tab vs settings.tabs', async ({ page }) => {
	await mockParityApi(page);

	await page.goto('/settings?tab=notifications.email');

	const tabStrip = page.locator('[data-ui="tab-strip"]');
	await expect(tabStrip).toBeVisible();
	await expect(page.getByRole('tab', { name: 'General' })).toBeVisible();
	await expect(page.getByRole('tab', { name: 'Email Channels' })).toBeVisible();

	await expect(tabStrip).toHaveScreenshot('ui-parity-settings-tabs.png', {
		animations: 'disabled',
		caret: 'hide'
	});
});

test('software tabs ui parity: built-in software tab vs software.tabs', async ({ page }) => {
	await mockParityApi(page);

	await page.goto('/software?tab=proxmox.hosts');

	const tabStrip = page.locator('[data-ui="tab-strip"]');
	await expect(tabStrip).toBeVisible();
	await expect(page.getByRole('tab', { name: /^Featured$/ })).toBeVisible();
	await expect(page.getByRole('tab', { name: 'Proxmox VE Hosts' })).toBeVisible();

	await expect(tabStrip).toHaveScreenshot('ui-parity-software-tabs.png', {
		animations: 'disabled',
		caret: 'hide'
	});
});

test('navigation ui parity: built-in nav item vs surface.page', async ({ page }) => {
	await mockParityApi(page);

	await page.goto('/software');

	const nav = page.locator('[data-ui="app-shell-nav"]');
	await expect(nav).toBeVisible();
	await expect(nav.getByRole('link', { name: 'Software' })).toBeVisible();
	await expect(nav.getByRole('link', { name: 'Surface One' })).toBeVisible();

	await expect(nav).toHaveScreenshot('ui-parity-app-nav-built-in-vs-surface-page.png', {
		animations: 'disabled',
		caret: 'hide'
	});
});

test('settings panel ui parity: settings.below.global', async ({ page }) => {
	await mockParityApi(page);

	await page.goto('/settings?tab=global-settings');

	const extensionCard = page
		.locator('[data-ui="section-card"]')
		.filter({ has: page.getByRole('heading', { name: 'Global Audit Extension' }) });
	await expect(extensionCard).toBeVisible();
	await expect(extensionCard).toHaveScreenshot('ui-parity-settings-below-global-panel.png', {
		animations: 'disabled',
		caret: 'hide'
	});
});

test('host detail ui parity: host_detail.tabs slot container', async ({ page }) => {
	await mockParityApi(page);

	await page.goto('/hosts/host-001');

	const hostDetailSurfaceCard = page
		.locator('[data-ui="section-card"]')
		.filter({ has: page.getByRole('heading', { name: 'Proxmox VE Info' }) });
	await expect(hostDetailSurfaceCard).toBeVisible();
	await expect(hostDetailSurfaceCard).toHaveScreenshot('ui-parity-host-detail-tabs-slot.png', {
		animations: 'disabled',
		caret: 'hide'
	});
});

test('software host context ui parity: software_item.host_context_menu launcher and opened modal', async ({ page }) => {
	await mockParityApi(page);

	await page.goto('/software/sw-001');

	const hostRow = page.locator('tr').filter({ has: page.getByRole('link', { name: 'prod-server' }) });
	await expect(hostRow).toBeVisible();
	await expect(hostRow).toHaveScreenshot('ui-parity-software-host-context-launcher.png', {
		animations: 'disabled',
		caret: 'hide'
	});

	await page.getByRole('button', { name: /actions for prod-server/i }).click();
	await expect(page.getByRole('menuitem', { name: 'Host Context Surface' })).toBeVisible();
	await page.getByRole('menuitem', { name: 'Host Context Surface' }).click();

	const contextModal = page.getByRole('dialog');
	await expect(contextModal).toBeVisible();
	await expect(contextModal).toHaveScreenshot('ui-parity-software-host-context-modal.png', {
		animations: 'disabled',
		caret: 'hide'
	});
});

test('surface page ui parity: surface.page loaded shell', async ({ page }) => {
	await mockParityApi(page);

	await page.goto('/surfaces/surface.one');

	const loadedSurfaceCard = page
		.locator('[data-ui="section-card"]')
		.filter({ has: page.getByText('Surface One Loaded Content', { exact: true }) });
	await expect(loadedSurfaceCard).toBeVisible();
	await expect(page.getByText('Surface One Loaded Content')).toBeVisible();
	await expect(loadedSurfaceCard).toHaveScreenshot('ui-parity-surface-page-loaded-shell.png', {
		animations: 'disabled',
		caret: 'hide'
	});
});

test('surface page ui parity: surface.page runtime-state shell', async ({ page }) => {
	const runtimeReadModels = buildDefaultReadModels(paritySurfaces);
	expect(runtimeReadModels['surface.one']).toBeDefined();

	const apiRequests = await mockParityApi(page, {
		runtimeActive: false,
		readModels: runtimeReadModels
	});

	await page.goto('/surfaces/surface.one');

	const runtimeStateCard = page
		.locator('[data-ui="section-card"]')
		.filter({ has: page.getByText('Surface contract is not available yet.', { exact: true }) });
	await expect(runtimeStateCard).toBeVisible();
	await expect(page.locator('[data-ui="app-shell-nav"]').getByRole('link', { name: 'Surface One' })).toHaveCount(0);
	await expect(page.getByText('Surface contract is not available yet.')).toBeVisible();
	await expect(page.getByText('Surface One Loaded Content')).toHaveCount(0);
	expect(apiRequests.readRequests).not.toContain('surface.one');
	await expect(runtimeStateCard).toHaveScreenshot('ui-parity-surface-page-runtime-state-shell.png', {
		animations: 'disabled',
		caret: 'hide'
	});
});
