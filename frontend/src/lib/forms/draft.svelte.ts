export interface FormDraft<T extends Record<string, unknown>> {
	readonly draft: T;
	readonly serverValues: T;
	readonly isDirty: boolean;
	isFieldDirty(key: keyof T): boolean;
	update<K extends keyof T>(key: K, value: T[K]): void;
	load(values: T): void;
	commit(updated: T): void;
	discard(): void;
}

// null, undefined, '', and NaN are all "no value" — treat as equal for dirty detection.
// Handles the common case where a cleared <input type="number"> produces '' but the
// server-side original was null.
function isEmpty(v: unknown): boolean {
	return v === null || v === undefined || v === '' || (typeof v === 'number' && isNaN(v));
}

function valuesEqual(a: unknown, b: unknown): boolean {
	return a === b || (isEmpty(a) && isEmpty(b));
}

export function createFormDraft<T extends Record<string, unknown>>(initial: T): FormDraft<T> {
	let serverValues = $state<T>({ ...initial });
	let draft = $state<T>({ ...initial });

	const isDirty = $derived(
		(Object.keys(serverValues) as (keyof T)[]).some((k) => !valuesEqual(draft[k], serverValues[k]))
	);

	return {
		get draft() {
			return draft;
		},
		get serverValues() {
			return serverValues;
		},
		get isDirty() {
			return isDirty;
		},
		isFieldDirty(key) {
			return !valuesEqual(draft[key], serverValues[key]);
		},
		update(key, value) {
			draft[key] = value;
		},
		load(values) {
			serverValues = { ...values };
			draft = { ...values };
		},
		commit(updated) {
			serverValues = { ...updated };
			draft = { ...updated };
		},
		discard() {
			draft = { ...serverValues };
		}
	};
}
