import { expect, test } from '@playwright/test';
import type { Locator, Page } from '@playwright/test';

type Rect = { x: number; y: number; width: number; height: number };

export const PARITY_MAX_DIFF_PIXEL_RATIO = 0.005;
export const PARITY_MAX_MASKED_AREA_RATIO = 0.15;
export const PARITY_DYNAMIC_MASK_SELECTOR = '[data-visual-dynamic]';
export const PARITY_ALLOWED_MASK_SELECTORS = [PARITY_DYNAMIC_MASK_SELECTOR] as const;
export const PARITY_LOCALE = 'en-US';
export const PARITY_TIMEZONE = 'UTC';
export const PARITY_REQUIRED_PROJECT = 'chromium';

export const PARITY_VIEWPORT_PRESETS = {
	desktop: { width: 1440, height: 900 },
	tablet: { width: 820, height: 1180 },
	mobile: { width: 393, height: 852 }
} as const;

export type ParityViewportPreset = keyof typeof PARITY_VIEWPORT_PRESETS;

type ParityScreenshotTarget = Locator | Page;

type ParityScreenshotOptions = {
	page: Page;
	target: ParityScreenshotTarget;
	name: string;
	viewport: ParityViewportPreset;
	maskSelectors?: readonly string[];
	waiverMaxMaskedAreaRatio?: number;
};

function assertProjectGuard() {
	const projectName = test.info().project.name;
	if (!projectName.startsWith(PARITY_REQUIRED_PROJECT)) {
		throw new Error(
			`ui parity harness requires Playwright project "${PARITY_REQUIRED_PROJECT}" ` +
				`(or a variant), received "${projectName}".`
		);
	}
}

function assertMaskSelectorAllowlist(maskSelectors: readonly string[]) {
	for (const selector of maskSelectors) {
		if (!PARITY_ALLOWED_MASK_SELECTORS.includes(selector as (typeof PARITY_ALLOWED_MASK_SELECTORS)[number])) {
			throw new Error(
				`ui parity mask selector "${selector}" is not allowlisted. Use checked-in selectors or ${PARITY_DYNAMIC_MASK_SELECTOR}.`
			);
		}
	}
}

function toTargetRect(viewportRect: { width: number; height: number }, targetRect: Rect | null): Rect {
	if (targetRect) return targetRect;
	return { x: 0, y: 0, width: viewportRect.width, height: viewportRect.height };
}

function normalizeRect(rect: Rect, bounds: Rect): Rect | null {
	const left = Math.max(rect.x, bounds.x);
	const top = Math.max(rect.y, bounds.y);
	const right = Math.min(rect.x + rect.width, bounds.x + bounds.width);
	const bottom = Math.min(rect.y + rect.height, bounds.y + bounds.height);
	const width = right - left;
	const height = bottom - top;
	if (width <= 0 || height <= 0) return null;
	return { x: left, y: top, width, height };
}

function unionArea(rects: Rect[]): number {
	if (rects.length === 0) return 0;

	const xEdges = new Set<number>();
	for (const rect of rects) {
		xEdges.add(rect.x);
		xEdges.add(rect.x + rect.width);
	}
	const sortedX = [...xEdges].sort((a, b) => a - b);
	if (sortedX.length < 2) return 0;

	let totalArea = 0;
	for (let i = 0; i < sortedX.length - 1; i += 1) {
		const x1 = sortedX[i];
		const x2 = sortedX[i + 1];
		const dx = x2 - x1;
		if (dx <= 0) continue;

		const yIntervals: Array<{ start: number; end: number }> = [];
		for (const rect of rects) {
			const rectX1 = rect.x;
			const rectX2 = rect.x + rect.width;
			if (rectX1 >= x2 || rectX2 <= x1) continue;
			yIntervals.push({ start: rect.y, end: rect.y + rect.height });
		}
		if (yIntervals.length === 0) continue;

		yIntervals.sort((a, b) => a.start - b.start || a.end - b.end);
		let coveredY = 0;
		let currentStart = yIntervals[0].start;
		let currentEnd = yIntervals[0].end;
		for (let j = 1; j < yIntervals.length; j += 1) {
			const next = yIntervals[j];
			if (next.start <= currentEnd) {
				currentEnd = Math.max(currentEnd, next.end);
				continue;
			}
			coveredY += currentEnd - currentStart;
			currentStart = next.start;
			currentEnd = next.end;
		}
		coveredY += currentEnd - currentStart;
		totalArea += coveredY * dx;
	}

	return totalArea;
}

