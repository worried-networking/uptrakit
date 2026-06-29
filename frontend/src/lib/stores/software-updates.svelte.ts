import { listSoftwareItems } from '$lib/api';

let count: number | null = $state(null);

/** Reactive getter — null before first successful fetch. */
export function getUpdatableSoftwareCount(): number | null {
	return count;
}

/**
 * Fetch the number of software items with updates available.
 *
 * Idempotent: if count is already set, returns immediately without a network
 * request. Silently swallows errors — the badge is non-critical.
 */
export async function fetchUpdatableSoftwareCount(): Promise<void> {
	if (count !== null) return;
	try {
		const { data: res } = await listSoftwareItems({ query: { per_page: 1, featured: true, updatable: true } });
		count = res.total;
	} catch {
		// Non-critical — badge stays hidden on error.
	}
}
