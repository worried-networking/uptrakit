import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';

// ---------------------------------------------------------------------------
// Shared fixtures
// ---------------------------------------------------------------------------

const mockUser = {
	id: '00000000-0000-0000-0000-000000000002',
	email: 'admin@example.com',
	first_name: 'Admin',
	last_name: 'User',
	permissions: ['view_hosts', 'update_hosts', 'deactivate_hosts', 'view_software']
};

const sampleHost = {
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
	agents: []
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

test.describe('Hosts page', () => {
	test('renders a host row returned by the API', async ({ page }) => {
		await mockSession(page);
		await page.route('**/api/v1/hosts**', (route) =>
			route.fulfill({
				json: {
					items: [sampleHost],
					total: 1,
					page: 1,
					per_page: 25,
					total_pages: 1
				}
			})
		);

		await page.goto('/hosts');
		await expect(page.getByText('Production Server')).toBeVisible();
		await expect(page.getByText('prod-server')).toBeVisible();
		await expect(page.getByText('Ubuntu 24.04')).toBeVisible();
	});

	test('shows empty-state message when there are no hosts', async ({ page }) => {
		await mockSession(page);
		await page.route('**/api/v1/hosts**', (route) =>
			route.fulfill({
				json: { items: [], total: 0, page: 1, per_page: 25, total_pages: 1 }
			})
		);

		await page.goto('/hosts');
		await expect(page.getByText(/No hosts discovered yet/)).toBeVisible();
	});

	test('shows deactivate option in the context menu for a host', async ({ page }) => {
		await mockSession(page);
		await page.route('**/api/v1/hosts**', (route) =>
			route.fulfill({
				json: {
					items: [sampleHost],
					total: 1,
					page: 1,
					per_page: 25,
					total_pages: 1
				}
			})
		);

		await page.goto('/hosts');
		await expect(page.getByText('Production Server')).toBeVisible();

		// Open the context menu for the host
		await page.getByRole('button', { name: /^actions for production server$/i }).click();
		await expect(page.getByRole('menuitem', { name: /deactivate/i })).toBeVisible();
	});

	test('deactivate confirmation dialog appears after clicking Deactivate', async ({ page }) => {
		await mockSession(page);
		await page.route('**/api/v1/hosts**', (route) =>
			route.fulfill({
				json: {
					items: [sampleHost],
					total: 1,
					page: 1,
					per_page: 25,
					total_pages: 1
				}
			})
		);

		await page.goto('/hosts');
		await expect(page.getByText('Production Server')).toBeVisible();

		await page.getByRole('button', { name: /^actions for production server$/i }).click();
		await page.getByRole('menuitem', { name: /deactivate/i }).click();

		// Confirmation dialog should appear
		const dialog = page.getByRole('dialog');
		await expect(dialog).toBeVisible();
		await expect(dialog.getByRole('heading', { name: 'Deactivate Host' })).toBeVisible();
		await expect(dialog.getByRole('button', { name: 'Deactivate' })).toBeVisible();
	});
});
