import { describe, expect, it } from 'vitest';
import { computeMenuPosition } from './context-menu-position';

const VIEWPORT = { vw: 1280, vh: 800 };
const MENU = { width: 176, height: 240 };

function rect(x: number, y: number, width: number, height: number): DOMRect {
	return new DOMRect(x, y, width, height);
}

describe('computeMenuPosition', () => {
	it('places menu top-left at trigger bottom-right (with horizontal gap matching vertical) when right side fits', () => {
		// Trigger near top-left; lots of space on the right and below.
		const anchor = rect(100, 100, 32, 32);
		const result = computeMenuPosition(anchor, MENU, VIEWPORT);
		expect(result).toEqual({ left: anchor.right + 2, top: anchor.bottom + 2 });
	});

	it('flips horizontally when right side would overflow', () => {
		// Trigger near right edge: anchor.right + gap + menu.width + pad > vw.
		const anchor = rect(VIEWPORT.vw - 50, 100, 32, 32);
		const result = computeMenuPosition(anchor, MENU, VIEWPORT);
		expect(result.left).toBe(anchor.left - 2 - MENU.width);
		expect(result.top).toBe(anchor.bottom + 2);
	});

	it('clamps menu bottom to vh - pad when below would overflow', () => {
		// Mid-viewport horizontally; trigger near bottom: anchor.bottom + gap + menu.height + pad > vh.
		const anchor = rect(100, VIEWPORT.vh - 80, 32, 32);
		const result = computeMenuPosition(anchor, MENU, VIEWPORT);
		// Default horizontal placement (right side fits), vertical clamp.
		expect(result.left).toBe(anchor.right + 2);
		expect(result.top).toBe(VIEWPORT.vh - 8 - MENU.height);
	});

	it('flips horizontally and clamps vertically when trigger sits in the bottom-right corner', () => {
		const anchor = rect(VIEWPORT.vw - 50, VIEWPORT.vh - 80, 32, 32);
		const result = computeMenuPosition(anchor, MENU, VIEWPORT);
		expect(result.left).toBe(anchor.left - 2 - MENU.width);
		expect(result.top).toBe(VIEWPORT.vh - 8 - MENU.height);
	});

	it('pins top to pad when even the clamped placement would overflow the top edge', () => {
		// Menu taller than viewport.
		const tallMenu = { width: 176, height: VIEWPORT.vh + 200 };
		const anchor = rect(100, 100, 32, 32);
		const result = computeMenuPosition(anchor, tallMenu, VIEWPORT);
		expect(result.top).toBe(8);
	});

	it('clamps left within [pad, vw - pad - width] when neither side fully fits', () => {
		// Tiny viewport: menu wider than vw - 2*pad. Both branches push left negative or beyond clamp range.
		const narrowVp = { vw: 200, vh: 800 };
		const anchor = rect(100, 100, 32, 32);
		const result = computeMenuPosition(anchor, MENU, narrowVp);
		expect(result.left).toBeGreaterThanOrEqual(8);
		expect(result.left).toBeLessThanOrEqual(narrowVp.vw - 8 - MENU.width);
	});

	it('respects explicit pad and gap options', () => {
		const anchor = rect(100, 100, 32, 32);
		const result = computeMenuPosition(anchor, MENU, VIEWPORT, { pad: 16, gap: 12 });
		expect(result.top).toBe(anchor.bottom + 12);
		expect(result.left).toBe(anchor.right + 12);
	});
});
