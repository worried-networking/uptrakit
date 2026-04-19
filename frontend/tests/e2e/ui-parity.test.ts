import { expect, test } from '@playwright/test';
import type { Locator, Page } from '@playwright/test';
import type { SurfaceReadResponse, SurfaceResponse } from '../../src/lib/surfaces/contract';
import {
	buildParityProvider,
	buildParitySurfacePageFixture,
	buildParitySurfaceTab,
	buildSharedVisualParityFixture,
	buildSettingsTabsParityFixture,
	buildSoftwareTabsParityFixture
} from '../../src/lib/test-fixtures/ui-parity';
import {
	PARITY_DYNAMIC_MASK_SELECTOR,
	PARITY_MAX_DIFF_PIXEL_RATIO,
	PARITY_MAX_MASKED_AREA_RATIO,
	PARITY_VIEWPORT_PRESETS,
	expectParityScreenshot,
	type ParityViewportPreset
} from './parity-config';

test.use({
	viewport: PARITY_VIEWPORT_PRESETS.desktop,
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
	softwareDetailHostActiveUpdate?: boolean;
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
		'view_services',
		'approve_services',
		'reject_services',
		'remove_services',
		'update_services',
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
const sharedVisualParity = buildSharedVisualParityFixture();

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

const hostsListItem = {
	id: sharedVisualParity.pillBadge.host.id,
	machine_id: 'machine-pill-001',
	hostname: 'pill-host.local',
	friendly_name: sharedVisualParity.pillBadge.host.friendlyName,
	os_type: 'Linux',
	os_version: 'Ubuntu 24.04',
	architecture: 'x86_64',
	ip_address: '10.0.0.8',
	last_seen_at: '2024-06-01T12:00:00Z',
	created_at: '2024-01-01T00:00:00Z',
	updated_at: '2024-01-01T00:00:00Z',
	agents: [{ id: 'agent-pill-001', friendly_name: 'Pill Agent', status: 'approved' }],
	tags: [
		{
			id: 'tag-pill-001',
			name: sharedVisualParity.pillBadge.host.tagName,
			color: '#334155'
		}
	]
};

const servicesListItem = {
	id: sharedVisualParity.contextMenu.service.id,
	friendly_name: sharedVisualParity.contextMenu.service.friendlyName,
	service_label: 'SSH AGENT',
	hostname: 'service-node.local',
	ip_address: '10.0.0.9',
	status: 'pending',
	is_embedded: false,
	yielded_to: [],
	last_seen_at: '2024-06-01T12:00:00Z',
	capabilities: ['software_discovery'],
	ping_interval_seconds: 30
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
	const models = Object.fromEntries(
		surfaces.map((surface) => [surface.surface_id, buildSurfaceRead(surface, `${surface.label} Loaded Content`)])
	);
	models[sharedVisualParity.tableFooter.surface.surface_id] = sharedVisualParity.tableFooter.readModel;
	return models;
}

async function freezeParityInputs(page: Page) {
	await page.addInitScript(() => {
		localStorage.setItem('theme-mode', 'light');
	});
	await page.emulateMedia({ colorScheme: 'light', reducedMotion: 'reduce' });
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

async function captureParityScreenshot(
	page: Page,
	target: Page | Locator,
	name: string,
	viewport: ParityViewportPreset = 'desktop',
	options?: {
		maskSelectors?: readonly string[];
		waiverMaxMaskedAreaRatio?: number;
	}
) {
	await expectParityScreenshot({
		page,
		target,
		name,
		viewport,
		maskSelectors: options?.maskSelectors,
		waiverMaxMaskedAreaRatio: options?.waiverMaxMaskedAreaRatio
	});
}

async function mountActionBadgeParityFixture(page: Page) {
	await page.evaluate((actionBadge) => {
		document.querySelector('[data-testid="parity-clickable-badge"]')?.remove();

		const root = document.createElement('div');
		root.setAttribute('data-testid', 'parity-clickable-badge');
		root.style.position = 'fixed';
		root.style.top = '112px';
		root.style.left = '112px';
		root.style.zIndex = '80';
		root.style.padding = '12px';
		root.style.background = 'var(--bg-surface)';
		root.style.border = '1px solid var(--border-subtle)';
		root.style.borderRadius = '4px';

		const button = document.createElement('button');
		button.type = 'button';
		button.setAttribute('data-ui', 'action-badge');
		button.setAttribute('data-variant', actionBadge.variant);
		button.setAttribute('data-tone', actionBadge.tone);
		button.style.position = 'relative';
		button.style.display = 'inline-flex';
		button.style.minWidth = 'max-content';
		button.style.minHeight = '14px';
		button.style.alignItems = 'center';
		button.style.justifyContent = 'center';
		button.style.padding = '0 6px';
		button.style.borderRadius = '2px';
		button.style.border = '1px solid var(--color-info-border)';
		button.style.background = 'var(--color-info-bg)';
		button.style.color = 'var(--color-info)';
		button.style.fontSize = '7.5px';
		button.style.fontWeight = '700';
		button.style.lineHeight = '1';
		button.style.letterSpacing = '0.04em';
		button.style.textTransform = 'uppercase';

		const idle = document.createElement('span');
		idle.className = 'idle';
		idle.textContent = actionBadge.idleLabel;
		button.append(idle);

		const hover = document.createElement('span');
		hover.className = 'hov';
		hover.setAttribute('aria-hidden', 'true');
		hover.textContent = actionBadge.hoverLabel;
		hover.style.position = 'absolute';
		hover.style.inset = '0';
		hover.style.display = 'flex';
		hover.style.alignItems = 'center';
		hover.style.justifyContent = 'center';
		hover.style.visibility = 'hidden';
		button.append(hover);
		button.addEventListener('mouseenter', () => {
			idle.style.visibility = 'hidden';
			hover.style.visibility = 'visible';
		});
		button.addEventListener('mouseleave', () => {
			idle.style.visibility = 'visible';
			hover.style.visibility = 'hidden';
		});

		root.append(button);
		document.body.append(root);
	}, sharedVisualParity.actionBadge);
}

async function mockParityApi(page: Page, scenario: MockScenario = {}): Promise<MockParityApiResult> {
	const surfaces = scenario.surfaces ?? paritySurfaces;
	const readModels = scenario.readModels ?? buildDefaultReadModels(surfaces);
	const runtimeActive = scenario.runtimeActive ?? true;
	const softwareDetailItem = buildSoftwareDetailItem(
		scenario.softwareDetailHostActiveUpdate ? 'history-live-001' : null
	);
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
		const invokeMatch = path.match(/^\/api\/v1\/surfaces\/([^/]+)\/interactions\/([^/]+)(?:\/invoke)?$/);
		if (method === 'POST' && invokeMatch) {
			const surfaceId = decodeURIComponent(invokeMatch[1] ?? '');
			const interactionId = decodeURIComponent(invokeMatch[2] ?? '');
			if (
				surfaceId === sharedVisualParity.tableFooter.surface.surface_id &&
				interactionId === sharedVisualParity.tableFooter.dataLoadInteractionId
			) {
				return json(sharedVisualParity.tableFooter.dataLoadResponse);
			}
			return json({ error: `Unhandled surface interaction fixture: ${surfaceId}/${interactionId}` }, 404);
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
		if (method === 'GET' && path === '/api/v1/hosts') {
			const requestedPage = Number(url.searchParams.get('page') ?? '1');
			const requestedPerPage = Number(url.searchParams.get('per_page') ?? '50');
			return json({
				items: [hostsListItem],
				total: 1,
				page: requestedPage,
				per_page: requestedPerPage,
				total_pages: 1
			});
		}
		if (method === 'GET' && path === '/api/v1/services') {
			const requestedPage = Number(url.searchParams.get('page') ?? '1');
			const requestedPerPage = Number(url.searchParams.get('per_page') ?? '50');
			return json({
				items: [servicesListItem],
				total: 1,
				page: requestedPage,
				per_page: requestedPerPage,
				total_pages: 1
			});
		}
		if (method === 'GET' && path === '/api/v1/update-history') {
			return json({
				items: [
					{
						id: 'hist-001',
						host_id: hostDetail.id,
						host_name: hostDetail.hostname,
						software_item_id: softwareListItem.id,
						software_item_name: softwareListItem.name,
						from_version: '1.24.0',
						to_version: '1.27.0',
						status: 'completed',
						actor_type: 'user',
						actor_id: mockUser.id,
						started_at: '2024-06-01T12:00:00Z',
						completed_at: '2024-06-01T12:02:00Z',
						output: 'Update complete.',
						created_at: '2024-06-01T12:00:00Z',
						interactive: false,
						output_truncated: false,
						pre_update_protection_status: null,
						pre_update_protection_summary: null,
						recovery_hint: null
					}
				],
				total: 1,
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

test('ui parity governance: enforce harness diff and mask budgets', () => {
	expect(PARITY_MAX_DIFF_PIXEL_RATIO).toBe(0.005);
	expect(PARITY_MAX_MASKED_AREA_RATIO).toBe(0.15);
});

test('ui parity governance: reject non-allowlisted mask selectors', async ({ page }) => {
	await mockParityApi(page);
	await page.goto('/software');

	const nav = page.locator('[data-ui="app-shell-nav"]');
	await expect(nav).toBeVisible();

	await expect(
		expectParityScreenshot({
			page,
			target: nav,
			name: 'ui-parity-governance-invalid-mask.png',
			viewport: 'desktop',
			maskSelectors: ['.forbidden-mask-selector']
		})
	).rejects.toThrow(/not allowlisted/i);
});

test('ui parity governance: reject viewport profile drift', async ({ page }) => {
	await mockParityApi(page);
	await page.goto('/software');

	const nav = page.locator('[data-ui="app-shell-nav"]');
	await expect(nav).toBeVisible();

	await expect(
		expectParityScreenshot({
			page,
			target: nav,
			name: 'ui-parity-governance-viewport-drift.png',
			viewport: 'mobile'
		})
	).rejects.toThrow(/viewport mismatch/i);
});

test('ui parity governance: reject reduced-motion drift', async ({ page }) => {
	await mockParityApi(page);
	await page.goto('/software');
	await page.emulateMedia({ colorScheme: 'light', reducedMotion: 'no-preference' });

	const nav = page.locator('[data-ui="app-shell-nav"]');
	await expect(nav).toBeVisible();

	await expect(
		expectParityScreenshot({
			page,
			target: nav,
			name: 'ui-parity-governance-motion-drift.png',
			viewport: 'desktop'
		})
	).rejects.toThrow(/reduced-motion/i);
});

test('ui parity governance: mask budget uses union area without double-counting overlap', async ({ page }) => {
	await page.setContent(`
		<div
			data-testid="parity-mask-union-target"
			style="position:relative;width:200px;height:200px;margin:20px;border:1px solid #111827;background:#e5e7eb;"
		>
			<div
				data-visual-dynamic
				style="position:absolute;left:20px;top:20px;width:60px;height:60px;background:#ef4444;"
			></div>
			<div
				data-visual-dynamic
				style="position:absolute;left:20px;top:20px;width:60px;height:60px;background:#f97316;"
			></div>
		</div>
	`);

	const target = page.getByTestId('parity-mask-union-target');
	await expect(target).toBeVisible();
	await captureParityScreenshot(page, target, 'ui-parity-governance-mask-union-area.png', 'desktop', {
		maskSelectors: [PARITY_DYNAMIC_MASK_SELECTOR]
	});
});

test('settings tabs ui parity: built-in settings tab vs settings.tabs', async ({ page }) => {
	await mockParityApi(page);

	await page.goto('/settings?tab=notifications.email');

	const tabStrip = page.locator('[data-ui="tab-strip"]');
	await expect(tabStrip).toBeVisible();
	await expect(page.getByRole('tab', { name: 'General' })).toBeVisible();
	await expect(page.getByRole('tab', { name: 'Email Channels' })).toBeVisible();

	await captureParityScreenshot(page, tabStrip, 'ui-parity-settings-tabs.png');
});

test('software tabs ui parity: built-in software tab vs software.tabs', async ({ page }) => {
	await mockParityApi(page);

	await page.goto('/software?tab=proxmox.hosts');

	const tabStrip = page.locator('[data-ui="tab-strip"]');
	await expect(tabStrip).toBeVisible();
	await expect(page.getByRole('tab', { name: /^Featured$/ })).toBeVisible();
	await expect(page.getByRole('tab', { name: 'Proxmox VE Hosts' })).toBeVisible();

	await captureParityScreenshot(page, tabStrip, 'ui-parity-software-tabs.png');
});

test('software page ui parity: grouped software row shell', async ({ page }) => {
	await mockParityApi(page);

	await page.goto('/software');

	const groupRow = page.getByTestId('software-group-sw-001');
	await expect(groupRow).toBeVisible();
	await captureParityScreenshot(page, groupRow, 'ui-parity-software-group-row.png');
});

test('history page ui parity: chronological feed row shell', async ({ page }) => {
	await mockParityApi(page);

	await page.goto('/history');

	const historyFeedItem = page.getByTestId('history-feed-item-hist-001');
	await expect(historyFeedItem).toBeVisible();
	await captureParityScreenshot(page, historyFeedItem, 'ui-parity-history-feed-row.png');
});

test('terminal ui parity: software detail shared terminal shell chrome', async ({ page }) => {
	await installMockWebSocket(page);
	await mockParityApi(page, { softwareDetailHostActiveUpdate: true });

	await page.goto('/software/sw-001');
	await page.getByTitle('View update progress').click();

	const terminalTitlebar = page.locator('[data-ui="terminal-titlebar"]');
	const terminalStatusbar = page.locator('[data-ui="terminal-statusbar"]');
	await expect(terminalTitlebar).toBeVisible();
	await expect(terminalStatusbar).toBeVisible();

	await captureParityScreenshot(page, terminalTitlebar, 'ui-parity-terminal-titlebar-chrome.png');
	await captureParityScreenshot(page, terminalStatusbar, 'ui-parity-terminal-statusbar-chrome.png');
});

test('navigation ui parity: built-in nav item vs surface.page', async ({ page }) => {
	await mockParityApi(page);

	await page.goto('/software');

	const nav = page.locator('[data-ui="app-shell-nav"]');
	await expect(nav).toBeVisible();
	await expect(nav.getByRole('link', { name: 'Software' })).toBeVisible();
	await expect(nav.getByRole('link', { name: 'Surface One' })).toBeVisible();

	await captureParityScreenshot(page, nav, 'ui-parity-app-nav-built-in-vs-surface-page.png');
});

test('navigation ui parity: deterministic tie-breakers for built-in and surface.page items', async ({ page }) => {
	const tieBreakerSurfaces: SurfaceResponse[] = [
		...paritySurfaces,
		buildParitySurfaceTab('surface.tie.b', 'Hosts', {
			slot: 'surface.page',
			priority: 400,
			root_node: { kind: 'text_block', text: 'surface-b' }
		}),
		buildParitySurfaceTab('surface.tie.a', 'Hosts', {
			slot: 'surface.page',
			priority: 400,
			root_node: { kind: 'text_block', text: 'surface-a' }
		})
	];

	await mockParityApi(page, { surfaces: tieBreakerSurfaces });
	await page.goto('/software');
	await expect(page.locator('[data-ui="app-shell-nav"]').first()).toBeVisible();

	const hostLinks = await page
		.locator('[data-ui="app-shell-nav"] [data-ui="app-shell-nav-item"]')
		.evaluateAll((nodes) =>
			nodes
				.map((node) => ({
					label: node.textContent?.trim() ?? '',
					href: node.getAttribute('href') ?? ''
				}))
				.filter((item) => item.label === 'Hosts')
				.map((item) => item.href)
		);

	expect(hostLinks).toEqual(['/hosts', '/surfaces/surface.tie.a', '/surfaces/surface.tie.b']);
});

test('shell ui parity: shared shell z-index scale for header, sidebar, and toasts', async ({ page }) => {
	await mockParityApi(page, {
		surfaces: paritySurfaces,
		readModels: buildDefaultReadModels(paritySurfaces)
	});
	await page.goto('/software');
	await expect(page.locator('[data-ui="app-shell-header"]')).toBeVisible();
	await expect(page.locator('[data-ui="app-shell-sidebar"][data-variant="desktop"]')).toBeVisible();

	const zIndex = await page.evaluate(() => {
		const getZ = (selector: string) => {
			const element = document.querySelector(selector);
			return element ? window.getComputedStyle(element).zIndex : null;
		};
		return {
			header: getZ('[data-ui="app-shell-header"]'),
			sidebar: getZ('[data-ui="app-shell-sidebar"][data-variant="desktop"]'),
			toasts: getZ('[data-ui="toast-notifications"]')
		};
	});

	expect(zIndex.header).toBe('10');
	expect(zIndex.sidebar).toBe('20');
	expect(zIndex.toasts).toBe('500');
});

test('settings panel ui parity: settings.below.global', async ({ page }) => {
	await mockParityApi(page);

	await page.goto('/settings?tab=global-settings');

	const extensionCard = page
		.locator('[data-ui="section-card"]')
		.filter({ has: page.getByRole('heading', { name: 'Global Audit Extension' }) });
	await expect(extensionCard).toBeVisible();
	await captureParityScreenshot(page, extensionCard, 'ui-parity-settings-below-global-panel.png');
});

test('host detail ui parity: host_detail.tabs slot container', async ({ page }) => {
	await mockParityApi(page);

	await page.goto('/hosts/host-001');

	const hostDetailSurfaceCard = page
		.locator('[data-ui="section-card"]')
		.filter({ has: page.getByRole('heading', { name: 'Proxmox VE Info' }) });
	await expect(hostDetailSurfaceCard).toBeVisible();
	await captureParityScreenshot(page, hostDetailSurfaceCard, 'ui-parity-host-detail-tabs-slot.png');
});

test('software host context ui parity: software_item.host_context_menu launcher and opened modal', async ({ page }) => {
	await mockParityApi(page);

	await page.goto('/software/sw-001');

	const hostRow = page.locator('tr').filter({ has: page.getByRole('link', { name: 'prod-server' }) });
	await expect(hostRow).toBeVisible();
	await captureParityScreenshot(page, hostRow, 'ui-parity-software-host-context-launcher.png');

	await page.getByRole('button', { name: /actions for prod-server/i }).click();
	await expect(page.getByRole('menuitem', { name: 'Host Context Surface' })).toBeVisible();
	await page.getByRole('menuitem', { name: 'Host Context Surface' }).click();

	const contextModal = page.getByRole('dialog');
	await expect(contextModal).toBeVisible();
	await captureParityScreenshot(page, contextModal, 'ui-parity-software-host-context-modal.png');
});

test('shared primitive ui parity: context menu shell sizing', async ({ page }) => {
	await mockParityApi(page);

	await page.goto('/services');
	await page
		.getByRole('button', { name: `Actions for ${sharedVisualParity.contextMenu.service.friendlyName}` })
		.click();

	const menuShell = page.locator('[data-ui="context-menu-shell"]');
	await expect(menuShell).toBeVisible();
	await captureParityScreenshot(page, menuShell, 'ui-parity-context-menu-shell.png');
});

test('shared primitive ui parity: context menu item sizing', async ({ page }) => {
	await mockParityApi(page);

	await page.goto('/services');
	await page.getByRole('checkbox', { name: `Select ${sharedVisualParity.contextMenu.service.friendlyName}` }).check();
	await page.getByRole('button', { name: 'More actions' }).click();

	const contextMenuItems = page
		.locator('[role="menu"]')
		.filter({
			has: page.locator('[data-ui="context-menu-item"]')
		})
		.first();
	await expect(contextMenuItems).toBeVisible();
	await captureParityScreenshot(page, contextMenuItems, 'ui-parity-context-menu-item-row.png');
});

test('shared primitive ui parity: action badge idle and hover states', async ({ page }) => {
	await mockParityApi(page);

	await page.goto('/software');
	await mountActionBadgeParityFixture(page);

	const fixture = page.getByTestId('parity-clickable-badge');
	const actionBadge = fixture.locator('[data-ui="action-badge"]');
	await expect(actionBadge).toBeVisible();
	await page.mouse.move(12, 12);

	await captureParityScreenshot(page, fixture, 'ui-parity-clickable-badge.png');

	await actionBadge.hover();
	await captureParityScreenshot(page, fixture, 'ui-parity-clickable-badge-hover.png');
});

test('shared primitive ui parity: pill badge sizing in host tags', async ({ page }) => {
	await mockParityApi(page);

	await page.goto('/hosts');

	const hostRow = page.locator('tr').filter({
		has: page.getByRole('link', { name: sharedVisualParity.pillBadge.host.friendlyName })
	});
	await expect(hostRow).toBeVisible();
	const pillBadge = hostRow.locator('[data-ui="pill-badge"]');
	await expect(pillBadge).toBeVisible();
	await captureParityScreenshot(page, pillBadge, 'ui-parity-pill-badge.png');
});

test('shared primitive ui parity: table footer totals and pagination alignment', async ({ page }) => {
	const tableFooterSurfaces = [...paritySurfaces, sharedVisualParity.tableFooter.surface];
	const tableFooterReadModels = buildDefaultReadModels(tableFooterSurfaces);
	await mockParityApi(page, {
		surfaces: tableFooterSurfaces,
		readModels: tableFooterReadModels
	});

	await page.goto(`/surfaces/${sharedVisualParity.tableFooter.surface.surface_id}`);

	const tableFooter = page.locator('[data-ui="table-footer-bar"]');
	await expect(tableFooter).toBeVisible();
	await expect(tableFooter.locator('nav[aria-label="Pagination"]')).toBeVisible();
	await captureParityScreenshot(page, tableFooter, 'ui-parity-table-footer.png');
});

test('surface page ui parity: surface.page loaded shell', async ({ page }) => {
	await mockParityApi(page);

	await page.goto('/surfaces/surface.one');

	const loadedSurfaceCard = page
		.locator('[data-ui="section-card"]')
		.filter({ has: page.getByText('Surface One Loaded Content', { exact: true }) });
	await expect(loadedSurfaceCard).toBeVisible();
	await expect(page.getByText('Surface One Loaded Content')).toBeVisible();
	await captureParityScreenshot(page, loadedSurfaceCard, 'ui-parity-surface-page-loaded-shell.png');
});

test('surface page ui parity: surface.page runtime-state shell', async ({ page }) => {
	const runtimeReadModels = buildDefaultReadModels(paritySurfaces);
	expect(runtimeReadModels['surface.one']).toBeDefined();

	const apiRequests = await mockParityApi(page, {
		runtimeActive: false,
		readModels: runtimeReadModels
	});

	await page.goto('/surfaces/surface.one');

	const runtimeStateCard = page.locator('[data-ui="section-card"]').filter({
		has: page.getByText('Surface contract mismatch detected. Please refresh and try again.', {
			exact: true
		})
	});
	await expect(runtimeStateCard).toBeVisible();
	await expect(page.locator('[data-ui="app-shell-nav"]').getByRole('link', { name: 'Surface One' })).toHaveCount(0);
	await expect(page.getByText('Surface contract mismatch detected. Please refresh and try again.')).toBeVisible();
	await expect(page.getByText('Surface One Loaded Content')).toHaveCount(0);
	expect(apiRequests.readRequests).not.toContain('surface.one');
	await captureParityScreenshot(page, runtimeStateCard, 'ui-parity-surface-page-runtime-state-shell.png');
});
