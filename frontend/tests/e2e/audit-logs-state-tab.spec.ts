import { expect, test } from '@playwright/test';

export const SEEDED_CORRELATION_ID = '00000000-0000-0000-0000-000000000abc';

const mockUser = {
	id: '00000000-0000-0000-0000-000000000010',
	email: 'audit@example.com',
	first_name: 'Audit',
	last_name: 'Viewer',
	permissions: ['view_audit_logs']
};

const statefulEntry = {
	id: 'audit-stateful-1',
	actor_type: 'user',
	actor_id: '00000000-0000-0000-0000-000000000011',
	actor_display: 'alice@example.com',
	action_type: 'plugin_config.update',
	action_kind: 'stateful',
	target_type: 'plugin_config',
	target_id: 'cfg-001',
	target_display: 'APT Defaults',
	outcome: 'success',
	details_json: null,
	before_snapshot: { enabled: false, name: 'old' },
	after_snapshot: { enabled: true, name: 'old' },
	correlation_id: SEEDED_CORRELATION_ID,
	request_id: 'req-1',
	occurred_at: '2026-01-01T12:00:00Z'
};

const statefulEntry2 = {
	...statefulEntry,
	id: 'audit-stateful-2',
	target_id: 'cfg-002',
	target_display: 'APT Mirror'
};

const eventEntry = {
	id: 'audit-event-1',
	actor_type: 'user',
	actor_id: '00000000-0000-0000-0000-000000000012',
	actor_display: 'bob@example.com',
	action_type: 'auth.login',
	action_kind: 'event',
	target_type: null,
	target_id: null,
	target_display: null,
	outcome: 'success',
	details_json: null,
	before_snapshot: null,
	after_snapshot: null,
	correlation_id: '00000000-0000-0000-0000-000000000999',
	request_id: 'req-2',
	occurred_at: '2026-01-01T11:00:00Z'
};

const allEntries = [statefulEntry, statefulEntry2, eventEntry];

function makePage(items: typeof allEntries) {
	return { items, total: items.length, page: 1, per_page: 25, total_pages: 1 };
}

async function mockAuditApi(page: import('@playwright/test').Page) {
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
				user: mockUser
			});
		}
		if (method === 'GET' && path === '/api/v1/auth/me') return json(mockUser);
		if (method === 'GET' && path === '/api/v1/system/alerts') return json({ alerts: [] });
		if (method === 'GET' && path === '/api/v1/surfaces') return json([]);

		if (method === 'GET' && path === '/api/v1/audit-logs') {
			const correlationId = url.searchParams.get('correlation_id');
			const filtered = correlationId ? allEntries.filter((e) => e.correlation_id === correlationId) : allEntries;
			return json(makePage(filtered));
		}

		if (method === 'GET' && path === '/api/v1/admin/events') return route.abort();

		return route.abort();
	});
}

test('state tab renders diff for stateful row and is hidden for event row', async ({ page }) => {
	await mockAuditApi(page);
	await page.goto('/audit-logs');
	await page.waitForLoadState('networkidle');

	// Click the stateful row
	await page.getByText('plugin_config.update').first().click();

	// State tab must appear
	await expect(page.getByRole('tab', { name: 'State' })).toBeVisible();

	// Click the State tab
	await page.getByRole('tab', { name: 'State' }).click();

	// Diff content: 'enabled' key changed (false → true), 'name' unchanged (suppressed for stateful display)
	// The StateTab shows all keys including unchanged; 'enabled' is present
	await expect(page.getByText('enabled')).toBeVisible();

	// Close the modal
	await page.keyboard.press('Escape');
	await expect(page.getByRole('tab', { name: 'State' })).not.toBeVisible();

	// Click the event row — State tab must NOT appear
	await page.getByText('auth.login').first().click();
	await expect(page.getByRole('tab', { name: 'Details' })).toBeVisible();
	await expect(page.getByRole('tab', { name: 'State' })).not.toBeVisible();
});

test('correlation_id filter narrows list', async ({ page }) => {
	await mockAuditApi(page);
	await page.goto('/audit-logs');
	await page.waitForLoadState('networkidle');

	// Verify all 3 rows initially (1 header + 3 data rows = 4 total)
	await expect(page.getByRole('row')).toHaveCount(4);

	// Filter by SEEDED_CORRELATION_ID → only 2 stateful entries match
	await page.getByLabel('Correlation ID').fill(SEEDED_CORRELATION_ID);
	await page.getByRole('button', { name: 'Apply Filters' }).click();
	await page.waitForLoadState('networkidle');

	// 1 header + 2 data rows = 3 total
	await expect(page.getByRole('row')).toHaveCount(3);
});
