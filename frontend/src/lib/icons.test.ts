import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Box, Trash2 } from 'lucide-svelte';
import { ICONS, resolveIcon } from './icons';

describe('resolveIcon', () => {
	let consoleErrorSpy: ReturnType<typeof vi.spyOn>;

	beforeEach(() => {
		consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
	});

	afterEach(() => {
		consoleErrorSpy.mockRestore();
	});

	it('resolves a known kebab-case icon name', () => {
		const result = resolveIcon('trash-2');
		expect(result.ok).toBe(true);
		expect(result.component).toBe(Trash2);
		expect(consoleErrorSpy).not.toHaveBeenCalled();
	});

	it('returns the Box fallback and logs an error for an unknown name', () => {
		const result = resolveIcon('Trash2');
		expect(result.ok).toBe(false);
		expect(result.component).toBe(Box);
		expect(consoleErrorSpy).toHaveBeenCalledWith('[surfaces] Unknown icon name: "Trash2"');
	});

	it('returns the Box fallback without logging when the name is null', () => {
		const result = resolveIcon(null);
		expect(result.ok).toBe(false);
		expect(result.component).toBe(Box);
		expect(consoleErrorSpy).not.toHaveBeenCalled();
	});

	it('returns the Box fallback without logging when the name is undefined', () => {
		const result = resolveIcon(undefined);
		expect(result.ok).toBe(false);
		expect(result.component).toBe(Box);
		expect(consoleErrorSpy).not.toHaveBeenCalled();
	});

	it('contains every key referenced by the surface refactor', () => {
		const required = [
			'box',
			'boxes',
			'check',
			'link',
			'plug-zap',
			'radar',
			'refresh-cw',
			'server-cog',
			'trash-2',
			'unlink'
		];
		for (const key of required) {
			expect(ICONS[key], `expected key "${key}" in ICONS`).toBeDefined();
		}
	});
});
