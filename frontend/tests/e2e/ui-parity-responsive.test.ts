import { expect, test } from '@playwright/test';
import type { Locator, Page } from '@playwright/test';
import type { SurfaceReadResponse, SurfaceResponse } from '../../src/lib/surfaces/contract';
import {
	buildParityProvider,
	buildParitySurfacePageFixture,
	buildSettingsTabsParityFixture,
	buildSoftwareTabsParityFixture
} from '../../src/lib/test-fixtures/ui-parity';
import { PARITY_VIEWPORT_PRESETS, expectParityScreenshot, type ParityViewportPreset } from './parity-config';

declare global {
	const process: {
		platform?: string;
	};
}

const isCanonicalUiParityHost = process.platform === 'darwin';
const canonicalUiParityReason =
	'ui parity screenshot baselines are canonicalized on macOS Chromium to avoid cross-OS rasterization drift';

test.use({
	locale: 'en-US',
	timezoneId: 'UTC'
});

type MockScenario = {
	systemAlerts?: Array<{
		id: string;
		severity: 'info' | 'warning' | 'error' | 'critical';
		title: string;
		message: string;
		action?: string;
	}>;
	softwareDetailHostActiveUpdate?: boolean;
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
	surfacePageParity.surface
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

function buildSoftwareDetailItem(activeUpdateHistoryId: string | null) {
	return {
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
				active_update_history_id: activeUpdateHistoryId
			}
		]
	};
}

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

async function captureParityScreenshot(
	page: Page,
	target: Page | Locator,
	name: string,
	viewport: ParityViewportPreset
) {
	await expectParityScreenshot({
		page,
		target,
		name,
		viewport
	});
}

async function installMockWebSocket(page: Page) {
	await page.addInitScript(() => {
		class MockWebSocket {
			static CONNECTING = 0;
			static OPEN = 1;
			static CLOSING = 2;
			static CLOSED = 3;

			readyState = MockWebSocket.CONNECTING;
			onopen: ((event: Event) => void) | null = null;
			onmessage: ((event: MessageEvent) => void) | null = null;
			onclose: ((event: CloseEvent) => void) | null = null;
			onerror: ((event: Event) => void) | null = null;

			constructor(_url: string) {
				setTimeout(() => {
					this.readyState = MockWebSocket.OPEN;
					this.onopen?.(new Event('open'));
				}, 0);
			}

			send(_data: string | ArrayBufferLike | Blob | ArrayBufferView) {}

			close(_code?: number, _reason?: string) {
				this.readyState = MockWebSocket.CLOSED;
				this.onclose?.(new CloseEvent('close', { code: 1000 }));
			}

			addEventListener() {}

			removeEventListener() {}

			dispatchEvent() {
				return true;
			}
		}

		Object.defineProperty(window, 'WebSocket', {
			configurable: true,
			writable: true,
			value: MockWebSocket
		});
	});
}

