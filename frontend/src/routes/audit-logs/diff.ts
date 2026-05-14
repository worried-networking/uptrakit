export type DiffStatus = 'unchanged' | 'changed' | 'added' | 'removed';

export type DiffEntry = {
	key: string;
	status: DiffStatus;
	before: unknown;
	after: unknown;
};

export function computeDiff(
	before: Record<string, unknown> | null,
	after: Record<string, unknown> | null
): DiffEntry[] {
	const out: DiffEntry[] = [];
	const beforeKeys = before ? Object.keys(before) : [];
	const afterKeys = after ? Object.keys(after) : [];
	const seen = new Set<string>();

	for (const key of afterKeys) {
		seen.add(key);
		const a = after![key];
		if (!before || !(key in before)) {
			out.push({ key, status: 'added', before: undefined, after: a });
			continue;
		}
		const b = before[key];
		const status: DiffStatus = jsonEqual(a, b) ? 'unchanged' : 'changed';
		out.push({ key, status, before: b, after: a });
	}

	for (const key of beforeKeys) {
		if (seen.has(key)) continue;
		out.push({ key, status: 'removed', before: before![key], after: undefined });
	}

	return out;
}

function jsonEqual(a: unknown, b: unknown): boolean {
	return JSON.stringify(a) === JSON.stringify(b);
}
