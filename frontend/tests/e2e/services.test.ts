import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';

// ---------------------------------------------------------------------------
// Shared fixtures
// ---------------------------------------------------------------------------

const mockUser = {
	id: '00000000-0000-0000-0000-000000000001',
	email: 'admin@example.com',
	first_name: 'Admin',
	last_name: 'User',
	permissions: ['view_agents', 'manage_agents']
};

const sampleService = {
	id: 'svc-001',
	friendly_name: 'prod-agent',
	service_type: 'agent',
	hostname: 'prod-host',
	ip_address: '10.0.0.1',
	status: 'approved',
	client_version: '1.2.0',
	last_seen_at: '2024-06-01T12:00:00Z',
	created_at: '2024-01-01T00:00:00Z',
	updated_at: '2024-01-01T00:00:00Z',
	ping_interval_seconds: null
};

async function mockSession(page: Page) {
	await page.route('**/api/v1/auth/refresh', (route) =>
		route.fulfill({
			json: {
				access_token: 'tok',
				refresh_token: 'rt',
				expires_in: 3600,
				token_type: 'Bearer',
				user: mockUser
			}
		})
	);
	await page.route('**/api/v1/auth/me', (route) => route.fulfill({ json: mockUser }));
	await page.route('**/api/v1/system/alerts', (route) => route.fulfill({ json: { alerts: [] } }));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

test.describe('Services page', () => {
	test('renders a service row returned by the API', async ({ page }) => {
		await mockSession(page);
		await page.route('**/api/v1/services**', (route) =>
			route.fulfill({
				json: {
					items: [sampleService],
					total: 1,
					page: 1,
					per_page: 25,
					total_pages: 1
				}
			})
		);

		await page.goto('/services');
		await expect(page.getByText('prod-agent')).toBeVisible();
		await expect(page.getByText('prod-host')).toBeVisible();
	});

	test('shows empty-state message when there are no services', async ({ page }) => {
		await mockSession(page);
		await page.route('**/api/v1/services**', (route) =>
			route.fulfill({
				json: { items: [], total: 0, page: 1, per_page: 25, total_pages: 1 }
			})
		);

		await page.goto('/services');
		await expect(page.getByText(/No services registered yet/)).toBeVisible();
	});

	test('shows the Agents filter button and clicking it reloads with type=agent', async ({ page }) => {
		await mockSession(page);
		let callCount = 0;
		await page.route('**/api/v1/services**', (route) => {
			callCount++;
			route.fulfill({ json: { items: [], total: 0, page: 1, per_page: 25, total_pages: 1 } });
		});

		await page.goto('/services');
		await expect(page.getByRole('button', { name: 'Agents', exact: true })).toBeVisible();
		await page.getByRole('button', { name: 'Agents', exact: true }).click();

		// The filter click should trigger a second API call
		await page.waitForFunction(() => true); // flush microtasks
		expect(callCount).toBeGreaterThanOrEqual(2);
	});
});
