/**
 * Shared token state extracted from auth.svelte.ts to break the
 * circular dependency between api.ts and auth.svelte.ts.
 *
 * Dependency graph:
 *   token-store.svelte.ts  <--  api.ts
 *   token-store.svelte.ts  <--  auth.svelte.ts  -->  api.ts
 */

/** In-memory access token — never persisted to localStorage. */
let accessToken: string | null = null;

export function getAccessToken(): string | null {
	return accessToken;
}

export function setAccessToken(token: string | null): void {
	accessToken = token;
}

/** Reactive flag set when a token refresh fails with a 4xx (session truly expired). */
let sessionExpired = $state(false);

export function getSessionExpired(): boolean {
	return sessionExpired;
}

export function setSessionExpired(v: boolean): void {
	sessionExpired = v;
}
