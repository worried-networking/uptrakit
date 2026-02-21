import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { copyToClipboard, formatDate, isValidLogoUrl, safeRedirect } from './utils';

// ── isValidLogoUrl ────────────────────────────────────────────────────────────

describe('isValidLogoUrl', () => {
	it('accepts a valid https URL', () => {
		expect(isValidLogoUrl('https://example.com/logo.png')).toBe(true);
	});

	it('rejects http URLs', () => {
		expect(isValidLogoUrl('http://example.com/logo.png')).toBe(false);
	});

	it('rejects javascript: URLs', () => {
		expect(isValidLogoUrl('javascript:alert(1)')).toBe(false);
	});

	it('rejects data: URLs', () => {
		expect(isValidLogoUrl('data:image/png;base64,abc123')).toBe(false);
	});

	it('rejects null', () => {
		expect(isValidLogoUrl(null)).toBe(false);
	});

	it('rejects undefined', () => {
		expect(isValidLogoUrl(undefined)).toBe(false);
	});

	it('rejects empty string', () => {
		expect(isValidLogoUrl('')).toBe(false);
	});

	it('rejects a malformed URL', () => {
		expect(isValidLogoUrl('not a url at all')).toBe(false);
	});
});

// ── formatDate ────────────────────────────────────────────────────────────────

describe('formatDate', () => {
	it('formats a valid ISO date string', () => {
		const result = formatDate('2025-06-01T12:00:00Z');
		expect(typeof result).toBe('string');
		expect(result.length).toBeGreaterThan(0);
		// Should not be the em-dash fallback
		expect(result).not.toBe('\u2014');
	});

	it('returns em-dash for null', () => {
		expect(formatDate(null)).toBe('\u2014');
	});

	it('returns em-dash for undefined', () => {
		expect(formatDate(undefined)).toBe('\u2014');
	});

	it('returns em-dash for empty string', () => {
		expect(formatDate('')).toBe('\u2014');
	});
});

// ── safeRedirect ──────────────────────────────────────────────────────────────

describe('safeRedirect', () => {
	it('accepts a valid relative path', () => {
		expect(safeRedirect('/dashboard')).toBe('/dashboard');
	});

	it('accepts a path with query string', () => {
		expect(safeRedirect('/hosts?page=2')).toBe('/hosts?page=2');
	});

	it('rejects a protocol-relative URL (starts with //)', () => {
		expect(safeRedirect('//evil.com')).toBe('/');
	});

	it('rejects an absolute http URL', () => {
		expect(safeRedirect('http://evil.com')).toBe('/');
	});

	it('rejects an absolute https URL', () => {
		expect(safeRedirect('https://evil.com/path')).toBe('/');
	});

	it('returns / for null', () => {
		expect(safeRedirect(null)).toBe('/');
	});

	it('returns / for empty string', () => {
		expect(safeRedirect('')).toBe('/');
	});
});

// ── copyToClipboard ───────────────────────────────────────────────────────────

describe('copyToClipboard', () => {
	beforeEach(() => {
		Object.defineProperty(globalThis, 'navigator', {
			value: { clipboard: { writeText: vi.fn() } },
			writable: true,
			configurable: true
		});
	});

	afterEach(() => {
		vi.restoreAllMocks();
	});

	it('returns true on success', async () => {
		vi.mocked(navigator.clipboard.writeText).mockResolvedValue(undefined);
		const result = await copyToClipboard('hello');
		expect(result).toBe(true);
		expect(navigator.clipboard.writeText).toHaveBeenCalledWith('hello');
	});

	it('returns false when clipboard write throws', async () => {
		vi.mocked(navigator.clipboard.writeText).mockRejectedValue(new Error('denied'));
		const result = await copyToClipboard('hello');
		expect(result).toBe(false);
	});
});
