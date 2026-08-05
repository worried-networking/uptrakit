import { authenticatedFetch, extractErrorMessage } from '$lib/api';
import { getOauthSettings, updateOauthSettings } from './generated';
import type { OAuthSettingsResponse, UpdateOAuthSettingsRequest } from './generated';

// This module covers only the browser OAuth consent flow (paths outside /api/v1) plus
// the /api/v1 OAuth settings passthroughs below. Operator client management and end-user
// consent management now go through the generated SDK ($lib/api) — see McpAccessTab.svelte
// and routes/settings/account/authorized-apps/+page.svelte.
//
// Internal helper — OAuth consent-flow paths are NOT under /api/v1/, so we use
// authenticatedFetch directly with absolute paths instead of the BASE-prefixed request() helper.
async function oauthRequest<T>(path: string, options: RequestInit = {}): Promise<T> {
	let res: Response;
	try {
		res = await authenticatedFetch(path, options);
	} catch (err) {
		if (err instanceof DOMException && (err.name === 'AbortError' || err.name === 'TimeoutError')) {
			throw new Error('Request timed out. Please try again.');
		} else if (err instanceof TypeError) {
			throw new Error('Network error: Unable to connect to the server. Check your network connection.');
		}
		throw err;
	}
	if (!res.ok) {
		const message = await extractErrorMessage(res);
		throw new Error(message);
	}
	return res.json();
}

export interface MetadataDiff {
	redirect_uris?: { from: string[]; to: string[] };
	client_name?: { from: string; to: string };
	client_uri?: { from: string | null; to: string | null };
}

export interface ConsentDetails {
	client_id: string;
	client_name: string;
	client_uri: string | null;
	redirect_uri: string;
	redirect_uri_host: string;
	scopes: string[];
	created_via: 'dcr' | 'cimd_cache' | 'manual';
	trusted_at: string | null;
	requires_typed_confirmation: boolean;
	typed_confirmation_value: string;
	metadata_change_diff: MetadataDiff | null;
}

export async function getConsentDetails(requestId: string): Promise<ConsentDetails> {
	return oauthRequest(`/oauth/consent/${encodeURIComponent(requestId)}`);
}

export async function approveConsent(requestId: string): Promise<{ redirect_to: string }> {
	return oauthRequest(`/oauth/consent/${encodeURIComponent(requestId)}/approve`, {
		method: 'POST',
		body: JSON.stringify({})
	});
}

export async function denyConsent(requestId: string): Promise<{ redirect_to: string }> {
	return oauthRequest(`/oauth/consent/${encodeURIComponent(requestId)}/deny`, {
		method: 'POST'
	});
}

// The OAuth global-settings endpoints ARE under /api/v1, so they route through the
// generated SDK (auth/ETag/refresh interceptors applied by the configured client) —
// unlike the OAuth client/consent paths above, which live outside /api/v1.
export type { OAuthSettingsResponse, UpdateOAuthSettingsRequest };

export async function getOAuthSettings(): Promise<OAuthSettingsResponse> {
	const { data } = await getOauthSettings();
	return data;
}

export async function updateOAuthSettings(body: UpdateOAuthSettingsRequest): Promise<OAuthSettingsResponse> {
	const { data } = await updateOauthSettings({ body });
	return data;
}
