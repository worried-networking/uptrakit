import type { BatchActionResponse } from './generated';

const CHUNK_SIZE = 100;

/**
 * Splits `ids` into chunks of at most 100 and calls `batchFn` sequentially,
 * aggregating the results. Use this whenever a selection may exceed the
 * server-side batch limit.
 */
export async function executeBatchChunked(
	action: string,
	ids: string[],
	batchFn: (action: string, ids: string[]) => Promise<BatchActionResponse>
): Promise<BatchActionResponse> {
	const result: BatchActionResponse = { succeeded: [], failed: [] };
	for (let i = 0; i < ids.length; i += CHUNK_SIZE) {
		const r = await batchFn(action, ids.slice(i, i + CHUNK_SIZE));
		result.succeeded.push(...r.succeeded);
		result.failed.push(...r.failed);
	}
	return result;
}
