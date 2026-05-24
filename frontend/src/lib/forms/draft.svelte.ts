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

export function createFormDraft<T extends Record<string, unknown>>(initial: T): FormDraft<T> {
	let serverValues = $state<T>({ ...initial });
	let draft = $state<T>({ ...initial });

	const isDirty = $derived((Object.keys(serverValues) as (keyof T)[]).some((k) => draft[k] !== serverValues[k]));

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
			return draft[key] !== serverValues[key];
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
