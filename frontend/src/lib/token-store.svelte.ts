/**
 * Shared token state extracted from auth.svelte.ts to break the
 * circular dependency between api.ts and auth.svelte.ts.
 *
 * Dependency graph:
 *   token-store.svelte.ts  <--  api.ts
 *   token-store.svelte.ts  <--  auth.svelte.ts  -->  api.ts
 */

import { SvelteSet } from 'svelte/reactivity';

type TokenChangeListener = (prev: string | null, next: string | null) => void;

/** In-memory access token — never persisted to localStorage. */
let accessToken: string | null = null;

// Project lint rule `svelte/prefer-svelte-reactivity` forbids `new Set()` in
// `.svelte.ts` files; `SvelteSet` is the sanctioned imperative replacement here.
const tokenChangeListeners: SvelteSet<TokenChangeListener> = new SvelteSet();

export function getAccessToken(): string | null {
	return accessToken;
}

/** Register a listener invoked synchronously after every `setAccessToken` call.
 *  Returns an unsubscribe handle. Safe under HMR — duplicate registration of the
 *  same callback identity is deduplicated by the underlying `Set`. */
export function onTokenChange(cb: TokenChangeListener): () => void {
	tokenChangeListeners.add(cb);
	return () => {
		tokenChangeListeners.delete(cb);
	};
}

export function setAccessToken(token: string | null): void {
	const prev = accessToken;
	accessToken = token;
	for (const cb of tokenChangeListeners) cb(prev, token);
}

/** Reactive flag set when a token refresh fails with a 4xx (session truly expired). */
let sessionExpired = $state(false);

export function getSessionExpired(): boolean {
	return sessionExpired;
}

export function setSessionExpired(v: boolean): void {
	sessionExpired = v;
}
