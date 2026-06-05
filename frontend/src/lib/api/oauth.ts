import { authenticatedFetch, extractErrorMessage, request } from '$lib/api';

// Internal helper — OAuth paths are NOT under /api/v1/, so we use authenticatedFetch
// directly with absolute paths instead of the BASE-prefixed request() helper.
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

async function oauthRequestVoid(path: string, options: RequestInit = {}): Promise<void> {
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

export interface OAuthClient {
	id: string;
	client_name: string;
	client_uri: string | null;
	created_via: 'dcr' | 'cimd_cache' | 'manual';
	created_at: string;
	last_used_at: string | null;
	revoked_at: string | null;
	trusted_at: string | null;
	redirect_uris: string[];
}

export async function listOAuthClients(): Promise<OAuthClient[]> {
	return oauthRequest('/api/oauth/clients');
}

export async function revokeOAuthClient(clientId: string): Promise<void> {
	return oauthRequestVoid(`/api/oauth/clients/${encodeURIComponent(clientId)}`, {
		method: 'DELETE'
	});
}

export async function trustOAuthClient(clientId: string): Promise<void> {
	return oauthRequestVoid(`/api/oauth/clients/${encodeURIComponent(clientId)}/trust`, {
		method: 'POST'
	});
}

export interface ManualRegisterClientRequest {
	client_name: string;
	client_uri: string | null;
	redirect_uris: string[];
	default_scope: string;
	token_endpoint_auth_method: 'none' | 'client_secret_basic';
}

export async function manualRegisterClient(body: ManualRegisterClientRequest): Promise<OAuthClient> {
	return oauthRequest('/api/oauth/clients', {
		method: 'POST',
		body: JSON.stringify(body)
	});
}

export interface OAuthConsent {
	id: string;
	client_id: string;
	client_name: string;
	scopes: string[];
	granted_at: string;
	last_used_at: string | null;
}

export async function listMyConsents(): Promise<OAuthConsent[]> {
	return oauthRequest('/api/oauth/consents');
}

export async function revokeMyConsent(consentId: string): Promise<void> {
	return oauthRequestVoid(`/api/oauth/consents/${encodeURIComponent(consentId)}`, {
		method: 'DELETE'
	});
}

export interface OAuthSettingsResponse {
	mcp_enabled: boolean;
	dcr_enabled: boolean;
	cimd_enabled: boolean;
	canonical_host: string | null;
	restart_required: boolean;
}

export interface UpdateOAuthSettingsRequest {
	mcp_enabled?: boolean;
	dcr_enabled?: boolean;
	cimd_enabled?: boolean;
	canonical_host?: string;
}

export function getOAuthSettings(): Promise<OAuthSettingsResponse> {
	return request('/global-settings/oauth');
}

export function updateOAuthSettings(body: UpdateOAuthSettingsRequest): Promise<OAuthSettingsResponse> {
	return request('/global-settings/oauth', {
		method: 'PUT',
		body: JSON.stringify(body)
	});
}
