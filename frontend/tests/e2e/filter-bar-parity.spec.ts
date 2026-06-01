import { expect, test } from '@playwright/test';

/**
 * Parity tests: verify that separate filter SectionCards are gone and
 * FilterBar is integrated into the table card header on all in-scope pages.
 *
 * These tests require the dev server to be running with a valid session.
 * Run: npm run test:e2e -- filter-bar-parity
 */

const PAGES_WITH_FILTER_BAR = [
	{ path: '/software', title: 'Software', removedSection: null },
	{ path: '/host-tags', title: 'Host Tags', removedSection: 'Search' },
	{ path: '/history', title: 'History Feed', removedSection: 'Filters' },
	{ path: '/services', title: 'Registered Services', removedSection: 'Service Filters' },
	{ path: '/system-services', title: 'Registered System Services', removedSection: 'Status Filters' },
	{ path: '/hosts', title: 'Registered Hosts', removedSection: null }
];

for (const { path, title, removedSection } of PAGES_WITH_FILTER_BAR) {
	test(`${path}: [data-ui="filter-bar"] is present in the table card`, async ({ page }) => {
		await page.goto(path);
		await expect(page.locator(`h1, h2`).filter({ hasText: title })).toBeVisible({ timeout: 10_000 });
		await expect(page.locator('[data-ui="filter-bar"]')).toBeVisible();
	});

	if (removedSection) {
		test(`${path}: separate "${removedSection}" SectionCard is absent`, async ({ page }) => {
			await page.goto(path);
			await expect(page.locator(`h1, h2`).filter({ hasText: title })).toBeVisible({ timeout: 10_000 });
			await expect(page.locator(`h2`).filter({ hasText: removedSection })).not.toBeVisible();
		});
	}
}

test('/software: TabStrip with All/Featured/Unfeatured tabs is absent', async ({ page }) => {
	await page.goto('/software');
	await expect(page.locator('h1, h2').filter({ hasText: 'Software' })).toBeVisible({ timeout: 10_000 });
	// The old TabStrip rendered buttons with these exact labels at top of page.
	await expect(page.locator('[role="tablist"] button').filter({ hasText: 'Featured' })).not.toBeVisible();
	await expect(page.locator('[role="tablist"] button').filter({ hasText: 'Unfeatured' })).not.toBeVisible();
	// The featured Select is present in the FilterBar instead.
	await expect(
		page.locator('[data-ui="filter-bar"] select, [data-ui="filter-bar"] [aria-label="Filter by featured status"]')
	).toBeVisible();
});

test('/software: URL reactivity — navigating to ?updatable=true applies filter', async ({ page }) => {
	// Start on /software without any filter.
	await page.goto('/software');
	await expect(page.locator('h1').filter({ hasText: 'Software' })).toBeVisible({ timeout: 10_000 });

	// Navigate to the same page with ?updatable=true (simulates clicking an external badge).
	await page.goto('/software?updatable=true');

	// The updatable checkbox should be checked.
	const checkbox = page.locator('#software-filter-updatable-only');
	await expect(checkbox).toBeChecked({ timeout: 5_000 });
});

test('/host-tags: ExpandableSearch is inside table card, not separate SectionCard', async ({ page }) => {
	await page.goto('/host-tags');
	await expect(page.locator('h1').filter({ hasText: 'Host Tags' })).toBeVisible({ timeout: 10_000 });
	const filterBar = page.locator('[data-ui="filter-bar"]');
	await expect(filterBar).toBeVisible();
	// The search icon button is inside the filter bar.
	await expect(filterBar.locator('button').first()).toBeVisible();
	// No separate "Search" heading.
	await expect(page.locator('h2').filter({ hasText: 'Search' })).not.toBeVisible();
});