async function collectMaskRects(page: Page, selectors: readonly string[]): Promise<Rect[]> {
	if (selectors.length === 0) return [];
	return page.evaluate((requestedSelectors: readonly string[]) => {
		const rects: Rect[] = [];
		for (const selector of requestedSelectors) {
			const nodes = document.querySelectorAll<HTMLElement>(selector);
			for (const node of nodes) {
				const style = window.getComputedStyle(node);
				if (style.display === 'none' || style.visibility === 'hidden' || Number(style.opacity || '1') === 0) {
					continue;
				}
				const rect = node.getBoundingClientRect();
				if (rect.width <= 0 || rect.height <= 0) continue;
				rects.push({
					x: rect.left,
					y: rect.top,
					width: rect.width,
					height: rect.height
				});
			}
		}
		return rects;
	}, selectors);
}

async function assertDeterministicCaptureProfile(page: Page, viewport: ParityViewportPreset) {
	assertProjectGuard();

	const expectedViewport = PARITY_VIEWPORT_PRESETS[viewport];
	const actualViewport = page.viewportSize();
	if (!actualViewport) {
		throw new Error('ui parity capture requires a concrete viewport size.');
	}
	if (actualViewport.width !== expectedViewport.width || actualViewport.height !== expectedViewport.height) {
		throw new Error(
			`ui parity viewport mismatch for "${viewport}". Expected ${expectedViewport.width}x${expectedViewport.height}, received ${actualViewport.width}x${actualViewport.height}.`
		);
	}

	const env = await page.evaluate(() => ({
		language: navigator.language,
		timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
		reducedMotion: window.matchMedia('(prefers-reduced-motion: reduce)').matches,
		devicePixelRatio: window.devicePixelRatio,
		prefersDark: window.matchMedia('(prefers-color-scheme: dark)').matches
	}));

	if (env.language !== PARITY_LOCALE) {
		throw new Error(`ui parity locale drift: expected ${PARITY_LOCALE}, received ${env.language}.`);
	}
	if (env.timezone !== PARITY_TIMEZONE) {
		throw new Error(`ui parity timezone drift: expected ${PARITY_TIMEZONE}, received ${env.timezone}.`);
	}
	if (!env.reducedMotion) {
		throw new Error('ui parity capture must run with prefers-reduced-motion: reduce.');
	}
	if (Math.abs(env.devicePixelRatio - 1) > 0.001) {
		throw new Error(`ui parity DPR drift: expected 1, received ${env.devicePixelRatio}.`);
	}

	const projectName = test.info().project.name;
	const expectedDark = projectName.includes('dark');
	if (env.prefersDark !== expectedDark) {
		throw new Error(
			`ui parity colorScheme mismatch: project "${projectName}" expects ` +
				`${expectedDark ? 'dark' : 'light'} but page has ` +
				`${env.prefersDark ? 'dark' : 'light'}.`
		);
	}
}

async function assertMaskedAreaBudget(
	page: Page,
	target: ParityScreenshotTarget,
	maskSelectors: readonly string[],
	waiverMaxMaskedAreaRatio: number | undefined
) {
	if (maskSelectors.length === 0) return;

	const viewportRect = page.viewportSize();
	if (!viewportRect) return;
	const targetRect = 'boundingBox' in target ? await target.boundingBox() : null;
	const captureRect = toTargetRect(viewportRect, targetRect);
	const targetArea = captureRect.width * captureRect.height;
	if (targetArea <= 0) return;

	const maskRects = await collectMaskRects(page, maskSelectors);
	const clippedRects: Rect[] = [];
	for (const rect of maskRects) {
		const clipped = normalizeRect(rect, captureRect);
		if (clipped) clippedRects.push(clipped);
	}
	const maskedArea = unionArea(clippedRects);
	const maskedAreaRatio = maskedArea / targetArea;
	const budget = waiverMaxMaskedAreaRatio ?? PARITY_MAX_MASKED_AREA_RATIO;
	if (maskedAreaRatio > budget) {
		throw new Error(
			`ui parity mask area ${Math.round(maskedAreaRatio * 10000) / 100}% exceeds budget ${Math.round(budget * 10000) / 100}%.`
		);
	}
}

export async function expectParityScreenshot({
	page,
	target,
	name,
	viewport,
	maskSelectors = [],
	waiverMaxMaskedAreaRatio
}: ParityScreenshotOptions) {
	await assertDeterministicCaptureProfile(page, viewport);
	assertMaskSelectorAllowlist(maskSelectors);
	await assertMaskedAreaBudget(page, target, maskSelectors, waiverMaxMaskedAreaRatio);

	const mask = maskSelectors.map((selector) => page.locator(selector));
	await expect(target).toHaveScreenshot(name, {
		animations: 'disabled',
		caret: 'hide',
		maxDiffPixelRatio: PARITY_MAX_DIFF_PIXEL_RATIO,
		mask
	});
}
