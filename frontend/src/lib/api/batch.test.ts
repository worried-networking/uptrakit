import { it, expect, vi } from 'vitest';
import { executeBatchChunked } from './batch';

it('chunks ids into <=100 and aggregates', async () => {
	const ids = Array.from({ length: 250 }, (_, i) => String(i));
	const fn = vi.fn(async (_a: string, batch: string[]) => ({
		succeeded: batch.map((id) => ({ id })),
		failed: []
	}));
	const r = await executeBatchChunked('approve', ids, fn);
	expect(fn).toHaveBeenCalledTimes(3); // 100 + 100 + 50
	expect(r.succeeded).toHaveLength(250);
});