async function mockParityApi(page: Page, scenario: MockScenario = {}) {
	const surfaces = paritySurfaces;
	const readModels = buildDefaultReadModels(surfaces);
	const softwareDetailItem = buildSoftwareDetailItem(
		scenario.softwareDetailHostActiveUpdate ? 'history-live-001' : null
	);
	const systemAlerts = scenario.systemAlerts ?? [];

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
			return json({ alerts: systemAlerts });
		}
		if (method === 'GET' && path === '/api/v1/surfaces/runtime-status') {
			return json({ active: true });
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
			return json({
				items: [softwareListItem],
				total: 1,
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
		if (method === 'GET' && path === '/api/v1/update-history') {
			return json({
				items: [],
				total: 0,
				page: 1,
				per_page: 5,
				total_pages: 1
			});
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
}

test.beforeEach(async ({ page }) => {
	test.skip(!isCanonicalUiParityHost, canonicalUiParityReason);
	await freezeParityInputs(page);
});

test('tablet responsive ui parity: overlay sidebar drawer', async ({ page }) => {
	await page.setViewportSize(PARITY_VIEWPORT_PRESETS.tablet);
	await mockParityApi(page);

	await page.goto('/software');

	await expect(page.getByRole('button', { name: /open navigation/i })).toBeVisible();
	await expect(page.locator('[data-ui="app-shell-sidebar"][data-variant="desktop"]')).toHaveCount(0);

	const tabletSidebar = page.locator('[data-ui="app-shell-sidebar"][data-variant="tablet"]');
	await expect(tabletSidebar).toBeHidden();

	await page.getByRole('button', { name: /open navigation/i }).click();

	await expect(page.locator('[data-ui="app-shell-sidebar-backdrop"]')).toBeVisible();
	await expect(tabletSidebar).toBeVisible();
	await captureParityScreenshot(page, page, 'ui-parity-responsive-tablet-sidebar-overlay.png', 'tablet');
});

test('tablet responsive ui parity: overlay drawer traps focus and closes on Escape', async ({ page }) => {
	await page.setViewportSize(PARITY_VIEWPORT_PRESETS.tablet);
	await mockParityApi(page);

	await page.goto('/software');

	await page.getByRole('button', { name: /open navigation/i }).click();

	const tabletSidebar = page.locator('[data-ui="app-shell-sidebar"][data-variant="tablet"]');
	const firstNavLink = tabletSidebar.getByRole('link', { name: 'Home' });
	await expect(firstNavLink).toBeFocused();

	await page.keyboard.press('Shift+Tab');
	await expect(tabletSidebar.getByRole('link', { name: 'Settings' })).toBeFocused();

	await page.keyboard.press('Tab');
	await expect(firstNavLink).toBeFocused();

	await page.keyboard.press('Escape');
	await expect(tabletSidebar).toBeHidden();
	await expect(page.getByRole('button', { name: /open navigation/i })).toBeFocused();
});

test('mobile responsive ui parity: bottom navigation and overflow sheet', async ({ page }) => {
	await page.setViewportSize(PARITY_VIEWPORT_PRESETS.mobile);
	await mockParityApi(page);

	await page.goto('/software');

	const mobileNav = page.locator('[data-ui="app-shell-mobile-nav"]');
	await expect(mobileNav).toBeVisible();
	await expect(mobileNav.getByRole('link', { name: 'Home' })).toBeVisible();
	await expect(mobileNav.getByRole('link', { name: 'Surface One' })).toBeVisible();
	await expect(mobileNav.getByRole('link', { name: 'Services', exact: true })).toBeVisible();
	await expect(mobileNav.getByRole('link', { name: 'System Services', exact: true })).toBeVisible();
	await expect(mobileNav.getByRole('button', { name: 'More' })).toBeVisible();

	await mobileNav.getByRole('button', { name: 'More' }).click();

	const overflowSheet = page.locator('[data-ui="app-shell-mobile-overflow-sheet"]');
	await expect(overflowSheet).toBeVisible();
	await expect(overflowSheet.getByRole('link', { name: 'Hosts' })).toBeVisible();
	await expect(overflowSheet.getByRole('link', { name: 'Settings' })).toBeVisible();
	await captureParityScreenshot(page, page, 'ui-parity-responsive-mobile-bottom-nav-overflow.png', 'mobile');
});

test('mobile responsive ui parity: overflow sheet traps focus and closes on Escape', async ({ page }) => {
	await page.setViewportSize(PARITY_VIEWPORT_PRESETS.mobile);
	await mockParityApi(page);

	await page.goto('/software');

	const moreButton = page.locator('[data-ui="app-shell-mobile-nav"]').getByRole('button', { name: 'More' });
	await moreButton.click();

	const overflowSheet = page.locator('[data-ui="app-shell-mobile-overflow-sheet"]');
	const firstOverflowLink = overflowSheet.getByRole('link', { name: 'Hosts' });
	await expect(firstOverflowLink).toBeFocused();

	await page.keyboard.press('Shift+Tab');
	await expect(overflowSheet.getByRole('link', { name: 'Settings' })).toBeFocused();

	await page.keyboard.press('Escape');
	await expect(overflowSheet).toBeHidden();
	await expect(moreButton).toBeFocused();
});

test('mobile responsive ui parity: bottom-centered toasts', async ({ page }) => {
	await page.setViewportSize(PARITY_VIEWPORT_PRESETS.mobile);
	await mockParityApi(page, {
		systemAlerts: [
			{
				id: 'alert-warning',
				severity: 'warning',
				title: 'Certificate warning',
				message: 'Certificate expires soon'
			}
		]
	});

	await page.goto('/software');

	const toastStack = page.locator('[data-ui="toast-notifications"]');
	await expect(toastStack).toBeVisible();

	const box = await toastStack.boundingBox();
	expect(box).not.toBeNull();
	expect(box!.y).toBeGreaterThan(520);
	expect(Math.abs(box!.x + box!.width / 2 - 393 / 2)).toBeLessThan(28);
	await captureParityScreenshot(page, toastStack, 'ui-parity-responsive-mobile-toasts.png', 'mobile');
});

test('tablet responsive ui parity: toasts dismiss on swipe-right', async ({ page }) => {
	await page.setViewportSize(PARITY_VIEWPORT_PRESETS.tablet);
	await mockParityApi(page, {
		systemAlerts: [
			{
				id: 'alert-warning',
				severity: 'warning',
				title: 'Certificate warning',
				message: 'Certificate expires soon'
			}
		]
	});

	await page.goto('/software');

	const toast = page.locator('[data-ui="toast-notification"]').first();
	await expect(toast).toBeVisible();
	const box = await toast.boundingBox();
	expect(box).not.toBeNull();

	await page.mouse.move(box!.x + 20, box!.y + 20);
	await page.mouse.down();
	await page.mouse.move(box!.x + 140, box!.y + 20, { steps: 10 });
	await page.mouse.up();

	await expect(toast).toHaveCount(0);
});

test('mobile responsive ui parity: toasts dismiss on swipe-down', async ({ page }) => {
	await page.setViewportSize(PARITY_VIEWPORT_PRESETS.mobile);
	await mockParityApi(page, {
		systemAlerts: [
			{
				id: 'alert-warning',
				severity: 'warning',
				title: 'Certificate warning',
				message: 'Certificate expires soon'
			}
		]
	});

	await page.goto('/software');

	const toast = page.locator('[data-ui="toast-notification"]').first();
	await expect(toast).toBeVisible();
	const box = await toast.boundingBox();
	expect(box).not.toBeNull();

	await page.mouse.move(box!.x + 20, box!.y + 20);
	await page.mouse.down();
	await page.mouse.move(box!.x + 20, box!.y + 140, { steps: 10 });
	await page.mouse.up();

	await expect(toast).toHaveCount(0);
});

test('mobile responsive ui parity: live terminal opens full-screen', async ({ page }) => {
	await page.setViewportSize(PARITY_VIEWPORT_PRESETS.mobile);
	await installMockWebSocket(page);
	await mockParityApi(page, { softwareDetailHostActiveUpdate: true });

	await page.goto('/software/sw-001');

	await page.getByTitle('View update progress').click();

	const liveTerminalModal = page.getByRole('dialog');
	await expect(liveTerminalModal).toBeVisible();

	const box = await liveTerminalModal.boundingBox();
	expect(box).not.toBeNull();
	expect(box!.width).toBeGreaterThan(388);
	expect(box!.height).toBeGreaterThan(840);
	await captureParityScreenshot(
		page,
		liveTerminalModal,
		'ui-parity-responsive-mobile-terminal-fullscreen.png',
		'mobile'
	);
	const terminalTitlebar = page.locator('[data-ui="terminal-titlebar"]');
	const terminalStatusbar = page.locator('[data-ui="terminal-statusbar"]');
	await expect(terminalTitlebar).toBeVisible();
	await expect(terminalStatusbar).toBeVisible();
	await captureParityScreenshot(page, terminalTitlebar, 'ui-parity-responsive-mobile-terminal-titlebar.png', 'mobile');
	await captureParityScreenshot(
		page,
		terminalStatusbar,
		'ui-parity-responsive-mobile-terminal-statusbar.png',
		'mobile'
	);
});

test('responsive ui parity: live terminal fullscreen responds to viewport breakpoint changes', async ({ page }) => {
	await page.setViewportSize(PARITY_VIEWPORT_PRESETS.mobile);
	await installMockWebSocket(page);
	await mockParityApi(page, { softwareDetailHostActiveUpdate: true });

	await page.goto('/software/sw-001');
	await page.getByTitle('View update progress').click();

	const liveTerminalModal = page.getByRole('dialog');
	await expect(liveTerminalModal).toBeVisible();

	const mobileBox = await liveTerminalModal.boundingBox();
	expect(mobileBox).not.toBeNull();
	expect(mobileBox!.width).toBeGreaterThan(388);

	await page.setViewportSize({ width: 900, height: 1180 });
	await expect.poll(async () => (await liveTerminalModal.boundingBox())?.width ?? 0).toBeLessThan(860);

	await page.setViewportSize({ width: 393, height: 852 });
	await expect.poll(async () => (await liveTerminalModal.boundingBox())?.width ?? 0).toBeGreaterThan(388);
});
