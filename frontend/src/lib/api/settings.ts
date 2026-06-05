import { request } from '$lib/api';
import type { AccessSettingsData, UpdateAccessSettingsRequest } from '$lib/types';

export function getAccessSettings(): Promise<AccessSettingsData> {
	return request('/settings/access');
}

export function updateAccessSettings(body: UpdateAccessSettingsRequest): Promise<AccessSettingsData> {
	return request('/settings/access', {
		method: 'PUT',
		body: JSON.stringify(body)
	});
}
