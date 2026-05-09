export type Viewport = { vw: number; vh: number };
export type MenuSize = { width: number; height: number };
export type MenuPositionOpts = { pad?: number; gap?: number };

// Anchor-based placement for ContextMenuShell.
// Default: menu top-left offset by `gap` from trigger's bottom-right corner — menu sits beside (right of) and below the trigger.
// Horizontal flip: top-right offset by `gap` from trigger's bottom-left corner when right side would overflow viewport.
// Vertical clamp: shifts menu up so its bottom rests at vh - pad; horizontal placement unchanged.
// Top edge falls back to pad if even the clamped placement still overflows (menu taller than viewport).
// `gap` applies symmetrically to both axes between trigger and menu; `pad` is the viewport-edge clearance.
export function computeMenuPosition(
	anchor: DOMRect,
	menu: MenuSize,
	viewport: Viewport,
	opts: MenuPositionOpts = {}
): { top: number; left: number } {
	const pad = opts.pad ?? 8;
	const gap = opts.gap ?? 2;
	const { vw, vh } = viewport;

	let left: number;
	if (anchor.right + gap + menu.width + pad <= vw) {
		left = anchor.right + gap;
	} else {
		left = anchor.left - gap - menu.width;
	}
	left = Math.max(pad, Math.min(left, vw - menu.width - pad));

	let top = anchor.bottom + gap;
	if (top + menu.height + pad > vh) {
		top = vh - menu.height - pad;
	}
	top = Math.max(pad, top);

	return { top, left };
}
