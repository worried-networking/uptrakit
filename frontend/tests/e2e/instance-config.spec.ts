import { expect, test } from '@playwright/test';

const mockUserWithView = {
	id: '00000000-0000-0000-0000-000000000301',
	email: 'admin@example.com',
	first_name: 'Admin',
	last_name: 'User',
	permissions: ['view_instance_config_state', 'manage_global_settings']
};

const mockUserWithManage = {
	...mockUserWithView,
	permissions: ['view_instance_config_state', 'manage_instance_config_state', 'manage_global_settings']
};

const mockUserWithoutView = {
	...mockUserWithView,
	permissions: ['manage_global_settings']
};

const idleConfigState: {
	coordinator_state: string;
	degraded: { since: string; failed_subsystems: string[]; reason: string } | null;
	file: {
		path: string;
		digest: string;
		loaded_at: string;
		pending_digest: string | null;
		pending_detected_at: string | null;
	};
	last_reload: { completed_at: string; sections: string[]; per_subsystem_ms: Record<string, number> } | null;
	sections: Record<string, unknown>;
	recent_events: unknown[];
} = {
	coordinator_state: 'idle',
	degraded: null,
	file: {
		path: '/etc/uptrakit/controller.toml',
		digest: 'abc123',
		loaded_at: '2026-05-14T10:00:00Z',
		pending_digest: null,
		pending_detected_at: null
	},
	last_reload: {
		completed_at: '2026-05-14T09:55:00Z',
		sections: ['network', 'tls'],
		per_subsystem_ms: { web_api: 12, nats: 8 }
	},
	sections: { network: '<redacted>', tls: '<redacted>' },
	recent_events: []
};

const degradedConfigState = {
	...idleConfigState,
	coordinator_state: 'degraded',
	degraded: {
		since: '2026-05-14T10:05:00Z',
		failed_subsystems: ['nats'],
		reason: 'NATS revert failed: connection timeout'
	}
};

async function mockSettingsApi(
	page: import('@playwright/test').Page,
	user: typeof mockUserWithView,
	configState: typeof idleConfigState = idleConfigState
) {
	await page.route('**/api/v1/**', async (route) => {
		const url = new URL(route.request().url());
		const path = url.pathname;
		const method = route.request().method();
		const json = (body: unknown, status = 200) =>
			route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });

		if (method === 'POST' && path === '/api/v1/auth/refresh') {
			return json({
				access_token: 'test-token',
				refresh_token: 'test-refresh',
				expires_in: 3600,
				token_type: 'Bearer',
				user
			});
		}
		if (method === 'GET' && path === '/api/v1/auth/me') return json(user);
		if (method === 'GET' && path === '/api/v1/system/alerts') return json({ alerts: [] });
		if (method === 'GET' && path === '/api/v1/surfaces') return json([]);
		if (method === 'GET' && path === '/api/v1/instance/config-state') return json(configState);
		if (method === 'POST' && path === '/api/v1/instance/config-reload/clear-degraded') return json(idleConfigState);
		if (method === 'GET' && path === '/api/v1/admin/events') return route.abort();
		return route.abort();
	});
}

test('instance config tab hidden without view_instance_config_state permission', async ({ page }) => {
	await mockSettingsApi(page, mockUserWithoutView);
	await page.goto('/settings?tab=instance-config');
	await expect(page.getByRole('tab', { name: 'Instance Configuration' })).not.toBeVisible();
});

test('instance config tab renders file path and coordinator state', async ({ page }) => {
	await mockSettingsApi(page, mockUserWithView);
	await page.goto('/settings?tab=instance-config');
	await expect(page.getByText('/etc/uptrakit/controller.toml')).toBeVisible();
	// `idle` is the no-banner state: InstanceConfigTab only renders a
	// coordinator banner when state is `degraded`. Assert the banner is absent
	// and the structural section cards are visible — that's the externally
	// observable "idle" signal.
	await expect(page.getByRole('heading', { name: 'Coordinator degraded' })).not.toBeVisible();
	await expect(page.getByRole('heading', { name: 'Config File' })).toBeVisible();
	await expect(page.getByRole('heading', { name: 'Last Reload' })).toBeVisible();
});

test('clear degraded button hidden without manage_instance_config_state permission', async ({ page }) => {
	await mockSettingsApi(page, mockUserWithView, degradedConfigState);
	await page.goto('/settings?tab=instance-config');
	await expect(page.getByRole('button', { name: /clear degraded/i })).not.toBeVisible();
});

test('clear degraded button visible with manage_instance_config_state permission', async ({ page }) => {
	await mockSettingsApi(page, mockUserWithManage, degradedConfigState);
	await page.goto('/settings?tab=instance-config');
	await expect(page.getByRole('button', { name: /clear degraded/i })).toBeVisible();
});

test('sections show redacted values', async ({ page }) => {
	await mockSettingsApi(page, mockUserWithView);
	await page.goto('/settings?tab=instance-config');
	await expect(page.getByText('<redacted>', { exact: false })).toBeVisible();
});

test('no reload now button present', async ({ page }) => {
	await mockSettingsApi(page, mockUserWithManage);
	await page.goto('/settings?tab=instance-config');
	await expect(page.getByRole('button', { name: /reload now/i })).not.toBeVisible();
});
