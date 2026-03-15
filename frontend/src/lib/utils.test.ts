import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
	copyToClipboard,
	formatDate,
	formatVersion,
	isValidExternalUrl,
	isValidLogoUrl,
	parseUrlPage,
	parseUrlParam,
	renderApiSubmitTemplate,
	resolveDisplayVersion,
	safeRedirect
} from './utils';

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

// ── isValidExternalUrl ──────────────────────────────────────────────────────

describe('isValidExternalUrl', () => {
	it('accepts https URLs', () => {
		expect(isValidExternalUrl('https://example.com/release')).toBe(true);
	});

	it('accepts http URLs', () => {
		expect(isValidExternalUrl('http://example.com/release')).toBe(true);
	});

	it('rejects javascript URLs', () => {
		expect(isValidExternalUrl('javascript:alert(1)')).toBe(false);
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

// ── parseUrlParam ─────────────────────────────────────────────────────────────

const TAB_VALUES = ['all', 'pending', 'active'] as const;

describe('parseUrlParam', () => {
	it('returns the value when it matches an allowed value', () => {
		const url = new URL('http://localhost/?tab=pending');
		expect(parseUrlParam(url, 'tab', TAB_VALUES, 'all')).toBe('pending');
	});

	it('returns the fallback when the param is absent', () => {
		const url = new URL('http://localhost/');
		expect(parseUrlParam(url, 'tab', TAB_VALUES, 'all')).toBe('all');
	});

	it('returns the fallback when the value is not in the allowed list', () => {
		const url = new URL('http://localhost/?tab=unknown');
		expect(parseUrlParam(url, 'tab', TAB_VALUES, 'all')).toBe('all');
	});

	it('returns the fallback for an empty string value', () => {
		const url = new URL('http://localhost/?tab=');
		expect(parseUrlParam(url, 'tab', TAB_VALUES, 'all')).toBe('all');
	});

	it('is case-sensitive and rejects values with wrong casing', () => {
		const url = new URL('http://localhost/?tab=Pending');
		expect(parseUrlParam(url, 'tab', TAB_VALUES, 'all')).toBe('all');
	});
});

// ── parseUrlPage ──────────────────────────────────────────────────────────────

describe('parseUrlPage', () => {
	it('returns the page number from the URL', () => {
		const url = new URL('http://localhost/?page=3');
		expect(parseUrlPage(url)).toBe(3);
	});

	it('returns 1 when the page param is absent', () => {
		const url = new URL('http://localhost/');
		expect(parseUrlPage(url)).toBe(1);
	});

	it('returns 1 for a non-integer string', () => {
		const url = new URL('http://localhost/?page=abc');
		expect(parseUrlPage(url)).toBe(1);
	});

	it('returns 1 for zero', () => {
		const url = new URL('http://localhost/?page=0');
		expect(parseUrlPage(url)).toBe(1);
	});

	it('returns 1 for a negative number', () => {
		const url = new URL('http://localhost/?page=-2');
		expect(parseUrlPage(url)).toBe(1);
	});

	it('returns 1 for a float string', () => {
		const url = new URL('http://localhost/?page=2.5');
		expect(parseUrlPage(url)).toBe(1);
	});
});

// ── formatVersion ─────────────────────────────────────────────────────────────

const SHA256_FULL = 'sha256:4d5a6206d4d5aaab00ba55dd8d519619dfdaecd4be3cf737799d9e709d91374a';

describe('formatVersion', () => {
	it('truncates a sha256 digest to short form', () => {
		expect(formatVersion(SHA256_FULL)).toBe('sha256:4d5a6206d4d5\u2026');
	});

	it('returns a short semver unchanged', () => {
		expect(formatVersion('1.2.3')).toBe('1.2.3');
	});

	it('returns em-dash for null', () => {
		expect(formatVersion(null)).toBe('\u2014');
	});

	it('returns em-dash for undefined', () => {
		expect(formatVersion(undefined)).toBe('\u2014');
	});

	it('accepts a custom fallback', () => {
		expect(formatVersion(null, '?')).toBe('?');
	});

	it('truncates a sha256 string that is exactly 19 chars after prefix', () => {
		// "sha256:" (7) + exactly 12 hex chars = boundary: no truncation needed
		const short = 'sha256:123456789012';
		expect(formatVersion(short)).toBe('sha256:123456789012\u2026');
	});

	it('formats an ISO datetime string as a human-readable date', () => {
		const result = formatVersion('2026-03-10T17:20:34Z');
		// Should not be the raw ISO string
		expect(result).not.toBe('2026-03-10T17:20:34Z');
		// Should contain the year
		expect(result).toContain('2026');
		// Should contain the month abbreviation
		expect(result).toMatch(/Mar/i);
	});

	it('sha256 truncation still works after ISO datetime branch added', () => {
		expect(formatVersion(SHA256_FULL)).toBe('sha256:4d5a6206d4d5\u2026');
	});

	it('passes regular semver through unchanged', () => {
		expect(formatVersion('3.14.1')).toBe('3.14.1');
	});
});

// ── resolveDisplayVersion ─────────────────────────────────────────────────────

describe('resolveDisplayVersion', () => {
	it('returns displayVersion when set', () => {
		expect(resolveDisplayVersion('sha256:abc', '2025-01-15')).toBe('2025-01-15');
	});
	it('falls back to canonicalVersion when displayVersion is null', () => {
		expect(resolveDisplayVersion('1.2.3', null)).toBe('1.2.3');
	});
	it('falls back to canonicalVersion when displayVersion is undefined', () => {
		expect(resolveDisplayVersion('1.2.3', undefined)).toBe('1.2.3');
	});
	it('handles null canonical with set displayVersion', () => {
		expect(resolveDisplayVersion(null, '2025-01-15')).toBe('2025-01-15');
	});
	it('returns undefined when both are undefined', () => {
		expect(resolveDisplayVersion(undefined, undefined)).toBeUndefined();
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

// ── renderApiSubmitTemplate ─────────────────────────────────────────────────

describe('renderApiSubmitTemplate', () => {
	it('renders nested placeholders with coercions', () => {
		const template = {
			name: '{{name}}',
			enabled: '{{enabled:bool}}',
			count: '{{count:number}}',
			tags: '{{tags:csv_array}}'
		};

		expect(
			renderApiSubmitTemplate(template, {
				name: 'example',
				enabled: 'true',
				count: '42',
				tags: 'alpha, beta ,, gamma'
			})
		).toEqual({
			name: 'example',
			enabled: true,
			count: 42,
			tags: ['alpha', 'beta', 'gamma']
		});
	});

	it('rejects missing fields', () => {
		expect(() => renderApiSubmitTemplate('{{missing}}', {})).toThrow('Unknown form field "missing"');
	});

	it('rejects invalid numbers', () => {
		expect(() => renderApiSubmitTemplate('{{count:number}}', { count: 'abc' })).toThrow(
			'Field "count" must be a valid number'
		);
	});
});
