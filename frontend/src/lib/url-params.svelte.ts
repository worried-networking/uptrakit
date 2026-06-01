import { page } from '$app/state';
import { goto } from '$app/navigation';
import { SvelteURL } from 'svelte/reactivity';

export interface UrlParamOptions<T> {
	parse?: (raw: string | null) => T;
	serialize?: (value: T) => string | null;
}

export interface UrlParam<T> {
	readonly value: T;
	set(value: T): void;
}

/**
 * Creates a URL search-param binding backed by $derived from page.url.searchParams.
 *
 * CONSTRAINT: must be called at component initialisation scope only — top-level
 * <script> in .svelte files, or top-level of .svelte.ts modules. Calling inside
 * a callback or event handler causes a Svelte rune-outside-reactive-context error.
 *
 * set() always removes 'page=' from the URL (resets pagination on filter change).
 * Does NOT re-run SvelteKit load() on pages that fetch data client-side via $effect.
 */
export function createUrlParam<T = string>(key: string, options?: UrlParamOptions<T>): UrlParam<T> {
	const parse = options?.parse ?? ((raw) => (raw ?? '') as unknown as T);
	const serialize = options?.serialize ?? ((v) => (v === '' || v == null ? null : String(v)));

	const derived = $derived(parse(page.url.searchParams.get(key)));

	return {
		get value() {
			return derived;
		},
		set(value: T) {
			const next = new SvelteURL(page.url.href);
			const serialized = serialize(value);
			if (serialized == null) {
				next.searchParams.delete(key);
			} else {
				next.searchParams.set(key, serialized);
			}
			next.searchParams.delete('page');
			void goto(next, { replaceState: true, keepFocus: true, noScroll: true });
		}
	};
}
