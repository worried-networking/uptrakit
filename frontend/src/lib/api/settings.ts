import { authenticatedFetch, extractErrorMessage } from '$lib/api';
import type { AccessSettingsData, AccessSettingsWithEtag, UpdateAccessSettingsRequest } from '$lib/types';

export async function getAccessSettings(): Promise<AccessSettingsWithEtag> {
	let res: Response;
	try {
		res = await authenticatedFetch('/api/v1/settings/access');
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
	const data: AccessSettingsData = await res.json();
	return { data, etag: res.headers.get('etag') };
}

export async function updateAccessSettings(
	body: UpdateAccessSettingsRequest,
	etag: string | null
): Promise<AccessSettingsWithEtag> {
	const headers: Record<string, string> = {};
	if (etag !== null) headers['if-match'] = etag;
	let res: Response;
	try {
		res = await authenticatedFetch('/api/v1/settings/access', {
			method: 'PUT',
			body: JSON.stringify(body),
			headers
		});
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
	const data: AccessSettingsData = await res.json();
	return { data, etag: res.headers.get('etag') };
}
