// frontend/tests/e2e/parity-fixtures.ts
import { test as base } from '@playwright/test';
import type { Page, TestInfo } from '@playwright/test';

export type ParityTheme = 'light' | 'dark';

export const parityTest = base.extend<{ parityTheme: ParityTheme }>({
	// eslint-disable-next-line no-empty-pattern
	parityTheme: async ({}, use, testInfo: TestInfo) => {
		const theme = testInfo.project.name.includes('dark') ? 'dark' : 'light';
		await use(theme);
	}
});

export async function freezeParityInputs(page: Page, theme: ParityTheme) {
	await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
	await page.addInitScript((t) => {
		localStorage.setItem('theme-mode', t);
	}, theme);
}
