import { describe, expect, it } from 'vitest';
import { htmlEscape } from './htmlEscape';

describe('htmlEscape', () => {
	it('escapes script tags', () => {
		expect(htmlEscape('<script>alert(1)</script>')).toBe('&lt;script&gt;alert(1)&lt;/script&gt;');
	});

	it('escapes ampersands first', () => {
		expect(htmlEscape('a & b')).toBe('a &amp; b');
	});

	it('returns plain text unchanged', () => {
		expect(htmlEscape('Cursor IDE')).toBe('Cursor IDE');
	});
});
