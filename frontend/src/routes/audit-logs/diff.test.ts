import { describe, expect, it } from 'vitest';
import { computeDiff, type DiffEntry } from './diff';

describe('computeDiff', () => {
	it('marks added/removed/changed/unchanged keys', () => {
		const before = { name: 'alpha', enabled: false, removed_only: 1 };
		const after = { name: 'alpha', enabled: true, added_only: 'x' };
		const rows = computeDiff(before, after);
		const get = (k: string): DiffEntry => rows.find((r) => r.key === k)!;

		expect(get('name').status).toBe('unchanged');
		expect(get('enabled').status).toBe('changed');
		expect(get('removed_only').status).toBe('removed');
		expect(get('added_only').status).toBe('added');
	});

	it('handles null snapshots', () => {
		expect(computeDiff(null, { a: 1 })).toEqual([{ key: 'a', status: 'added', before: undefined, after: 1 }]);
		expect(computeDiff({ a: 1 }, null)).toEqual([{ key: 'a', status: 'removed', before: 1, after: undefined }]);
	});

	it('preserves declared key order from after when possible', () => {
		const before = { a: 1, b: 2, c: 3 };
		const after = { c: 30, a: 10, b: 20 };
		const rows = computeDiff(before, after);
		expect(rows.map((r) => r.key)).toEqual(['c', 'a', 'b']);
	});
});
